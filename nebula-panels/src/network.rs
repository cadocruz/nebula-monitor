//! Painel de Rede: recebimento e envio por interface (taxa atual, total e atividade
//! recente), lidos do sysinfo — no Linux, de /proc/net/dev, sem permissões especiais.

use std::collections::VecDeque;
use std::time::Instant;

use nebula_core::ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Widget},
};
use nebula_core::sysinfo::Networks;
use nebula_core::theme::*;
use nebula_core::widgets::{braille_peaks_line, render_panel_titles, set_string_clipped};
use nebula_core::{Context, Panel, Size, View};

const HISTORY_LEN: usize = 30;
/// Abaixo disso o gráfico não amplia: evita que ruído de poucos bytes pareça um pico
const MIN_SCALE_BPS: f64 = 1024.0;

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceInfo {
    pub name: String,
    /// Bytes por segundo no último intervalo
    pub rx_bps: f64,
    pub tx_bps: f64,
    pub total_rx: u64,
    pub total_tx: u64,
    /// Bytes por segundo dos últimos ciclos (mais antigo primeiro)
    pub rx_history: VecDeque<f64>,
    pub tx_history: VecDeque<f64>,
}

pub struct NetworkPanel {
    interfaces: Vec<InterfaceInfo>,
    networks: Networks,
    /// Momento da última leitura: `received()` do sysinfo conta desde a leitura anterior
    last_refresh: Option<Instant>,
}

impl Default for NetworkPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkPanel {
    pub fn new() -> Self {
        Self {
            interfaces: Vec::new(),
            networks: Networks::new_with_refreshed_list(),
            last_refresh: Some(Instant::now()),
        }
    }

    /// Dados fictícios do modo demonstração
    pub fn demo() -> Self {
        let wave = |base: f64, amp: f64, phase: f64| -> VecDeque<f64> {
            (0..HISTORY_LEN)
                .map(|i| (base + amp * (i as f64 * 0.45 + phase).sin()).max(0.0))
                .collect()
        };
        const MB: f64 = 1024.0 * 1024.0;
        Self::with_interfaces(vec![
            InterfaceInfo {
                name: "enp5s0".to_string(),
                rx_bps: 12.4 * MB,
                tx_bps: 1.2 * MB,
                total_rx: 182 * 1024 * 1024 * 1024,
                total_tx: 23 * 1024 * 1024 * 1024,
                rx_history: wave(9.0 * MB, 4.0 * MB, 0.0),
                tx_history: wave(1.0 * MB, 0.6 * MB, 1.3),
            },
            InterfaceInfo {
                name: "tailscale0".to_string(),
                rx_bps: 220.0 * 1024.0,
                tx_bps: 480.0 * 1024.0,
                total_rx: 3 * 1024 * 1024 * 1024,
                total_tx: 5 * 1024 * 1024 * 1024,
                rx_history: wave(200.0 * 1024.0, 120.0 * 1024.0, 2.1),
                tx_history: wave(400.0 * 1024.0, 200.0 * 1024.0, 0.7),
            },
            InterfaceInfo {
                name: "wlan0".to_string(),
                rx_bps: 0.0,
                tx_bps: 0.0,
                total_rx: 640 * 1024 * 1024,
                total_tx: 96 * 1024 * 1024,
                rx_history: VecDeque::from(vec![0.0; HISTORY_LEN]),
                tx_history: VecDeque::from(vec![0.0; HISTORY_LEN]),
            },
        ])
    }

    fn with_interfaces(interfaces: Vec<InterfaceInfo>) -> Self {
        Self {
            interfaces,
            networks: Networks::new(),
            last_refresh: None,
        }
    }

    fn collect(&mut self) {
        let now = Instant::now();
        self.networks.refresh(true);
        let dt = self
            .last_refresh
            .map(|t| now.duration_since(t).as_secs_f64())
            .unwrap_or(0.0);
        self.last_refresh = Some(now);

        let mut updated = Vec::new();
        for (name, data) in &self.networks {
            let (total_rx, total_tx) = (data.total_received(), data.total_transmitted());
            if !keep_interface(name, total_rx + total_tx) {
                continue;
            }
            let (rx_bps, tx_bps) = (
                per_second(data.received(), dt),
                per_second(data.transmitted(), dt),
            );
            let (mut rx_history, mut tx_history) =
                match self.interfaces.iter().find(|i| &i.name == name) {
                    Some(old) => (old.rx_history.clone(), old.tx_history.clone()),
                    None => (
                        VecDeque::from(vec![0.0; HISTORY_LEN]),
                        VecDeque::from(vec![0.0; HISTORY_LEN]),
                    ),
                };
            for (history, value) in [(&mut rx_history, rx_bps), (&mut tx_history, tx_bps)] {
                if history.len() >= HISTORY_LEN {
                    history.pop_front();
                }
                history.push_back(value);
            }
            updated.push(InterfaceInfo {
                name: name.clone(),
                rx_bps,
                tx_bps,
                total_rx,
                total_tx,
                rx_history,
                tx_history,
            });
        }
        sort_by_traffic(&mut updated);
        self.interfaces = updated;
    }
}

