//! OS backends. Each backend lives in its own folder and only has to implement `Collector`.
//! To port to another OS: add `platform/<os>/`, implement `Collector`, and select it below.
//! `docker.rs` is shared: it only shells out to the `docker` CLI.

use std::collections::HashMap;
use std::time::Instant;

use crate::model::{Process, Snapshot, SystemStats};

pub mod docker;

pub trait Collector {
    /// Take one sample. Rates (bytes/s, CPU %) are computed against the previous call.
    fn sample(&mut self) -> Snapshot;
}

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub fn collector() -> Box<dyn Collector> {
    Box::new(linux::LinuxCollector::new())
}

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub fn collector() -> Box<dyn Collector> {
    Box::new(macos::MacCollector::new())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("ntop has no backend for this OS; add one under src/platform/<os>/");

/// Monotonic counters a backend reads on every sample. Units only need to be consistent
/// between calls: `cpu_busy`/`cpu_total` and per-process `ticks` share one unit.
#[derive(Default)]
pub struct Counters {
    pub cpu_busy: u64,
    pub cpu_total: u64,
    pub net: (u64, u64),
    pub disk: (u64, u64),
    /// pid → (cpu ticks, disk read bytes, disk write bytes)
    pub procs: HashMap<u32, (u64, u64, u64)>,
}

/// Turns two successive `Counters` into the rates shown in the UI.
#[derive(Default)]
pub struct Rates {
    prev: Option<(Instant, Counters)>,
}

impl Rates {
    pub fn apply(&mut self, cur: Counters, system: &mut SystemStats, processes: &mut [Process]) {
        let now = Instant::now();
        if let Some((t0, prev)) = &self.prev {
            let dt = now.duration_since(*t0).as_secs_f64().max(1e-3);
            let d_total = cur.cpu_total.saturating_sub(prev.cpu_total).max(1) as f64;
            system.cpu_pct = cur.cpu_busy.saturating_sub(prev.cpu_busy) as f64 / d_total * 100.0;
            system.net_rx_bps = cur.net.0.saturating_sub(prev.net.0) as f64 / dt;
            system.net_tx_bps = cur.net.1.saturating_sub(prev.net.1) as f64 / dt;
            system.disk_read_bps = cur.disk.0.saturating_sub(prev.disk.0) as f64 / dt;
            system.disk_write_bps = cur.disk.1.saturating_sub(prev.disk.1) as f64 / dt;
            for p in processes.iter_mut() {
                let (ticks, rd, wr) = cur.procs.get(&p.pid).copied().unwrap_or_default();
                let (t0, r0, w0) = prev.procs.get(&p.pid).copied().unwrap_or_default();
                p.cpu_pct = ticks.saturating_sub(t0) as f64 / d_total * 100.0;
                p.disk_read_bps = rd.saturating_sub(r0) as f64 / dt;
                p.disk_write_bps = wr.saturating_sub(w0) as f64 / dt;
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
            procs: HashMap::from([(7, (ticks, 0, 0))]),
            ..Default::default()
        };
        r.apply(counters(50, 100, 10), &mut sys, &mut procs);
        assert_eq!(sys.cpu_pct, 0.0); // first sample has nothing to diff against
        r.apply(counters(75, 200, 60), &mut sys, &mut procs);
        assert_eq!((sys.cpu_pct, procs[0].cpu_pct), (25.0, 50.0));
    }
}
