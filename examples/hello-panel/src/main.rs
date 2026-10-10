//! Exemplo de painel próprio ("plugin") para o nebula-monitor.
//!
//! O painel `HelloPanel` depende só do `nebula-core` (o SDK); o `main` monta um binário
//! pessoal com os painéis nativos + este. Para usá-lo, ponha o id na configuração:
//!
//! ```toml
//! [layout]
//! split = "cols"
//! sizes = [50, 50]
//! children = [{ panels = ["gpu", "hello"] }, { panels = ["processes"] }]
//! ```
//!
//! Num projeto real, o painel costuma ficar num crate próprio (biblioteca) e o binário
//! pessoal só o registra.
//!
//! O painel também mostra o uso de um `Worker`: uma consulta lenta (simulada) roda numa
//! thread própria, e o `update` só pega o resultado pronto, sem travar a tela.

use std::thread;
use std::time::Duration;

use nebula_core::crossterm::event::{KeyCode, KeyEvent};
use nebula_core::ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Widget},
};
use nebula_core::theme::{BORDER_COLOR, CYAN_NEON, MAGENTA_NEON, TEXT_DIM, TEXT_WHITE};
use nebula_core::widgets::{render_panel_titles, set_string_clipped};
use nebula_core::{Context, Handled, KeyHint, Panel, Size, View, Worker};
use nebula_monitor::registry::{self, PanelFactory};

/// Intervalo entre consultas lentas
const SLOW_EVERY: Duration = Duration::from_secs(10);

/// Conta quantas vezes foi atualizado, mostra a CPU média (do sysinfo compartilhado) e o
/// resultado de uma consulta lenta feita por um `Worker`
pub struct HelloPanel {
    updates: u64,
    cpu_avg: Option<f32>,
    /// Consulta lenta numa thread própria; None no modo demo (dados fixos, sem thread)
    slow: Option<Worker<u64>>,
    /// Última resposta da consulta lenta
    answers: Option<u64>,
}

impl HelloPanel {
    pub fn new() -> Self {
        let mut count = 0;
        let slow = Worker::spawn("hello", SLOW_EVERY, move || {
            // Simula algo lento (rede, disco, comando externo). Se isto rodasse no
            // update, a tela ficaria congelada por 2 s a cada consulta.
            thread::sleep(Duration::from_secs(2));
            count += 1;
            count
        });
        Self {
            updates: 0,
            cpu_avg: None,
            slow: Some(slow),
            answers: None,
        }
    }

    /// Dados fictícios do modo demo (`--demo`)
    pub fn demo() -> Self {
        Self {
            updates: 42,
            cpu_avg: Some(37.5),
            slow: None,
            answers: Some(7),
        }
    }

    fn slow_line(&self) -> String {
        if let Some(message) = self.slow.as_ref().and_then(Worker::failure) {
            return format!("Consulta lenta falhou: {message}");
        }
        match self.answers {
            Some(n) => format!("Consulta lenta: {n} respostas"),
            None => "Consulta lenta: aguardando…".to_string(),
        }
    }
}

impl Default for HelloPanel {
    fn default() -> Self {
        Self::new()
    }
}

const KEYS: &[KeyHint] = &[
    KeyHint {
        key: "r",
        label: "Zerar",
        description: "Zerar o contador do painel de exemplo",
    },
    KeyHint {
        key: "u",
        label: "Consultar",
        description: "Fazer a consulta lenta do exemplo agora",
    },
];

