//! O contrato que todo painel (nativo ou plugin) implementa.

use crossterm::event::{KeyEvent, MouseEvent};
use nvml_wrapper::Nvml;
use ratatui::{buffer::Buffer, layout::Rect};
use sysinfo::System;

/// Dados compartilhados entre painéis, atualizados pelo monitor uma vez por ciclo.
///
/// Só entra aqui o que mais de um painel usa; dados exclusivos de um painel
/// (ex.: /proc/meminfo para a Memória) são lidos pelo próprio painel em `update`.
pub struct Context<'a> {
    /// CPU, memória e processos do sysinfo, já atualizados neste ciclo
    pub system: &'a System,
    /// Biblioteca NVML (GPUs NVIDIA); None sem driver NVIDIA
    pub nvml: Option<&'a Nvml>,
}

/// Como o painel deve se desenhar neste quadro
#[derive(Debug, Clone, Copy, Default)]
pub struct View {
    /// Usar ícones Unicode universais em vez de Nerd Font
    pub unicode_icons: bool,
    /// O painel está focado (recebe as teclas primeiro); o monitor já destaca a moldura
    pub focused: bool,
}

/// Tamanho mínimo útil de um painel, em células
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: u16,
    pub height: u16,
}

/// Se o painel tratou a tecla (senão ela é oferecida a outro)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handled {
    Yes,
    No,
}

/// Atalho exibido no rodapé (`key` + `label`) e na ajuda (`key` + `description`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyHint {
    pub key: &'static str,
    /// Rótulo curto do rodapé
    pub label: &'static str,
    /// Frase da ajuda (cabe em ~44 colunas)
    pub description: &'static str,
}

/// Teclas globais reservadas pelo monitor: q, Ctrl+C, h, i, p, z (maximizar) e os dígitos
/// 1–9 (foco). As demais vão primeiro ao painel focado e depois aos outros. `Esc` é
/// oferecido aos painéis antes de encerrar o monitor (para desfazer um estado, como um
/// filtro). Enquanto `captures_input()` for `true`, todas as teclas (exceto Ctrl+C) vão só
/// para aquele painel.
pub trait Panel {
    /// Identificador usado na configuração do layout ("memory", "cpu", ...)
    fn id(&self) -> &'static str;

    /// `false` quando o painel não tem o que mostrar nesta máquina (ex.: GPU sem NVIDIA)
    fn available(&self) -> bool {
        true
    }

    /// Coleta os dados do painel. Chamado uma vez por ciclo, exceto quando pausado.
    fn update(&mut self, ctx: &Context);

    fn render(&self, area: Rect, buf: &mut Buffer, view: &View);

    fn handle_key(&mut self, _key: KeyEvent) -> Handled {
        Handled::No
    }

    fn keybindings(&self) -> &[KeyHint] {
        &[]
    }

    /// `true` enquanto o painel recebe texto ou espera uma confirmação: as teclas globais
    /// (q, p, dígitos...) deixam de valer e tudo, exceto Ctrl+C, vai para ele
    fn captures_input(&self) -> bool {
        false
    }

    /// Evento de mouse dentro do painel. `area` é a mesma área recebida no último `render`;
    /// as coordenadas do evento são da tela inteira. O clique já foca a vaga antes disto.
    fn handle_mouse(&mut self, _event: MouseEvent, _area: Rect) -> Handled {
        Handled::No
    }

    fn min_size(&self) -> Size;
}
