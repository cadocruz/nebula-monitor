//! Testes de referência da tela inteira (fase F0 da migração para painéis substituíveis).
//!
//! Cada cenário renderiza o modo demo num terminal virtual e compara com dois arquivos em
//! `tests/golden/`: `<nome>.txt` (os caracteres, legível num diff) e `<nome>.styles` (cores e
//! modificadores, uma linha por linha da tela, em trechos de estilo contínuo).
//!
//! Mudança visual intencional? Regere as referências e revise o diff no git:
//! `UPDATE_GOLDEN=1 cargo test golden`

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
};

use crossterm::event::{KeyCode, KeyEvent};

use crate::model::AppState;
use crate::panel_set::PanelSet;
use crate::render::render_ui;

fn render(state: &AppState, panels: &PanelSet, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| render_ui(f, state, panels)).unwrap();
    terminal.backend().buffer().clone()
}

fn symbols(buf: &Buffer) -> String {
    let area = buf.area;
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn color(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        other => format!("{other:?}"),
    }
}

/// Uma linha por linha da tela: "y: x0-x1 frente/fundo [MODIFICADORES]; ..."
fn styles(buf: &Buffer) -> String {
    let area = buf.area;
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        let style_at = |x: u16| {
            let cell = &buf[(x, y)];
            (cell.fg, cell.bg, cell.modifier)
        };
        let mut runs = Vec::new();
        let mut start = area.left();
        while start < area.right() {
            let current = style_at(start);
            let mut end = start;
            while end + 1 < area.right() && style_at(end + 1) == current {
                end += 1;
            }
            let (fg, bg, modifier) = current;
            let mut run = format!("{start}-{end} {}/{}", color(fg), color(bg));
            if modifier != Modifier::empty() {
                write!(run, " {modifier:?}").unwrap();
            }
            runs.push(run);
            start = end + 1;
        }
        writeln!(out, "{y}: {}", runs.join("; ")).unwrap();
    }
    out
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

fn check(name: &str, state: &AppState, panels: &PanelSet, width: u16, height: u16) {
    let buf = render(state, panels, width, height);
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();

    for (ext, actual) in [("txt", symbols(&buf)), ("styles", styles(&buf))] {
        let path = golden_dir().join(format!("{name}.{ext}"));
        if update {
            fs::create_dir_all(golden_dir()).unwrap();
            fs::write(&path, &actual).unwrap();
            continue;
        }
        let expected = fs::read_to_string(&path)
            .unwrap_or_else(|_| {
                panic!(
                    "referência {} não existe; gere com UPDATE_GOLDEN=1 cargo test golden",
                    path.display()
                )
            })
            .replace("\r\n", "\n");
        if expected != actual {
            panic!(
                "{name}.{ext} difere da referência {}\n\
                 Se a mudança for intencional: UPDATE_GOLDEN=1 cargo test golden (e revise o diff)",
                describe_first_difference(&expected, &actual)
            );
        }
    }
}

/// "na linha L, coluna C:" + trecho de até 30 caracteres antes e depois da primeira diferença
fn describe_first_difference(expected: &str, actual: &str) -> String {
    let Some((i, (exp, got))) = expected
        .lines()
        .zip(actual.lines())
        .enumerate()
        .find(|(_, (e, a))| e != a)
    else {
        return format!(
            "no número de linhas: esperado {}, obtido {}",
            expected.lines().count(),
            actual.lines().count()
        );
    };
    let (exp, got): (Vec<char>, Vec<char>) = (exp.chars().collect(), got.chars().collect());
    let col = exp
        .iter()
        .zip(&got)
        .position(|(e, g)| e != g)
        .unwrap_or(exp.len().min(got.len()));
    let excerpt = |s: &[char]| -> String {
        let from = col.saturating_sub(30);
        let to = (col + 30).min(s.len());
        s.get(from..to)
            .map(|c| c.iter().collect())
            .unwrap_or_default()
    };
    format!(
        "na linha {}, coluna {}:\n  esperado: …{}…\n  obtido:   …{}…",
        i + 1,
        col + 1,
        excerpt(&exp),
        excerpt(&got)
    )
}

