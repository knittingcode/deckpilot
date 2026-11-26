//! "Wróć do pracy" ("Back to work", Return to Previous Profile): the
//! return sessions of the devices (`docs/calls.md` §Back to work).
//!
//! Pressing *Back to work* on the Calls overlay's active page **suspends**
//! the device's overlay: the device shows what it showed before the call
//! (which the overlay never changed) while the call goes on. A return
//! session decides when the overlay comes back:
//!
//! | Mode | Comes back when |
//! | --- | --- |
//! | [`ReturnMode::Timer`] | its time (20, 30 or 60 s, or 5 to 600 s) has passed |
//! | [`ReturnMode::Completion`] | a qualifying run started on the device while suspended has really ended |
//! | [`ReturnMode::CompletionOrTimeout`] (default, 60 s) | whichever comes first |
//!
//! - **One session per device.** Each suspension gets a new [`ReturnId`]
//!   (never reused); anything that carries an older one is ignored, so a
//!   stale timer or a run that ends late never restores anything.
//! - **Real completion.** A run counts when the caller reports it started
//!   from a key of the restored presentation ([`ReturnController::hold`],
//!   where the agent holds a folder for the key's runs) and ended
//!   ([`ReturnController::run_finished`], with the run's own outcome).
//!   Navigation-only keys and keys with nothing bound never count, unless
//!   the settings count navigation.
//! - **The call comes first.** A call that ends, fails or is lost ends every
//!   session without resuming ([`ReturnController::call_ended`]: the
//!   overlay is popped and the device keeps its own presentation); an
//!   incoming call preempts them ([`ReturnController::preempt`]).
//! - **No free-running timer.** Deadlines are on the engine's clock;
//!   [`ReturnController::next_wake`] says when [`ReturnController::tick`] is
//!   due. A session that ended leaves nothing scheduled.
//! - **Memory only.** After an agent restart there is no session.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use crate::clock::Instant;

#[cfg(test)]
#[path = "ret/tests.rs"]
mod tests;

/// The timer presets of [`ReturnMode::Timer`], in seconds.
pub const TIMER_PRESETS: [u64; 3] = [20, 30, 60];
/// The shortest and longest custom timer, in seconds.
pub const TIMER_RANGE: (u64, u64) = (5, 600);
/// The shortest and longest maximum of [`ReturnMode::CompletionOrTimeout`],
/// in seconds.
pub const MAX_TIMEOUT_RANGE: (u64, u64) = (10, 1800);
/// The default maximum of [`ReturnMode::CompletionOrTimeout`], in seconds.
pub const DEFAULT_MAX_TIMEOUT: u64 = 60;
/// Most runs one session follows (a key that starts more is followed by
/// its first ones).
pub const MAX_HOLDS: usize = 16;

/// The generation token of one return session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReturnId(pub u64);

/// The caller's id of one run (an action, a workflow run).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunToken(pub u64);

/// When a suspended overlay comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnMode {
    /// Mode A: after this time.
    Timer { after: Duration },
    /// Mode B: when a qualifying run ends (or the call does, or the user
    /// shows Calls again).
    Completion,
    /// Mode C, the default: whichever comes first of a qualifying run's
    /// end and `max`.
    CompletionOrTimeout { max: Duration },
}

/// A value outside its range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{seconds} s is outside {min} to {max} s")]
pub struct OutOfRange {
    pub seconds: u64,
    pub min: u64,
    pub max: u64,
}

impl ReturnMode {
    /// Mode A with a preset or a custom time of [`TIMER_RANGE`].
    pub fn timer(seconds: u64) -> Result<Self, OutOfRange> {
        check(seconds, TIMER_RANGE)?;
        Ok(Self::Timer {
            after: Duration::from_secs(seconds),
        })
    }

    /// Mode C with a maximum of [`MAX_TIMEOUT_RANGE`].
    pub fn completion_or_timeout(seconds: u64) -> Result<Self, OutOfRange> {
        check(seconds, MAX_TIMEOUT_RANGE)?;
        Ok(Self::CompletionOrTimeout {
            max: Duration::from_secs(seconds),
        })
    }

    /// The mode with its time brought into its range ([`TIMER_RANGE`],
    /// [`MAX_TIMEOUT_RANGE`]): the variants' fields are public, so a mode
    /// built without [`ReturnMode::timer`] or
    /// [`ReturnMode::completion_or_timeout`] is clamped where it is used.
    #[must_use]
    pub fn clamped(self) -> Self {
        let clamp = |d: Duration, (min, max): (u64, u64)| {
            d.clamp(Duration::from_secs(min), Duration::from_secs(max))
        };
        match self {
            Self::Timer { after } => Self::Timer {
                after: clamp(after, TIMER_RANGE),
            },
            Self::CompletionOrTimeout { max } => Self::CompletionOrTimeout {
                max: clamp(max, MAX_TIMEOUT_RANGE),
            },
            Self::Completion => Self::Completion,
        }
    }

    fn deadline(self, now: Instant) -> Option<Instant> {
        match self {
            Self::Timer { after } => Some(now.after(after)),
            Self::CompletionOrTimeout { max } => Some(now.after(max)),
            Self::Completion => None,
        }
    }

