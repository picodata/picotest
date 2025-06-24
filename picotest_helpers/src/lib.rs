use anyhow::{bail, Context};
use bytes::Bytes;
use log::{debug, info, warn};
use pike::cluster::{
    PicodataInstance, PicodataInstanceProperties, RunParamsBuilder, StopParamsBuilder, Topology,
};
use pike::config::ApplyParamsBuilder;
use rand::distr::Alphanumeric;
use rand::Rng;
use rmpv::Value;
use rusty_tarantool::tarantool::{ClientConfig, ExecWithParamaters, TarantoolResponse};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::BTreeMap;
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
use topology::PluginTopology;
use uuid::Uuid;

pub mod migration;
pub mod topology;

pub type PluginConfigMap = pike::config::PluginConfigMap;

const ADMIN_SOCKET_NAME: &str = "admin.sock";
const LOCALHOST_IP: &str = "127.0.0.1";
pub const PICOTEST_USER: &str = "Picotest";
pub const PICOTEST_USER_IPROTO: &str = "PicotestBin";
pub const PICOTEST_USER_PASSWORD: &str = "Pic0test";

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

pub struct PicotestInstance {
    inner: PicodataInstance,
    pub socket_path: PathBuf,
    pub bin_port: u16,
    pub pg_port: u16,
    pub http_port: u16,
    pub instance_name: String,
    pub tier: String,
    pub instance_id: u16,
}

impl From<(PicodataInstance, &PathBuf)> for PicotestInstance {
    fn from((instance, data_dir): (PicodataInstance, &PathBuf)) -> Self {
        let properties = instance.properties();
        let instance_name = properties.instance_name;
        let socket_path = data_dir
            .join("cluster")
            .join(instance_name)
            .join(ADMIN_SOCKET_NAME);
        PicotestInstance {
            bin_port: *properties.bin_port,
            pg_port: *properties.pg_port,
            http_port: *properties.http_port,
            instance_name: instance_name.to_string(),
            tier: properties.tier.to_string(),
            instance_id: *properties.instance_id,
            inner: instance,
            socket_path,
        }
    }
}

