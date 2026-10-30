//! Network slots for independent clusters.
//!
//! Every [`Cluster`](crate::Cluster) owns a slot: a loopback address and
//! a port offset that no other cluster uses. This is what lets several
//! clusters run side by side, both inside a single test binary and across
//! test processes.

use anyhow::{bail, Context};
use log::{info, warn};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::path::{Path, PathBuf};

pub const BASE_BIN_PORT: u16 = 3000;
pub const BASE_HTTP_PORT: u16 = 8000;
pub const BASE_PG_PORT: u16 = 5432;

/// Topology variables that make Picodata listen on explicitly given addresses
/// instead of `host:(base port + instance number)`, paired with the base port.
///
/// Such addresses are chosen by the topology author, a slot has no say in them.
const LISTEN_VARS: [(&str, u16); 3] = [
    ("PICODATA_IPROTO_LISTEN", BASE_BIN_PORT),
    ("PICODATA_HTTP_LISTEN", BASE_HTTP_PORT),
    ("PICODATA_PG_LISTEN", BASE_PG_PORT),
];

const SLOTS_DIR_NAME: &str = "picotest-slots";

const LAST_LOOPBACK_HOST: u8 = 254;

const PORT_OFFSET_STEP: u16 = 100;
const PORT_OFFSETS_COUNT: u16 = 19;

/// Loopback address and port offset exclusively owned by one cluster.
///
/// The slot is held by a lock on a file in the system temporary directory,
/// so it's released when the slot is dropped or the process dies.
#[derive(Debug)]
pub struct ClusterSlot {
    host: Ipv4Addr,
    port_offset: u16,
    _lock: File,
}

impl ClusterSlot {
    /// Acquires the first free slot for a cluster of `instances_count` instances.
    ///
    /// Loopback addresses `127.0.0.1..=127.0.0.254` with default ports are
    /// tried first. If none of them is free, shifted port ranges on
    /// `127.0.0.1` are tried. A slot is free if the ports of every instance
    /// of the cluster are free.
    ///
    /// If a `PICODATA_*_LISTEN` variable is set in the topology environment,
    /// Picodata listens on the address from the topology, and the slot has no say in it.
    /// Such a cluster is pinned to `127.0.0.1` with default ports, the address topologies
    /// usually use, and fails to start if that slot is taken.
    pub fn acquire(
        instances_count: u16,
        environment: &BTreeMap<String, String>,
    ) -> anyhow::Result<Self> {
        let (overridden, checked): (Vec<_>, Vec<_>) = LISTEN_VARS
            .into_iter()
            .partition(|(var, _)| environment.contains_key(*var));
        let base_ports = checked
            .into_iter()
            .map(|(_, base_port)| base_port)
            .collect::<Vec<_>>();
        let pinned = !overridden.is_empty();
        if pinned {
            warn!(
                "{:?} set in the topology: Picodata listens on addresses from the topology, \
                 so the cluster is pinned to {} with default ports \
                 and can't run side by side with another one",
                overridden.iter().map(|(var, _)| var).collect::<Vec<_>>(),
                Ipv4Addr::LOCALHOST
            );
        }

        // The topology chooses the addresses and the slot doesn't know them,
        // so the cluster can't move to another address.
        let candidates = if pinned {
            vec![(Ipv4Addr::LOCALHOST, 0)]
        } else {
            candidates()
        };

        Self::acquire_in(
            &std::env::temp_dir().join(SLOTS_DIR_NAME),
            candidates,
            |host, port_offset| is_free(host, port_offset, instances_count, &base_ports),
        )
    }

    fn acquire_in<F>(
        slots_dir: &Path,
        candidates: Vec<(Ipv4Addr, u16)>,
        is_free: F,
    ) -> anyhow::Result<Self>
    where
        F: Fn(Ipv4Addr, u16) -> bool,
    {
        fs::create_dir_all(slots_dir)
            .with_context(|| format!("Failed to create directory '{}'", slots_dir.display()))?;

        for (host, port_offset) in candidates {
            let lock_path = slots_dir.join(format!("{host}+{port_offset}.lock"));
            let Some(lock) = try_lock_file(&lock_path)? else {
                continue;
            };
            if !is_free(host, port_offset) {
                continue;
            }

            info!("Acquired cluster slot: host {host}, port offset {port_offset}");
            return Ok(Self {
                host,
                port_offset,
                _lock: lock,
            });
        }

        bail!("no free loopback address or port range left for a new cluster")
    }

    /// Address every instance of the cluster listens on.
    pub fn host(&self) -> Ipv4Addr {
        self.host
    }

    pub fn base_bin_port(&self) -> u16 {
        BASE_BIN_PORT + self.port_offset
    }

    pub fn base_http_port(&self) -> u16 {
        BASE_HTTP_PORT + self.port_offset
    }

