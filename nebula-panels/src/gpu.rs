//! Painel de GPU (NVIDIA via NVML): medidor, VRAM, temperatura, potência, clocks, ventoinha
//! e histórico. Cada leitura da NVML pode falhar isoladamente e vira "N/D" na tela.

use std::collections::VecDeque;

use nebula_core::crossterm::event::{KeyCode, KeyEvent};
use nebula_core::nvml_wrapper::Nvml;
use nebula_core::nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use nebula_core::ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Widget},
};
use nebula_core::theme::*;
use nebula_core::widgets::*;
use nebula_core::{Context, Handled, KeyHint, Panel, Size, View};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuGaugeMode {
    Vram,
    Core,
}

#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub name: String,
    pub driver_version: Option<String>,
    /// Link PCIe atual, ex.: "PCIe 4.0 x16" (None se a NVML não informar)
    pub pcie_info: Option<String>,
    // Cada leitura da NVML pode falhar isoladamente: None vira "N/D" na tela
    pub tdp_watts: Option<u32>,
    pub power_watts: Option<f64>,
    pub temp_c: Option<u32>,
    pub core_util: Option<f64>,
    pub vram_used_bytes: Option<u64>,
    pub vram_total_bytes: Option<u64>,
    pub clock_core_mhz: Option<u32>,
    pub clock_mem_mhz: Option<u32>,
    /// None em placas sem ventoinha controlada pelo driver (ex.: notebooks)
    pub fan_speed_pct: Option<u32>,
    /// RPM medido; None quando o driver/placa não expõe o tacômetro
    pub fan_rpm: Option<u32>,
    pub history: VecDeque<f64>,
    pub vram_history: VecDeque<f64>,
}

/// Quantas placas NVIDIA o registro expõe (ids gpu, gpu2, gpu3, gpu4)
pub const MAX_GPUS: u32 = 4;

pub struct GpuPanel {
    /// Índice da placa na NVML (0 = primeira)
    index: u32,
    /// None quando a placa não existe (ou não há NVML) — o painel mostra o aviso
    info: Option<GpuInfo>,
    gauge_mode: GpuGaugeMode,
}

impl Default for GpuPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuPanel {
    /// Primeira placa (id "gpu")
    pub fn new() -> Self {
        Self::with_index(0)
    }

    /// Placa `index` da NVML (0 a MAX_GPUS - 1)
    pub fn with_index(index: u32) -> Self {
        Self {
            index: index.min(MAX_GPUS - 1),
            info: None,
            gauge_mode: GpuGaugeMode::Core,
        }
    }

    /// Dados fictícios do modo demonstração (primeira placa)
    pub fn demo() -> Self {
        Self::demo_with_index(0)
    }

    /// Dados fictícios do modo demonstração para a placa `index`
    pub fn demo_with_index(index: u32) -> Self {
        let index = index.min(MAX_GPUS - 1);
        let mut gpu_hist = VecDeque::new();
        let mut vram_hist = VecDeque::new();
        for i in 0..80 {
            let t = i as f64 * 0.12;
            gpu_hist.push_back((0.68 + 0.22 * (t * 0.6).sin()).clamp(0.15, 0.98));
            vram_hist.push_back((0.62 + 0.10 * (t * 0.3).cos()).clamp(0.3, 0.85));
        }

        let info = Some(GpuInfo {
            name: if index == 0 {
                "NVIDIA GeForce RTX 4090 24GB".to_string()
            } else {
                "NVIDIA GeForce RTX 4070 12GB".to_string()
            },
            driver_version: Some("570.86.16".to_string()),
            pcie_info: Some("PCIe 4.0 x16".to_string()),
            tdp_watts: Some(450),
            power_watts: Some(215.0),
            temp_c: Some(52),
            core_util: Some(68.0),
            vram_used_bytes: Some(14_800_000_000),
            vram_total_bytes: Some(24_576_000_000),
            clock_core_mhz: Some(2520),
            clock_mem_mhz: Some(10500),
            fan_speed_pct: Some(42),
            fan_rpm: Some(1380),
            history: gpu_hist,
            vram_history: vram_hist,
        });
        Self {
            index,
            info,
            gauge_mode: GpuGaugeMode::Core,
        }
    }

