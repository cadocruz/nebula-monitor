//! Painéis distribuídos nas vagas do layout, com foco e roteamento de teclas.
//!
//! Cada vaga tem uma lista de candidatos; o painel ativo é o primeiro disponível
//! (`Panel::available`). Se nenhum estiver, a vaga mostra o último da lista — que
//! desenha o próprio aviso (ex.: "Nenhuma GPU NVIDIA detectada").
//!
//! Toda chamada a um painel roda isolada: se ele entrar em pânico, é retirado da vaga
//! (o próximo candidato assume, ou a vaga mostra a caixa de erro) e o resto continua.

use std::cell::RefCell;

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use nebula_core::theme::{BG_DARK, RED_ALERT, TEXT_DIM, TEXT_WHITE};
use nebula_core::{Context, Handled, KeyHint, Panel, View, isolate};
use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Modifier, Style},
    text::Span,
    widgets::{Block, BorderType, Borders, Clear, Widget},
};

use crate::layout::Node;
use crate::registry::{self, PanelFactory};

/// Painel que recebe as teclas logo depois do focado (ou primeiro, sem foco): mantém
/// Tab e ↑↓ funcionando sem precisar focar nada
const PRIMARY_PANEL: &str = "processes";

struct Candidate {
    panel: Box<dyn Panel>,
    /// Mensagem do pânico, se o painel já falhou (nunca mais é chamado)
    failure: RefCell<Option<String>>,
}

impl Candidate {
    fn new(panel: Box<dyn Panel>) -> Self {
        Self {
            panel,
            failure: RefCell::new(None),
        }
    }

    fn failed(&self) -> bool {
        self.failure.borrow().is_some()
    }

    fn record(&self, message: String) {
        *self.failure.borrow_mut() = Some(message);
    }

    /// Saudável e com algo a mostrar
    fn usable(&self) -> bool {
        if self.failed() {
            return false;
        }
        isolate(|| self.panel.available()).unwrap_or_else(|message| {
            self.record(message);
            false
        })
    }
}

struct Slot {
    candidates: Vec<Candidate>,
}

impl Slot {
    /// Primeiro candidato utilizável; se nenhum, o último (mostra o aviso ou a falha dele)
    fn active_index(&self) -> usize {
        self.candidates
            .iter()
            .position(Candidate::usable)
            .unwrap_or(self.candidates.len() - 1)
    }

    fn active(&self) -> &Candidate {
        &self.candidates[self.active_index()]
    }

    fn active_mut(&mut self) -> &mut Candidate {
        let i = self.active_index();
        &mut self.candidates[i]
    }
}

pub struct PanelSet {
    layout: Node,
    slots: Vec<Slot>,
    /// Índice (0-based) da vaga focada
    focused: Option<usize>,
    /// A vaga focada ocupa o corpo da tela inteiro
    maximized: bool,
}

impl PanelSet {
    /// Cria os painéis de cada vaga com `make(id)`. Os ids já foram validados pela
    /// configuração.
    pub fn build(layout: Node, make: &mut dyn FnMut(&str) -> Box<dyn Panel>) -> Self {
        let slots = layout
            .slots()
            .iter()
            .map(|ids| Slot {
                candidates: ids.iter().map(|id| Candidate::new(make(id))).collect(),
            })
            .collect();
        Self {
            layout,
            slots,
            focused: None,
            maximized: false,
        }
    }

    pub fn new(layout: Node, registry: &[PanelFactory]) -> Self {
        Self::build(layout, &mut |id| (lookup(registry, id).create)())
    }

    pub fn demo_with(layout: Node, registry: &[PanelFactory]) -> Self {
        Self::build(layout, &mut |id| (lookup(registry, id).demo)())
    }

    /// Layout padrão com os painéis nativos em modo demo (testes e referências de tela)
    #[cfg(test)]
    pub fn demo() -> Self {
        Self::demo_with(Node::default_layout(), &registry::builtin())
    }

    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Painel ativo da vaga `i` (0-based)
    pub fn slot(&self, i: usize) -> &dyn Panel {
        self.slots[i].active().panel.as_ref()
    }

    /// Retângulo de cada vaga dentro do corpo da tela
    pub fn areas(&self, body: Rect) -> Vec<Rect> {
        self.layout.areas(body)
    }