    pub fn base_pg_port(&self) -> u16 {
        BASE_PG_PORT + self.port_offset
    }
}

/// Slots to try, in order.
fn candidates() -> Vec<(Ipv4Addr, u16)> {
    let hosts = (1..=LAST_LOOPBACK_HOST).map(|octet| (Ipv4Addr::new(127, 0, 0, octet), 0));
    let port_offsets =
        (1..=PORT_OFFSETS_COUNT).map(|n| (Ipv4Addr::LOCALHOST, n * PORT_OFFSET_STEP));

    hosts.chain(port_offsets).collect()
}

/// Opens (creating if needed) and exclusively locks a file.
///
/// Returns `None` if the file is locked by someone else, and an error if locking failed.
pub(crate) fn try_lock_file(path: &PathBuf) -> anyhow::Result<Option<File>> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .with_context(|| format!("Failed to open lock file '{}'", path.display()))?;

    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(err)) => {
            Err(err).with_context(|| format!("Failed to lock file '{}'", path.display()))
        }
    }
}

/// Checks that nobody listens on the ports of any of the cluster instances.
///
/// Instances are numbered from 1 and take `base port + instance number`.
/// Covers what file locks can't see: an address that is not configured
/// on the machine, a cluster started by hand or left over by a killed
/// test run.
fn is_free(host: Ipv4Addr, port_offset: u16, instances_count: u16, base_ports: &[u16]) -> bool {
    base_ports
        .iter()
        .flat_map(|base_port| (1..=instances_count).map(move |id| base_port + port_offset + id))
        .all(|port| TcpListener::bind(SocketAddrV4::new(host, port)).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmp_dir;

    fn slots_dir() -> PathBuf {
        std::env::temp_dir().join(tmp_dir().file_name().unwrap())
    }

    #[test]
    fn slots_do_not_overlap() {
        let dir = slots_dir();

        let first = ClusterSlot::acquire_in(&dir, candidates(), |_, _| true).unwrap();
        let second = ClusterSlot::acquire_in(&dir, candidates(), |_, _| true).unwrap();

        assert_eq!(first.host(), Ipv4Addr::new(127, 0, 0, 1));
        assert_eq!(second.host(), Ipv4Addr::new(127, 0, 0, 2));
        assert_eq!(first.base_pg_port(), second.base_pg_port());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn slot_is_released_on_drop() {
        let dir = slots_dir();

        let first = ClusterSlot::acquire_in(&dir, candidates(), |_, _| true).unwrap();
        drop(first);
        let second = ClusterSlot::acquire_in(&dir, candidates(), |_, _| true).unwrap();

        assert_eq!(second.host(), Ipv4Addr::LOCALHOST);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn busy_addresses_are_skipped() {
        let dir = slots_dir();

        let slot =
            ClusterSlot::acquire_in(&dir, candidates(), |host, _| host.octets()[3] >= 3).unwrap();

        assert_eq!(slot.host(), Ipv4Addr::new(127, 0, 0, 3));

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn falls_back_to_port_offsets_on_single_loopback_address() {
        let dir = slots_dir();
        let only_localhost = |host, _| host == Ipv4Addr::LOCALHOST;

        let first = ClusterSlot::acquire_in(&dir, candidates(), only_localhost).unwrap();
        let second = ClusterSlot::acquire_in(&dir, candidates(), only_localhost).unwrap();

        assert_eq!(first.host(), second.host());
        assert_ne!(first.base_bin_port(), second.base_bin_port());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ports_of_all_instances_and_listed_protocols_are_checked() {
        let host = Ipv4Addr::LOCALHOST;
        let offset = 1500;
        // Busy: bin port of the third instance and pg port of the first one.
        let _bin = TcpListener::bind(SocketAddrV4::new(host, BASE_BIN_PORT + offset + 3)).unwrap();
        let _pg = TcpListener::bind(SocketAddrV4::new(host, BASE_PG_PORT + offset + 1)).unwrap();

        // A busy port is noticed only if its instance and protocol are checked.
        assert!(is_free(host, offset, 2, &[BASE_BIN_PORT, BASE_HTTP_PORT]));
        assert!(!is_free(host, offset, 3, &[BASE_BIN_PORT]));
        assert!(!is_free(host, offset, 1, &[BASE_PG_PORT]));
    }

    #[test]
    fn pinned_cluster_does_not_move_to_another_address() {
        let dir = slots_dir();

        let first = ClusterSlot::acquire_in(&dir, vec![(Ipv4Addr::LOCALHOST, 0)], |_, _| true);
        let second = ClusterSlot::acquire_in(&dir, vec![(Ipv4Addr::LOCALHOST, 0)], |_, _| true);

        assert_eq!(first.unwrap().host(), Ipv4Addr::LOCALHOST);
        assert!(second.is_err());

        fs::remove_dir_all(dir).unwrap();
    }
}
