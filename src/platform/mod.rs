//! OS backends. Each backend lives in its own folder and only has to implement `Collector`.
//! To port to another OS: add `platform/<os>/`, implement `Collector`, and select it below.
//! `docker.rs` is shared: it only shells out to the `docker` CLI.

use std::collections::HashMap;
use std::time::Instant;

use crate::config::Config;
use crate::model::{Process, Snapshot, SystemStats};

pub mod docker;

pub trait Collector {
    /// Take one sample. Rates (bytes/s, CPU %) are computed against the previous call.
    fn sample(&mut self) -> Snapshot;
}

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub fn collector(cfg: &Config) -> Box<dyn Collector> {
    Box::new(linux::LinuxCollector::new(cfg))
}

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub fn collector(cfg: &Config) -> Box<dyn Collector> {
    Box::new(macos::MacCollector::new(cfg))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("ntop has no backend for this OS; add one under src/platform/<os>/");

/// Monotonic per-process counters. `cpu` shares its unit with `Counters::cpu_total`.
#[derive(Default, Clone, Copy, Debug)]
pub struct ProcCounters {
    pub cpu: u64,
    pub disk_read: u64,
    pub disk_write: u64,
    pub net_rx: u64,
    pub net_tx: u64,
}

/// Monotonic counters a backend reads on every sample. Units only need to be consistent
/// between calls: `cpu_busy`/`cpu_total`, `cores` and per-process `cpu` share one unit.
#[derive(Default)]
pub struct Counters {
    pub cpu_busy: u64,
    pub cpu_total: u64,
    /// (busy, total) per core.
    pub cores: Vec<(u64, u64)>,
    pub net: (u64, u64),
    pub disk: (u64, u64),
    pub procs: HashMap<u32, ProcCounters>,
}

/// Turns two successive `Counters` into the rates shown in the UI.
#[derive(Default)]
pub struct Rates {
    prev: Option<(Instant, Counters)>,
}

fn pct(cur: u64, prev: u64, total: f64) -> f64 {
    cur.saturating_sub(prev) as f64 / total * 100.0
}

impl Rates {
    pub fn apply(&mut self, cur: Counters, system: &mut SystemStats, processes: &mut [Process]) {
        let now = Instant::now();
        if let Some((t0, prev)) = &self.prev {
            let dt = now.duration_since(*t0).as_secs_f64().max(1e-3);
            let d_total = cur.cpu_total.saturating_sub(prev.cpu_total).max(1) as f64;
            system.cpu_pct = pct(cur.cpu_busy, prev.cpu_busy, d_total);
            system.core_pct = cur
                .cores
                .iter()
                .zip(&prev.cores)
                .map(|((b, t), (b0, t0))| pct(*b, *b0, t.saturating_sub(*t0).max(1) as f64))
                .collect();
            system.net_rx_bps = cur.net.0.saturating_sub(prev.net.0) as f64 / dt;
            system.net_tx_bps = cur.net.1.saturating_sub(prev.net.1) as f64 / dt;
            system.disk_read_bps = cur.disk.0.saturating_sub(prev.disk.0) as f64 / dt;
            system.disk_write_bps = cur.disk.1.saturating_sub(prev.disk.1) as f64 / dt;
            for p in processes.iter_mut() {
                let c = cur.procs.get(&p.pid).copied().unwrap_or_default();
                let c0 = prev.procs.get(&p.pid).copied().unwrap_or_default();
                p.cpu_pct = pct(c.cpu, c0.cpu, d_total);
                p.disk_read_bps = c.disk_read.saturating_sub(c0.disk_read) as f64 / dt;
                p.disk_write_bps = c.disk_write.saturating_sub(c0.disk_write) as f64 / dt;
                p.net_rx_bps = c.net_rx.saturating_sub(c0.net_rx) as f64 / dt;
                p.net_tx_bps = c.net_tx.saturating_sub(c0.net_tx) as f64 / dt;
            }
        }
        self.prev = Some((now, cur));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_are_deltas() {
        let mut r = Rates::default();
        let mut sys = SystemStats::default();
        let mut procs = vec![Process { pid: 7, ..Default::default() }];
        let counters = |busy, total, ticks| Counters {
            cpu_busy: busy,
            cpu_total: total,
            cores: vec![(busy, total)],
            procs: HashMap::from([(7, ProcCounters { cpu: ticks, ..Default::default() })]),
            ..Default::default()
        };
        r.apply(counters(50, 100, 10), &mut sys, &mut procs);
        assert_eq!(sys.cpu_pct, 0.0); // first sample has nothing to diff against
        r.apply(counters(75, 200, 60), &mut sys, &mut procs);
        assert_eq!((sys.cpu_pct, sys.core_pct[0], procs[0].cpu_pct), (25.0, 25.0, 50.0));
    }
}
