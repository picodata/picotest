use anyhow::Context;
use log::{debug, info, trace, warn};
use pike::cluster::{PicodataInstance, RunParamsBuilder, StopParamsBuilder, Topology};
use rand::distributions::Alphanumeric;
use rand::Rng;
use std::ffi::OsStr;
use std::fmt::Debug;
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

#[cfg(test)]
mod tests;

const SOCKET_PATH: &str = "cluster/i1/admin.sock";
const TOPOLOGY_FILENAME: &str = "topology.toml";
const TARGET_DIR: &str = "target/debug";

const MANIFEST_FILENAME: &str = "manifest.yaml";
const MANIFEST_FILENAME_BACKUP: &str = "manifest.backup.yaml";
const MANIFEST_SERVICES: &str = "services";
const MANIFEST_SERVICE_NAME: &str = "name";

fn parse_topology(path: &PathBuf) -> anyhow::Result<Topology> {
    toml::from_str(
        &fs::read_to_string(path).context(format!("Failed to read file '{}'", path.display()))?,
    )
    .context(format!(
        "Failed to parse topology TOML from path '{}'",
        path.display()
    ))
}

fn parse_yaml<T: AsRef<Path>>(path: &T) -> anyhow::Result<serde_yaml::Value> {
    let reader = fs::File::open(path).context("Failed to open YAML file")?;
    serde_yaml::from_reader(reader).context("Failed to parse YAML content")
}

pub struct Cluster {
    pub uuid: Uuid,
    pub plugin_path: PathBuf,
    pub data_dir: PathBuf,
    pub timeout: Duration,
    socket_path: PathBuf,
    topology: Topology,
    instances: Option<Vec<PicodataInstance>>,
}

impl Drop for Cluster {
    fn drop(&mut self) {
        if let Err(err) = self.restore_manifest_file() {
            warn!("Failed to restore plugin manifest file: {err}");
        }
        if self.instances.is_none() {
            return;
        }
        if let Err(err) = self.stop() {
            warn!("Failed to stop picodata cluster: {err}");
        }
    }
}

