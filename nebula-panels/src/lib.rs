//! Painéis nativos do nebula-monitor.
//!
//! Este crate só enxerga a API pública do `nebula-core` — a mesma que um plugin usaria.
//! Se um painel nativo precisar de algo que o core não oferece, o core é que deve crescer.

pub mod cpu;
pub mod disks;
pub mod gpu;
pub mod memory;
pub mod network;
pub mod processes;

pub use cpu::CpuPanel;
pub use disks::DisksPanel;
pub use gpu::GpuPanel;
pub use memory::MemoryPanel;
pub use network::NetworkPanel;
pub use processes::ProcessesPanel;