/// Modo demo com ícones Nerd Font fixos (o padrão depende de variáveis de ambiente)
fn demo() -> AppState {
    let mut state = AppState::demo();
    state.unicode_fallback_icons = false;
    state
}

#[test]
fn golden_demo_160x40() {
    check("demo_160x40", &demo(), &PanelSet::demo(), 160, 40);
}

#[test]
fn golden_demo_120x34() {
    check("demo_120x34", &demo(), &PanelSet::demo(), 120, 34);
}

#[test]
fn golden_demo_100x30() {
    check("demo_100x30", &demo(), &PanelSet::demo(), 100, 30);
}

#[test]
fn golden_demo_icones_unicode_160x40() {
    let mut state = demo();
    state.unicode_fallback_icons = true;
    check(
        "demo_icones_unicode_160x40",
        &state,
        &PanelSet::demo(),
        160,
        40,
    );
}

#[test]
fn golden_demo_ajuda_160x40() {
    let mut state = demo();
    state.show_help = true;
    check("demo_ajuda_160x40", &state, &PanelSet::demo(), 160, 40);
}

#[test]
fn golden_demo_sem_gpu_nem_bateria_160x40() {
    let mut state = demo();
    state.battery = None;
    let registry = crate::registry::builtin();
    let panels = PanelSet::build(crate::layout::Node::default_layout(), &mut |id| {
        if id == "gpu" {
            Box::new(nebula_panels::GpuPanel::new()) // sem NVML: nenhum dado coletado
        } else {
            (crate::registry::find(&registry, id).unwrap().demo)()
        }
    });
    check("demo_sem_gpu_nem_bateria_160x40", &state, &panels, 160, 40);
}

#[test]
fn golden_demo_pausado_com_selecao_160x40() {
    let mut state = demo();
    state.paused = true;
    let mut panels = PanelSet::demo();
    // Tab: ordena por memória; ↓ três vezes: seleciona a 4ª linha
    for code in [KeyCode::Tab, KeyCode::Down, KeyCode::Down, KeyCode::Down] {
        panels.handle_key(KeyEvent::from(code)); // sem foco: vai primeiro a Processos
    }
    check("demo_pausado_com_selecao_160x40", &state, &panels, 160, 40);
}

#[test]
fn golden_demo_foco_memoria_160x40() {
    let mut panels = PanelSet::demo();
    panels.focus(2);
    check("demo_foco_memoria_160x40", &demo(), &panels, 160, 40);
}

#[test]
fn golden_demo_tema_ambar_160x40() {
    let ids = crate::registry::ids(&crate::registry::builtin());
    let config = crate::config::parse(
        include_str!("../../examples/configs/amber-theme.toml"),
        &ids,
    )
    .expect("tema de exemplo inválido");
    let mut state = demo();
    state.palette = config.palette;
    check("demo_tema_ambar_160x40", &state, &PanelSet::demo(), 160, 40);
}

#[test]
fn golden_demo_processos_filtrados_160x40() {
    let mut panels = PanelSet::demo();
    let keys = std::iter::once(KeyCode::Char('/'))
        .chain("fire".chars().map(KeyCode::Char))
        .chain([KeyCode::Enter]);
    for code in keys {
        panels.handle_key(KeyEvent::from(code));
    }
    check("demo_processos_filtrados_160x40", &demo(), &panels, 160, 40);
}

#[test]
fn golden_demo_confirmar_encerramento_160x40() {
    let mut panels = PanelSet::demo();
    panels.handle_key(KeyEvent::from(KeyCode::Down));
    panels.handle_key(KeyEvent::from(KeyCode::Char('k')));
    check(
        "demo_confirmar_encerramento_160x40",
        &demo(),
        &panels,
        160,
        40,
    );
}

#[test]
fn golden_demo_processos_maximizados_160x40() {
    let mut panels = PanelSet::demo();
    panels.focus(5);
    panels.toggle_maximize();
    check(
        "demo_processos_maximizados_160x40",
        &demo(),
        &panels,
        160,
        40,
    );
}