impl PicotestInstance {
    #[deprecated(
        since = "1.2.2",
        note = "You can access the field directly with .pg_port"
    )]
    pub fn pg_port(&self) -> &u16 {
        &self.pg_port
    }

    pub fn properties(&self) -> PicodataInstanceProperties<'_> {
        self.inner.properties()
    }

    pub fn inner(&self) -> &PicodataInstance {
        &self.inner
    }

    pub async fn execute_rpc<S, G>(
        &self,
        plugin_name: &str,
        path: &str,
        service_name: &str,
        plugin_version: &str,
        input: &S,
    ) -> anyhow::Result<G>
    where
        G: DeserializeOwned,
        S: Serialize,
    {
        let bin_port = self.bin_port;
        let client = ClientConfig::new(
            format!("{LOCALHOST_IP}:{bin_port}"),
            PICOTEST_USER_IPROTO,
            PICOTEST_USER_PASSWORD,
        )
        .build();

        let input_encoded =
            rmp_serde::encode::to_vec_named(input).context("failed to encode input to msgpack")?;

        // In beloved Picodata, the rpc request args have custom serialisation function
        // See: https://github.com/picodata/picodata/blob/1e89dd6a4634f3a8be065fadaa522b2f37d3719c/picodata-plugin/src/transport/context.rs#L167

        let mut context_map = BTreeMap::new();
        let request_id_bytes = Uuid::new_v4().as_bytes().to_vec();
        context_map.insert(1, Value::Ext(2, request_id_bytes));
        context_map.insert(2, Value::String(plugin_name.into()));
        context_map.insert(3, Value::String(service_name.into()));
        context_map.insert(4, Value::String(plugin_version.into()));

        let response: TarantoolResponse = client
            .prepare_fn_call(".proc_rpc_dispatch")
            .bind(path)?
            .bind(Bytes::copy_from_slice(&input_encoded))?
            .bind_ref(&context_map)?
            .execute()
            .await
            .context("Rpc calls should not fail")?;

        if response.code != 0 {
            bail!("Rpc calls should not fail");
        }

        // RustyTarantool library uses binary protocol, thus the return value from RPC is
        // encoded to MsgPack twice. First layer is an array of binary data.
        let response: Vec<rmpv::Value> = rmp_serde::from_slice(response.data.as_ref())
            .context("Failed to deserialise rpc response")?;
        let Value::Binary(response_bin) = &response[0] else {
            bail!("Expected to recieve binary input")
        };

        // Second layer is the struct itself
        let response_decoded: G =
            rmp_serde::from_slice(response_bin).context("Failed to deserialise rpc response")?;

        Ok(response_decoded)
    }

    /// Executes an SQL query through the picodata admin console.
    ///
    /// # Workflow
    /// 1. Establishes connection with the admin console (`await_picodata_admin`)
    /// 2. Writes the query to the process's stdin
    /// 3. Reads the result from stdout, skipping the first 2 lines (typically headers)
    /// 4. Terminates the process after receiving the result
    ///
    /// # Arguments
    /// * `query` - SQL query as a byte slice or convertible type
    ///
    /// # Return Value
    /// `Result<String, Error>` where:
    /// * `Ok(String)` - query execution result
    /// * `Err(Error)` - I/O or execution error
    ///
    /// # Examples
    /// ```rust,ignore
    /// use picotest::*;
    ///
    /// #[picotest]
    /// fn run_sql_query() {
    ///     let result = cluster.instances[0].run_query("SELECT * FROM users").unwrap();
    ///     println!("{}", result);
    /// }
    /// ```
    pub fn run_query<T: AsRef<[u8]>>(&self, query: T) -> Result<String, Error> {
        let mut picodata_admin = self.await_picodata_admin()?;

        let stdout = picodata_admin
            .stdout
            .take()
            .expect("Failed to capture stdout");
        {
            let picodata_stdin = picodata_admin.stdin.as_mut().unwrap();
            picodata_stdin.write_all(query.as_ref())?;
            picodata_admin.wait()?;
        }

        let mut result = String::new();
        let reader = BufReader::new(stdout);
        for line in reader.lines().skip(2) {
            match line {
                Ok(l) => result.push_str(&l),
                Err(e) => return Err(e),
            }
        }
        picodata_admin.kill()?;

        Ok(result)
    }

    /// Executes Lua script through picodata's query mechanism.
    ///
    /// Prepends `\lua\n` to the query and passes it to `run_query`.
    ///
    /// # Arguments
    /// * `query` - Lua code as a byte slice or convertible type
    ///
    /// # Return Value
    /// `Result<String, Error>` where:
    /// * `Ok(String)` - script execution result
    /// * `Err(Error)` - execution error (inherited from `run_query`)
    ///
    /// # Examples
    /// ```rust,ignore
    /// use picotest::*;
    ///
    /// #[picotest]
    /// fn test_run_lua_query() {
    ///     let res = cluster.instances()[1].run_lua("return 1 + 1")?;
    ///     assert!(res.contains("2"));
    /// }
    /// ```
    pub fn run_lua<T: AsRef<[u8]>>(&self, query: T) -> Result<String, Error> {
        self.run_query([b"\\lua\n", query.as_ref()].concat())
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
}

pub struct Cluster {
    pub uuid: Uuid,
    pub plugin_path: PathBuf,
    pub data_dir: PathBuf,
    pub timeout: Duration,
    topology: Topology,
    instances: Vec<PicotestInstance>,
}

impl Drop for Cluster {
    fn drop(&mut self) {
        if let Err(err) = self.stop() {
            warn!("Failed to stop picodata cluster: {err}");
        }
    }
}

impl Cluster {
    pub fn new(
        plugin_path: PathBuf,
        topology: PluginTopology,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        let data_dir = tmp_dir();

        if let Err(err) = fs::remove_dir_all(plugin_path.join(data_dir.parent().unwrap())) {
            warn!("Failed to remove cluster data directory: {err}");
        }

        let cluster = Self {
            uuid: Uuid::new_v4(),
            plugin_path,
            data_dir,
            timeout,
            topology,
            instances: Default::default(),
        };

        Ok(cluster)
    }

    pub fn data_dir_path(&self) -> PathBuf {
        self.plugin_path.join(self.data_dir.clone())
    }

    pub fn stop(&self) -> anyhow::Result<()> {
        let params = StopParamsBuilder::default()
            .plugin_path(self.plugin_path.clone())
            .data_dir(self.data_dir.clone())
            .build()?;

        debug!("Stopping the cluster with parameters {params:?}");
        pike::cluster::stop(&params)?;

        Ok(())
    }

