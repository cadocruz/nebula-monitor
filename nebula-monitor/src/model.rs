use crate::theme::Palette;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryStatus {
    Charging,
    Discharging,
    Full,
    NotCharging,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BatteryInfo {
    pub percent: u8,
    pub status: BatteryStatus,
    /// Fonte externa conectada; None se o sistema não expõe um adaptador "Mains"
    pub ac_online: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct AppState {
    // Header info
    pub hostname: String,
    pub distro: String,
    pub kernel: String,
    pub uptime_str: String,
    pub datetime_str: String,
    /// None quando a máquina não tem bateria (desktop) — nada é exibido
    pub battery: Option<BatteryInfo>,

    // UI state
    pub show_help: bool,
    pub paused: bool,
    pub unicode_fallback_icons: bool,
    /// Cores do `[theme]` da configuração, aplicadas sobre o quadro desenhado
    pub palette: Palette,
}

impl Default for AppState {
    fn default() -> Self {
        let no_nerd =
            std::env::var("NEBULA_NO_ICONS").is_ok() || std::env::var("NO_NERD_FONT").is_ok();
        Self {
            hostname: "megadeth".to_string(),
            distro: "Arch Linux".to_string(),
            kernel: "6.8.9".to_string(),
            uptime_str: "0d 0h 0m".to_string(),
            datetime_str: "".to_string(),
            battery: None,

            show_help: false,
            paused: false,
            unicode_fallback_icons: no_nerd,
            palette: Palette::NEON,
        }
    }
}

impl AppState {
    pub fn demo() -> Self {
        let no_nerd =
            std::env::var("NEBULA_NO_ICONS").is_ok() || std::env::var("NO_NERD_FONT").is_ok();

        Self {
            hostname: "nebula-station".to_string(),
            distro: "Arch Linux (Cyberdeck)".to_string(),
            kernel: "6.12-zen1-1-zen".to_string(),
            uptime_str: "14d 6h 32m".to_string(),
            datetime_str: "2026-10-08 22:30".to_string(),
            battery: Some(BatteryInfo {
                percent: 100,
                status: BatteryStatus::Full,
                ac_online: Some(true),
            }),

            show_help: false,
            paused: false,
            unicode_fallback_icons: no_nerd,
            palette: Palette::NEON,
        }
    }
}
