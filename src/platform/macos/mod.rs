//! macOS backend: shells out to `sysctl`, `vm_stat`, `netstat`, `ioreg`, `ps` and `dscl`,
//! plus `proc_pid_rusage` for per-process disk I/O. Containers come from the shared `docker` CLI poller.

mod cmd;

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::{docker, Collector, Counters, Rates};
use crate::model::{Process, Snapshot, SystemStats};

pub struct MacCollector {
    rates: Rates,
    started: Instant,
    users: HashMap<u32, String>,
    docker: Arc<Mutex<docker::DockerState>>,
}

fn run(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd).args(args).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

/// Bytes read/written to storage by `pid`. Other users' processes need root; failure reads as 0.
fn disk_io(pid: u32) -> (u64, u64) {
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is a valid rusage_info_v2 buffer and the flavor matches its layout.
    let rc = unsafe { libc::proc_pid_rusage(pid as i32, libc::RUSAGE_INFO_V2, &mut info as *mut _ as *mut _) };
    if rc == 0 { (info.ri_diskio_bytesread, info.ri_diskio_byteswritten) } else { (0, 0) }
}

impl MacCollector {
    pub fn new() -> Self {
        Self {
            rates: Rates::default(),
            started: Instant::now(),
            users: cmd::parse_dscl_users(&run("dscl", &[".", "-list", "/Users", "UniqueID"])),
            docker: docker::spawn_poller(),
        }
    }
}

impl Collector for MacCollector {
    fn sample(&mut self) -> Snapshot {
        let sys = cmd::parse_sysctl(&run("sysctl", &["hw.ncpu", "hw.memsize", "vm.swapusage", "vm.loadavg", "kern.boottime", "kern.hostname"]));
        let get = |k: &str| sys.get(k).map(String::as_str).unwrap_or("");
        let cores: usize = get("hw.ncpu").parse().unwrap_or(1);
        let (swap_total, swap_used) = cmd::parse_swapusage(get("vm.swapusage"));
        let now_epoch = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

        let (containers, docker_error) = {
            let d = self.docker.lock().unwrap_or_else(|e| e.into_inner());
            (d.containers.clone(), d.error.clone())
        };

        let rows = cmd::parse_ps(&run("ps", &["-axo", "pid=,uid=,state=,rss=,time=,comm=", "-ww"]));
        let mut args = cmd::parse_ps_args(&run("ps", &["-axo", "pid=,args=", "-ww"]));
        let mut cur = Counters {
            // shortcut: busy = Σ process CPU time, so time of processes that exited is lost; swap for host_statistics if that matters.
            cpu_busy: rows.iter().map(|r| r.cpu_ms).sum(),
            cpu_total: self.started.elapsed().as_millis() as u64 * cores as u64,
            net: cmd::parse_netstat_ib(&run("netstat", &["-ib"])),
            disk: cmd::parse_ioreg_disk(&run("ioreg", &["-c", "IOBlockStorageDriver", "-r", "-w", "0"])),
            procs: HashMap::with_capacity(rows.len()),
        };
        let mut processes = Vec::with_capacity(rows.len());
        for r in rows {
            let (rd, wr) = disk_io(r.pid);
            cur.procs.insert(r.pid, (r.cpu_ms, rd, wr));
            processes.push(Process {
                pid: r.pid,
                user: self.users.get(&r.uid).cloned().unwrap_or_else(|| r.uid.to_string()),
                state: r.state,
                name: r.exe.rsplit('/').next().unwrap_or(&r.exe).to_string(),
                cmdline: args.remove(&r.pid).unwrap_or(r.exe),
                mem_bytes: r.rss_bytes,
                container: None, // Docker Desktop runs containers in a VM; host processes never belong to one
                ..Default::default()
            });
        }

        let mut system = SystemStats {
            hostname: get("kern.hostname").to_string(),
            uptime_secs: now_epoch.saturating_sub(cmd::parse_boottime(get("kern.boottime"))),
            load_avg: cmd::parse_loadavg(get("vm.loadavg")),
            cpu_cores: cores,
            mem_total: get("hw.memsize").parse().unwrap_or(0),
            mem_used: cmd::parse_vm_stat(&run("vm_stat", &[])),
            swap_total,
            swap_used,
            ..Default::default()
        };
        self.rates.apply(cur, &mut system, &mut processes);

        Snapshot { system, processes, containers, docker_error }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs against the real machine.
    #[test]
    fn samples_real_system() {
        let mut c = MacCollector::new();
        c.sample();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let s = c.sample();
        assert!(s.system.mem_total > 0 && s.system.mem_used > 0 && s.system.cpu_cores >= 1 && s.system.uptime_secs > 0);
        let me = s.processes.iter().find(|p| p.pid == std::process::id()).expect("own pid listed");
        assert!(me.mem_bytes > 0 && !me.user.is_empty() && !me.name.is_empty());
    }
}
