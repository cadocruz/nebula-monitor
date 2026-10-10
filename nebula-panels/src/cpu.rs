//! Painel de CPU: medidor geral, núcleos (uso, frequência, temperatura), load average,
//! histórico, consumo (RAPL) e a tabela dos processos que mais usam CPU.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use nebula_core::ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Row, Table, Widget},
};
use nebula_core::sysinfo::{Components, System, Users};
use nebula_core::theme::*;
use nebula_core::widgets::*;
use nebula_core::{Context, Panel, Size, View};

use crate::processes::ProcessItem;

#[derive(Debug, Clone)]
pub struct CoreInfo {
    pub id: usize,
    pub usage: f64,
    pub history: VecDeque<f64>,
    pub freq_ghz: f64,
    /// Temperatura do núcleo; None quando não há sensor
    pub temp_c: Option<f64>,
}

/// Contador de energia de um domínio RAPL de pacote (µJ, volta a zero em max_range_uj)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RaplDomain {
    energy_uj: u64,
    max_range_uj: u64,
}

struct RaplSample {
    domains: Vec<RaplDomain>,
    timestamp: Instant,
}

pub struct CpuPanel {
    cpu_brand: String,
    cpu_cores_physical: usize,
    cpu_cores_logical: usize,
    cpu_overall_usage: f64,
    cpu_history: VecDeque<f64>,
    cpu_cores: Vec<CoreInfo>,
    cpu_temp_c: Option<f64>,
    /// Temperaturas dos CCDs informadas pelo k10temp; não são temperaturas por núcleo.
    ccd_temps: Vec<(u32, f64)>,
    /// CPU lógica -> número do CCD usado como temperatura compartilhada na linha.
    ccd_for_core: HashMap<usize, u32>,
    /// Medido via RAPL; None sem acesso aos contadores (normalmente exige root)
    cpu_power_w: Option<f64>,
    cpu_avg_freq_ghz: f64,
    load_avg: (f64, f64, f64),
    top_cpu_processes: Vec<ProcessItem>,
    /// Sensores de temperatura (sysinfo)
    components: Components,
    /// Lista de usuários lida uma vez, para a coluna USER da tabela
    users: Users,
    prev_rapl: Option<RaplSample>,
    /// (pacote, núcleo físico) de cada CPU lógica, na ordem de System::cpus()
    cpu_topology: Vec<Option<(u32, u32)>>,
    /// Die de cada CPU lógica, usado para associar TccdN ao CCD correspondente.
    cpu_die_ids: Vec<Option<u32>>,
}

impl Default for CpuPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuPanel {
    pub fn new() -> Self {
        Self {
            cpu_brand: String::new(),
            cpu_cores_physical: 0,
            cpu_cores_logical: 0,
            cpu_overall_usage: 0.0,
            cpu_history: VecDeque::from(vec![0.0; 120]),
            cpu_cores: Vec::new(),
            cpu_temp_c: None,
            ccd_temps: Vec::new(),
            ccd_for_core: HashMap::new(),
            cpu_power_w: None,
            cpu_avg_freq_ghz: 0.0,
            load_avg: (0.0, 0.0, 0.0),
            top_cpu_processes: Vec::new(),
            components: Components::new_with_refreshed_list(),
            users: Users::new_with_refreshed_list(),
            prev_rapl: None,
            cpu_topology: Vec::new(),
            cpu_die_ids: Vec::new(),
        }
    }

