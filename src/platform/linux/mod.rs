//! Linux backend: reads `/proc`, `/sys` and `/etc/passwd`, and the `docker` CLI for containers.

mod proc;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::{Arc, Mutex};

use super::{docker, Collector, Counters, Rates};
use crate::model::{Process, Snapshot, SystemStats};

pub struct LinuxCollector {
    rates: Rates,
    users: HashMap<u32, String>,
    disks: HashSet<String>,
    docker: Arc<Mutex<docker::DockerState>>,
}

fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

impl LinuxCollector {
    pub fn new() -> Self {
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
            docker: docker::spawn_poller(),
        }
    }

    fn read_process(&self, pid: u32, containers: &HashMap<&str, &str>) -> Option<(Process, (u64, u64, u64))> {
        let dir = format!("/proc/{pid}");
        let stat = proc::parse_pid_stat(&fs::read_to_string(format!("{dir}/stat")).ok()?)?;
        let (uid, rss) = proc::parse_pid_status(&read(&format!("{dir}/status")));
        // io needs ptrace access: other users' processes read as 0 unless running as root.
        let (rd, wr) = proc::parse_pid_io(&read(&format!("{dir}/io")));
        let cmdline = fs::read(format!("{dir}/cmdline")).unwrap_or_default();
        let cmdline = String::from_utf8_lossy(&cmdline).replace('\0', " ").trim_end().to_string();
        let container = proc::container_id(&read(&format!("{dir}/cgroup")))
            .map(|id| containers.get(id.as_str()).map_or(id, |n| n.to_string()));
        let p = Process {
            pid,
            user: self.users.get(&uid).cloned().unwrap_or_else(|| uid.to_string()),
            state: stat.state,
            name: stat.name,
            cmdline,
            mem_bytes: rss,
            container,
            ..Default::default()
        };
        Some((p, (stat.ticks, rd, wr)))
    }
}

impl Collector for LinuxCollector {
    fn sample(&mut self) -> Snapshot {
        let cpu = proc::parse_stat_cpu(&read("/proc/stat"));
        let mem = proc::parse_meminfo(&read("/proc/meminfo"));
        let net = proc::parse_net_dev(&read("/proc/net/dev"));
        let disk = proc::parse_diskstats(&read("/proc/diskstats"), |n| self.disks.contains(n));

        let (containers, docker_error) = {
            let d = self.docker.lock().unwrap_or_else(|e| e.into_inner());
            (d.containers.clone(), d.error.clone())
        };
        let names: HashMap<&str, &str> = containers.iter().map(|c| (c.id.as_str(), c.name.as_str())).collect();

        let pids = fs::read_dir("/proc")
            .map(|d| d.filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok()).collect::<Vec<_>>())
            .unwrap_or_default();
        let mut processes = Vec::with_capacity(pids.len());
        let mut cur = Counters { cpu_busy: cpu.busy, cpu_total: cpu.total, net, disk, procs: HashMap::new() };
        for pid in pids {
            if let Some((p, counters)) = self.read_process(pid, &names) {
                processes.push(p);
                cur.procs.insert(pid, counters);
            }
        }

        let mut system = SystemStats {
            hostname: read("/proc/sys/kernel/hostname").trim().to_string(),
            uptime_secs: proc::parse_uptime(&read("/proc/uptime")),
            load_avg: proc::parse_loadavg(&read("/proc/loadavg")),
            cpu_cores: cpu.cores.max(1),
            mem_total: mem.total,
            mem_used: mem.used,
            swap_total: mem.swap_total,
            swap_used: mem.swap_used,
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
        let mut c = LinuxCollector::new();
        c.sample();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let s = c.sample();
        assert!(s.system.mem_total > 0 && s.system.cpu_cores >= 1);
        let me = s.processes.iter().find(|p| p.pid == std::process::id()).expect("own pid listed");
        assert!(me.mem_bytes > 0 && !me.user.is_empty());
    }
}
