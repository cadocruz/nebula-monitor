//! Painel de Memória: uso de RAM, cache, buffers, swap e histórico.

use std::collections::VecDeque;
use std::fs;

use nebula_core::ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Widget},
};
use nebula_core::theme::*;
use nebula_core::widgets::*;
use nebula_core::{Context, Panel, Size, View};

const GB: f64 = 1024.0 * 1024.0 * 1024.0;
const HISTORY_LEN: usize = 80;

pub struct MemoryPanel {
    /// Texto ao lado do título (ex.: "16 GB")
    specs: String,
    used: u64,
    total: u64,
    free: u64,
    cache: u64,
    buffers: u64,
    swap_used: u64,
    swap_total: u64,
    /// Fração de RAM usada (0..1) a cada ciclo
    history: VecDeque<f64>,
}

impl Default for MemoryPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryPanel {
    pub fn new() -> Self {
        Self {
            specs: String::new(),
            used: 0,
            total: 0,
            free: 0,
            cache: 0,
            buffers: 0,
            swap_used: 0,
            swap_total: 0,
            history: VecDeque::from(vec![0.0; HISTORY_LEN]),
        }
    }

    /// Dados fictícios do modo demonstração
    pub fn demo() -> Self {
        let history = (0..HISTORY_LEN)
            .map(|i| {
                let t = i as f64 * 0.08;
                (0.55 + 0.15 * (t * 0.4).sin() + 0.08 * (t * 1.1).cos()).clamp(0.2, 0.85)
            })
            .collect();
        Self {
            specs: "64 GB DDR5 6000 MHz".to_string(),
            used: 24 * 1024 * 1024 * 1024 + 600 * 1024 * 1024,
            total: 64 * 1024 * 1024 * 1024,
            free: 28 * 1024 * 1024 * 1024,
            cache: 10 * 1024 * 1024 * 1024,
            buffers: 1024 * 1024 * 1024,
            swap_used: 800 * 1024 * 1024,
            swap_total: 32 * 1024 * 1024 * 1024,
            history,
        }
    }
}

impl Panel for MemoryPanel {
    fn id(&self) -> &'static str {
        "memory"
    }

    fn update(&mut self, ctx: &Context) {
        let sys = ctx.system;
        self.total = sys.total_memory();
        self.used = sys.used_memory();
        self.free = sys.free_memory();

        let extra = fs::read_to_string("/proc/meminfo")
            .map(|content| parse_meminfo(&content))
            .unwrap_or_default();
        self.cache = extra.cache;
        self.buffers = extra.buffers;
        (self.swap_total, self.swap_used) = match (extra.swap_total, extra.swap_free) {
            (Some(total), Some(free)) => (total, total.saturating_sub(free)),
            _ => (sys.total_swap(), sys.used_swap()),
        };

        // Tipo e frequência da memória (DDR4/DDR5, MHz) exigem SMBIOS/root; mostramos só o total
        self.specs = format!("{:.0} GB", (self.total as f64 / GB).round());

        let ratio = if self.total > 0 {
            self.used as f64 / self.total as f64
        } else {
            0.0
        };
        if self.history.len() >= HISTORY_LEN {
            self.history.pop_front();
        }
        self.history.push_back(ratio.clamp(0.0, 1.0));
    }

    fn render(&self, area: Rect, buf: &mut Buffer, view: &View) {
        let used_gb = self.used as f64 / GB;
        let tot_gb = self.total as f64 / GB;
        let pct = if tot_gb > 0.0 {
            (used_gb / tot_gb) * 100.0
        } else {
            0.0
        };

        let m_icon = format!(" {} MEMÓRIA ", icon_mem(view.unicode_icons));
        let title = Line::from(vec![
            Span::styled(
                m_icon,
                Style::default()
                    .fg(MAGENTA_NEON)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{}   ", self.specs),
                Style::default().fg(TEXT_WHITE),
            ),
        ]);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER_COLOR))
            .title(title);
        let inner = block.inner(area);
        block.render(area, buf);

        // Informação de uso na borda superior direita
        let r_title = Line::from(vec![Span::styled(
            format!("Uso: {:.0}% | {:.1} / {:.1} GB ", pct, used_gb, tot_gb),
            Style::default().fg(CYAN_NEON),
        )]);
        let r_len = 26;
        if area.width > 60 {
            buf.set_line(
                area.right().saturating_sub(r_len + 2),
                area.top(),
                &r_title,
                r_len,
            );
        }

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(40), Constraint::Min(16)])
            .split(inner);

        let left_area = cols[0];
        let free_gb = self.free as f64 / GB;
        let cache_gb = self.cache as f64 / GB;
        let buf_gb = self.buffers as f64 / GB;
        let swap_u_gb = self.swap_used as f64 / GB;
        let swap_t_gb = self.swap_total as f64 / GB;
        let swap_pct = if swap_t_gb > 0.0 {
            (swap_u_gb / swap_t_gb) * 100.0
        } else {
            0.0
        };

        let items = [
            ("Usada", used_gb, (used_gb / tot_gb) * 100.0, MAGENTA_NEON),
            ("Livre", free_gb, (free_gb / tot_gb) * 100.0, GREEN_NEON),
            ("Cache", cache_gb, (cache_gb / tot_gb) * 100.0, BLUE_NEON),
            ("Buffers", buf_gb, (buf_gb / tot_gb) * 100.0, CYAN_DIM),
        ];

        for (idx, (label, gb, p, col)) in items.iter().enumerate() {
            let y = left_area.top() + (idx as u16);
            if y >= left_area.bottom() {
                break;
            }

            buf.set_string(
                left_area.left(),
                y,
                format!("{:<7}", label),
                Style::default().fg(TEXT_WHITE),
            );
            buf.set_string(
                left_area.left() + 8,
                y,
                format!("{:>4.1} GB", gb),
                Style::default().fg(*col).add_modifier(Modifier::BOLD),
            );

            let bar_line = render_segmented_bar(*p, 14, *col);
            let mut cur_x = left_area.left() + 18;
            for span in &bar_line.spans {
                buf.set_string(cur_x, y, &span.content, span.style);
                cur_x += span.content.chars().count() as u16;
            }
            buf.set_string(
                cur_x + 1,
                y,
                format!("{:>3.0}%", p),
                Style::default().fg(TEXT_WHITE),
            );
        }

        let swap_y = left_area.top() + 5;
        if swap_y < left_area.bottom() {
            buf.set_string(
                left_area.left(),
                swap_y,
                "Swap",
                Style::default().fg(TEXT_WHITE),
            );
            buf.set_string(
                left_area.left() + 6,
                swap_y,
                format!("{:.1} / {:.1} GB ({:.0}%)", swap_u_gb, swap_t_gb, swap_pct),
                Style::default().fg(YELLOW_NEON),
            );
            let swap_bar = render_segmented_bar(swap_pct, 12, YELLOW_NEON);
            let mut cur_x = left_area.left() + 24;
            for span in &swap_bar.spans {
                buf.set_string(cur_x, swap_y, &span.content, span.style);
                cur_x += span.content.chars().count() as u16;
            }
        }

        // Direita: Histórico sólido de RAM em Magenta
        let right_area = cols[1];
        buf.set_string(
            right_area.left(),
            right_area.top(),
            "Histórico (uso de RAM)",
            Style::default().fg(CYAN_NEON),
        );
        let ram_slice: Vec<f64> = self.history.iter().copied().collect();
        let chart_box = Rect::new(
            right_area.left(),
            right_area.top() + 1,
            right_area.width,
            right_area.height.saturating_sub(1),
        );
        render_solid_block_chart(
            buf,
            chart_box,
            &ram_slice,
            false,
            &format!("{:.0}G", tot_gb),
            Some(&format!("{:.0}G", tot_gb / 2.0)),
            "0G",
        );
    }

    fn min_size(&self) -> Size {
        // 40 colunas de números + 16 de gráfico + bordas; 6 linhas de dados + bordas
        Size {
            width: 58,
            height: 8,
        }
    }
}