    /// Dados fictícios do modo demonstração
    pub fn demo() -> Self {
        let mut cpu_cores = vec![
            CoreInfo {
                id: 0,
                usage: 48.0,
                history: VecDeque::from(vec![0.3, 0.4, 0.5, 0.48]),
                freq_ghz: 4.85,
                temp_c: None,
            },
            CoreInfo {
                id: 1,
                usage: 62.0,
                history: VecDeque::from(vec![0.4, 0.5, 0.6, 0.62]),
                freq_ghz: 5.10,
                temp_c: None,
            },
            CoreInfo {
                id: 2,
                usage: 35.0,
                history: VecDeque::from(vec![0.2, 0.3, 0.4, 0.35]),
                freq_ghz: 4.60,
                temp_c: None,
            },
            CoreInfo {
                id: 3,
                usage: 84.0,
                history: VecDeque::from(vec![0.6, 0.7, 0.8, 0.84]),
                freq_ghz: 5.35,
                temp_c: None,
            },
            CoreInfo {
                id: 4,
                usage: 22.0,
                history: VecDeque::from(vec![0.1, 0.2, 0.2, 0.22]),
                freq_ghz: 4.40,
                temp_c: None,
            },
            CoreInfo {
                id: 5,
                usage: 55.0,
                history: VecDeque::from(vec![0.4, 0.5, 0.5, 0.55]),
                freq_ghz: 4.90,
                temp_c: None,
            },
            CoreInfo {
                id: 6,
                usage: 71.0,
                history: VecDeque::from(vec![0.5, 0.6, 0.7, 0.71]),
                freq_ghz: 5.20,
                temp_c: None,
            },
            CoreInfo {
                id: 7,
                usage: 39.0,
                history: VecDeque::from(vec![0.3, 0.4, 0.4, 0.39]),
                freq_ghz: 4.70,
                temp_c: None,
            },
            CoreInfo {
                id: 8,
                usage: 18.0,
                history: VecDeque::from(vec![0.1, 0.1, 0.2, 0.18]),
                freq_ghz: 4.30,
                temp_c: None,
            },
            CoreInfo {
                id: 9,
                usage: 65.0,
                history: VecDeque::from(vec![0.5, 0.6, 0.6, 0.65]),
                freq_ghz: 5.15,
                temp_c: None,
            },
            CoreInfo {
                id: 10,
                usage: 42.0,
                history: VecDeque::from(vec![0.3, 0.4, 0.4, 0.42]),
                freq_ghz: 4.75,
                temp_c: None,
            },
            CoreInfo {
                id: 11,
                usage: 58.0,
                history: VecDeque::from(vec![0.4, 0.5, 0.6, 0.58]),
                freq_ghz: 5.00,
                temp_c: None,
            },
        ];
        let mut ccd_for_core = HashMap::new();
        for core in &mut cpu_cores {
            let (ccd, temp) = if core.id < 8 { (1, 42.0) } else { (2, 43.0) };
            core.temp_c = Some(temp);
            ccd_for_core.insert(core.id, ccd);
        }

        let mut cpu_hist = VecDeque::new();
        for i in 0..120 {
            let t = i as f64 * 0.1;
            let val = (0.42 + 0.25 * (t * 0.5).sin() + 0.15 * (t * 1.3).cos()).clamp(0.1, 0.95);
            cpu_hist.push_back(val);
        }

        Self {
            cpu_brand: "AMD Ryzen 9 7950X (16 cores / 32 threads)".to_string(),
            cpu_cores_physical: 16,
            cpu_cores_logical: 32,
            cpu_overall_usage: 48.0,
            cpu_history: cpu_hist,
            cpu_cores,
            cpu_temp_c: Some(46.0),
            ccd_temps: vec![(1, 42.0), (2, 43.0)],
            ccd_for_core,
            cpu_power_w: Some(125.0),
            cpu_avg_freq_ghz: 4.82,
            load_avg: (1.45, 1.22, 0.98),
            top_cpu_processes: crate::processes::demo_processes(),
            components: Components::new(),
            users: Users::new(),
            prev_rapl: None,
            cpu_topology: Vec::new(),
            cpu_die_ids: Vec::new(),
        }
    }

