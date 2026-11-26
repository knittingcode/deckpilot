//! A panic in the agent's main loop ends the process (DP-AGENT-02).
//!
//! The agent's main loop runs on a Tokio runtime: on the main thread when
//! headless, on a background thread next to the tray's event loop
//! otherwise. A panic there used to unwind only that thread, and in tray
//! mode the process stayed alive with a tray whose menu did nothing.
//! [`run_agent`] catches it instead, logs it, shuts the runtime down (the
//! IPC endpoint closes, then the instance lock is released: `server.rs`)
//! and reports [`Outcome::Panicked`], for which the process exits with
//! [`PANIC_EXIT_CODE`].
//!
//! [`install_panic_hook`] writes every panic to the agent's log as well,
//! because the configuration app starts the agent with its standard error
//! going nowhere.

use std::future::Future;
use std::panic::{self, AssertUnwindSafe};
use std::time::Duration;

use tracing::error;

/// The exit code of an agent whose main loop panicked: Rust's own for a
/// panic, so a caught panic and one that was not look the same to whoever
/// started the agent.
pub const PANIC_EXIT_CODE: u8 = 101;
/// The exit code of an agent that stopped with an error (it could not
/// start, say).
pub const FAILURE_EXIT_CODE: u8 = 1;
/// How long the runtime may take, once the main loop has ended (as asked,
/// with an error or with a panic), to stop its tasks (the IPC server among
/// them) and wait for its blocking threads. One that is stuck (a credential
/// store prompt, a plugin host call) does not keep the process, and the
/// tray icon, alive.
pub const RUNTIME_SHUTDOWN: Duration = Duration::from_secs(2);
/// The longest panic message written to the log.
const MAX_MESSAGE: usize = 500;

/// How the agent's main loop ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// It stopped as asked (Quit, a client, a signal).
    Stopped,
    /// It stopped with an error, or could not start.
    Failed,
    /// It panicked.
    Panicked,
}

impl Outcome {
    /// The process's exit code for this outcome.
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Stopped => 0,
            Self::Failed => FAILURE_EXIT_CODE,
            Self::Panicked => PANIC_EXIT_CODE,
        }
    }
}

/// Runs the agent's main loop `main` on `runtime` to its end, then shuts
/// the runtime down (its tasks dropped, its blocking threads given
/// [`RUNTIME_SHUTDOWN`]). A panic in the loop is caught, and the outcome is
/// [`Outcome::Panicked`]. By then the agent has been dropped with its
/// instance lock, and the IPC endpoint is closed.
pub fn run_agent<E: std::fmt::Display>(
    runtime: tokio::runtime::Runtime,
    main: impl Future<Output = Result<(), E>>,
) -> Outcome {
    let outcome = match panic::catch_unwind(AssertUnwindSafe(|| runtime.block_on(main))) {
        Ok(Ok(())) => Outcome::Stopped,
        Ok(Err(error)) => {
            error!(%error, "agent stopped with an error");
            Outcome::Failed
        }
        Err(_) => {
            // The panic itself is in the log already (the hook).
            error!("the agent's main loop panicked; the agent stops");
            Outcome::Panicked
        }
    };
    // Bounded on every outcome: dropping the runtime would wait for its
    // blocking threads without a limit.
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN);
    outcome
}

/// Logs every panic (thread, location, message) before the previous hook
/// runs (Rust's default prints it to standard error). Panics that are
/// caught on purpose (the tray icon's, a plugin task's) are logged too.
/// The message is logged escaped (`Debug`): a panic's text may come from a
/// plugin, and must not be able to forge log lines.
pub fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_default();
        let message = message(info.payload());
        let thread = std::thread::current();
        error!(
            thread = thread.name().unwrap_or("unnamed"),
            %location,
            ?message,
            "panic"
        );
        previous(info);
    }));
}

