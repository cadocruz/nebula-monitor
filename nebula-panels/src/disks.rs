//! Painel de Discos: um disco físico por linha (montagem, uso, espaço, I/O, IOPS, latência)
//! e, abaixo, o modelo e a temperatura, com a atividade recente em Braille.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::time::Instant;

use nebula_core::ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Widget},
};
use nebula_core::sysinfo::Disks;
use nebula_core::theme::*;
use nebula_core::widgets::*;
use nebula_core::{Context, Panel, Size, View};

#[derive(Debug, Clone)]
pub struct DiskInfo {
    pub device: String,
    pub mount_point: String,
    pub usage_pct: f64,
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub read_mbs: f64,
    pub write_mbs: f64,
    pub iops: u64,
    pub lat_ms: Option<f64>,
    pub read_history: VecDeque<f64>,
    pub write_history: VecDeque<f64>,
    /// Latência normalizada em escala log (ver latency_level); 0 = sem I/O
    pub lat_history: VecDeque<f64>,
    pub model: String,
    pub temp_c: Option<f64>,
    pub is_mounted: bool,
}

/// Contadores cumulativos de uma linha de /proc/diskstats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DiskCounters {
    reads: u64,
    sectors_read: u64,
    read_ms: u64,
    writes: u64,
    sectors_written: u64,
    write_ms: u64,
}

struct DiskStatPrev {
    counters: DiskCounters,
    timestamp: Instant,
}

pub struct DisksPanel {
    disks: Vec<DiskInfo>,
    /// Volumes montados (espaço livre/total), do sysinfo
    volumes: Disks,
    /// Última leitura de /proc/diskstats por dispositivo, para calcular taxas
    prev_stats: HashMap<String, DiskStatPrev>,
}

impl Default for DisksPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl DisksPanel {
    pub fn new() -> Self {
        Self {
            disks: Vec::new(),
            volumes: Disks::new_with_refreshed_list(),
            prev_stats: HashMap::new(),
        }
    }

    /// Dados fictícios do modo demonstração
    pub fn demo() -> Self {
        let r1 = VecDeque::from(vec![0.4, 0.6, 0.8, 0.5, 0.7, 0.9, 0.6, 0.4]);
        let w1 = VecDeque::from(vec![0.2, 0.3, 0.5, 0.4, 0.3, 0.2, 0.1, 0.3]);
        let r2 = VecDeque::from(vec![0.1, 0.2, 0.4, 0.3, 0.2, 0.5, 0.3, 0.1]);
        let w2 = VecDeque::from(vec![0.5, 0.7, 0.9, 0.8, 0.6, 0.7, 0.5, 0.4]);

        let disks = vec![
            DiskInfo {
                device: "nvme0n1".to_string(),
                mount_point: "/".to_string(),
                usage_pct: 48.0,
                used_bytes: 960 * 1024 * 1024 * 1024,
                total_bytes: 2000 * 1024 * 1024 * 1024,
                read_mbs: 142.5,
                write_mbs: 38.2,
                iops: 5420,
                lat_ms: Some(0.32),
                read_history: r1,
                write_history: w1,
                lat_history: VecDeque::from(vec![0.3, 0.35, 0.4, 0.32, 0.3, 0.45, 0.38, 0.3]),
                model: "Samsung SSD 990 PRO 2TB".to_string(),
                temp_c: Some(36.0),
                is_mounted: true,
            },
            DiskInfo {
                device: "nvme1n1".to_string(),
                mount_point: "/mnt/data".to_string(),
                usage_pct: 64.0,
                used_bytes: 2560 * 1024 * 1024 * 1024,
                total_bytes: 4000 * 1024 * 1024 * 1024,
                read_mbs: 85.0,
                write_mbs: 195.4,
                iops: 7850,
                lat_ms: Some(0.45),
                read_history: r2,
                write_history: w2,
                lat_history: VecDeque::from(vec![0.4, 0.42, 0.5, 0.47, 0.41, 0.44, 0.4, 0.39]),
                model: "WD_BLACK SN850X 4TB".to_string(),
                temp_c: Some(41.0),
                is_mounted: true,
            },
            DiskInfo {
                device: "sda".to_string(),
                mount_point: "— sem volume montado".to_string(),
                usage_pct: 0.0,
                used_bytes: 0,
                total_bytes: 1000 * 1024 * 1024 * 1024,
                read_mbs: 0.0,
                write_mbs: 0.0,
                iops: 0,
                lat_ms: None,
                read_history: VecDeque::new(),
                write_history: VecDeque::new(),
                lat_history: VecDeque::new(),
                model: "Crucial MX500 1TB".to_string(),
                temp_c: Some(29.0),
                is_mounted: false,
            },
        ];
        Self {
            disks,
            volumes: Disks::new(),
            prev_stats: HashMap::new(),
        }
    }

