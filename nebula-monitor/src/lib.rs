//! nebula-monitor como biblioteca: `run` abre o monitor com os painéis de um registro.
//!
//! O binário `nebula-monitor` chama `run(registry::builtin())`. Um binário pessoal pode
//! acrescentar painéis próprios (crates que implementam `nebula_core::Panel`):
//!
//! ```no_run
//! use nebula_monitor::registry::{self, PanelFactory};
//! # struct MeuPainel;
//! # impl nebula_core::Panel for MeuPainel {
//! #     fn id(&self) -> &'static str { "meu" }
//! #     fn update(&mut self, _: &nebula_core::Context) {}
//! #     fn render(&self, _: nebula_core::ratatui::layout::Rect, _: &mut nebula_core::ratatui::buffer::Buffer, _: &nebula_core::View) {}
//! #     fn min_size(&self) -> nebula_core::Size { nebula_core::Size { width: 1, height: 1 } }
//! # }
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut paineis = registry::builtin();
//!     paineis.push(PanelFactory {
//!         id: "meu",
//!         create: || Box::new(MeuPainel),
//!         demo: || Box::new(MeuPainel),
//!     });
//!     nebula_monitor::run(paineis)
//! }
//! ```
//!
//! Depois é só usar o id na configuração: `panels = ["gpu", "meu"]`.

mod collector;
mod config;
#[cfg(test)]
mod golden;
mod layout;
mod model;
mod panel_set;
pub mod registry;
mod render;
mod theme;

use std::error::Error;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
        MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};

use nebula_core::{Context, Handled};

use crate::collector::Collector;
use crate::model::AppState;
use crate::panel_set::PanelSet;
use crate::registry::PanelFactory;
use crate::render::render_ui;

/// Argumentos da linha de comando
#[derive(Parser, Debug)]
#[command(
    name = "nebula-monitor",
    version,
    about = "Monitor de sistema para terminal (Linux), com painéis configuráveis",
    disable_help_flag = true,
    disable_version_flag = true,
    help_template = "{about}\n\nUso: {usage}\n\nOpções:\n{options}\n"
)]
struct Cli {
    /// Usa dados fictícios em vez de ler o sistema (bom para screenshots)
    #[arg(long, visible_alias = "mock")]
    demo: bool,

    /// Arquivo de configuração (padrão: ~/.config/nebula/config.toml)
    #[arg(long, value_name = "ARQUIVO")]
    config: Option<PathBuf>,

    /// Imprime a tela como texto e sai (padrão: 170 x 48)
    #[arg(long, num_args = 0..=2, value_names = ["COLUNAS", "LINHAS"])]
    dump: Option<Vec<u16>>,

    /// Imprime a tela como JSON, célula a célula, e sai (padrão: 175 x 48)
    #[arg(long, num_args = 0..=2, value_names = ["COLUNAS", "LINHAS"])]
    dump_json: Option<Vec<u16>>,

    /// Desenha a tela uma vez no terminal e sai
    #[arg(long, visible_alias = "snapshot")]
    once: bool,

    /// Mostra esta ajuda
    #[arg(short, long, action = clap::ArgAction::Help)]
    help: Option<bool>,

    /// Mostra a versão
    #[arg(short = 'V', long, action = clap::ArgAction::Version)]
    version: Option<bool>,
}

/// Um ciclo de coleta: cabeçalho e fontes compartilhadas (coletor), depois os painéis
fn tick(collector: &mut Collector, state: &mut AppState, panels: &mut PanelSet) {
    if state.paused {
        return;
    }
    collector.collect(state);
    panels.update(&Context {
        system: collector.system(),
        nvml: collector.nvml(),
    });
}

