//! The folders each device is in: their Auto Exit countdowns and the runs
//! started from their keys (`docs/data-model.md`, "Auto Exit" and "Exit
//! after action completes").
//!
//! - **One countdown per device, the active folder's.** A device is in a
//!   chain of folders (the folder it shows and the folders around it);
//!   only the innermost one, the folder it shows, counts down. A folder
//!   around it keeps what was left of its time and goes on from there when
//!   the device comes back to it, so a folder that is not shown never
//!   expires in the background.
//! - **Runs hold it.** A run started from a key of the folder the device
//!   shows ([`FolderTimers::hold`]) stops its countdown at once, keeping
//!   what was left; the time a run takes is not counted. Each run has its
//!   own hold id (a workflow's execution, a plugin's action, the key's own
//!   handling), so the countdown goes on only when every one of them is
//!   over ([`FolderTimers::release`]); one of several ending changes
//!   nothing.
//! - **Exit after completion.** When the last hold of a folder is released
//!   and one of them ran ([`HoldEnd::Ran`]: success, a handled failure or
//!   a cancellation; not a run that could not start), the folder is
//!   reported as finished; the runtime leaves it when it has
//!   [`crate::LayoutPage::auto_return`], else its countdown goes on from
//!   what was left.
//! - **Generations.** Every entry into a folder has a generation of its
//!   own; exits and holds carry it, so a countdown or a run of an entry
//!   that was left (Back, another folder, page, layout or profile, a
//!   disconnect, locking) can never navigate.
//!
//! Deadlines are measured on a monotonic clock ([`Instant`]) and live in
//! memory only.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use deckpilot_device::DeviceId;

use crate::{DeviceLayout, LayoutId, PageId};

/// One folder entry of a device: a due Auto Exit, what
/// [`FolderTimers::due`] hands out and [`crate::Runtime::exit_folder`]
/// takes back, or the folder a finished run leaves. `generation` is the
/// entry's own: an exit whose entry was left or entered again since does
/// nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderExit {
    pub device: DeviceId,
    pub layout: LayoutId,
    pub page: PageId,
    pub generation: u64,
}

/// A run that holds the countdown of a folder entry: taken when a key of
/// the folder started it ([`crate::Runtime::hold_folder`]), released when
/// it is over ([`crate::Runtime::release_folder_hold`]). Every hold is
/// released once; a hold of an entry that is gone is released without
/// effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderHold {
    pub entry: FolderExit,
    pub id: u64,
}

/// How a held run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldEnd {
    /// It ran to its end: success, a handled failure or a cancellation.
    Ran,
    /// It did not run: it could not start, or nothing was started (a key
    /// that only navigated, or was not bound).
    NotRun,
}

/// What the protected Back key of the folder a device shows says about its
/// Auto Exit: the whole seconds left (rounded up, so 10 s show `10` from
/// the start and `1` in the last second) and whether it is paused while a
/// run started from the folder is not over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderCountdown {
    pub seconds: u32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub paused: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Countdown {
    /// No Auto Exit for this entry.
    Off,
    /// Counting down to `due`.
    Running { due: Instant },
    /// Stopped with `left` to go: a run holds it, or the folder is not the
    /// one shown.
    Stopped { left: Duration },
}

/// One folder a device is in, since it entered it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FolderEntry {
    page: PageId,
    generation: u64,
    countdown: Countdown,
    /// The runs started from its keys that are not over.
    holds: BTreeSet<u64>,
    /// One of the runs released since it was last idle ran.
    ran: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeviceFolders {
    layout: LayoutId,
    /// The folders that contain the shown page, outermost first; the last
    /// is the folder shown.
    entries: Vec<FolderEntry>,
}

impl DeviceFolders {
    /// Runs the countdown of the folder shown while nothing holds it, and
    /// stops every other one where it is.
    fn settle(&mut self, now: Instant) {
        let last = self.entries.len().saturating_sub(1);
        for (index, entry) in self.entries.iter_mut().enumerate() {
            let active = index == last && entry.holds.is_empty();
            entry.countdown = match (entry.countdown, active) {
                (Countdown::Running { due }, false) => Countdown::Stopped {
                    left: due.saturating_duration_since(now),
                },
                (Countdown::Stopped { left }, true) => Countdown::Running { due: now + left },
                (countdown, _) => countdown,
            };
        }
    }

