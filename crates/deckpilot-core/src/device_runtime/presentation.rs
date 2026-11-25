//! The per-device presentation stack: overlays such as the Calls overlay
//! (`docs/calls.md`, "Overlay").
//!
//! An overlay frame makes a device show and run another layout **without
//! changing anything of what it showed**: the active profile, the
//! navigation, the remembered pages, the switched-to layouts, the layout
//! history and the working layout stay as they were, so taking the frame
//! away shows the exact earlier presentation. Nothing of a frame is saved.
//!
//! - **What a frame shows.** Its own layout (a managed layout for the
//!   device's model, copied when it is pushed), its own page stack, and the
//!   faces its owner gives it, drawn over the layout's own faces by the
//!   same renderer.
//! - **What its keys do.** Nothing runs by itself: a bound action of the
//!   frame is handed to its owner ([`RuntimeOutcome::OverlayAction`]);
//!   pages of the frame's layout open and close in the frame's own stack;
//!   *Back* on its first page, and its protected dismiss control, take the
//!   frame away ([`RuntimeOutcome::OverlayDismissed`]). Workflows, stateful
//!   bindings and gestures do not run on an overlay. A device can never be
//!   trapped: the dismiss control always works.
//! - **Priorities** are a closed list compiled in ([`OverlayPriority`]):
//!   a higher one covers a lower one, the newer frame covers an older one
//!   of the same priority. A suspended frame stays on the stack and shows
//!   what is under it.
//! - **The input fence.** Whenever what a device shows changes this way
//!   (a frame shown, taken away, suspended or resumed), the controls held
//!   down at that instant are fenced: their remaining input is dropped
//!   until they are released. A press on one presentation never releases,
//!   or finishes a gesture, on the other.
//! - **Auto Exit underneath is paused** while a frame covers the device
//!   (a hold of the folder's entry, as a running action holds it) and goes
//!   on with the time it had when the frame goes.
//! - **The editor's live preview wins**: a device with a working layout
//!   shows it; the frame stays and shows again when the preview ends.
//! - **Disconnects.** Frames stay with their device; the owner decides
//!   whether a reconnecting device still has them.

use std::collections::{BTreeMap, BTreeSet};

use deckpilot_device::{ControlId, ControlInput, DeviceId};
use serde::{Deserialize, Serialize};

use super::{Runtime, RuntimeOutcome};
use crate::{
    ActionInstance, ActionParameters, BindingTarget, DeviceLayout, EngineError, FolderHold,
    HoldEnd, KeyFace, PageId, Profile,
};

/// Most frames one device holds.
pub const MAX_FRAMES: usize = 4;

/// How far up an overlay goes: a closed list, never a number an owner
/// chooses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayPriority {
    /// Reserved: a notification.
    Notification,
    /// The Calls overlay.
    Calls,
    /// Reserved: the system's own (an unlock prompt).
    System,
}

/// Who owns an overlay: a closed list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayOwner {
    /// The bundled Calls plugin's overlay (`deckpilot.calls`).
    Calls,
}

impl OverlayOwner {
    /// The priority of the owner's overlays.
    pub fn priority(self) -> OverlayPriority {
        match self {
            Self::Calls => OverlayPriority::Calls,
        }
    }
}

/// The id of one frame, never reused within a runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OverlaySession(pub u64);

/// What an owner pushes.
#[derive(Debug, Clone)]
pub struct OverlaySpec {
    pub owner: OverlayOwner,
    /// A layout for the device's model.
    pub layout: DeviceLayout,
    /// The action instances the layout's bindings name.
    pub actions: Vec<ActionInstance>,
    /// The page shown first.
    pub page: PageId,
    /// The protected control that always takes the frame away, whatever is
    /// bound to it.
    pub dismiss: Option<ControlId>,
}