impl Cluster {
    pub fn new(plugin_path: &str, data_dir: &str, timeout: Duration) -> anyhow::Result<Self> {
        let plugin_path = PathBuf::from(plugin_path);
        let data_dir = PathBuf::from(data_dir);
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
            instances: None,
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

    pub fn run(mut self) -> anyhow::Result<Self> {
        let params = RunParamsBuilder::default()
            .plugin_path(self.plugin_path.clone())
            .data_dir(self.data_dir.clone())
            .topology(self.topology.clone())
            .use_release(false)
            .build()?;

        debug!("Starting the cluster with parameters {params:?}");
        let intances: Vec<PicodataInstance> = pike::cluster::run(&params)?;
        self.instances.replace(intances);
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

    fn plugin_manifest_dir(&self) -> PathBuf {
        let (plugin_name, plugin_info) = self
            .topology
            .plugins
            .first_key_value()
            .expect("Topology should define at least 1 plugin");

        // Use 0.1.0 as default plugin version unless specified in topology.
        let plugin_version = plugin_info.version.as_ref().map_or("0.1.0", |v| v);

        self.plugin_path
            .join(TARGET_DIR)
            .join(plugin_name)
            .join(plugin_version)
    }

    fn plugin_manifest_path(&self) -> PathBuf {
        self.plugin_manifest_dir().join(MANIFEST_FILENAME)
    }

    fn plugin_manifest_backup_path(&self) -> PathBuf {
        self.plugin_manifest_dir().join(MANIFEST_FILENAME_BACKUP)
    }

    fn backup_manifest_file(&self) -> anyhow::Result<()> {
        let manifest_path = self.plugin_manifest_path();
        let manifest_backup_path = self.plugin_manifest_backup_path();

        std::fs::copy(manifest_path, manifest_backup_path)
            .context("Failed to back up plugin manifest file")?;

        Ok(())
    }

    fn restore_manifest_file(&self) -> anyhow::Result<()> {
        let manifest_path = self.plugin_manifest_path();
        let manifest_backup_path = self.plugin_manifest_backup_path();

        if fs::metadata(&manifest_backup_path).is_ok() {
            std::fs::copy(&manifest_backup_path, &manifest_path)
                .context("Failed to restore plugin manifest file from backup")?;

            std::fs::remove_file(manifest_backup_path)
                .context("Failed to remove backup manifest file")?;
        }

        Ok(())
    }
}

/// Spin up Picodata cluster
///
/// # Arguments
///
/// - `plugin_path` - path to the plugin directory.
/// - `plugin_config_path` - path to the plugin configuration.
///                          If `None`, default plugin configuration is used.
/// - `timeout_secs` - amount of seconds to be waited right after cluster is started.
///
/// # Returns
///
/// A `Result` containing either up and running `Cluster` or instance of [`anyhow::Error`]
///
pub fn run_cluster(
    plugin_path: &str,
    plugin_config_path: Option<&str>,
    timeout_secs: u64,
) -> anyhow::Result<Cluster> {
    let data_dir = tmp_dir();
    let timeout = Duration::from_secs(timeout_secs);
    let cluster = Cluster::new(plugin_path, &data_dir, timeout)?;

    if let Some(plugin_config_path) = plugin_config_path {
        cluster
            .backup_manifest_file()
            .context("Failed to create backup manifest file")?;

        apply_plugin_configuration(plugin_config_path, cluster.plugin_manifest_path())
            .context("Failed to apply plugin configuration")?;
    }

    cluster.run()
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

pub fn tmp_dir() -> String {
    let mut rng = rand::thread_rng();
    format!(
        "tmp/tests/{}",
        (0..8)
            .map(|_| rng.sample(Alphanumeric))
            .map(char::from)
            .collect::<String>()
    )
}

/// Applies input plugin configuration to the manifest file.
///
/// # Arguments
///
/// - `plugin_config_path` - path to the YAML plugin configuration to be applied.
/// - `manifest_path` - path to the plugin manifest YAML file.
///
fn apply_plugin_configuration<P, M>(plugin_config_path: P, manifest_path: M) -> anyhow::Result<()>
where
    P: AsRef<Path> + Debug,
    M: AsRef<Path> + Debug,
{
    info!(
        "Applying plugin configuration from {:?} to manifest {:?}",
        plugin_config_path, manifest_path
    );

    let plugin_config = parse_yaml(&plugin_config_path)?;
    let mut plugin_manifest = parse_yaml(&manifest_path)?;

    replace_services_configuration(&plugin_config, &mut plugin_manifest)
        .context("Failed to replace plugin services configuration(s)")?;

    let writer = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(manifest_path)
        .expect("Failed to open plugin manifest to write");

    serde_yaml::to_writer(writer, &plugin_manifest).context("Failed to write plugin manifest")
}

/// Iterate over services in manifest yaml and replace their default configuration
/// with corresponding config found in provided plugin configuration.
///
/// Changes is applied in-place, so the result will be in `plugin_manifest` argument.
///
/// # Arguments
///
/// - `plugin_config` - yaml-formatted plugin configuration to be applied.
/// - `plugin_manifest` - yaml-formatted manifest of the plugin.
///
fn replace_services_configuration(
    plugin_config: &serde_yaml::Value,
    plugin_manifest: &mut serde_yaml::Value,
) -> anyhow::Result<()> {
    let manifest_services = plugin_manifest
        .get_mut(MANIFEST_SERVICES)
        .context("Failed to get services mapping from plugin manifest")?;

    for service in manifest_services
        .as_sequence_mut()
        .expect("Should be a sequence of services")
    {
        let service_name = service
            .get(MANIFEST_SERVICE_NAME)
            .context("Failed to get name of the service from plugin manifest")?;

        let Some(service_configuration) = plugin_config.get(service_name) else {
            debug!("Service {service_name:?} was not found in provided plugin configuration");
            continue;
        };

        trace!(
            "Replacing configuration of service {:?} to {:?}",
            service_name,
            service_configuration
        );

        let service_default_configuration = service
            .get_mut("default_configuration")
            .context("Failed to get service default configuration from plugin manifest")?;

        *service_default_configuration = service_configuration.clone();
    }

    Ok(())
}