    fn collect(&mut self, ctx: &Context) {
        self.components.refresh(true);
        let cpu_count = ctx.system.cpus().len();
        if self.cpu_topology.len() != cpu_count {
            self.cpu_topology = read_cpu_topology(Path::new("/sys/devices/system/cpu"), cpu_count);
        }
        if self.cpu_die_ids.len() != cpu_count {
            self.cpu_die_ids = read_cpu_die_ids(Path::new("/sys/devices/system/cpu"), cpu_count);
        }

        self.cpu_brand = ctx
            .system
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .unwrap_or_else(|| "Unknown Processor".to_string());
        self.cpu_cores_logical = ctx.system.cpus().len();
        self.cpu_cores_physical =
            System::physical_core_count().unwrap_or(self.cpu_cores_logical / 2);

        let cpus = ctx.system.cpus();
        let mut sum_usage = 0.0;
        let mut sum_freq = 0.0;
        for c in cpus {
            sum_usage += c.cpu_usage() as f64;
            sum_freq += c.frequency() as f64;
        }
        let count = cpus.len().max(1) as f64;
        self.cpu_overall_usage = sum_usage / count;
        self.cpu_avg_freq_ghz = (sum_freq / count) / 1000.0;

        if self.cpu_history.len() >= 120 {
            self.cpu_history.pop_front();
        }
        self.cpu_history
            .push_back((self.cpu_overall_usage / 100.0).clamp(0.0, 1.0));

        // None quando nenhum sensor de CPU é encontrado (a tela omite a temperatura)
        // A ordem da lista de sensores varia. Em AMD, Tctl representa a CPU no
        // título; TccdN fica na linha própria abaixo de Load Average.
        let cpu_temp = ["tctl", "tdie", "package", "cpu"].iter().find_map(|name| {
            self.components.list().iter().find_map(|c| {
                c.label()
                    .to_lowercase()
                    .contains(name)
                    .then(|| c.temperature().map(|t| t as f64))
                    .flatten()
            })
        });
        self.cpu_temp_c = cpu_temp;
        self.ccd_temps = read_ccd_temps(Path::new("/sys/class/hwmon"));

        // Consumo medido pelos contadores RAPL; precisa de duas leituras e, desde o kernel
        // 5.10, de permissão de leitura em energy_uj (normalmente só root) — senão None.
        let now = Instant::now();
        let rapl = read_rapl(Path::new("/sys/class/powercap"));
        self.cpu_power_w = match (&self.prev_rapl, &rapl) {
            (Some(prev), Some(cur)) => rapl_power_watts(
                &prev.domains,
                cur,
                now.duration_since(prev.timestamp).as_secs_f64(),
            ),
            _ => None,
        };
        self.prev_rapl = rapl.map(|domains| RaplSample {
            domains,
            timestamp: now,
        });

        // Sensor por núcleo físico (Intel coretemp); AMD k10temp não expõe por núcleo -> None
        let core_temps = read_coretemp(Path::new("/sys/class/hwmon"));
        let package = self.cpu_topology.iter().flatten().map(|(p, _)| *p).next();
        let single_package = package.is_some_and(|p| {
            self.cpu_topology
                .iter()
                .all(|topo| topo.is_some_and(|(candidate, _)| candidate == p))
        });
        self.ccd_for_core = if core_temps.is_empty() {
            match_ccd_to_cpus(&self.cpu_die_ids, &self.ccd_temps, single_package)
        } else {
            HashMap::new()
        };

        let now_cores = cpus
            .iter()
            .enumerate()
            .map(|(id, c)| {
                let usage = c.cpu_usage() as f64;
                let freq_ghz = (c.frequency() as f64) / 1000.0;
                let core_temp = self
                    .cpu_topology
                    .get(id)
                    .copied()
                    .flatten()
                    .and_then(|key| core_temps.get(&key).copied())
                    .or_else(|| {
                        let ccd = self.ccd_for_core.get(&id)?;
                        self.ccd_temps
                            .iter()
                            .find(|(sensor, _)| sensor == ccd)
                            .map(|(_, temp)| *temp)
                    });

                let mut hist = if let Some(old) = self.cpu_cores.get(id) {
                    old.history.clone()
                } else {
                    VecDeque::from(vec![0.0; 12])
                };
                if hist.len() >= 12 {
                    hist.pop_front();
                }
                hist.push_back((usage / 100.0).clamp(0.0, 1.0));

                CoreInfo {
                    id,
                    usage,
                    history: hist,
                    freq_ghz,
                    temp_c: core_temp,
                }
            })
            .collect();
        self.cpu_cores = now_cores;

        let l = System::load_average();
        self.load_avg = (l.one, l.five, l.fifteen);

        self.collect_top_cpu(ctx);
    }

    /// Os 5 processos que mais usam CPU, para a tabela "Top CPU Processes"
    fn collect_top_cpu(&mut self, ctx: &Context) {
        let mut procs = Vec::new();

        // Total vem direto do sysinfo: os processos não dependem do painel de Memória
        let total_mem = ctx.system.total_memory().max(1) as f32;

        for (pid, process) in ctx.system.processes() {
            let pid_u32 = pid.as_u32();
            let cpu = process.cpu_usage();
            let mem_bytes = process.memory() as f32;
            let mem_pct = (mem_bytes / total_mem) * 100.0;

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
            let disk_usage = process.disk_usage();
            let r_mbs = (disk_usage.read_bytes as f32) / (1024.0 * 1024.0);
            let w_mbs = (disk_usage.written_bytes as f32) / (1024.0 * 1024.0);

            procs.push(ProcessItem {
                pid: pid_u32,
                user,
                cpu_usage: cpu,
                mem_usage: mem_pct,
                gpu_usage: 0.0, // a tabela da CPU não mostra %GPU
                command: name,
                cmdline: String::new(), // a tabela da CPU mostra só o nome
                read_mbs: r_mbs,
                write_mbs: w_mbs,
            });
        }

        let mut top_cpu = procs;
        top_cpu.sort_by(|a, b| b.cpu_usage.total_cmp(&a.cpu_usage));
        self.top_cpu_processes = top_cpu.into_iter().take(5).collect();
    }
}

impl Panel for CpuPanel {
    fn id(&self) -> &'static str {
        "cpu"
    }

    fn update(&mut self, ctx: &Context) {
        self.collect(ctx);
    }

    fn render(&self, area: Rect, buf: &mut Buffer, view: &View) {
        render_cpu(self, view.unicode_icons, buf, area);
    }

    fn min_size(&self) -> Size {
        // medidor (26) + núcleos; medidor (8) + load/histórico (5) + tabela (mín. 6)
        Size {
            width: 70,
            height: 16,
        }
    }
}

