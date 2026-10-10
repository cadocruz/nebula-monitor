//! Painel de Processos: tabela ordenável (CPU, MEM, GPU, IO) com seleção e rolagem.
//! Teclas: Tab troca a ordenação; ↑/↓ movem a seleção.

use std::collections::HashMap;

use nebula_core::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use nebula_core::nvml_wrapper::struct_wrappers::device::ProcessUtilizationSample;
use nebula_core::ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Row, Table, Widget},
};
use nebula_core::sysinfo::{Pid, Signal, System, Users};
use nebula_core::theme::*;
use nebula_core::widgets::set_string_clipped;
use nebula_core::{Context, Handled, KeyHint, Panel, Size, View};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessSort {
    Cpu,
    Mem,
    Gpu,
    Io,
}

#[derive(Debug, Clone)]
pub struct ProcessItem {
    pub pid: u32,
    pub user: String,
    pub cpu_usage: f32,
    pub mem_usage: f32,
    /// Uso dos SMs (3D/compute) pelo processo, em %, via nvmlDeviceGetProcessUtilization
    pub gpu_usage: f32,
    pub command: String,
    /// Linha de comando completa; processos do kernel (sem cmdline) aparecem como "[nome]"
    pub cmdline: String,
    pub read_mbs: f32,
    pub write_mbs: f32,
}

/// O que o painel está fazendo com o teclado
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    Normal,
    /// Digitando o filtro (`/`)
    Filtering,
    /// Esperando s/N para encerrar o processo (`k`)
    ConfirmKill {
        pid: u32,
        name: String,
    },
}

pub struct ProcessesPanel {
    processes: Vec<ProcessItem>,
    sort_key: ProcessSort,
    selected: usize,
    scroll_offset: usize,
    /// Linhas da tabela no último quadro desenhado: a rolagem com ↓ usa este valor
    visible_rows: std::cell::Cell<usize>,
    /// Lista de usuários lida uma vez na inicialização
    users: Users,
    /// Por placa: timestamp (µs) da amostra mais recente de uso por processo já lida da NVML
    last_util_ts: Vec<Option<u64>>,
    /// Coluna COMMAND mostra a linha de comando completa em vez do nome
    show_cmdline: bool,
    /// Filtro por nome, linha de comando ou PID (vazio = todos)
    filter: String,
    mode: Mode,
    /// Encerramento confirmado: o SIGTERM sai no próximo update, que tem o sysinfo
    pending_kill: Option<(u32, String)>,
    /// Resultado da última ação (sinal enviado, sem permissão...), até a próxima tecla
    status: Option<String>,
    /// Modo demo: confirmar o encerramento não envia sinal nenhum
    demo: bool,
    /// Primeira linha da lista desenhada no último quadro (o clique do mouse usa)
    drawn_offset: std::cell::Cell<usize>,
}

impl Default for ProcessesPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessesPanel {
    pub fn new() -> Self {
        Self::with(Vec::new(), Users::new_with_refreshed_list())
    }

    /// Dados fictícios do modo demonstração
    pub fn demo() -> Self {
        let mut panel = Self::with(demo_processes(), Users::new());
        panel.demo = true;
        panel
    }

    fn with(processes: Vec<ProcessItem>, users: Users) -> Self {
        Self {
            processes,
            sort_key: ProcessSort::Cpu,
            selected: 0,
            scroll_offset: 0,
            visible_rows: std::cell::Cell::new(0),
            users,
            last_util_ts: Vec::new(),
            show_cmdline: false,
            filter: String::new(),
            mode: Mode::Normal,
            pending_kill: None,
            status: None,
            demo: false,
            drawn_offset: std::cell::Cell::new(0),
        }
    }

    /// Ordena a lista (decrescente) pelo critério da aba ativa
    fn apply_sort(&mut self) {
        let key = |p: &ProcessItem| match self.sort_key {
            ProcessSort::Cpu => p.cpu_usage,
            ProcessSort::Mem => p.mem_usage,
            ProcessSort::Gpu => p.gpu_usage,
            ProcessSort::Io => p.read_mbs + p.write_mbs,
        };
        self.processes.sort_by(|a, b| key(b).total_cmp(&key(a)));
    }

    /// Processos que passam no filtro, na ordem atual
    fn visible(&self) -> Vec<&ProcessItem> {
        let needle = self.filter.to_lowercase();
        self.processes
            .iter()
            .filter(|p| needle.is_empty() || matches_filter(p, &needle))
            .collect()
    }

