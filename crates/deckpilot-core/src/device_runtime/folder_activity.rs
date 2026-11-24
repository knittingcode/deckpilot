//! Runs started from a folder's keys, and the folder's countdown on its
//! protected Back key (`docs/data-model.md`, "Auto Exit" and "Exit after
//! action completes").
//!
//! The runtime does not know when what a key started is over: a plugin's
//! action and a workflow run elsewhere. The caller that runs them (the
//! agent) tells it: it notes the folder entry a key belongs to when it
//! handles the key ([`Runtime::folder_entry`]), holds it for every run the
//! key started that ends later ([`Runtime::hold_folder_entry`]: a
//! workflow's execution, a plugin's action; the countdown stops at once),
//! releases each hold when its run reached its end
//! ([`Runtime::release_folder_hold`]) and reports an action that ran in
//! place ([`Runtime::folder_action_ran`]). Once the last hold is released
//! the countdown goes on from what was left, or, when one of the runs ran
//! and the folder has
//! [`crate::LayoutPage::auto_return`] ("Exit after action completes"), the
//! device leaves the folder as Back would. Plugins never see any of this:
//! their actions take part through the run the agent holds for them.

use std::time::Instant;

use deckpilot_device::DeviceId;

use super::{Runtime, RuntimeOutcome, system_back_face};
use crate::{FolderCountdown, FolderExit, FolderHold, Glyph, HoldEnd, KeyFace};

impl Runtime {
    /// Now, on the clock countdowns are measured with.
    pub fn folder_clock(&self) -> Instant {
        (self.clock.0)()
    }

    /// When the agent's loop next has to look at the folders: a countdown
    /// ends, or the seconds a Back key shows change.
    pub fn next_folder_wake(&self) -> Option<Instant> {
        self.folders.next_wake(self.folder_clock())
    }

    /// The countdown of the folder `device` shows, as its Back key shows
    /// it now. `None`: no folder, or its Auto Exit is off.
    pub fn folder_countdown(&self, device: &DeviceId) -> Option<FolderCountdown> {
        self.folders.countdown(device, self.folder_clock())
    }

    /// The entry of the folder `device` shows (`None`: no folder): taken
    /// when a key is handled, so what the key starts can hold that entry
    /// ([`Runtime::hold_folder_entry`]) even after the key navigated.
    pub fn folder_entry(&self, device: &DeviceId) -> Option<FolderExit> {
        self.folders.entry_of(device)
    }

    /// A run that `entry`'s key started and that ends later begins (a
    /// workflow's execution, a plugin's action): the folder's countdown
    /// stops at once, keeping what is left, until the hold is released.
    /// `None` when the device left that entry.
    pub fn hold_folder_entry(&mut self, entry: &FolderExit) -> Option<FolderHold> {
        let now = self.folder_clock();
        self.folders.hold_entry(entry, now)
    }

    /// [`Runtime::hold_folder_entry`] of the folder `device` shows.
    pub fn hold_folder(&mut self, device: &DeviceId) -> Option<FolderHold> {
        let page = self.current_page(device)?;
        let now = self.folder_clock();
        self.folders.hold(device, page, now)
    }

    /// A further run of the entry `hold` belongs to, released on its own.
    /// `None` when the device left that entry.
    pub fn fork_folder_hold(&mut self, hold: &FolderHold) -> Option<FolderHold> {
        let now = self.folder_clock();
        self.folders.fork(hold, now)
    }

    /// A key of `entry` ran an action in place, to its end. Without runs
    /// of the entry still going, a folder with
    /// [`crate::LayoutPage::auto_return`] that is still shown is left
    /// (`Some(Navigated)`); with runs going, it is left when the last of
    /// them is over.
    pub fn folder_action_ran(&mut self, entry: &FolderExit) -> Option<RuntimeOutcome> {
        let finished = self.folders.ran(entry)?;
        self.leave_finished_folder(&finished)
    }

    /// The run of `hold` is over (`end`). When it was the folder's last
    /// one and one of them ran, a folder with
    /// [`crate::LayoutPage::auto_return`] is left as Back from it would
    /// (`Some(Navigated)`); otherwise its countdown goes on from what was
    /// left. A hold of an entry that was left (or entered again) does
    /// nothing.
    pub fn release_folder_hold(
        &mut self,
        hold: &FolderHold,
        end: HoldEnd,
    ) -> Option<RuntimeOutcome> {
        let now = self.folder_clock();
        let finished = self.folders.release(hold, end, now)?;
        self.leave_finished_folder(&finished)
    }

    /// Leaves the folder entry whose runs are over when the folder exits
    /// after its actions complete.
    fn leave_finished_folder(&mut self, finished: &FolderExit) -> Option<RuntimeOutcome> {
        let exits = self
            .layout_of(&finished.device)
            .filter(|layout| layout.id == finished.layout)
            .and_then(|layout| layout.page(finished.page))
            .is_some_and(|page| page.auto_return);
        if exits {
            self.exit_folder(finished)
        } else {
            None
        }
    }

    /// Draws the folder's countdown on the protected Back key in `faces`
    /// of `device` ([`folder_back_face`]).
    pub(super) fn show_folder_countdown(
        &self,
        device: &DeviceId,
        faces: &mut [(deckpilot_device::ControlId, KeyFace)],
    ) {
        let (Some(back), Some(countdown)) =
            (self.system_back(device), self.folder_countdown(device))
        else {
            return;
        };
        if let Some((_, face)) = faces.iter_mut().find(|(control, _)| *control == back) {
            *face = folder_back_face(Some(countdown));
        }
    }
}

/// The face of a folder's protected Back key with its Auto Exit countdown:
/// the Back icon with the whole seconds left under it, and the pause mark
/// in the corner while a run started from the folder holds it. Without a
/// countdown, the plain [`system_back_face`]. Drawn by the same renderer
/// for the device, the simulated device and the editor's Canvas.
pub fn folder_back_face(countdown: Option<FolderCountdown>) -> KeyFace {
    let face = system_back_face();
    let Some(countdown) = countdown else {
        return face;
    };
    let mut face = KeyFace {
        title: countdown.seconds.to_string(),
        ..face
    };
    if countdown.paused {
        face.glyph = Some(Glyph::Pause);
    }
    face
}
