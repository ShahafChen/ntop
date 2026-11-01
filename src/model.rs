//! Platform-neutral data model. Backends in `platform/` fill these structs; the UI only reads them.

use std::cmp::Ordering;

#[derive(Default, Clone)]
pub struct SystemStats {
    pub hostname: String,
    pub uptime_secs: u64,
    pub load_avg: [f64; 3],
    pub cpu_pct: f64,
    pub cpu_cores: usize,
    pub mem_total: u64,
    pub mem_used: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub net_rx_bps: f64,
    pub net_tx_bps: f64,
    pub disk_read_bps: f64,
    pub disk_write_bps: f64,
}

#[derive(Default, Clone)]
pub struct Process {
    pub pid: u32,
    pub user: String,
    pub state: char,
    pub name: String,
    pub cmdline: String,
    /// Share of total machine CPU capacity (all cores = 100%), like Windows Task Manager.
    pub cpu_pct: f64,
    pub mem_bytes: u64,
    pub disk_read_bps: f64,
    pub disk_write_bps: f64,
    /// Docker container name when the process runs inside one.
    pub container: Option<String>,
}

#[derive(Default, Clone)]
pub struct Container {
    pub id: String,
    pub name: String,
    /// As reported by `docker stats`: percent of one core, may exceed 100.
    pub cpu_pct: f64,
    pub mem_bytes: u64,
    pub mem_limit: u64,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    pub blk_read_bytes: u64,
    pub blk_write_bytes: u64,
    pub pids: u32,
}

#[derive(Default, Clone)]
pub struct Snapshot {
    pub system: SystemStats,
    pub processes: Vec<Process>,
    pub containers: Vec<Container>,
    /// Why the container list is empty, if the backend could not get it.
    pub docker_error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Cpu,
    Mem,
    Disk,
    Net,
    Name,
    Pid,
    User,
}

impl SortKey {
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Cpu => "CPU",
            SortKey::Mem => "MEM",
            SortKey::Disk => "DISK",
            SortKey::Net => "NET",
            SortKey::Name => "NAME",
            SortKey::Pid => "PID",
            SortKey::User => "USER",
        }
    }

    /// Numeric columns show the biggest first; text columns A→Z.
    fn descending_by_default(self) -> bool {
        matches!(self, SortKey::Cpu | SortKey::Mem | SortKey::Disk | SortKey::Net)
    }
}

pub fn sort_processes(p: &mut [Process], key: SortKey, reverse: bool) {
    p.sort_by(|a, b| {
        let o = match key {
            SortKey::Cpu => a.cpu_pct.total_cmp(&b.cpu_pct),
            SortKey::Mem => a.mem_bytes.cmp(&b.mem_bytes),
            SortKey::Disk => (a.disk_read_bps + a.disk_write_bps).total_cmp(&(b.disk_read_bps + b.disk_write_bps)),
            SortKey::Net => Ordering::Equal, // no per-process network accounting on Linux
            SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortKey::Pid => a.pid.cmp(&b.pid),
            SortKey::User => a.user.cmp(&b.user),
        };
        o.then(a.pid.cmp(&b.pid))
    });
    if key.descending_by_default() ^ reverse {
        p.reverse();
    }
}

pub fn sort_containers(c: &mut [Container], key: SortKey, reverse: bool) {
    c.sort_by(|a, b| {
        let o = match key {
            SortKey::Cpu => a.cpu_pct.total_cmp(&b.cpu_pct),
            SortKey::Mem => a.mem_bytes.cmp(&b.mem_bytes),
            SortKey::Disk => (a.blk_read_bytes + a.blk_write_bytes).cmp(&(b.blk_read_bytes + b.blk_write_bytes)),
            SortKey::Net => (a.net_rx_bytes + a.net_tx_bytes).cmp(&(b.net_rx_bytes + b.net_tx_bytes)),
            SortKey::Pid => a.pids.cmp(&b.pids),
            SortKey::Name | SortKey::User => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        };
        o.then(a.id.cmp(&b.id))
    });
    if key.descending_by_default() ^ reverse {
        c.reverse();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_sort_is_descending_and_reversible() {
        let mut p: Vec<Process> = [1.0, 5.0, 3.0]
            .iter()
            .enumerate()
            .map(|(i, c)| Process { pid: i as u32, cpu_pct: *c, ..Default::default() })
            .collect();
        sort_processes(&mut p, SortKey::Cpu, false);
        assert_eq!(p.iter().map(|x| x.pid).collect::<Vec<_>>(), [1, 2, 0]);
        sort_processes(&mut p, SortKey::Cpu, true);
        assert_eq!(p.iter().map(|x| x.pid).collect::<Vec<_>>(), [0, 2, 1]);
        sort_processes(&mut p, SortKey::Pid, false);
        assert_eq!(p.iter().map(|x| x.pid).collect::<Vec<_>>(), [0, 1, 2]);
    }
}