    fn reset_selection(&mut self) {
        self.selected = 0;
        self.scroll_offset = 0;
    }

    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            if self.selected < self.scroll_offset {
                self.scroll_offset = self.selected;
            }
        }
    }

    fn move_down(&mut self) {
        if self.selected + 1 < self.visible().len() {
            self.selected += 1;
            // Antes do primeiro desenho a altura é desconhecida; o render mantém
            // a seleção visível de qualquer forma
            let max_visible = self.visible_rows.get();
            if max_visible > 0 && self.selected >= self.scroll_offset + max_visible {
                self.scroll_offset = self.selected + 1 - max_visible;
            }
        }
    }

    fn handle_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.mode = Mode::Normal,
            KeyCode::Esc => {
                self.filter.clear();
                self.mode = Mode::Normal;
            }
            KeyCode::Backspace => {
                self.filter.pop();
            }
            KeyCode::Char(c) if !c.is_control() => self.filter.push(c),
            _ => return,
        }
        self.reset_selection();
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) {
        let Mode::ConfirmKill { pid, name } = std::mem::replace(&mut self.mode, Mode::Normal)
        else {
            return;
        };
        self.status = Some(match key.code {
            KeyCode::Char('s' | 'S' | 'y' | 'Y') if self.demo => {
                "Modo demo: nenhum sinal enviado".to_string()
            }
            KeyCode::Char('s' | 'S' | 'y' | 'Y') => {
                let message = format!("Enviando SIGTERM para {pid} ({name})…");
                self.pending_kill = Some((pid, name));
                message
            }
            _ => format!("Encerramento de {pid} cancelado"),
        });
    }

    fn collect(&mut self, ctx: &Context) {
        let mut procs = Vec::new();

        // %GPU por processo = uso dos SMs (3D/compute) nas amostras da NVML desde a última
        // leitura, somado em todas as placas (como o %CPU soma núcleos). Processos sem
        // amostra nova ficaram ociosos na GPU no intervalo (a NVML só guarda amostras com
        // uso > 0). Placas sem suporte (anteriores a Maxwell) contribuem com 0.
        let mut gpu_procs: HashMap<u32, f32> = HashMap::new();
        if let Some(nvml) = ctx.nvml {
            let count = nvml.device_count().unwrap_or(0) as usize;
            self.last_util_ts.resize(count, None);
            for (i, last_ts) in self.last_util_ts.iter_mut().enumerate() {
                let Ok(dev) = nvml.device_by_index(i as u32) else {
                    continue;
                };
                let Ok(samples) = dev.process_utilization_stats(*last_ts) else {
                    continue;
                };
                let (per_pid, newest_ts) = latest_sm_util_per_pid(&samples);
                add_gpu_util(&mut gpu_procs, per_pid);
                if newest_ts.is_some() {
                    *last_ts = newest_ts;
                }
            }
        }

        // Total vem direto do sysinfo: os processos não dependem do painel de Memória
        let total_mem = ctx.system.total_memory().max(1) as f32;

        for (pid, process) in ctx.system.processes() {
            let pid_u32 = pid.as_u32();
            let cpu = process.cpu_usage();
            let mem_bytes = process.memory() as f32;
            let mem_pct = (mem_bytes / total_mem) * 100.0;
            let gpu_pct = gpu_procs.get(&pid_u32).copied().unwrap_or(0.0);

            // Lista de usuários lida uma vez na inicialização; sem nome, mostra o UID/SID real
            let user = match process.user_id() {
                Some(uid) => self
                    .users
                    .get_user_by_id(uid)
                    .map(|u| u.name().to_string())
                    .unwrap_or_else(|| (**uid).to_string()),
                None => "?".to_string(),
            };

            let name = process.name().to_string_lossy().to_string();
            let cmdline = cmdline_or_name(process.cmd(), &name);
            let disk_usage = process.disk_usage();
            let r_mbs = (disk_usage.read_bytes as f32) / (1024.0 * 1024.0);
            let w_mbs = (disk_usage.written_bytes as f32) / (1024.0 * 1024.0);

            procs.push(ProcessItem {
                pid: pid_u32,
                user,
                cpu_usage: cpu,
                mem_usage: mem_pct,
                gpu_usage: gpu_pct,
                command: name,
                cmdline,
                read_mbs: r_mbs,
                write_mbs: w_mbs,
            });
        }

        self.processes = procs;
        self.apply_sort();
    }
}

