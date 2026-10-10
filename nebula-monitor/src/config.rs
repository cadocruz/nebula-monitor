//! Arquivo de configuração (`config.toml`): intervalo de atualização, tema e layout das vagas.
//!
//! ```toml
//! refresh_ms = 1000         # intervalo entre coletas, em ms (200 a 60000)
//!
//! [theme]                   # opcional; cada chave troca uma cor da paleta neon
//! background = "#100b05"
//! cyan = "#ffb000"
//!
//! [layout]
//! split = "rows"            # "rows" (empilha) ou "cols" (lado a lado)
//! sizes = [55, 45]          # porcentagens, somam 100
//! children = [
//!   { split = "cols", sizes = [52, 48], children = [
//!       { panels = ["cpu"] },
//!       { panels = ["gpu", "memory"] },   # sem NVIDIA, a Memória assume a vaga
//!   ]},
//!   { panels = ["processes"] },
//! ]
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ratatui::layout::Direction;
use serde::Deserialize;

use crate::layout::Node;
use crate::theme::{self, Palette};

/// Teclas 1–9 focam as vagas: mais que isso não seria alcançável pelo teclado
pub const MAX_SLOTS: usize = 9;

/// Intervalo entre coletas: o padrão de sempre e os limites aceitos
pub const DEFAULT_REFRESH_MS: u64 = 1000;
/// O %CPU é a diferença entre duas leituras do sysinfo; abaixo de 200 ms ele não mede
pub const MIN_REFRESH_MS: u64 = 200;
pub const MAX_REFRESH_MS: u64 = 60_000;

#[derive(Debug, PartialEq)]
pub struct Config {
    pub layout: Node,
    pub refresh: Duration,
    pub palette: Palette,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            layout: Node::default_layout(),
            refresh: Duration::from_millis(DEFAULT_REFRESH_MS),
            palette: Palette::NEON,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    refresh_ms: Option<u64>,
    theme: Option<BTreeMap<String, String>>,
    layout: Option<NodeSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeSpec {
    split: Option<String>,
    sizes: Option<Vec<u16>>,
    children: Option<Vec<NodeSpec>>,
    panels: Option<Vec<String>>,
}

/// `$XDG_CONFIG_HOME/nebula/config.toml` ou, sem essa variável, `~/.config/nebula/config.toml`
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("nebula").join("config.toml"))
}