    fn collect(&mut self, nvml: Option<&Nvml>) {
        if let Some(nvml) = nvml
            && let Ok(device) = nvml.device_by_index(self.index)
        {
            // Cada leitura falha de forma independente (driver antigo, placa sem sensor);
            // o que faltar vira None e a tela mostra "N/D" em vez de um número inventado.
            let name = device.name().unwrap_or_else(|_| "GPU NVIDIA".to_string());
            let driver = nvml.sys_driver_version().ok();
            let temp = device.temperature(TemperatureSensor::Gpu).ok();
            let power_w = device.power_usage().ok().map(|mw| mw as f64 / 1000.0);
            let tdp_w = device.enforced_power_limit().ok().map(|mw| mw / 1000);
            let (vram_used, vram_total) = match device.memory_info() {
                Ok(mem) => (Some(mem.used), Some(mem.total)),
                Err(_) => (None, None),
            };
            let util = device.utilization_rates().ok().map(|u| u.gpu as f64);
            let clock_core = device.clock_info(Clock::Graphics).ok();
            let clock_mem = device.clock_info(Clock::Memory).ok();
            let fan_pct = device.fan_speed(0).ok();
            let fan_rpm = device.fan_speed_rpm(0).ok();
            let pcie = match (
                device.current_pcie_link_gen(),
                device.current_pcie_link_width(),
            ) {
                (Ok(gen_), Ok(width)) => Some(format!("PCIe {}.0 x{}", gen_, width)),
                _ => None,
            };

            let vram_ratio = match (vram_used, vram_total) {
                (Some(used), Some(total)) if total > 0 => {
                    Some((used as f64 / total as f64).clamp(0.0, 1.0))
                }
                _ => None,
            };
            let (mut core_hist, mut vram_hist) = if let Some(ref old_gpu) = self.info {
                (old_gpu.history.clone(), old_gpu.vram_history.clone())
            } else {
                (VecDeque::from(vec![0.0; 80]), VecDeque::from(vec![0.0; 80]))
            };

            // Só registra no histórico o que foi de fato medido
            if let Some(u) = util {
                if core_hist.len() >= 80 {
                    core_hist.pop_front();
                }
                core_hist.push_back((u / 100.0).clamp(0.0, 1.0));
            }
            if let Some(ratio) = vram_ratio {
                if vram_hist.len() >= 80 {
                    vram_hist.pop_front();
                }
                vram_hist.push_back(ratio);
            }

            self.info = Some(GpuInfo {
                name,
                driver_version: driver,
                pcie_info: pcie,
                tdp_watts: tdp_w,
                power_watts: power_w,
                temp_c: temp,
                core_util: util,
                vram_used_bytes: vram_used,
                vram_total_bytes: vram_total,
                clock_core_mhz: clock_core,
                clock_mem_mhz: clock_mem,
                fan_speed_pct: fan_pct,
                fan_rpm,
                history: core_hist,
                vram_history: vram_hist,
            });
            return;
        }
        self.info = None;
    }
}

const KEYS: &[KeyHint] = &[KeyHint {
    key: "g",
    label: "GPU VRAM/Core",
    description: "Medidor da GPU: VRAM alocada ↔ uso do Core",
}];

