//! Desenhos reutilizáveis: barras, sparklines Braille, medidor circular, gráficos e
//! utilitários de borda (títulos sem sobreposição, texto cortado na área).

use std::collections::VecDeque;
use std::f64::consts::PI;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::theme::*;

/// Renderiza barra de progresso segmentada [██████░░░░]
pub fn render_segmented_bar(pct: f64, width: usize, fill_color: Color) -> Line<'static> {
    let pct = pct.clamp(0.0, 100.0);
    let filled_count = ((pct / 100.0) * (width as f64)).round() as usize;
    let empty_count = width.saturating_sub(filled_count);

    Line::from(vec![
        Span::styled("█".repeat(filled_count), Style::default().fg(fill_color)),
        Span::styled("░".repeat(empty_count), Style::default().fg(BAR_EMPTY)),
    ])
}

/// Converte série temporal em sparkline Braille com largura exata (esticando suavemente)
pub fn braille_sparkline_len(
    values: &[f64],
    target_width: usize,
    fill_color: Color,
) -> Span<'static> {
    if target_width == 0 {
        return Span::styled("", Style::default().fg(fill_color));
    }
    let col0_levels = [0x00, 0x40, 0x04, 0x02, 0x01];
    let col1_levels = [0x00, 0x80, 0x20, 0x10, 0x08];

    let needed_points = target_width * 2;
    let mut sampled = Vec::with_capacity(needed_points);
    if values.is_empty() {
        sampled.resize(needed_points, 0.0);
    } else if values.len() == needed_points {
        sampled.extend_from_slice(values);
    } else if values.len() > needed_points {
        sampled.extend_from_slice(&values[values.len() - needed_points..]);
    } else {
        // Interpolação linear para preencher exatamente needed_points
        let v_len = values.len();
        for i in 0..needed_points {
            let t = (i as f64) / ((needed_points - 1).max(1) as f64);
            let idx_f = t * ((v_len - 1) as f64);
            let i0 = (idx_f.floor() as usize).min(v_len - 1);
            let i1 = (idx_f.ceil() as usize).min(v_len - 1);
            let frac = idx_f - (i0 as f64);
            let val = values[i0] * (1.0 - frac) + values[i1] * frac;
            sampled.push(val);
        }
    }

    let mut out = String::with_capacity(target_width);
    for chunk in sampled.chunks(2) {
        let v0 = chunk.first().copied().unwrap_or(0.0).clamp(0.0, 1.0);
        let l0 = (v0 * 4.0).round() as usize;
        let v1 = chunk.get(1).copied().unwrap_or(v0).clamp(0.0, 1.0);
        let l1 = (v1 * 4.0).round() as usize;

        let code = if l0 == 0 && l1 == 0 {
            0x2840 // Linha base suave
        } else {
            0x2800 + col0_levels[l0.min(4)] + col1_levels[l1.min(4)]
        };
        out.push(char::from_u32(code).unwrap_or('⠤'));
    }
    Span::styled(out, Style::default().fg(fill_color))
}

/// Converte série temporal em sparkline Braille suave de linha contorno
#[allow(dead_code)]
pub fn braille_sparkline(values: &[f64], fill_color: Color) -> Span<'static> {
    braille_sparkline_len(values, values.len().div_ceil(2), fill_color)
}

/// Sparkline vertical em blocos sólidos ( ▂▃▄▅▆▇█) idêntica à coluna de Atividade da imagem
#[allow(dead_code)]
pub fn block_sparkline(values: &[f64], fill_color: Color) -> Span<'static> {
    let levels = [' ', ' ', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let mut s = String::new();
    for &v in values.iter().take(14) {
        let idx = (v.clamp(0.0, 1.0) * 8.0).round() as usize;
        s.push(levels[idx.min(8)]);
    }
    Span::styled(s, Style::default().fg(fill_color))
}

