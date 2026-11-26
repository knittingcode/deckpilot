//! Bounded restart of an agent this app started (DP-AGENT-02).
//!
//! The bridge (`agent.rs`) reconnects to whatever agent runs. This module
//! also restarts the agent when one **this app started** exits
//! unexpectedly. The owner's decision, a conservative default:
//!
//! - at most [`MAX_RESTARTS`] restarts in [`RESTART_WINDOW`] (3 in 10
//!   minutes), after [`BACKOFF`] (1 s, 5 s, then 30 s);
//! - after that it stops and shows "DeckPilot agent stopped" with a Restart
//!   action ([`Supervision::Stopped`]); the user's Start or Restart begins
//!   again with a clean slate;
//! - a stop that was asked for is never a crash: the agent's
//!   `shuttingDown` (the tray's Quit, a restart, an update, a signal), a
//!   stop this app requests ([`Supervisor::expect_stop`]), or an exit with
//!   code 0;
//! - closing the configuration window never stops the agent: it runs in
//!   its own process group, and nothing here kills it;
//! - an agent this app did not start (autostart at login, started by the
//!   user or by another app) is never restarted: the bridge only
//!   reconnects to it.
//!
//! Before a restart, nothing is started while an update is in progress
//! (`update-intent.json`), while any agent answers on the endpoint (of
//! whatever version: a foreign or older one keeps the compatibility notice,
//! nothing is downgraded or replaced here), or while the instance lock is
//! held (an agent is starting). The OS releases the lock of a dead agent,
//! and the agent that starts replaces a stale socket and token itself
//! (`docs/background-agent.md`), so a restart never makes a second agent.
//!
//! A restarted agent starts locked, like any agent. It is unlocked only the
//! way the bridge unlocks any agent that (re)started: with the key of this
//! app while this app is unlocked, and never after a lock that needs the
//! password (`agent.rs`, `unlock_if_needed`). Nothing here reads or holds a
//! key.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;

/// Restarts allowed within [`RESTART_WINDOW`].
pub const MAX_RESTARTS: usize = 3;
/// The window [`MAX_RESTARTS`] counts in.
pub const RESTART_WINDOW: Duration = Duration::from_secs(10 * 60);
/// The wait before the first, second and third restart in the window.
pub const BACKOFF: [Duration; MAX_RESTARTS] = [
    Duration::from_secs(1),
    Duration::from_secs(5),
    Duration::from_secs(30),
];
/// How often the agents this app started are checked.
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// How long a restart waits for the instance lock to be free.
pub const LOCK_WAIT: Duration = Duration::from_secs(5);
/// How long an expected stop is believed: an agent still running this long
/// after it was asked to stop (the request failed) is supervised again.
pub const STOP_EXPECTED_FOR: Duration = Duration::from_secs(30);

/// How an agent process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Exit {
    pub success: bool,
    /// The exit code, if it exited (none if a signal ended it).
    pub code: Option<i32>,
    /// The signal that ended it (Unix).
    pub signal: Option<i32>,
}

impl From<std::process::ExitStatus> for Exit {
    fn from(status: std::process::ExitStatus) -> Self {
        #[cfg(unix)]
        let signal = std::os::unix::process::ExitStatusExt::signal(&status);
        #[cfg(not(unix))]
        let signal = None;
        Self {
            success: status.success(),
            code: status.code(),
            signal,
        }
    }
}

/// An agent process this app started.
pub trait Process: Send {
    fn id(&self) -> u32;
    /// `Some` once it has exited; never blocks.
    fn try_wait(&mut self) -> std::io::Result<Option<Exit>>;
}

impl Process for std::process::Child {
    fn id(&self) -> u32 {
        std::process::Child::id(self)
    }

    fn try_wait(&mut self) -> std::io::Result<Option<Exit>> {
        std::process::Child::try_wait(self).map(|status| status.map(Exit::from))
    }
}

/// Starts the agent process (in the app: `agent::spawn_agent_process`).
pub trait Launcher: Send + Sync {
    fn launch(&self) -> Result<Box<dyn Process>, String>;
}