/// Abre o monitor com os painéis de `registry` (lê os argumentos da linha de comando e a
/// configuração). Configuração inválida ou ids repetidos no registro encerram o processo
/// com uma mensagem e código 2, antes de mexer no terminal.
pub fn run(registry: Vec<PanelFactory>) -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let is_demo = cli.demo;
    let config_path = cli.config;

    // Registro ou configuração inválidos param aqui, antes de mexer no terminal
    if let Some(id) = registry::duplicate_id(&registry) {
        eprintln!("nebula-monitor: dois painéis registrados com o id \"{id}\"");
        std::process::exit(2);
    }
    let config = match config::load(config_path.as_deref(), &registry::ids(&registry)) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("nebula-monitor: configuração inválida: {e}");
            std::process::exit(2);
        }
    };

    // No modo demo nada é coletado: nem NVML nem sysinfo são inicializados
    let mut collector = (!is_demo).then(Collector::new);
    let mut panels = if is_demo {
        PanelSet::demo_with(config.layout, &registry)
    } else {
        PanelSet::new(config.layout, &registry)
    };
    let mut state = match collector.as_mut() {
        Some(c) => {
            let mut s = AppState::default();
            tick(c, &mut s, &mut panels);
            s
        }
        None => AppState::demo(),
    };
    state.palette = config.palette;

    if let Some(size) = &cli.dump {
        let cols = size.first().copied().unwrap_or(170);
        let rows = size.get(1).copied().unwrap_or(48);
        let backend = ratatui::backend::TestBackend::new(cols, rows);
        let mut terminal = Terminal::new(backend)?;
        terminal.draw(|f| render_ui(f, &state, &panels))?;
        let buf = terminal.backend().buffer();
        for y in 0..rows {
            let mut line = String::new();
            for x in 0..cols {
                line.push_str(buf[(x, y)].symbol());
            }
            println!("{}", line);
        }
        return Ok(());
    }

    if let Some(size) = &cli.dump_json {
        let cols = size.first().copied().unwrap_or(175);
        let rows = size.get(1).copied().unwrap_or(48);
        let backend = ratatui::backend::TestBackend::new(cols, rows);
        let mut terminal = Terminal::new(backend)?;
        terminal.draw(|f| render_ui(f, &state, &panels))?;
        let buf = terminal.backend().buffer();
        println!("[");
        for y in 0..rows {
            for x in 0..cols {
                let cell = &buf[(x, y)];
                let sym = cell
                    .symbol()
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "");
                let fg = match cell.fg {
                    ratatui::style::Color::Rgb(r, g, b) => format!("[{},{},{}]", r, g, b),
                    _ => "[200,200,200]".to_string(),
                };
                let bg = match cell.bg {
                    ratatui::style::Color::Rgb(r, g, b) => format!("[{},{},{}]", r, g, b),
                    _ => "[7,11,18]".to_string(),
                };
                println!(
                    "{{\"x\":{},\"y\":{},\"s\":\"{}\",\"fg\":{},\"bg\":{}}},",
                    x, y, sym, fg, bg
                );
            }
        }
        println!("{{\"done\":true}}]");
        return Ok(());
    }

    if cli.once {
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;
        terminal.draw(|f| render_ui(f, &state, &panels))?;
        return Ok(());
    }

    // Restaura o terminal antes de imprimir um panic fatal
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Pânico isolado (painel ou Worker): quem isolou trata e mostra no lugar do painel
        if nebula_core::panicking_isolated() {
            return;
        }
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            crossterm::cursor::Show
        );
        default_hook(info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(
        &mut terminal,
        collector.as_mut(),
        &mut state,
        &mut panels,
        config.refresh,
    );

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("Erro na execução: {:?}", err);
    }

    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mut collector: Option<&mut Collector>,
    state: &mut AppState,
    panels: &mut PanelSet,
    tick_rate: Duration,
) -> Result<(), Box<dyn Error>> {
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| render_ui(f, state, panels))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_millis(0));

        if crossterm::event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if on_key(key, state, panels) == Flow::Quit => return Ok(()),
                Event::Mouse(mouse) => {
                    let size = terminal.size()?;
                    on_mouse(
                        mouse,
                        state,
                        panels,
                        Rect::new(0, 0, size.width, size.height),
                    );
                }
                _ => {}
            }
        }
        if last_tick.elapsed() >= tick_rate {
            if let Some(c) = collector.as_deref_mut() {
                tick(c, state, panels);
            }
            last_tick = Instant::now();
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Flow {
    Continue,
    Quit,
}

/// Teclas globais primeiro; as demais vão aos painéis (o focado, ou Processos, antes)
fn on_key(key: KeyEvent, state: &mut AppState, panels: &mut PanelSet) -> Flow {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Flow::Quit;
    }
    // Painel digitando um filtro ou esperando uma confirmação: recebe tudo, até q e dígitos
    if panels.capturing_slot().is_some() {
        panels.handle_key(key);
        return Flow::Continue;
    }
    match key.code {
        KeyCode::Esc if state.show_help => state.show_help = false,
        KeyCode::Esc if panels.maximized() => {
            panels.restore();
        }
        KeyCode::Esc if panels.focused().is_some() => {
            panels.clear_focus();
        }
        // Um painel pode usar o Esc para desfazer o próprio estado (ex.: limpar o filtro)
        KeyCode::Esc if panels.handle_key(key) == Handled::Yes => {}
        KeyCode::Char('q') | KeyCode::Esc => return Flow::Quit,
        KeyCode::Char('p') => {
            state.paused = !state.paused;
        }
        KeyCode::Char('h') => {
            state.show_help = !state.show_help;
        }
        KeyCode::Char('i') => {
            state.unicode_fallback_icons = !state.unicode_fallback_icons;
        }
        KeyCode::Char('z') => {
            panels.toggle_maximize();
        }
        KeyCode::Char(c @ '1'..='9') => {
            panels.focus(c as usize - '0' as usize);
        }
        _ => {
            panels.handle_key(key);
        }
    }
    Flow::Continue
}