/// Ajusta um histórico (valores 0..1) a `width` colunas de picos Braille. Cada coluna mostra
/// o pico do seu trecho do histórico, para nenhuma amostra (inclusive a mais recente) sumir.
pub fn braille_peaks_line(history: &VecDeque<f64>, width: usize) -> String {
    const PEAKS: [char; 7] = ['⠤', '⡠', '⣄', '⣆', '⣼', '⣾', '⣿'];
    let len = history.len();
    (0..width)
        .map(|i| {
            let val = if len == 0 {
                0.0
            } else {
                let start = (i * len / width).min(len - 1);
                let end = ((i + 1) * len / width).clamp(start + 1, len);
                history.range(start..end).copied().fold(0.0, f64::max)
            };
            PEAKS[((val.clamp(0.0, 1.0) * 6.0).round() as usize).min(6)]
        })
        .collect()
}

/// Medidor Circular de alta definição em Braille (sub-pixels 2x4) perfeitamente redondo, fluido e dinâmico
pub fn render_circular_gauge(
    buf: &mut Buffer,
    area: Rect,
    pct: f64,
    label: &str,
    sub_label: Option<&str>,
) {
    if area.width < 12 || area.height < 6 {
        return;
    }

    let w = area.width as usize;
    let h = area.height as usize;
    let sub_w = w * 2;
    let sub_h = h * 4;

    let cx = sub_w as f64 / 2.0;
    let cy = sub_h as f64 / 2.0;

    // Em fontes mono, 2 sub-colunas x 4 sub-linhas formam células aproximadamente quadradas (1:1).
    // O raio se adapta perfeitamente à área delimitada com margem suave.
    let radius = (cx.min(cy) - 2.2).max(5.0);
    let thickness = 1.6;

    let active_color = if pct < 60.0 {
        GREEN_NEON
    } else if pct < 85.0 {
        YELLOW_NEON
    } else {
        RED_ALERT
    };
    let track_color = Color::Rgb(28, 42, 58);

    // Mapeamento dos 8 pontos do bloco Braille:
    // bit 0: (0,0), bit 1: (0,1), bit 2: (0,2), bit 6: (0,3)
    // bit 3: (1,0), bit 4: (1,1), bit 5: (1,2), bit 7: (1,3)
    let dot_bits = [
        [0x01u32, 0x08u32],
        [0x02u32, 0x10u32],
        [0x04u32, 0x20u32],
        [0x40u32, 0x80u32],
    ];

    // Texto centralizado
    let pct_str = format!("{:.0}%", pct);
    let pct_x = area.x + (area.width.saturating_sub(pct_str.len() as u16) / 2);
    let pct_y = area.y + (area.height / 2).saturating_sub(1);

    let lbl_x = area.x + (area.width.saturating_sub(label.len() as u16) / 2);
    let lbl_y = pct_y + 1;

    let sub_info = sub_label.map(|sub| {
        let sx = area.x + (area.width.saturating_sub(sub.len() as u16) / 2);
        let sy = lbl_y + 1;
        (sub, sx, sy)
    });

    // Zona de exclusão de texto para não renderizar pontos Braille por trás das legendas
    let t_left = pct_x
        .min(lbl_x)
        .min(sub_info.map(|s| s.1).unwrap_or(u16::MAX))
        .saturating_sub(1);
    let t_right = (pct_x + pct_str.len() as u16)
        .max(lbl_x + label.len() as u16)
        .max(sub_info.map(|s| s.1 + s.0.len() as u16).unwrap_or(0))
        + 1;
    let t_top = pct_y;
    let t_bottom = sub_info.map(|s| s.2 + 1).unwrap_or(lbl_y + 1);

    // Rasterização por célula de caractere
    for cy_cell in 0..h {
        let cell_y = area.y + cy_cell as u16;
        for cx_cell in 0..w {
            let cell_x = area.x + cx_cell as u16;

            // Se coincide com a área do texto central, não desenha pontos por baixo
            if cell_x >= t_left && cell_x < t_right && cell_y >= t_top && cell_y < t_bottom {
                continue;
            }

            let mut code = 0u32;
            let mut filled_count = 0;

            for (dy, row_bits) in dot_bits.iter().enumerate() {
                for (dx, &bit) in row_bits.iter().enumerate() {
                    let px = cx_cell * 2 + dx;
                    let py = cy_cell * 4 + dy;

                    let d_x = (px as f64 + 0.5) - cx;
                    let d_y = (py as f64 + 0.5) - cy;
                    let dist = (d_x * d_x + d_y * d_y).sqrt();

                    if (dist - radius).abs() <= thickness {
                        let mut angle = d_x.atan2(-d_y);
                        if angle < 0.0 {
                            angle += 2.0 * PI;
                        }

                        let pct_angle = (angle / (2.0 * PI)) * 100.0;
                        code |= bit;
                        if pct_angle <= pct {
                            filled_count += 1;
                        }
                    }
                }
            }

            if code != 0 {
                let ch = char::from_u32(0x2800 + code).unwrap_or(' ');
                let color = if filled_count > 0 {
                    active_color
                } else {
                    track_color
                };
                if cell_x < area.right() && cell_y < area.bottom() {
                    buf[(cell_x, cell_y)].set_char(ch).set_fg(color);
                }
            }
        }
    }

    // Desenha o texto centralizado
    if pct_x < area.right() && pct_y < area.bottom() {
        buf.set_string(
            pct_x,
            pct_y,
            &pct_str,
            Style::default()
                .fg(active_color)
                .add_modifier(Modifier::BOLD),
        );
    }
    if lbl_x < area.right() && lbl_y < area.bottom() {
        buf.set_string(
            lbl_x,
            lbl_y,
            label,
            Style::default().fg(TEXT_WHITE).add_modifier(Modifier::DIM),
        );
    }
    if let Some((sub, sx, sy)) = sub_info
        && sx < area.right()
        && sy < area.bottom()
    {
        buf.set_string(sx, sy, sub, Style::default().fg(CYAN_NEON));
    }
}