impl Panel for GpuPanel {
    fn id(&self) -> &'static str {
        match self.index {
            0 => "gpu",
            1 => "gpu2",
            2 => "gpu3",
            _ => "gpu4",
        }
    }

    fn available(&self) -> bool {
        self.info.is_some()
    }

    fn update(&mut self, ctx: &Context) {
        self.collect(ctx.nvml);
    }

    fn render(&self, area: Rect, buf: &mut Buffer, view: &View) {
        render_gpu(
            self.info.as_ref(),
            self.index,
            self.gauge_mode,
            view.unicode_icons,
            buf,
            area,
        );
    }

    fn handle_key(&mut self, key: KeyEvent) -> Handled {
        if key.code != KeyCode::Char('g') {
            return Handled::No;
        }
        // Alterna medidor da GPU: VRAM (alocada) <-> Core (computação SM)
        self.gauge_mode = match self.gauge_mode {
            GpuGaugeMode::Vram => GpuGaugeMode::Core,
            GpuGaugeMode::Core => GpuGaugeMode::Vram,
        };
        Handled::Yes
    }

    fn keybindings(&self) -> &[KeyHint] {
        KEYS
    }

    fn min_size(&self) -> Size {
        // medidor (20) + métricas (38) + bordas; o gráfico só aparece se sobrar espaço
        Size {
            width: 60,
            height: 8,
        }
    }
}

