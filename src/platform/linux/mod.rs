//! Linux backend: reads `/proc`, `/sys` and `/etc/passwd`, plus `nvidia-smi` and `ss` when present.

mod proc;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::process::Command;
use std::sync::{Arc, Mutex};

use super::{docker, Collector, Counters, ProcCounters, Rates};
use crate::config::Config;
use crate::model::{Process, Snapshot, SystemStats};

pub struct LinuxCollector {
    rates: Rates,
    users: HashMap<u32, String>,
    disks: HashSet<String>,
    docker: Option<Arc<Mutex<docker::DockerState>>>,
    has_nvidia: bool,
    per_process_net: bool,
}

fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

const NVIDIA_ARGS: [&str; 2] = ["--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu", "--format=csv,noheader,nounits"];

impl LinuxCollector {
    pub fn new(cfg: &Config) -> Self {
        // /sys/block lists whole disks only; loop/ram/zram devices are not real storage.
        let disks = fs::read_dir("/sys/block")
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| !["loop", "ram", "zram"].iter().any(|p| n.starts_with(p)))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            rates: Rates::default(),
            users: proc::parse_passwd(&read("/etc/passwd")),
            disks,
            docker: docker::spawn_poller(cfg.docker),
            has_nvidia: run("nvidia-smi", &NVIDIA_ARGS).is_some(),
            per_process_net: cfg.per_process_net,
        }
    }

    fn read_process(&self, pid: u32, containers: &HashMap<&str, &str>) -> Option<(Process, ProcCounters)> {
        let dir = format!("/proc/{pid}");
        let stat = proc::parse_pid_stat(&fs::read_to_string(format!("{dir}/stat")).ok()?)?;
        let (uid, rss) = proc::parse_pid_status(&read(&format!("{dir}/status")));
        // io needs ptrace access: other users' processes read as 0 unless running as root.
        let (disk_read, disk_write) = proc::parse_pid_io(&read(&format!("{dir}/io")));
        let cmdline = fs::read(format!("{dir}/cmdline")).unwrap_or_default();
        let cmdline = String::from_utf8_lossy(&cmdline).replace('\0', " ").trim_end().to_string();
        let container = proc::container_id(&read(&format!("{dir}/cgroup")))
            .map(|id| containers.get(id.as_str()).map_or(id, |n| n.to_string()));
        let p = Process {
            pid,
            ppid: stat.ppid,
            user: self.users.get(&uid).cloned().unwrap_or_else(|| uid.to_string()),
            state: stat.state,
            name: stat.name,
            cmdline,
            mem_bytes: rss,
            container,
            ..Default::default()
        };
        Some((p, ProcCounters { cpu: stat.ticks, disk_read, disk_write, ..Default::default() }))
    }

    /// `/sys/class/hwmon/*`: first temperature sensor of each chip.
    fn temps() -> Vec<(String, f64)> {
        let Ok(dir) = fs::read_dir("/sys/class/hwmon") else { return Vec::new() };
        let mut out: Vec<(String, f64)> = dir
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let base = e.path().to_string_lossy().into_owned();
                let milli: f64 = read(&format!("{base}/temp1_input")).trim().parse().ok()?;
                let label = read(&format!("{base}/temp1_label"));
                let name = if label.trim().is_empty() { read(&format!("{base}/name")) } else { label };
                Some((name.trim().to_string(), milli / 1000.0))
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
}

impl Collector for LinuxCollector {
    fn sample(&mut self) -> Snapshot {
        let cpu = proc::parse_stat_cpu(&read("/proc/stat"));
        let mem = proc::parse_meminfo(&read("/proc/meminfo"));
        let (containers, docker_error) = docker::snapshot(&self.docker);
        let names: HashMap<&str, &str> = containers.iter().map(|c| (c.id.as_str(), c.name.as_str())).collect();
        let net_by_pid = if self.per_process_net { run("ss", &["-tinHOp"]).map(|o| proc::parse_ss(&o)).unwrap_or_default() } else { HashMap::new() };

        let pids = fs::read_dir("/proc")
            .map(|d| d.filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok()).collect::<Vec<_>>())
            .unwrap_or_default();
        let mut processes = Vec::with_capacity(pids.len());
        let mut cur = Counters {
            cpu_busy: cpu.busy,
            cpu_total: cpu.total,
            cores: cpu.cores.clone(),
            net: proc::parse_net_dev(&read("/proc/net/dev")),
            disk: proc::parse_diskstats(&read("/proc/diskstats"), |n| self.disks.contains(n)),
            procs: HashMap::with_capacity(pids.len()),
        };
        for pid in pids {
            if let Some((p, mut counters)) = self.read_process(pid, &names) {
                (counters.net_rx, counters.net_tx) = net_by_pid.get(&pid).copied().unwrap_or_default();
                processes.push(p);
                cur.procs.insert(pid, counters);
            }
        }

        let mut system = SystemStats {
            hostname: read("/proc/sys/kernel/hostname").trim().to_string(),
            uptime_secs: proc::parse_uptime(&read("/proc/uptime")),
            load_avg: proc::parse_loadavg(&read("/proc/loadavg")),
            cpu_cores: cpu.cores.len().max(1),
            mem_total: mem.total,
            mem_used: mem.used,
            swap_total: mem.swap_total,
            swap_used: mem.swap_used,
            gpus: if self.has_nvidia { run("nvidia-smi", &NVIDIA_ARGS).map(|o| proc::parse_nvidia_smi(&o)).unwrap_or_default() } else { Vec::new() },
            temps: Self::temps(),
            ..Default::default()
        };
        self.rates.apply(cur, &mut system, &mut processes);

        Snapshot { system, processes, containers, docker_error }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs against the real /proc of the test machine.
    #[test]
    fn samples_real_system() {
        let mut c = LinuxCollector::new(&Config::default());
        c.sample();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let s = c.sample();
        assert!(s.system.mem_total > 0 && s.system.cpu_cores >= 1 && s.system.core_pct.len() == s.system.cpu_cores);
        let me = s.processes.iter().find(|p| p.pid == std::process::id()).expect("own pid listed");
        assert!(me.mem_bytes > 0 && !me.user.is_empty() && me.ppid > 0);
    }
}