/// Counts restarts in the window and gives the wait before the next.
#[derive(Debug, Default)]
pub struct RestartPolicy {
    restarts: VecDeque<Instant>,
}

impl RestartPolicy {
    /// An agent crashed at `now`: the wait before restarting it, or `None`
    /// once [`MAX_RESTARTS`] restarts happened within [`RESTART_WINDOW`].
    /// The restart counts from here.
    pub fn on_crash(&mut self, now: Instant) -> Option<Duration> {
        while self
            .restarts
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= RESTART_WINDOW)
        {
            self.restarts.pop_front();
        }
        let delay = *BACKOFF.get(self.restarts.len())?;
        self.restarts.push_back(now);
        Some(delay)
    }

    /// The last restart did not happen after all: it does not count.
    pub fn forget_last(&mut self) {
        self.restarts.pop_back();
    }

    /// Restarts counted in the window.
    pub fn restarts(&self) -> usize {
        self.restarts.len()
    }

    pub fn reset(&mut self) {
        self.restarts.clear();
    }
}

/// Whether an exit is a crash: not asked for, and not a success.
pub fn is_crash(stop_expected: bool, exit: Exit) -> bool {
    !stop_expected && !exit.success
}

/// What the app shows while the agent is not running, after an agent it
/// started exited unexpectedly (`AgentConnection::supervision`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Supervision {
    /// It is restarted after `delay_ms`: restart `restart` of `max`.
    Restarting {
        restart: u32,
        max: u32,
        delay_ms: u64,
        exit: Exit,
    },
    /// "DeckPilot agent stopped": it is not restarted again until the user
    /// starts it. `restarts` restarts within `window_minutes` came before
    /// (fewer if starting it again failed).
    Stopped {
        restarts: u32,
        window_minutes: u32,
        exit: Exit,
    },
}

/// What [`Supervisor::poll`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Nothing,
    /// An agent crashed: restart it after the delay.
    Restart(Duration),
    /// An agent crashed once too often: it stays stopped.
    GaveUp,
}

/// An agent process this app started.
struct Owned {
    process: Box<dyn Process>,
    /// When the app asked it to stop, or it announced that it stops.
    /// Forgotten after [`STOP_EXPECTED_FOR`] if it is still running.
    stop_expected: Option<Instant>,
}

#[derive(Default)]
struct Inner {
    owned: Vec<Owned>,
    policy: RestartPolicy,
    supervision: Option<Supervision>,
    /// Whether anything checks the agents (`run`): without it (a
    /// development session manages the agent) nothing is recorded.
    active: bool,
}

/// The agents this app started, and the restart policy.
pub struct Supervisor {
    inner: Mutex<Inner>,
    launcher: Box<dyn Launcher>,
}