impl Panel for HelloPanel {
    fn id(&self) -> &'static str {
        "hello"
    }

    fn update(&mut self, ctx: &Context) {
        // Rode rápido: update é chamado a cada refresh_ms (1 s por padrão), no mesmo
        // laço da tela. O trabalho lento fica no Worker; aqui só se pega o resultado.
        self.updates += 1;
        if let Some(n) = self.slow.as_ref().and_then(Worker::take_latest) {
            self.answers = Some(n);
        }
        let cpus = ctx.system.cpus();
        self.cpu_avg = (!cpus.is_empty())
            .then(|| cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len() as f32);
    }

    fn render(&self, area: Rect, buf: &mut Buffer, _view: &View) {
        // Moldura e títulos no mesmo estilo dos painéis nativos; o foco (moldura amarela)
        // é desenhado pelo monitor
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER_COLOR));
        let inner = block.inner(area);
        block.render(area, buf);
        let title = Line::from(Span::styled(
            " OLÁ, PLUGIN ",
            Style::default()
                .fg(MAGENTA_NEON)
                .add_modifier(Modifier::BOLD),
        ));
        let right = Line::from(Span::styled(
            format!("atualizações: {} ", self.updates),
            Style::default().fg(CYAN_NEON),
        ));
        render_panel_titles(buf, area, &title, Some(&right));

        let cpu = match self.cpu_avg {
            Some(pct) => format!("CPU média: {pct:.1}%"),
            None => "CPU média: N/D".to_string(),
        };
        let slow = self.slow_line();
        let lines = [
            ("Este painel vem de um crate de exemplo.", TEXT_WHITE),
            (cpu.as_str(), CYAN_NEON),
            (slow.as_str(), MAGENTA_NEON),
            ("r zera o contador, u consulta agora", TEXT_DIM),
        ];
        for (row, (text, color)) in lines.into_iter().enumerate() {
            let y = inner.top() + row as u16;
            set_string_clipped(
                buf,
                inner,
                inner.left() + 1,
                y,
                text,
                Style::default().fg(color),
            );
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Handled {
        match key.code {
            KeyCode::Char('r') => self.updates = 0,
            // Acorda o Worker sem esperar o intervalo; a resposta chega num próximo update
            KeyCode::Char('u') => {
                if let Some(slow) = &self.slow {
                    slow.wake();
                }
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn keybindings(&self) -> &[KeyHint] {
        KEYS
    }

    fn min_size(&self) -> Size {
        Size {
            width: 42,
            height: 6,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut paineis = registry::builtin();
    paineis.push(PanelFactory {
        id: "hello",
        create: || Box::new(HelloPanel::new()),
        demo: || Box::new(HelloPanel::demo()),
    });
    nebula_monitor::run(paineis)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texto(panel: &HelloPanel) -> String {
        let area = Rect::new(0, 0, 50, 6);
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
    fn desenha_titulo_contador_e_cpu() {
        let text = texto(&HelloPanel::demo());
        assert!(text.contains("OLÁ, PLUGIN"), "{text}");
        assert!(text.contains("atualizações: 42"), "{text}");
        assert!(text.contains("CPU média: 37.5%"), "{text}");
        assert!(text.contains("Consulta lenta: 7 respostas"), "{text}");
    }

    #[test]
    fn tecla_r_zera_e_outras_passam_adiante() {
        let mut panel = HelloPanel::demo();
        assert_eq!(
            panel.handle_key(KeyEvent::from(KeyCode::Char('r'))),
            Handled::Yes
        );
        assert_eq!(panel.updates, 0);
        assert_eq!(panel.handle_key(KeyEvent::from(KeyCode::Tab)), Handled::No);
        assert_eq!(panel.keybindings()[0].key, "r");
        // no demo não há Worker: u é aceito e não faz nada
        assert_eq!(
            panel.handle_key(KeyEvent::from(KeyCode::Char('u'))),
            Handled::Yes
        );
        assert_eq!(panel.answers, Some(7));
    }

    #[test]
    fn id_nao_colide_com_os_paineis_nativos() {
        let mut paineis = registry::builtin();
        paineis.push(PanelFactory {
            id: HelloPanel::demo().id(),
            create: || Box::new(HelloPanel::new()),
            demo: || Box::new(HelloPanel::demo()),
        });
        assert_eq!(registry::duplicate_id(&paineis), None);
    }
}
