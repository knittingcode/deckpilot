//! The Calls driver: the engine's overlay intent, the "Back to work"
//! return sessions and a core [`Runtime`]'s overlay frames, kept in step
//! (`docs/calls.md`, "Overlay").
//!
//! The agent owns one driver. It hands it provider events, the runtime's
//! overlay outcomes and the runs started from a suspended device's keys,
//! and wakes it at [`CallsDriver::next_wake`]. Everything is on the
//! engine's clock, so the tests drive it with a virtual one.
//!
//! - **Push and pop.** While the focused call is live, every connected
//!   device with a managed layout for its model and the overlay on shows the
//!   overlay frame on the page for the call's state; when no call is, every
//!   frame goes. A meeting that follows a meeting keeps the frames.
//! - **Faces** are drawn from the snapshot: the confirmed state, a pending
//!   command as pending, an unsupported control disabled with its reason.
//!   Nothing is shown as done before the call app confirmed it.
//! - **Dismissed** (the protected key): the device stays on its own
//!   presentation for the rest of that call; a new incoming call shows the
//!   overlay again.
//! - **Back to work** suspends the device's frame with a return session;
//!   its end resumes it, or pops it when the call is over. An incoming call
//!   preempts every session.
//! - **The watchdog**: a Calls frame the driver does not know, or one of a
//!   call that is no longer live, is popped.

use std::collections::{BTreeMap, BTreeSet};

use deckpilot_core::{
    ActionParameters, FaceStyle, KeyFace, OverlayOwner, OverlaySession, OverlaySpec, Runtime,
};
use deckpilot_device::{Capability as Family, ControlId, DeviceId, DeviceModelId};

use crate::clock::Instant;
use crate::engine::{CallSession, Engine, OverlayIntent, OverlayPage, Request, TrackId};
use crate::layouts::{Generated, OWNER, Page};
use crate::ops::{GateError, action_of};
use crate::provider::{CallAction, Capability, ProviderEvent, ProviderId, Support, Tri};
use crate::ret::{
    AfterRun, Qualify, ReturnController, ReturnEnded, ReturnMode, ReturnSettings, RunEnd, RunKind,
    RunToken,
};
use crate::snapshot::{CallSnapshot, CallState};

#[cfg(test)]
#[path = "driver/tests.rs"]
mod tests;

/// What an overlay key asked for, after the driver handled it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Send this command to the provider.
    Send(Request),
    /// The device was suspended ("Back to work") or resumed.
    Presentation,
    /// The gate refused it; shown on the key, nothing sent.
    Refused(GateError),
    /// Not a Calls action.
    Ignored,
}

/// The languages of the faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Locale {
    #[default]
    En,
    Pl,
}

#[derive(Debug, Clone)]
struct Frame {
    session: OverlaySession,
    call: CallSession,
}

/// The Calls driver.
#[derive(Debug)]
pub struct CallsDriver {
    engine: Engine,
    returns: ReturnController<DeviceId>,
    layouts: BTreeMap<DeviceModelId, Generated>,
    frames: BTreeMap<DeviceId, Frame>,
    /// Per-device overlay setting; absent: on for devices with key
    /// displays.
    enabled: BTreeMap<DeviceId, bool>,
    /// Devices whose overlay the user dismissed, with the call session
    /// and the focused call they dismissed it for: hidden while that call
    /// has the focus.
    dismissed: BTreeMap<DeviceId, (CallSession, Option<TrackId>)>,
    locale: Locale,
    /// The frames the watchdog popped (for the log, once each).
    watchdog_pops: usize,
    /// The editor's previews of a call page (no call, no provider): one
    /// per device, never at the same time as a real call's frame.
    previews: BTreeMap<DeviceId, (OverlaySession, Page)>,
    /// Unit tests: the next preview's push fails as if the device took no
    /// frame.
    #[cfg(test)]
    fail_next_preview: bool,
}

/// Why a page could not be previewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewError {
    /// A real call shows: it always wins.
    InCall,
    /// The device's model has no managed layout, or the layout no such page.
    NoLayout,
    /// The device could not take the frame.
    Refused,
}

