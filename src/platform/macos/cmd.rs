//! Pure parsers for the output of macOS command-line tools. No I/O here, so every function is unit-testable.

use std::collections::HashMap;

use crate::model::Gpu;

/// `sysctl key1 key2 ...` → key → value.
pub fn parse_sysctl(s: &str) -> HashMap<String, String> {
    s.lines()
        .filter_map(|l| l.split_once(": "))
        .map(|(k, v)| (k.to_string(), v.trim().to_string()))
        .collect()
}

/// "34816.00M" style numbers used by `vm.swapusage`.
fn parse_mb(s: &str) -> u64 {
    let s = s.trim();
    let (num, unit) = s.split_at(s.len().saturating_sub(1));
    let mult = match unit {
        "K" => 1u64 << 10,
        "M" => 1 << 20,
        "G" => 1 << 30,
        _ => 1,
    };
    (num.parse::<f64>().unwrap_or(0.0) * mult as f64).round() as u64
}

/// `vm.swapusage` value: "total = 2048.00M  used = 1077.00M  free = 971.00M  (encrypted)" → (total, used).
pub fn parse_swapusage(s: &str) -> (u64, u64) {
    let field = |k: &str| s.split(k).nth(1).and_then(|r| r.split_whitespace().next()).map_or(0, parse_mb);
    (field("total = "), field("used = "))
}

/// `vm.loadavg` value: "{ 5.76 4.71 4.23 }".
pub fn parse_loadavg(s: &str) -> [f64; 3] {
    let mut it = s.trim_matches(|c| c == '{' || c == '}' || c == ' ').split_whitespace().map(|x| x.parse().unwrap_or(0.0));
    [it.next().unwrap_or(0.0), it.next().unwrap_or(0.0), it.next().unwrap_or(0.0)]
}

/// `kern.boottime` value: "{ sec = 1784791251, usec = 464568 } Thu Jul 23 ..." → epoch seconds.
pub fn parse_boottime(s: &str) -> u64 {
    s.split("sec = ").nth(1).and_then(|r| r.split(',').next()).and_then(|v| v.trim().parse().ok()).unwrap_or(0)
}

/// `vm_stat` → bytes in use (active + wired + compressed), the way Activity Monitor counts it.
pub fn parse_vm_stat(s: &str) -> u64 {
    let page: u64 = s.split("page size of ").nth(1).and_then(|r| r.split_whitespace().next()).and_then(|v| v.parse().ok()).unwrap_or(4096);
    let pages = |key: &str| {
        s.lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|r| r.trim().trim_end_matches('.').parse::<u64>().ok())
            .unwrap_or(0)
    };
    (pages("Pages active:") + pages("Pages wired down:") + pages("Pages occupied by compressor:")) * page
}

/// `netstat -ib` → (rx_bytes, tx_bytes) summed over the link-level row of every interface but loopback.
pub fn parse_netstat_ib(s: &str) -> (u64, u64) {
    let (mut rx, mut tx) = (0, 0);
    for line in s.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        // Name Mtu Network [Address] Ipkts Ierrs Ibytes Opkts Oerrs Obytes Coll; Address may be absent.
        if f.len() < 10 || !f[2].starts_with("<Link") || f[0].starts_with("lo") {
            continue;
        }
        rx += f[f.len() - 5].parse::<u64>().unwrap_or(0);
        tx += f[f.len() - 2].parse::<u64>().unwrap_or(0);
    }
    (rx, tx)
}

/// `ioreg -c IOBlockStorageDriver -r -w 0` → (read_bytes, write_bytes) summed over all drivers.
pub fn parse_ioreg_disk(s: &str) -> (u64, u64) {
    let sum = |key: &str| s.split(key).skip(1).map(|r| r.trim_start_matches('=').split(|c: char| !c.is_ascii_digit()).next().unwrap_or("0").parse::<u64>().unwrap_or(0)).sum();
    (sum("\"Bytes (Read)\""), sum("\"Bytes (Write)\""))
}

#[derive(Debug, PartialEq)]
pub struct PsRow {
    pub pid: u32,
    pub ppid: u32,
    pub uid: u32,
    pub state: char,
    pub rss_bytes: u64,
    pub cpu_ms: u64,
    /// Executable path (`comm`), which may contain spaces, so it must be the last column.
    pub exe: String,
}

/// `ps -axo pid=,ppid=,uid=,state=,rss=,time=,comm= -ww`.
pub fn parse_ps(s: &str) -> Vec<PsRow> {
    s.lines().filter_map(parse_ps_line).collect()
}

fn parse_ps_line(line: &str) -> Option<PsRow> {
    let mut rest = line.trim_start();
    let mut next = || {
        let (tok, r) = rest.split_once(char::is_whitespace)?;
        rest = r.trim_start();
        Some(tok)
    };
    let pid = next()?.parse().ok()?;
    let ppid = next()?.parse().ok()?;
    let uid = next()?.parse().ok()?;
    let state = next()?.chars().next()?;
    let rss_bytes = next()?.parse::<u64>().ok()? * 1024;
    let cpu_ms = parse_cputime_ms(next()?);
    Some(PsRow { pid, ppid, uid, state, rss_bytes, cpu_ms, exe: rest.trim_end().to_string() })
}

/// `ps -axo pid=,args= -ww` → pid → full command line.
pub fn parse_ps_args(s: &str) -> HashMap<u32, String> {
    s.lines()
        .filter_map(|l| {
            let (pid, args) = l.trim_start().split_once(char::is_whitespace)?;
            Some((pid.parse().ok()?, args.trim().to_string()))
        })
        .collect()
}

