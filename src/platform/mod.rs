//! OS backends. Each backend lives in its own folder and only has to implement `Collector`.
//! To port to another OS: add `platform/<os>/`, implement `Collector`, and select it below.

use crate::model::Snapshot;

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

#[cfg(not(target_os = "linux"))]
compile_error!("nstop currently has only a Linux backend; add one under src/platform/<os>/");