impl CallsDriver {
    pub fn new(engine: Engine) -> Self {
        Self {
            engine,
            returns: ReturnController::default(),
            layouts: BTreeMap::new(),
            frames: BTreeMap::new(),
            enabled: BTreeMap::new(),
            dismissed: BTreeMap::new(),
            previews: BTreeMap::new(),
            #[cfg(test)]
            fail_next_preview: false,
            locale: Locale::default(),
            watchdog_pops: 0,
        }
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    pub fn returns(&self) -> &ReturnController<DeviceId> {
        &self.returns
    }

    pub fn set_locale(&mut self, locale: Locale) {
        self.locale = locale;
    }

    /// The managed layout for `model` (from storage, seeded from the
    /// template of the model's class).
    pub fn set_layout(&mut self, model: DeviceModelId, generated: Generated) {
        self.layouts.insert(model, generated);
    }

    /// The per-device overlay setting (`calls.overlay.devices`).
    pub fn set_device_enabled(
        &mut self,
        runtime: &mut Runtime,
        device: &DeviceId,
        on: bool,
        now: Instant,
    ) {
        self.enabled.insert(device.clone(), on);
        if !on {
            self.pop(runtime, device);
        }
        self.sync(runtime, now);
    }

    /// Whether `device` gets the overlay (its setting, or the default).
    pub fn overlay_enabled(&self, runtime: &Runtime, device: &DeviceId) -> bool {
        self.device_enabled(runtime, device)
    }

    /// The managed layout used for `model`.
    pub fn layout(&self, model: &DeviceModelId) -> Option<&Generated> {
        self.layouts.get(model)
    }

    fn device_enabled(&self, runtime: &Runtime, device: &DeviceId) -> bool {
        self.enabled.get(device).copied().unwrap_or_else(|| {
            runtime
                .catalog()
                .get(&device.model)
                .is_some_and(|m| m.capabilities.supports(Family::KeyDisplays))
        })
    }

    /// How many frames the watchdog popped so far.
    pub fn watchdog_pops(&self) -> usize {
        self.watchdog_pops
    }

    // -------------------------------------------------------------- events

    /// One provider event.
    pub fn on_provider_event(
        &mut self,
        runtime: &mut Runtime,
        provider: &ProviderId,
        event: ProviderEvent,
        now: Instant,
    ) {
        self.engine.on_event(provider, event, now);
        self.sync(runtime, now);
    }

    /// When [`CallsDriver::tick`] is due.
    pub fn next_wake(&self) -> Option<Instant> {
        [self.engine.next_wake(), self.returns.next_wake()]
            .into_iter()
            .flatten()
            .min()
    }

    /// Applies every timeout due at `now`.
    pub fn tick(&mut self, runtime: &mut Runtime, now: Instant) {
        self.engine.tick(now);
        for ended in self.returns.tick(now) {
            self.apply(runtime, &ended);
        }
        self.sync(runtime, now);
    }

    /// A device connected (or came back): it shows the overlay if the call
    /// still needs it.
    pub fn device_connected(&mut self, runtime: &mut Runtime, now: Instant) {
        self.sync(runtime, now);
    }

    // ---------------------------------------------------------------- keys

    /// A key of a Calls overlay was pressed
    /// ([`deckpilot_core::RuntimeOutcome::OverlayAction`]).
    pub fn overlay_key(
        &mut self,
        runtime: &mut Runtime,
        device: &DeviceId,
        session: OverlaySession,
        action_type: &str,
        parameters: &ActionParameters,
        now: Instant,
    ) -> KeyOutcome {
        let Some(frame) = self.frames.get(device) else {
            return KeyOutcome::Ignored;
        };
        if frame.session != session {
            return KeyOutcome::Ignored;
        }
        let Some(id) = action_type
            .strip_prefix(OWNER)
            .and_then(|rest| rest.strip_prefix('.'))
        else {
            return KeyOutcome::Ignored;
        };
        if id == "return-to-work" {
            return self.return_to_work(runtime, device, parameters, now);
        }
        let Some(action) = action_of(id) else {
            return KeyOutcome::Ignored;
        };
        let outcome = match self.engine.request(action, now) {
            Ok(request) => KeyOutcome::Send(request),
            Err(error) => KeyOutcome::Refused(error),
        };
        self.sync(runtime, now);
        outcome
    }

    /// Shows `page` of `device`'s managed layout as the editor previews it:
    /// a frame of its own, with faces drawn from a fixed snapshot in that
    /// page's state. The snapshot holds no call: no title, no participant,
    /// the microphone and camera unknown, and the provider's capabilities as
    /// reported now (so unsupported controls show disabled with their
    /// reason). Its keys run nothing. A preview already on the device is
    /// replaced. A real call always wins: refused while one shows, and
    /// ended by [`CallsDriver::sync`] when one comes.
    ///
    /// # Errors
    ///
    /// [`PreviewError`].
    pub fn preview(
        &mut self,
        runtime: &mut Runtime,
        device: &DeviceId,
        page: Page,
    ) -> Result<(), PreviewError> {
        if matches!(self.engine.overlay(), OverlayIntent::Shown { .. })
            || self.frames.contains_key(device)
        {
            return Err(PreviewError::InCall);
        }
        let generated = self
            .layouts
            .get(&device.model)
            .ok_or(PreviewError::NoLayout)?;
        let page_id = *generated.pages.get(&page).ok_or(PreviewError::NoLayout)?;
        let spec = OverlaySpec {
            owner: OverlayOwner::Calls,
            layout: generated.layout.clone(),
            actions: generated.actions.clone(),
            page: page_id,
            dismiss: generated.dismiss,
        };
        let state = match page {
            Page::Incoming => CallState::Incoming,
            Page::Outgoing => CallState::Outgoing,
            Page::Active => CallState::Active,
        };
        let live = self.engine.snapshot();
        let mut snapshot = CallSnapshot::idle(live.capabilities, live.generation);
        snapshot.state = state;
        snapshot.connected = true;
        let faces = faces(generated, page_id, &snapshot, self.locale);
        // The new frame first: when the device takes none, the old preview
        // (if any) stays as it was, still known.
        #[cfg(test)]
        if std::mem::take(&mut self.fail_next_preview) {
            return Err(PreviewError::Refused);
        }
        let session = runtime
            .push_overlay(device, spec)
            .map_err(|_| PreviewError::Refused)?;
        runtime.set_overlay_faces(device, session, faces);
        if let Some((old, _)) = self.previews.insert(device.clone(), (session, page)) {
            runtime.pop_overlay(device, old);
        }
        Ok(())
    }

    /// Ends `device`'s preview, if it has one: its frame goes.
    pub fn end_preview(&mut self, runtime: &mut Runtime, device: &DeviceId) {
        if let Some((session, _)) = self.previews.remove(device) {
            runtime.pop_overlay(device, session);
        }
    }

    /// Ends every preview.
    pub fn end_previews(&mut self, runtime: &mut Runtime) {
        let devices: Vec<DeviceId> = self.previews.keys().cloned().collect();
        for device in devices {
            self.end_preview(runtime, &device);
        }
    }

    /// The devices with a preview.
    pub fn previews(&self) -> impl Iterator<Item = &DeviceId> {
        self.previews.keys()
    }

    /// The page `device` previews, if it has a preview.
    pub fn preview_page(&self, device: &DeviceId) -> Option<Page> {
        self.previews.get(device).map(|(_, page)| *page)
    }

    /// The overlay was dismissed on `device`
    /// ([`deckpilot_core::RuntimeOutcome::OverlayDismissed`]): the runtime
    /// popped it; the device stays on its own presentation for this call.
    pub fn dismissed(&mut self, device: &DeviceId, session: OverlaySession) {
        // A preview dismissed on the device: the runtime popped its frame.
        if self.previews.get(device).map(|(s, _)| *s) == Some(session) {
            self.previews.remove(device);
        }
        if self
            .frames
            .get(device)
            .is_some_and(|f| f.session == session)
            && let Some(frame) = self.frames.remove(device)
        {
            self.dismissed
                .insert(device.clone(), (frame.call, self.engine.focus()));
        }
        self.returns.drop_device(device);
    }

    /// *Show calls* on `device` (a key of the user's own layout): resumes a
    /// suspended overlay at once, or shows a dismissed one again.
    pub fn show_calls(&mut self, runtime: &mut Runtime, device: &DeviceId, now: Instant) -> bool {
        if let Some(ended) = self.returns.user_resumed(device) {
            self.apply(runtime, &ended);
            self.sync(runtime, now);
            return true;
        }
        let was = self.dismissed.remove(device).is_some();
        self.sync(runtime, now);
        was || self.frames.contains_key(device)
    }

    fn return_to_work(
        &mut self,
        runtime: &mut Runtime,
        device: &DeviceId,
        parameters: &ActionParameters,
        now: Instant,
    ) -> KeyOutcome {
        let settings = settings_of(parameters);
        let devices: Vec<DeviceId> = if settings.all_devices {
            self.frames.keys().cloned().collect()
        } else {
            vec![device.clone()]
        };
        for target in devices {
            let Some(frame) = self.frames.get(&target) else {
                continue;
            };
            runtime.suspend_overlay(&target, frame.session);
            self.returns.suspend(target, settings, now);
        }
        KeyOutcome::Presentation
    }

    /// A key of `device`'s own presentation started `run` (`kind`) while it
    /// was suspended. `true` when the run counts for its return session.
    pub fn run_started(&mut self, device: &DeviceId, run: RunToken, kind: RunKind) -> bool {
        match self.returns.current(device) {
            Some(id) => self.returns.hold(device, id, run, kind),
            None => false,
        }
    }

    /// `run` of `device` ended with `end`.
    pub fn run_finished(
        &mut self,
        runtime: &mut Runtime,
        device: &DeviceId,
        run: RunToken,
        end: RunEnd,
        now: Instant,
    ) {
        if let Some(ended) = self.returns.run_finished(device, run, end) {
            self.apply(runtime, &ended);
            self.sync(runtime, now);
        }
    }

    fn apply(&mut self, runtime: &mut Runtime, ended: &ReturnEnded<DeviceId>) {
        let Some(frame) = self.frames.get(&ended.device) else {
            return;
        };
        if ended.resumes() {
            runtime.resume_overlay(&ended.device, frame.session);
        } else {
            let device = ended.device.clone();
            self.pop(runtime, &device);
        }
    }

    fn pop(&mut self, runtime: &mut Runtime, device: &DeviceId) {
        if let Some(frame) = self.frames.remove(device) {
            runtime.pop_overlay(device, frame.session);
        }
        self.returns.drop_device(device);
    }

    // ---------------------------------------------------------------- sync

    /// Brings every device's frame in line with the engine.
    pub fn sync(&mut self, runtime: &mut Runtime, _now: Instant) {
        let intent = self.engine.overlay();
        if matches!(intent, OverlayIntent::Shown { .. }) {
            // A real call always wins over the editor's preview.
            self.end_previews(runtime);
        }
        let OverlayIntent::Shown { session, page, .. } = intent else {
            // No call: every session ends without resuming, every frame
            // goes.
            self.returns.call_ended();
            let devices: Vec<DeviceId> = self.frames.keys().cloned().collect();
            for device in devices {
                self.pop(runtime, &device);
            }
            self.dismissed.clear();
            self.watchdog(runtime);
            return;
        };
        let page = match page {
            OverlayPage::Incoming => Page::Incoming,
            OverlayPage::Outgoing => Page::Outgoing,
            OverlayPage::Active => Page::Active,
        };
        if page == Page::Incoming {
            // A new incoming call takes every suspended device. (A dismissed
            // device sees it because it is another call: below.)
            for ended in self.returns.preempt() {
                self.apply(runtime, &ended);
            }
        }
        // Frames of an earlier call session (should not happen: the engine
        // keeps one session per run of calls).
        let stale: Vec<DeviceId> = self
            .frames
            .iter()
            .filter(|(_, f)| f.call != session)
            .map(|(d, _)| d.clone())
            .collect();
        for device in stale {
            self.pop(runtime, &device);
        }
        let snapshot = self.engine.snapshot();
        let focus = self.engine.focus();
        let devices: Vec<DeviceId> = runtime.devices().connected().cloned().collect();
        for device in devices {
            if !self.device_enabled(runtime, &device) {
                continue;
            }
            // Dismissed for the call that has the focus: stays away (also
            // when the focus comes back to it).
            if self.dismissed.get(&device) == Some(&(session, focus)) {
                if self.frames.contains_key(&device) {
                    self.pop(runtime, &device);
                }
                continue;
            }
            let Some(generated) = self.layouts.get(&device.model) else {
                continue;
            };
            let Some(page_id) = generated.page(page) else {
                continue;
            };
            let frame = if let Some(frame) = self.frames.get(&device) {
                runtime.show_overlay_page(&device, frame.session, page_id);
                frame.session
            } else {
                let spec = OverlaySpec {
                    owner: OverlayOwner::Calls,
                    layout: generated.layout.clone(),
                    actions: generated.actions.clone(),
                    page: page_id,
                    dismiss: generated.dismiss,
                };
                let Ok(pushed) = runtime.push_overlay(&device, spec) else {
                    continue;
                };
                self.frames.insert(
                    device.clone(),
                    Frame {
                        session: pushed,
                        call: session,
                    },
                );
                pushed
            };
            let faces = faces(generated, page_id, &snapshot, self.locale);
            runtime.set_overlay_faces(&device, frame, faces);
        }
        self.watchdog(runtime);
    }

    /// Pops every Calls frame the driver does not hold.
    fn watchdog(&mut self, runtime: &mut Runtime) {
        for (device, views) in runtime.all_overlays() {
            for view in views {
                let known = self
                    .frames
                    .get(&device)
                    .is_some_and(|f| f.session == view.session)
                    || self.previews.get(&device).map(|(s, _)| *s) == Some(view.session);
                if view.owner == OverlayOwner::Calls && !known {
                    runtime.pop_overlay(&device, view.session);
                    self.watchdog_pops += 1;
                }
            }
        }
    }

    /// The overlay frame of `device`, if it has one.
    pub fn frame(&self, device: &DeviceId) -> Option<OverlaySession> {
        self.frames.get(device).map(|f| f.session)
    }

    /// Every Calls frame goes and every session ends (the plugin disabled,
    /// consent withdrawn).
    pub fn clear(&mut self, runtime: &mut Runtime) {
        self.returns.clear();
        self.end_previews(runtime);
        let devices: Vec<DeviceId> = self.frames.keys().cloned().collect();
        for device in devices {
            self.pop(runtime, &device);
        }
        runtime.pop_overlays_of(OverlayOwner::Calls);
    }
}

/// The parameters that [`settings_of`] reads back as `settings` (its
/// inverse for every mode, outcome and flag; times as whole seconds).
pub fn parameters_of(settings: &ReturnSettings) -> ActionParameters {
    use deckpilot_core::ParameterValue::{Boolean, Choice, Integer};
    let whole = |d: std::time::Duration| i64::try_from(d.as_secs()).unwrap_or(i64::MAX);
    let mut parameters = match settings.mode.clamped() {
        ReturnMode::Timer { after } => ActionParameters::new()
            .with("mode", Choice("timer".into()))
            .with("seconds", Integer(whole(after))),
        ReturnMode::Completion => ActionParameters::new().with("mode", Choice("completion".into())),
        ReturnMode::CompletionOrTimeout { max } => ActionParameters::new()
            .with("mode", Choice("completionOrTimeout".into()))
            .with("maxSeconds", Integer(whole(max))),
    };
    let after = |a: AfterRun| {
        Choice(match a {
            AfterRun::Resume => "resume".into(),
            AfterRun::Stay => "stay".into(),
        })
    };
    parameters = parameters
        .with("onFailure", after(settings.on_failure))
        .with("onCancel", after(settings.on_cancel))
        .with("returnAllDevices", Boolean(settings.all_devices))
        .with("countNavigation", Boolean(settings.qualify.navigation));
    parameters
}

/// The *Back to work* settings in an action's parameters (ordinary
/// parameters the inspector edits); the defaults for anything missing or
/// out of range.
pub fn settings_of(parameters: &ActionParameters) -> ReturnSettings {
    let mut settings = ReturnSettings::default();
    // A choice parameter (the manifest's) or plain text.
    let word = |key: &str| match parameters.get(key) {
        Some(
            deckpilot_core::ParameterValue::Choice(text)
            | deckpilot_core::ParameterValue::Text(text),
        ) => Some(text.as_str()),
        _ => None,
    };
    let seconds = |key: &str| {
        parameters.get(key).and_then(|v| match v {
            deckpilot_core::ParameterValue::Integer(n) => u64::try_from(*n).ok(),
            _ => None,
        })
    };
    match word("mode") {
        Some("timer") => {
            // A time out of range is the default time, still a timer.
            settings.mode = seconds("seconds")
                .and_then(|s| ReturnMode::timer(s).ok())
                .unwrap_or(ReturnMode::Timer {
                    after: std::time::Duration::from_secs(30),
                });
        }
        Some("completion") => settings.mode = ReturnMode::Completion,
        _ => {
            if let Some(Ok(mode)) = seconds("maxSeconds").map(ReturnMode::completion_or_timeout) {
                settings.mode = mode;
            }
        }
    }
    let flag = |key: &str| match parameters.get(key) {
        Some(deckpilot_core::ParameterValue::Boolean(b)) => Some(*b),
        _ => None,
    };
    let after = |key: &str, default: AfterRun| match word(key) {
        Some("resume") => AfterRun::Resume,
        Some("stay") => AfterRun::Stay,
        _ => default,
    };
    settings.qualify = Qualify {
        navigation: flag("countNavigation").unwrap_or(false),
        ..Qualify::default()
    };
    settings.on_failure = after("onFailure", AfterRun::Resume);
    settings.on_cancel = after("onCancel", AfterRun::Stay);
    settings.all_devices = flag("returnAllDevices").unwrap_or(false);
    settings
}

/// The faces of `page` for `snapshot`: the confirmed state, pending
/// commands as pending, unsupported controls disabled with their reason.
fn faces(
    generated: &Generated,
    page: deckpilot_core::PageId,
    snapshot: &CallSnapshot,
    locale: Locale,
) -> Vec<(ControlId, KeyFace)> {
    let Some(layout_page) = generated.layout.page(page) else {
        return Vec::new();
    };
    layout_page
        .bindings
        .iter()
        .filter_map(|binding| {
            let id = generated.action_of(binding.target.action_id()?)?;
            Some((binding.control, face(id, snapshot, locale)))
        })
        .collect()
}

fn text(locale: Locale, en: &'static str, pl: &'static str) -> &'static str {
    match locale {
        Locale::En => en,
        Locale::Pl => pl,
    }
}