    fn collect(&mut self) {
        let now = Instant::now();
        let cur_stats = fs::read_to_string("/proc/diskstats")
            .map(|content| parse_diskstats(&content))
            .unwrap_or_default();

        // 1. Ler temperaturas dos dispositivos NVMe em /sys/class/hwmon
        let mut nvme_temps: HashMap<String, f64> = HashMap::new();
        if let Ok(entries) = fs::read_dir("/sys/class/hwmon") {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Ok(real) = fs::canonicalize(&path) {
                    let real_str = real.to_string_lossy();
                    let t_file = path.join("temp1_input");
                    if let Ok(content) = fs::read_to_string(&t_file)
                        && let Ok(val) = content.trim().parse::<f64>()
                    {
                        for i in 0..10 {
                            let key = format!("nvme{}", i);
                            if real_str.contains(&key) {
                                nvme_temps.insert(key, val / 1000.0);
                                break;
                            }
                        }
                    }
                }
            }
        }

        // 2. Descobrir todos os discos físicos em /sys/block
        let mut physical_disks: Vec<(String, String, Option<f64>)> = Vec::new();
        if let Ok(entries) = fs::read_dir("/sys/block") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("loop")
                    || name.starts_with("ram")
                    || name.starts_with("dm-")
                    || name.starts_with("zram")
                    || name.starts_with("md")
                {
                    continue;
                }

                let p = entry.path();
                let model = fs::read_to_string(p.join("device/model"))
                    .or_else(|_| fs::read_to_string(p.join("device/device/model")))
                    .or_else(|_| fs::read_to_string(p.join("device/name")))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();