fn render_cpu(panel: &CpuPanel, unicode_icons: bool, buf: &mut Buffer, area: Rect) {
    let brand_clean = panel
        .cpu_brand
        .replace("12-Core Processor", "")
        .replace("8-Core Processor", "")
        .replace("16-Core Processor", "")
        .trim()
        .to_string();

    let c_icon = format!(" {} CPU ", icon_cpu(unicode_icons));
    let title = Line::from(vec![
        Span::styled(
            c_icon,
            Style::default().fg(CYAN_NEON).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "{} ({} cores / {} threads) ",
                brand_clean, panel.cpu_cores_physical, panel.cpu_cores_logical
            ),
            Style::default().fg(TEXT_WHITE),
        ),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER_COLOR));
    let inner = block.inner(area);
    block.render(area, buf);

    // Métricas no canto superior direito da borda
    let mut r_spans = vec![Span::styled(
        format!("{:.1} GHz ", panel.cpu_avg_freq_ghz),
        Style::default().fg(CYAN_NEON),
    )];
    if let Some(t) = panel.cpu_temp_c {
        r_spans.push(Span::styled("| ", Style::default().fg(CYAN_DIM)));
        r_spans.push(Span::styled(
            format!("T: {:.0}°C ", t),
            Style::default().fg(YELLOW_NEON),
        ));
    }
    if let Some(w) = panel.cpu_power_w {
        r_spans.push(Span::styled("| ", Style::default().fg(CYAN_DIM)));
        r_spans.push(Span::styled(
            format!("{:.1} W ", w),
            Style::default().fg(CYAN_NEON),
        ));
    }
    render_panel_titles(buf, area, &title, Some(&Line::from(r_spans)));

    // O Painel CPU é dividido em 3 seções verticais (idêntico ao mockup):
    // Seção 1 (Top, 8 linhas): Gauge Circular (esq, 26 cols) | Cores 0..7 (dir, resto)
    // Seção 2 (Mid, 4 linhas): Load Average (esq, 26 cols)  | Histórico CPU Line Chart (dir, resto)
    // Seção 3 (Bottom, min 7): Top CPU Processes (largura inteira!)
    let v_sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),
            Constraint::Length(5),
            Constraint::Min(6),
        ])
        .split(inner);

    let left_col_w = 30;

    // --- SEÇÃO 1 (TOP): GAUGE (ESQ) + CORES 0..7 (DIR) ---
    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(left_col_w), Constraint::Min(40)])
        .split(v_sections[0]);

    render_circular_gauge(buf, top_cols[0], panel.cpu_overall_usage, "CPU Usage", None);

    let cores_area = top_cols[1];
    let visible_cores = panel.cpu_cores.iter().take(8).collect::<Vec<_>>();
    for (i, core) in visible_cores.iter().enumerate() {
        let y = cores_area.top() + (i as u16);
        if y >= cores_area.bottom() {
            break;
        }

        let name = format!("Core {:<2}", core.id);
        let usage_str = format!("{:>3.0}%", core.usage);

        buf.set_string(cores_area.left(), y, &name, Style::default().fg(TEXT_WHITE));
        buf.set_string(
            cores_area.left() + 8,
            y,
            &usage_str,
            Style::default().fg(if core.usage < 60.0 {
                GREEN_NEON
            } else {
                YELLOW_NEON
            }),
        );

        // Barra de progresso proporcional
        // Até 12 blocos, deixando uma coluna livre antes da frequência (alinhada à direita)
        let freq_col = cores_area.right().saturating_sub(16);
        let bar_w = (freq_col.saturating_sub(cores_area.left() + 14 + 1) as usize).min(12);
        let bar_line = render_segmented_bar(core.usage, bar_w, GREEN_NEON);
        let mut cur_x = cores_area.left() + 14;
        for span in &bar_line.spans {
            buf.set_string(cur_x, y, &span.content, span.style);
            cur_x += span.content.chars().count() as u16;
        }

        // Alinhamento dinâmico à direita para preencher 100% da largura até a borda
        let temp_str = core
            .temp_c
            .map(|t| {
                if panel.ccd_for_core.contains_key(&core.id) {
                    format!("{t:.0}°C*")
                } else {
                    format!("{t:.0}°C")
                }
            })
            .unwrap_or_else(|| "  —".to_string());
        let freq_str = format!("{:.1} GHz", core.freq_ghz);

        let temp_x = cores_area.right().saturating_sub(6);
        let freq_x = temp_x.saturating_sub(10);
        let spark_start = cur_x + 2;
        let spark_end = freq_x.saturating_sub(2);

        if spark_end > spark_start {
            let spark_w = (spark_end - spark_start) as usize;
            let hist_slice: Vec<f64> = core.history.iter().copied().collect();
            let sp_span = braille_sparkline_len(
                &hist_slice,
                spark_w,
                if core.usage > 40.0 {
                    YELLOW_NEON
                } else {
                    CYAN_NEON
                },
            );
            buf.set_string(spark_start, y, &sp_span.content, sp_span.style);
        }

        buf.set_string(freq_x, y, &freq_str, Style::default().fg(CYAN_NEON));
        let temp_color = match core.temp_c {
            Some(t) if t >= 65.0 => YELLOW_NEON,
            Some(_) => GREEN_NEON,
            None => TEXT_DIM,
        };
        buf.set_string(temp_x, y, &temp_str, Style::default().fg(temp_color));
    }

    // --- SEÇÃO 2 (MID): LOAD AVERAGE (ESQ) + HISTÓRICO CPU LINE CHART (DIR) ---
    if v_sections.len() > 1 && v_sections[1].height >= 4 {
        let mid_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(left_col_w), Constraint::Min(40)])
            .split(v_sections[1]);

        let load_area = mid_cols[0];
        buf.set_string(
            load_area.left(),
            load_area.top() + 1,
            "Load Average (1m / 5m / 15m)",
            Style::default().fg(CYAN_NEON),
        );
        let load_val_line = Line::from(vec![
            Span::styled(
                format!(" {:.2}  ", panel.load_avg.0),
                Style::default().fg(GREEN_NEON).add_modifier(Modifier::BOLD),
            ),
            Span::styled("|  ", Style::default().fg(CYAN_DIM)),
            Span::styled(
                format!("{:.2}  ", panel.load_avg.1),
                Style::default().fg(CYAN_NEON).add_modifier(Modifier::BOLD),
            ),
            Span::styled("|  ", Style::default().fg(CYAN_DIM)),
            Span::styled(
                format!("{:.2}  ", panel.load_avg.2),
                Style::default()
                    .fg(YELLOW_NEON)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("|", Style::default().fg(CYAN_DIM)),
        ]);
        buf.set_line(
            load_area.left(),
            load_area.top() + 2,
            &load_val_line,
            load_area.width,
        );

        // Divisor vertical sutil entre Load Avg e o gráfico de linha
        let div_x = load_area.right();
        let div_color = Color::Rgb(20, 60, 80);
        for y in v_sections[1].top()..v_sections[1].bottom() {
            buf[(div_x, y)].set_char('│').set_fg(div_color);
        }

        // Gráfico de Histórico de CPU (perfeitamente alinhado com Cores 0..7 acima!)
        let chart_area = mid_cols[1];
        let cpu_hist: Vec<f64> = panel.cpu_history.iter().copied().collect();
        render_braille_line_chart(buf, chart_area, &cpu_hist, CYAN_NEON, "100%", "0%");
    }

    // --- SEÇÃO 3 (BOTTOM): TOP CPU PROCESSES (LARGURA COMPLETA) ---
    if v_sections.len() > 2 {
        let proc_area = v_sections[2];
        buf.set_string(
            proc_area.left(),
            proc_area.top(),
            "Top CPU Processes",
            Style::default().fg(CYAN_NEON).add_modifier(Modifier::BOLD),
        );

        let mut p_rows = vec![Row::new(vec![
            Cell::from("#").style(Style::default().fg(TEXT_DIM)),
            Cell::from("PID").style(Style::default().fg(TEXT_DIM)),
            Cell::from("USER").style(Style::default().fg(TEXT_DIM)),
            Cell::from("%CPU").style(Style::default().fg(TEXT_DIM)),
            Cell::from("%MEM").style(Style::default().fg(TEXT_DIM)),
            Cell::from("COMMAND").style(Style::default().fg(TEXT_DIM)),
        ])];

        for (idx, p) in panel.top_cpu_processes.iter().enumerate().take(5) {
            let row = Row::new(vec![
                Cell::from((idx + 1).to_string()).style(Style::default().fg(TEXT_DIM)),
                Cell::from(p.pid.to_string()).style(Style::default().fg(TEXT_WHITE)),
                Cell::from(p.user.clone()).style(Style::default().fg(CYAN_NEON)),
                Cell::from(format!("{:.1}", p.cpu_usage)).style(
                    Style::default()
                        .fg(YELLOW_NEON)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(format!("{:.1}", p.mem_usage)).style(Style::default().fg(MAGENTA_NEON)),
                Cell::from(p.command.clone()).style(Style::default().fg(TEXT_WHITE)),
            ]);
            p_rows.push(row);
        }

        let p_table = Table::new(
            p_rows,
            [
                Constraint::Length(4),
                Constraint::Length(9),
                Constraint::Length(8),
                Constraint::Length(8),
                Constraint::Length(8),
                Constraint::Min(15),
            ],
        );
        p_table.render(
            Rect::new(
                proc_area.left(),
                proc_area.top() + 1,
                proc_area.width,
                proc_area.height.saturating_sub(1),
            ),
            buf,
        );
    }
}