/// The face of one Calls action.
pub fn face(id: &str, snapshot: &CallSnapshot, locale: Locale) -> KeyFace {
    let t = |en, pl| text(locale, en, pl);
    if id == "return-to-work" {
        return KeyFace::new(FaceStyle::Action, t("Back to work", "Wróć do pracy"));
    }
    let Some(action) = action_of(id) else {
        return KeyFace::new(FaceStyle::Notice, id.to_owned());
    };
    let title = match action {
        CallAction::Answer => t("Answer", "Odbierz"),
        CallAction::Reject => t("Reject", "Odrzuć"),
        CallAction::CancelOutgoing => t("Cancel", "Anuluj"),
        CallAction::End => t("Leave", "Zakończ"),
        CallAction::MicToggle | CallAction::MicSet { .. } => t("Mic", "Mikrofon"),
        CallAction::CameraToggle | CallAction::CameraSet { .. } => t("Camera", "Kamera"),
    };
    let capability = action.capability();
    let support = snapshot.capabilities.get(capability);
    if !snapshot.connected && snapshot.state.is_live() {
        return KeyFace::new(FaceStyle::Notice, title).with_detail(t(
            "No connection to the call app",
            "Brak połączenia z dostawcą",
        ));
    }
    if let Support::Unsupported { reason } = support
        && !matches!(
            action,
            CallAction::MicSet { .. } | CallAction::CameraSet { .. }
        )
    {
        return KeyFace::new(FaceStyle::Notice, title).with_detail(reason_text(reason, locale));
    }
    if !support.allows_command() && !toggle_covers(snapshot, action) {
        return KeyFace::new(FaceStyle::Notice, title).with_detail(t(
            "The call app does not allow it",
            "Aplikacja nie udostępnia sterowania",
        ));
    }
    if snapshot.pending == Some(action) {
        return KeyFace::new(FaceStyle::Action, title).with_detail(t("Waiting…", "Czekam…"));
    }
    if snapshot.unconfirmed {
        return KeyFace::new(FaceStyle::Action, title)
            .with_detail(t("Not confirmed", "Nie potwierdzono"));
    }
    let state = |value: Tri, on: &'static str, off: &'static str| match value {
        Tri::Known(true) => on,
        Tri::Known(false) => off,
        Tri::Unknown => t("Unknown", "Nieznany"),
    };
    let detail = match action {
        CallAction::MicToggle | CallAction::MicSet { .. } => Some(state(
            snapshot.mic_muted,
            t("Muted", "Wyciszony"),
            t("On", "Włączony"),
        )),
        CallAction::CameraToggle | CallAction::CameraSet { .. } => Some(state(
            snapshot.camera_on,
            t("On", "Włączona"),
            t("Off", "Wyłączona"),
        )),
        _ => None,
    };
    let face = KeyFace::new(FaceStyle::Action, title);
    match detail {
        Some(detail) if snapshot.state == CallState::Active => face.with_detail(detail),
        _ => face,
    }
}

