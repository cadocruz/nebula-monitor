use nebula_core::theme::*;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Widget},
};

use crate::model::{AppState, BatteryInfo, BatteryStatus};
use crate::panel_set::PanelSet;
use nebula_core::widgets::highlight_border;
use nebula_core::{KeyHint, View};

pub fn render_top_bar(buf: &mut Buffer, area: Rect, state: &AppState) {
    let block = Block::default()
        .borders(Borders::NONE)
        .style(Style::default().bg(BG_DARK));
    block.render(area, buf);

    let right_line = state.battery.as_ref().map(battery_line);
    let right_len = right_line.as_ref().map(|l| l.width() as u16).unwrap_or(0);
    let show_right = right_line.is_some() && area.width > 100;
    let right_x = if show_right {
        area.right().saturating_sub(right_len + 1)
    } else {
        area.right()
    };

    let clean_distro = state
        .distro
        .replace("Linux (Ubuntu 26.04)", "Ubuntu 26.04")
        .replace("Linux (", "")
        .replace(")", "");

    let d_icon = format!(
        "{} ",
        icon_distro(&state.distro, state.unicode_fallback_icons)
    );
    let clean_kernel = state
        .kernel
        .split('-')
        .next()
        .unwrap_or(&state.kernel)
        .to_string();

    let left_spans = vec![
        Span::styled(
            format!("NEBULA-MONITOR v{}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(CYAN_NEON).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  |  ", Style::default().fg(CYAN_DIM)),
        Span::styled("hostname: ", Style::default().fg(TEXT_DIM)),
        Span::styled(state.hostname.clone(), Style::default().fg(TEXT_WHITE)),
        Span::styled("  |  ", Style::default().fg(CYAN_DIM)),
        Span::styled("distro: ", Style::default().fg(TEXT_DIM)),
        Span::styled(d_icon, Style::default().fg(CYAN_NEON)),
        Span::styled(clean_distro, Style::default().fg(CYAN_NEON)),
        Span::styled("  |  ", Style::default().fg(CYAN_DIM)),
        Span::styled("kernel: ", Style::default().fg(TEXT_DIM)),
        Span::styled(clean_kernel, Style::default().fg(TEXT_WHITE)),
        Span::styled("  |  ", Style::default().fg(CYAN_DIM)),
        Span::styled("uptime: ", Style::default().fg(TEXT_DIM)),
        Span::styled(state.uptime_str.clone(), Style::default().fg(TEXT_WHITE)),
        Span::styled("  |  ", Style::default().fg(CYAN_DIM)),
        Span::styled(state.datetime_str.clone(), Style::default().fg(TEXT_WHITE)),
    ];
    let max_left_w = right_x.saturating_sub(area.left() + 2);
    let left_line = Line::from(left_spans);
    buf.set_line(area.left(), area.top(), &left_line, max_left_w);

    if let (true, Some(line)) = (show_right, right_line) {
        buf.set_line(right_x, area.top(), &line, right_len);
    }
}

/// "⚡ AC  🔋 57%  [██████░░░░] Descarregando" — só existe quando há bateria
fn battery_line(b: &BatteryInfo) -> Line<'static> {
    let color = match b.percent {
        0..=15 => RED_ALERT,
        16..=35 => YELLOW_NEON,
        _ => GREEN_NEON,
    };
    let status = match b.status {
        BatteryStatus::Charging => "Carregando",
        BatteryStatus::Discharging => "Descarregando",
        BatteryStatus::Full => "Completa",
        BatteryStatus::NotCharging => "Sem carga",
        BatteryStatus::Unknown => "",
    };
    let filled = (b.percent as usize + 5) / 10;

    let mut spans = Vec::new();
    if b.ac_online == Some(true) {
        spans.push(Span::styled("⚡ AC  ", Style::default().fg(TEXT_WHITE)));
    }
    spans.push(Span::styled(
        format!("🔋 {}%  ", b.percent),
        Style::default().fg(color),
    ));
    spans.push(Span::styled(
        format!("[{}{}]", "█".repeat(filled), "░".repeat(10 - filled)),
        Style::default().fg(color),
    ));
    if !status.is_empty() {
        spans.push(Span::styled(
            format!(" {}", status),
            Style::default().fg(TEXT_DIM),
        ));
    }
    Line::from(spans)
}