/// Renderiza histograma/espectrograma suave com retículo de osciloscópio (┄/┆/┌/┐/└/┘) e barras equalizadoras verticais
pub fn render_solid_block_chart(
    buf: &mut Buffer,
    area: Rect,
    data: &[f64],
    gradient: bool,
    max_label: &str,
    mid_label: Option<&str>,
    min_label: &str,
) {
    if area.width < 10 || area.height < 3 {
        return;
    }

    let label_width = 5;
    let chart_left = area.left() + label_width as u16;
    let chart_right = area.right();
    let chart_width = chart_right.saturating_sub(chart_left) as usize;
    let height = area.height as usize;
    let grid_color = Color::Rgb(35, 65, 85);

    if chart_width < 4 {
        return;
    }

    // Rótulos do eixo Y (alinhados à esquerda da caixa)
    let max_lbl_y = area.top();
    let min_lbl_y = area.bottom().saturating_sub(1);
    let mid_y = area.top() + (height as u16 / 2);

    buf.set_string(
        area.left(),
        max_lbl_y,
        format!("{:>4} ", max_label),
        Style::default().fg(TEXT_DIM),
    );
    if let Some(mid) = mid_label
        && height >= 4
    {
        buf.set_string(
            area.left(),
            mid_y,
            format!("{:>4} ", mid),
            Style::default().fg(TEXT_DIM),
        );
    }
    buf.set_string(
        area.left(),
        min_lbl_y,
        format!("{:>4} ", min_label),
        Style::default().fg(TEXT_DIM),
    );

    // Moldura do osciloscópio
    buf[(chart_left, max_lbl_y)]
        .set_char('┌')
        .set_fg(grid_color);
    for col in 1..chart_width - 1 {
        let ch = if col % 6 == 0 { '┬' } else { '┄' };
        buf[(chart_left + col as u16, max_lbl_y)]
            .set_char(ch)
            .set_fg(grid_color);
    }
    buf[(chart_right - 1, max_lbl_y)]
        .set_char('┐')
        .set_fg(grid_color);

    buf[(chart_left, min_lbl_y)]
        .set_char('└')
        .set_fg(grid_color);
    for col in 1..chart_width - 1 {
        let ch = if col % 6 == 0 { '┴' } else { '┄' };
        buf[(chart_left + col as u16, min_lbl_y)]
            .set_char(ch)
            .set_fg(grid_color);
    }
    buf[(chart_right - 1, min_lbl_y)]
        .set_char('┘')
        .set_fg(grid_color);

    for r in 1..height - 1 {
        let py = area.top() + r as u16;
        buf[(chart_left, py)].set_char('│').set_fg(grid_color);
        buf[(chart_right - 1, py)].set_char('│').set_fg(grid_color);
    }

    if mid_label.is_some() && height >= 4 {
        buf[(chart_left, mid_y)].set_char('├').set_fg(grid_color);
        for col in 1..chart_width - 1 {
            let ch = if col % 6 == 0 { '┼' } else { '┄' };
            buf[(chart_left + col as u16, mid_y)]
                .set_char(ch)
                .set_fg(grid_color);
        }
        buf[(chart_right - 1, mid_y)]
            .set_char('┤')
            .set_fg(grid_color);
    }

    let inner_w = chart_width.saturating_sub(2);
    let inner_h = height.saturating_sub(2);
    if inner_w == 0 || inner_h == 0 {
        return;
    }

    let total_steps = inner_h * 8;
    let block_levels = [' ', ' ', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

    // Amostragem dos dados para preencher exatamente inner_w colunas
    let mut sampled_values = Vec::with_capacity(inner_w);
    if data.is_empty() {
        sampled_values.resize(inner_w, 0.0);
    } else if data.len() >= inner_w {
        let start = data.len() - inner_w;
        sampled_values.extend_from_slice(&data[start..]);
    } else {
        for i in 0..inner_w {
            let t = (i as f64) / ((inner_w - 1).max(1) as f64);
            let idx_f = t * ((data.len() - 1) as f64);
            let idx0 = idx_f.floor() as usize;
            let idx1 = (idx0 + 1).min(data.len() - 1);
            let frac = idx_f - idx0 as f64;
            let val = data[idx0] * (1.0 - frac) + data[idx1] * frac;
            sampled_values.push(val);
        }
    }

    for (col_idx, &v) in sampled_values.iter().enumerate() {
        let val = v.clamp(0.0, 1.0);
        let steps = (val * total_steps as f64).round() as usize;
        let px = chart_left + 1 + col_idx as u16;
        let is_odd = col_idx % 2 == 1;

        for r in 0..inner_h {
            let row_from_bottom = inner_h - 1 - r;
            let bottom_step = row_from_bottom * 8;
            let py = area.top() + 1 + r as u16;

            let ch = if steps >= bottom_step + 8 {
                '█'
            } else if steps > bottom_step {
                block_levels[steps - bottom_step]
            } else {
                ' '
            };

            if ch != ' ' {
                let cell_color = if gradient {
                    let row_pct = (row_from_bottom as f64) / (inner_h as f64);
                    if row_pct < 0.50 {
                        if is_odd {
                            Color::Rgb(0, 190, 95)
                        } else {
                            GREEN_NEON
                        }
                    } else if row_pct < 0.75 {
                        if is_odd {
                            Color::Rgb(210, 175, 0)
                        } else {
                            YELLOW_NEON
                        }
                    } else {
                        if is_odd {
                            Color::Rgb(215, 20, 55)
                        } else {
                            RED_ALERT
                        }
                    }
                } else {
                    let row_pct = (row_from_bottom as f64) / (inner_h as f64);
                    if row_pct < 0.40 {
                        if is_odd {
                            Color::Rgb(110, 20, 130)
                        } else {
                            Color::Rgb(140, 25, 160)
                        }
                    } else if row_pct < 0.75 {
                        if is_odd {
                            Color::Rgb(170, 40, 190)
                        } else {
                            MAGENTA_NEON
                        }
                    } else {
                        if is_odd {
                            Color::Rgb(210, 70, 230)
                        } else {
                            Color::Rgb(245, 90, 255)
                        }
                    }
                };
                buf[(px, py)].set_char(ch).set_fg(cell_color);
            } else {
                if py == mid_y && (col_idx + 1) % 6 == 0 {
                    buf[(px, py)].set_char('┼').set_fg(grid_color);
                } else if py == mid_y {
                    buf[(px, py)].set_char('┄').set_fg(grid_color);
                } else if (col_idx + 1) % 6 == 0 {
                    buf[(px, py)].set_char('┆').set_fg(grid_color);
                } else {
                    buf[(px, py)].set_char(' ');
                }
            }
        }
    }
}

/// Renderiza gráfico de linha em Braille suave para histórico geral de CPU com retículo
pub fn render_braille_line_chart(
    buf: &mut Buffer,
    area: Rect,
    data: &[f64],
    color: Color,
    max_label: &str,
    min_label: &str,
) {
    if area.width < 10 || area.height < 3 {
        return;
    }

    let label_width = 5;
    let chart_left = area.left() + label_width as u16;
    let chart_right = area.right();
    let chart_width = chart_right.saturating_sub(chart_left) as usize;
    let height = area.height as usize;
    let grid_color = Color::Rgb(30, 60, 80);

    if chart_width < 4 {
        return;
    }

    let max_lbl_y = area.top();
    let min_lbl_y = area.bottom().saturating_sub(1);

    buf.set_string(
        area.left(),
        max_lbl_y,
        format!("{:>4} ", max_label),
        Style::default().fg(TEXT_DIM),
    );
    buf.set_string(
        area.left(),
        min_lbl_y,
        format!("{:>4} ", min_label),
        Style::default().fg(TEXT_DIM),
    );

    // Moldura do retículo
    buf[(chart_left, max_lbl_y)]
        .set_char('┌')
        .set_fg(grid_color);
    for col in 1..chart_width - 1 {
        let ch = if col % 6 == 0 { '┬' } else { '┄' };
        buf[(chart_left + col as u16, max_lbl_y)]
            .set_char(ch)
            .set_fg(grid_color);
    }
    buf[(chart_right - 1, max_lbl_y)]
        .set_char('┐')
        .set_fg(grid_color);

    buf[(chart_left, min_lbl_y)]
        .set_char('└')
        .set_fg(grid_color);
    for col in 1..chart_width - 1 {
        let ch = if col % 6 == 0 { '┴' } else { '┄' };
        buf[(chart_left + col as u16, min_lbl_y)]
            .set_char(ch)
            .set_fg(grid_color);
    }
    buf[(chart_right - 1, min_lbl_y)]
        .set_char('┘')
        .set_fg(grid_color);

    for r in 1..height - 1 {
        let py = area.top() + r as u16;
        buf[(chart_left, py)].set_char('│').set_fg(grid_color);
        buf[(chart_right - 1, py)].set_char('│').set_fg(grid_color);
    }

    let inner_w = chart_width.saturating_sub(2);
    let inner_h = height.saturating_sub(2);
    if inner_w == 0 || inner_h == 0 {
        return;
    }

    let sub_w = inner_w * 2;
    let sub_h = inner_h * 4;

    let mut sampled = Vec::with_capacity(sub_w);
    if data.is_empty() {
        sampled.resize(sub_w, 0.25);
    } else {
        for i in 0..sub_w {
            let t = (i as f64) / ((sub_w - 1).max(1) as f64);
            let idx_f = t * ((data.len() - 1) as f64);
            let idx0 = idx_f.floor() as usize;
            let idx1 = (idx0 + 1).min(data.len() - 1);
            let frac = idx_f - idx0 as f64;
            let v = data[idx0] * (1.0 - frac) + data[idx1] * frac;
            sampled.push(v.clamp(0.04, 0.96));
        }
    }

    let points: Vec<usize> = sampled
        .iter()
        .map(|&v| (v * (sub_h.saturating_sub(1) as f64)).round() as usize)
        .collect();

    let col0_dots = [0x40, 0x04, 0x02, 0x01];
    let col1_dots = [0x80, 0x20, 0x10, 0x08];

    for cx in 0..inner_w {
        let p0 = points[cx * 2];
        let p1 = points[cx * 2 + 1];

        let prev_p = if cx > 0 { points[cx * 2 - 1] } else { p0 };
        let next_p = if cx * 2 + 2 < points.len() {
            points[cx * 2 + 2]
        } else {
            p1
        };

        let mid_prev = (p0 + prev_p) / 2;
        let mid_next = (p1 + next_p) / 2;

        let col0_min = p0.min(mid_prev);
        let col0_max = p0.max(mid_prev);
        let col1_min = p1.min(mid_next);
        let col1_max = p1.max(mid_next);

        for cy in 0..inner_h {
            let row_from_bottom = inner_h - 1 - cy;
            let bot_dot = row_from_bottom * 4;

            let mut code = 0x2800;

            for d in 0..4 {
                let dot_y = bot_dot + d;
                if dot_y >= col0_min && dot_y <= col0_max {
                    code |= col0_dots[d];
                }
                if dot_y >= col1_min && dot_y <= col1_max {
                    code |= col1_dots[d];
                }
            }

            let px = chart_left + 1 + cx as u16;
            let py = area.top() + 1 + cy as u16;

            if code != 0x2800 {
                let ch = char::from_u32(code).unwrap_or('·');
                buf[(px, py)].set_char(ch).set_fg(color);
            } else {
                if (cx + 1) % 6 == 0 {
                    buf[(px, py)].set_char('┆').set_fg(grid_color);
                } else {
                    buf[(px, py)].set_char(' ');
                }
            }
        }
    }
}

/// Escreve `text` em (x, y) sem passar da borda direita de `area`
pub fn set_string_clipped(buf: &mut Buffer, area: Rect, x: u16, y: u16, text: &str, style: Style) {
    if x < area.right() && y < area.bottom() {
        buf.set_stringn(x, y, text, (area.right() - x) as usize, style);
    }
}

/// Desenha o título à esquerda e o resumo à direita na borda superior sem sobreposição:
/// o título da esquerda é truncado com "…"; se nem assim couber, o da direita é omitido.
pub fn render_panel_titles(buf: &mut Buffer, area: Rect, left: &Line, right: Option<&Line>) {
    const MIN_LEFT: u16 = 12;
    if area.width < 4 {
        return;
    }
    let y = area.top();
    let left_x = area.left() + 1;
    let usable = area.width - 2; // entre os cantos arredondados

    let right_w = right.map(|r| r.width() as u16).unwrap_or(0);
    let show_right = right.is_some() && usable >= right_w + 1 + MIN_LEFT;
    let left_max = if show_right {
        usable - right_w - 1
    } else {
        usable
    };

    let left_w = left.width() as u16;
    buf.set_line(left_x, y, left, left_max);
    if left_w > left_max && left_max > 0 {
        buf[(left_x + left_max - 1, y)].set_char('…');
    }

    if let (true, Some(r)) = (show_right, right) {
        buf.set_line(area.right() - 1 - right_w, y, r, right_w);
    }
}

/// Recolore só a moldura de `area` (caracteres de borda), sem tocar no texto dos títulos
/// que os painéis escrevem por cima dela. Usado para destacar o painel focado.
pub fn highlight_border(buf: &mut Buffer, area: Rect, color: Color) {
    const BORDER_CHARS: &str = "─│╭╮╰╯┌┐└┘";
    if area.width == 0 || area.height == 0 {
        return;
    }
    let (left, right) = (area.left(), area.right() - 1);
    let (top, bottom) = (area.top(), area.bottom() - 1);
    let mut paint = |x: u16, y: u16| {
        let cell = &mut buf[(x, y)];
        let symbol = cell.symbol();
        if !symbol.is_empty() && symbol.chars().all(|c| BORDER_CHARS.contains(c)) {
            cell.set_fg(color);
        }
    };
    for x in left..=right {
        paint(x, top);
        paint(x, bottom);
    }
    for y in top..=bottom {
        paint(left, y);
        paint(right, y);
    }
}
