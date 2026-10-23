//! Network slots for independent clusters.
//!
//! Every [`Cluster`](crate::Cluster) owns a slot: a loopback address and
//! a port offset that no other cluster uses. This is what lets several
//! clusters run side by side, both inside a single test binary and across
//! test processes.

use anyhow::{bail, Context};
use log::debug;
use std::fs::{self, File, OpenOptions};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::path::{Path, PathBuf};

pub const BASE_BIN_PORT: u16 = 3000;
pub const BASE_HTTP_PORT: u16 = 8000;
pub const BASE_PG_PORT: u16 = 5432;

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
    /// Acquires the first free slot.
    ///
    /// Loopback addresses `127.0.0.1..=127.0.0.254` with default ports are
    /// tried first. If none of them is free, shifted port ranges on
    /// `127.0.0.1` are tried.
    pub fn acquire() -> anyhow::Result<Self> {
        Self::acquire_in(&std::env::temp_dir().join(SLOTS_DIR_NAME), is_free)
    }

    fn acquire_in<F>(slots_dir: &Path, is_free: F) -> anyhow::Result<Self>
    where
        F: Fn(Ipv4Addr, u16) -> bool,
    {
        fs::create_dir_all(slots_dir)
            .with_context(|| format!("Failed to create directory '{}'", slots_dir.display()))?;

        for (host, port_offset) in candidates() {
            let lock_path = slots_dir.join(format!("{host}+{port_offset}.lock"));
            let Some(lock) = try_lock_file(&lock_path)? else {
                continue;
            };
            if !is_free(host, port_offset) {
                continue;
            }

            debug!("Acquired cluster slot: host {host}, port offset {port_offset}");
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

fn candidates() -> impl Iterator<Item = (Ipv4Addr, u16)> {
    let hosts = (1..=LAST_LOOPBACK_HOST).map(|octet| (Ipv4Addr::new(127, 0, 0, octet), 0));
    let port_offsets =
        (1..=PORT_OFFSETS_COUNT).map(|n| (Ipv4Addr::LOCALHOST, n * PORT_OFFSET_STEP));

    hosts.chain(port_offsets)
}

/// Opens (creating if needed) and exclusively locks a file.
///
/// Returns `None` if the file is locked by someone else.
pub(crate) fn try_lock_file(path: &PathBuf) -> anyhow::Result<Option<File>> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .with_context(|| format!("Failed to open lock file '{}'", path.display()))?;

    Ok(file.try_lock().is_ok().then_some(file))
}

/// Checks that nobody listens on the ports of the first cluster instance.
///
/// Covers what file locks can't see: an address that is not configured
/// on the machine, a cluster started by hand or left over by a killed
/// test run.
fn is_free(host: Ipv4Addr, port_offset: u16) -> bool {
    [BASE_BIN_PORT, BASE_HTTP_PORT, BASE_PG_PORT]
        .into_iter()
        .all(|base_port| {
            TcpListener::bind(SocketAddrV4::new(host, base_port + port_offset + 1)).is_ok()
        })
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

        let first = ClusterSlot::acquire_in(&dir, |_, _| true).unwrap();
        let second = ClusterSlot::acquire_in(&dir, |_, _| true).unwrap();

        assert_eq!(first.host(), Ipv4Addr::new(127, 0, 0, 1));
        assert_eq!(second.host(), Ipv4Addr::new(127, 0, 0, 2));
        assert_eq!(first.base_pg_port(), second.base_pg_port());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn slot_is_released_on_drop() {
        let dir = slots_dir();

        let first = ClusterSlot::acquire_in(&dir, |_, _| true).unwrap();
        drop(first);
        let second = ClusterSlot::acquire_in(&dir, |_, _| true).unwrap();

        assert_eq!(second.host(), Ipv4Addr::LOCALHOST);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn busy_addresses_are_skipped() {
        let dir = slots_dir();

        let slot = ClusterSlot::acquire_in(&dir, |host, _| host.octets()[3] >= 3).unwrap();

        assert_eq!(slot.host(), Ipv4Addr::new(127, 0, 0, 3));

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn falls_back_to_port_offsets_on_single_loopback_address() {
        let dir = slots_dir();
        let only_localhost = |host, _| host == Ipv4Addr::LOCALHOST;

        let first = ClusterSlot::acquire_in(&dir, only_localhost).unwrap();
        let second = ClusterSlot::acquire_in(&dir, only_localhost).unwrap();

        assert_eq!(first.host(), Ipv4Addr::LOCALHOST);
        assert_eq!(first.base_bin_port(), BASE_BIN_PORT);
        assert_eq!(second.host(), Ipv4Addr::LOCALHOST);
        assert_eq!(second.base_bin_port(), BASE_BIN_PORT + PORT_OFFSET_STEP);
        assert_eq!(second.base_http_port(), BASE_HTTP_PORT + PORT_OFFSET_STEP);
        assert_eq!(second.base_pg_port(), BASE_PG_PORT + PORT_OFFSET_STEP);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn fails_when_nothing_is_free() {
        let dir = slots_dir();

        assert!(ClusterSlot::acquire_in(&dir, |_, _| false).is_err());

        fs::remove_dir_all(dir).unwrap();
    }
}