    /// Applies passed plugin config to the running cluster through the interface of command
    /// "[pike config apply](https://github.com/picodata/pike?tab=readme-ov-file#config-apply)".
    ///
    /// ### Arguments:
    ///
    /// - `config` - mapping of plugin services to their values.
    ///   This structure should be able to deserialize into [`PluginConfigMap`].
    ///
    /// ### Returns
    ///
    /// - On sucess, returns nothing.
    /// - On failure, instance of [`anyhow::Result`].
    ///
    /// **Note:** your cluster logs may contain messages related to status of config applying.
    /// E.g., if some fields are erroneously serialized, Picodata plugin environment will
    /// throw a descriptive error during config validation saying what went wrong.
    ///
    /// Thus, this routine will raise an error if something goes wrong, but details of an error
    /// rather be found in cluster logs.
    ///
    /// ### Examples:
    ///
    /// #### Plugin with single service called `router`.
    ///
    /// Assume [plugin YAML configuration file](https://github.com/picodata/pike?tab=readme-ov-file#config-apply)
    /// has the following mapping:
    ///
    /// ```yaml
    /// router:
    ///     rpc_endpoint: "/hello"
    ///     max_rpc_message_size_bytes: 1024
    ///     max_rpc_message_queue_size: 2048
    /// ```
    ///
    /// Our integration test will override all fields of service configuration
    /// and apply new config through [`Cluster::apply_config`] routine.
    ///
    /// There're several approaches of assembling [`PluginConfigMap`] that is passed into the routine.
    ///
    /// **1. Using YAML formatted string.**
    ///
    /// Put desired config inside string and deserialize it using [`serde_yaml::from_str`]. Then attach it to
    /// a particular plugin service.
    ///
    /// ```rust,ignore
    /// use rmpv::Value;
    /// use std::collections::HashMap;
    /// use serde_yaml::Value;
    /// use picotest::*;
    ///
    /// #[picotest]
    /// fn test_apply_plugin_from_yaml_string() {
    ///     // 1. Assemble YAML string.
    ///
    ///     let plugin_config_yaml = r#"
    ///         router:
    ///             rpc_endpoint: "/test"
    ///             max_rpc_message_size_bytes: 128
    ///             max_rpc_message_queue_size: 32
    ///         "#;
    ///
    ///     let plugin_config: PluginConfigMap =
    ///         serde_yaml::from_str(plugin_config_yaml).unwrap();
    ///
    ///     // 2. Apply config to the running cluster instance.
    ///
    ///     cluster // implicitly created variable by picotest magic
    ///         .apply_config(plugin_config)
    ///         .expect("Failed to apply config");
    ///
    ///     // Callback Serivce::on_config_change should've been already
    ///     // called at this point.
    /// }
    /// ```
    ///
    /// **2. Using `HashMap`.
    ///
    /// Config can be assembled by means of [`std::collections::HashMap`] and [`serde_yaml::to_str`].
    ///
    /// ```rust,ignore
    /// use rmpv::Value;
    /// use std::collections::HashMap;
    /// use serde_yaml::Value;
    /// use picotest::*;
    ///
    /// #[picotest]
    /// fn test_apply_plugin_from_hashmap() {
    ///     // 1. Override properties of the "router" service.
    ///
    ///     let router_config = HashMap::from([
    ///         ("rpc_endpoint".to_string(), Value::String("/test".into())),
    ///         ("max_rpc_message_size_bytes".to_string(), Value::Number(128.into())),
    ///         ("max_rpc_message_queue_size".to_string(), Value::Number(32.into())),
    ///     ]);
    ///
    ///     // 2. "Attach" overridden properties to the service "router".
    ///
    ///     let plugin_config = HashMap::from([("router".into(), router_config)]);
    ///
    ///     // 3. Apply config to the running cluster instance.
    ///
    ///     cluster // implicitly created variable by picotest magic
    ///         .apply_config(plugin_config)
    ///         .expect("Failed to apply config");
    ///
    ///     // Callback Serivce::on_config_change should've been already
    ///     // called at this point.
    /// }
    /// ```
    ///
    /// #### Plugin with single service called `router`, which has nested properties for RPC machinery.
    ///
    /// Nested sections in config mapping are handled similarly. We just need to wrap nested map value into [`serde_yaml::Value`].
    ///
    /// Assume [plugin YAML configuration file](https://github.com/picodata/pike?tab=readme-ov-file#config-apply)
    /// has the following mapping:
    ///
    /// ```yaml
    /// router:
    ///  rpc:
    ///   endpoint: "/hello"
    ///   max_message_size_bytes: 1024
    ///   max_message_queue_size: 2048
    /// ```
    ///
    /// Out integration test will look like:
    ///
    /// ```rust,ignore
    /// use rmpv::Value;
    /// use std::collections::HashMap;
    /// use serde_yaml::Value;
    /// use picotest::*;
    ///
    /// #[picotest]
    /// fn test_apply_plugin_nested_config() {
    ///     // Override properties of the RPC machinery.
    ///     let rpc_config = HashMap::from([
    ///         ("endpoint".to_string(), Value::String("/test".into())),
    ///         ("max_message_size_bytes".to_string(), Value::Number(128.into())),
    ///         ("max_message_queue_size".to_string(), Value::Number(32.into())),
    ///     ]);
    ///
    ///     let router_config = HashMap::from([("rpc".to_string(), serde_yaml::to_value(rpc_config).unwrap())]);
    ///     let plugin_config = HashMap::from([("router".to_string(), router_config)]);
    ///
    ///     cluster // implicitly created variable by picotest magic
    ///         .apply_config(plugin_config)
    ///         .expect("Failed to apply config");
    /// }
    /// ```
    ///
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