    fn entry_mut(&mut self, exit: &FolderExit) -> Option<(usize, &mut FolderEntry)> {
        if self.layout != exit.layout {
            return None;
        }
        self.entries
            .iter_mut()
            .enumerate()
            .find(|(_, entry)| entry.page == exit.page && entry.generation == exit.generation)
    }
}

/// Whole seconds in `left`, rounded up.
fn whole_seconds(left: Duration) -> u64 {
    left.as_secs() + u64::from(left.subsec_nanos() > 0)
}

/// When the seconds a countdown to `due` shows at `now` change next: at
/// `due` itself in its last second.
fn next_tick(due: Instant, now: Instant) -> Instant {
    let shown = whole_seconds(due.saturating_duration_since(now));
    if shown <= 1 {
        return due;
    }
    due.checked_sub(Duration::from_secs(shown - 1))
        .unwrap_or(due)
}

/// The folders of the devices and their countdowns, per device (devices
/// that show the same layout have their own).
///
/// A device **enters** a folder when the page it shows comes to be inside
/// the folder (the folder page itself or any page inside it) and was not
/// before: from the folder's parent, from anywhere else, by any navigation.
/// Entering starts its countdown with the folder's whole time, unless
/// `quiet` (a reconnect, a restored page, a reload) enters it without one.
/// Leaving the folder (Back, Home, another layout or profile, locking,
/// disconnecting) cancels it, and entering again starts it anew. See the
/// module documentation for which countdown runs and what holds it.
#[derive(Debug, Default)]
pub struct FolderTimers {
    devices: BTreeMap<DeviceId, DeviceFolders>,
    next_generation: u64,
    next_hold: u64,
}

impl FolderTimers {
    /// Follows what `device` shows now: `page` of `layout` (`None`:
    /// nothing). Folders it left are cancelled with their holds; folders
    /// it entered start their countdown at `now` unless `quiet`, which
    /// takes them as they are without starting any.
    pub(crate) fn follow(
        &mut self,
        device: &DeviceId,
        shown: Option<(&DeviceLayout, PageId)>,
        now: Instant,
        quiet: bool,
    ) {
        let Some((layout, page)) = shown else {
            self.devices.remove(device);
            return;
        };
        let parents = layout.page_parents();
        // The shown page and its folders, outermost first; only pages
        // inside a folder are folders one can leave.
        let mut chain: Vec<PageId> = Vec::new();
        let mut current = Some(page);
        while let Some(id) = current {
            if chain.contains(&id) || !parents.contains_key(&id) {
                break;
            }
            chain.push(id);
            current = parents.get(&id).copied();
        }
        chain.reverse();
        let previous = self
            .devices
            .remove(device)
            .filter(|state| state.layout == layout.id)
            .map(|state| state.entries)
            .unwrap_or_default();
        let mut entries = Vec::with_capacity(chain.len());
        for folder in chain {
            if let Some(kept) = previous.iter().find(|entry| entry.page == folder) {
                // Still inside: its countdown and its runs go on, unless
                // Auto Exit was switched off meanwhile.
                let mut kept = kept.clone();
                if layout.auto_exit(folder).is_none() {
                    kept.countdown = Countdown::Off;
                }
                entries.push(kept);
                continue;
            }
            self.next_generation += 1;
            let countdown = match layout.auto_exit(folder) {
                Some(left) if !quiet => Countdown::Stopped { left },
                _ => Countdown::Off,
            };
            entries.push(FolderEntry {
                page: folder,
                generation: self.next_generation,
                countdown,
                holds: BTreeSet::new(),
                ran: false,
            });
        }
        if !entries.is_empty() {
            let mut state = DeviceFolders {
                layout: layout.id,
                entries,
            };
            state.settle(now);
            self.devices.insert(device.clone(), state);
        }
    }