/// Lê os domínios de pacote do RAPL (`intel-rapl:N`, também usados por AMD Zen).
/// None se não houver RAPL ou se energy_uj não puder ser lido (exige root desde o kernel 5.10).
fn read_rapl(root: &Path) -> Option<Vec<RaplDomain>> {
    let mut dirs: Vec<_> = fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_prefix("intel-rapl:"))
                .is_some_and(|idx| !idx.is_empty() && idx.bytes().all(|b| b.is_ascii_digit()))
        })
        .collect();
    dirs.sort();

    let read_u64 = |dir: &Path, file: &str| {
        fs::read_to_string(dir.join(file))
            .ok()?
            .trim()
            .parse::<u64>()
            .ok()
    };
    let domains: Option<Vec<RaplDomain>> = dirs
        .iter()
        .map(|d| {
            Some(RaplDomain {
                energy_uj: read_u64(d, "energy_uj")?,
                max_range_uj: read_u64(d, "max_energy_range_uj")?,
            })
        })
        .collect();
    domains.filter(|d| !d.is_empty())
}

/// Potência média entre duas leituras, somando os pacotes e tratando a volta do contador
fn rapl_power_watts(prev: &[RaplDomain], cur: &[RaplDomain], dt_secs: f64) -> Option<f64> {
    if dt_secs <= 0.0 || prev.len() != cur.len() {
        return None;
    }
    let mut total_uj = 0u64;
    for (p, c) in prev.iter().zip(cur) {
        total_uj += if c.energy_uj >= p.energy_uj {
            c.energy_uj - p.energy_uj
        } else if c.max_range_uj > p.energy_uj {
            c.max_range_uj - p.energy_uj + c.energy_uj
        } else {
            return None;
        };
    }
    Some(total_uj as f64 / 1_000_000.0 / dt_secs)
}

