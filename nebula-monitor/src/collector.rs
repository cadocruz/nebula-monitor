use chrono::Local;
use nvml_wrapper::Nvml;
use std::fs;
use std::path::Path;
use std::time::Instant;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::model::{AppState, BatteryInfo, BatteryStatus};

pub struct Collector {
    sys: System,
    nvml: Option<Nvml>,
    distro_cached: String,
    kernel_cached: String,
}

/// Atualiza só o que a tela usa. `refresh_all` também relia environ, cwd e as threads de
/// cada processo, e listava threads como se fossem processos. A cmdline é lida uma vez por
/// processo (não muda depois que ele começa).
fn refresh_system(sys: &mut System) {
    sys.refresh_cpu_all();
    sys.refresh_memory();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .with_disk_usage()
            .with_user(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet)
            .without_tasks(),
    );
}

impl Collector {
    pub fn new() -> Self {
        let mut sys = System::new();
        refresh_system(&mut sys);
        let baseline_at = Instant::now();
        let nvml = Nvml::init().ok();

        let distro = System::long_os_version()
            .or_else(System::name)
            .unwrap_or_else(|| "Linux".to_string());
        let kernel = System::kernel_version().unwrap_or_else(|| "Unknown".to_string());

        // O %CPU (total e por processo) é a diferença entre leituras separadas por pelo menos
        // MINIMUM_CPU_UPDATE_INTERVAL. No Windows, o processo só passa a ter base de
        // comparação na segunda leitura (na primeira coleta aparecia a média desde que o
        // processo nasceu), então fazemos duas leituras de base antes da primeira coleta.
        if let Some(rest) = sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.checked_sub(baseline_at.elapsed())
        {
            std::thread::sleep(rest);
        }
        refresh_system(&mut sys);
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);

        Self {
            sys,
            nvml,
            distro_cached: distro,
            kernel_cached: kernel,
        }
    }

    /// NVML inicializada no início (fonte compartilhada com os painéis); None sem NVIDIA
    pub fn nvml(&self) -> Option<&Nvml> {
        self.nvml.as_ref()
    }

    /// sysinfo já atualizado pela última coleta (fonte compartilhada com os painéis)
    pub fn system(&self) -> &System {
        &self.sys
    }

    pub fn collect(&mut self, state: &mut AppState) {
        if state.paused {
            return;
        }

        refresh_system(&mut self.sys);

        // Header info
        state.hostname = System::host_name().unwrap_or_else(|| "localhost".to_string());
        state.distro = self.distro_cached.clone();
        state.kernel = self.kernel_cached.clone();
        let uptime_secs = System::uptime();
        let days = uptime_secs / 86400;
        let hours = (uptime_secs % 86400) / 3600;
        let mins = (uptime_secs % 3600) / 60;
        state.uptime_str = format!("{}d {}h {}m", days, hours, mins);
        state.datetime_str = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        state.battery = read_battery(Path::new("/sys/class/power_supply"));
    }
}

/// Lê bateria e adaptador de energia de `root` (normalmente /sys/class/power_supply).
/// Devolve None quando não há bateria do sistema (desktop, ou fora do Linux).
fn read_battery(root: &Path) -> Option<BatteryInfo> {
    let read = |dev: &Path, file: &str| {
        fs::read_to_string(dev.join(file))
            .ok()
            .map(|s| s.trim().to_string())
    };
    let read_u64 = |dev: &Path, file: &str| read(dev, file).and_then(|s| s.parse::<u64>().ok());

    let mut devices: Vec<_> = fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    devices.sort();

    let mut battery: Option<(u8, BatteryStatus)> = None;
    let mut ac_online: Option<bool> = None;

    for dev in &devices {
        match read(dev, "type").as_deref() {
            Some("Mains") => {
                if let Some(online) = read(dev, "online") {
                    ac_online = Some(ac_online.unwrap_or(false) || online == "1");
                }
            }
            Some("Battery") => {
                // scope=Device são baterias de periféricos (mouse, fone), não do sistema
                if battery.is_some() || read(dev, "scope").as_deref() == Some("Device") {
                    continue;
                }
                let percent = read_u64(dev, "capacity").or_else(|| {
                    let ratio =
                        |now: &str, full: &str| match (read_u64(dev, now), read_u64(dev, full)) {
                            (Some(n), Some(f)) if f > 0 => Some(n * 100 / f),
                            _ => None,
                        };
                    ratio("energy_now", "energy_full")
                        .or_else(|| ratio("charge_now", "charge_full"))
                });
                let Some(percent) = percent else { continue };
                let status = match read(dev, "status").as_deref() {
                    Some("Charging") => BatteryStatus::Charging,
                    Some("Discharging") => BatteryStatus::Discharging,
                    Some("Full") => BatteryStatus::Full,
                    Some("Not charging") => BatteryStatus::NotCharging,
                    _ => BatteryStatus::Unknown,
                };
                battery = Some((percent.min(100) as u8, status));
            }
            _ => {}
        }
    }

    battery.map(|(percent, status)| BatteryInfo {
        percent,
        status,
        ac_online,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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

    #[test]
    fn bateria_de_notebook_descarregando() {
        let root = sysfs_fixture(
            "notebook",
            &[
                ("AC", &[("type", "Mains"), ("online", "0")]),
                (
                    "BAT0",
                    &[
                        ("type", "Battery"),
                        ("capacity", "57"),
                        ("status", "Discharging"),
                    ],
                ),
            ],
        );
        assert_eq!(
            read_battery(&root),
            Some(BatteryInfo {
                percent: 57,
                status: BatteryStatus::Discharging,
                ac_online: Some(false)
            })
        );
    }

    #[test]
    fn desktop_sem_bateria_retorna_none() {
        let root = sysfs_fixture("desktop", &[("AC", &[("type", "Mains"), ("online", "1")])]);
        assert_eq!(read_battery(&root), None);
        assert_eq!(read_battery(Path::new("/caminho/que/nao/existe")), None);
    }

    #[test]
    fn ignora_bateria_de_periferico() {
        let root = sysfs_fixture(
            "periferico",
            &[(
                "hidpp_battery_0",
                &[
                    ("type", "Battery"),
                    ("scope", "Device"),
                    ("capacity", "12"),
                    ("status", "Discharging"),
                ],
            )],
        );
        assert_eq!(read_battery(&root), None);
    }

    #[test]
    fn calcula_percentual_por_energia_sem_capacity() {
        let root = sysfs_fixture(
            "energia",
            &[(
                "BAT1",
                &[
                    ("type", "Battery"),
                    ("energy_now", "30000000"),
                    ("energy_full", "40000000"),
                    ("status", "Charging"),
                ],
            )],
        );
        assert_eq!(
            read_battery(&root),
            Some(BatteryInfo {
                percent: 75,
                status: BatteryStatus::Charging,
                ac_online: None
            })
        );
    }
}