impl Supervisor {
    pub fn new(launcher: Box<dyn Launcher>) -> Self {
        Self {
            inner: Mutex::default(),
            launcher,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The supervision loop runs: from now on started agents are recorded.
    pub fn activate(&self) {
        self.lock().active = true;
    }

    /// Starts the agent and supervises it (the first launch, an update,
    /// the security flows). Earlier restarts still count.
    pub fn start(&self) -> Result<(), String> {
        let process = self.launcher.launch()?;
        let mut inner = self.lock();
        if inner.active {
            inner.owned.push(Owned {
                process,
                stop_expected: None,
            });
        }
        Ok(())
    }

    /// The user's Start or Restart: forgets earlier restarts and a
    /// "stopped" state, then starts the agent.
    pub fn start_by_user(&self) -> Result<(), String> {
        {
            let mut inner = self.lock();
            inner.policy.reset();
            inner.supervision = None;
        }
        self.start()
    }

    /// This app is about to ask the agent to stop (a restart, Stop agent,
    /// an update), at `now`: its exit is not a crash, whatever its code. If
    /// the stop does not happen, the agent is supervised again after
    /// [`STOP_EXPECTED_FOR`].
    pub fn expect_stop(&self, now: Instant) {
        for owned in &mut self.lock().owned {
            owned.stop_expected = Some(now);
        }
    }

    /// The connected agent, process `pid`, announced at `now` that it
    /// stops.
    pub fn stopping(&self, pid: u32, now: Instant) {
        for owned in &mut self.lock().owned {
            if owned.process.id() == pid {
                owned.stop_expected = Some(now);
            }
        }
    }

    /// An agent answers: what the app showed about a crash is over.
    pub fn connected(&self) {
        self.lock().supervision = None;
    }

    pub fn supervision(&self) -> Option<Supervision> {
        self.lock().supervision
    }

    /// Whether an agent this app started still runs (as far as it knows:
    /// exits are noticed by [`Supervisor::poll`]).
    pub fn owns_running_agent(&self) -> bool {
        !self.lock().owned.is_empty()
    }

    /// Checks the agents this app started: forgets those that exited, and
    /// decides about a crash.
    pub fn poll(&self, now: Instant) -> Action {
        let mut inner = self.lock();
        let mut crashed = None;
        inner
            .owned
            .retain_mut(|owned| match owned.process.try_wait() {
                Ok(None) => {
                    // Asked to stop long ago and still running: the stop
                    // failed, and a later exit is a crash again.
                    if owned
                        .stop_expected
                        .is_some_and(|at| now.saturating_duration_since(at) >= STOP_EXPECTED_FOR)
                    {
                        owned.stop_expected = None;
                    }
                    true
                }
                Ok(Some(exit)) => {
                    if is_crash(owned.stop_expected.is_some(), exit) {
                        crashed = Some(exit);
                    }
                    false
                }
                // Cannot be supervised any longer: forgotten, never restarted.
                Err(error) => {
                    eprintln!("the agent process cannot be checked: {error}");
                    false
                }
            });
        let Some(exit) = crashed else {
            return Action::Nothing;
        };
        if !inner.owned.is_empty() {
            // Another agent this app started is still there: it takes over
            // once the lock is free, or it leaves as "already running".
            // Nothing is restarted for this exit (conservative).
            return Action::Nothing;
        }
        let Some(delay) = inner.policy.on_crash(now) else {
            inner.supervision = Some(stopped(inner.policy.restarts(), exit));
            return Action::GaveUp;
        };
        inner.supervision = Some(Supervision::Restarting {
            restart: count(inner.policy.restarts()),
            max: count(MAX_RESTARTS),
            delay_ms: u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
            exit,
        });
        Action::Restart(delay)
    }

    /// A restart did not happen: another agent is there or is starting, or
    /// an update is in progress. Nothing is shown about it, and it does not
    /// count toward the limit.
    fn restart_skipped(&self) {
        let mut inner = self.lock();
        inner.supervision = None;
        inner.policy.forget_last();
    }

    /// Starting the agent again failed: it stays stopped.
    fn restart_failed(&self) {
        let mut inner = self.lock();
        if let Some(Supervision::Restarting { exit, .. }) = inner.supervision {
            let restarts = inner.policy.restarts().saturating_sub(1);
            inner.supervision = Some(stopped(restarts, exit));
        }
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn stopped(restarts: usize, exit: Exit) -> Supervision {
    Supervision::Stopped {
        restarts: count(restarts),
        window_minutes: count(usize::try_from(RESTART_WINDOW.as_secs() / 60).unwrap_or(0)),
        exit,
    }
}

/// What a restart needs from the app (mocked in tests).
pub trait Environment: Sync {
    fn now(&self) -> Instant;
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send;
    /// An update is being installed or recovered (`update-intent.json`).
    fn update_in_progress(&self) -> bool;
    /// Whether any agent answers on the endpoint, of any version.
    fn agent_answers(&self) -> impl Future<Output = bool> + Send;
    /// Whether the instance lock is free within [`LOCK_WAIT`].
    fn lock_free(&self) -> impl Future<Output = bool> + Send;
    /// What [`Supervisor::supervision`] says changed: tell the windows the
    /// agent's actual state.
    fn changed(&self);
}

/// How a restart attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart {
    Started,
    /// The user started an agent meanwhile.
    AlreadyStarted,
    UpdateInProgress,
    /// An agent answers (autostarted, started by the user, of another
    /// version): the bridge reconnects to it.
    AgentAnswers,
    /// The instance lock is held: an agent is starting.
    LockHeld,
    Failed,
}

/// Restarts the agent after a crash, unless something says not to.
pub async fn restart(supervisor: &Supervisor, env: &impl Environment) -> Restart {
    let outcome = if supervisor.owns_running_agent() {
        Restart::AlreadyStarted
    } else if env.update_in_progress() {
        Restart::UpdateInProgress
    } else if env.agent_answers().await {
        Restart::AgentAnswers
    } else if !env.lock_free().await {
        Restart::LockHeld
    } else {
        start_unless_updating(supervisor, env)
    };
    if outcome == Restart::Failed {
        supervisor.restart_failed();
    } else if outcome != Restart::Started && outcome != Restart::AlreadyStarted {
        supervisor.restart_skipped();
    }
    outcome
}

/// Starts the agent, unless an update began while the lock was awaited.
fn start_unless_updating(supervisor: &Supervisor, env: &impl Environment) -> Restart {
    if env.update_in_progress() {
        return Restart::UpdateInProgress;
    }
    match supervisor.start() {
        Ok(()) => Restart::Started,
        Err(error) => {
            eprintln!("the agent could not be restarted: {error}");
            Restart::Failed
        }
    }
}

/// One round of supervision: checks the agents this app started and
/// restarts a crashed one after its delay.
pub async fn step(supervisor: &Supervisor, env: &impl Environment) -> Option<Restart> {
    match supervisor.poll(env.now()) {
        Action::Nothing => None,
        Action::GaveUp => {
            env.changed();
            None
        }
        Action::Restart(delay) => {
            env.changed();
            env.sleep(delay).await;
            let outcome = restart(supervisor, env).await;
            // A started agent is reported by the bridge once it answers.
            // Otherwise the "restarting" notice must go (the app sends the
            // real state, not "stopped", which could hide a running agent).
            if outcome != Restart::Started {
                env.changed();
            }
            Some(outcome)
        }
    }
}

/// Supervises the agents this app starts, for as long as the app runs.
/// [`Supervisor::activate`] must have been called before the first start.
pub async fn run(supervisor: &Supervisor, env: &impl Environment) {
    supervisor.activate();
    loop {
        env.sleep(POLL_INTERVAL).await;
        step(supervisor, env).await;
    }
}

/// Starts `agent::spawn_agent_process`.
struct AgentLauncher;

impl Launcher for AgentLauncher {
    fn launch(&self) -> Result<Box<dyn Process>, String> {
        crate::agent::spawn_agent_process().map(|child| Box::new(child) as Box<dyn Process>)
    }
}

static SUPERVISOR: LazyLock<Supervisor> =
    LazyLock::new(|| Supervisor::new(Box::new(AgentLauncher)));

/// The app's supervisor of the agents it starts.
pub fn global() -> &'static Supervisor {
    &SUPERVISOR
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use super::*;

    const CRASH: Exit = Exit {
        success: false,
        code: Some(101),
        signal: None,
    };
    const KILLED: Exit = Exit {
        success: false,
        code: None,
        signal: Some(9),
    };
    const CLEAN: Exit = Exit {
        success: true,
        code: Some(0),
        signal: None,
    };

    /// A process whose exit the test decides.
    struct Fake {
        pid: u32,
        exit: FakeExit,
    }

    impl Process for Fake {
        fn id(&self) -> u32 {
            self.pid
        }

        fn try_wait(&mut self) -> std::io::Result<Option<Exit>> {
            Ok(*self.exit.lock().unwrap())
        }
    }

    /// How a [`Fake`] exits, set by the test.
    type FakeExit = Arc<Mutex<Option<Exit>>>;

    /// Launches [`Fake`]s and keeps a handle on each one's exit.
    #[derive(Clone, Default)]
    struct FakeLauncher {
        launched: Arc<Mutex<Vec<FakeExit>>>,
        fail: Arc<AtomicBool>,
    }

    impl FakeLauncher {
        fn count(&self) -> usize {
            self.launched.lock().unwrap().len()
        }

        /// Makes the last launched agent exit with `exit`.
        fn exit_last(&self, exit: Exit) {
            let launched = self.launched.lock().unwrap();
            *launched.last().unwrap().lock().unwrap() = Some(exit);
        }

        fn last_pid(&self) -> u32 {
            u32::try_from(self.count()).unwrap()
        }
    }

    impl Launcher for FakeLauncher {
        fn launch(&self) -> Result<Box<dyn Process>, String> {
            if self.fail.load(Ordering::SeqCst) {
                return Err("no agent executable".into());
            }
            let exit = Arc::new(Mutex::new(None));
            let mut launched = self.launched.lock().unwrap();
            launched.push(exit.clone());
            Ok(Box::new(Fake {
                pid: u32::try_from(launched.len()).unwrap(),
                exit,
            }))
        }
    }

    /// An app whose clock, sleeps and endpoint the test controls.
    struct FakeEnv {
        now: Mutex<Instant>,
        slept: Mutex<Vec<Duration>>,
        update: AtomicBool,
        answers: AtomicBool,
        lock_held: AtomicBool,
        changes: AtomicU32,
    }

    impl FakeEnv {
        fn new() -> Self {
            Self {
                now: Mutex::new(Instant::now()),
                slept: Mutex::default(),
                update: AtomicBool::new(false),
                answers: AtomicBool::new(false),
                lock_held: AtomicBool::new(false),
                changes: AtomicU32::new(0),
            }
        }

        fn advance(&self, by: Duration) {
            *self.now.lock().unwrap() += by;
        }

        fn slept(&self) -> Vec<Duration> {
            std::mem::take(&mut *self.slept.lock().unwrap())
        }
    }

    impl Environment for FakeEnv {
        fn now(&self) -> Instant {
            *self.now.lock().unwrap()
        }

        fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
            self.slept.lock().unwrap().push(duration);
            self.advance(duration);
            std::future::ready(())
        }

        fn update_in_progress(&self) -> bool {
            self.update.load(Ordering::SeqCst)
        }

        fn agent_answers(&self) -> impl Future<Output = bool> + Send {
            std::future::ready(self.answers.load(Ordering::SeqCst))
        }

        fn lock_free(&self) -> impl Future<Output = bool> + Send {
            std::future::ready(!self.lock_held.load(Ordering::SeqCst))
        }

        fn changed(&self) {
            self.changes.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn setup() -> (Supervisor, FakeLauncher, FakeEnv) {
        let launcher = FakeLauncher::default();
        let supervisor = Supervisor::new(Box::new(launcher.clone()));
        supervisor.activate();
        (supervisor, launcher, FakeEnv::new())
    }

    fn run_step(supervisor: &Supervisor, env: &FakeEnv) -> Option<Restart> {
        tauri::async_runtime::block_on(step(supervisor, env))
    }

    #[test]
    fn the_policy_allows_three_restarts_in_ten_minutes_with_backoff() {
        let mut policy = RestartPolicy::default();
        let start = Instant::now();
        assert_eq!(policy.on_crash(start), Some(Duration::from_secs(1)));
        assert_eq!(
            policy.on_crash(start + Duration::from_secs(60)),
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            policy.on_crash(start + Duration::from_secs(120)),
            Some(Duration::from_secs(30))
        );
        assert_eq!(policy.on_crash(start + Duration::from_secs(180)), None);
        assert_eq!(policy.on_crash(start + Duration::from_secs(590)), None);
        // Ten minutes after the first restart it no longer counts.
        assert_eq!(
            policy.on_crash(start + RESTART_WINDOW),
            Some(Duration::from_secs(30))
        );
        policy.reset();
        assert_eq!(
            policy.on_crash(start + RESTART_WINDOW),
            Some(Duration::from_secs(1))
        );
    }

    #[test]
    fn only_an_exit_nobody_asked_for_that_is_not_a_success_is_a_crash() {
        assert!(is_crash(false, CRASH));
        assert!(is_crash(false, KILLED));
        assert!(!is_crash(false, CLEAN), "exit 0: e.g. already running");
        assert!(!is_crash(true, CRASH), "a stop that was asked for");
        assert!(!is_crash(true, CLEAN));
    }

    #[test]
    fn a_crashed_agent_is_restarted_three_times_then_stays_stopped() {
        let (supervisor, launcher, env) = setup();
        supervisor.start().unwrap();
        for (n, delay) in BACKOFF.iter().enumerate() {
            launcher.exit_last(if n == 1 { KILLED } else { CRASH });
            env.slept();
            assert_eq!(run_step(&supervisor, &env), Some(Restart::Started));
            assert_eq!(env.slept(), [*delay], "restart {}", n + 1);
            assert_eq!(launcher.count(), n + 2);
            env.advance(Duration::from_secs(10));
        }
        // The supervision ends once the new agent answers.
        supervisor.connected();
        assert_eq!(supervisor.supervision(), None);

        launcher.exit_last(CRASH);
        assert_eq!(run_step(&supervisor, &env), None);
        assert_eq!(launcher.count(), 4, "no fourth restart");
        assert_eq!(
            supervisor.supervision(),
            Some(Supervision::Stopped {
                restarts: 3,
                window_minutes: 10,
                exit: CRASH
            })
        );
        assert!(
            env.changes.load(Ordering::SeqCst) > 0,
            "the windows are told"
        );
        // Nothing more happens by itself.
        assert_eq!(run_step(&supervisor, &env), None);
        assert_eq!(launcher.count(), 4);

        // The user's Restart starts over.
        supervisor.start_by_user().unwrap();
        assert_eq!(supervisor.supervision(), None);
        launcher.exit_last(CRASH);
        env.slept();
        assert_eq!(run_step(&supervisor, &env), Some(Restart::Started));
        assert_eq!(env.slept(), [Duration::from_secs(1)]);
    }

    #[test]
    fn the_restart_is_shown_while_it_waits() {
        let (supervisor, launcher, _env) = setup();
        supervisor.start().unwrap();
        launcher.exit_last(CRASH);
        assert_eq!(
            supervisor.poll(Instant::now()),
            Action::Restart(Duration::from_secs(1))
        );
        assert_eq!(
            supervisor.supervision(),
            Some(Supervision::Restarting {
                restart: 1,
                max: 3,
                delay_ms: 1000,
                exit: CRASH
            })
        );
    }

    #[test]
    fn a_quit_or_a_requested_stop_is_never_restarted() {
        // The tray's Quit (or any stop the agent announces).
        let (supervisor, launcher, env) = setup();
        supervisor.start().unwrap();
        supervisor.stopping(launcher.last_pid(), env.now());
        launcher.exit_last(CRASH);
        assert_eq!(run_step(&supervisor, &env), None);
        assert_eq!(launcher.count(), 1);
        assert_eq!(supervisor.supervision(), None);

        // A stop this app asks for (Stop agent, a restart, an update).
        supervisor.start().unwrap();
        supervisor.expect_stop(env.now());
        launcher.exit_last(KILLED);
        assert_eq!(run_step(&supervisor, &env), None);
        assert_eq!(launcher.count(), 2);

        // Exit 0 without an announcement (another agent already ran).
        supervisor.start().unwrap();
        launcher.exit_last(CLEAN);
        assert_eq!(run_step(&supervisor, &env), None);
        assert_eq!(launcher.count(), 3);
        assert!(!supervisor.owns_running_agent());
        assert_eq!(supervisor.supervision(), None);
    }

    #[test]
    fn a_stop_that_did_not_happen_does_not_disable_restarts() {
        let (supervisor, launcher, env) = setup();
        supervisor.start().unwrap();
        // The app asked it to stop, but the request failed: still running.
        supervisor.expect_stop(env.now());
        assert_eq!(run_step(&supervisor, &env), None);
        env.advance(STOP_EXPECTED_FOR);
        assert_eq!(run_step(&supervisor, &env), None);
        // A later crash is a crash again.
        launcher.exit_last(CRASH);
        assert_eq!(run_step(&supervisor, &env), Some(Restart::Started));
    }

    #[test]
    fn skipped_restarts_do_not_use_up_the_limit() {
        let (supervisor, launcher, env) = setup();
        env.answers.store(true, Ordering::SeqCst);
        for _ in 0..5 {
            supervisor.start().unwrap();
            launcher.exit_last(CRASH);
            assert_eq!(run_step(&supervisor, &env), Some(Restart::AgentAnswers));
        }
        env.answers.store(false, Ordering::SeqCst);
        supervisor.start().unwrap();
        launcher.exit_last(CRASH);
        env.slept();
        assert_eq!(run_step(&supervisor, &env), Some(Restart::Started));
        assert_eq!(
            env.slept(),
            [Duration::from_secs(1)],
            "still the first restart"
        );
    }

    #[test]
    fn a_started_or_skipped_restart_does_not_report_the_agent_stopped() {
        let (supervisor, launcher, env) = setup();
        supervisor.start().unwrap();
        launcher.exit_last(CRASH);
        assert_eq!(run_step(&supervisor, &env), Some(Restart::Started));
        // Only the "restarting" notice before the wait.
        assert_eq!(env.changes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn an_agent_started_right_after_activation_is_supervised() {
        // main.rs activates before the first-launch start, before `run`.
        let launcher = FakeLauncher::default();
        let supervisor = Supervisor::new(Box::new(launcher.clone()));
        supervisor.activate();
        supervisor.start().unwrap();
        assert!(supervisor.owns_running_agent());
    }

    #[test]
    fn a_skipped_restart_tells_the_windows() {
        let (supervisor, launcher, env) = setup();
        supervisor.start().unwrap();
        env.update.store(true, Ordering::SeqCst);
        launcher.exit_last(CRASH);
        assert_eq!(run_step(&supervisor, &env), Some(Restart::UpdateInProgress));
        // The "restarting" notice, then the state after the skip.
        assert_eq!(env.changes.load(Ordering::SeqCst), 2);
        assert_eq!(supervisor.supervision(), None);
    }

    #[test]
    fn nothing_is_recorded_without_the_supervision_loop() {
        // A development session manages the agent: no loop runs.
        let launcher = FakeLauncher::default();
        let supervisor = Supervisor::new(Box::new(launcher.clone()));
        supervisor.start().unwrap();
        assert_eq!(launcher.count(), 1);
        assert!(!supervisor.owns_running_agent());
    }

    #[test]
    fn another_agents_stop_does_not_count_for_this_apps_agent() {
        let (supervisor, launcher, env) = setup();
        supervisor.start().unwrap();
        supervisor.stopping(launcher.last_pid() + 100, env.now());
        launcher.exit_last(CRASH);
        assert_eq!(run_step(&supervisor, &env), Some(Restart::Started));
    }

    #[test]
    fn an_agent_this_app_did_not_start_is_never_restarted() {
        // Nothing started here: an autostarted or user-launched agent may
        // come and go, the supervisor has nothing to restart.
        let (supervisor, launcher, env) = setup();
        for _ in 0..5 {
            assert_eq!(run_step(&supervisor, &env), None);
        }
        assert_eq!(launcher.count(), 0);
        assert_eq!(supervisor.supervision(), None);
    }

    #[test]
    fn nothing_is_started_while_another_agent_answers_or_starts() {
        let (supervisor, launcher, env) = setup();
        supervisor.start().unwrap();
        // An agent of any version answers (autostart, or an older one):
        // the bridge reconnects, the compatibility notice stays its own.
        env.answers.store(true, Ordering::SeqCst);
        launcher.exit_last(CRASH);
        assert_eq!(run_step(&supervisor, &env), Some(Restart::AgentAnswers));
        assert_eq!(launcher.count(), 1);
        assert_eq!(supervisor.supervision(), None);

        // An agent is starting: it holds the lock.
        env.answers.store(false, Ordering::SeqCst);
        env.lock_held.store(true, Ordering::SeqCst);
        supervisor.start().unwrap();
        launcher.exit_last(CRASH);
        assert_eq!(run_step(&supervisor, &env), Some(Restart::LockHeld));
        assert_eq!(launcher.count(), 2);
    }

    #[test]
    fn nothing_is_started_during_an_update() {
        let (supervisor, launcher, env) = setup();
        supervisor.start().unwrap();
        env.update.store(true, Ordering::SeqCst);
        launcher.exit_last(CRASH);
        assert_eq!(run_step(&supervisor, &env), Some(Restart::UpdateInProgress));
        assert_eq!(launcher.count(), 1);
        assert_eq!(supervisor.supervision(), None);
    }

    #[test]
    fn a_start_by_the_user_during_the_wait_is_not_doubled() {
        let (supervisor, launcher, env) = setup();
        supervisor.start().unwrap();
        launcher.exit_last(CRASH);
        assert!(matches!(supervisor.poll(env.now()), Action::Restart(_)));
        // The user pressed Start while the restart waited.
        supervisor.start_by_user().unwrap();
        let outcome = tauri::async_runtime::block_on(restart(&supervisor, &env));
        assert_eq!(outcome, Restart::AlreadyStarted);
        assert_eq!(launcher.count(), 2);
    }

    #[test]
    fn a_restart_that_cannot_start_the_agent_leaves_it_stopped() {
        let (supervisor, launcher, env) = setup();
        supervisor.start().unwrap();
        launcher.fail.store(true, Ordering::SeqCst);
        launcher.exit_last(CRASH);
        assert_eq!(run_step(&supervisor, &env), Some(Restart::Failed));
        assert_eq!(
            supervisor.supervision(),
            Some(Supervision::Stopped {
                restarts: 0,
                window_minutes: 10,
                exit: CRASH
            })
        );
    }

    #[test]
    fn the_supervision_is_sent_as_the_frontend_reads_it() {
        let restarting = Supervision::Restarting {
            restart: 2,
            max: 3,
            delay_ms: 5000,
            exit: KILLED,
        };
        assert_eq!(
            serde_json::to_value(restarting).unwrap(),
            serde_json::json!({
                "state": "restarting", "restart": 2, "max": 3, "delayMs": 5000,
                "exit": {"success": false, "code": null, "signal": 9}
            })
        );
        assert_eq!(
            serde_json::to_value(stopped(3, CRASH)).unwrap(),
            serde_json::json!({
                "state": "stopped", "restarts": 3, "windowMinutes": 10,
                "exit": {"success": false, "code": 101, "signal": null}
            })
        );
    }

    /// A real child process: its exit is seen without blocking, with its
    /// code (and its signal on Unix).
    #[test]
    fn a_real_process_exit_is_seen() {
        #[cfg(unix)]
        let mut command = {
            let mut command = std::process::Command::new("sh");
            command.args(["-c", "exit 101"]);
            command
        };
        #[cfg(windows)]
        let mut command = {
            let mut command = std::process::Command::new("cmd");
            command.args(["/C", "exit 101"]);
            command
        };
        let mut child: Box<dyn Process> = Box::new(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(10);
        let exit = loop {
            if let Some(exit) = child.try_wait().unwrap() {
                break exit;
            }
            assert!(Instant::now() < deadline, "the process did not exit");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(exit, CRASH);
        #[cfg(unix)]
        {
            let mut sleeper: Box<dyn Process> = Box::new(
                std::process::Command::new("sleep")
                    .arg("30")
                    .spawn()
                    .unwrap(),
            );
            assert_eq!(sleeper.try_wait().unwrap(), None);
            let pid = sleeper.id();
            std::process::Command::new("kill")
                .args(["-9", &pid.to_string()])
                .status()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            let exit = loop {
                if let Some(exit) = sleeper.try_wait().unwrap() {
                    break exit;
                }
                assert!(Instant::now() < deadline, "the killed process did not exit");
                std::thread::sleep(Duration::from_millis(20));
            };
            assert_eq!(exit, KILLED);
        }
    }
}