fn render_gpu(
    info: Option<&GpuInfo>,
    index: u32,
    mode: GpuGaugeMode,
    unicode_icons: bool,
    buf: &mut Buffer,
    area: Rect,
) {
    // "GPU" para a primeira placa (como sempre), "GPU 2", "GPU 3"... para as demais
    let label = match index {
        0 => "GPU".to_string(),
        i => format!("GPU {}", i + 1),
    };
    let g_icon = format!(" {} {label} ", icon_gpu(unicode_icons));

    let Some(g) = info else {
        render_gpu_unavailable(buf, area, g_icon, index);
        return;
    };

    let core_history: Vec<f64> = g.history.iter().copied().collect();
    let vram_history: Vec<f64> = g.vram_history.iter().copied().collect();

    let name_driver = match &g.driver_version {
        Some(driver) => format!("{} (Driver {}) ", g.name, driver),
        None => format!("{} ", g.name),
    };
    let title = Line::from(vec![
        Span::styled(
            g_icon,
            Style::default().fg(GREEN_NEON).add_modifier(Modifier::BOLD),
        ),
        Span::styled(name_driver, Style::default().fg(TEXT_WHITE)),
    ]);

    // Info PCIe e TDP no canto superior direito da borda
    let right_parts: Vec<String> = [
        g.pcie_info.clone(),
        g.tdp_watts.map(|t| format!("TDP: {} W", t)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let r_title = (!right_parts.is_empty()).then(|| {
        Line::from(Span::styled(
            format!("{} ", right_parts.join(" | ")),
            Style::default().fg(CYAN_NEON),
        ))
    });

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER_COLOR));
    let inner = block.inner(area);
    block.render(area, buf);
    render_panel_titles(buf, area, &title, r_title.as_ref());

    // Medidor (20) + métricas (38) + gráfico (mín. 16). Se não couber, o gráfico sai
    // primeiro: as métricas são a informação principal do painel.
    const GAUGE_W: u16 = 20;
    const METRICS_W: u16 = 38;
    const CHART_MIN_W: u16 = 16;
    let show_chart = inner.width >= GAUGE_W + METRICS_W + CHART_MIN_W;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(if show_chart {
            vec![
                Constraint::Length(GAUGE_W),
                Constraint::Length(METRICS_W),
                Constraint::Min(CHART_MIN_W),
            ]
        } else {
            vec![Constraint::Length(GAUGE_W), Constraint::Min(0)]
        })
        .split(inner);

    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let vram = match (g.vram_used_bytes, g.vram_total_bytes) {
        (Some(used), Some(total)) if total > 0 => Some((
            used as f64 / GB,
            total as f64 / GB,
            used as f64 / total as f64 * 100.0,
        )),
        _ => None,
    };
    let v_pct = vram.map(|(_, _, pct)| pct);
    let fmt_pct = |p: Option<f64>| {
        p.map(|v| format!("{:.0}%", v))
            .unwrap_or_else(|| "N/D".to_string())
    };

    let (gauge_pct, gauge_lbl, sub_lbl, chart_title, chart_data) = match mode {
        GpuGaugeMode::Core => (
            g.core_util,
            "GPU Usage",
            format!("VRAM: {}", fmt_pct(v_pct)),
            "Uso da GPU (histórico)",
            &core_history,
        ),
        GpuGaugeMode::Vram => (
            v_pct,
            "VRAM Usage",
            format!("Core: {}", fmt_pct(g.core_util)),
            "Uso de VRAM (histórico)",
            &vram_history,
        ),
    };

    render_circular_gauge(
        buf,
        cols[0],
        gauge_pct.unwrap_or(0.0),
        gauge_lbl,
        Some(&sub_lbl),
    );

    let mid_area = cols[1];

    // Linha 0: VRAM (sem barra)
    if mid_area.height > 0 {
        let y0 = mid_area.top();
        buf.set_string(mid_area.left(), y0, "VRAM", Style::default().fg(TEXT_WHITE));
        let (text, style) = match vram {
            Some((used_gb, tot_gb, pct)) => (
                format!("{:.1} / {:.1} GB ({:.0}%)", used_gb, tot_gb, pct),
                Style::default().fg(CYAN_NEON),
            ),
            None => ("N/D".to_string(), Style::default().fg(TEXT_DIM)),
        };
        set_string_clipped(buf, mid_area, mid_area.left() + 13, y0, &text, style);
    }

    let temp_color = if g.temp_c.is_some_and(|t| t >= 70) {
        RED_ALERT
    } else {
        GREEN_NEON
    };
    render_gpu_metric_row(
        buf,
        mid_area,
        1,
        "Temperatura",
        g.temp_c.map(|t| (t as f64, temp_color)),
        g.temp_c.map(|t| format!("{}°C", t)),
        Style::default()
            .fg(YELLOW_NEON)
            .add_modifier(Modifier::BOLD),
    );

    let power_text = match (g.power_watts, g.tdp_watts) {
        (Some(p), Some(tdp)) if tdp > 0 => Some(format!(
            "{:.0} W / {} W({:.0}%)",
            p,
            tdp,
            (p / tdp as f64 * 100.0).clamp(0.0, 100.0)
        )),
        (Some(p), _) => Some(format!("{:.0} W", p)),
        (None, _) => None,
    };
    let power_pct = match (g.power_watts, g.tdp_watts) {
        (Some(p), Some(tdp)) if tdp > 0 => Some(((p / tdp as f64) * 100.0).clamp(0.0, 100.0)),
        _ => None,
    };
    render_gpu_metric_row(
        buf,
        mid_area,
        2,
        "Potência",
        power_pct.map(|p| (p, YELLOW_NEON)),
        power_text,
        Style::default().fg(YELLOW_NEON),
    );

    render_gpu_metric_row(
        buf,
        mid_area,
        3,
        "Clock (Core)",
        g.clock_core_mhz
            .map(|c| (c as f64 / 3000.0 * 100.0, YELLOW_NEON)),
        g.clock_core_mhz
            .map(|c| format!("{:.3} GHz", c as f64 / 1000.0)),
        Style::default().fg(TEXT_WHITE),
    );

    render_gpu_metric_row(
        buf,
        mid_area,
        4,
        "Clock (Mem)",
        g.clock_mem_mhz
            .map(|c| (c as f64 / 15000.0 * 100.0, GREEN_NEON)),
        g.clock_mem_mhz
            .map(|c| format!("{:.3} GHz", c as f64 / 1000.0)),
        Style::default().fg(TEXT_WHITE),
    );

    let fan_text = match (g.fan_speed_pct, g.fan_rpm) {
        (Some(pct), Some(rpm)) => Some(format!("{}% ({} RPM)", pct, rpm)),
        (Some(pct), None) => Some(format!("{}%", pct)),
        (None, _) => None,
    };
    render_gpu_metric_row(
        buf,
        mid_area,
        5,
        "Fan Speed",
        g.fan_speed_pct.map(|p| (p as f64, CYAN_NEON)),
        fan_text,
        Style::default().fg(CYAN_NEON),
    );

    if show_chart {
        let right_area = cols[2];
        set_string_clipped(
            buf,
            right_area,
            right_area.left(),
            right_area.top(),
            chart_title,
            Style::default().fg(CYAN_NEON),
        );
        let chart_box = Rect::new(
            right_area.left(),
            right_area.top() + 1,
            right_area.width,
            right_area.height.saturating_sub(1),
        );
        render_solid_block_chart(buf, chart_box, chart_data, true, "100%", Some("50%"), "0%");
    }
}

