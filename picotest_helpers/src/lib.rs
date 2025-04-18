use anyhow::Context;
use log::{debug, info, warn};
use pike::cluster::{PicodataInstance, RunParamsBuilder, StopParamsBuilder, Topology};
use pike::config::{ApplyParamsBuilder, PluginConfigMap};
use rand::distr::Alphanumeric;
use rand::Rng;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::{
    io::Error,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use uuid::Uuid;

const SOCKET_PATH: &str = "cluster/i1/admin.sock";
const TOPOLOGY_FILENAME: &str = "topology.toml";

pub fn tmp_dir() -> PathBuf {
    let mut rng = rand::rng();
    PathBuf::from(format!(
        "tmp/tests/{}",
        (0..8)
            .map(|_| rng.sample(Alphanumeric))
            .map(char::from)
            .collect::<String>()
    ))
}

fn parse_topology(path: &PathBuf) -> anyhow::Result<Topology> {
    toml::from_str(
        &fs::read_to_string(path).context(format!("Failed to read file '{}'", path.display()))?,
    )
    .context(format!(
        "Failed to parse topology TOML from path '{}'",
        path.display()
    ))
}

pub struct Cluster {
    pub uuid: Uuid,
    pub plugin_path: PathBuf,
    pub data_dir: PathBuf,
    pub timeout: Duration,
    socket_path: PathBuf,
    topology: Topology,
    instances: Vec<PicodataInstance>,
}

impl Drop for Cluster {
    fn drop(&mut self) {
        if let Err(err) = self.stop() {
            warn!("Failed to stop picodata cluster: {err}");
        }
    }
}

impl Cluster {
    pub fn new(plugin_path: &str, timeout: Duration) -> anyhow::Result<Self> {
        let data_dir = tmp_dir();
        let plugin_path = PathBuf::from(plugin_path);
        let socket_path = plugin_path.join(&data_dir).join(SOCKET_PATH);

        let topology_path = plugin_path.join(TOPOLOGY_FILENAME);
        let topology = parse_topology(&topology_path)?;

        let cluster = Self {
            uuid: Uuid::new_v4(),
            plugin_path,
            data_dir,
            timeout,
            socket_path,
            topology,
            instances: Default::default(),
        };

        Ok(cluster)
    }

    pub fn stop(&self) -> anyhow::Result<()> {
        let params = StopParamsBuilder::default()
            .plugin_path(self.plugin_path.clone())
            .data_dir(self.data_dir.clone())
            .build()?;

        debug!("Stopping the cluster with parameters {params:?}");
        pike::cluster::stop(&params)?;

        if let Err(err) = fs::remove_dir_all(self.plugin_path.join(&self.data_dir)) {
            warn!("Failed to remove cluster data directory: {err}");
        }

        Ok(())
    }

    pub fn apply_config<T>(&self, config: T) -> anyhow::Result<()>
    where
        T: Into<PluginConfigMap>,
    {
        let params = ApplyParamsBuilder::default()
            .plugin_path(self.plugin_path.clone())
            .data_dir(self.data_dir.clone())
            .config_map(config.into())
            .build()?;

        debug!("Applying plugin configuration with parameters {params:?}");
        pike::config::apply(&params)
    }

    pub fn run(mut self) -> anyhow::Result<Self> {
        let params = RunParamsBuilder::default()
            .plugin_path(self.plugin_path.clone())
            .data_dir(self.data_dir.clone())
            .topology(self.topology.clone())
            .use_release(false)
            .build()?;

        debug!("Starting the cluster with parameters {params:?}");
        let mut intances: Vec<PicodataInstance> = pike::cluster::run(&params)?;
        std::mem::swap(&mut self.instances, &mut intances);
        self.wait()
    }

    pub fn recreate(self) -> anyhow::Result<Self> {
        self.stop()?;
        self.run()
    }

    fn wait(self) -> anyhow::Result<Self> {
        let timeout = Duration::from_secs(60);
        let start_time = Instant::now();

        debug!(
            "Awaiting of cluster readiness (timeout {}s)",
            timeout.as_secs()
        );

        loop {
            let mut picodata_admin: Child = self.await_picodata_admin()?;
            let stdout = picodata_admin
                .stdout
                .take()
                .expect("Failed to capture stdout");
            assert!(start_time.elapsed() < timeout, "cluster setup timeouted");

            let queries = vec![
                r"SELECT enabled FROM _pico_plugin;",
                r"SELECT current_state FROM _pico_instance;",
                r"\help;",
            ];

            {
                let picodata_stdin = picodata_admin.stdin.as_mut().unwrap();
                for query in queries {
                    picodata_stdin.write_all(query.as_bytes()).unwrap();
                }
                picodata_admin.wait().unwrap();
            }

            let mut plugin_ready = false;
            let mut can_connect = false;

            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let line = line.expect("Failed to read picodata stdout");
                if line.contains("true") {
                    plugin_ready = true;
                }
                if line.contains("Connected to admin console by socket") {
                    can_connect = true;
                }
            }

            picodata_admin.kill().unwrap();
            if can_connect && plugin_ready {
                thread::sleep(self.timeout);
                return Ok(self);
            }

            thread::sleep(Duration::from_secs(10));
        }
    }

    pub fn run_query<T: AsRef<[u8]>>(&self, query: T) -> Result<String, Error> {
        let mut picodata_admin = self.await_picodata_admin()?;

        let stdout = picodata_admin
            .stdout
            .take()
            .expect("Failed to capture stdout");
        {
            let picodata_stdin = picodata_admin.stdin.as_mut().unwrap();

            picodata_stdin.write_all(query.as_ref()).unwrap();
            picodata_admin.wait().unwrap();
        }

        let mut result = String::new();
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(l) => result.push_str(&l),
                Err(e) => return Err(e),
            }
        }
        picodata_admin.kill()?;

        Ok(result)
    }

    fn await_picodata_admin(&self) -> Result<Child, Error> {
        let timeout = Duration::from_secs(60);
        let start_time = Instant::now();
        loop {
            assert!(
                start_time.elapsed() < timeout,
                "process hanging for too long"
            );

            let picodata_admin = Command::new("picodata")
                .arg("admin")
                .arg(self.socket_path.clone())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn();

            match picodata_admin {
                Ok(process) => {
                    info!("Successfully connected to picodata cluster.");
                    return Ok(process);
                }
                Err(_) => {
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }

    /// Method returns first running cluster instance
    pub fn main(&self) -> &PicodataInstance {
        self.instances()
            .first()
            .expect("Main server failed to start")
    }

    /// Method returns all running instances of cluster
    pub fn instances(&self) -> &Vec<PicodataInstance> {
        &self.instances
    }
}

pub fn run_pike<A, P>(args: Vec<A>, current_dir: P) -> Result<std::process::Child, Error>
where
    A: AsRef<OsStr>,
    P: AsRef<Path>,
{
    Command::new("cargo")
        .arg("pike")
        .args(args)
        .current_dir(current_dir)
        .spawn()
}