/// (pacote, núcleo físico) de cada CPU lógica, lido de cpuN/topology
fn read_cpu_topology(cpu_root: &Path, count: usize) -> Vec<Option<(u32, u32)>> {
    let read_u32 = |path: PathBuf| fs::read_to_string(path).ok()?.trim().parse::<u32>().ok();
    (0..count)
        .map(|i| {
            let topo = cpu_root.join(format!("cpu{i}")).join("topology");
            Some((
                read_u32(topo.join("physical_package_id"))?,
                read_u32(topo.join("core_id"))?,
            ))
        })
        .collect()
}

fn read_cpu_die_ids(cpu_root: &Path, count: usize) -> Vec<Option<u32>> {
    (0..count)
        .map(|i| {
            fs::read_to_string(cpu_root.join(format!("cpu{i}/topology/die_id")))
                .ok()?
                .trim()
                .parse::<u32>()
                .ok()
        })
        .collect()
}

/// Tccd1 corresponde ao die_id 0, Tccd2 ao die_id 1, e assim por diante.
/// Só usamos essa relação quando a topologia identifica um único pacote.
fn match_ccd_to_cpus(
    die_ids: &[Option<u32>],
    ccd_temps: &[(u32, f64)],
    single_package: bool,
) -> HashMap<usize, u32> {
    if !single_package {
        return HashMap::new();
    }
    die_ids
        .iter()
        .enumerate()
        .filter_map(|(cpu, die)| {
            let ccd = (*die)?.checked_add(1)?;
            ccd_temps
                .iter()
                .any(|(sensor, _)| *sensor == ccd)
                .then_some((cpu, ccd))
        })
        .collect()
}

/// Temperaturas por núcleo do driver coretemp (Intel): rótulos "Core N" (N = core_id) e
/// "Package id P" em cada hwmon com name = coretemp. Chave: (pacote, núcleo).
fn read_coretemp(hwmon_root: &Path) -> HashMap<(u32, u32), f64> {
    let mut temps = HashMap::new();
    let Ok(entries) = fs::read_dir(hwmon_root) else {
        return temps;
    };
    for hwmon in entries.flatten().map(|e| e.path()) {
        let read = |file: &str| {
            fs::read_to_string(hwmon.join(file))
                .ok()
                .map(|s| s.trim().to_string())
        };
        if read("name").as_deref() != Some("coretemp") {
            continue;
        }
        let Ok(files) = fs::read_dir(&hwmon) else {
            continue;
        };
        let labels: Vec<(String, String)> = files
            .flatten()
            .filter_map(|f| {
                let name = f.file_name().into_string().ok()?;
                let sensor = name.strip_suffix("_label")?.to_string();
                Some((sensor, read(&name)?))
            })
            .collect();

        let package = labels
            .iter()
            .find_map(|(_, label)| label.strip_prefix("Package id ")?.parse::<u32>().ok())
            .unwrap_or(0);
        for (sensor, label) in &labels {
            let Some(core) = label
                .strip_prefix("Core ")
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            if let Some(milli) =
                read(&format!("{sensor}_input")).and_then(|v| v.parse::<f64>().ok())
            {
                temps.insert((package, core), milli / 1000.0);
            }
        }
    }
    temps
}