    fn waits_for_runs(self) -> bool {
        !matches!(self, Self::Timer { .. })
    }
}

impl Default for ReturnMode {
    fn default() -> Self {
        Self::CompletionOrTimeout {
            max: Duration::from_secs(DEFAULT_MAX_TIMEOUT),
        }
    }
}

fn check(seconds: u64, (min, max): (u64, u64)) -> Result<(), OutOfRange> {
    if (min..=max).contains(&seconds) {
        Ok(())
    } else {
        Err(OutOfRange { seconds, min, max })
    }
}

/// What a key started, as far as a return session cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunKind {
    /// An action run in place.
    Action,
    /// A plugin action run detached (its `DetachedFinished` is its end).
    DetachedAction,
    /// A workflow run (its `Execution::Finished` is its end).
    Workflow,
    /// A Multi Action (it ends when its last step does).
    MultiAction,
    /// Opening or leaving a page, switching a layout or profile.
    Navigation,
    /// The key has nothing bound.
    Nothing,
}

/// Which runs count (the default: actions, workflows and Multi Actions,
/// not navigation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct Qualify {
    pub actions: bool,
    pub workflows: bool,
    pub multi_actions: bool,
    pub navigation: bool,
}

impl Default for Qualify {
    fn default() -> Self {
        Self {
            actions: true,
            workflows: true,
            multi_actions: true,
            navigation: false,
        }
    }
}

impl Qualify {
    pub fn counts(self, kind: RunKind) -> bool {
        match kind {
            RunKind::Action | RunKind::DetachedAction => self.actions,
            RunKind::Workflow => self.workflows,
            RunKind::MultiAction => self.multi_actions,
            RunKind::Navigation => self.navigation,
            RunKind::Nothing => false,
        }
    }
}

/// What a session does when a qualifying run fails or is cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterRun {
    /// Show the call again.
    Resume,
    /// Keep the device on its own presentation (until the maximum of mode
    /// C or the next qualifying run's end).
    Stay,
}

/// The settings of *Back to work* (ordinary action parameters).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnSettings {
    pub mode: ReturnMode,
    pub qualify: Qualify,
    /// Default: resume (and the failure shows on the restored key).
    pub on_failure: AfterRun,
    /// Default: stay (a cancelled run did not complete the user's intent).
    pub on_cancel: AfterRun,
    /// Suspend every device that shows the overlay (default off).
    pub all_devices: bool,
}

impl Default for ReturnSettings {
    fn default() -> Self {
        Self {
            mode: ReturnMode::default(),
            qualify: Qualify::default(),
            on_failure: AfterRun::Resume,
            on_cancel: AfterRun::Stay,
            all_devices: false,
        }
    }
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    Succeeded,
    Failed,
    Cancelled,
    /// The agent stopped: the run's end is unknown and counts for nothing.
    Interrupted,
}

/// Why a return session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnReason {
    /// Mode A's time passed.
    TimerElapsed,
    /// Mode C's maximum passed.
    MaxTimeout,
    RunSucceeded,
    RunFailed,
    RunCancelled,
    /// The user showed Calls again.
    UserResumed,
    /// A new incoming call takes the device.
    Preempted,
    /// The call ended, failed or was lost: the overlay goes.
    CallEnded,
    /// The overlay went another way (dismissed, popped): nothing to resume.
    Dropped,
}

/// The end of one return session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnEnded<D> {
    pub device: D,
    pub id: ReturnId,
    pub reason: ReturnReason,
}

impl<D> ReturnEnded<D> {
    /// Whether the overlay is shown again (as opposed to popped or left
    /// as it is).
    pub fn resumes(&self) -> bool {
        !matches!(self.reason, ReturnReason::CallEnded | ReturnReason::Dropped)
    }
}

/// One device's return session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnSession {
    pub id: ReturnId,
    pub settings: ReturnSettings,
    pub started: Instant,
    pub deadline: Option<Instant>,
    /// The runs that count, started from the restored presentation.
    pub holds: BTreeSet<RunToken>,
}

/// The return sessions of every device, at most one each.
#[derive(Debug, Clone)]
pub struct ReturnController<D: Ord + Clone> {
    sessions: BTreeMap<D, ReturnSession>,
    next_id: u64,
}

impl<D: Ord + Clone> Default for ReturnController<D> {
    fn default() -> Self {
        Self {
            sessions: BTreeMap::new(),
            next_id: 1,
        }
    }
}

impl<D: Ord + Clone> ReturnController<D> {
    /// Suspends `device`'s overlay with `settings`. A session the device had
    /// is replaced (its id is stale from now on) and returned as ended,
    /// [`ReturnReason::Dropped`].
    pub fn suspend(
        &mut self,
        device: D,
        settings: ReturnSettings,
        now: Instant,
    ) -> (ReturnId, Option<ReturnEnded<D>>) {
        let mut settings = settings;
        settings.mode = settings.mode.clamped();
        let id = ReturnId(self.next_id);
        self.next_id += 1;
        let session = ReturnSession {
            id,
            settings,
            started: now,
            deadline: settings.mode.deadline(now),
            holds: BTreeSet::new(),
        };
        let replaced = self
            .sessions
            .insert(device.clone(), session)
            .map(|old| ReturnEnded {
                device,
                id: old.id,
                reason: ReturnReason::Dropped,
            });
        (id, replaced)
    }

