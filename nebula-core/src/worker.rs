//! Trabalho lento fora da thread da tela.
//!
//! `update` de um painel roda no mesmo laço que desenha a tela e lê o teclado: se ele
//! demora (rede, disco lento, um comando externo), o monitor inteiro congela. Um `Worker`
//! roda o trabalho numa thread própria, no ritmo escolhido, e o painel só pega o último
//! resultado pronto — o que é instantâneo.
//!
//! ```no_run
//! use std::time::Duration;
//! use nebula_core::Worker;
//!
//! struct CaixaDeEntrada {
//!     contagem: Worker<usize>,
//!     nao_lidas: Option<usize>,
//! }
//!
//! impl CaixaDeEntrada {
//!     fn new() -> Self {
//!         // roda já e depois a cada 60 s, sem travar a tela
//!         let contagem = Worker::spawn("email", Duration::from_secs(60), || {
//!             # fn consultar_servidor() -> usize { 0 }
//!             consultar_servidor() // pode levar segundos
//!         });
//!         Self { contagem, nao_lidas: None }
//!     }
//!
//!     // no `Panel::update`:
//!     fn update(&mut self) {
//!         if let Some(n) = self.contagem.take_latest() {
//!             self.nao_lidas = Some(n);
//!         }
//!     }
//! }
//! ```

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::isolation::isolate;

pub struct Worker<T> {
    latest: Arc<Mutex<Option<T>>>,
    failure: Arc<Mutex<Option<String>>>,
    /// Acorda a thread antes do prazo; quando o `Worker` é descartado, o canal fecha e a
    /// thread termina depois do trabalho em andamento
    wake: Sender<()>,
}

impl<T: Send + 'static> Worker<T> {
    /// Roda `job` agora e depois a cada `every`, numa thread chamada `nebula-<name>`.
    ///
    /// Se `job` entrar em pânico, a thread para e `failure()` passa a devolver a mensagem;
    /// o monitor não cai.
    pub fn spawn(name: &str, every: Duration, mut job: impl FnMut() -> T + Send + 'static) -> Self {
        let latest = Arc::new(Mutex::new(None));
        let failure = Arc::new(Mutex::new(None));
        let (wake, wake_rx) = mpsc::channel::<()>();

        let (latest_t, failure_t) = (Arc::clone(&latest), Arc::clone(&failure));
        thread::Builder::new()
            .name(format!("nebula-{name}"))
            .spawn(move || {
                loop {
                    match isolate(&mut job) {
                        Ok(value) => *lock(&latest_t) = Some(value),
                        Err(message) => {
                            *lock(&failure_t) = Some(message);
                            return;
                        }
                    }
                    match wake_rx.recv_timeout(every) {
                        Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .expect("não foi possível criar a thread do Worker");

        Self {
            latest,
            failure,
            wake,
        }
    }

    /// Resultado mais recente ainda não lido (None se não chegou nada novo desde a
    /// última chamada)
    pub fn take_latest(&self) -> Option<T> {
        lock(&self.latest).take()
    }

    /// Mensagem do pânico, se o trabalho falhou (a thread não roda mais)
    pub fn failure(&self) -> Option<String> {
        lock(&self.failure).clone()
    }

    /// Pede uma nova execução já, sem esperar o intervalo (ex.: o usuário pediu "atualizar")
    pub fn wake(&self) {
        let _ = self.wake.send(());
    }
}

/// Um Mutex envenenado aqui só significa que o trabalho entrou em pânico segurando a trava,
/// o que `isolate` já registra; o valor guardado continua válido
fn lock<V>(m: &Mutex<V>) -> std::sync::MutexGuard<'_, V> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    /// Espera até `cond` ser verdadeira (ou falha após 5 s; o CI pode ser lento)
    fn eventually(mut cond: impl FnMut() -> bool) {
        let start = Instant::now();
        while !cond() {
            assert!(start.elapsed() < Duration::from_secs(5), "tempo esgotado");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn roda_logo_e_depois_periodicamente() {
        let runs = Arc::new(AtomicUsize::new(0));
        let r = Arc::clone(&runs);
        let worker = Worker::spawn("teste", Duration::from_millis(10), move || {
            r.fetch_add(1, Ordering::SeqCst) + 1
        });
        eventually(|| runs.load(Ordering::SeqCst) >= 3);
        assert!(worker.take_latest().is_some());
    }

    #[test]
    fn take_latest_so_devolve_resultado_novo() {
        let worker = Worker::spawn("teste", Duration::from_secs(3600), || 42);
        let mut first = None;
        eventually(|| {
            first = worker.take_latest();
            first.is_some()
        });
        assert_eq!(first, Some(42));
        assert_eq!(
            worker.take_latest(),
            None,
            "já foi lido e o próximo é daqui a 1 h"
        );
    }

    #[test]
    fn wake_antecipa_a_proxima_execucao() {
        let runs = Arc::new(AtomicUsize::new(0));
        let r = Arc::clone(&runs);
        let worker = Worker::spawn("teste", Duration::from_secs(3600), move || {
            r.fetch_add(1, Ordering::SeqCst)
        });
        eventually(|| runs.load(Ordering::SeqCst) == 1);
        worker.wake();
        eventually(|| runs.load(Ordering::SeqCst) == 2);
    }

    #[test]
    fn panico_no_trabalho_vira_falha_e_para_a_thread() {
        let runs = Arc::new(AtomicUsize::new(0));
        let r = Arc::clone(&runs);
        let worker = Worker::spawn("teste", Duration::from_millis(5), move || -> u8 {
            r.fetch_add(1, Ordering::SeqCst);
            panic!("servidor fora do ar");
        });
        eventually(|| worker.failure().is_some());
        assert_eq!(worker.failure().as_deref(), Some("servidor fora do ar"));
        assert_eq!(worker.take_latest(), None);
        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            runs.load(Ordering::SeqCst),
            1,
            "não tenta de novo depois de falhar"
        );
    }

    #[test]
    fn descartar_o_worker_para_a_thread() {
        let runs = Arc::new(AtomicUsize::new(0));
        let r = Arc::clone(&runs);
        let worker = Worker::spawn("teste", Duration::from_millis(5), move || {
            r.fetch_add(1, Ordering::SeqCst)
        });
        eventually(|| runs.load(Ordering::SeqCst) >= 2);
        drop(worker);
        // a execução em andamento pode terminar; depois disso, mais nenhuma
        thread::sleep(Duration::from_millis(30));
        let parado = runs.load(Ordering::SeqCst);
        thread::sleep(Duration::from_millis(50));
        assert_eq!(runs.load(Ordering::SeqCst), parado);
    }
}