/// Processos fictícios do modo demonstração (também usados pela tabela do painel de CPU)
pub fn demo_processes() -> Vec<ProcessItem> {
    vec![
        ProcessItem {
            pid: 1420,
            user: "operator".to_string(),
            cpu_usage: 142.0,
            mem_usage: 12.5,
            gpu_usage: 54.0,
            command: "ollama-server (qwen3.5-32b)".to_string(),
            cmdline: "/usr/local/bin/ollama serve --model qwen3.5-32b".to_string(),
            read_mbs: 18.2,
            write_mbs: 4.1,
        },
        ProcessItem {
            pid: 3891,
            user: "operator".to_string(),
            cpu_usage: 68.5,
            mem_usage: 8.2,
            gpu_usage: 0.0,
            command: "rust-analyzer --workspace".to_string(),
            cmdline: "/home/operator/.cargo/bin/rust-analyzer --workspace".to_string(),
            read_mbs: 35.0,
            write_mbs: 12.0,
        },
        ProcessItem {
            pid: 2104,
            user: "operator".to_string(),
            cpu_usage: 28.0,
            mem_usage: 6.4,
            gpu_usage: 4.5,
            command: "firefox --isolated-process".to_string(),
            cmdline: "/usr/lib/firefox/firefox -contentproc -isForBrowser".to_string(),
            read_mbs: 5.2,
            write_mbs: 0.8,
        },
        ProcessItem {
            pid: 4890,
            user: "operator".to_string(),
            cpu_usage: 14.5,
            mem_usage: 2.1,
            gpu_usage: 0.0,
            command: "cargo build --release".to_string(),
            cmdline: "cargo build --release --workspace".to_string(),
            read_mbs: 42.1,
            write_mbs: 18.5,
        },
        ProcessItem {
            pid: 1892,
            user: "operator".to_string(),
            cpu_usage: 8.2,
            mem_usage: 1.8,
            gpu_usage: 3.2,
            command: "alacritty (neovim)".to_string(),
            cmdline: "alacritty -e nvim src/main.rs".to_string(),
            read_mbs: 0.5,
            write_mbs: 0.1,
        },
        ProcessItem {
            pid: 982,
            user: "system".to_string(),
            cpu_usage: 4.8,
            mem_usage: 1.2,
            gpu_usage: 0.0,
            command: "dockerd /containerd".to_string(),
            cmdline: "/usr/bin/dockerd -H fd:// --containerd=/run/containerd/containerd.sock"
                .to_string(),
            read_mbs: 1.2,
            write_mbs: 2.8,
        },
        ProcessItem {
            pid: 1120,
            user: "system".to_string(),
            cpu_usage: 2.1,
            mem_usage: 0.9,
            gpu_usage: 0.0,
            command: "pipewire-pulse".to_string(),
            cmdline: "/usr/bin/pipewire-pulse".to_string(),
            read_mbs: 0.0,
            write_mbs: 0.0,
        },
        ProcessItem {
            pid: 745,
            user: "system".to_string(),
            cpu_usage: 1.5,
            mem_usage: 0.8,
            gpu_usage: 0.0,
            command: "systemd-journald".to_string(),
            cmdline: "/usr/lib/systemd/systemd-journald".to_string(),
            read_mbs: 0.1,
            write_mbs: 1.4,
        },
    ]
}

const NORMAL_KEYS: &[KeyHint] = &[
    KeyHint {
        key: "Tab",
        label: "Ordenar",
        description: "Ordenar processos: CPU → MEM → GPU → IO",
    },
    KeyHint {
        key: "↑↓",
        label: "Navegar",
        description: "Selecionar e rolar processos",
    },
    KeyHint {
        key: "c",
        label: "Comando",
        description: "Mostrar nome ou linha de comando completa",
    },
    KeyHint {
        key: "/",
        label: "Filtrar",
        description: "Filtrar por nome, comando ou PID",
    },
    KeyHint {
        key: "k",
        label: "Encerrar",
        description: "Encerrar o processo selecionado (SIGTERM)",
    },
];

/// Enquanto o filtro está sendo digitado
const FILTER_KEYS: &[KeyHint] = &[
    KeyHint {
        key: "Enter",
        label: "Aplicar",
        description: "Aplicar o filtro",
    },
    KeyHint {
        key: "Esc",
        label: "Limpar",
        description: "Limpar o filtro",
    },
];

/// Enquanto espera a confirmação do encerramento
const CONFIRM_KEYS: &[KeyHint] = &[
    KeyHint {
        key: "s",
        label: "Encerrar",
        description: "Confirmar: enviar SIGTERM ao processo",
    },
    KeyHint {
        key: "n",
        label: "Cancelar",
        description: "Cancelar o encerramento",
    },
];