/// Campos de /proc/meminfo que o sysinfo não expõe (valores em bytes)
#[derive(Debug, Default, PartialEq, Eq)]
struct MemInfoExtra {
    cache: u64,
    buffers: u64,
    swap_total: Option<u64>,
    swap_free: Option<u64>,
}

/// Extrai cache (Cached + SReclaimable), buffers e swap de /proc/meminfo (valores em kB)
fn parse_meminfo(content: &str) -> MemInfoExtra {
    let mut info = MemInfoExtra::default();
    for line in content.lines() {
        let mut parts = line.split_whitespace();
        let (Some(key), Some(value)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Ok(kb) = value.parse::<u64>() else {
            continue;
        };
        let bytes = kb * 1024;
        match key.trim_end_matches(':') {
            "Cached" | "SReclaimable" => info.cache += bytes,
            "Buffers" => info.buffers = bytes,
            "SwapTotal" => info.swap_total = Some(bytes),
            "SwapFree" => info.swap_free = Some(bytes),
            _ => {}
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMINFO: &str = "\
MemTotal:       16314564 kB
MemFree:         1203456 kB
MemAvailable:    6543210 kB
Buffers:          123456 kB
Cached:          2000000 kB
SwapCached:         4096 kB
SwapTotal:       8388604 kB
SwapFree:        8000000 kB
SReclaimable:     300000 kB
HugePages_Total:       0
";

    #[test]
    fn meminfo_soma_cache_e_le_swap() {
        let info = parse_meminfo(MEMINFO);
        assert_eq!(
            info.cache,
            (2_000_000 + 300_000) * 1024,
            "Cached + SReclaimable, sem SwapCached"
        );
        assert_eq!(info.buffers, 123_456 * 1024);
        assert_eq!(info.swap_total, Some(8_388_604 * 1024));
        assert_eq!(info.swap_free, Some(8_000_000 * 1024));
    }

    #[test]
    fn meminfo_vazio_ou_invalido_nao_inventa_swap() {
        assert_eq!(parse_meminfo(""), MemInfoExtra::default());
        assert_eq!(
            parse_meminfo("SwapTotal: muito kB\nlixo"),
            MemInfoExtra::default()
        );
    }
}
