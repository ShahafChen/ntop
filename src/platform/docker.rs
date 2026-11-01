//! Container stats via the `docker` CLI, polled on a background thread because
//! `docker stats --no-stream` blocks for ~1-2 s while it samples.

use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::model::Container;

const FORMAT: &str = "{{.ID}}\t{{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.NetIO}}\t{{.BlockIO}}\t{{.PIDs}}";

#[derive(Default)]
pub struct DockerState {
    pub containers: Vec<Container>,
    pub error: Option<String>,
}

pub fn spawn_poller() -> Arc<Mutex<DockerState>> {
    let state = Arc::new(Mutex::new(DockerState::default()));
    let shared = Arc::clone(&state);
    thread::spawn(move || loop {
        let result = fetch();
        let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(c) => *s = DockerState { containers: c, error: None },
            Err(e) => *s = DockerState { containers: Vec::new(), error: Some(e) },
        }
        drop(s);
        thread::sleep(Duration::from_secs(1));
    });
    state
}

fn fetch() -> Result<Vec<Container>, String> {
    let out = Command::new("docker")
        .args(["stats", "--no-stream", "--format", FORMAT])
        .output()
        .map_err(|e| format!("docker not runnable: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().next().unwrap_or("docker stats failed").to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().filter_map(parse_stats_line).collect())
}

/// One tab-separated line produced by `FORMAT`.
pub fn parse_stats_line(line: &str) -> Option<Container> {
    let f: Vec<&str> = line.split('\t').collect();
    if f.len() < 7 {
        return None;
    }
    let pair = |s: &str| s.split_once('/').map_or((0, 0), |(a, b)| (parse_size(a), parse_size(b)));
    let (mem_bytes, mem_limit) = pair(f[3]);
    let (net_rx_bytes, net_tx_bytes) = pair(f[4]);
    let (blk_read_bytes, blk_write_bytes) = pair(f[5]);
    Some(Container {
        id: f[0].to_string(),
        name: f[1].to_string(),
        cpu_pct: f[2].trim().trim_end_matches('%').parse().unwrap_or(0.0),
        mem_bytes,
        mem_limit,
        net_rx_bytes,
        net_tx_bytes,
        blk_read_bytes,
        blk_write_bytes,
        pids: f[6].trim().parse().unwrap_or(0),
    })
}

/// Docker's human sizes: binary for memory ("1.5MiB"), decimal for I/O ("1.2kB").
pub fn parse_size(s: &str) -> u64 {
    let s = s.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let num: f64 = s[..split].parse().unwrap_or(0.0);
    let mult = match s[split..].trim() {
        "" | "B" => 1.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        "KiB" => 1024.0,
        "MiB" => 1024f64.powi(2),
        "GiB" => 1024f64.powi(3),
        "TiB" => 1024f64.powi(4),
        _ => 1.0,
    };
    (num * mult).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("0B"), 0);
        assert_eq!(parse_size("1.5MiB"), 1_572_864);
        assert_eq!(parse_size("2kB"), 2000);
        assert_eq!(parse_size(" 3GiB "), 3 * 1024 * 1024 * 1024);
    }

    #[test]
    fn stats_line() {
        let c = parse_stats_line("abc123def456\tweb\t12.5%\t1.5MiB / 7.7GiB\t1.2kB / 3MB\t0B / 4.1MB\t7").unwrap();
        assert_eq!((c.id.as_str(), c.name.as_str(), c.cpu_pct, c.pids), ("abc123def456", "web", 12.5, 7));
        assert_eq!((c.mem_bytes, c.net_rx_bytes, c.net_tx_bytes, c.blk_write_bytes), (1_572_864, 1200, 3_000_000, 4_100_000));
        assert!(parse_stats_line("garbage").is_none());
    }
}
