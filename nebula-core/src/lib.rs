//! SDK do nebula-monitor: o que um painel (nativo ou plugin) precisa para ter a mesma cara
//! dos painéis de fábrica.
//!
//! Re-exporta `ratatui`, `crossterm`, `sysinfo` e `nvml_wrapper` para que todo painel compile contra as
//! mesmas versões que o monitor usa — tipos de versões diferentes não seriam compatíveis.

pub mod isolation;
pub mod panel;
pub mod theme;
pub mod widgets;
pub mod worker;

pub use isolation::{isolate, panicking_isolated};
pub use panel::{Context, Handled, KeyHint, Panel, Size, View};
pub use worker::Worker;

pub use crossterm;
pub use nvml_wrapper;
pub use ratatui;
pub use sysinfo;