    /// The session of `device`.
    pub fn session(&self, device: &D) -> Option<&ReturnSession> {
        self.sessions.get(device)
    }

    /// The current session id of `device`.
    pub fn current(&self, device: &D) -> Option<ReturnId> {
        self.sessions.get(device).map(|s| s.id)
    }

    /// Whether `device` is suspended.
    pub fn is_suspended(&self, device: &D) -> bool {
        self.sessions.contains_key(device)
    }

    /// A key of `device`'s restored presentation started `run` while
    /// session `id` lasted. `true` when the run counts for the session
    /// (its mode waits for runs, `kind` qualifies, `id` is current).
    pub fn hold(&mut self, device: &D, id: ReturnId, run: RunToken, kind: RunKind) -> bool {
        let Some(session) = self.sessions.get_mut(device) else {
            return false;
        };
        if session.id != id
            || !session.settings.mode.waits_for_runs()
            || !session.settings.qualify.counts(kind)
            || session.holds.len() >= MAX_HOLDS
        {
            return false;
        }
        session.holds.insert(run);
        true
    }

    /// `run` ended. Resumes `device` when the run counted for its current
    /// session and its end resumes (success always; failure and
    /// cancellation as the settings say; an interruption never). A run of
    /// an older session, or one that never counted, changes nothing.
    pub fn run_finished(
        &mut self,
        device: &D,
        run: RunToken,
        end: RunEnd,
    ) -> Option<ReturnEnded<D>> {
        let session = self.sessions.get_mut(device)?;
        if !session.holds.remove(&run) {
            return None;
        }
        let reason = match end {
            RunEnd::Succeeded => ReturnReason::RunSucceeded,
            RunEnd::Failed if session.settings.on_failure == AfterRun::Resume => {
                ReturnReason::RunFailed
            }
            RunEnd::Cancelled if session.settings.on_cancel == AfterRun::Resume => {
                ReturnReason::RunCancelled
            }
            RunEnd::Failed | RunEnd::Cancelled | RunEnd::Interrupted => return None,
        };
        self.end(device, reason)
    }

    /// The user showed Calls again on `device` (the *Show calls* key).
    pub fn user_resumed(&mut self, device: &D) -> Option<ReturnEnded<D>> {
        self.end(device, ReturnReason::UserResumed)
    }

    /// A new incoming call takes every suspended device.
    pub fn preempt(&mut self) -> Vec<ReturnEnded<D>> {
        self.end_all(ReturnReason::Preempted)
    }

    /// The call ended, failed or was lost: every session ends without
    /// resuming.
    pub fn call_ended(&mut self) -> Vec<ReturnEnded<D>> {
        self.end_all(ReturnReason::CallEnded)
    }

    /// The overlay of `device` went another way (dismissed, popped).
    pub fn drop_device(&mut self, device: &D) -> Option<ReturnEnded<D>> {
        self.end(device, ReturnReason::Dropped)
    }

    /// Ends every session without resuming (the plugin disabled, consent
    /// withdrawn).
    pub fn clear(&mut self) -> Vec<ReturnEnded<D>> {
        self.end_all(ReturnReason::Dropped)
    }

    fn end(&mut self, device: &D, reason: ReturnReason) -> Option<ReturnEnded<D>> {
        self.sessions.remove(device).map(|session| ReturnEnded {
            device: device.clone(),
            id: session.id,
            reason,
        })
    }

    fn end_all(&mut self, reason: ReturnReason) -> Vec<ReturnEnded<D>> {
        std::mem::take(&mut self.sessions)
            .into_iter()
            .map(|(device, session)| ReturnEnded {
                device,
                id: session.id,
                reason,
            })
            .collect()
    }

    /// When [`ReturnController::tick`] is due next.
    pub fn next_wake(&self) -> Option<Instant> {
        self.sessions.values().filter_map(|s| s.deadline).min()
    }

    /// How many deadlines are scheduled: at most one per device.
    pub fn scheduled(&self) -> usize {
        self.sessions
            .values()
            .filter(|s| s.deadline.is_some())
            .count()
    }

    /// Ends every session whose deadline passed at `now`.
    pub fn tick(&mut self, now: Instant) -> Vec<ReturnEnded<D>> {
        let due: Vec<D> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.deadline.is_some_and(|d| now >= d))
            .map(|(device, _)| device.clone())
            .collect();
        due.into_iter()
            .filter_map(|device| {
                let reason = match self.sessions.get(&device)?.settings.mode {
                    ReturnMode::Timer { .. } => ReturnReason::TimerElapsed,
                    _ => ReturnReason::MaxTimeout,
                };
                self.end(&device, reason)
            })
            .collect()
    }
}