/// Temperaturas dos CCDs (TccdN) expostas pelo driver AMD k10temp.
fn read_ccd_temps(hwmon_root: &Path) -> Vec<(u32, f64)> {
    let mut temps = Vec::new();
    let Ok(entries) = fs::read_dir(hwmon_root) else {
        return temps;
    };
    for hwmon in entries.flatten().map(|e| e.path()) {
        let read = |file: &str| {
            fs::read_to_string(hwmon.join(file))
                .ok()
                .map(|s| s.trim().to_string())
        };
        if read("name").as_deref() != Some("k10temp") {
            continue;
        }
        let Ok(files) = fs::read_dir(&hwmon) else {
            continue;
        };
        for file in files.flatten() {
            let Some(name) = file.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(sensor) = name.strip_suffix("_label") else {
                continue;
            };
            let Some(id) =
                read(&name).and_then(|label| label.strip_prefix("Tccd")?.parse::<u32>().ok())
            else {
                continue;
            };
            if let Some(temp) = read(&format!("{sensor}_input")).and_then(|v| v.parse::<f64>().ok())
            {
                temps.push((id, temp / 1000.0));
            }
        }
    }
    temps.sort_by_key(|(id, _)| *id);
    temps
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monta uma árvore falsa de /sys ou /proc: [(diretório, [(arquivo, conteúdo)])]
    fn sysfs_fixture(name: &str, devices: &[(&str, &[(&str, &str)])]) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("nebula-sysfs-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&root);
        for (dev, files) in devices {
            let dir = root.join(dev);
            fs::create_dir_all(&dir).unwrap();
            for (file, content) in *files {
                fs::write(dir.join(file), format!("{content}\n")).unwrap();
            }
        }
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// Tamanho do painel de CPU na tela demo de 160x40
    fn render_panel(panel: &CpuPanel) -> String {
        let area = Rect::new(0, 0, 83, 21);
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
    fn cpu_sem_sensor_nao_mostra_temperatura() {
        let mut panel = CpuPanel::demo();
        panel.cpu_temp_c = None;
        panel.ccd_temps.clear();
        panel.ccd_for_core.clear();
        for core in &mut panel.cpu_cores {
            core.temp_c = None;
        }
        let text = render_panel(&panel);
        assert!(
            !text.contains("T: "),
            "título da CPU não deve ter temperatura"
        );
        let core0 = text.lines().find(|l| l.contains("Core 0")).unwrap();
        assert!(!core0.contains("°C") && core0.contains('—'), "{core0}");
    }

    #[test]
    fn cpu_amd_mostra_temperatura_compartilhada_so_na_coluna() {
        let text = render_panel(&CpuPanel::demo());
        let core0 = text.lines().find(|l| l.contains("Core 0")).unwrap();
        assert!(core0.contains("42°C*") && !core0.contains('—'), "{core0}");
        assert!(!text.contains("CCD 1:") && !text.contains("compartilhada"));
    }

    #[test]
    fn cpu_sem_rapl_omite_consumo() {
        let mut panel = CpuPanel::demo();
        panel.cpu_power_w = None;
        let cpu_title = render_panel(&panel).lines().next().unwrap().to_string();
        assert!(cpu_title.contains("GHz"), "{cpu_title}");
        assert!(
            !cpu_title.contains("125.0 W") && !cpu_title.contains(" W "),
            "{cpu_title}"
        );
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "Windows não aceita ':' em nomes de pasta; roda no CI (Linux)"
    )]
    fn rapl_le_so_dominios_de_pacote() {
        let root = sysfs_fixture(
            "rapl",
            &[
                (
                    "intel-rapl:1",
                    &[("energy_uj", "500"), ("max_energy_range_uj", "1000")],
                ),
                (
                    "intel-rapl:0",
                    &[("energy_uj", "100"), ("max_energy_range_uj", "1000")],
                ),
                (
                    "intel-rapl:0:0",
                    &[("energy_uj", "999"), ("max_energy_range_uj", "1000")],
                ),
                (
                    "intel-rapl-mmio:0",
                    &[("energy_uj", "999"), ("max_energy_range_uj", "1000")],
                ),
            ],
        );
        assert_eq!(
            read_rapl(&root),
            Some(vec![
                RaplDomain {
                    energy_uj: 100,
                    max_range_uj: 1000
                },
                RaplDomain {
                    energy_uj: 500,
                    max_range_uj: 1000
                },
            ])
        );
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "Windows não aceita ':' em nomes de pasta; roda no CI (Linux)"
    )]
    fn rapl_sem_permissao_retorna_none() {
        // sem energy_uj legível (o que acontece sem root desde o kernel 5.10)
        let root = sysfs_fixture(
            "rapl-sem-permissao",
            &[("intel-rapl:0", &[("max_energy_range_uj", "1000")])],
        );
        assert_eq!(read_rapl(&root), None);
    }

    #[test]
    fn rapl_ausente_retorna_none() {
        assert_eq!(read_rapl(Path::new("/caminho/que/nao/existe")), None);
        let vazio = sysfs_fixture("rapl-vazio", &[("outra-coisa", &[("energy_uj", "1")])]);
        assert_eq!(read_rapl(&vazio), None);
    }

    #[test]
    fn potencia_rapl_soma_pacotes_e_trata_volta_do_contador() {
        let d = |energy_uj| RaplDomain {
            energy_uj,
            max_range_uj: 1_000_000_000,
        };
        // 10 J em 2 s = 5 W
        assert_eq!(rapl_power_watts(&[d(0)], &[d(10_000_000)], 2.0), Some(5.0));
        // dois pacotes: 3 J + 1 J em 1 s
        assert_eq!(
            rapl_power_watts(&[d(0), d(0)], &[d(3_000_000), d(1_000_000)], 1.0),
            Some(4.0)
        );
        // contador voltou a zero: 10 J até o máximo + 9 J depois
        assert_eq!(
            rapl_power_watts(&[d(990_000_000)], &[d(9_000_000)], 1.0),
            Some(19.0)
        );
        assert_eq!(rapl_power_watts(&[d(0)], &[d(1), d(1)], 1.0), None);
        assert_eq!(rapl_power_watts(&[d(0)], &[d(1)], 0.0), None);
    }

    #[test]
    fn topologia_mapeia_cpu_logica_para_nucleo_fisico() {
        let root = sysfs_fixture(
            "topologia",
            &[
                (
                    "cpu0/topology",
                    &[
                        ("physical_package_id", "0"),
                        ("core_id", "0"),
                        ("die_id", "0"),
                    ],
                ),
                (
                    "cpu1/topology",
                    &[
                        ("physical_package_id", "0"),
                        ("core_id", "0"),
                        ("die_id", "0"),
                    ],
                ), // hyperthread do 0
                (
                    "cpu2/topology",
                    &[
                        ("physical_package_id", "0"),
                        ("core_id", "4"),
                        ("die_id", "1"),
                    ],
                ),
            ],
        );
        assert_eq!(
            read_cpu_topology(&root, 4),
            vec![Some((0, 0)), Some((0, 0)), Some((0, 4)), None]
        );
        assert_eq!(
            read_cpu_die_ids(&root, 4),
            vec![Some(0), Some(0), Some(1), None]
        );
        assert_eq!(
            match_ccd_to_cpus(&read_cpu_die_ids(&root, 4), &[(1, 35.0), (2, 38.0)], true),
            HashMap::from([(0, 1), (1, 1), (2, 2)])
        );
        assert!(match_ccd_to_cpus(&[Some(0)], &[(1, 35.0)], false).is_empty());
        assert!(match_ccd_to_cpus(&[Some(2)], &[(1, 35.0)], true).is_empty());
    }

    #[test]
    fn coretemp_le_temperatura_por_nucleo_e_ignora_outros_sensores() {
        let root = sysfs_fixture(
            "hwmon",
            &[
                (
                    "hwmon0",
                    &[
                        ("name", "acpitz"),
                        ("temp1_label", "Core 9"),
                        ("temp1_input", "99000"),
                    ],
                ),
                (
                    "hwmon3",
                    &[
                        ("name", "coretemp"),
                        ("temp1_label", "Package id 1"),
                        ("temp1_input", "55000"),
                        ("temp2_label", "Core 0"),
                        ("temp2_input", "48000"),
                        ("temp6_label", "Core 4"),
                        ("temp6_input", "51500"),
                        ("temp7_label", "Core 8"), // sem _input: ignorado
                    ],
                ),
                (
                    "hwmon5",
                    &[
                        ("name", "k10temp"),
                        ("temp1_label", "Tctl"),
                        ("temp1_input", "61000"),
                    ],
                ),
            ],
        );
        let temps = read_coretemp(&root);
        assert_eq!(temps.len(), 2);
        assert_eq!(temps.get(&(1, 0)), Some(&48.0));
        assert_eq!(temps.get(&(1, 4)), Some(&51.5));
        assert!(read_coretemp(Path::new("/caminho/que/nao/existe")).is_empty());
    }

    #[test]
    fn k10temp_le_ccds_em_ordem_e_ignora_sensores_sem_leitura() {
        let root = sysfs_fixture(
            "k10temp",
            &[
                (
                    "hwmon0",
                    &[
                        ("name", "coretemp"),
                        ("temp1_label", "Tccd9"),
                        ("temp1_input", "99000"),
                    ],
                ),
                (
                    "hwmon3",
                    &[
                        ("name", "k10temp"),
                        ("temp1_label", "Tctl"),
                        ("temp1_input", "42000"),
                        ("temp3_label", "Tccd2"),
                        ("temp3_input", "34500"),
                        ("temp4_label", "Tccd1"),
                        ("temp4_input", "35750"),
                        ("temp5_label", "Tccd3"),
                        ("temp6_label", "Tccd4"),
                        ("temp6_input", "inválido"),
                    ],
                ),
            ],
        );
        assert_eq!(read_ccd_temps(&root), vec![(1, 35.75), (2, 34.5)]);
        assert!(read_ccd_temps(Path::new("/caminho/que/nao/existe")).is_empty());
    }
}