                let temp = if name.starts_with("nvme") {
                    if let Some(rest) = name.strip_prefix("nvme") {
                        if let Some(idx) = rest.split('n').next() {
                            nvme_temps.get(&format!("nvme{}", idx)).copied()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                physical_disks.push((name, model, temp));
            }
        }

        physical_disks.sort_by(|a, b| a.0.cmp(&b.0));

        let mut updated_disks = Vec::new();

        for (dev_name, model, temp_c) in physical_disks {
            // Verificar quais partições pertencem a este disco físico
            let mut matching_mounts: Vec<(String, u64, u64)> = Vec::new();
            for disk in self.volumes.list() {
                let part_name = disk.name().to_string_lossy();
                let part_base = part_name.trim_start_matches("/dev/");
                if part_base == dev_name
                    || part_base.starts_with(&format!("{}p", dev_name))
                    || (dev_name.starts_with("sd") && part_base.starts_with(&dev_name))
                {
                    let mount = disk.mount_point().to_string_lossy().to_string();
                    if !mount.starts_with("/snap") {
                        matching_mounts.push((mount, disk.total_space(), disk.available_space()));
                    }
                }
            }

            let (mount_point, total, available, is_mounted) = if matching_mounts.is_empty() {
                ("—".to_string(), 0u64, 0u64, false)
            } else {
                // Ordenar para dar preferência a partições de dados (ex: "/" ou "/home" ou "/media") sobre "/boot"
                matching_mounts.sort_by(|a, b| {
                    let a_boot = a.0.starts_with("/boot");
                    let b_boot = b.0.starts_with("/boot");
                    match (a_boot, b_boot) {
                        (true, false) => std::cmp::Ordering::Greater,
                        (false, true) => std::cmp::Ordering::Less,
                        _ => b.1.cmp(&a.1), // maior partição
                    }
                });
                let (m, tot, avail) = matching_mounts[0].clone();
                (m, tot, avail, true)
            };

            let used = total.saturating_sub(available);
            let usage_pct = if total > 0 {
                (used as f64 / total as f64) * 100.0
            } else {
                0.0
            };

            // I/O stats
            let mut read_mbs = 0.0;
            let mut write_mbs = 0.0;
            let mut iops = 0;
            let mut lat_ms = None;

            if let Some(&cur) = cur_stats.get(&dev_name) {
                if let Some(prev) = self.prev_stats.get(&dev_name) {
                    let dt = now.duration_since(prev.timestamp).as_secs_f64().max(0.1);
                    let rates = disk_rates(&prev.counters, &cur, dt);
                    read_mbs = rates.read_mbs;
                    write_mbs = rates.write_mbs;
                    iops = rates.iops;
                    lat_ms = rates.lat_ms;
                }

                self.prev_stats.insert(
                    dev_name.clone(),
                    DiskStatPrev {
                        counters: cur,
                        timestamp: now,
                    },
                );
            }

            let (mut r_hist, mut w_hist, mut l_hist) =
                if let Some(old) = self.disks.iter().find(|d| d.device == dev_name) {
                    (
                        old.read_history.clone(),
                        old.write_history.clone(),
                        old.lat_history.clone(),
                    )
                } else {
                    (
                        VecDeque::from(vec![0.0; 24]),
                        VecDeque::from(vec![0.0; 24]),
                        VecDeque::from(vec![0.0; 24]),
                    )
                };

            for hist in [&mut r_hist, &mut w_hist, &mut l_hist] {
                if hist.len() >= 24 {
                    hist.pop_front();
                }
            }

            // Escala de 80 MB/s; piso de 0.1 deixa qualquer atividade real visível
            let r_val = if read_mbs > 0.0 {
                (read_mbs / 80.0).clamp(0.1, 1.0)
            } else {
                0.0
            };
            let w_val = if write_mbs > 0.0 {
                (write_mbs / 80.0).clamp(0.1, 1.0)
            } else {
                0.0
            };
            r_hist.push_back(r_val);
            w_hist.push_back(w_val);
            l_hist.push_back(lat_ms.map(latency_level).unwrap_or(0.0));

            updated_disks.push(DiskInfo {
                device: dev_name,
                mount_point,
                usage_pct,
                used_bytes: used,
                total_bytes: total,
                read_mbs,
                write_mbs,
                iops,
                lat_ms,
                read_history: r_hist,
                write_history: w_hist,
                lat_history: l_hist,
                model,
                temp_c,
                is_mounted,
            });
        }

        self.disks = updated_disks;
    }
}

impl Panel for DisksPanel {
    fn id(&self) -> &'static str {
        "disks"
    }

    fn update(&mut self, _ctx: &Context) {
        self.volumes.refresh(true);
        self.collect();
    }

    fn render(&self, area: Rect, buf: &mut Buffer, view: &View) {
        render_disks(&self.disks, view.unicode_icons, buf, area);
    }

    fn min_size(&self) -> Size {
        Size {
            width: 60,
            height: 6,
        }
    }
}

/// Renderiza linha de atividade de disco em Braille multi-colorida (Leitura em Ciano, Gravação em Magenta, Latência em Amarelo)
fn render_disk_activity_braille(disk: &DiskInfo, total_width: usize) -> Line<'static> {
    let w = total_width.max(12);
    let w_read = (w * 10) / 24;
    let w_write = (w * 8) / 24;
    let w_lat = w - w_read - w_write;

    let is_boot = disk.mount_point == "/boot/efi" || disk.mount_point == "/boot";
    let is_idle = !disk.is_mounted || ((disk.read_mbs == 0.0 && disk.write_mbs == 0.0) && is_boot);

    let (cyan_chars, mag_chars, yel_chars) = if is_idle {
        // Linha de base pontilhada: disco sem volume montado ou partição de boot parada
        ("⠤".repeat(w_read), "⠤".repeat(w_write), "⠤".repeat(w_lat))
    } else {
        (
            braille_peaks_line(&disk.read_history, w_read),
            braille_peaks_line(&disk.write_history, w_write),
            braille_peaks_line(&disk.lat_history, w_lat),
        )
    };

    Line::from(vec![
        Span::styled(cyan_chars, Style::default().fg(CYAN_NEON)),
        Span::styled(mag_chars, Style::default().fg(MAGENTA_NEON)),
        Span::styled(yel_chars, Style::default().fg(YELLOW_NEON)),
    ])
}

