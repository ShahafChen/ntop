//! Pure parsers for Linux `/proc` and `/etc` text. No I/O here, so every function is unit-testable.

use std::collections::HashMap;

/// Value of a `Key:   123 ...` line (first token after the key), 0 when absent.
fn kv(s: &str, key: &str) -> u64 {
    s.lines()
        .find_map(|l| l.strip_prefix(key))
        .and_then(|r| r.split_whitespace().next())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct CpuTimes {
    pub busy: u64,
    pub total: u64,
    pub cores: usize,
}

/// `/proc/stat`: aggregate jiffies over all cores plus the core count.
pub fn parse_stat_cpu(s: &str) -> CpuTimes {
    let mut t = CpuTimes::default();
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("cpu ") {
            let v: Vec<u64> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            t.total = v.iter().sum();
            let idle = v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
            t.busy = t.total.saturating_sub(idle);
        } else if line.starts_with("cpu") {
            t.cores += 1;
        }
    }
    t
}

#[derive(Default, Debug, PartialEq)]
pub struct Mem {
    pub total: u64,
    pub used: u64,
    pub swap_total: u64,
    pub swap_used: u64,
}

/// `/proc/meminfo` (values are kB). "used" follows `free`: total − available.
pub fn parse_meminfo(s: &str) -> Mem {
    let kb = |k| kv(s, k) * 1024;
    let total = kb("MemTotal:");
    let swap_total = kb("SwapTotal:");
    Mem {
        total,
        used: total.saturating_sub(kb("MemAvailable:")),
        swap_total,
        swap_used: swap_total.saturating_sub(kb("SwapFree:")),
    }
}

/// Virtual interfaces that would double-count traffic already seen on the physical NIC.
const SKIP_IFACES: [&str; 4] = ["lo", "veth", "docker", "br-"];

/// `/proc/net/dev` → (rx_bytes, tx_bytes) summed over physical-looking interfaces.
pub fn parse_net_dev(s: &str) -> (u64, u64) {
    let (mut rx, mut tx) = (0, 0);
    for line in s.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else { continue };
        let name = name.trim();
        if SKIP_IFACES.iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let f: Vec<u64> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        rx += f.first().copied().unwrap_or(0);
        tx += f.get(8).copied().unwrap_or(0);
    }
    (rx, tx)
}

/// `/proc/diskstats` → (read_bytes, written_bytes) over devices accepted by `is_disk`
/// (callers pass whole-disk names from `/sys/block` so partitions are not counted twice).
pub fn parse_diskstats(s: &str, is_disk: impl Fn(&str) -> bool) -> (u64, u64) {
    let (mut r, mut w) = (0, 0);
    for line in s.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 10 || !is_disk(f[2]) {
            continue;
        }
        r += f[5].parse::<u64>().unwrap_or(0) * 512;
        w += f[9].parse::<u64>().unwrap_or(0) * 512;
    }
    (r, w)
}

#[derive(Debug, PartialEq)]
pub struct PidStat {
    pub name: String,
    pub state: char,
    /// utime + stime in jiffies.
    pub ticks: u64,
}

/// `/proc/<pid>/stat`. The comm field is in parens and may contain spaces, so split after the last `)`.
pub fn parse_pid_stat(s: &str) -> Option<PidStat> {
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    let f: Vec<&str> = s[close + 1..].split_whitespace().collect();
    Some(PidStat {
        name: s[open + 1..close].to_string(),
        state: f.first()?.chars().next()?,
        ticks: f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?,
    })
}

/// `/proc/<pid>/status` → (real uid, resident set bytes).
pub fn parse_pid_status(s: &str) -> (u32, u64) {
    (kv(s, "Uid:") as u32, kv(s, "VmRSS:") * 1024)
}

/// `/proc/<pid>/io` → (read_bytes, write_bytes) actually hitting storage.
pub fn parse_pid_io(s: &str) -> (u64, u64) {
    (kv(s, "read_bytes:"), kv(s, "write_bytes:"))
}

/// `/proc/<pid>/cgroup` → short (12-char) container id, if the process lives in one.
/// Works for cgroup v1/v2, docker, containerd and podman paths: all embed a 64-hex id.
pub fn container_id(cgroup: &str) -> Option<String> {
    cgroup
        .split(|c: char| !c.is_ascii_hexdigit())
        .find(|t| t.len() == 64)
        .map(|t| t[..12].to_string())
}