/// ps TIME column "[[dd-]hh:]mm:ss.cc" (minutes may exceed 59) → milliseconds.
pub fn parse_cputime_ms(s: &str) -> u64 {
    let (days, rest) = s.split_once('-').map_or((0, s), |(d, r)| (d.parse().unwrap_or(0), r));
    let mut secs = 0.0;
    for part in rest.split(':') {
        secs = secs * 60.0 + part.parse::<f64>().unwrap_or(0.0);
    }
    ((days as f64 * 86_400.0 + secs) * 1000.0).round() as u64
}

/// `nettop -P -L 1 -x -J bytes_in,bytes_out` (CSV, "name.pid,in,out,") → pid → (bytes in, bytes out).
pub fn parse_nettop(s: &str) -> HashMap<u32, (u64, u64)> {
    s.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            let pid = f.first()?.rsplit('.').next()?.parse().ok()?;
            Some((pid, (f.get(1)?.parse().ok()?, f.get(2)?.parse().ok()?)))
        })
        .collect()
}

/// `ioreg -r -d 1 -c IOAccelerator` → the GPU's utilisation and memory, if the accelerator reports them.
pub fn parse_ioreg_gpu(s: &str) -> Option<Gpu> {
    let num = |key: &str| -> Option<f64> { s.split(key).nth(1)?.trim_start_matches('=').split(|c: char| !c.is_ascii_digit()).next()?.parse().ok() };
    Some(Gpu {
        name: "GPU".to_string(),
        util_pct: num("\"Device Utilization %\"")?,
        mem_used: num("\"In use system memory\"").unwrap_or(0.0) as u64,
        ..Default::default()
    })
}

/// `dscl . -list /Users UniqueID` → uid → user name.
pub fn parse_dscl_users(s: &str) -> HashMap<u32, String> {
    s.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let name = it.next()?;
            Some((it.next()?.parse().ok()?, name.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sysctl_values() {
        let m = parse_sysctl("hw.ncpu: 10\nhw.memsize: 17179869184\nvm.loadavg: { 5.76 4.71 4.23 }\nkern.boottime: { sec = 1784791251, usec = 464568 } Thu Jul 23 10:20:51 2026\n");
        assert_eq!(m["hw.ncpu"], "10");
        assert_eq!(parse_loadavg(&m["vm.loadavg"]), [5.76, 4.71, 4.23]);
        assert_eq!(parse_boottime(&m["kern.boottime"]), 1784791251);
        assert_eq!(parse_swapusage("total = 2048.00M  used = 1077.50M  free = 971.00M  (encrypted)"), (2048 << 20, 1077 * (1 << 20) + (1 << 19)));
    }

    #[test]
    fn vm_stat_used() {
        let s = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\nPages free:   10.\nPages active:   3.\nPages inactive:  5.\nPages wired down:   2.\nPages occupied by compressor:  1.\n";
        assert_eq!(parse_vm_stat(s), 6 * 16384);
    }

    #[test]
    fn netstat_link_rows() {
        let s = "Name  Mtu Network Address Ipkts Ierrs Ibytes Opkts Oerrs Obytes Coll\n\
                 lo0 16384 <Link#1> 5 0 999 5 0 999 0\n\
                 lo0 16384 127 localhost 5 - 999 5 - 999 -\n\
                 en0 1500 <Link#4> aa:bb:cc:dd:ee:ff 10 0 100 20 0 200 0\n\
                 en0 1500 192.168.1 192.168.1.5 10 - 100 20 - 200 -\n";
        assert_eq!(parse_netstat_ib(s), (100, 200));
    }

    #[test]
    fn ioreg_sums() {
        let s = "| \"Statistics\" = {\"Bytes (Read)\"=100,\"Operations (Write)\"=3,\"Bytes (Write)\"=50}\n| \"Statistics\" = {\"Bytes (Read)\"=1,\"Bytes (Write)\"=2}\n";
        assert_eq!(parse_ioreg_disk(s), (101, 52));
    }

    #[test]
    fn ps_rows_and_cputime() {
        let rows = parse_ps("    1     0     0 Ss     9280 329:25.16 /sbin/launchd\n  500     1   501 R+      100  1-02:03:04.50 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome\nbad line\n");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], PsRow { pid: 1, ppid: 0, uid: 0, state: 'S', rss_bytes: 9280 * 1024, cpu_ms: 329 * 60_000 + 25_160, exe: "/sbin/launchd".into() });
        assert_eq!(rows[1].cpu_ms, 86_400_000 + 2 * 3_600_000 + 3 * 60_000 + 4_500);
        assert_eq!(rows[1].exe, "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
        assert_eq!(parse_cputime_ms("0:00.12"), 120);
        assert_eq!(parse_ps_args("  500 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome --type=renderer\n")[&500], "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome --type=renderer");
    }

    #[test]
    fn nettop_and_gpu() {
        let n = parse_nettop(",bytes_in,bytes_out,\nkernel_task.0,5944614,0,\nGoogle Chrome H.1234,10,20,\n");
        assert_eq!((n[&0], n[&1234], n.len()), ((5944614, 0), (10, 20), 2));
        let g = parse_ioreg_gpu("| \"PerformanceStatistics\" = {\"Renderer Utilization %\"=1,\"Device Utilization %\"=12,\"In use system memory\"=290537472}\n").unwrap();
        assert_eq!((g.util_pct, g.mem_used), (12.0, 290537472));
        assert!(parse_ioreg_gpu("nothing").is_none());
    }

    #[test]
    fn dscl_users() {
        assert_eq!(parse_dscl_users("_www   70\nshahaf  501\nnobody  -2\n")[&501], "shahaf");
    }
}
