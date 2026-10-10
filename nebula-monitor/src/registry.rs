//! Painéis disponíveis pelo id usado na configuração. Binários pessoais partem de
//! `builtin()` e acrescentam os próprios painéis antes de chamar `nebula_monitor::run`.

use nebula_core::Panel;
use nebula_panels::{CpuPanel, DisksPanel, GpuPanel, MemoryPanel, NetworkPanel, ProcessesPanel};

pub struct PanelFactory {
    pub id: &'static str,
    /// Painel com coleta real
    pub create: fn() -> Box<dyn Panel>,
    /// Painel com dados fictícios (modo demo e testes)
    pub demo: fn() -> Box<dyn Panel>,
}

pub fn builtin() -> Vec<PanelFactory> {
    vec![
        PanelFactory {
            id: "cpu",
            create: || Box::new(CpuPanel::new()),
            demo: || Box::new(CpuPanel::demo()),
        },
        PanelFactory {
            id: "memory",
            create: || Box::new(MemoryPanel::new()),
            demo: || Box::new(MemoryPanel::demo()),
        },
        PanelFactory {
            id: "gpu",
            create: || Box::new(GpuPanel::new()),
            demo: || Box::new(GpuPanel::demo()),
        },
        PanelFactory {
            id: "gpu2",
            create: || Box::new(GpuPanel::with_index(1)),
            demo: || Box::new(GpuPanel::demo_with_index(1)),
        },
        PanelFactory {
            id: "gpu3",
            create: || Box::new(GpuPanel::with_index(2)),
            demo: || Box::new(GpuPanel::demo_with_index(2)),
        },
        PanelFactory {
            id: "gpu4",
            create: || Box::new(GpuPanel::with_index(3)),
            demo: || Box::new(GpuPanel::demo_with_index(3)),
        },
        PanelFactory {
            id: "disks",
            create: || Box::new(DisksPanel::new()),
            demo: || Box::new(DisksPanel::demo()),
        },
        PanelFactory {
            id: "processes",
            create: || Box::new(ProcessesPanel::new()),
            demo: || Box::new(ProcessesPanel::demo()),
        },
        PanelFactory {
            id: "network",
            create: || Box::new(NetworkPanel::new()),
            demo: || Box::new(NetworkPanel::demo()),
        },
    ]
}

/// Primeiro id que aparece mais de uma vez no registro, se houver
pub fn duplicate_id(registry: &[PanelFactory]) -> Option<&'static str> {
    registry
        .iter()
        .enumerate()
        .find(|(i, f)| registry[..*i].iter().any(|g| g.id == f.id))
        .map(|(_, f)| f.id)
}

pub fn ids(registry: &[PanelFactory]) -> Vec<&'static str> {
    registry.iter().map(|f| f.id).collect()
}

pub fn find<'a>(registry: &'a [PanelFactory], id: &str) -> Option<&'a PanelFactory> {
    registry.iter().find(|f| f.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registro_nativo_nao_tem_ids_repetidos() {
        assert_eq!(duplicate_id(&builtin()), None);
        let mut registro = builtin();
        registro.push(PanelFactory {
            id: "gpu",
            create: || Box::new(MemoryPanel::new()),
            demo: || Box::new(MemoryPanel::demo()),
        });
        assert_eq!(duplicate_id(&registro), Some("gpu"));
    }

    #[test]
    fn cada_fabrica_cria_o_painel_do_proprio_id() {
        for f in builtin() {
            assert_eq!((f.demo)().id(), f.id);
        }
    }
}
