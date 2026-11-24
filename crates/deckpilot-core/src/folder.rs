//! Folder pages: the protected system Back key and Auto Exit
//! (`docs/data-model.md`, "Pages and folders").
//!
//! **The reserved Back key.** Every page inside a folder (a page with a
//! parent, stored or inferred; never a top-level page and never the home
//! page) returns one folder level with Back. A folder page whose
//! [`FolderBack`] is [`BackPosition::Reserved`] has that Back on a fixed,
//! core-owned control: the **first key with a display** in the model's
//! ordered controls ([`reserved_back_control`]; key 0, top left on a
//! rectangular grid). Never a dial or a touch strip; a model without keys
//! with a display has no reserved key and relies on the host's navigation.
//! The key is not a binding: the agent shows and runs it (with every plugin
//! disabled too), the editor shows it protected, and a layout that binds
//! anything to it is refused when it is saved
//! ([`crate::LayoutIssue::ReservedBack`]).
//!
//! [`BackPosition::Legacy`] (and a folder page without the setting, until it
//! is migrated or saved) keeps the behaviour from before: an automatic Back
//! on the page's first free key when the page has no way back of its own
//! ([`DeviceLayout::automatic_back`]).
//!
//! **Auto Exit.** A folder page may leave itself after a number of seconds
//! ([`AutoExit`]) and, with [`crate::LayoutPage::auto_return`] ("Exit after
//! action completes"), once what one of its keys started is over. Both are
//! run by [`FolderTimers`] (`folder/timers.rs`), which the agent's loop
//! drives and which pauses while a key's run is not over.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use deckpilot_device::{ControlId, ControlKind, DeviceCapabilities};

use crate::{BindingTarget, DeviceLayout, PageId};

/// Where a folder page's Back is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackPosition {
    /// On the reserved, protected key ([`reserved_back_control`]).
    Reserved,
    /// The behaviour from before the reserved key: an automatic Back on
    /// the first free key when the page has no way back of its own. Kept
    /// for folders whose first key already ran something when they were
    /// migrated; [`DeviceLayout::move_back_to_reserved`] changes it.
    Legacy,
}

/// The Back of a folder page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FolderBack {
    pub position: BackPosition,
}

impl FolderBack {
    pub const RESERVED: Self = Self {
        position: BackPosition::Reserved,
    };
    pub const LEGACY: Self = Self {
        position: BackPosition::Legacy,
    };
}

/// A folder page that leaves itself `seconds` after a device entered it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutoExit {
    pub enabled: bool,
    pub seconds: u32,
}

impl AutoExit {
    pub const MIN_SECONDS: u32 = 1;
    pub const MAX_SECONDS: u32 = 3600;
    /// What a new folder stores (switched off).
    pub const DEFAULT_SECONDS: u32 = 10;

    /// Switched off, with the default time: what new folders store.
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            seconds: Self::DEFAULT_SECONDS,
        }
    }

    /// The time must be whole seconds from 1 to 3600, also while it is
    /// switched off (it is kept for when it is switched on).
    ///
    /// # Errors
    ///
    /// [`AutoExitError::Seconds`] outside that range.
    pub fn check(self) -> Result<(), AutoExitError> {
        if (Self::MIN_SECONDS..=Self::MAX_SECONDS).contains(&self.seconds) {
            Ok(())
        } else {
            Err(AutoExitError::Seconds(self.seconds))
        }
    }

    /// How long a device stays in the folder; `None` when it is switched
    /// off or its time is not valid.
    pub fn duration(self) -> Option<Duration> {
        (self.enabled && self.check().is_ok()).then(|| Duration::from_secs(u64::from(self.seconds)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AutoExitError {
    #[error(
        "Auto Exit takes whole seconds from {min} to {max}, not {0}",
        min = AutoExit::MIN_SECONDS,
        max = AutoExit::MAX_SECONDS
    )]
    Seconds(u32),
}

/// The control that holds a folder page's reserved Back on a model: the
/// first key with a display in its ordered controls. `None` for a model
/// without such keys (dials, touch strips and keys without a display are
/// never reserved).
pub fn reserved_back_control(capabilities: &DeviceCapabilities) -> Option<ControlId> {
    capabilities
        .controls()
        .into_iter()
        .find(|control| control.kind() == ControlKind::Key && capabilities.has_display(*control))
}

/// What [`DeviceLayout::assign_folder_backs`] decided.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderBackReport {
    /// Folder pages that got the reserved Back.
    pub reserved: Vec<PageId>,
    /// Folder pages whose reserved key runs something of the user's: they
    /// keep the Back on the first free key.
    pub legacy: Vec<PageId>,
    /// Back bindings on the reserved key: kept (the key goes back anyway).
    pub kept_backs: usize,
}

impl FolderBackReport {
    pub fn changed(&self) -> bool {
        !self.reserved.is_empty() || !self.legacy.is_empty()
    }
}

/// Why [`DeviceLayout::move_back_to_reserved`] changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum FolderBackError {
    #[error("the page is not inside a folder")]
    NotAFolder,
    #[error("the device has no key with a display for Back")]
    NoReservedKey,
    #[error("every key of the page runs something; free a key first")]
    NoFreeKey,
}

