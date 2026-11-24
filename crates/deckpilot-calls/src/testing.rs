//! The mock provider: a deterministic, scripted [`CallProvider`] with
//! **full control** (answer, reject, cancel, end, microphone, camera), for
//! the tests and the dev seed. It is the only provider that exercises the
//! control path; no real call app is ever driven by it.
//!
//! - A **timeline** of `(instant, event)` is played as the clock passes it
//!   ([`MockProvider::script`] and the helpers such as
//!   [`MockProvider::ring`]).
//! - **Commands** get the response configured for their action
//!   ([`Response`]): confirmed after a delay (the call changes as the
//!   command asked, and a full observation follows), rejected, never
//!   answered, or unknown (a blind toggle). Capabilities are configurable,
//!   and a command for one the mock does not support is refused before
//!   anything is sent.
//! - **Restarts** of the call app and lost connections are events like any
//!   other.

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use crate::clock::Instant;
use crate::provider::{
    AppVersion, Availability, CallAction, CallCommand, CallKey, CallKind, CallObservation,
    CallProvider, Capability, CapabilitySet, CommandOutcome, CommandRefusal, CommandToken,
    Direction, DisconnectReason, Discovery, Evidence, ObservedEnd, ObservedPhase, ProviderCx,
    ProviderError, ProviderEvent, ProviderId, ProviderSession, Support, Tri, UnsupportedReason,
};
use crate::snapshot::FailureCode;

#[cfg(test)]
#[path = "testing/tests.rs"]
mod tests;

/// What the mock answers a command with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// Done after this long: the outcome is confirmed and the call shows it.
    ConfirmAfter(Duration),
    /// Refused by the call app, with this code.
    Reject(String),
    /// No answer at all.
    Never,
    /// Sent, but the app cannot tell whether it worked.
    Unknown,
}

/// The mock provider.
#[derive(Debug)]
pub struct MockProvider {
    id: ProviderId,
    capabilities: CapabilitySet,
    availability: Availability,
    timeline: VecDeque<(Instant, ProviderEvent)>,
    responses: BTreeMap<&'static str, Response>,
    /// Commands waiting for their response.
    pending: Vec<(Instant, CommandToken, CallCommand)>,
    /// The calls as the mock app has them now.
    calls: BTreeMap<CallKey, CallObservation>,
    session: Option<ProviderSession>,
    sent: Vec<CallCommand>,
    /// The instant of the last poll: commands are timed from it.
    clock: Instant,
}

fn action_name(action: CallAction) -> &'static str {
    match action {
        CallAction::Answer => "answer",
        CallAction::Reject => "reject",
        CallAction::CancelOutgoing => "cancel",
        CallAction::End => "end",
        CallAction::MicToggle | CallAction::MicSet { .. } => "mic",
        CallAction::CameraToggle | CallAction::CameraSet { .. } => "camera",
    }
}

impl MockProvider {
    /// A mock with every capability supported and every command confirmed
    /// after 200 ms.
    pub fn new() -> Self {
        let (capabilities, _) = CapabilitySet::from_reported(
            Capability::ALL.into_iter().map(|c| (c, Support::Supported)),
        );
        Self::with_capabilities(capabilities)
    }

    pub fn with_capabilities(capabilities: CapabilitySet) -> Self {
        Self {
            id: ProviderId::new(ProviderId::MOCK).expect("a valid id"),
            capabilities,
            availability: Availability::Available,
            timeline: VecDeque::new(),
            responses: BTreeMap::new(),
            pending: Vec::new(),
            calls: BTreeMap::new(),
            session: None,
            sent: Vec::new(),
            clock: Instant::ZERO,
        }
    }

    /// A detection-only mock: it observes, it controls nothing (as the
    /// Teams adapter).
    pub fn detection_only() -> Self {
        let (capabilities, _) = CapabilitySet::from_reported([
            (Capability::DetectMeeting, Support::Inferred),
            (Capability::MicReadState, Support::Inferred),
            (Capability::CameraReadState, Support::Inferred),
        ]);
        Self::with_capabilities(capabilities)
    }