/// A set the provider can do by toggling its read state.
fn toggle_covers(snapshot: &CallSnapshot, action: CallAction) -> bool {
    let (toggle, read) = match action {
        CallAction::MicSet { .. } => (Capability::MicToggle, Capability::MicReadState),
        CallAction::CameraSet { .. } => (Capability::CameraToggle, Capability::CameraReadState),
        _ => return false,
    };
    snapshot.capabilities.get(toggle).allows_command()
        && snapshot.capabilities.get(read).allows_command()
}

fn reason_text(reason: crate::provider::UnsupportedReason, locale: Locale) -> &'static str {
    use crate::provider::UnsupportedReason as R;
    let t = |en, pl| text(locale, en, pl);
    match reason {
        R::ProviderHasNoApi => t(
            "The call app offers no control",
            "Aplikacja nie udostępnia sterowania",
        ),
        R::PlatformNotSupported => t("Not on this system", "Niedostępne w tym systemie"),
        R::NotInMeeting => t("Not in a meeting", "Poza spotkaniem"),
        R::NoPermissionInMeeting => t(
            "Not allowed in this meeting",
            "Niedozwolone w tym spotkaniu",
        ),
        R::ToggleOnly => t("State unknown", "Stan nieznany"),
        R::NeedsPairing => t("Pair the app first", "Najpierw sparuj aplikację"),
        R::NeedsOsPermission => t("Needs a system permission", "Wymaga uprawnienia systemu"),
        R::ConsentNotGiven => t("Not allowed for Calls", "Brak zgody dla Rozmów"),
    }
}

/// The devices that show a Calls frame now (for tests and diagnostics).
pub fn shown(runtime: &Runtime) -> BTreeSet<DeviceId> {
    runtime
        .all_overlays()
        .into_iter()
        .filter(|(_, views)| {
            views
                .iter()
                .any(|v| v.visible && v.owner == OverlayOwner::Calls)
        })
        .map(|(d, _)| d)
        .collect()
}