fn render_disks(disks: &[DiskInfo], unicode_icons: bool, buf: &mut Buffer, area: Rect) {
    let d_icon = format!(" {} DISCOS E ARMAZENAMENTO ", icon_disk(unicode_icons));
    let title = Line::from(vec![Span::styled(
        d_icon,
        Style::default().fg(CYAN_NEON).add_modifier(Modifier::BOLD),
    )]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER_COLOR))
        .title(title);
    let inner = block.inner(area);
    block.render(area, buf);

    // Legenda no canto superior direito do painel
    if area.width >= 60 {
        let legend = Line::from(vec![
            Span::styled("■ ", Style::default().fg(CYAN_NEON)),
            Span::styled("Leitura   ", Style::default().fg(TEXT_WHITE)),
            Span::styled("■ ", Style::default().fg(MAGENTA_NEON)),
            Span::styled("Gravação   ", Style::default().fg(TEXT_WHITE)),
            Span::styled("■ ", Style::default().fg(YELLOW_NEON)),
            Span::styled("Latência (ms) ", Style::default().fg(TEXT_WHITE)),
        ]);
        let leg_len = 44;
        buf.set_line(
            area.right().saturating_sub(leg_len + 2),
            area.top(),
            &legend,
            leg_len,
        );
    }

    if inner.height < 2 || inner.width < 30 {
        return;
    }

    // Posições das colunas compactas e elegantes
    let x_dev = inner.left() + 1;
    let x_mount = x_dev + 15;
    let x_uso = x_mount + 17;
    let x_space = x_uso + 14;
    let x_rw = x_space + 17;
    let x_iops = x_rw + 12;
    let x_lat = x_iops + 6;
    let x_act = x_lat + 8;

    // Cabeçalho da tabela
    let y_head = inner.top();
    buf.set_string(x_dev, y_head, "DISPOSITIVO", Style::default().fg(TEXT_DIM));
    buf.set_string(x_mount, y_head, "MONTAGEM", Style::default().fg(TEXT_DIM));
    buf.set_string(x_uso, y_head, "USO", Style::default().fg(TEXT_DIM));
    buf.set_string(
        x_space,
        y_head,
        "ESPAÇO LIVRE",
        Style::default().fg(TEXT_DIM),
    );

    if x_rw + 10 < inner.right() {
        buf.set_string(x_rw, y_head, "R/W (MB/s)", Style::default().fg(TEXT_DIM));
    }
    if x_iops + 4 < inner.right() {
        buf.set_string(x_iops, y_head, "IOPS", Style::default().fg(TEXT_DIM));
    }
    if x_lat + 6 < inner.right() {
        buf.set_string(x_lat, y_head, "LAT ms", Style::default().fg(TEXT_DIM));
    }
    if x_act + 9 < inner.right() {
        let act_head = if x_act + 15 < inner.right() {
            "ATIVIDADE (60s)"
        } else {
            "ATIVIDADE"
        };
        buf.set_string(x_act, y_head, act_head, Style::default().fg(TEXT_DIM));
    }

    let mut y = inner.top() + 1;

    let max_model_len = disks
        .iter()
        .map(|d| {
            if d.model.is_empty() {
                "Dispositivo de Armazenamento".chars().count()
            } else {
                d.model.chars().count()
            }
        })
        .max()
        .unwrap_or(30)
        .max(37);
    let avail_w = (inner.width.saturating_sub(12) as usize).max(20);
    let model_col_w = max_model_len.min(avail_w);

    for d in disks {
        if y >= inner.bottom() {
            break;
        }

        // --- LINHA 1: DISPOSITIVO, MONTAGEM, USO, LIVRE/TOTAL, R/W, IOPS, LAT, ATIVIDADE ---
        buf.set_string(
            x_dev,
            y,
            format!("/dev/{}", d.device),
            Style::default().fg(TEXT_WHITE),
        );

        if d.is_mounted {
            let m_cut = if d.mount_point.len() > 15 {
                format!("{}...", &d.mount_point[..12])
            } else {
                d.mount_point.clone()
            };
            buf.set_string(x_mount, y, &m_cut, Style::default().fg(TEXT_WHITE));

            // USO: 69%  ████░░
            let filled_b = ((d.usage_pct / 100.0) * 5.0).round() as usize;
            let empty_b = 5usize.saturating_sub(filled_b);
            let u_color = if d.usage_pct < 60.0 {
                GREEN_NEON
            } else if d.usage_pct < 85.0 {
                YELLOW_NEON
            } else {
                RED_ALERT
            };
            let uso_str = format!("{:>3.0}%  ", d.usage_pct);
            buf.set_string(x_uso, y, &uso_str, Style::default().fg(TEXT_WHITE));
            let bar_line = Line::from(vec![
                Span::styled("█".repeat(filled_b), Style::default().fg(u_color)),
                Span::styled("░".repeat(empty_b), Style::default().fg(BAR_EMPTY)),
            ]);
            buf.set_line(x_uso + 6, y, &bar_line, 6);

            // ESPAÇO LIVRE (LIVRE / TOTAL)
            let used_gb = d.used_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            let tot_gb = d.total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            let free_gb = (tot_gb - used_gb).max(0.0);
            let space_str = if tot_gb >= 1024.0 {
                format!("{:.1} TB / {:.1} TB", free_gb / 1024.0, tot_gb / 1024.0)
            } else {
                format!("{:.0} GB / {:.0} GB", free_gb, tot_gb)
            };
            buf.set_string(x_space, y, &space_str, Style::default().fg(TEXT_WHITE));
        } else {
            buf.set_string(x_mount, y, "—", Style::default().fg(TEXT_DIM));
            buf.set_string(
                x_uso,
                y,
                "— sem volume montado",
                Style::default().fg(TEXT_DIM),
            );
        }

        if x_rw + 10 < inner.right() {
            if d.is_mounted {
                let rw_line = Line::from(vec![
                    Span::styled(
                        format!("{:.1} ", d.read_mbs),
                        Style::default().fg(CYAN_NEON),
                    ),
                    Span::styled("/ ", Style::default().fg(TEXT_DIM)),
                    Span::styled(
                        format!("{:.1}", d.write_mbs),
                        Style::default().fg(MAGENTA_NEON),
                    ),
                ]);
                buf.set_line(x_rw, y, &rw_line, 10);
            } else {
                buf.set_string(x_rw, y, "—", Style::default().fg(TEXT_DIM));
            }
        }

        if x_iops + 4 < inner.right() {
            if d.is_mounted {
                let iops_str = if d.iops >= 1000 {
                    format!("{:.1}K", d.iops as f64 / 1000.0)
                } else {
                    d.iops.to_string()
                };
                buf.set_string(x_iops, y, &iops_str, Style::default().fg(TEXT_WHITE));
            } else {
                buf.set_string(x_iops, y, "—", Style::default().fg(TEXT_DIM));
            }
        }

        if x_lat + 5 < inner.right() {
            if let (true, Some(lat)) = (d.is_mounted, d.lat_ms) {
                // NVMe costuma ficar abaixo de 0,1 ms: duas casas evitam mostrar "0.0"
                let lat_str = if lat < 10.0 {
                    format!("{:.2}", lat)
                } else {
                    format!("{:.1}", lat)
                };
                buf.set_string(x_lat, y, lat_str, Style::default().fg(YELLOW_NEON));
            } else {
                buf.set_string(x_lat, y, "—", Style::default().fg(TEXT_DIM));
            }
        }

        if x_act + 8 < inner.right() {
            let act_w = (inner.right().saturating_sub(x_act + 1) as usize).min(24);
            let sp = render_disk_activity_braille(d, act_w);
            buf.set_line(x_act, y, &sp, act_w as u16);
        }

        // --- LINHA 2 (SUB-LINHA): MODELO DE HARDWARE E TEMPERATURA ALINHADA VERTICALMENTE ---
        let y_sub = y + 1;
        if y_sub < inner.bottom() {
            let temp_str = match d.temp_c {
                Some(t) => format!("{:.0}°C", t),
                None => "—".to_string(),
            };
            let t_color = match d.temp_c {
                Some(t) if t < 45.0 => GREEN_NEON,
                Some(t) if t < 60.0 => YELLOW_NEON,
                Some(_) => RED_ALERT,
                None => TEXT_DIM,
            };

            let model_display = if d.model.is_empty() {
                "Dispositivo de Armazenamento".to_string()
            } else {
                d.model.clone()
            };

            let padded_model = if model_display.chars().count() < model_col_w {
                format!("{:<width$}", model_display, width = model_col_w)
            } else if model_display.chars().count() > model_col_w {
                let trimmed: String = model_display
                    .chars()
                    .take(model_col_w.saturating_sub(1))
                    .collect();
                format!("{}…", trimmed)
            } else {
                model_display
            };

            let sub_line = Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(padded_model, Style::default().fg(TEXT_DIM)),
                Span::styled(" · ", Style::default().fg(TEXT_DIM)),
                Span::styled(temp_str, Style::default().fg(t_color)),
            ]);
            buf.set_line(x_dev, y_sub, &sub_line, inner.width.saturating_sub(2));
        }

        y += 2;
    }
}