    /// Atualiza todos os candidatos (não só os ativos): um painel indisponível pode
    /// ficar disponível e voltar a ocupar a vaga
    /// Atualiza todos os candidatos (não só os ativos): um painel indisponível pode
    /// ficar disponível e voltar a ocupar a vaga. Quem entrar em pânico é retirado.
    pub fn update(&mut self, ctx: &Context) {
        for slot in &mut self.slots {
            for candidate in &mut slot.candidates {
                if candidate.failed() {
                    continue;
                }
                if let Err(message) = isolate(|| candidate.panel.update(ctx)) {
                    candidate.record(message);
                }
            }
        }
    }

    /// Desenha o painel ativo da vaga `i`; se ele falhar (agora ou antes), a caixa de erro
    pub fn render_slot(&self, i: usize, area: Rect, buf: &mut Buffer, view: &View) {
        let candidate = self.slots[i].active();
        if !candidate.failed()
            && let Err(message) = isolate(|| candidate.panel.render(area, buf, view))
        {
            candidate.record(message);
        }
        if let Some(message) = candidate.failure.borrow().as_deref() {
            render_failure(buf, area, candidate.panel.id(), message);
        }
    }

    pub fn focused(&self) -> Option<usize> {
        self.focused
    }

    /// Foca a vaga `n` (1 = primeira). Número fora das vagas existentes não muda nada.
    pub fn focus(&mut self, n: usize) -> bool {
        if (1..=self.slots.len()).contains(&n) {
            self.focused = Some(n - 1);
            true
        } else {
            false
        }
    }

    /// Tira o foco (e restaura um painel maximizado); devolve se havia foco
    pub fn clear_focus(&mut self) -> bool {
        self.maximized = false;
        self.focused.take().is_some()
    }

    /// Maximiza ou restaura a vaga focada; sem foco não faz nada
    pub fn toggle_maximize(&mut self) -> bool {
        if self.focused.is_none() {
            return false;
        }
        self.maximized = !self.maximized;
        true
    }

    pub fn maximized(&self) -> bool {
        self.maximized && self.focused.is_some()
    }

    /// Volta ao layout normal; devolve se havia painel maximizado
    pub fn restore(&mut self) -> bool {
        std::mem::take(&mut self.maximized)
    }

    /// Vagas a desenhar e onde: todas pelo layout ou, se maximizada, só a focada no corpo
    pub fn visible_slots(&self, body: Rect) -> Vec<(usize, Rect)> {
        match self.focused {
            Some(i) if self.maximized => vec![(i, body)],
            _ => self.areas(body).into_iter().enumerate().collect(),
        }
    }

    /// Ordem em que teclas e atalhos são considerados: a vaga focada, depois a do painel
    /// principal (Processos) e então as demais na ordem das vagas. Focar um painel sem
    /// atalhos próprios não reordena o rodapé.
    fn key_order(&self) -> Vec<usize> {
        let mut order = Vec::with_capacity(self.slots.len());
        order.extend(self.focused);
        if let Some(primary) = (0..self.slots.len()).find(|&i| self.slot(i).id() == PRIMARY_PANEL)
            && !order.contains(&primary)
        {
            order.push(primary);
        }
        let rest: Vec<usize> = (0..self.slots.len())
            .filter(|i| !order.contains(i))
            .collect();
        order.extend(rest);
        order
    }

    /// Vaga cujo painel está capturando o teclado (digitando um filtro, esperando uma
    /// confirmação...): enquanto houver uma, só ela recebe as teclas
    pub fn capturing_slot(&self) -> Option<usize> {
        (0..self.slots.len()).find(|&i| {
            let candidate = self.slots[i].active();
            !candidate.failed()
                && isolate(|| candidate.panel.captures_input()).unwrap_or_else(|message| {
                    candidate.record(message);
                    false
                })
        })
    }

    /// Vagas que recebem teclas e anunciam atalhos: só a que captura, se houver
    fn input_order(&self) -> Vec<usize> {
        match self.capturing_slot() {
            Some(i) => vec![i],
            None => self.key_order(),
        }
    }

    /// Oferece a tecla aos painéis ativos até um deles tratá-la. Um painel que entra em
    /// pânico aqui é retirado e a tecla segue para os próximos.
    pub fn handle_key(&mut self, key: KeyEvent) -> Handled {
        for i in self.input_order() {
            let candidate = self.slots[i].active_mut();
            if candidate.failed() {
                continue;
            }
            match isolate(|| candidate.panel.handle_key(key)) {
                Ok(Handled::Yes) => return Handled::Yes,
                Ok(Handled::No) => {}
                Err(message) => candidate.record(message),
            }
        }
        Handled::No
    }