pub fn render_footer(buf: &mut Buffer, area: Rect, state: &AppState, hints: &[KeyHint]) {
    let block = Block::default()
        .borders(Borders::NONE)
        .style(Style::default().bg(BG_DARK));
    block.render(area, buf);

    let mut spans = vec![Span::raw(" ")];
    for (i, hint) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" | ", Style::default().fg(TEXT_DIM)));
        }
        let is_quit = hint.key == "q";
        spans.push(Span::styled(
            format!("[{}] ", hint.key),
            Style::default()
                .fg(if is_quit { RED_ALERT } else { CYAN_NEON })
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            hint.label,
            Style::default().fg(if is_quit { TEXT_WHITE } else { TEXT_DIM }),
        ));
    }
    buf.set_line(area.left(), area.top(), &Line::from(spans), area.width);

    if state.paused {
        let badge = " ⏸ PAUSADO [p] ";
        let w = badge.chars().count() as u16;
        let x = area.right().saturating_sub(w + 1).max(area.left());
        buf.set_stringn(
            x,
            area.top(),
            badge,
            area.width as usize,
            Style::default()
                .fg(Color::Black)
                .bg(YELLOW_NEON)
                .add_modifier(Modifier::BOLD),
        );
    }
}

/// Atalhos globais (tratados em main.rs) que aparecem antes dos atalhos dos painéis
const GLOBAL_HINTS_BEFORE: &[KeyHint] = &[KeyHint {
    key: "h",
    label: "Ajuda",
    description: "Abrir / fechar esta ajuda (Esc também fecha)",
}];

/// ... e depois deles
const GLOBAL_HINTS_AFTER: &[KeyHint] = &[
    KeyHint {
        key: "i",
        label: "Ícones",
        description: "Ícones Nerd Font ↔ Unicode universal",
    },
    KeyHint {
        key: "p",
        label: "Pausar",
        description: "Pausar / retomar a atualização",
    },
    KeyHint {
        key: "q",
        label: "Sair",
        description: "Sair e restaurar o terminal",
    },
];

/// Com um painel focado: maximizar (ou restaurar, se já estiver maximizado)
const MAXIMIZE_HINT: KeyHint = KeyHint {
    key: "z",
    label: "Maximizar",
    description: "Maximizar / restaurar o painel focado",
};
const RESTORE_HINT: KeyHint = KeyHint {
    label: "Restaurar",
    ..MAXIMIZE_HINT
};

/// Enquanto um painel captura o teclado, a única tecla global que vale
const CTRL_C_HINT: KeyHint = KeyHint {
    key: "Ctrl+C",
    label: "Sair",
    description: "Sair",
};

/// Rodapé: atalhos globais e os de cada painel (o focado primeiro). Com um painel
/// capturando o teclado, só os atalhos dele e o Ctrl+C.
fn footer_hints(panels: &PanelSet) -> Vec<KeyHint> {
    if panels.capturing_slot().is_some() {
        let mut hints = panels.keybindings();
        hints.push(CTRL_C_HINT);
        return hints;
    }
    let mut hints = GLOBAL_HINTS_BEFORE.to_vec();
    hints.extend(panels.keybindings());
    if panels.focused().is_some() {
        hints.push(if panels.maximized() {
            RESTORE_HINT
        } else {
            MAXIMIZE_HINT
        });
    }
    hints.extend_from_slice(GLOBAL_HINTS_AFTER);
    hints
}