/// Why a frame was not pushed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OverlayError {
    #[error("the device model is unknown")]
    UnknownModel,
    #[error("the layout is made for another device model")]
    WrongModel,
    #[error("the layout has no such page")]
    UnknownPage,
    #[error("the device has {MAX_FRAMES} overlays")]
    TooMany,
}

/// One frame of a device's presentation stack.
#[derive(Debug, Clone)]
pub(crate) struct OverlayFrame {
    owner: OverlayOwner,
    priority: OverlayPriority,
    session: OverlaySession,
    /// The layout and actions as a profile of their own, for the faces.
    profile: Profile,
    stack: Vec<PageId>,
    dismiss: Option<ControlId>,
    suspended: bool,
    faces: BTreeMap<ControlId, KeyFace>,
}

impl OverlayFrame {
    fn layout(&self) -> &DeviceLayout {
        &self.profile.layouts[0]
    }

    fn page(&self) -> PageId {
        self.stack
            .last()
            .copied()
            .unwrap_or(self.layout().home_page)
    }
}

/// A frame as its owner sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayView {
    pub owner: OverlayOwner,
    pub priority: OverlayPriority,
    pub session: OverlaySession,
    pub layout: crate::LayoutId,
    pub page: PageId,
    pub suspended: bool,
    /// Whether the device shows it now.
    pub visible: bool,
}

/// The presentation state of the runtime.
#[derive(Debug, Default)]
pub(crate) struct Presentation {
    /// Every device's frames, lowest first.
    frames: BTreeMap<DeviceId, Vec<OverlayFrame>>,
    /// The hold that pauses the covered folder's Auto Exit.
    holds: BTreeMap<DeviceId, FolderHold>,
    /// Controls whose remaining input is dropped until they are released.
    fences: BTreeMap<DeviceId, BTreeSet<ControlId>>,
    /// Controls held down now.
    pressed: BTreeMap<DeviceId, BTreeSet<ControlId>>,
    next_session: u64,
    /// Bumped on every change of what a device shows through frames.
    generation: u64,
    /// The same, per device: a gesture recognized on a device before its
    /// presentation changed is stale ([`Runtime::presentation_generation_of`]).
    device_generations: BTreeMap<DeviceId, u64>,
}

impl Runtime {
    // ------------------------------------------------------------ reading

    /// The frame `device` shows, by index, if it shows one: the highest
    /// that is not suspended. None while the editor previews the device,
    /// and none while the runtime shows its placeholder ("Locked"): a
    /// locked device shows no overlay and its keys hand no overlay action
    /// to an owner, whatever frames are still stacked (defence in depth:
    /// Calls also pops its frames on lock).
    fn visible_frame(&self, device: &DeviceId) -> Option<usize> {
        if self.working.contains_key(device) || self.shows_placeholder() {
            return None;
        }
        self.presentation
            .frames
            .get(device)?
            .iter()
            .rposition(|frame| !frame.suspended)
    }

    fn frame_of(&self, device: &DeviceId, session: OverlaySession) -> Option<usize> {
        self.presentation
            .frames
            .get(device)?
            .iter()
            .position(|frame| frame.session == session)
    }

    /// Whether `device` shows an overlay now.
    pub fn shows_overlay(&self, device: &DeviceId) -> bool {
        self.visible_frame(device).is_some()
    }