    pub fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    /// How the mock answers `action`'s commands from now on.
    pub fn respond(&mut self, action: CallAction, response: Response) {
        self.responses.insert(action_name(action), response);
    }

    /// The commands the mock received, oldest first.
    pub fn sent(&self) -> &[CallCommand] {
        &self.sent
    }

    /// Plays `event` once the clock reaches `at`.
    pub fn script(&mut self, at: Instant, event: ProviderEvent) {
        let index = self.timeline.partition_point(|(t, _)| *t <= at);
        self.timeline.insert(index, (at, event));
    }

    fn observation(key: &str, phase: ObservedPhase, direction: Option<Direction>, at: Instant) -> CallObservation {
        CallObservation {
            key: CallKey::new(key).expect("a valid key"),
            kind: CallKind::OneToOne,
            phase,
            direction,
            mic_muted: Tri::Known(false),
            camera_on: Tri::Known(false),
            title: None,
            source: Evidence::AppApi,
            observed_at: at,
        }
    }

    /// An incoming call `key` rings at `at`.
    pub fn ring(&mut self, at: Instant, key: &str) {
        let observation = Self::observation(key, ObservedPhase::Ringing, Some(Direction::Incoming), at);
        self.script(at, ProviderEvent::Call(observation));
    }

    /// An outgoing call `key` dials at `at`.
    pub fn dial(&mut self, at: Instant, key: &str) {
        let observation = Self::observation(key, ObservedPhase::Dialing, Some(Direction::Outgoing), at);
        self.script(at, ProviderEvent::Call(observation));
    }

    /// A meeting `key` is joined at `at` (mic and camera off... known).
    pub fn join(&mut self, at: Instant, key: &str) {
        let observation = Self::observation(key, ObservedPhase::Connected, None, at);
        self.script(at, ProviderEvent::Call(observation));
    }

    /// The other side hangs up `key` at `at`.
    pub fn hang_up(&mut self, at: Instant, key: &str) {
        let observation = Self::observation(
            key,
            ObservedPhase::Ended {
                cause: ObservedEnd::RemoteHangUp,
            },
            None,
            at,
        );
        self.script(at, ProviderEvent::Call(observation));
    }

    /// The call app exits at `at` and comes back at `back` (a restart).
    pub fn restart_app(&mut self, at: Instant, back: Instant) {
        self.script(
            at,
            ProviderEvent::Disconnected {
                reason: DisconnectReason::AppExited,
            },
        );
        self.script(back, ProviderEvent::Connected {
            session: ProviderSession(0),
        });
    }

    /// When the mock has something to report next.
    pub fn next_due(&self) -> Option<Instant> {
        let timeline = self.timeline.front().map(|(t, _)| *t);
        let pending = self.pending.iter().map(|(t, _, _)| *t).min();
        [timeline, pending].into_iter().flatten().min()
    }

    fn next_session(&mut self) -> ProviderSession {
        let next = ProviderSession(self.session.map_or(1, |s| s.0 + 1));
        self.session = Some(next);
        next
    }

    /// What the call app does with a confirmed command.
    fn apply(&mut self, command: &CallCommand, at: Instant) -> Option<ProviderEvent> {
        let call = self.calls.get_mut(&command.key)?;
        call.observed_at = at;
        match command.action {
            CallAction::Answer => call.phase = ObservedPhase::Connected,
            CallAction::Reject => {
                call.phase = ObservedPhase::Ended {
                    cause: ObservedEnd::Rejected,
                };
            }
            CallAction::CancelOutgoing => {
                call.phase = ObservedPhase::Ended {
                    cause: ObservedEnd::Cancelled,
                };
            }
            CallAction::End => {
                call.phase = ObservedPhase::Ended {
                    cause: ObservedEnd::Left,
                };
            }
            CallAction::MicToggle => {
                call.mic_muted = Tri::Known(!call.mic_muted.known().unwrap_or(false));
            }
            CallAction::MicSet { muted } => call.mic_muted = Tri::Known(muted),
            CallAction::CameraToggle => {
                call.camera_on = Tri::Known(!call.camera_on.known().unwrap_or(false));
            }
            CallAction::CameraSet { on } => call.camera_on = Tri::Known(on),
        }
        Some(ProviderEvent::Call(call.clone()))
    }
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CallProvider for MockProvider {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }

