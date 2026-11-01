//! Platform-neutral data model. Backends in `platform/` fill these structs; the UI only reads them.

#[derive(Default, Clone, Debug)]
pub struct Gpu {
    pub name: String,
    pub util_pct: f64,
    pub mem_used: u64,
    pub mem_total: u64,
    pub temp_c: Option<f64>,
}

#[derive(Default, Clone)]
pub struct SystemStats {
    pub hostname: String,
    pub uptime_secs: u64,
    pub load_avg: [f64; 3],
    pub cpu_pct: f64,
    pub cpu_cores: usize,
    /// Per-core busy %, same order as the OS numbers them. Empty if the backend cannot tell.
    pub core_pct: Vec<f64>,
    pub mem_total: u64,
    pub mem_used: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub net_rx_bps: f64,
    pub net_tx_bps: f64,
    pub disk_read_bps: f64,
    pub disk_write_bps: f64,
    pub gpus: Vec<Gpu>,
    /// (sensor label, °C)
    pub temps: Vec<(String, f64)>,
}

#[derive(Default, Clone, Debug)]
pub struct Process {
    pub pid: u32,
    pub ppid: u32,
    pub user: String,
    pub state: char,
    pub name: String,
    pub cmdline: String,
    /// Share of total machine CPU capacity (all cores = 100%), like Windows Task Manager.
    pub cpu_pct: f64,
    pub mem_bytes: u64,
    pub disk_read_bps: f64,
    pub disk_write_bps: f64,
    /// Only filled when per-process network accounting is enabled.
    pub net_rx_bps: f64,
    pub net_tx_bps: f64,
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

    pub fn parse(s: &str) -> Option<SortKey> {
        Some(match s.to_ascii_lowercase().as_str() {
            "cpu" => SortKey::Cpu,
            "mem" | "memory" => SortKey::Mem,
            "disk" => SortKey::Disk,
            "net" | "network" => SortKey::Net,
            "name" => SortKey::Name,
            "pid" => SortKey::Pid,
            "user" => SortKey::User,
            _ => return None,
        })
    }

    /// Numeric columns show the biggest first; text columns A→Z.
    fn descending_by_default(self) -> bool {
        matches!(self, SortKey::Cpu | SortKey::Mem | SortKey::Disk | SortKey::Net)
    }
}

pub fn sort_processes(p: &mut [Process], key: SortKey, reverse: bool) {
    let desc = key.descending_by_default() ^ reverse;
    p.sort_by(|a, b| {
        let o = match key {
            SortKey::Cpu => a.cpu_pct.total_cmp(&b.cpu_pct),
            SortKey::Mem => a.mem_bytes.cmp(&b.mem_bytes),
            SortKey::Disk => (a.disk_read_bps + a.disk_write_bps).total_cmp(&(b.disk_read_bps + b.disk_write_bps)),
            SortKey::Net => (a.net_rx_bps + a.net_tx_bps).total_cmp(&(b.net_rx_bps + b.net_tx_bps)),
            SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortKey::Pid => a.pid.cmp(&b.pid),
            SortKey::User => a.user.cmp(&b.user),
        };
        if desc { o.reverse() } else { o }.then(a.pid.cmp(&b.pid))
    });
}

pub fn sort_containers(c: &mut [Container], key: SortKey, reverse: bool) {
    let desc = key.descending_by_default() ^ reverse;
    c.sort_by(|a, b| {
        let o = match key {
            SortKey::Cpu => a.cpu_pct.total_cmp(&b.cpu_pct),
            SortKey::Mem => a.mem_bytes.cmp(&b.mem_bytes),
            SortKey::Disk => (a.blk_read_bytes + a.blk_write_bytes).cmp(&(b.blk_read_bytes + b.blk_write_bytes)),
            SortKey::Net => (a.net_rx_bytes + a.net_tx_bytes).cmp(&(b.net_rx_bytes + b.net_tx_bytes)),
            SortKey::Pid => a.pids.cmp(&b.pids),
            SortKey::Name | SortKey::User => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        };
        if desc { o.reverse() } else { o }.then(a.id.cmp(&b.id))
    });
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
        assert_eq!(SortKey::parse("Memory"), Some(SortKey::Mem));
    }
}