/// Ajuda: como o rodapé, com as frases longas, mais o foco por número e Ctrl+C
fn help_lines(panels: &PanelSet) -> Vec<(String, &'static str)> {
    let mut lines = vec![(
        GLOBAL_HINTS_BEFORE[0].key.to_string(),
        GLOBAL_HINTS_BEFORE[0].description,
    )];
    lines.push((
        format!("1–{}", panels.slot_count()),
        "Focar painel (Esc tira o foco)",
    ));
    lines.push((MAXIMIZE_HINT.key.to_string(), MAXIMIZE_HINT.description));
    lines.extend(
        panels
            .keybindings()
            .iter()
            .map(|h| (h.key.to_string(), h.description)),
    );
    for h in &GLOBAL_HINTS_AFTER[..GLOBAL_HINTS_AFTER.len() - 1] {
        lines.push((h.key.to_string(), h.description));
    }
    lines.push(("q / Esc".to_string(), "Sair e restaurar o terminal"));
    lines.push((CTRL_C_HINT.key.to_string(), CTRL_C_HINT.description));
    lines.push((
        "Mouse".to_string(),
        "Clique foca o painel; roda rola a lista",
    ));
    lines
}
pub fn render_help_popup(buf: &mut Buffer, area: Rect, lines: &[(String, &str)]) {
    let width = 56.min(area.width);
    let height = (lines.len() as u16 + 4).min(area.height);
    let popup = Rect::new(
        area.left() + area.width.saturating_sub(width) / 2,
        area.top() + area.height.saturating_sub(height) / 2,
        width,
        height,
    );

    Clear.render(popup, buf);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(MAGENTA_NEON))
        .style(Style::default().bg(BG_DARK))
        .title(Span::styled(
            " AJUDA — ATALHOS ",
            Style::default()
                .fg(MAGENTA_NEON)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(popup);
    block.render(popup, buf);

    for (i, (key, desc)) in lines.iter().enumerate() {
        let y = inner.top() + 1 + i as u16;
        if y >= inner.bottom() {
            break;
        }
        let line = Line::from(vec![
            Span::styled(
                format!(" {:<9}", key),
                Style::default().fg(CYAN_NEON).add_modifier(Modifier::BOLD),
            ),
            Span::styled(*desc, Style::default().fg(TEXT_WHITE)),
        ]);
        buf.set_line(inner.left(), y, &line, inner.width);
    }
}

pub fn render_ui(frame: &mut ratatui::Frame, state: &AppState, panels: &PanelSet) {
    let area = frame.area();
    if area.width < 40 || area.height < 6 {
        return;
    }
    let buf = frame.buffer_mut();

    buf.set_style(area, Style::default().bg(BG_DARK));

    let ui = compute_areas(area);

    render_top_bar(buf, ui.top_bar, state);
    render_footer(buf, ui.footer, state, &footer_hints(panels));

    for (i, slot_area) in panels.visible_slots(ui.body) {
        let view = View {
            unicode_icons: state.unicode_fallback_icons,
            focused: panels.focused() == Some(i),
        };
        panels.render_slot(i, slot_area, buf, &view);
        if view.focused {
            highlight_border(buf, slot_area, FOCUS_BORDER);
        }
    }
    if state.show_help {
        render_help_popup(buf, ui.body, &help_lines(panels));
    }
    // Por último: o tema troca as cores da paleta no quadro inteiro, inclusive dos plugins
    state.palette.apply(buf);
}

struct UiAreas {
    top_bar: Rect,
    body: Rect,
    footer: Rect,
}

/// Corpo da tela (onde ficam as vagas), para localizar o mouse
pub fn body_area(area: Rect) -> Rect {
    compute_areas(area).body
}

/// Moldura da tela: barra do topo, corpo (onde ficam as vagas do layout) e rodapé
fn compute_areas(area: Rect) -> UiAreas {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);
    UiAreas {
        top_bar: chunks[0],
        body: chunks[1],
        footer: chunks[2],
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn render_to_text(state: &AppState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
        terminal
            .draw(|f| render_ui(f, state, &PanelSet::demo()))
            .unwrap();
        let buf = terminal.backend().buffer();
        (0..40)
            .map(|y| (0..160).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn rodape_lista_apenas_atalhos_existentes() {
        let text = render_to_text(&AppState::demo());
        let footer = text.lines().next_back().unwrap();
        assert!(footer.contains("[q] Sair"));
        for fantasma in ["Rede", "Sensores", "Tema", "Selecionar"] {
            assert!(!footer.contains(fantasma), "rodapé ainda cita {fantasma}");
        }
    }

    #[test]
    fn tecla_h_exibe_popup_de_ajuda() {
        let mut state = AppState::demo();
        assert!(!render_to_text(&state).contains("AJUDA"));
        state.show_help = true;
        assert!(render_to_text(&state).contains("AJUDA"));
    }

    #[test]
    fn bateria_so_aparece_quando_existe() {
        let mut state = AppState::demo();
        state.battery = Some(BatteryInfo {
            percent: 57,
            status: BatteryStatus::Discharging,
            ac_online: Some(false),
        });
        let top = render_to_text(&state).lines().next().unwrap().to_string();
        assert!(
            top.contains("57%") && top.contains("Descarregando"),
            "topo: {top}"
        );
        assert!(!top.contains("AC"), "sem tomada não deve mostrar AC: {top}");

        state.battery = None;
        let top = render_to_text(&state).lines().next().unwrap().to_string();
        assert!(
            !top.contains('🔋') && !top.contains("100%"),
            "desktop não deve ter bateria: {top}"
        );
    }

    #[test]
    fn painel_de_gpu_estreito_prioriza_metricas() {
        let texto = |w, h| {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|f| render_ui(f, &AppState::demo(), &PanelSet::demo()))
                .unwrap();
            let buf = terminal.backend().buffer();
            (0..h)
                .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let estreito = texto(120, 34);
        let vram = estreito.lines().find(|l| l.contains("VRAM ")).unwrap();
        assert!(
            vram.contains("13.8 / 22.9 GB (60%)"),
            "valor de VRAM cortado: {vram}"
        );
        assert!(
            !estreito.contains("Uso da GPU (hist"),
            "gráfico não cabe e deve sair"
        );

        assert!(
            texto(160, 40).contains("Uso da GPU (hist"),
            "em tela larga o gráfico continua"
        );
    }

    #[test]
    fn foco_destaca_so_a_moldura_do_painel_focado() {
        let mut panels = PanelSet::demo();
        assert!(panels.focus(2)); // Memória
        let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
        terminal
            .draw(|f| render_ui(f, &AppState::demo(), &panels))
            .unwrap();
        let buf = terminal.backend().buffer();
        let areas = panels.areas(compute_areas(Rect::new(0, 0, 160, 40)).body);
        let (cpu, memory, processes) = (areas[0], areas[1], areas[4]);

        let canto = &buf[(memory.left(), memory.top())];
        assert_eq!(canto.symbol(), "╭");
        assert_eq!(canto.fg, FOCUS_BORDER, "moldura da vaga focada");
        let base = &buf[(memory.left(), memory.bottom() - 1)];
        assert_eq!(base.fg, FOCUS_BORDER, "borda inferior também");

        // o texto do título fica com a cor dele
        let titulo_x = (memory.left()..memory.right())
            .find(|&x| buf[(x, memory.top())].symbol() == "M")
            .unwrap();
        assert_ne!(buf[(titulo_x, memory.top())].fg, FOCUS_BORDER);

        // as outras vagas continuam com a moldura normal
        assert_eq!(buf[(cpu.left(), cpu.top())].fg, BORDER_COLOR);
        assert_eq!(buf[(processes.left(), processes.top())].fg, BORDER_COLOR);
    }

    #[test]
    fn ajuda_inclui_foco_e_atalhos_declarados_pelos_paineis() {
        let mut state = AppState::demo();
        state.show_help = true;
        let text = render_to_text(&state);
        assert!(text.contains("1–5") && text.contains("Focar painel"));
        assert!(text.contains("Ordenar processos")); // declarado pelo painel de Processos
        assert!(text.contains("Medidor da GPU")); // declarado pelo painel de GPU
        assert!(text.contains("Ctrl+C"));
    }
    #[test]
    fn titulos_dos_paineis_nao_se_sobrepoem() {
        for (w, h) in [(160, 40), (120, 34), (100, 30)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|f| render_ui(f, &AppState::demo(), &PanelSet::demo()))
                .unwrap();
            let buf = terminal.backend().buffer();
            let top_border = (0..w).map(|x| buf[(x, 1)].symbol()).collect::<String>();
            // o resumo da direita, quando aparece, aparece inteiro
            if top_border.contains("TDP") {
                assert!(
                    top_border.contains("PCIe 4.0 x16 | TDP: 450 W"),
                    "{w}x{h}: {top_border}"
                );
            }
            if top_border.contains("GHz") {
                assert!(top_border.contains("125.0 W"), "{w}x{h}: {top_border}");
            }
            // e o título da esquerda nunca encosta nele: antes vem borda ou reticências
            for inicio_direita in ["PCIe 4.0", "4.8 GHz"] {
                if let Some(pos) = top_border.find(inicio_direita) {
                    let anterior = top_border[..pos].chars().next_back().unwrap();
                    assert!(
                        anterior == '─' || anterior == '…',
                        "{w}x{h}: '{anterior}' antes de {inicio_direita}: {top_border}"
                    );
                }
            }
        }
    }

    #[test]
    fn rodape_com_painel_capturando_mostra_so_os_atalhos_dele() {
        let mut panels = PanelSet::demo();
        panels.handle_key(crossterm::event::KeyEvent::from(
            crossterm::event::KeyCode::Char('/'),
        ));
        let keys: Vec<_> = footer_hints(&panels).iter().map(|h| h.key).collect();
        assert_eq!(keys, ["Enter", "Esc", "Ctrl+C"]);
    }

    #[test]
    fn clock_de_memoria_da_gpu_em_ghz() {
        // demo: 10500 MHz
        assert!(render_to_text(&AppState::demo()).contains("10.500 GHz"));
    }

    #[test]
    fn pausa_aparece_no_rodape() {
        let mut state = AppState::demo();
        assert!(!render_to_text(&state).contains("PAUSADO"));
        state.paused = true;
        assert!(
            render_to_text(&state)
                .lines()
                .next_back()
                .unwrap()
                .contains("PAUSADO")
        );
    }
}
