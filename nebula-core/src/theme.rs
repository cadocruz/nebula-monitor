//! Paleta neon e ícones compartilhados por todos os painéis.
//!
//! O `[theme]` da configuração troca estas cores na tela já desenhada: um painel que usa
//! as constantes daqui segue o tema escolhido sem precisar conhecê-lo.

use ratatui::style::Color;

// Paleta de cores Cyberpunk/Neon Dark idêntica ao mockup
pub const CYAN_NEON: Color = Color::Rgb(0, 229, 255);
pub const CYAN_DIM: Color = Color::Rgb(20, 110, 140);
pub const GREEN_NEON: Color = Color::Rgb(0, 230, 118);
pub const MAGENTA_NEON: Color = Color::Rgb(224, 64, 251);
pub const YELLOW_NEON: Color = Color::Rgb(255, 214, 0);
pub const BLUE_NEON: Color = Color::Rgb(41, 121, 255);
pub const RED_ALERT: Color = Color::Rgb(255, 23, 68);
pub const BG_DARK: Color = Color::Rgb(7, 11, 18);
pub const BORDER_COLOR: Color = Color::Rgb(0, 210, 240);
pub const TEXT_WHITE: Color = Color::Rgb(235, 243, 250);
pub const TEXT_DIM: Color = Color::Rgb(95, 125, 145);
pub const BAR_EMPTY: Color = Color::Rgb(28, 40, 54);
/// Moldura do painel focado
pub const FOCUS_BORDER: Color = YELLOW_NEON;

// Ícones com paridade exata ao mockup e compatibilidade garantida com Nerd Fonts / FontAwesome / Unicode
pub fn icon_cpu(fallback: bool) -> &'static str {
    if fallback { "⚙" } else { "" }
}
pub fn icon_mem(fallback: bool) -> &'static str {
    if fallback { "💾" } else { "\u{efc5}" }
}
pub fn icon_gpu(fallback: bool) -> &'static str {
    if fallback { "🖥" } else { "\u{f108}" }
}
pub fn icon_disk(fallback: bool) -> &'static str {
    if fallback { "🖴" } else { "" }
}
pub fn icon_proc(fallback: bool) -> &'static str {
    if fallback { "⚡" } else { "" }
}
pub fn icon_net(fallback: bool) -> &'static str {
    if fallback { "🌐" } else { "\u{f0200}" }
}
pub fn icon_distro(distro: &str, fallback: bool) -> &'static str {
    if fallback {
        "🐧"
    } else {
        let d = distro.to_lowercase();
        if d.contains("ubuntu") {
            ""
        } else if d.contains("arch") {
            ""
        } else if d.contains("debian") {
            ""
        } else if d.contains("fedora") {
            ""
        } else {
            ""
        }
    }
}
