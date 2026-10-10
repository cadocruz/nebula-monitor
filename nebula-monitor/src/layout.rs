//! Árvore de vagas da tela: divisões em linhas ou colunas (com porcentagens) e, nas
//! folhas, a lista de painéis candidatos — o primeiro disponível ocupa a vaga.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Split {
        direction: Direction,
        /// Porcentagem de cada filho (somam 100)
        sizes: Vec<u16>,
        children: Vec<Node>,
    },
    Slot {
        /// Ids dos painéis em ordem de preferência
        panels: Vec<String>,
    },
}

fn slot(ids: &[&str]) -> Node {
    Node::Slot {
        panels: ids.iter().map(|s| s.to_string()).collect(),
    }
}

fn split(direction: Direction, sizes: &[u16], children: Vec<Node>) -> Node {
    Node::Split {
        direction,
        sizes: sizes.to_vec(),
        children,
    }
}

impl Node {
    /// A tela de sempre: CPU | (Memória / GPU) em cima, Discos | Processos embaixo
    pub fn default_layout() -> Node {
        split(
            Direction::Vertical,
            &[55, 45],
            vec![
                split(
                    Direction::Horizontal,
                    &[52, 48],
                    vec![
                        slot(&["cpu"]),
                        split(
                            Direction::Vertical,
                            &[50, 50],
                            vec![slot(&["memory"]), slot(&["gpu"])],
                        ),
                    ],
                ),
                split(
                    Direction::Horizontal,
                    &[60, 40],
                    vec![slot(&["disks"]), slot(&["processes"])],
                ),
            ],
        )
    }

    /// Listas de candidatos de cada vaga, na ordem de leitura (é a ordem das teclas 1–9)
    pub fn slots(&self) -> Vec<&[String]> {
        let mut out = Vec::new();
        self.collect_slots(&mut out);
        out
    }

    fn collect_slots<'a>(&'a self, out: &mut Vec<&'a [String]>) {
        match self {
            Node::Slot { panels } => out.push(panels),
            Node::Split { children, .. } => {
                for child in children {
                    child.collect_slots(out);
                }
            }
        }
    }

    /// Retângulo de cada vaga dentro de `area`, na mesma ordem de `slots()`
    pub fn areas(&self, area: Rect) -> Vec<Rect> {
        let mut out = Vec::new();
        self.collect_areas(area, &mut out);
        out
    }

    fn collect_areas(&self, area: Rect, out: &mut Vec<Rect>) {
        match self {
            Node::Slot { .. } => out.push(area),
            Node::Split {
                direction,
                sizes,
                children,
            } => {
                let parts = Layout::default()
                    .direction(*direction)
                    .constraints(sizes.iter().map(|&s| Constraint::Percentage(s)))
                    .split(area);
                for (child, part) in children.iter().zip(parts.iter()) {
                    child.collect_areas(*part, out);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_padrao_tem_as_cinco_vagas_de_sempre() {
        let layout = Node::default_layout();
        let ids: Vec<&str> = layout.slots().iter().map(|s| s[0].as_str()).collect();
        assert_eq!(ids, ["cpu", "memory", "gpu", "disks", "processes"]);
    }

    #[test]
    fn layout_padrao_reproduz_as_areas_da_tela_fixa() {
        // corpo da tela 160x40: entre a barra do topo e o rodapé
        let body = Rect::new(0, 1, 160, 38);
        let areas = Node::default_layout().areas(body);
        assert_eq!(areas.len(), 5);
        // CPU à esquerda em cima, Processos à direita embaixo, sem sobreposição
        assert_eq!((areas[0].x, areas[0].y), (0, 1));
        assert_eq!(areas[4].right(), 160);
        assert_eq!(areas[4].bottom(), 39);
        assert_eq!(areas[1].x, areas[0].right());
        assert_eq!(areas[2].y, areas[1].bottom());
        assert_eq!(areas[3].y, areas[0].bottom());
    }
}