/// Linha "Rótulo  [████░░░░]  valor" do painel de GPU; sem dado, barra vazia e "N/D"
fn render_gpu_metric_row(
    buf: &mut Buffer,
    area: Rect,
    row: u16,
    label: &str,
    bar: Option<(f64, Color)>,
    value: Option<String>,
    value_style: Style,
) {
    if row >= area.height {
        return;
    }
    let y = area.top() + row;
    set_string_clipped(
        buf,
        area,
        area.left(),
        y,
        label,
        Style::default().fg(TEXT_WHITE),
    );

    let (pct, color) = bar.unwrap_or((0.0, BAR_EMPTY));
    let mut cur_x = area.left() + 13;
    for s in &render_segmented_bar(pct, 8, color).spans {
        set_string_clipped(buf, area, cur_x, y, &s.content, s.style);
        cur_x += s.content.chars().count() as u16;
    }

    match value {
        Some(text) => set_string_clipped(buf, area, cur_x + 2, y, &text, value_style),
        None => set_string_clipped(
            buf,
            area,
            cur_x + 2,
            y,
            "N/D",
            Style::default().fg(TEXT_DIM),
        ),
    };
}

/// Painel de GPU quando a NVML não encontrou nenhuma placa NVIDIA
fn render_gpu_unavailable(buf: &mut Buffer, area: Rect, g_icon: String, index: u32) {
    let title = Line::from(Span::styled(
        g_icon,
        Style::default().fg(GREEN_NEON).add_modifier(Modifier::BOLD),
    ));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER_COLOR))
        .title(title);
    let inner = block.inner(area);
    block.render(area, buf);

    let lines = if index == 0 {
        [
            ("Nenhuma GPU NVIDIA detectada".to_string(), TEXT_WHITE),
            ("(driver NVIDIA / NVML indisponível)".to_string(), TEXT_DIM),
        ]
    } else {
        [
            (
                format!("GPU NVIDIA {} não encontrada", index + 1),
                TEXT_WHITE,
            ),
            ("(a máquina tem menos placas)".to_string(), TEXT_DIM),
        ]
    };
    let start_y = inner.top() + inner.height.saturating_sub(lines.len() as u16) / 2;
    for (i, (text, color)) in lines.iter().enumerate() {
        let y = start_y + i as u16;
        if y >= inner.bottom() {
            break;
        }
        let w = text.chars().count() as u16;
        let x = inner.left() + inner.width.saturating_sub(w) / 2;
        buf.set_stringn(
            x,
            y,
            text,
            inner.width as usize,
            Style::default().fg(*color),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tamanho do painel de GPU na tela demo de 160x40
    fn render_panel(panel: &GpuPanel) -> String {
        let area = Rect::new(0, 0, 77, 11);
        let mut buf = Buffer::empty(area);
        panel.render(area, &mut buf, &View::default());
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn gpu_ausente_nao_mostra_dados_ficticios() {
        let text = render_panel(&GpuPanel::new());
        assert!(text.contains("Nenhuma GPU NVIDIA detectada"));
        assert!(!text.contains("580.xx"));
        assert!(!text.contains("TDP"));
    }

    #[test]
    fn gpu_sem_ventoinha_e_sem_pcie_nao_inventa_valores() {
        let mut panel = GpuPanel::demo();
        let gpu = panel.info.as_mut().unwrap();
        gpu.fan_speed_pct = None;
        gpu.fan_rpm = None;
        gpu.pcie_info = None;
        let text = render_panel(&panel);
        assert!(text.contains("N/D"));
        assert!(!text.contains("RPM"));
        assert!(!text.contains("PCIe"));
        assert!(text.contains("TDP: 450 W"));

        // ventoinha com % mas sem tacômetro: mostra só a %
        panel.info.as_mut().unwrap().fan_speed_pct = Some(42);
        let text = render_panel(&panel);
        assert!(text.contains("42%") && !text.contains("RPM"));
    }

    #[test]
    fn leituras_da_nvml_que_falham_viram_nd() {
        let mut panel = GpuPanel::demo();
        let gpu = panel.info.as_mut().unwrap();
        gpu.driver_version = None;
        gpu.temp_c = None;
        gpu.power_watts = None;
        gpu.tdp_watts = None;
        gpu.clock_core_mhz = None;
        gpu.clock_mem_mhz = None;
        gpu.vram_used_bytes = None;
        gpu.vram_total_bytes = None;
        gpu.core_util = None;
        let text = render_panel(&panel);

        // Linhas de métrica do painel de GPU (coluna do meio)
        for rotulo in [
            "VRAM ",
            "Temperatura",
            "Potência",
            "Clock (Core)",
            "Clock (Mem)",
        ] {
            let linha = text
                .lines()
                .find(|l| l.contains(rotulo))
                .unwrap_or_else(|| panic!("sem linha {rotulo}"));
            assert!(linha.contains("N/D"), "{rotulo} deveria ser N/D: {linha}");
        }
        // Nenhum dos antigos valores de reserva
        for inventado in [
            "580.xx",
            "35°C",
            "24 W",
            "180 W",
            "2.580 GHz",
            "13.800 GHz",
            "16.0 GB",
            "Driver",
        ] {
            assert!(
                !text.contains(inventado),
                "valor inventado na tela: {inventado}"
            );
        }
    }

    #[test]
    fn tecla_g_alterna_medidor_e_outras_teclas_passam_adiante() {
        let mut panel = GpuPanel::demo();
        assert!(render_panel(&panel).contains("GPU Usage"));

        assert_eq!(
            panel.handle_key(KeyEvent::from(KeyCode::Char('g'))),
            Handled::Yes
        );
        assert!(render_panel(&panel).contains("VRAM Usage"));

        assert_eq!(
            panel.handle_key(KeyEvent::from(KeyCode::Char('x'))),
            Handled::No
        );
        assert_eq!(panel.keybindings()[0].key, "g");
    }

    #[test]
    fn sem_nvidia_o_painel_fica_indisponivel() {
        assert!(!GpuPanel::new().available());
        assert!(GpuPanel::demo().available());
    }

    #[test]
    fn cada_placa_tem_seu_id_e_seu_titulo() {
        let ids: Vec<&str> = (0..MAX_GPUS)
            .map(|i| GpuPanel::with_index(i).id())
            .collect();
        assert_eq!(ids, ["gpu", "gpu2", "gpu3", "gpu4"]);
        assert_eq!(
            GpuPanel::with_index(99).id(),
            "gpu4",
            "índice limitado à última"
        );

        let segunda = render_panel(&GpuPanel::demo_with_index(1));
        assert!(
            segunda.contains("GPU 2") && segunda.contains("RTX 4070"),
            "{segunda}"
        );
        let primeira = render_panel(&GpuPanel::demo());
        assert!(!primeira.contains("GPU 2") && primeira.contains("RTX 4090"));
    }

    #[test]
    fn placa_inexistente_fica_indisponivel_com_aviso_proprio() {
        let panel = GpuPanel::with_index(1);
        assert!(!panel.available());
        let text = render_panel(&panel);
        assert!(text.contains("GPU NVIDIA 2 não encontrada"), "{text}");
        assert!(!text.contains("Nenhuma GPU NVIDIA detectada"));
    }
}