/// Lê /proc/diskstats. Colunas (a partir de 0): 2 nome, 3 leituras, 5 setores lidos,
/// 6 ms lendo, 7 escritas, 9 setores escritos, 10 ms escrevendo.
fn parse_diskstats(content: &str) -> HashMap<String, DiskCounters> {
    content
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 14 {
                return None;
            }
            let field = |i: usize| parts[i].parse::<u64>().ok();
            let counters = DiskCounters {
                reads: field(3)?,
                sectors_read: field(5)?,
                read_ms: field(6)?,
                writes: field(7)?,
                sectors_written: field(9)?,
                write_ms: field(10)?,
            };
            Some((parts[2].to_string(), counters))
        })
        .collect()
}

#[derive(Debug, PartialEq)]
struct DiskRates {
    read_mbs: f64,
    write_mbs: f64,
    iops: u64,
    /// Latência média por operação (o "await" do iostat); None se não houve I/O
    lat_ms: Option<f64>,
}

fn disk_rates(prev: &DiskCounters, cur: &DiskCounters, dt_secs: f64) -> DiskRates {
    const SECTOR: u64 = 512; // /proc/diskstats sempre conta em setores de 512 bytes
    const MB: f64 = 1024.0 * 1024.0;
    let ops = cur.reads.saturating_sub(prev.reads) + cur.writes.saturating_sub(prev.writes);
    let busy_ms =
        cur.read_ms.saturating_sub(prev.read_ms) + cur.write_ms.saturating_sub(prev.write_ms);
    DiskRates {
        read_mbs: (cur.sectors_read.saturating_sub(prev.sectors_read) * SECTOR) as f64
            / MB
            / dt_secs,
        write_mbs: (cur.sectors_written.saturating_sub(prev.sectors_written) * SECTOR) as f64
            / MB
            / dt_secs,
        iops: (ops as f64 / dt_secs).round() as u64,
        lat_ms: (ops > 0).then(|| busy_ms as f64 / ops as f64),
    }
}