    /// The frames of `device`, lowest first.
    pub fn overlays(&self, device: &DeviceId) -> Vec<OverlayView> {
        let visible = self.visible_frame(device);
        self.presentation
            .frames
            .get(device)
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(index, frame)| OverlayView {
                owner: frame.owner,
                priority: frame.priority,
                session: frame.session,
                layout: frame.layout().id,
                page: frame.page(),
                suspended: frame.suspended,
                visible: visible == Some(index),
            })
            .collect()
    }

    /// Every device with frames, and its frames.
    pub fn all_overlays(&self) -> Vec<(DeviceId, Vec<OverlayView>)> {
        self.presentation
            .frames
            .keys()
            .map(|device| (device.clone(), self.overlays(device)))
            .collect()
    }

    /// Bumped whenever what a device shows through frames changes: a
    /// gesture recognized before is stale.
    pub fn presentation_generation(&self) -> u64 {
        self.presentation.generation
    }

    /// Bumped whenever what `device` shows changes through frames (a push,
    /// pop, suspend or resume). The caller that recognizes gestures records
    /// it when a sequence starts and drops the gesture when it changed by the
    /// time it fires: the input fence for keys whose edges never reach the
    /// runtime.
    pub fn presentation_generation_of(&self, device: &DeviceId) -> u64 {
        self.presentation
            .device_generations
            .get(device)
            .copied()
            .unwrap_or(0)
    }

    /// The layout and page `device` shows through its frame, if it shows
    /// one.
    pub fn overlay_page(&self, device: &DeviceId) -> Option<(&DeviceLayout, PageId)> {
        let frame = &self.presentation.frames.get(device)?[self.visible_frame(device)?];
        Some((frame.layout(), frame.page()))
    }

    // ----------------------------------------------------------- changing

    /// Pushes a frame onto `device`'s stack. The device shows it at once
    /// unless a higher one covers it.
    pub fn push_overlay(
        &mut self,
        device: &DeviceId,
        spec: OverlaySpec,
    ) -> Result<OverlaySession, OverlayError> {
        if self.catalog.get(&device.model).is_none() {
            return Err(OverlayError::UnknownModel);
        }
        if spec.layout.model != device.model {
            return Err(OverlayError::WrongModel);
        }
        if spec.layout.page(spec.page).is_none() {
            return Err(OverlayError::UnknownPage);
        }
        if self
            .presentation
            .frames
            .get(device)
            .is_some_and(|f| f.len() >= MAX_FRAMES)
        {
            return Err(OverlayError::TooMany);
        }
        self.presentation.next_session += 1;
        let session = OverlaySession(self.presentation.next_session);
        let mut profile = Profile::new("overlay");
        profile.actions = spec.actions;
        let page = spec.page;
        profile.layouts.push(spec.layout);
        let frame = OverlayFrame {
            owner: spec.owner,
            priority: spec.owner.priority(),
            session,
            profile,
            stack: vec![page],
            dismiss: spec.dismiss,
            suspended: false,
            faces: BTreeMap::new(),
        };
        self.change_presentation(device, |frames| {
            frames.push(frame);
            frames.sort_by_key(|f| (f.priority, f.session));
        });
        Ok(session)
    }

    /// Takes frame `session` away from `device`. `false` when it had none.
    pub fn pop_overlay(&mut self, device: &DeviceId, session: OverlaySession) -> bool {
        let Some(index) = self.frame_of(device, session) else {
            return false;
        };
        self.change_presentation(device, |frames| {
            frames.remove(index);
        });
        true
    }

    /// Takes every frame of `owner` away from every device (the owner is
    /// disabled); returns what was taken.
    pub fn pop_overlays_of(&mut self, owner: OverlayOwner) -> Vec<(DeviceId, OverlaySession)> {
        let mut popped = Vec::new();
        let devices: Vec<DeviceId> = self.presentation.frames.keys().cloned().collect();
        for device in devices {
            self.change_presentation(&device, |frames| {
                frames.retain(|frame| {
                    let keep = frame.owner != owner;
                    if !keep {
                        popped.push((device.clone(), frame.session));
                    }
                    keep
                });
            });
        }
        popped
    }

    /// Suspends frame `session`: the device shows what is under it while it
    /// stays on the stack. `false` when there is no such frame.
    pub fn suspend_overlay(&mut self, device: &DeviceId, session: OverlaySession) -> bool {
        self.set_suspended(device, session, true)
    }

    /// Shows a suspended frame again.
    pub fn resume_overlay(&mut self, device: &DeviceId, session: OverlaySession) -> bool {
        self.set_suspended(device, session, false)
    }

    fn set_suspended(
        &mut self,
        device: &DeviceId,
        session: OverlaySession,
        suspended: bool,
    ) -> bool {
        let Some(index) = self.frame_of(device, session) else {
            return false;
        };
        self.change_presentation(device, |frames| frames[index].suspended = suspended);
        true
    }

    /// Shows `page` of frame `session` as its only page (the owner's page
    /// for the call's state). `false` without such a frame or page.
    pub fn show_overlay_page(
        &mut self,
        device: &DeviceId,
        session: OverlaySession,
        page: PageId,
    ) -> bool {
        let Some(index) = self.frame_of(device, session) else {
            return false;
        };
        let frame = &self.presentation.frames[device][index];
        if frame.layout().page(page).is_none() {
            return false;
        }
        if frame.stack == [page] {
            return true;
        }
        self.change_presentation(device, |frames| frames[index].stack = vec![page]);
        true
    }

    /// The faces the owner draws over frame `session`'s, by control
    /// (replacing those it gave before).
    pub fn set_overlay_faces(
        &mut self,
        device: &DeviceId,
        session: OverlaySession,
        faces: impl IntoIterator<Item = (ControlId, KeyFace)>,
    ) -> bool {
        let Some(index) = self.frame_of(device, session) else {
            return false;
        };
        if let Some(frames) = self.presentation.frames.get_mut(device) {
            frames[index].faces = faces.into_iter().collect();
        }
        true
    }

    /// Applies `edit` to `device`'s frames, and what follows when what the
    /// device shows changed: the input fence, the paused Auto Exit, the
    /// underlying navigation repaired when it is shown again.
    fn change_presentation(
        &mut self,
        device: &DeviceId,
        edit: impl FnOnce(&mut Vec<OverlayFrame>),
    ) {
        let before = self.shown_session(device);
        let frames = self.presentation.frames.entry(device.clone()).or_default();
        edit(frames);
        if frames.is_empty() {
            self.presentation.frames.remove(device);
        }
        self.after_presentation_change(device, before);
    }

    /// The session `device` shows, with its page (`None`: its own
    /// presentation): what [`Runtime::after_presentation_change`] compares.
    pub(super) fn shown_overlay_session(
        &self,
        device: &DeviceId,
    ) -> Option<(OverlaySession, PageId)> {
        self.shown_session(device)
    }

    fn shown_session(&self, device: &DeviceId) -> Option<(OverlaySession, PageId)> {
        let index = self.visible_frame(device)?;
        let frame = &self.presentation.frames[device][index];
        Some((frame.session, frame.page()))
    }

    /// Call with what `device` showed (`before`) after anything that may
    /// have changed which frame it shows (also the editor's live preview).
    pub(super) fn after_presentation_change(
        &mut self,
        device: &DeviceId,
        before: Option<(OverlaySession, PageId)>,
    ) {
        let after = self.shown_session(device);
        if before == after {
            return;
        }
        self.presentation.generation += 1;
        *self
            .presentation
            .device_generations
            .entry(device.clone())
            .or_default() += 1;
        self.refused.clear();
        // Controls held down now finish on nothing.
        let held = self
            .presentation
            .pressed
            .get(device)
            .cloned()
            .unwrap_or_default();
        if !held.is_empty() {
            self.presentation
                .fences
                .entry(device.clone())
                .or_default()
                .extend(held);
        }
        match (before.is_some(), after.is_some()) {
            // Covered now: the folder under it stops counting down.
            (false, true) => self.pause_covered_folder(device),
            // Its own presentation again, as it was; anything that is gone
            // meanwhile is repaired as a reload would.
            (true, false) => {
                if let Some(hold) = self.presentation.holds.remove(device) {
                    let now = self.folder_clock();
                    self.folders.release(&hold, HoldEnd::NotRun, now);
                }
                self.repair_navigation(device);
                self.follow_folders(device, true);
            }
            _ => {}
        }
    }

    fn pause_covered_folder(&mut self, device: &DeviceId) {
        if self.presentation.holds.contains_key(device) {
            return;
        }
        let now = self.folder_clock();
        if let Some(hold) = self
            .folders
            .entry_of(device)
            .and_then(|entry| self.folders.hold_entry(&entry, now))
        {
            self.presentation.holds.insert(device.clone(), hold);
        }
    }

    /// A device connected: it pauses the folder it shows again if a frame
    /// covers it.
    pub(super) fn presentation_connected(&mut self, device: &DeviceId) {
        self.presentation.pressed.remove(device);
        self.presentation.fences.remove(device);
        if self.shows_overlay(device) {
            self.pause_covered_folder(device);
        }
    }

    /// A device disconnected: its folders are forgotten (and with them the
    /// hold); its frames stay.
    pub(super) fn presentation_disconnected(&mut self, device: &DeviceId) {
        self.presentation.holds.remove(device);
        self.presentation.pressed.remove(device);
        self.presentation.fences.remove(device);
    }

    // -------------------------------------------------------------- input

    /// Notes what `control` is doing and whether its input is fenced:
    /// `true` when the input must be dropped.
    pub(super) fn fence_input(
        &mut self,
        device: &DeviceId,
        control: ControlId,
        input: ControlInput,
    ) -> bool {
        match input {
            ControlInput::Press => {
                self.presentation
                    .pressed
                    .entry(device.clone())
                    .or_default()
                    .insert(control);
            }
            ControlInput::Release => {
                if let Some(pressed) = self.presentation.pressed.get_mut(device) {
                    pressed.remove(&control);
                }
            }
            _ => {}
        }
        let Some(fence) = self.presentation.fences.get_mut(device) else {
            return false;
        };
        if !fence.contains(&control) {
            return false;
        }
        if input == ControlInput::Release {
            fence.remove(&control);
            if fence.is_empty() {
                self.presentation.fences.remove(device);
            }
        }
        true
    }

    /// The input of a device that shows a frame: `None` when it shows none.
    pub(super) fn overlay_input(
        &mut self,
        device: &DeviceId,
        control: ControlId,
        input: ControlInput,
    ) -> Option<Result<RuntimeOutcome, EngineError>> {
        let index = self.visible_frame(device)?;
        let ignored = |reason: &str| {
            Some(Ok(RuntimeOutcome::Ignored {
                reason: reason.into(),
            }))
        };
        let model = self.catalog.get(&device.model)?;
        let frame = &self.presentation.frames[device][index];
        let session = frame.session;
        let layout = frame.layout();
        let page_id = frame.page();
        let at_root = frame.stack.len() <= 1;
        let press = input == ControlInput::Press;
        // The protected controls: dismiss, the folder's reserved Back.
        if frame.dismiss == Some(control) {
            return if press {
                Some(Ok(self.dismiss_overlay(device, session)))
            } else {
                ignored("the overlay's dismiss key reacts to a press")
            };
        }
        if layout.system_back(page_id, &model.capabilities) == Some(control) {
            return if press {
                Some(Ok(self.overlay_back(device, session)))
            } else {
                ignored("the folder's Back key reacts to a press")
            };
        }
        if let Some(folder) = layout.folder_control(page_id, control) {
            return if press {
                Some(Ok(self.overlay_open(device, session, folder)))
            } else {
                ignored("a folder key reacts to a press")
            };
        }
        let page = layout.page(page_id)?;
        let mut target = page
            .binding_for(control, input)
            .map(|binding| binding.target.clone());
        if target.is_none() && press {
            let controls: Vec<ControlId> = model
                .capabilities
                .controls()
                .into_iter()
                .filter(|c| model.capabilities.has_display(*c))
                .collect();
            if layout.automatic_back(page_id, &controls) == Some(control) {
                target = Some(BindingTarget::Back);
            }
        }
        match target {
            None => ignored("control is not bound"),
            Some(BindingTarget::Back) if at_root => Some(Ok(self.dismiss_overlay(device, session))),
            Some(BindingTarget::Back) => Some(Ok(self.overlay_back(device, session))),
            Some(BindingTarget::OpenPage { page }) => {
                Some(Ok(self.overlay_open(device, session, page)))
            }
            Some(BindingTarget::Action { action, overrides }) => {
                let Some(instance) = frame.profile.action(action) else {
                    return ignored("the overlay has no such action");
                };
                Some(Ok(RuntimeOutcome::OverlayAction {
                    device: device.clone(),
                    session,
                    control,
                    page: page_id,
                    action,
                    action_type: instance.action_type.clone(),
                    parameters: merged(&instance.parameters, &overrides),
                }))
            }
            Some(BindingTarget::Workflow { .. } | BindingTarget::Stateful(_)) => {
                ignored("workflows and stateful keys do not run on an overlay")
            }
        }
    }

    fn dismiss_overlay(&mut self, device: &DeviceId, session: OverlaySession) -> RuntimeOutcome {
        self.pop_overlay(device, session);
        RuntimeOutcome::OverlayDismissed {
            device: device.clone(),
            session,
        }
    }

    fn overlay_back(&mut self, device: &DeviceId, session: OverlaySession) -> RuntimeOutcome {
        let Some(index) = self.frame_of(device, session) else {
            return RuntimeOutcome::Ignored {
                reason: "no overlay".into(),
            };
        };
        if self.presentation.frames[device][index].stack.len() <= 1 {
            return self.dismiss_overlay(device, session);
        }
        self.change_presentation(device, |frames| {
            frames[index].stack.pop();
        });
        let page = self.presentation.frames[device][index].page();
        RuntimeOutcome::Navigated {
            device: device.clone(),
            page,
        }
    }

    fn overlay_open(
        &mut self,
        device: &DeviceId,
        session: OverlaySession,
        page: PageId,
    ) -> RuntimeOutcome {
        let Some(index) = self.frame_of(device, session) else {
            return RuntimeOutcome::Ignored {
                reason: "no overlay".into(),
            };
        };
        if self.presentation.frames[device][index]
            .layout()
            .page(page)
            .is_none()
        {
            return RuntimeOutcome::Ignored {
                reason: "the overlay's layout has no such page".into(),
            };
        }
        self.change_presentation(device, |frames| frames[index].stack.push(page));
        RuntimeOutcome::Navigated {
            device: device.clone(),
            page,
        }
    }

    // -------------------------------------------------------------- faces

    /// The faces of the frame `device` shows: the layout's own, with the
    /// owner's faces over them and the dismiss control's Back face.
    pub(super) fn overlay_faces(
        &self,
        device: &DeviceId,
        controls: &[ControlId],
        capabilities: &deckpilot_device::DeviceCapabilities,
    ) -> Option<Vec<(ControlId, KeyFace)>> {
        let index = self.visible_frame(device)?;
        let frame = &self.presentation.frames[device][index];
        let layout = frame.layout();
        let (mut faces, ()) =
            self.engine
                .display_templates(&frame.profile, &self.machine, None, |environment| {
                    let recorder = crate::variables::RecordingTemplates::new(environment);
                    let faces = super::page_faces_scoped(
                        self.engine.registry(),
                        layout,
                        frame.page(),
                        &frame.profile,
                        None,
                        controls,
                        Some(capabilities),
                        &recorder,
                        &self.workflow_scope,
                    );
                    (faces, ())
                });
        for (control, face) in &mut faces {
            if frame.dismiss == Some(*control) {
                *face = super::system_back_face();
            } else if let Some(owned) = frame.faces.get(control) {
                *face = owned.clone();
            }
        }
        Some(faces)
    }
}

/// The instance's parameters with the binding's overrides over them.
fn merged(parameters: &ActionParameters, overrides: &ActionParameters) -> ActionParameters {
    let mut out = parameters.clone();
    for (key, value) in overrides.iter() {
        out = out.with(key, value.clone());
    }
    out
}