impl Panel for ProcessesPanel {
    fn id(&self) -> &'static str {
        "processes"
    }

    fn update(&mut self, ctx: &Context) {
        if let Some((pid, name)) = self.pending_kill.take() {
            self.status = Some(send_sigterm(ctx.system, pid, &name));
        }
        self.collect(ctx);
    }

    fn render(&self, area: Rect, buf: &mut Buffer, view: &View) {
        render_processes(self, view.unicode_icons, buf, area);
    }

    fn handle_key(&mut self, key: KeyEvent) -> Handled {
        if self.mode == Mode::Filtering {
            self.handle_filter_key(key);
            return Handled::Yes;
        }
        if matches!(self.mode, Mode::ConfirmKill { .. }) {
            self.handle_confirm_key(key);
            return Handled::Yes;
        }

        let had_status = self.status.take().is_some();
        match key.code {
            KeyCode::Tab => {
                self.sort_key = match self.sort_key {
                    ProcessSort::Cpu => ProcessSort::Mem,
                    ProcessSort::Mem => ProcessSort::Gpu,
                    ProcessSort::Gpu => ProcessSort::Io,
                    ProcessSort::Io => ProcessSort::Cpu,
                };
                self.reset_selection();
                // Só reordena: coletar de novo a <1 s da última leitura distorce o %CPU
                self.apply_sort();
            }
            KeyCode::Up => self.move_up(),
            KeyCode::Down => self.move_down(),
            KeyCode::Char('c') => self.show_cmdline = !self.show_cmdline,
            KeyCode::Char('/') => self.mode = Mode::Filtering,
            KeyCode::Char('k') => {
                let target = self
                    .visible()
                    .get(self.selected)
                    .map(|p| (p.pid, p.command.clone()));
                let Some((pid, name)) = target else {
                    return Handled::No;
                };
                self.mode = Mode::ConfirmKill { pid, name };
            }
            // Esc desfaz o estado do painel antes de chegar a encerrar o monitor
            KeyCode::Esc if had_status => {}
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.reset_selection();
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn keybindings(&self) -> &[KeyHint] {
        match self.mode {
            Mode::Normal => NORMAL_KEYS,
            Mode::Filtering => FILTER_KEYS,
            Mode::ConfirmKill { .. } => CONFIRM_KEYS,
        }
    }

    fn captures_input(&self) -> bool {
        self.mode != Mode::Normal
    }

    fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> Handled {
        if self.mode != Mode::Normal {
            // no meio de uma digitação ou confirmação, o mouse não muda a seleção
            return Handled::Yes;
        }
        match event.kind {
            MouseEventKind::ScrollDown => self.move_down(),
            MouseEventKind::ScrollUp => self.move_up(),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(sort) = tab_at(area, event.column, event.row) {
                    self.sort_key = sort;
                    self.reset_selection();
                    self.apply_sort();
                } else if let Some(row) = row_at(area, event.row, self.visible_rows.get()) {
                    let index = self.drawn_offset.get() + row;
                    if index < self.visible().len() {
                        self.selected = index;
                        self.scroll_offset = self.drawn_offset.get();
                    }
                }
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn min_size(&self) -> Size {
        Size {
            width: 50,
            height: 5,
        }
    }
}

fn render_processes(p: &ProcessesPanel, unicode_icons: bool, buf: &mut Buffer, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER_COLOR));
    let inner = block.inner(area);
    block.render(area, buf);

    // Botões de abas no cabeçalho: CPU, MEM, GPU, IO
    let cpu_style = if p.sort_key == ProcessSort::Cpu {
        Style::default()
            .fg(Color::Black)
            .bg(CYAN_NEON)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(TEXT_DIM)
    };
    let mem_style = if p.sort_key == ProcessSort::Mem {
        Style::default()
            .fg(Color::Black)
            .bg(MAGENTA_NEON)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(TEXT_DIM)
    };
    let gpu_style = if p.sort_key == ProcessSort::Gpu {
        Style::default()
            .fg(Color::Black)
            .bg(YELLOW_NEON)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(TEXT_DIM)
    };
    let io_style = if p.sort_key == ProcessSort::Io {
        Style::default()
            .fg(Color::Black)
            .bg(GREEN_NEON)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(TEXT_DIM)
    };

    let p_icon = format!(" {} PROCESSOS ", icon_proc(unicode_icons));
    let header_line = Line::from(vec![
        Span::styled(
            p_icon,
            Style::default()
                .fg(MAGENTA_NEON)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("(Top) ", Style::default().fg(TEXT_DIM)),
    ]);
    buf.set_line(area.left() + 1, area.top(), &header_line, area.width);

    let tabs_line = Line::from(vec![
        Span::styled(" CPU ", cpu_style),
        Span::raw(" "),
        Span::styled(" MEM ", mem_style),
        Span::raw(" "),
        Span::styled(" GPU ", gpu_style),
        Span::raw(" "),
        Span::styled(" IO ", io_style),
        Span::raw(" "),
    ]);
    let tabs_len = 24;
    if area.width > 50 {
        buf.set_line(
            area.right().saturating_sub(tabs_len + 2),
            area.top(),
            &tabs_line,
            tabs_len,
        );
    }

    let mut rows = vec![Row::new(vec![
        Cell::from("#").style(Style::default().fg(TEXT_DIM)),
        Cell::from("PID").style(Style::default().fg(TEXT_DIM)),
        Cell::from("USER").style(Style::default().fg(TEXT_DIM)),
        Cell::from("%CPU").style(Style::default().fg(TEXT_DIM)),
        Cell::from("%MEM").style(Style::default().fg(TEXT_DIM)),
        Cell::from("%GPU").style(Style::default().fg(TEXT_DIM)),
        Cell::from("COMMAND").style(Style::default().fg(TEXT_DIM)),
    ])];

    // Linha de baixo: filtro sendo digitado, confirmação, resultado de uma ação ou filtro ativo
    let visible = p.visible();
    let bar = status_bar(p, visible.len());
    let table_height = inner.height.saturating_sub(u16::from(bar.is_some()));
    let max_visible = (table_height.saturating_sub(1) as usize).max(1);
    p.visible_rows.set(max_visible);
    // Garante a linha selecionada visível mesmo após redimensionar o terminal
    let selected = p.selected.min(visible.len().saturating_sub(1));
    let offset = p
        .scroll_offset
        .clamp(selected.saturating_sub(max_visible - 1), selected);
    p.drawn_offset.set(offset);
    let slice = visible.iter().skip(offset).take(max_visible);

    let show_cmdline = p.show_cmdline;
    for (idx, p) in slice.enumerate() {
        let global_idx = offset + idx;
        let is_selected = global_idx == selected;

        let num_str = (global_idx + 1).to_string();
        let pid_str = p.pid.to_string();
        let user_str = p.user.clone();
        let cpu_str = format!("{:.1}", p.cpu_usage);
        let mem_str = format!("{:.1}", p.mem_usage);
        let gpu_str = format!("{:.1}", p.gpu_usage);
        let cmd_str = if show_cmdline {
            p.cmdline.clone()
        } else {
            p.command.clone()
        };

        let row = if is_selected {
            Row::new(vec![
                Cell::from(num_str)
                    .style(Style::default().fg(CYAN_NEON).add_modifier(Modifier::BOLD)),
                Cell::from(pid_str).style(Style::default().fg(TEXT_WHITE)),
                Cell::from(user_str).style(Style::default().fg(CYAN_NEON)),
                Cell::from(cpu_str).style(
                    Style::default()
                        .fg(YELLOW_NEON)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(mem_str).style(
                    Style::default()
                        .fg(MAGENTA_NEON)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(gpu_str).style(
                    Style::default()
                        .fg(YELLOW_NEON)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(cmd_str)
                    .style(Style::default().fg(TEXT_WHITE).add_modifier(Modifier::BOLD)),
            ])
            .style(Style::default().bg(Color::Rgb(16, 28, 48)))
        } else {
            Row::new(vec![
                Cell::from(num_str).style(Style::default().fg(TEXT_DIM)),
                Cell::from(pid_str).style(Style::default().fg(TEXT_WHITE)),
                Cell::from(user_str).style(Style::default().fg(CYAN_NEON)),
                Cell::from(cpu_str).style(Style::default().fg(if p.cpu_usage > 5.0 {
                    YELLOW_NEON
                } else {
                    GREEN_NEON
                })),
                Cell::from(mem_str).style(Style::default().fg(TEXT_WHITE)),
                Cell::from(gpu_str).style(Style::default().fg(if p.gpu_usage > 1.0 {
                    YELLOW_NEON
                } else {
                    TEXT_DIM
                })),
                Cell::from(cmd_str).style(Style::default().fg(TEXT_WHITE)),
            ])
        };
        rows.push(row);
    }

    let table = Table::new(
        rows,
        [
            Constraint::Length(3),
            Constraint::Length(7),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Min(10),
        ],
    );
    table.render(
        Rect::new(inner.left(), inner.top(), inner.width, table_height),
        buf,
    );

    if let Some((text, style)) = bar {
        let y = inner.top() + table_height;
        set_string_clipped(buf, inner, inner.left() + 1, y, &text, style);
    }
}

/// Filtro sem diferenciar maiúsculas: nome, linha de comando ou PID (`needle` já em minúsculas)
fn matches_filter(p: &ProcessItem, needle: &str) -> bool {
    p.command.to_lowercase().contains(needle)
        || p.cmdline.to_lowercase().contains(needle)
        || p.pid.to_string().contains(needle)
}

/// Envia SIGTERM (pede para o processo terminar) e descreve o resultado
fn send_sigterm(system: &System, pid: u32, name: &str) -> String {
    match system.process(Pid::from_u32(pid)) {
        None => format!("PID {pid} ({name}) já tinha terminado"),
        Some(process) => match process.kill_with(Signal::Term) {
            Some(true) => format!("SIGTERM enviado para {pid} ({name})"),
            Some(false) => format!("Sem permissão para encerrar {pid} ({name})"),
            None => "Este sistema não suporta SIGTERM".to_string(),
        },
    }
}

/// Texto e estilo da linha de baixo do painel, quando há algo a mostrar
fn status_bar(p: &ProcessesPanel, shown: usize) -> Option<(String, Style)> {
    match &p.mode {
        Mode::Filtering => Some((
            format!("Filtrar: {}▏  Enter aplica, Esc limpa", p.filter),
            Style::default()
                .fg(YELLOW_NEON)
                .add_modifier(Modifier::BOLD),
        )),
        Mode::ConfirmKill { pid, name } => Some((
            format!("Encerrar PID {pid} ({name})? s/N"),
            Style::default().fg(RED_ALERT).add_modifier(Modifier::BOLD),
        )),
        Mode::Normal => match (&p.status, p.filter.is_empty()) {
            (Some(message), _) => Some((message.clone(), Style::default().fg(CYAN_NEON))),
            (None, false) => Some((
                format!(
                    "Filtro: {}  ({} de {}), Esc limpa",
                    p.filter,
                    shown,
                    p.processes.len()
                ),
                Style::default().fg(TEXT_DIM),
            )),
            (None, true) => None,
        },
    }
}

/// Linha da lista sob o mouse (0 = primeira desenhada): borda + cabeçalho vêm antes
fn row_at(area: Rect, y: u16, visible_rows: usize) -> Option<usize> {
    let first = area.y + 2;
    (y >= first && usize::from(y - first) < visible_rows).then(|| usize::from(y - first))
}

/// Aba de ordenação sob o mouse, na borda superior (mesmas posições do desenho)
fn tab_at(area: Rect, x: u16, y: u16) -> Option<ProcessSort> {
    if y != area.y || area.width <= 50 {
        return None;
    }
    let start = area.right().saturating_sub(24 + 2);
    match x.checked_sub(start)? {
        0..=4 => Some(ProcessSort::Cpu),
        6..=10 => Some(ProcessSort::Mem),
        12..=16 => Some(ProcessSort::Gpu),
        18..=21 => Some(ProcessSort::Io),
        _ => None,
    }
}

/// Junta os argumentos; sem nenhum (threads do kernel), mostra o nome entre colchetes,
/// como o ps e o htop
fn cmdline_or_name(args: &[std::ffi::OsString], name: &str) -> String {
    if args.is_empty() {
        format!("[{name}]")
    } else {
        args.iter()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Soma o uso de uma placa ao total por processo (um processo pode usar várias placas)
fn add_gpu_util(total: &mut HashMap<u32, f32>, device: HashMap<u32, f32>) {
    for (pid, util) in device {
        *total.entry(pid).or_default() += util;
    }
}

/// Fica com a amostra mais recente de cada PID; devolve também o timestamp mais novo visto
fn latest_sm_util_per_pid(
    samples: &[ProcessUtilizationSample],
) -> (HashMap<u32, f32>, Option<u64>) {
    let mut latest: HashMap<u32, (u64, u32)> = HashMap::new();
    for s in samples {
        let entry = latest.entry(s.pid).or_insert((s.timestamp, s.sm_util));
        if s.timestamp >= entry.0 {
            *entry = (s.timestamp, s.sm_util);
        }
    }
    let newest_ts = samples.iter().map(|s| s.timestamp).max();
    let per_pid = latest
        .into_iter()
        .map(|(pid, (_, sm))| (pid, sm.min(100) as f32))
        .collect();
    (per_pid, newest_ts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_panel(panel: &ProcessesPanel, width: u16, height: u16) -> String {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        panel.render(area, &mut buf, &View::default());
        (0..height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn sessenta_processos() -> ProcessesPanel {
        let base = demo_processes()[0].clone();
        ProcessesPanel::with(
            (0..60u32)
                .map(|i| ProcessItem {
                    pid: 90_000 + i,
                    ..base.clone()
                })
                .collect(),
            Users::new(),
        )
    }

    #[test]
    fn ordena_processos_pela_aba_ativa_sem_coletar() {
        let mut panel = ProcessesPanel::demo();
        for (sort, valor) in [
            (
                ProcessSort::Mem,
                (|p: &ProcessItem| p.mem_usage) as fn(&ProcessItem) -> f32,
            ),
            (ProcessSort::Gpu, |p: &ProcessItem| p.gpu_usage),
            (ProcessSort::Io, |p: &ProcessItem| p.read_mbs + p.write_mbs),
            (ProcessSort::Cpu, |p: &ProcessItem| p.cpu_usage),
        ] {
            panel.sort_key = sort;
            panel.apply_sort();
            let valores: Vec<f32> = panel.processes.iter().map(valor).collect();
            assert!(
                valores.windows(2).all(|w| w[0] >= w[1]),
                "{sort:?} fora de ordem: {valores:?}"
            );
        }
    }

    #[test]
    fn rolagem_usa_altura_real_do_painel() {
        // tamanho do painel de processos na tela de 160x40: 64x17 -> 14 linhas de tabela
        let mut panel = sessenta_processos();
        let text = render_panel(&panel, 64, 17);
        let shown = (0..60u32)
            .filter(|i| text.contains(&format!("{}", 90_000 + i)))
            .count();
        assert_eq!(shown, 14);
        assert_eq!(panel.visible_rows.get(), 14);

        // seleção além do offset salvo (ex.: terminal encolheu) continua visível
        panel.selected = 45;
        panel.scroll_offset = 0;
        assert!(render_panel(&panel, 64, 17).contains("90045"));
    }

    #[test]
    fn seta_para_baixo_rola_quando_passa_da_ultima_linha_visivel() {
        let mut panel = sessenta_processos();
        render_panel(&panel, 64, 17); // registra 14 linhas visíveis
        for _ in 0..20 {
            assert_eq!(
                panel.handle_key(KeyEvent::from(KeyCode::Down)),
                Handled::Yes
            );
        }
        assert_eq!(panel.selected, 20);
        assert_eq!(panel.scroll_offset, 20 + 1 - 14);
        for _ in 0..30 {
            panel.handle_key(KeyEvent::from(KeyCode::Up));
        }
        assert_eq!((panel.selected, panel.scroll_offset), (0, 0));
        assert_eq!(
            panel.handle_key(KeyEvent::from(KeyCode::Char('g'))),
            Handled::No
        );
    }

    fn sample(pid: u32, timestamp: u64, sm_util: u32) -> ProcessUtilizationSample {
        ProcessUtilizationSample {
            pid,
            timestamp,
            sm_util,
            mem_util: 0,
            enc_util: 0,
            dec_util: 0,
        }
    }

    #[test]
    fn uso_de_gpu_por_processo_usa_amostra_mais_recente() {
        let samples = [
            sample(10, 100, 80),
            sample(10, 300, 25),
            sample(20, 200, 60),
            sample(10, 250, 90),
        ];
        let (per_pid, newest) = latest_sm_util_per_pid(&samples);
        assert_eq!(per_pid.get(&10), Some(&25.0));
        assert_eq!(per_pid.get(&20), Some(&60.0));
        assert_eq!(newest, Some(300));

        let (vazio, sem_ts) = latest_sm_util_per_pid(&[]);
        assert!(vazio.is_empty());
        assert_eq!(sem_ts, None);
    }

    #[test]
    fn tecla_c_alterna_nome_e_linha_de_comando() {
        let mut panel = ProcessesPanel::demo();
        assert!(render_panel(&panel, 100, 12).contains("ollama-server"));
        assert_eq!(
            panel.handle_key(KeyEvent::from(KeyCode::Char('c'))),
            Handled::Yes
        );
        let text = render_panel(&panel, 100, 12);
        assert!(text.contains("/usr/local/bin/ollama serve"), "{text}");
        panel.handle_key(KeyEvent::from(KeyCode::Char('c')));
        assert!(render_panel(&panel, 100, 12).contains("ollama-server"));
    }

    #[test]
    fn linha_de_comando_junta_argumentos_ou_usa_o_nome_do_kernel() {
        let args: Vec<std::ffi::OsString> =
            vec!["python3".into(), "manage.py".into(), "runserver".into()];
        assert_eq!(
            cmdline_or_name(&args, "python3"),
            "python3 manage.py runserver"
        );
        assert_eq!(cmdline_or_name(&[], "kworker/0:1"), "[kworker/0:1]");
    }

    #[test]
    fn uso_de_gpu_soma_as_placas_por_processo() {
        let mut total = HashMap::new();
        add_gpu_util(&mut total, HashMap::from([(10, 40.0), (20, 15.0)]));
        add_gpu_util(&mut total, HashMap::from([(10, 25.0), (30, 5.0)]));
        assert_eq!(total.get(&10), Some(&65.0));
        assert_eq!(total.get(&20), Some(&15.0));
        assert_eq!(total.get(&30), Some(&5.0));
    }

    fn tecla(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn digitar(panel: &mut ProcessesPanel, texto: &str) {
        for c in texto.chars() {
            assert_eq!(panel.handle_key(tecla(KeyCode::Char(c))), Handled::Yes);
        }
    }

    #[test]
    fn filtro_por_nome_comando_ou_pid() {
        let mut panel = ProcessesPanel::demo();
        assert_eq!(panel.handle_key(tecla(KeyCode::Char('/'))), Handled::Yes);
        assert!(panel.captures_input());
        digitar(&mut panel, "FIRE"); // sem diferenciar maiúsculas
        assert_eq!(panel.visible().len(), 1);
        assert_eq!(panel.handle_key(tecla(KeyCode::Enter)), Handled::Yes);
        assert!(!panel.captures_input());
        let text = render_panel(&panel, 100, 12);
        assert!(
            text.contains("firefox") && !text.contains("ollama"),
            "{text}"
        );
        assert!(text.contains("Filtro: FIRE  (1 de 8)"), "{text}");

        // PID e linha de comando também valem
        panel.filter = "1420".into();
        assert_eq!(panel.visible()[0].pid, 1420);
        panel.filter = "containerd.sock".into();
        assert_eq!(panel.visible()[0].command, "dockerd /containerd");
    }

    #[test]
    fn digitando_o_filtro_teclas_globais_viram_texto() {
        let mut panel = ProcessesPanel::demo();
        panel.handle_key(tecla(KeyCode::Char('/')));
        digitar(&mut panel, "q1p");
        assert_eq!(panel.filter, "q1p");
        panel.handle_key(tecla(KeyCode::Backspace));
        assert_eq!(panel.filter, "q1");
        assert_eq!(panel.keybindings()[0].key, "Enter");
        // Esc durante a digitação limpa e sai
        panel.handle_key(tecla(KeyCode::Esc));
        assert!(panel.filter.is_empty() && !panel.captures_input());
    }

    #[test]
    fn esc_limpa_o_filtro_antes_de_deixar_o_monitor_fechar() {
        let mut panel = ProcessesPanel::demo();
        panel.handle_key(tecla(KeyCode::Char('/')));
        digitar(&mut panel, "rust");
        panel.handle_key(tecla(KeyCode::Enter));
        assert_eq!(panel.handle_key(tecla(KeyCode::Esc)), Handled::Yes);
        assert!(panel.filter.is_empty());
        assert_eq!(
            panel.handle_key(tecla(KeyCode::Esc)),
            Handled::No,
            "sem filtro, o Esc segue para o monitor"
        );
    }

    #[test]
    fn encerrar_pede_confirmacao_e_no_demo_nao_envia_sinal() {
        let mut panel = ProcessesPanel::demo();
        assert_eq!(panel.handle_key(tecla(KeyCode::Char('k'))), Handled::Yes);
        assert!(panel.captures_input());
        let text = render_panel(&panel, 100, 12);
        assert!(text.contains("Encerrar PID 1420 (ollama-server"), "{text}");

        panel.handle_key(tecla(KeyCode::Char('n')));
        assert!(!panel.captures_input());
        assert!(panel.status.as_deref().unwrap().contains("cancelado"));

        panel.handle_key(tecla(KeyCode::Char('k')));
        panel.handle_key(tecla(KeyCode::Char('s')));
        assert_eq!(panel.pending_kill, None, "modo demo nunca envia sinal");
        assert!(panel.status.as_deref().unwrap().contains("Modo demo"));
    }

    #[test]
    fn encerramento_confirmado_sai_no_update() {
        // painel "real" (não demo), mas o sysinfo vazio: o processo não é encontrado e
        // nenhum sinal é enviado a ninguém
        let mut panel = ProcessesPanel::with(demo_processes(), Users::new());
        panel.handle_key(tecla(KeyCode::Char('k')));
        panel.handle_key(tecla(KeyCode::Char('s')));
        assert_eq!(panel.pending_kill.as_ref().map(|(pid, _)| *pid), Some(1420));
        let system = System::new();
        panel.update(&Context {
            system: &system,
            nvml: None,
        });
        assert_eq!(panel.pending_kill, None);
        assert!(
            panel
                .status
                .as_deref()
                .unwrap()
                .contains("já tinha terminado")
        );
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: nebula_core::crossterm::event::KeyModifiers::NONE,
        }
    }

    #[test]
    fn mouse_seleciona_linha_rola_e_troca_a_aba() {
        let area = Rect::new(10, 5, 64, 17);
        let mut panel = sessenta_processos();
        render_panel_at(&panel, area); // registra linhas visíveis e o deslocamento

        // borda (y=5), cabeçalho (y=6), primeira linha de dados em y=7
        let click = MouseEventKind::Down(MouseButton::Left);
        assert_eq!(panel.handle_mouse(mouse(click, 20, 9), area), Handled::Yes);
        assert_eq!(panel.selected, 2);

        panel.handle_mouse(mouse(MouseEventKind::ScrollDown, 20, 9), area);
        assert_eq!(panel.selected, 3);
        panel.handle_mouse(mouse(MouseEventKind::ScrollUp, 20, 9), area);
        assert_eq!(panel.selected, 2);

        // aba MEM na borda superior: começa 26 colunas antes da borda direita + 6
        let mem_x = area.right() - 26 + 7;
        panel.handle_mouse(mouse(click, mem_x, area.y), area);
        assert_eq!(panel.sort_key, ProcessSort::Mem);
        assert_eq!(panel.selected, 0);
    }

    fn render_panel_at(panel: &ProcessesPanel, area: Rect) {
        let mut buf = Buffer::empty(Rect::new(0, 0, area.right(), area.bottom()));
        panel.render(area, &mut buf, &View::default());
    }
}