    fn discover(&mut self, _now: Instant) -> Discovery {
        Discovery {
            availability: self.availability.clone(),
            capabilities: self.capabilities.clone(),
            app_version: AppVersion::new("mock-1.0"),
        }
    }

    fn connect(&mut self, cx: &ProviderCx) -> Result<(), ProviderError> {
        if !cx.consent.observe {
            return Err(ProviderError::ConsentNotGiven);
        }
        self.session = Some(cx.session);
        self.timeline.push_front((
            Instant::ZERO,
            ProviderEvent::Connected { session: cx.session },
        ));
        Ok(())
    }

    fn disconnect(&mut self) {
        self.session = None;
        self.pending.clear();
    }

    fn poll(&mut self, now: Instant) -> Vec<ProviderEvent> {
        self.clock = self.clock.max(now);
        let mut events = Vec::new();
        while self.timeline.front().is_some_and(|(t, _)| *t <= now) {
            let Some((_, mut event)) = self.timeline.pop_front() else {
                break;
            };
            match &mut event {
                // A reconnect of a restart gets a new session.
                ProviderEvent::Connected { session } if session.0 == 0 => {
                    *session = self.next_session();
                }
                ProviderEvent::Disconnected { .. } => self.pending.clear(),
                ProviderEvent::Call(observation) => {
                    self.calls.insert(observation.key.clone(), observation.clone());
                }
                ProviderEvent::CallGone { key } => {
                    self.calls.remove(key);
                }
                _ => {}
            }
            events.push(event);
        }
        let (due, waiting): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.pending).into_iter().partition(|(t, _, _)| *t <= now);
        self.pending = waiting;
        for (at, token, command) in due {
            events.push(ProviderEvent::CommandOutcome {
                token,
                outcome: CommandOutcome::Confirmed,
            });
            if let Some(observation) = self.apply(&command, at) {
                events.push(observation);
            }
        }
        events
    }

    fn command(&mut self, command: CallCommand, token: CommandToken) -> Result<(), CommandRefusal> {
        if self.session != Some(token.session) {
            return Err(CommandRefusal::NotConnected);
        }
        let support = self.capabilities.get(command.action.capability());
        if !support.allows_command() {
            return Err(CommandRefusal::Unsupported(match support {
                Support::Unsupported { reason } => reason,
                _ => UnsupportedReason::ProviderHasNoApi,
            }));
        }
        if !self.calls.contains_key(&command.key) {
            return Err(CommandRefusal::UnknownCall);
        }
        self.sent.push(command.clone());
        let response = self
            .responses
            .get(action_name(command.action))
            .cloned()
            .unwrap_or(Response::ConfirmAfter(Duration::from_millis(200)));
        match response {
            Response::ConfirmAfter(delay) => {
                // Confirmed `delay` after the command (on the last poll's
                // clock); a later poll delivers it.
                self.pending.push((self.clock.after(delay), token, command));
            }
            Response::Reject(code) => self.timeline.push_front((
                Instant::ZERO,
                ProviderEvent::CommandOutcome {
                    token,
                    outcome: CommandOutcome::Rejected {
                        code: FailureCode::new(&code),
                    },
                },
            )),
            Response::Unknown => self.timeline.push_front((
                Instant::ZERO,
                ProviderEvent::CommandOutcome {
                    token,
                    outcome: CommandOutcome::Unknown,
                },
            )),
            Response::Never => {}
        }
        Ok(())
    }
}
