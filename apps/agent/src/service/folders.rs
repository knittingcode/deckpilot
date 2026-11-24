//! Auto Exit and Exit after action completes of folders on the service
//! task (`docs/data-model.md`, "Auto Exit").
//!
//! The countdowns live in the core's runtime ([`deckpilot_core::FolderTimers`]),
//! per device, in memory only: the agent runs them with the desktop app
//! closed, and a restart starts none. The run loop sleeps until the next
//! one ends or the seconds a folder's Back key shows change (once a
//! second, and only while a countdown runs), like gesture deadlines:
//! nothing is spawned and input is never held up. An exit applies only
//! while its entry is still the device's current one (its generation), so
//! a countdown of a folder that was left, or entered again, does nothing.
//! A device that leaves its folder this way is shown and published as
//! after a key that went back.
//!
//! **The runs of a folder's keys.** While a key is handled
//! ([`Agent::with_folder_scope`]) the agent notes the folder entry the key
//! belongs to. Every run the key starts that ends later holds that entry
//! with an id of its own: a workflow's execution (also a Multi Action, a
//! switch's or a pick's workflow; by its [`ExecutionId`]) and a plugin's
//! action (its detached run). The countdown stops when the first starts
//! and goes on, from what was left, when the last one reached its end:
//! finished, failed or cancelled. An action that ran in place is reported
//! when the key's handling is over. A folder that exits after its actions
//! complete is then left, once. Runs that could not start release their
//! hold without counting as run, so the device stays in the folder.
//! Plugins take part through these runs only; they have no say in the
//! navigation.

use std::collections::BTreeMap;
use std::time::Duration;

use deckpilot_core::{ExecutionId, FolderExit, FolderHold, HoldEnd, RuntimeOutcome};
use deckpilot_device::DeviceId;
use tracing::debug;

use super::Agent;

/// The folder a key being handled belongs to, and the runs of folders'
/// keys that are not over.
#[derive(Debug, Default)]
pub(crate) struct FolderRuns {
    /// The folder entry of the key being handled, while it is handled.
    scope: Option<FolderExit>,
    /// An action of that key ran in place.
    ran: bool,
    /// Workflow runs started from a folder's key, by execution.
    pub(super) executions: BTreeMap<ExecutionId, FolderHold>,
    /// When the loop last planned to look at the folders.
    planned: Option<std::time::Instant>,
}

impl Agent {
    /// How long until the loop next has to look at the folders, if a
    /// countdown runs: on the runtime's clock, which tests move. A look
    /// that was due already (the clock moved past it while the loop
    /// handled something else) is taken first, so no second is skipped.
    ///
    /// The next look is planned from the same `now` the due one was
    /// checked against: planned from a later reading, a second that began
    /// in between would be planned past without ever being drawn, and only
    /// a look already due catches it up.
    pub(super) fn next_folder_wake(&mut self) -> Option<Duration> {
        let now = self.runtime.folder_clock();
        if self
            .folder_runs
            .planned
            .is_some_and(|planned| planned <= now)
        {
            self.on_folder_wake();
        }
        let wake = self.runtime.folder_timers().next_wake(now);
        self.folder_runs.planned = wake;
        Some(wake?.saturating_duration_since(self.runtime.folder_clock()))
    }

    /// A countdown ended, or the seconds a Back key shows changed: devices
    /// leave their folders, or the Back key is redrawn (only the keys
    /// whose face changed are written).
    pub(super) fn on_folder_wake(&mut self) {
        if self.exit_folders_at(self.runtime.folder_clock()) == 0 {
            self.render_all();
        }
    }

    /// Applies the exits due at `now`; returns how many devices left a
    /// folder.
    pub(super) fn exit_folders_at(&mut self, now: std::time::Instant) -> usize {
        let mut left = 0;
        for exit in self.runtime.due_folder_exits(now) {
            if let Some(RuntimeOutcome::Navigated { device, page }) =
                self.runtime.exit_folder(&exit)
            {
                debug!(%device, %page, "auto exit");
                self.gestures.page_changed(&device);
                left += 1;
            }
        }
        if left > 0 {
            self.after_change();
        }
        left
    }

    /// Handles one key of `device` in `handle`, noting the folder it
    /// belongs to: the runs it starts hold that folder, and an action that
    /// ran in place is reported once it is handled. A key handled inside
    /// another's handling (a pick, a switch's state) belongs to the outer
    /// key.
    pub(super) fn with_folder_scope<T>(
        &mut self,
        device: &DeviceId,
        handle: impl FnOnce(&mut Self) -> T,
    ) -> T {
        if self.folder_runs.scope.is_some() {
            return handle(self);
        }
        self.folder_runs.scope = self.runtime.folder_entry(device);
        self.folder_runs.ran = false;
        let handled = handle(self);
        let entry = self.folder_runs.scope.take();
        let ran = std::mem::take(&mut self.folder_runs.ran);
        if let Some(entry) = entry.filter(|_| ran) {
            let outcome = self.runtime.folder_action_ran(&entry);
            self.folder_left(outcome);
        }
        handled
    }

    /// An action of the key being handled ran in place.
    pub(super) fn folder_action_ran(&mut self) {
        self.folder_runs.ran = true;
    }

    /// A run of the key being handled starts and ends later: it holds the
    /// key's folder (`None`: no key of a folder is being handled, or the
    /// device left the folder).
    pub(super) fn hold_folder_for_run(&mut self) -> Option<FolderHold> {
        let entry = self.folder_runs.scope.clone()?;
        self.runtime.hold_folder_entry(&entry)
    }

    /// The run of `hold` is over (`end`): its folder goes on counting down
    /// or, when that was its last run, is left if it exits after its
    /// actions complete.
    pub(super) fn release_folder_run(&mut self, hold: Option<FolderHold>, end: HoldEnd) {
        let Some(hold) = hold else {
            return;
        };
        let outcome = self.runtime.release_folder_hold(&hold, end);
        if !self.folder_left(outcome) {
            // Resumed: the Back key shows it.
            self.render_all();
        }
    }

    /// The workflow run `execution` is over, whatever its end.
    pub(super) fn execution_left_folder(&mut self, execution: ExecutionId) {
        let hold = self.folder_runs.executions.remove(&execution);
        self.release_folder_run(hold, HoldEnd::Ran);
    }

    /// Shows a device that left its folder after its runs; returns whether
    /// one did.
    fn folder_left(&mut self, outcome: Option<RuntimeOutcome>) -> bool {
        let Some(RuntimeOutcome::Navigated { device, page }) = outcome else {
            return false;
        };
        debug!(%device, %page, "exit after action completes");
        self.gestures.page_changed(&device);
        self.after_change();
        true
    }
}