/// The text of a panic's payload, at most [`MAX_MESSAGE`] characters.
fn message(payload: &(dyn std::any::Any + Send)) -> String {
    let text = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("(no message)");
    match text.char_indices().nth(MAX_MESSAGE) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

/// Test only (feature `test-hooks`): makes the main loop panic.
#[cfg(feature = "test-hooks")]
pub mod testing {
    use std::time::Duration;

    use crate::service::AgentHandle;

    /// Milliseconds after the start at which the agent's main loop panics
    /// (a test of a spawned agent, headless or with the tray). Never read
    /// by a build without `test-hooks`.
    pub const PANIC_AFTER_START_ENV: &str = "DECKPILOT_TEST_PANIC_AFTER_START_MS";

    /// Arranges the panic [`PANIC_AFTER_START_ENV`] asks for, if any. Must
    /// be called from within the agent's runtime.
    pub fn arm(handle: &AgentHandle) {
        let Some(after) = std::env::var(PANIC_AFTER_START_ENV)
            .ok()
            .and_then(|ms| ms.trim().parse::<u64>().ok())
        else {
            return;
        };
        let handle = handle.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(after)).await;
            tracing::warn!("test hook: the agent's main loop is made to panic");
            handle.panic_main_loop();
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    }

    /// Sets its flag when dropped.
    struct Dropped(Arc<AtomicBool>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn outcomes_have_their_exit_codes() {
        assert_eq!(Outcome::Stopped.exit_code(), 0);
        assert_eq!(Outcome::Failed.exit_code(), 1);
        assert_eq!(Outcome::Panicked.exit_code(), 101);
    }

    #[test]
    fn a_loop_that_stops_or_fails_says_so() {
        assert_eq!(
            run_agent(runtime(), async { Ok::<(), String>(()) }),
            Outcome::Stopped
        );
        assert_eq!(
            run_agent(runtime(), async { Err::<(), _>("no endpoint") }),
            Outcome::Failed
        );
    }

    #[test]
    fn a_panic_is_caught_and_stops_the_runtimes_tasks() {
        // Stands for the IPC server: a task that would run forever.
        let server = Arc::new(AtomicBool::new(false));
        // Stands for the agent, owned by the main loop.
        let agent = Arc::new(AtomicBool::new(false));
        let (server_flag, agent_flag) = (server.clone(), agent.clone());
        let outcome = run_agent(runtime(), async move {
            let guard = Dropped(server_flag);
            tokio::spawn(async move {
                let _guard = guard;
                std::future::pending::<()>().await;
            });
            let _agent = Dropped(agent_flag);
            tokio::task::yield_now().await;
            panic!("the main loop panics");
            #[allow(unreachable_code)]
            Ok::<(), String>(())
        });
        assert_eq!(outcome, Outcome::Panicked);
        assert!(agent.load(Ordering::SeqCst), "the agent was dropped");
        assert!(
            server.load(Ordering::SeqCst),
            "the runtime's tasks were stopped"
        );
    }

    #[test]
    fn a_stuck_blocking_thread_does_not_keep_the_agent_alive() {
        let started = std::time::Instant::now();
        let outcome = run_agent(runtime(), async {
            tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_secs(30)));
            tokio::task::yield_now().await;
            panic!("the main loop panics");
            #[allow(unreachable_code)]
            Ok::<(), String>(())
        });
        assert_eq!(outcome, Outcome::Panicked);
        assert!(started.elapsed() < RUNTIME_SHUTDOWN + Duration::from_secs(5));
    }

    #[test]
    fn a_stuck_blocking_thread_does_not_hold_up_a_normal_stop() {
        let started = std::time::Instant::now();
        let outcome = run_agent(runtime(), async {
            tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_secs(30)));
            tokio::task::yield_now().await;
            Ok::<(), String>(())
        });
        assert_eq!(outcome, Outcome::Stopped);
        assert!(started.elapsed() < RUNTIME_SHUTDOWN + Duration::from_secs(5));
    }

    #[test]
    fn long_messages_are_cut() {
        let long = "x".repeat(MAX_MESSAGE * 2);
        let cut = message(&long);
        assert_eq!(cut.chars().count(), MAX_MESSAGE + 1);
        assert_eq!(message(&"short"), "short");
        assert_eq!(message(&42_u8), "(no message)");
    }
}
