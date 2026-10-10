//! Tema configurável (`[theme]` do `config.toml`).
//!
//! Os painéis desenham com as constantes de `nebula_core::theme` (a paleta neon). Depois
//! que o quadro inteiro está desenhado, `Palette::apply` troca cada cor da paleta pela cor
//! do tema. Assim nenhum painel precisa conhecer o tema, e plugins que usam as mesmas
//! constantes seguem o tema sem mudar nada.
//!
//! Só as 12 cores da paleta mudam. Tons derivados (degradês dos gráficos, grades) e as
//! cores nomeadas do terminal (`Color::Black`...) ficam como estão.

use nebula_core::theme::{
    BAR_EMPTY, BG_DARK, BLUE_NEON, BORDER_COLOR, CYAN_DIM, CYAN_NEON, GREEN_NEON, MAGENTA_NEON,
    RED_ALERT, TEXT_DIM, TEXT_WHITE, YELLOW_NEON,
};
use ratatui::buffer::Buffer;
use ratatui::style::Color;

/// As 12 cores da paleta. Os nomes são as chaves de `[theme]` no `config.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub background: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub panel_border: Color,
    pub bar_track: Color,
    pub primary_accent: Color,
    pub separator: Color,
    pub status_good: Color,
    pub status_warning: Color,
    pub status_critical: Color,
    pub secondary_accent: Color,
    pub cache_usage: Color,
}

impl Palette {
    /// A paleta neon de sempre (a de `nebula_core::theme`)
    pub const NEON: Palette = Palette {
        background: BG_DARK,
        text_primary: TEXT_WHITE,
        text_secondary: TEXT_DIM,
        panel_border: BORDER_COLOR,
        bar_track: BAR_EMPTY,
        primary_accent: CYAN_NEON,
        separator: CYAN_DIM,
        status_good: GREEN_NEON,
        status_warning: YELLOW_NEON,
        status_critical: RED_ALERT,
        secondary_accent: MAGENTA_NEON,
        cache_usage: BLUE_NEON,
    };

    /// Cada cor com o nome da chave, na ordem da documentação
    pub fn entries_mut(&mut self) -> [(&'static str, &mut Color); 12] {
        [
            ("background", &mut self.background),
            ("text_primary", &mut self.text_primary),
            ("text_secondary", &mut self.text_secondary),
            ("panel_border", &mut self.panel_border),
            ("bar_track", &mut self.bar_track),
            ("primary_accent", &mut self.primary_accent),
            ("separator", &mut self.separator),
            ("status_good", &mut self.status_good),
            ("status_warning", &mut self.status_warning),
            ("status_critical", &mut self.status_critical),
            ("secondary_accent", &mut self.secondary_accent),
            ("cache_usage", &mut self.cache_usage),
        ]
    }

    fn colors(&self) -> [Color; 12] {
        [
            self.background,
            self.text_primary,
            self.text_secondary,
            self.panel_border,
            self.bar_track,
            self.primary_accent,
            self.separator,
            self.status_good,
            self.status_warning,
            self.status_critical,
            self.secondary_accent,
            self.cache_usage,
        ]
    }

    /// Troca, no quadro desenhado, cada cor da paleta neon pela cor deste tema
    pub fn apply(&self, buf: &mut Buffer) {
        if *self == Self::NEON {
            return;
        }
        let pairs: Vec<(Color, Color)> = Self::NEON
            .colors()
            .into_iter()
            .zip(self.colors())
            .filter(|(from, to)| from != to)
            .collect();
        let swap = |color: &mut Color| {
            if let Some((_, to)) = pairs.iter().find(|(from, _)| from == color) {
                *color = *to;
            }
        };
        for cell in &mut buf.content {
            swap(&mut cell.fg);
            swap(&mut cell.bg);
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::NEON
    }
}

/// `"#rrggbb"` → cor
pub fn parse_hex(text: &str) -> Option<Color> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(Color::Rgb(channel(0)?, channel(2)?, channel(4)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn hex_valido_e_invalido() {
        assert_eq!(parse_hex("#00e5FF"), Some(Color::Rgb(0, 229, 255)));
        for invalido in ["00e5ff", "#00e5f", "#00e5ffa", "#gg0000", "", "#"] {
            assert_eq!(parse_hex(invalido), None, "{invalido}");
        }
    }

    #[test]
    fn tema_troca_so_as_cores_da_paleta() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 1));
        buf[(0, 0)].set_fg(CYAN_NEON).set_bg(BG_DARK);
        buf[(1, 0)].set_fg(YELLOW_NEON).set_bg(BG_DARK);
        buf[(2, 0)].set_fg(Color::Rgb(1, 2, 3)).set_bg(Color::Black);

        let tema = Palette {
            primary_accent: Color::Rgb(255, 176, 0),
            background: Color::Rgb(16, 11, 5),
            ..Palette::NEON
        };
        tema.apply(&mut buf);
        assert_eq!(buf[(0, 0)].fg, Color::Rgb(255, 176, 0));
        assert_eq!(buf[(0, 0)].bg, Color::Rgb(16, 11, 5));
        assert_eq!(buf[(1, 0)].fg, YELLOW_NEON, "cor não configurada continua");
        assert_eq!(buf[(2, 0)].fg, Color::Rgb(1, 2, 3), "fora da paleta");
        assert_eq!(buf[(2, 0)].bg, Color::Black);
    }

    #[test]
    fn trocas_nao_se_encadeiam() {
        // ciano vira magenta e magenta vira ciano: cada célula muda uma vez só
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        buf[(0, 0)].set_fg(CYAN_NEON);
        buf[(1, 0)].set_fg(MAGENTA_NEON);
        Palette {
            primary_accent: MAGENTA_NEON,
            secondary_accent: CYAN_NEON,
            ..Palette::NEON
        }
        .apply(&mut buf);
        assert_eq!(buf[(0, 0)].fg, MAGENTA_NEON);
        assert_eq!(buf[(1, 0)].fg, CYAN_NEON);
    }

    #[test]
    fn as_doze_cores_da_paleta_sao_distintas() {
        // duas cores iguais na paleta neon não poderiam ser trocadas separadamente
        let cores = Palette::NEON.colors();
        for (i, a) in cores.iter().enumerate() {
            assert!(!cores[i + 1..].contains(a), "{a:?} repetida");
        }
    }
}