impl Panel for NetworkPanel {
    fn id(&self) -> &'static str {
        "network"
    }

    /// Sem interface com tráfego, a vaga pode passar para o próximo painel da lista
    fn available(&self) -> bool {
        !self.interfaces.is_empty()
    }

    fn update(&mut self, _ctx: &Context) {
        self.collect();
    }

    fn render(&self, area: Rect, buf: &mut Buffer, view: &View) {
        render_network(&self.interfaces, view.unicode_icons, buf, area);
    }

    fn min_size(&self) -> Size {
        Size {
            width: 56,
            height: 4,
        }
    }
}

/// Ignora o loopback e interfaces que nunca trafegaram nada
fn keep_interface(name: &str, total_bytes: u64) -> bool {
    name != "lo" && total_bytes > 0
}

fn per_second(bytes: u64, dt_secs: f64) -> f64 {
    if dt_secs > 0.0 {
        bytes as f64 / dt_secs
    } else {
        0.0
    }
}

/// Mais tráfego total primeiro; empate pelo nome
fn sort_by_traffic(interfaces: &mut [InterfaceInfo]) {
    interfaces.sort_by(|a, b| {
        (b.total_rx + b.total_tx)
            .cmp(&(a.total_rx + a.total_tx))
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Escala os dois históricos pelo maior valor entre eles (mínimo de 1 KB/s) para 0..1
fn normalized(rx: &VecDeque<f64>, tx: &VecDeque<f64>) -> (VecDeque<f64>, VecDeque<f64>) {
    let max = rx.iter().chain(tx).copied().fold(MIN_SCALE_BPS, f64::max);
    let scale = |h: &VecDeque<f64>| h.iter().map(|v| (v / max).clamp(0.0, 1.0)).collect();
    (scale(rx), scale(tx))
}

fn human(value: f64, suffix: &str) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = value.max(0.0);
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    match unit {
        0 => format!("{v:.0} {}{suffix}", UNITS[unit]),
        _ => format!("{v:.1} {}{suffix}", UNITS[unit]),
    }
}

fn human_rate(bps: f64) -> String {
    human(bps, "/s")
}

fn human_bytes(bytes: u64) -> String {
    human(bytes as f64, "")
}

fn render_network(interfaces: &[InterfaceInfo], unicode_icons: bool, buf: &mut Buffer, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER_COLOR));
    let inner = block.inner(area);
    block.render(area, buf);

    let title = Line::from(Span::styled(
        format!(" {} REDE ", icon_net(unicode_icons)),
        Style::default().fg(CYAN_NEON).add_modifier(Modifier::BOLD),
    ));
    let (sum_rx, sum_tx) = interfaces
        .iter()
        .fold((0.0, 0.0), |(r, t), i| (r + i.rx_bps, t + i.tx_bps));
    let summary = Line::from(vec![
        Span::styled(
            format!("↓ {}  ", human_rate(sum_rx)),
            Style::default().fg(CYAN_NEON),
        ),
        Span::styled(
            format!("↑ {} ", human_rate(sum_tx)),
            Style::default().fg(MAGENTA_NEON),
        ),
    ]);
    render_panel_titles(buf, area, &title, Some(&summary));

    if interfaces.is_empty() {
        let text = "Nenhuma interface de rede ativa";
        let x = inner.left() + inner.width.saturating_sub(text.chars().count() as u16) / 2;
        let y = inner.top() + inner.height / 2;
        set_string_clipped(buf, inner, x, y, text, Style::default().fg(TEXT_DIM));
        return;
    }

    // Colunas: interface | recebendo | enviando | totais | atividade (↓ ciano, ↑ magenta)
    let x = inner.left() + 1;
    let (x_rx, x_tx, x_total, x_act) = (x + 13, x + 25, x + 37, x + 58);
    let dim = Style::default().fg(TEXT_DIM);
    for (col, label) in [
        (x, "INTERFACE"),
        (x_rx, "↓ RECEBENDO"),
        (x_tx, "↑ ENVIANDO"),
        (x_total, "TOTAL ↓ / ↑"),
        (x_act, "ATIVIDADE"),
    ] {
        set_string_clipped(buf, inner, col, inner.top(), label, dim);
    }

    for (row, iface) in interfaces.iter().enumerate() {
        let y = inner.top() + 1 + row as u16;
        if y >= inner.bottom() {
            break;
        }
        let name: String = iface.name.chars().take(12).collect();
        set_string_clipped(buf, inner, x, y, &name, Style::default().fg(TEXT_WHITE));
        let rate_style = |bps: f64, color| {
            if bps > 0.0 {
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else {
                dim
            }
        };
        set_string_clipped(
            buf,
            inner,
            x_rx,
            y,
            &human_rate(iface.rx_bps),
            rate_style(iface.rx_bps, CYAN_NEON),
        );
        set_string_clipped(
            buf,
            inner,
            x_tx,
            y,
            &human_rate(iface.tx_bps),
            rate_style(iface.tx_bps, MAGENTA_NEON),
        );
        let totals = format!(
            "{} / {}",
            human_bytes(iface.total_rx),
            human_bytes(iface.total_tx)
        );
        set_string_clipped(buf, inner, x_total, y, &totals, dim);

        let act_width = inner.right().saturating_sub(x_act + 1) as usize;
        if act_width >= 4 {
            let (rx, tx) = normalized(&iface.rx_history, &iface.tx_history);
            let half = act_width / 2;
            let rx_line = braille_peaks_line(&rx, half);
            let tx_line = braille_peaks_line(&tx, act_width - half);
            set_string_clipped(
                buf,
                inner,
                x_act,
                y,
                &rx_line,
                Style::default().fg(CYAN_NEON),
            );
            set_string_clipped(
                buf,
                inner,
                x_act + half as u16,
                y,
                &tx_line,
                Style::default().fg(MAGENTA_NEON),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(name: &str, total_rx: u64, total_tx: u64) -> InterfaceInfo {
        InterfaceInfo {
            name: name.to_string(),
            rx_bps: 0.0,
            tx_bps: 0.0,
            total_rx,
            total_tx,
            rx_history: VecDeque::new(),
            tx_history: VecDeque::new(),
        }
    }

    fn render_panel(panel: &NetworkPanel, width: u16, height: u16) -> String {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        panel.render(area, &mut buf, &View::default());
        (0..height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn taxas_por_segundo_e_intervalo_invalido() {
        assert_eq!(per_second(2048, 2.0), 1024.0);
        assert_eq!(
            per_second(2048, 0.0),
            0.0,
            "primeira leitura: sem intervalo"
        );
    }

    #[test]
    fn ignora_loopback_e_interfaces_sem_trafego() {
        assert!(!keep_interface("lo", 10_000));
        assert!(!keep_interface("docker0", 0));
        assert!(keep_interface("enp5s0", 1));
        assert!(
            keep_interface("lo0x", 1),
            "só o nome exato \"lo\" é o loopback"
        );
    }

    #[test]
    fn ordena_por_trafego_total_e_desempata_pelo_nome() {
        let mut list = vec![
            iface("wlan0", 10, 0),
            iface("eth1", 50, 50),
            iface("eth0", 50, 50),
        ];
        sort_by_traffic(&mut list);
        let names: Vec<&str> = list.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["eth0", "eth1", "wlan0"]);
    }

    #[test]
    fn formata_taxas_e_totais() {
        assert_eq!(human_rate(512.0), "512 B/s");
        assert_eq!(human_rate(1536.0), "1.5 KB/s");
        assert_eq!(human_rate(12.4 * 1024.0 * 1024.0), "12.4 MB/s");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn historico_normalizado_pelo_maior_valor_com_piso() {
        let rx = VecDeque::from(vec![0.0, 4096.0]);
        let tx = VecDeque::from(vec![2048.0, 0.0]);
        let (r, t) = normalized(&rx, &tx);
        assert_eq!(r, VecDeque::from(vec![0.0, 1.0]));
        assert_eq!(t, VecDeque::from(vec![0.5, 0.0]));

        // tráfego desprezível não vira pico: escala mínima de 1 KB/s
        let (r, _) = normalized(&VecDeque::from(vec![10.0]), &VecDeque::from(vec![0.0]));
        assert!(r[0] < 0.01);
    }

    #[test]
    fn desenha_interfaces_do_demo() {
        let text = render_panel(&NetworkPanel::demo(), 96, 8);
        assert!(text.contains("REDE"), "{text}");
        assert!(
            text.contains("enp5s0") && text.contains("tailscale0"),
            "{text}"
        );
        assert!(text.contains("12.4 MB/s"), "{text}");
        assert!(text.contains("182.0 GB / 23.0 GB"), "{text}");
    }

    #[test]
    fn sem_interfaces_fica_indisponivel_e_avisa() {
        let panel = NetworkPanel::with_interfaces(Vec::new());
        assert!(!panel.available());
        assert!(NetworkPanel::demo().available());
        assert!(render_panel(&panel, 60, 6).contains("Nenhuma interface de rede ativa"));
    }
}