    /// Forgets `device` (disconnected): its countdowns and holds are
    /// cancelled.
    pub(crate) fn forget(&mut self, device: &DeviceId) {
        self.devices.remove(device);
    }

    /// Cancels every countdown and hold (profile changed, locked).
    pub(crate) fn clear(&mut self) {
        self.devices.clear();
    }

    /// When the next countdown ends.
    pub fn next_due(&self) -> Option<Instant> {
        self.running().min()
    }

    /// When something next has to happen at `now` or later: a countdown
    /// ends, or the whole seconds a running countdown shows change (once a
    /// second, so the Back key is redrawn only then).
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        self.running().map(|due| next_tick(due, now)).min()
    }

    fn running(&self) -> impl Iterator<Item = Instant> + '_ {
        self.devices.values().flat_map(|state| {
            state
                .entries
                .iter()
                .filter_map(|entry| match entry.countdown {
                    Countdown::Running { due } => Some(due),
                    _ => None,
                })
        })
    }

    /// The exits due at `now`: per device the folder whose countdown ended
    /// (only the folder shown counts down; leaving it leaves the folders
    /// inside it too).
    pub fn due(&self, now: Instant) -> Vec<FolderExit> {
        self.devices
            .iter()
            .filter_map(|(device, state)| {
                let entry = state.entries.iter().find(
                    |entry| matches!(entry.countdown, Countdown::Running { due } if due <= now),
                )?;
                Some(FolderExit {
                    device: device.clone(),
                    layout: state.layout,
                    page: entry.page,
                    generation: entry.generation,
                })
            })
            .collect()
    }

    /// Whether `exit` is still the device's current entry of its folder.
    pub(crate) fn is_current(&self, exit: &FolderExit) -> bool {
        self.devices.get(&exit.device).is_some_and(|state| {
            state.layout == exit.layout
                && state
                    .entries
                    .iter()
                    .any(|entry| entry.page == exit.page && entry.generation == exit.generation)
        })
    }

    /// When the countdown of the folder `page` that `device` is in ends.
    /// `None`: not in it, or its countdown does not run (off, held, or not
    /// the folder shown).
    pub fn due_of(&self, device: &DeviceId, page: PageId) -> Option<Instant> {
        match self
            .devices
            .get(device)?
            .entries
            .iter()
            .find(|entry| entry.page == page)?
            .countdown
        {
            Countdown::Running { due } => Some(due),
            _ => None,
        }
    }

    /// What is left of the countdown of the folder `page` that `device` is
    /// in, at `now`. `None`: not in it, or Auto Exit is off for it.
    pub fn left_of(&self, device: &DeviceId, page: PageId, now: Instant) -> Option<Duration> {
        match self
            .devices
            .get(device)?
            .entries
            .iter()
            .find(|entry| entry.page == page)?
            .countdown
        {
            Countdown::Off => None,
            Countdown::Running { due } => Some(due.saturating_duration_since(now)),
            Countdown::Stopped { left } => Some(left),
        }
    }

    /// How many runs hold the folder `page` that `device` is in.
    pub fn holds_of(&self, device: &DeviceId, page: PageId) -> usize {
        self.devices
            .get(device)
            .and_then(|state| state.entries.iter().find(|entry| entry.page == page))
            .map_or(0, |entry| entry.holds.len())
    }

    /// The countdown of the folder `device` shows, as its Back key shows
    /// it at `now`. `None`: it shows no folder, or the folder has no Auto
    /// Exit.
    pub fn countdown(&self, device: &DeviceId, now: Instant) -> Option<FolderCountdown> {
        let entry = self.devices.get(device)?.entries.last()?;
        let (left, paused) = match entry.countdown {
            Countdown::Off => return None,
            Countdown::Running { due } => (due.saturating_duration_since(now), false),
            Countdown::Stopped { left } => (left, !entry.holds.is_empty()),
        };
        Some(FolderCountdown {
            seconds: u32::try_from(whole_seconds(left)).unwrap_or(u32::MAX),
            paused,
        })
    }

    /// The entry of the folder `device` shows: what a key handled now
    /// belongs to. `None` when it shows no folder.
    pub(crate) fn entry_of(&self, device: &DeviceId) -> Option<FolderExit> {
        let state = self.devices.get(device)?;
        let entry = state.entries.last()?;
        Some(FolderExit {
            device: device.clone(),
            layout: state.layout,
            page: entry.page,
            generation: entry.generation,
        })
    }

    /// A run of the folder entry `entry` starts: its countdown stops at
    /// once, keeping what is left, until the hold is released. `None`
    /// when the entry is gone (the device left it, or entered it again).
    pub(crate) fn hold_entry(&mut self, entry: &FolderExit, now: Instant) -> Option<FolderHold> {
        let state = self.devices.get_mut(&entry.device)?;
        let (_, held) = state.entry_mut(entry)?;
        self.next_hold += 1;
        held.holds.insert(self.next_hold);
        state.settle(now);
        Some(FolderHold {
            entry: entry.clone(),
            id: self.next_hold,
        })
    }

    /// A run starts from a key of `page`, which `device` shows
    /// ([`FolderTimers::hold_entry`] of its entry). `None` when `page` is
    /// not the folder the device shows (a page outside folders).
    pub(crate) fn hold(
        &mut self,
        device: &DeviceId,
        page: PageId,
        now: Instant,
    ) -> Option<FolderHold> {
        let entry = self.entry_of(device).filter(|entry| entry.page == page)?;
        self.hold_entry(&entry, now)
    }

    /// Another run of the same entry as `hold`, with its own id. `None`
    /// when the entry is gone.
    pub(crate) fn fork(&mut self, hold: &FolderHold, now: Instant) -> Option<FolderHold> {
        self.hold_entry(&hold.entry, now)
    }

    /// Something a key of `entry` started ran to its end at once (an
    /// action that ran in place). With runs of the entry still going, it
    /// counts when the last of them ends; otherwise the entry is returned
    /// when it is the folder shown, as by [`FolderTimers::release`].
    pub(crate) fn ran(&mut self, entry: &FolderExit) -> Option<FolderExit> {
        let state = self.devices.get_mut(&entry.device)?;
        let last = state.entries.len().saturating_sub(1);
        let (index, found) = state.entry_mut(entry)?;
        if !found.holds.is_empty() {
            found.ran = true;
            return None;
        }
        (index == last).then(|| entry.clone())
    }

    /// The run of `hold` is over. When it was the last one of its entry,
    /// the countdown goes on from what was left; and when one of the runs
    /// ran and the entry is the folder shown, the entry is returned: the
    /// runtime leaves it if the folder exits after its actions complete.
    /// A hold of an entry that is gone, or released before, does nothing.
    pub(crate) fn release(
        &mut self,
        hold: &FolderHold,
        end: HoldEnd,
        now: Instant,
    ) -> Option<FolderExit> {
        let state = self.devices.get_mut(&hold.entry.device)?;
        let last = state.entries.len().saturating_sub(1);
        let (index, entry) = state.entry_mut(&hold.entry)?;
        if !entry.holds.remove(&hold.id) {
            return None;
        }
        entry.ran |= end == HoldEnd::Ran;
        if !entry.holds.is_empty() {
            return None;
        }
        let ran = std::mem::take(&mut entry.ran);
        state.settle(now);
        (ran && index == last).then(|| hold.entry.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seconds_shown_change_once_a_second_and_round_up() {
        let now = Instant::now();
        let due = now + Duration::from_secs(10);
        assert_eq!(next_tick(due, now), now + Duration::from_secs(1));
        let later = now + Duration::from_millis(1500);
        assert_eq!(next_tick(due, later), now + Duration::from_secs(2));
        let last = now + Duration::from_millis(9200);
        assert_eq!(next_tick(due, last), due);
        assert_eq!(next_tick(due, due + Duration::from_secs(1)), due);
        assert_eq!(whole_seconds(Duration::from_millis(9001)), 10);
        assert_eq!(whole_seconds(Duration::from_secs(9)), 9);
        assert_eq!(whole_seconds(Duration::ZERO), 0);
    }
}