/// Latência em escala logarítmica para o gráfico: 0,01 ms → 0, 0,1 ms → 0,25, 1 ms → 0,5,
/// 10 ms → 0,75, 100 ms+ → 1. Escala linear achataria NVMe (~0,1 ms) contra HDD (~10 ms).
fn latency_level(lat_ms: f64) -> f64 {
    if lat_ms <= 0.0 {
        return 0.0;
    }
    ((lat_ms.log10() + 2.0) / 4.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_panel(panel: &DisksPanel, width: u16, height: u16) -> String {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        panel.render(area, &mut buf, &View::default());
        (0..height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn atividade_de_disco_usa_historico_real_de_latencia() {
        let mut disk = DisksPanel::demo().disks[0].clone();
        disk.read_history = VecDeque::from(vec![0.0; 24]);
        disk.write_history = VecDeque::from(vec![0.0; 24]);
        disk.lat_history = VecDeque::from(vec![0.0; 24]);
        let plano: String = render_disk_activity_braille(&disk, 24)
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(
            plano.chars().all(|c| c == '⠤'),
            "sem I/O tudo é linha de base: {plano}"
        );

        // um pico de latência no fim do histórico aparece no fim do segmento amarelo
        *disk.lat_history.back_mut().unwrap() = 1.0;
        let linha = render_disk_activity_braille(&disk, 24);
        let amarelo = &linha.spans[2].content;
        assert_eq!(amarelo.chars().last(), Some('⣿'), "{amarelo}");
        assert!(
            amarelo.chars().rev().skip(1).all(|c| c == '⠤'),
            "nada de onda decorativa: {amarelo}"
        );
    }

    #[test]
    fn disco_ocioso_nao_desenha_pico_falso() {
        let mut disk = DisksPanel::demo().disks[2].clone(); // sda sem volume montado
        disk.is_mounted = false;
        let linha: String = render_disk_activity_braille(&disk, 24)
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(linha.chars().all(|c| c == '⠤'), "{linha}");
    }

    #[test]
    fn diskstats_le_colunas_certas_e_ignora_linhas_curtas() {
        // formato do kernel 5.5+ (20 colunas) e linha antiga/curta
        let content = "\
 259       0 nvme0n1 150000 2000 9000000 45000 80000 5000 6000000 120000 0 90000 165000 0 0 0 0 1000 500
   8       0 sda 10 0 80 7 0 0 0 0 0 7 7 0 0 0 0
   7       0 loop0 1 2 3
";
        let stats = parse_diskstats(content);
        assert_eq!(
            stats.get("nvme0n1"),
            Some(&DiskCounters {
                reads: 150_000,
                sectors_read: 9_000_000,
                read_ms: 45_000,
                writes: 80_000,
                sectors_written: 6_000_000,
                write_ms: 120_000
            })
        );
        assert_eq!(stats.get("sda").map(|c| c.reads), Some(10));
        assert!(!stats.contains_key("loop0"));
    }

    #[test]
    fn taxas_de_disco_e_latencia_media_por_operacao() {
        let prev = DiskCounters {
            reads: 1000,
            sectors_read: 0,
            read_ms: 100,
            writes: 500,
            sectors_written: 0,
            write_ms: 200,
        };
        // 2 s: 60 leituras + 40 escritas, 4 MiB lidos, 2 MiB escritos, 30 ms + 20 ms de espera
        let cur = DiskCounters {
            reads: 1060,
            sectors_read: 8192,
            read_ms: 130,
            writes: 540,
            sectors_written: 4096,
            write_ms: 220,
        };
        let r = disk_rates(&prev, &cur, 2.0);
        assert_eq!(r.read_mbs, 2.0);
        assert_eq!(r.write_mbs, 1.0);
        assert_eq!(r.iops, 50);
        assert_eq!(r.lat_ms, Some(0.5)); // 50 ms / 100 operações

        let parado = disk_rates(&cur, &cur, 1.0);
        assert_eq!(parado.iops, 0);
        assert_eq!(parado.lat_ms, None, "sem I/O não há latência para mostrar");
    }

    #[test]
    fn latencia_em_escala_logaritmica() {
        let casos = [
            (0.0, 0.0),
            (0.01, 0.0),
            (0.1, 0.25),
            (1.0, 0.5),
            (10.0, 0.75),
            (100.0, 1.0),
            (5000.0, 1.0),
        ];
        for (ms, esperado) in casos {
            assert!(
                (latency_level(ms) - esperado).abs() < 1e-9,
                "{ms} ms -> {} (esperado {esperado})",
                latency_level(ms)
            );
        }
    }

    #[test]
    fn latencia_de_disco_sem_io_mostra_traco() {
        let mut panel = DisksPanel::demo();
        panel.disks[0].lat_ms = None;
        panel.disks[1].lat_ms = Some(0.05);
        let text = render_panel(&panel, 96, 17);
        let linha = |dev: &str| text.lines().find(|l| l.contains(dev)).unwrap().to_string();
        let nvme0 = linha("/dev/nvme0n1");
        assert!(
            !nvme0.contains("0.32") && nvme0.trim_end_matches(['│', ' ']).ends_with('—'),
            "{nvme0}"
        );
        assert!(
            linha("/dev/nvme1n1").contains("0.05"),
            "latência sub-0,1 ms não pode virar 0.0"
        );
    }
}