    /// Atalhos dos painéis ativos (exceto os que falharam), na ordem das teclas
    pub fn keybindings(&self) -> Vec<KeyHint> {
        self.input_order()
            .into_iter()
            .map(|i| self.slots[i].active())
            .filter(|c| !c.failed())
            .flat_map(|c| c.panel.keybindings().iter().copied())
            .collect()
    }

    /// Mouse no corpo da tela: o clique foca a vaga sob o cursor e o evento vai para o
    /// painel dela. Enquanto um painel captura o teclado, as outras vagas ignoram o mouse.
    pub fn handle_mouse(&mut self, event: MouseEvent, body: Rect) -> Handled {
        let position = Position::new(event.column, event.row);
        let Some((i, area)) = self
            .visible_slots(body)
            .into_iter()
            .find(|(_, area)| area.contains(position))
        else {
            return Handled::No;
        };
        if self.capturing_slot().is_some_and(|c| c != i) {
            return Handled::No;
        }
        if event.kind == MouseEventKind::Down(MouseButton::Left) {
            self.focused = Some(i);
        }
        let candidate = self.slots[i].active_mut();
        if candidate.failed() {
            return Handled::No;
        }
        match isolate(|| candidate.panel.handle_mouse(event, area)) {
            Ok(handled) => handled,
            Err(message) => {
                candidate.record(message);
                Handled::No
            }
        }
    }
}
/// Caixa vermelha no lugar de um painel que entrou em pânico
fn render_failure(buf: &mut Buffer, area: Rect, id: &str, message: &str) {
    Clear.render(area, buf);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(RED_ALERT))
        .style(Style::default().bg(BG_DARK))
        .title(Span::styled(
            format!(" {id} "),
            Style::default().fg(RED_ALERT).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    block.render(area, buf);

    let lines = [
        (format!("O painel \"{id}\" falhou:"), RED_ALERT),
        (message.to_string(), TEXT_WHITE),
        (
            "Os demais painéis continuam funcionando.".to_string(),
            TEXT_DIM,
        ),
    ];
    for (row, (text, color)) in lines.iter().enumerate() {
        let y = inner.top() + row as u16;
        if y >= inner.bottom() {
            break;
        }
        buf.set_stringn(
            inner.left() + 1,
            y,
            text,
            inner.width.saturating_sub(2) as usize,
            Style::default().fg(*color),
        );
    }
}

fn lookup<'a>(registry: &'a [PanelFactory], id: &str) -> &'a PanelFactory {
    registry::find(registry, id).unwrap_or_else(|| {
        panic!("painel \"{id}\" não registrado (a configuração já valida os ids)")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;
    use nebula_core::Size;
    use nebula_core::sysinfo::System;
    use nebula_panels::GpuPanel;
    use ratatui::buffer::Buffer;
    use ratatui::style::Style;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn hint_keys(panels: &PanelSet) -> Vec<&'static str> {
        panels.keybindings().iter().map(|h| h.key).collect()
    }

    fn active_ids(panels: &PanelSet) -> Vec<&'static str> {
        (0..panels.slot_count())
            .map(|i| panels.slot(i).id())
            .collect()
    }

    #[test]
    fn sem_foco_teclas_e_atalhos_comecam_pelos_processos() {
        let mut panels = PanelSet::demo();
        assert_eq!(panels.focused(), None);
        assert_eq!(hint_keys(&panels), ["Tab", "↑↓", "c", "/", "k", "g"]);
        assert_eq!(panels.handle_key(key(KeyCode::Tab)), Handled::Yes);
        assert_eq!(panels.handle_key(key(KeyCode::Down)), Handled::Yes);
    }

    #[test]
    fn foco_muda_a_ordem_mas_teclas_de_outros_paineis_continuam_chegando() {
        let mut panels = PanelSet::demo();
        assert!(panels.focus(3)); // GPU
        assert_eq!(panels.focused(), Some(2));
        assert_eq!(hint_keys(&panels), ["g", "Tab", "↑↓", "c", "/", "k"]);

        assert!(panels.focus(1)); // CPU não trata g, mas a GPU recebe em seguida
        assert_eq!(panels.handle_key(key(KeyCode::Char('g'))), Handled::Yes);
        assert_eq!(panels.handle_key(key(KeyCode::Char('z'))), Handled::No);
    }

    #[test]
    fn focar_painel_sem_atalhos_nao_reordena_o_rodape() {
        let mut panels = PanelSet::demo();
        assert!(panels.focus(2)); // Memória: não declara atalhos
        assert_eq!(hint_keys(&panels), ["Tab", "↑↓", "c", "/", "k", "g"]);
        assert_eq!(panels.key_order(), [1, 4, 0, 2, 3]);
    }

    #[test]
    fn foco_fora_das_vagas_e_ignorado_e_esc_solta() {
        let mut panels = PanelSet::demo();
        assert!(!panels.focus(0));
        assert!(!panels.focus(panels.slot_count() + 1));
        assert_eq!(panels.focused(), None);

        assert!(panels.focus(panels.slot_count()));
        assert!(panels.clear_focus());
        assert!(!panels.clear_focus());
        assert_eq!(panels.focused(), None);
    }

    fn layout_gpu_ou_memoria() -> Node {
        Node::Slot {
            panels: vec!["gpu".into(), "memory".into()],
        }
    }

    #[test]
    fn sem_gpu_a_proxima_alternativa_ocupa_a_vaga() {
        let registry = registry::builtin();
        let panels = PanelSet::build(layout_gpu_ou_memoria(), &mut |id| {
            if id == "gpu" {
                Box::new(GpuPanel::new()) // sem NVML: indisponível
            } else {
                (lookup(&registry, id).demo)()
            }
        });
        assert_eq!(active_ids(&panels), ["memory"]);

        // com GPU disponível, ela tem a preferência
        let panels = PanelSet::demo_with(layout_gpu_ou_memoria(), &registry);
        assert_eq!(active_ids(&panels), ["gpu"]);
    }

    #[test]
    fn nenhuma_alternativa_disponivel_mostra_o_aviso_da_ultima() {
        let layout = Node::Slot {
            panels: vec!["gpu".into()],
        };
        let panels = PanelSet::build(layout, &mut |_| Box::new(GpuPanel::new()));
        assert_eq!(active_ids(&panels), ["gpu"]);
    }

    #[test]
    fn layout_sem_processos_continua_roteando_teclas() {
        let layout = Node::Slot {
            panels: vec!["gpu".into()],
        };
        let mut panels = PanelSet::demo_with(layout, &registry::builtin());
        assert_eq!(panels.key_order(), [0]);
        assert_eq!(panels.handle_key(key(KeyCode::Char('g'))), Handled::Yes);
        assert_eq!(panels.handle_key(key(KeyCode::Tab)), Handled::No);
    }

    /// Painel de teste que entra em pânico na etapa escolhida
    struct Bomba(&'static str);

    impl Panel for Bomba {
        fn id(&self) -> &'static str {
            "bomba"
        }
        fn available(&self) -> bool {
            assert!(self.0 != "available", "falha simulada em available");
            true
        }
        fn update(&mut self, _ctx: &Context) {
            assert!(self.0 != "update", "falha simulada no update");
        }
        fn render(&self, area: Rect, buf: &mut Buffer, _view: &View) {
            assert!(self.0 != "render", "falha simulada no render");
            buf.set_string(area.x, area.y, "bomba ok", Style::default());
        }
        fn handle_key(&mut self, _key: KeyEvent) -> Handled {
            assert!(self.0 != "key", "falha simulada na tecla");
            Handled::No
        }
        fn min_size(&self) -> Size {
            Size {
                width: 1,
                height: 1,
            }
        }
    }

    /// Vagas com uma Bomba (que falha em `etapa`) e os demais ids em modo demo
    fn com_bomba(etapa: &'static str, layout: Node) -> PanelSet {
        let registry = registry::builtin();
        PanelSet::build(layout, &mut |id| {
            if id == "bomba" {
                Box::new(Bomba(etapa))
            } else {
                (lookup(&registry, id).demo)()
            }
        })
    }

    fn vaga(ids: &[&str]) -> Node {
        Node::Slot {
            panels: ids.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn falha(panels: &PanelSet, slot: usize) -> Option<String> {
        panels.slots[slot].active().failure.borrow().clone()
    }

    fn texto(panels: &PanelSet, slot: usize) -> String {
        let area = Rect::new(0, 0, 60, 6);
        let mut buf = Buffer::empty(area);
        panels.render_slot(slot, area, &mut buf, &View::default());
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
    fn falha_no_update_passa_a_vaga_para_o_proximo_candidato() {
        let mut panels = com_bomba("update", vaga(&["bomba", "memory"]));
        assert_eq!(active_ids(&panels), ["bomba"]);
        let system = System::new();
        panels.update(&Context {
            system: &system,
            nvml: None,
        });
        assert_eq!(active_ids(&panels), ["memory"]);
        let bomba = &panels.slots[0].candidates[0];
        assert!(
            bomba
                .failure
                .borrow()
                .as_deref()
                .unwrap()
                .contains("falha simulada no update")
        );
    }

    #[test]
    fn falha_no_render_vira_caixa_de_erro_e_nao_repete() {
        let panels = com_bomba("render", vaga(&["bomba"]));
        let primeira = texto(&panels, 0);
        assert!(primeira.contains("\"bomba\" falhou"), "{primeira}");
        assert!(primeira.contains("falha simulada no render"), "{primeira}");
        assert!(falha(&panels, 0).is_some());
        // já marcado como falho: o próximo quadro desenha a caixa sem chamar o painel
        assert_eq!(texto(&panels, 0), primeira);
    }

    #[test]
    fn falha_na_tecla_retira_o_painel_e_a_tecla_segue_adiante() {
        let layout = Node::Split {
            direction: ratatui::layout::Direction::Horizontal,
            sizes: vec![50, 50],
            children: vec![vaga(&["bomba"]), vaga(&["gpu"])],
        };
        let mut panels = com_bomba("key", layout);
        assert_eq!(
            panels.handle_key(key(KeyCode::Char('g'))),
            Handled::Yes,
            "a GPU ainda recebe o g"
        );
        assert!(
            falha(&panels, 0)
                .unwrap()
                .contains("falha simulada na tecla")
        );
        assert_eq!(
            hint_keys(&panels),
            ["g"],
            "painel falho não anuncia atalhos"
        );
    }

    #[test]
    fn falha_em_available_conta_como_indisponivel() {
        let panels = com_bomba("available", vaga(&["bomba", "memory"]));
        assert_eq!(active_ids(&panels), ["memory"]);
    }

    #[test]
    fn maximizar_exige_foco_e_mostra_so_a_vaga_focada() {
        let body = Rect::new(0, 1, 160, 38);
        let mut panels = PanelSet::demo();
        assert!(!panels.toggle_maximize(), "sem foco não maximiza");
        assert_eq!(panels.visible_slots(body).len(), 5);

        assert!(panels.focus(5));
        assert!(panels.toggle_maximize());
        assert!(panels.maximized());
        assert_eq!(panels.visible_slots(body), vec![(4, body)]);

        // trocar o foco mantém maximizado, agora na nova vaga
        assert!(panels.focus(1));
        assert_eq!(panels.visible_slots(body), vec![(0, body)]);

        assert!(panels.restore());
        assert!(!panels.maximized());
        assert_eq!(panels.visible_slots(body).len(), 5);
    }

    #[test]
    fn painel_capturando_recebe_todas_as_teclas_e_so_ele_anuncia_atalhos() {
        let mut panels = PanelSet::demo();
        assert_eq!(panels.capturing_slot(), None);
        assert_eq!(panels.handle_key(key(KeyCode::Char('/'))), Handled::Yes);
        assert_eq!(panels.capturing_slot(), Some(4)); // Processos digitando o filtro
        assert_eq!(hint_keys(&panels), ["Enter", "Esc"]);
        // o g vira texto do filtro em vez de chegar à GPU
        assert_eq!(panels.handle_key(key(KeyCode::Char('g'))), Handled::Yes);
        assert_eq!(panels.handle_key(key(KeyCode::Esc)), Handled::Yes);
        assert_eq!(panels.capturing_slot(), None);
    }

    fn clique(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }
    }

    #[test]
    fn clique_foca_a_vaga_sob_o_mouse() {
        let body = Rect::new(0, 1, 160, 38);
        let mut panels = PanelSet::demo();
        let gpu = panels.areas(body)[2];
        panels.handle_mouse(clique(gpu.x + 5, gpu.y + 5), body);
        assert_eq!(panels.focused(), Some(2));

        // fora do corpo não muda nada
        assert_eq!(panels.handle_mouse(clique(5, 0), body), Handled::No);
        assert_eq!(panels.focused(), Some(2));

        // com Processos capturando o teclado, clicar em outra vaga não tira o foco
        panels.handle_key(key(KeyCode::Char('k')));
        let cpu = panels.areas(body)[0];
        assert_eq!(
            panels.handle_mouse(clique(cpu.x + 5, cpu.y + 5), body),
            Handled::No
        );
        assert_eq!(panels.focused(), Some(2));
    }

    #[test]
    fn tirar_o_foco_restaura_o_layout() {
        let mut panels = PanelSet::demo();
        panels.focus(2);
        panels.toggle_maximize();
        assert!(panels.clear_focus());
        assert!(!panels.maximized());
        assert!(!panels.toggle_maximize());
    }
}
