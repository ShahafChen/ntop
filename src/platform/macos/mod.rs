//! macOS backend: shells out to `sysctl`, `vm_stat`, `netstat`, `ioreg`, `ps`, `dscl` and
//! (opt-in) `nettop`, plus Mach calls for per-core CPU and `proc_pid_rusage` for per-process
//! disk I/O. Containers come from the shared `docker` CLI poller.

mod cmd;

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{docker, Collector, Counters, ProcCounters, Rates};
use crate::config::Config;
use crate::model::{Process, Snapshot, SystemStats};

/// Mach CPU ticks run at 100 Hz; `ps` CPU time is in ms. Everything is converted to ms.
const MS_PER_TICK: u64 = 10;

pub struct MacCollector {
    rates: Rates,
    users: HashMap<u32, String>,
    docker: Option<Arc<Mutex<docker::DockerState>>>,
    per_process_net: bool,
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

/// (busy, total) ms per core from `host_processor_info`.
// libc marks the Mach helpers deprecated in favour of the `mach2` crate; not worth a dependency for two calls.
#[allow(deprecated)]
fn core_ticks() -> Vec<(u64, u64)> {
    let mut count: libc::natural_t = 0;
    let mut info: libc::processor_info_array_t = std::ptr::null_mut();
    let mut info_len: libc::mach_msg_type_number_t = 0;
    // SAFETY: out-pointers are valid; the kernel allocates `info`, which is freed below.
    let rc = unsafe { libc::host_processor_info(libc::mach_host_self(), libc::PROCESSOR_CPU_LOAD_INFO, &mut count, &mut info, &mut info_len) };
    if rc != 0 || info.is_null() {
        return Vec::new();
    }
    let states = unsafe { std::slice::from_raw_parts(info, info_len as usize) };
    let cores = states
        .as_chunks::<{ libc::CPU_STATE_MAX as usize }>()
        .0
        .iter()
        .map(|c| {
            let total: u64 = c.iter().map(|&x| x as u32 as u64).sum();
            let idle = c[libc::CPU_STATE_IDLE as usize] as u32 as u64;
            ((total - idle) * MS_PER_TICK, total * MS_PER_TICK)
        })
        .collect();
    unsafe { libc::vm_deallocate(libc::mach_task_self(), info as libc::vm_address_t, info_len as usize * std::mem::size_of::<libc::integer_t>()) };
    cores
}

impl MacCollector {
    pub fn new(cfg: &Config) -> Self {
        Self {
            rates: Rates::default(),
            users: cmd::parse_dscl_users(&run("dscl", &[".", "-list", "/Users", "UniqueID"])),
            docker: docker::spawn_poller(cfg.docker),
            per_process_net: cfg.per_process_net,
        }
    }
}

impl Collector for MacCollector {
    fn sample(&mut self) -> Snapshot {
        let sys = cmd::parse_sysctl(&run("sysctl", &["hw.ncpu", "hw.memsize", "vm.swapusage", "vm.loadavg", "kern.boottime", "kern.hostname"]));
        let get = |k: &str| sys.get(k).map(String::as_str).unwrap_or("");
        let (swap_total, swap_used) = cmd::parse_swapusage(get("vm.swapusage"));
        let now_epoch = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let (containers, docker_error) = docker::snapshot(&self.docker);
        let net_by_pid = if self.per_process_net { cmd::parse_nettop(&run("nettop", &["-P", "-L", "1", "-x", "-J", "bytes_in,bytes_out"])) } else { HashMap::new() };

        let rows = cmd::parse_ps(&run("ps", &["-axo", "pid=,ppid=,uid=,state=,rss=,time=,comm=", "-ww"]));
        let mut args = cmd::parse_ps_args(&run("ps", &["-axo", "pid=,args=", "-ww"]));
        let cores = core_ticks();
        let mut cur = Counters {
            cpu_busy: cores.iter().map(|c| c.0).sum(),
            cpu_total: cores.iter().map(|c| c.1).sum(),
            cores,
            net: cmd::parse_netstat_ib(&run("netstat", &["-ib"])),
            disk: cmd::parse_ioreg_disk(&run("ioreg", &["-c", "IOBlockStorageDriver", "-r", "-w", "0"])),
            procs: HashMap::with_capacity(rows.len()),
        };
        let mut processes = Vec::with_capacity(rows.len());
        for r in rows {
            let (disk_read, disk_write) = disk_io(r.pid);
            let (net_rx, net_tx) = net_by_pid.get(&r.pid).copied().unwrap_or_default();
            cur.procs.insert(r.pid, ProcCounters { cpu: r.cpu_ms, disk_read, disk_write, net_rx, net_tx });
            processes.push(Process {
                pid: r.pid,
                ppid: r.ppid,
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
            cpu_cores: get("hw.ncpu").parse().unwrap_or(1),
            mem_total: get("hw.memsize").parse().unwrap_or(0),
            mem_used: cmd::parse_vm_stat(&run("vm_stat", &[])),
            swap_total,
            swap_used,
            gpus: cmd::parse_ioreg_gpu(&run("ioreg", &["-r", "-d", "1", "-c", "IOAccelerator"])).into_iter().collect(),
            temps: Vec::new(), // needs `sudo powermetrics`; not worth a privileged helper
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
        let mut c = MacCollector::new(&Config::default());
        c.sample();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let s = c.sample();
        assert!(s.system.mem_total > 0 && s.system.mem_used > 0 && s.system.uptime_secs > 0);
        assert_eq!(s.system.core_pct.len(), s.system.cpu_cores);
        let me = s.processes.iter().find(|p| p.pid == std::process::id()).expect("own pid listed");
        assert!(me.mem_bytes > 0 && !me.user.is_empty() && !me.name.is_empty() && me.ppid > 0);
    }
}