/// `/etc/passwd` → uid → user name.
pub fn parse_passwd(s: &str) -> HashMap<u32, String> {
    s.lines()
        .filter_map(|l| {
            let mut p = l.split(':');
            let name = p.next()?;
            p.next()?;
            Some((p.next()?.parse().ok()?, name.to_string()))
        })
        .collect()
}

/// `/proc/loadavg` → the three load averages.
pub fn parse_loadavg(s: &str) -> [f64; 3] {
    let mut it = s.split_whitespace().map(|x| x.parse().unwrap_or(0.0));
    [it.next().unwrap_or(0.0), it.next().unwrap_or(0.0), it.next().unwrap_or(0.0)]
}

/// `/proc/uptime` → seconds since boot.
pub fn parse_uptime(s: &str) -> u64 {
    s.split_whitespace().next().and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_cpu() {
        let s = "cpu  10 2 5 100 3 0 1 0 0 0\ncpu0 5 1 2 50 1 0 0 0 0 0\ncpu1 5 1 3 50 2 0 1 0 0 0\nintr 1 2\n";
        assert_eq!(parse_stat_cpu(s), CpuTimes { busy: 18, total: 121, cores: 2 });
    }

    #[test]
    fn meminfo() {
        let s = "MemTotal:       16 kB\nMemFree:  2 kB\nMemAvailable:   10 kB\nSwapTotal:  8 kB\nSwapFree:  6 kB\n";
        assert_eq!(parse_meminfo(s), Mem { total: 16 * 1024, used: 6 * 1024, swap_total: 8 * 1024, swap_used: 2 * 1024 });
    }

    #[test]
    fn net_dev_skips_virtual() {
        let s = "Inter-|   Receive |  Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets\n\
                    lo: 999 1 0 0 0 0 0 0 999 1 0 0 0 0 0 0\n  eth0: 100 1 0 0 0 0 0 0 200 1 0 0 0 0 0 0\n\
                  docker0: 50 1 0 0 0 0 0 0 50 1 0 0 0 0 0 0\nveth1a: 5 1 0 0 0 0 0 0 5 1 0 0 0 0 0 0\n";
        assert_eq!(parse_net_dev(s), (100, 200));
    }

    #[test]
    fn diskstats_whole_disks_only() {
        let s = "   8       0 sda 100 0 2000 0 50 0 4000 0 0 0 0\n   8       1 sda1 100 0 2000 0 50 0 4000 0 0 0 0\n";
        assert_eq!(parse_diskstats(s, |n| n == "sda"), (2000 * 512, 4000 * 512));
    }

    #[test]
    fn pid_stat_with_spaces_in_name() {
        let s = "1234 (my (odd) proc) S 1 1234 1234 0 -1 4194560 100 0 0 0 7 3 0 0 20 0 2 0 100 1000 50 0";
        assert_eq!(parse_pid_stat(s), Some(PidStat { name: "my (odd) proc".into(), state: 'S', ticks: 10 }));
    }

    #[test]
    fn pid_status_and_io() {
        assert_eq!(parse_pid_status("Name:\tbash\nUid:\t1000\t1000\t1000\t1000\nVmRSS:\t    4 kB\n"), (1000, 4096));
        assert_eq!(parse_pid_io("rchar: 9\nread_bytes: 12\nwrite_bytes: 34\n"), (12, 34));
    }

    #[test]
    fn cgroup_container_id() {
        let id = "a".repeat(64);
        assert_eq!(container_id(&format!("0::/system.slice/docker-{id}.scope\n")), Some("a".repeat(12)));
        assert_eq!(container_id(&format!("12:cpu:/docker/{id}\n")), Some("a".repeat(12)));
        assert_eq!(container_id("0::/user.slice/user-1000.slice\n"), None);
    }

    #[test]
    fn passwd_and_loadavg() {
        assert_eq!(parse_passwd("root:x:0:0:root:/root:/bin/sh\nbob:x:1000:1000::/home/bob:/bin/sh\nbroken\n")[&1000], "bob");
        assert_eq!(parse_loadavg("0.5 1.25 2.0 1/300 4567"), [0.5, 1.25, 2.0]);
        assert_eq!(parse_uptime("12345.67 99.0"), 12345);
    }
}