        let data_dir = self.data_dir_path();

        debug!("Starting the cluster with parameters {params:?}");
        let mut instances: Vec<PicotestInstance> = pike::cluster::run(&params)?
            .into_iter()
            .map(|instance| PicotestInstance::from((instance, &data_dir)))
            .collect();

        debug_assert!(
            self.instances.is_empty(),
            "trying to replace already running cluster?"
        );
        std::mem::swap(&mut self.instances, &mut instances);

        self.wait()?;
        self.create_picotest_users();
        //wait user timeout
        thread::sleep(self.timeout);

        Ok(self)
    }

    pub fn recreate(self) -> anyhow::Result<Self> {
        self.stop()?;
        self.run()
    }

    fn wait(&self) -> anyhow::Result<()> {
        let timeout = Duration::from_secs(60);
        let start_time = Instant::now();

        debug!(
            "Awaiting of cluster readiness (timeout {}s)",
            timeout.as_secs()
        );

        loop {
            let mut picodata_admin: Child = self.main().await_picodata_admin()?;
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
                return Ok(());
            }

            thread::sleep(Duration::from_secs(10));
        }
    }

    /// Executes an SQL query through the picodata admin console.
    ///
    /// # Workflow
    /// 1. Establishes connection with the admin console (`await_picodata_admin`)
    /// 2. Writes the query to the process's stdin
    /// 3. Reads the result from stdout, skipping the first 2 lines (typically headers)
    /// 4. Terminates the process after receiving the result
    ///
    /// # Arguments
    /// * `query` - SQL query as a byte slice or convertible type
    ///
    /// # Return Value
    /// `Result<String, Error>` where:
    /// * `Ok(String)` - query execution result
    /// * `Err(Error)` - I/O or execution error
    ///
    /// # Examples
    /// ```rust,ignore
    /// use picotest::*;
    ///
    /// #[picotest]
    /// fn run_sql_query() {
    ///     let result = cluster.run_query("SELECT * FROM users").unwrap();
    ///     println!("{}", result);
    /// }
    /// ```
    pub fn run_query<T: AsRef<[u8]>>(&self, query: T) -> Result<String, Error> {
        self.main().run_query(query)
    }

    /// Executes Lua script through picodata's query mechanism.
    ///
    /// Prepends `\lua\n` to the query and passes it to `run_query`.
    ///
    /// # Arguments
    /// * `query` - Lua code as a byte slice or convertible type
    ///
    /// # Return Value
    /// `Result<String, Error>` where:
    /// * `Ok(String)` - script execution result
    /// * `Err(Error)` - execution error (inherited from `run_query`)
    ///
    /// # Examples
    /// ```rust,ignore
    /// use picotest::*;
    ///
    /// #[picotest]
    /// fn test_run_lua_query() {
    ///     let res = cluster.instances()[1].run_lua("return 1 + 1")?;
    ///     assert!(res.contains("2"));
    /// }
    /// ```
    pub fn run_lua<T: AsRef<[u8]>>(&self, query: T) -> Result<String, Error> {
        self.main().run_lua(query)
    }

    /// Method returns first running cluster instance
    pub fn main(&self) -> &PicotestInstance {
        self.instances()
            .first()
            .expect("Main server failed to start")
    }

    /// Method returns all running instances of cluster
    pub fn instances(&self) -> &Vec<PicotestInstance> {
        &self.instances
    }

    // Create two users for pgproto and iproto with different password encryption
    fn create_picotest_users(&self) {
        for (user, auth_method) in [(PICOTEST_USER, "md5"), (PICOTEST_USER_IPROTO, "chap-sha1")] {
            self.run_query(format!(
                r#"CREATE USER "{user}" with password '{PICOTEST_USER_PASSWORD}' using {auth_method};"#
            ))
            .expect("Picotest user create should not fail");

            self.run_query(format!(r#"GRANT CREATE TABLE TO "{user}""#))
                .expect("Picotest user grant should not fail");

            self.run_query(format!(r#"GRANT READ TABLE TO "{user}""#))
                .expect("Picotest user grant should not fail");

            self.run_query(format!(r#"GRANT WRITE TABLE TO "{user}""#))
                .expect("Picotest user grant should not fail");
        }
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