/// Com a ajuda aberta, um clique a fecha; senão o evento vai para a vaga sob o mouse
fn on_mouse(mouse: MouseEvent, state: &mut AppState, panels: &mut PanelSet, screen: Rect) {
    if state.show_help {
        if matches!(mouse.kind, MouseEventKind::Down(_)) {
            state.show_help = false;
        }
        return;
    }
    panels.handle_mouse(mouse, render::body_area(screen));
}

#[cfg(test)]
mod event_tests {
    use super::*;
    use crossterm::event::MouseButton;

    fn tecla(state: &mut AppState, panels: &mut PanelSet, code: KeyCode) -> Flow {
        on_key(KeyEvent::from(code), state, panels)
    }

    #[test]
    fn esc_desfaz_ajuda_maximizado_foco_e_filtro_antes_de_sair() {
        let (mut state, mut panels) = (AppState::demo(), PanelSet::demo());
        tecla(&mut state, &mut panels, KeyCode::Char('/'));
        for c in "fire".chars() {
            tecla(&mut state, &mut panels, KeyCode::Char(c));
        }
        tecla(&mut state, &mut panels, KeyCode::Enter);
        tecla(&mut state, &mut panels, KeyCode::Char('5'));
        tecla(&mut state, &mut panels, KeyCode::Char('z'));
        tecla(&mut state, &mut panels, KeyCode::Char('h'));
        assert!(state.show_help && panels.maximized());

        let esc = |state: &mut AppState, panels: &mut PanelSet| tecla(state, panels, KeyCode::Esc);
        assert_eq!(esc(&mut state, &mut panels), Flow::Continue); // ajuda
        assert!(!state.show_help);
        assert_eq!(esc(&mut state, &mut panels), Flow::Continue); // maximizado
        assert!(!panels.maximized());
        assert_eq!(esc(&mut state, &mut panels), Flow::Continue); // foco
        assert_eq!(panels.focused(), None);
        assert_eq!(esc(&mut state, &mut panels), Flow::Continue); // filtro
        assert_eq!(esc(&mut state, &mut panels), Flow::Quit);
    }

    #[test]
    fn digitando_o_filtro_q_e_digitos_nao_sao_globais() {
        let (mut state, mut panels) = (AppState::demo(), PanelSet::demo());
        tecla(&mut state, &mut panels, KeyCode::Char('/'));
        for code in [KeyCode::Char('q'), KeyCode::Char('3'), KeyCode::Char('p')] {
            assert_eq!(tecla(&mut state, &mut panels, code), Flow::Continue);
        }
        assert_eq!(panels.focused(), None);
        assert!(!state.paused);
        // Ctrl+C sempre encerra
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(on_key(ctrl_c, &mut state, &mut panels), Flow::Quit);
    }

    #[test]
    fn clique_fecha_a_ajuda_sem_chegar_aos_paineis() {
        let (mut state, mut panels) = (AppState::demo(), PanelSet::demo());
        state.show_help = true;
        let clique = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 10,
            modifiers: KeyModifiers::NONE,
        };
        let screen = Rect::new(0, 0, 160, 40);
        on_mouse(clique, &mut state, &mut panels, screen);
        assert!(!state.show_help);
        assert_eq!(panels.focused(), None);
        on_mouse(clique, &mut state, &mut panels, screen);
        assert_eq!(
            panels.focused(),
            Some(0),
            "sem a ajuda, o clique foca a CPU"
        );
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("nebula-monitor").chain(args.iter().copied()))
    }

    #[test]
    fn definicao_dos_argumentos_e_valida() {
        Cli::command().debug_assert();
    }

    #[test]
    fn argumentos_de_sempre_continuam_funcionando() {
        let cli = parse(&["--demo", "--dump", "160", "40"]).unwrap();
        assert!(cli.demo);
        assert_eq!(cli.dump, Some(vec![160, 40]));

        assert!(parse(&["--mock"]).unwrap().demo);
        assert!(parse(&["--snapshot"]).unwrap().once);
        assert_eq!(parse(&["--dump"]).unwrap().dump, Some(vec![]));
        assert_eq!(
            parse(&["--dump-json", "120"]).unwrap().dump_json,
            Some(vec![120])
        );
        assert_eq!(
            parse(&["--config", "/tmp/x.toml"]).unwrap().config,
            Some(PathBuf::from("/tmp/x.toml"))
        );
    }

    #[test]
    fn argumentos_invalidos_sao_rejeitados() {
        assert!(parse(&["--dump", "largo"]).is_err());
        assert!(parse(&["--dump", "1", "2", "3"]).is_err());
        assert!(parse(&["--nao-existe"]).is_err());
        assert!(parse(&["--config"]).is_err());
    }

    #[test]
    fn ajuda_em_portugues() {
        let ajuda = Cli::command().render_help().to_string();
        assert!(
            ajuda.contains("Uso:") && ajuda.contains("Opções:"),
            "{ajuda}"
        );
        assert!(ajuda.contains("dados fictícios"), "{ajuda}");
    }
}