impl DeviceLayout {
    /// Where the Back of `page` is: `None` for a page in no folder;
    /// [`BackPosition::Legacy`] for a folder page without the setting.
    pub fn back_position(&self, page: PageId) -> Option<BackPosition> {
        self.back_position_with(&self.page_parents(), page)
    }

    pub(crate) fn back_position_with(
        &self,
        parents: &BTreeMap<PageId, PageId>,
        page: PageId,
    ) -> Option<BackPosition> {
        if !parents.contains_key(&page) {
            return None;
        }
        Some(
            self.page(page)
                .and_then(|p| p.back)
                .map_or(BackPosition::Legacy, |back| back.position),
        )
    }

    /// The control that is the protected system Back of `page` on a
    /// device with `capabilities`: the reserved key of a folder page with
    /// [`BackPosition::Reserved`]. `None` elsewhere.
    pub fn system_back(
        &self,
        page: PageId,
        capabilities: &DeviceCapabilities,
    ) -> Option<ControlId> {
        self.system_back_with(&self.page_parents(), page, capabilities)
    }

    pub(crate) fn system_back_with(
        &self,
        parents: &BTreeMap<PageId, PageId>,
        page: PageId,
        capabilities: &DeviceCapabilities,
    ) -> Option<ControlId> {
        (self.back_position_with(parents, page) == Some(BackPosition::Reserved))
            .then(|| reserved_back_control(capabilities))
            .flatten()
    }

    /// How long a device stays in the folder `page` before it leaves it;
    /// `None` when the page is in no folder or Auto Exit is off.
    pub fn auto_exit(&self, page: PageId) -> Option<Duration> {
        self.parent_of(page)?;
        self.page(page)?.auto_exit?.duration()
    }

    /// Gives every folder page without a [`FolderBack`] one, without
    /// losing anything: the reserved Back when the reserved key binds
    /// nothing or only Back (those Back bindings are kept, unchanged: the
    /// key goes back as they did), else [`BackPosition::Legacy`], which keeps the Back
    /// where it was. Explicit Back bindings on other keys are kept. Pages
    /// that have the setting, and top-level pages, are left alone, so this
    /// is idempotent. The migration of stored layouts (schema 11), saving
    /// and importing run it.
    pub fn assign_folder_backs(&mut self, capabilities: &DeviceCapabilities) -> FolderBackReport {
        let parents = self.page_parents();
        let reserved_key = reserved_back_control(capabilities);
        let mut report = FolderBackReport::default();
        for page in &mut self.pages {
            if page.back.is_some() || !parents.contains_key(&page.id) {
                continue;
            }
            let on_key = |b: &&crate::ControlBinding| Some(b.control) == reserved_key;
            let only_back = page
                .bindings
                .iter()
                .filter(on_key)
                .all(|b| b.target == BindingTarget::Back);
            if only_back {
                report.kept_backs += page.bindings.iter().filter(on_key).count();
                page.back = Some(FolderBack::RESERVED);
                report.reserved.push(page.id);
            } else {
                page.back = Some(FolderBack::LEGACY);
                report.legacy.push(page.id);
            }
        }
        report
    }

    /// "Move Back to the first key": moves what the reserved key of the
    /// folder page `page` runs (its bindings and gesture timings) to the
    /// page's first free key with a display, and gives the page the
    /// reserved Back. Back bindings on the reserved key stay where they
    /// are. Returns the key the bindings went to (`None`: nothing had to
    /// move).
    ///
    /// # Errors
    ///
    /// Nothing changes for a page in no folder, a model without a key for
    /// Back, or a page whose keys all run something.
    pub fn move_back_to_reserved(
        &mut self,
        page: PageId,
        capabilities: &DeviceCapabilities,
    ) -> Result<Option<ControlId>, FolderBackError> {
        if self.parent_of(page).is_none() {
            return Err(FolderBackError::NotAFolder);
        }
        let reserved = reserved_back_control(capabilities).ok_or(FolderBackError::NoReservedKey)?;
        let stored = self.page_mut(page).ok_or(FolderBackError::NotAFolder)?;
        // Changed on a copy: a refusal leaves the page as it was.
        let mut copy = stored.clone();
        let entry = &mut copy;
        let to_move =
            |b: &crate::ControlBinding| b.control == reserved && b.target != BindingTarget::Back;
        let occupied = entry.bindings.iter().any(to_move);
        let mut moved = None;
        if occupied {
            let bound: BTreeSet<ControlId> = entry.bindings.iter().map(|b| b.control).collect();
            let free = capabilities
                .controls()
                .into_iter()
                .find(|c| {
                    *c != reserved
                        && c.kind() == ControlKind::Key
                        && capabilities.has_display(*c)
                        && !bound.contains(c)
                })
                .ok_or(FolderBackError::NoFreeKey)?;
            for binding in entry.bindings.iter_mut().filter(|b| to_move(b)) {
                binding.control = free;
            }
            entry.gestures.retain(|g| g.control != free);
            for settings in entry.gestures.iter_mut().filter(|g| g.control == reserved) {
                settings.control = free;
            }
            moved = Some(free);
        }
        entry.back = Some(FolderBack::RESERVED);
        *stored = copy;
        Ok(moved)
    }
}

mod timers;
pub use timers::{FolderCountdown, FolderExit, FolderHold, FolderTimers, HoldEnd};

#[cfg(test)]
mod tests;
