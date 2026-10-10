//! Isolamento de pânicos: o código de um painel (ou de um `Worker`) que entra em pânico
//! vira um erro com mensagem, em vez de derrubar o monitor.
//!
//! Enquanto o código isolado roda, a thread fica marcada; o panic hook do monitor consulta
//! `panicking_isolated()` para não restaurar o terminal nem imprimir nada — a falha é
//! mostrada no lugar do painel.

use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};

thread_local! {
    static ISOLATED: Cell<bool> = const { Cell::new(false) };
}

/// O pânico em andamento nesta thread aconteceu dentro de `isolate` (e será tratado)?
pub fn panicking_isolated() -> bool {
    ISOLATED.with(Cell::get)
}

/// Roda `f` capturando um eventual pânico; em caso de falha devolve a mensagem dele
pub fn isolate<R>(f: impl FnOnce() -> R) -> Result<R, String> {
    let was_isolated = ISOLATED.with(|c| c.replace(true));
    let result = panic::catch_unwind(AssertUnwindSafe(f));
    ISOLATED.with(|c| c.set(was_isolated));
    result.map_err(|payload| {
        payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "pânico sem mensagem".to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marca_a_thread_so_durante_a_chamada() {
        assert!(!panicking_isolated());
        assert_eq!(isolate(panicking_isolated), Ok(true));
        assert!(!panicking_isolated());
    }

    #[test]
    fn devolve_a_mensagem_do_panico() {
        assert_eq!(
            isolate(|| -> u8 { panic!("estática") }),
            Err("estática".to_string())
        );
        assert_eq!(
            isolate(|| -> u8 { panic!("{}", String::from("dinâmica")) }),
            Err("dinâmica".to_string())
        );
    }

    #[test]
    fn isolamento_aninhado_mantem_a_marca_de_fora() {
        let dentro = isolate(|| {
            let _ = isolate(|| ());
            panicking_isolated()
        });
        assert_eq!(
            dentro,
            Ok(true),
            "sair do isolamento interno não desmarca o externo"
        );
    }
}