/// Carrega a configuração. Sem `--config`, um arquivo ausente no caminho padrão significa
/// "use o padrão"; com `--config`, o arquivo precisa existir.
pub fn load(explicit: Option<&Path>, known_ids: &[&str]) -> Result<Config, String> {
    let path = match explicit {
        Some(p) => p.to_path_buf(),
        None => match default_path() {
            Some(p) if p.exists() => p,
            _ => return Ok(Config::default()),
        },
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("{}: não foi possível ler: {e}", path.display()))?;
    parse(&text, known_ids).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn parse(text: &str, known_ids: &[&str]) -> Result<Config, String> {
    let file: ConfigFile = toml::from_str(text).map_err(|e| e.to_string())?;

    let refresh_ms = file.refresh_ms.unwrap_or(DEFAULT_REFRESH_MS);
    if !(MIN_REFRESH_MS..=MAX_REFRESH_MS).contains(&refresh_ms) {
        return Err(format!(
            "refresh_ms = {refresh_ms}; use um valor entre {MIN_REFRESH_MS} e {MAX_REFRESH_MS} (ms)"
        ));
    }

    let layout = match file.layout {
        None => Node::default_layout(),
        Some(spec) => {
            let node = convert(spec, "layout", known_ids)?;
            let count = node.slots().len();
            if count > MAX_SLOTS {
                return Err(format!(
                    "o layout tem {count} vagas; o máximo é {MAX_SLOTS} (teclas 1–9)"
                ));
            }
            node
        }
    };
    let palette = match file.theme {
        None => Palette::NEON,
        Some(colors) => convert_theme(&colors)?,
    };
    Ok(Config {
        layout,
        refresh: Duration::from_millis(refresh_ms),
        palette,
    })
}

/// `[theme]`: cada chave troca uma cor da paleta neon; as ausentes ficam como estão
fn convert_theme(colors: &BTreeMap<String, String>) -> Result<Palette, String> {
    let mut palette = Palette::NEON;
    for (key, value) in colors {
        let mut entries = palette.entries_mut();
        let Some((_, slot)) = entries.iter_mut().find(|(name, _)| name == key) else {
            let mut neon = Palette::NEON;
            let names = neon.entries_mut().map(|(name, _)| name);
            return Err(format!(
                "theme.{key}: cor desconhecida; disponíveis: {}",
                names.join(", ")
            ));
        };
        **slot = theme::parse_hex(value).ok_or_else(|| {
            format!("theme.{key} = \"{value}\": use o formato \"#rrggbb\" (ex.: \"#00e5ff\")")
        })?;
    }
    Ok(palette)
}

/// Só o layout (usado pelos testes de layout)
#[cfg(test)]
pub fn parse_layout(text: &str, known_ids: &[&str]) -> Result<Node, String> {
    parse(text, known_ids).map(|c| c.layout)
}

fn convert(spec: NodeSpec, path: &str, known_ids: &[&str]) -> Result<Node, String> {
    if let Some(panels) = spec.panels {
        if spec.split.is_some() || spec.sizes.is_some() || spec.children.is_some() {
            return Err(format!(
                "{path}: uma vaga tem `panels` OU `split`/`sizes`/`children`, não os dois"
            ));
        }
        if panels.is_empty() {
            return Err(format!("{path}: `panels` precisa de pelo menos um painel"));
        }
        if let Some(unknown) = panels.iter().find(|id| !known_ids.contains(&id.as_str())) {
            return Err(format!(
                "{path}: painel desconhecido \"{unknown}\"; disponíveis: {}",
                known_ids.join(", ")
            ));
        }
        return Ok(Node::Slot { panels });
    }

    let direction = match spec.split.as_deref() {
        Some("rows") => Direction::Vertical,
        Some("cols") => Direction::Horizontal,
        Some(other) => {
            return Err(format!(
                "{path}: split = \"{other}\" inválido; use \"rows\" ou \"cols\""
            ));
        }
        None => {
            return Err(format!(
                "{path}: defina `panels` (uma vaga) ou `split` (uma divisão)"
            ));
        }
    };
    let (Some(sizes), Some(children)) = (spec.sizes, spec.children) else {
        return Err(format!(
            "{path}: uma divisão precisa de `sizes` e `children`"
        ));
    };
    if children.is_empty() || sizes.len() != children.len() {
        return Err(format!(
            "{path}: `sizes` tem {} itens e `children` tem {}; precisam ser iguais e não vazios",
            sizes.len(),
            children.len()
        ));
    }
    let total: u32 = sizes.iter().map(|&s| u32::from(s)).sum();
    if total != 100 {
        return Err(format!("{path}: `sizes` soma {total}; precisa somar 100"));
    }
    let children = children
        .into_iter()
        .enumerate()
        .map(|(i, child)| convert(child, &format!("{path}.children[{i}]"), known_ids))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Node::Split {
        direction,
        sizes,
        children,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDS: &[&str] = &["cpu", "memory", "gpu", "disks", "processes"];

    #[test]
    fn arquivo_sem_layout_usa_o_padrao() {
        assert_eq!(parse_layout("", IDS), Ok(Node::default_layout()));
    }

    #[test]
    fn layout_escrito_igual_ao_padrao_gera_a_mesma_arvore() {
        let text = r#"
            [layout]
            split = "rows"
            sizes = [55, 45]
            children = [
              { split = "cols", sizes = [52, 48], children = [
                  { panels = ["cpu"] },
                  { split = "rows", sizes = [50, 50], children = [
                      { panels = ["memory"] },
                      { panels = ["gpu"] },
                  ]},
              ]},
              { split = "cols", sizes = [60, 40], children = [
                  { panels = ["disks"] },
                  { panels = ["processes"] },
              ]},
            ]
        "#;
        assert_eq!(parse_layout(text, IDS), Ok(Node::default_layout()));
    }

    #[test]
    fn vaga_com_alternativas() {
        let text = r#"
            [layout]
            split = "cols"
            sizes = [50, 50]
            children = [{ panels = ["gpu", "memory"] }, { panels = ["processes"] }]
        "#;
        let node = parse_layout(text, IDS).unwrap();
        assert_eq!(node.slots()[0], ["gpu".to_string(), "memory".to_string()]);
    }

    fn erro(text: &str) -> String {
        parse_layout(text, IDS).unwrap_err()
    }

    #[test]
    fn erros_apontam_o_caminho_e_o_motivo() {
        let e = erro(
            r#"
            [layout]
            split = "cols"
            sizes = [50, 50]
            children = [{ panels = ["cpu"] }, { panels = ["rede"] }]
        "#,
        );
        assert!(
            e.contains("layout.children[1]") && e.contains("\"rede\""),
            "{e}"
        );
        assert!(e.contains("disponíveis: cpu, memory"), "{e}");

        let e = erro(
            "[layout]\nsplit = \"cols\"\nsizes = [50, 40]\nchildren = [{ panels = [\"cpu\"] }, { panels = [\"gpu\"] }]",
        );
        assert!(e.contains("soma 90"), "{e}");

        let e = erro(
            "[layout]\nsplit = \"cols\"\nsizes = [100]\nchildren = [{ panels = [\"cpu\"] }, { panels = [\"gpu\"] }]",
        );
        assert!(e.contains("1 itens") && e.contains("2"), "{e}");

        let e = erro(
            "[layout]\nsplit = \"diagonal\"\nsizes = [100]\nchildren = [{ panels = [\"cpu\"] }]",
        );
        assert!(e.contains("\"diagonal\""), "{e}");

        let e = erro("[layout]\npanels = []");
        assert!(e.contains("pelo menos um"), "{e}");

        let e = erro("[layout]\npanels = [\"cpu\"]\nsplit = \"rows\"");
        assert!(e.contains("não os dois"), "{e}");
    }

    #[test]
    fn campo_desconhecido_e_toml_invalido_sao_rejeitados() {
        assert!(erro("[layut]\npanels = [\"cpu\"]").contains("layut"));
        assert!(erro("[layout\n").contains("line") || erro("[layout\n").contains("linha"));
    }

    #[test]
    fn configuracao_de_exemplo_do_repositorio_e_valida() {
        let registry = crate::registry::builtin();
        let ids = crate::registry::ids(&registry);
        let text = include_str!("../../examples/configs/network-fallback.toml");
        let node = parse_layout(text, &ids).expect("exemplo inválido");
        assert!(
            node.slots()
                .iter()
                .any(|s| s.iter().any(|id| id == "network"))
        );
    }

    #[test]
    fn no_maximo_nove_vagas() {
        let children = ["{ panels = [\"cpu\"] }"; 10].join(", ");
        let text = format!(
            "[layout]\nsplit = \"cols\"\nsizes = [10,10,10,10,10,10,10,10,10,10]\nchildren = [{children}]"
        );
        assert!(erro(&text).contains("máximo é 9"));
    }

    #[test]
    fn intervalo_padrao_e_configurado() {
        assert_eq!(parse("", IDS), Ok(Config::default()));
        assert_eq!(
            parse("refresh_ms = 500", IDS).unwrap().refresh,
            Duration::from_millis(500)
        );
        // intervalo e layout juntos
        let cfg = parse("refresh_ms = 2000\n[layout]\npanels = [\"cpu\"]", IDS).unwrap();
        assert_eq!(cfg.refresh, Duration::from_millis(2000));
        assert_eq!(cfg.layout.slots().len(), 1);
    }

    #[test]
    fn tema_troca_so_as_cores_informadas() {
        let cfg = parse("[theme]\ncyan = \"#ffb000\"\nbackground = \"#100B05\"", IDS).unwrap();
        assert_eq!(cfg.palette.cyan, ratatui::style::Color::Rgb(255, 176, 0));
        assert_eq!(
            cfg.palette.background,
            ratatui::style::Color::Rgb(16, 11, 5)
        );
        assert_eq!(cfg.palette.magenta, Palette::NEON.magenta);
        assert_eq!(cfg.layout, Node::default_layout());
        assert_eq!(parse("", IDS).unwrap().palette, Palette::NEON);
    }

    #[test]
    fn tema_com_chave_ou_cor_invalida_e_rejeitado() {
        let e = parse("[theme]\nciano = \"#ffb000\"", IDS).unwrap_err();
        assert!(
            e.contains("theme.ciano") && e.contains("background, text"),
            "{e}"
        );
        let e = parse("[theme]\ncyan = \"laranja\"", IDS).unwrap_err();
        assert!(e.contains("theme.cyan") && e.contains("#rrggbb"), "{e}");
        assert!(parse("[theme]\ncyan = 255", IDS).is_err());
    }

    #[test]
    fn tema_de_exemplo_do_repositorio_e_valido() {
        let text = include_str!("../../examples/configs/amber-theme.toml");
        let cfg = parse(text, IDS).expect("exemplo inválido");
        assert_ne!(cfg.palette, Palette::NEON);
    }

    #[test]
    fn intervalo_fora_dos_limites_ou_do_tipo_errado_e_rejeitado() {
        assert!(
            parse("refresh_ms = 100", IDS)
                .unwrap_err()
                .contains("entre 200 e 60000")
        );
        assert!(parse("refresh_ms = 600000", IDS).is_err());
        assert!(parse("refresh_ms = \"rápido\"", IDS).is_err());
        assert!(parse("refresh_ms = -1", IDS).is_err());
    }
}
