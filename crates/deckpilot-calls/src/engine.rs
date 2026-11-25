//! The call state engine (`docs/calls.md` §State engine).
//!
//! The engine turns provider events into call tracks with the states
//! `idle`, `incoming`, `outgoing`, `connecting`, `active`, `ending`,
//! `ended`, `failed` and `unknown`, picks the focused call, and says
//! whether the Calls overlay should be shown and on which page.
//!
//! - **Pure and deterministic.** No I/O and no clock reads: every call that
//!   needs the time is given `now`, and [`Engine::next_wake`] says when
//!   [`Engine::tick`] is due next. Tests run on a virtual clock.
//! - **No state without an exit.** Every live state has a timeout
//!   ([`Timeouts`]); `ended` and `failed` linger briefly and go.
//! - **No optimistic success.** A command never changes a state by itself:
//!   only a confirming observation (or, for *end*, the provider's
//!   confirmation) does. A command that is not confirmed in time ends as
//!   unconfirmed.
//! - **Full snapshots.** Each observation describes a whole call, so a
//!   lost event is repaired by the next one; a provider that reconnects
//!   reconciles its calls from its first snapshot.
//! - **Nothing is persisted.** After an agent restart the engine starts
//!   empty, and the providers report what is still going on.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::clock::Instant;
use crate::ops::{GateError, GateInput, Plan, gate};
use crate::provider::{
    CallAction, CallCommand, CallKey, CallKind, CallObservation, Capability, CapabilitySet,
    CommandOutcome, CommandRefusal, CommandToken, ConsentState, Direction, DisconnectReason,
    Evidence, ObservedEnd, ObservedPhase, Private, ProviderEvent, ProviderId, ProviderSession, Tri,
    UnsupportedReason,
};
use crate::snapshot::{CallSnapshot, CallState, EndCause, FailureCode};

#[cfg(test)]
#[path = "engine/tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "engine/proptests.rs"]
mod proptests;

/// Most call tracks the engine holds. Beyond it a new call is not
/// tracked (a provider flooding calls cannot grow the engine).
pub const MAX_TRACKS: usize = 4;

/// Most operating-system observations waiting for [`Timeouts::infer_on`].
/// Beyond it a new one is dropped (and reported as an overflow).
pub const MAX_CANDIDATES: usize = 8;

/// Most calls remembered after their track went, so that a late
/// observation of an ended call does not bring it back.
pub const MAX_ENDED_MARKS: usize = 16;

/// Most events kept between two [`Engine::drain_events`]; beyond it the
/// oldest go (the caller drains after every step, so this is a bound, not
/// a queue).
pub const MAX_EVENTS: usize = 256;

/// The engine's timeouts. [`Timeouts::default`] is the normative set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// An incoming call that rings longer is missed.
    pub ring_max: Duration,
    /// An outgoing call that dials longer is unknown.
    pub dial_max: Duration,
    /// A call that connects longer is unknown.
    pub connect_max: Duration,
    /// A call that ends longer has ended.
    pub ending_max: Duration,
    /// An active call with no news for longer is unknown (then its
    /// provider's next snapshot reconciles it).
    pub active_max: Duration,
    /// An unknown call without evidence for longer is lost.
    pub stale_max: Duration,
    /// An unknown call that operating-system signals still show stays
    /// unknown at most this long.
    pub inferred_max: Duration,
    /// How long `failed` is shown.
    pub failed_show: Duration,
    /// How long `ended` is shown.
    pub ended_show: Duration,
    /// How long a command waits for its confirmation.
    pub command_max: Duration,
    /// How long "not confirmed" is shown.
    pub unconfirmed_show: Duration,
    /// Operating-system signals must show a call this long before it is
    /// active.
    pub infer_on: Duration,
    /// ... and be gone this long before it has ended.
    pub infer_off: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            ring_max: Duration::from_secs(90),
            dial_max: Duration::from_secs(120),
            connect_max: Duration::from_secs(60),
            ending_max: Duration::from_secs(10),
            active_max: Duration::from_secs(24 * 60 * 60),
            stale_max: Duration::from_secs(15),
            inferred_max: Duration::from_secs(4 * 60 * 60),
            failed_show: Duration::from_secs(3),
            ended_show: Duration::from_secs(2),
            command_max: Duration::from_secs(5),
            unconfirmed_show: Duration::from_secs(2),
            infer_on: Duration::from_secs(2),
            infer_off: Duration::from_secs(8),
        }
    }
}

/// The id of one call track, never reused within an engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TrackId(pub u64);

/// The id of one showing of the overlay: from the moment a call needs it
/// until no call does. A meeting that follows a meeting keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CallSession(pub u64);

/// The overlay's page for the focused call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OverlayPage {
    Incoming,
    Outgoing,
    Active,
}

/// What the overlay should be, on every device that has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayIntent {
    Hidden,
    Shown {
        session: CallSession,
        page: OverlayPage,
        /// Bumped whenever the focused call changes, so faces redraw.
        generation: u64,
    },
}

/// How a command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandEnd {
    /// An observation (or for *end* the provider) confirmed it.
    Confirmed,
    /// Not confirmed in time, or the provider cannot confirm it.
    Unconfirmed,
    /// The provider refused it (a stable code).
    Rejected { code: String },
}

/// What happened, for the caller to log and act on
/// ([`Engine::drain_events`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineEvent {
    Transition {
        track: TrackId,
        provider: ProviderId,
        from: CallState,
        to: CallState,
    },
    CommandDone {
        token: CommandToken,
        end: CommandEnd,
    },
    /// A provider left out capabilities; they are unsupported. Logged once
    /// per provider.
    MissingCapabilities { provider: ProviderId, count: usize },
    /// A new call was not tracked: [`MAX_TRACKS`] calls are live.
    Overflow { provider: ProviderId },
}

/// A command the caller sends to the provider
/// ([`crate::CallProvider::command`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Send {
        provider: ProviderId,
        command: CallCommand,
        token: CommandToken,
    },
    /// The call already is in the asked-for state: nothing to send.
    AlreadyThere,
}

#[derive(Debug, Clone)]
struct Pending {
    token: CommandToken,
    action: CallAction,
    /// The microphone or camera state that confirms a toggle or a set.
    wanted: Option<bool>,
    deadline: Instant,
}

#[derive(Debug, Clone)]
struct Track {
    id: TrackId,
    provider: ProviderId,
    key: CallKey,
    state: CallState,
    entered: Instant,
    /// When the engine last had an observation of it (an active call's
    /// timeout runs from here).
    last_seen: Instant,
    observed_at: Instant,
    /// When it last became active (focus order).
    connected_at: Option<Instant>,
    kind: CallKind,
    direction: Option<Direction>,
    mic: Tri,
    camera: Tri,
    title: Option<Private<String>>,
    evidence: Evidence,
    /// Operating-system evidence of an active call went away at this
    /// instant (hysteresis).
    signal_lost_at: Option<Instant>,
    pending: Option<Pending>,
    unconfirmed_until: Option<Instant>,
    /// When it ended or failed (engine clock).
    ended_at: Option<Instant>,
}

/// What is remembered of an ended call: its last observation's time and
/// when it ended. An observation made before either is late; one made at
/// the same instant or after is a new call.
#[derive(Debug, Clone, Copy)]
struct EndMark {
    observed: Instant,
    at: Instant,
}

impl EndMark {
    fn is_late(self, obs: &CallObservation) -> bool {
        obs.observed_at < self.observed || obs.observed_at < self.at
    }
}

#[derive(Debug, Clone)]
struct Provider {
    session: Option<ProviderSession>,
    /// What the host registered: a provider's own `Capabilities` event can
    /// narrow it, never widen it.
    registered: CapabilitySet,
    capabilities: CapabilitySet,
    consent: ConsentState,
    missing_reported: bool,
}

/// An operating-system observation of a call not confirmed yet
/// ([`Timeouts::infer_on`]).
#[derive(Debug, Clone)]
struct Candidate {
    first_seen: Instant,
    observation: CallObservation,
}

/// The call state engine.
#[derive(Debug, Clone)]
pub struct Engine {
    timeouts: Timeouts,
    providers: BTreeMap<ProviderId, Provider>,
    tracks: Vec<Track>,
    candidates: BTreeMap<(ProviderId, CallKey), Candidate>,
    /// Calls whose track went ([`MAX_ENDED_MARKS`]).
    ended: BTreeMap<(ProviderId, CallKey), EndMark>,
    next_track: u64,
    next_serial: u64,
    next_session: u64,
    overlay_session: Option<CallSession>,
    generation: u64,
    events: Vec<EngineEvent>,
    /// Whether operating-system evidence alone may show the overlay
    /// (default on).
    overlay_on_inferred: bool,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(Timeouts::default())
    }
}

impl Engine {
    pub fn new(timeouts: Timeouts) -> Self {
        Self {
            timeouts,
            providers: BTreeMap::new(),
            tracks: Vec::new(),
            candidates: BTreeMap::new(),
            ended: BTreeMap::new(),
            next_track: 1,
            next_serial: 1,
            next_session: 1,
            overlay_session: None,
            generation: 0,
            events: Vec::new(),
            overlay_on_inferred: true,
        }
    }

    pub fn timeouts(&self) -> Timeouts {
        self.timeouts
    }

    /// Whether a call known only from operating-system signals shows the
    /// overlay (default `true`).
    pub fn set_overlay_on_inferred(&mut self, on: bool) {
        self.overlay_on_inferred = on;
        self.generation += 1;
    }

    // ------------------------------------------------------------ providers

    /// Adds a provider (not connected yet), with its consents and
    /// capabilities. Adding one again updates them. A consent withdrawn
    /// takes effect at once: without `observe` its calls end as lost and
    /// nothing it observed is kept; without `titles` their titles go;
    /// without `control` their pending commands end unconfirmed.
    pub fn add_provider(
        &mut self,
        id: ProviderId,
        consent: ConsentState,
        capabilities: CapabilitySet,
    ) {
        self.apply_consent(&id, consent);
        let entry = self.providers.entry(id).or_insert(Provider {
            session: None,
            registered: capabilities.clone(),
            capabilities: capabilities.clone(),
            consent,
            missing_reported: false,
        });
        entry.consent = consent;
        entry.registered = capabilities.clone();
        entry.capabilities = capabilities;
        self.generation += 1;
    }

    /// Applies `consent` to what the engine holds of provider `id`.
    fn apply_consent(&mut self, id: &ProviderId, consent: ConsentState) {
        if !consent.observe {
            self.candidates.retain(|(p, _), _| p != id);
        }
        // There is no clock here: a withdrawn consent ends calls at the
        // instant they last changed, which is in the past.
        for index in 0..self.tracks.len() {
            if &self.tracks[index].provider != id {
                continue;
            }
            if !consent.titles {
                self.tracks[index].title = None;
            }
            if !consent.control || !consent.observe {
                self.fail_pending(index);
            }
            if !consent.observe && self.tracks[index].state.is_live() {
                let at = self.tracks[index].last_seen;
                self.enter(
                    index,
                    CallState::Ended {
                        cause: EndCause::Lost,
                    },
                    at,
                );
            }
        }
        self.after_change();
    }

    /// Removes a provider (disabled, consent withdrawn): its calls end as
    /// lost and their pending commands as unconfirmed.
    pub fn remove_provider(&mut self, id: &ProviderId, now: Instant) {
        if self.providers.remove(id).is_none() {
            return;
        }
        self.candidates.retain(|(p, _), _| p != id);
        self.ended.retain(|(p, _), _| p != id);
        for index in 0..self.tracks.len() {
            if &self.tracks[index].provider == id && self.tracks[index].state.is_live() {
                self.fail_pending(index);
                self.enter(
                    index,
                    CallState::Ended {
                        cause: EndCause::Lost,
                    },
                    now,
                );
            }
        }
        self.after_change();
    }

    /// The provider reported a protocol error: its live calls fail.
    pub fn provider_failed(&mut self, id: &ProviderId, code: &str, now: Instant) {
        let code = FailureCode::new(code);
        for index in 0..self.tracks.len() {
            if &self.tracks[index].provider == id && self.tracks[index].state.is_live() {
                self.fail_pending(index);
                self.enter(index, CallState::Failed { code: code.clone() }, now);
            }
        }
        self.after_change();
    }

    /// One event of provider `id`. Events of a provider the engine does not
    /// know are dropped.
    pub fn on_event(&mut self, id: &ProviderId, event: ProviderEvent, now: Instant) {
        if !self.providers.contains_key(id) {
            return;
        }
        match event {
            ProviderEvent::Connected { session } => self.connected(id, session),
            ProviderEvent::Disconnected { reason } => self.disconnected(id, reason, now),
            // Narrowed to what the host registered: a detection-only
            // provider cannot make itself controllable.
            ProviderEvent::Capabilities(set) => {
                if let Some(provider) = self.providers.get_mut(id) {
                    provider.capabilities = set.narrowed_to(&provider.registered);
                    self.generation += 1;
                }
            }
            ProviderEvent::Call(observation) => {
                // Nothing is observed without the user's consent.
                if self.providers.get(id).is_some_and(|p| p.consent.observe) {
                    self.observe(id, observation, now);
                }
            }
            ProviderEvent::CallGone { key } => self.gone(id, &key, now),
            ProviderEvent::CommandOutcome { token, outcome } => {
                self.outcome(id, token, outcome, now);
            }
        }
        self.after_change();
    }

    /// Records that a provider left out `count` capabilities (from
    /// [`CapabilitySet::from_reported`]); reported once per provider.
    pub fn report_missing(&mut self, id: &ProviderId, count: usize) {
        if count == 0 {
            return;
        }
        if let Some(provider) = self.providers.get_mut(id)
            && !provider.missing_reported
        {
            provider.missing_reported = true;
            self.emit(EngineEvent::MissingCapabilities {
                provider: id.clone(),
                count,
            });
        }
    }

    fn connected(&mut self, id: &ProviderId, session: ProviderSession) {
        let Some(provider) = self.providers.get_mut(id) else {
            return;
        };
        provider.session = Some(session);
        // Commands of an older connection can no longer be confirmed.
        for index in 0..self.tracks.len() {
            if &self.tracks[index].provider == id
                && self.tracks[index]
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.token.session != session)
            {
                self.fail_pending(index);
            }
        }
    }

    fn disconnected(&mut self, id: &ProviderId, reason: DisconnectReason, now: Instant) {
        if let Some(provider) = self.providers.get_mut(id) {
            provider.session = None;
        }
        self.candidates.retain(|(p, _), _| p != id);
        for index in 0..self.tracks.len() {
            if &self.tracks[index].provider != id || !self.tracks[index].state.is_live() {
                continue;
            }
            self.fail_pending(index);
            let to = if reason == DisconnectReason::Protocol {
                CallState::Failed {
                    code: FailureCode::new(FailureCode::PROTOCOL),
                }
            } else {
                CallState::Unknown
            };
            self.enter(index, to, now);
        }
    }

    // --------------------------------------------------------- observations

    fn find(&self, provider: &ProviderId, key: &CallKey) -> Option<usize> {
        // The newest track of the call: an ended one lingers next to none.
        self.tracks
            .iter()
            .rposition(|t| &t.provider == provider && &t.key == key)
    }

    fn observe(&mut self, id: &ProviderId, mut obs: CallObservation, now: Instant) {
        let titles = self.providers.get(id).is_some_and(|p| p.consent.titles);
        if !titles {
            obs.title = None;
        }
        match self.find(id, &obs.key) {
            Some(index) if self.tracks[index].state.is_live() => {
                // Older than what the track already shows: out of order.
                if obs.observed_at < self.tracks[index].observed_at {
                    return;
                }
                self.update(index, obs, now);
            }
            Some(index) => {
                let track = &self.tracks[index];
                let mark = EndMark {
                    observed: track.observed_at,
                    at: track.ended_at.unwrap_or(track.entered),
                };
                // A late observation of the ended call does not revive it;
                // one made after its end is a new call.
                if !mark.is_late(&obs) {
                    self.start(id, obs, now);
                }
            }
            None => {
                let late = self
                    .ended
                    .get(&(id.clone(), obs.key.clone()))
                    .is_some_and(|mark| mark.is_late(&obs));
                if !late {
                    self.start(id, obs, now);
                }
            }
        }
    }

    /// Remembers the call of track `index`, which is going.
    fn remember_end(&mut self, index: usize) {
        let track = &self.tracks[index];
        let mark = EndMark {
            observed: track.observed_at,
            at: track.ended_at.unwrap_or(track.entered),
        };
        let key = (track.provider.clone(), track.key.clone());
        self.ended.insert(key, mark);
        while self.ended.len() > MAX_ENDED_MARKS {
            let oldest = self
                .ended
                .iter()
                .min_by_key(|(_, m)| m.at)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(key) => self.ended.remove(&key),
                None => break,
            };
        }
    }

    /// Removes track `index`, remembering its call.
    fn drop_track(&mut self, index: usize) {
        self.remember_end(index);
        self.tracks.remove(index);
    }

    /// Keeps `event` for [`Engine::drain_events`], at most [`MAX_EVENTS`].
    fn emit(&mut self, event: EngineEvent) {
        if self.events.len() >= MAX_EVENTS {
            self.events.remove(0);
        }
        self.events.push(event);
    }

    /// A call the engine does not track yet.
    fn start(&mut self, id: &ProviderId, obs: CallObservation, now: Instant) {
        let to = match obs.phase {
            ObservedPhase::Ringing if obs.direction != Some(Direction::Outgoing) => {
                CallState::Incoming
            }
            ObservedPhase::Dialing | ObservedPhase::Ringing => CallState::Outgoing,
            ObservedPhase::Connected => CallState::Active,
            // Not a way into a call (§ transitions): nothing is tracked.
            ObservedPhase::Connecting
            | ObservedPhase::Ending
            | ObservedPhase::Ended { .. }
            | ObservedPhase::Unknown => {
                self.candidates.remove(&(id.clone(), obs.key.clone()));
                return;
            }
        };
        // Operating-system evidence must hold for a moment first.
        if to == CallState::Active && obs.source == Evidence::OsSignal {
            let key = (id.clone(), obs.key.clone());
            let full = self.candidates.len() >= MAX_CANDIDATES;
            match self.candidates.get_mut(&key) {
                Some(candidate) => {
                    if obs.observed_at < candidate.observation.observed_at {
                        return;
                    }
                    candidate.observation = obs;
                    if now.since(candidate.first_seen) < self.timeouts.infer_on {
                        return;
                    }
                    let candidate = self.candidates.remove(&key);
                    if let Some(candidate) = candidate {
                        self.create(id, candidate.observation, CallState::Active, now);
                    }
                }
                None if full => {
                    self.emit(EngineEvent::Overflow {
                        provider: id.clone(),
                    });
                }
                None => {
                    self.candidates.insert(
                        key,
                        Candidate {
                            first_seen: now,
                            observation: obs,
                        },
                    );
                }
            }
            return;
        }
        self.create(id, obs, to, now);
    }

    fn create(&mut self, id: &ProviderId, obs: CallObservation, to: CallState, now: Instant) {
        // Ended and failed tracks make room first.
        if self.tracks.len() >= MAX_TRACKS {
            if let Some(index) = self.tracks.iter().position(|t| !t.state.is_live()) {
                self.drop_track(index);
            } else {
                self.emit(EngineEvent::Overflow {
                    provider: id.clone(),
                });
                return;
            }
        }
        let track = Track {
            id: TrackId(self.next_track),
            provider: id.clone(),
            key: obs.key,
            state: CallState::Idle,
            entered: now,
            last_seen: now,
            observed_at: obs.observed_at,
            connected_at: None,
            kind: obs.kind,
            direction: obs.direction,
            mic: obs.mic_muted,
            camera: obs.camera_on,
            title: obs.title,
            evidence: obs.source,
            signal_lost_at: None,
            pending: None,
            unconfirmed_until: None,
            ended_at: None,
        };
        self.next_track += 1;
        self.tracks.push(track);
        let index = self.tracks.len() - 1;
        self.enter(index, to, now);
    }

    /// A new observation of a live track.
    fn update(&mut self, index: usize, obs: CallObservation, now: Instant) {
        let from = self.tracks[index].state.clone();
        {
            let track = &mut self.tracks[index];
            track.last_seen = now;
            track.observed_at = obs.observed_at;
            track.kind = obs.kind;
            if obs.direction.is_some() {
                track.direction = obs.direction;
            }
            track.mic = obs.mic_muted;
            track.camera = obs.camera_on;
            track.title = obs.title;
            track.evidence = obs.source;
            track.signal_lost_at = None;
        }
        let to = match (&from, obs.phase) {
            // The same phase again (incoming ringing, active connected, ...)
            // is no transition; see the last arm.
            (CallState::Incoming | CallState::Outgoing, ObservedPhase::Connecting) => {
                Some(CallState::Connecting)
            }
            (
                CallState::Incoming
                | CallState::Outgoing
                | CallState::Connecting
                | CallState::Unknown,
                ObservedPhase::Connected,
            ) => Some(CallState::Active),
            (CallState::Active | CallState::Unknown, ObservedPhase::Ending) => {
                Some(CallState::Ending)
            }
            (CallState::Unknown, ObservedPhase::Ringing) => Some(CallState::Incoming),
            (CallState::Unknown, ObservedPhase::Dialing) => Some(CallState::Outgoing),
            (CallState::Unknown, ObservedPhase::Connecting) => Some(CallState::Connecting),
            (_, ObservedPhase::Ended { cause }) => Some(CallState::Ended {
                cause: self.end_cause(index, &from, Some(cause)),
            }),
            (state, ObservedPhase::Unknown) if *state != CallState::Unknown => {
                Some(CallState::Unknown)
            }
            // Any other change is not a transition: the state stays.
            _ => None,
        };
        if let Some(to) = to {
            self.enter(index, to, now);
        }
        self.confirm_pending(index);
    }

    /// The cause of an end in state `from` (§ transitions): the provider's
    /// cause where it fits the state, else what the state implies.
    fn end_cause(&self, index: usize, from: &CallState, observed: Option<ObservedEnd>) -> EndCause {
        let pending = self.tracks[index].pending.as_ref().map(|p| p.action);
        match from {
            CallState::Incoming => match observed {
                Some(ObservedEnd::Rejected) => EndCause::Rejected,
                Some(ObservedEnd::AnsweredElsewhere) => EndCause::AnsweredElsewhere,
                _ if pending == Some(CallAction::Reject) => EndCause::Rejected,
                _ => EndCause::Missed,
            },
            CallState::Outgoing | CallState::Connecting => match observed {
                Some(ObservedEnd::Cancelled) => EndCause::Cancelled,
                Some(ObservedEnd::Declined) => EndCause::Declined,
                Some(ObservedEnd::NoAnswer) => EndCause::NoAnswer,
                Some(ObservedEnd::RemoteHangUp) => EndCause::RemoteHangUp,
                Some(ObservedEnd::Left) => EndCause::Left,
                _ if matches!(pending, Some(CallAction::CancelOutgoing | CallAction::End)) => {
                    EndCause::Cancelled
                }
                Some(_) => EndCause::Unknown,
                None => EndCause::NoAnswer,
            },
            _ => match observed {
                Some(ObservedEnd::RemoteHangUp) => EndCause::RemoteHangUp,
                Some(ObservedEnd::Left) => EndCause::Left,
                _ if pending == Some(CallAction::End) || *from == CallState::Ending => {
                    EndCause::Left
                }
                _ => EndCause::Unknown,
            },
        }
    }

    fn gone(&mut self, id: &ProviderId, key: &CallKey, now: Instant) {
        self.candidates.remove(&(id.clone(), key.clone()));
        let Some(index) = self.find(id, key) else {
            return;
        };
        if !self.tracks[index].state.is_live() {
            return;
        }
        let track = &self.tracks[index];
        // Operating-system evidence flaps: an active call ends only after
        // the signal stayed away (`infer_off`).
        if track.evidence == Evidence::OsSignal && track.state == CallState::Active {
            if track.signal_lost_at.is_none() {
                self.tracks[index].signal_lost_at = Some(now);
            }
            return;
        }
        let from = track.state.clone();
        let cause = self.end_cause(index, &from, None);
        self.enter(index, CallState::Ended { cause }, now);
        self.confirm_pending(index);
    }

    // ------------------------------------------------------------- commands

    /// Asks for `action` on the focused call. Passes the gate
    /// ([`crate::ops::gate`]) with the provider's consent and
    /// capabilities; the command is pending until confirmed. One command
    /// at a time per call: a second one while the first waits is refused
    /// as unconfirmed.
    pub fn request(&mut self, action: CallAction, now: Instant) -> Result<Request, GateError> {
        let Some(index) = self.focus_index() else {
            return Err(GateError::NotInCall);
        };
        let track = &self.tracks[index];
        let Some(provider) = self.providers.get(&track.provider) else {
            return Err(GateError::ProviderOffline);
        };
        let plan = gate(
            GateInput {
                state: &track.state,
                mic_muted: track.mic,
                camera_on: track.camera,
                capabilities: &provider.capabilities,
                connected: provider.session.is_some(),
                consent: provider.consent,
            },
            action,
        )?;
        let send = match plan {
            Plan::AlreadyThere => return Ok(Request::AlreadyThere),
            Plan::Send(send) => send,
        };
        if track.pending.is_some() {
            return Err(GateError::Unconfirmed);
        }
        let Some(session) = provider.session else {
            return Err(GateError::ProviderOffline);
        };
        let token = CommandToken {
            session,
            serial: self.next_serial,
        };
        self.next_serial += 1;
        // A microphone or camera command is confirmed only by a state the
        // app itself reports: without a readable state there is nothing
        // that could confirm it, and it ends unconfirmed.
        let reads = |capability| provider.capabilities.get(capability).allows_command();
        let wanted = match action {
            CallAction::MicSet { muted } if reads(Capability::MicReadState) => Some(muted),
            CallAction::MicToggle if reads(Capability::MicReadState) => {
                track.mic.known().map(|m| !m)
            }
            CallAction::CameraSet { on } if reads(Capability::CameraReadState) => Some(on),
            CallAction::CameraToggle if reads(Capability::CameraReadState) => {
                track.camera.known().map(|c| !c)
            }
            _ => None,
        };
        let request = Request::Send {
            provider: track.provider.clone(),
            command: CallCommand {
                key: track.key.clone(),
                action: send,
            },
            token,
        };
        let track = &mut self.tracks[index];
        track.pending = Some(Pending {
            token,
            action,
            wanted,
            deadline: now.after(self.timeouts.command_max),
        });
        track.unconfirmed_until = None;
        self.generation += 1;
        Ok(request)
    }

    /// The provider refused a command before sending it.
    pub fn command_refused(&mut self, token: CommandToken, refusal: &CommandRefusal) {
        let Some(index) = self.pending_index(token) else {
            return;
        };
        self.tracks[index].pending = None;
        let code = match refusal {
            CommandRefusal::Unsupported(reason) => reason.code(),
            CommandRefusal::NotConnected => "not-connected",
            CommandRefusal::UnknownCall => "unknown-call",
            CommandRefusal::Busy => "busy",
        };
        self.emit(EngineEvent::CommandDone {
            token,
            end: CommandEnd::Rejected { code: code.into() },
        });
        self.generation += 1;
    }

    fn pending_index(&self, token: CommandToken) -> Option<usize> {
        self.tracks
            .iter()
            .position(|t| t.pending.as_ref().is_some_and(|p| p.token == token))
    }

    fn outcome(
        &mut self,
        id: &ProviderId,
        token: CommandToken,
        outcome: CommandOutcome,
        now: Instant,
    ) {
        // Only the current connection's own commands: a stale or forged
        // outcome is dropped.
        let current = self.providers.get(id).and_then(|p| p.session);
        if current != Some(token.session) {
            return;
        }
        let Some(index) = self.pending_index(token) else {
            return;
        };
        if &self.tracks[index].provider != id {
            return;
        }
        match outcome {
            CommandOutcome::Confirmed => {
                let action = self.tracks[index].pending.as_ref().map(|p| p.action);
                // Ending a call is confirmed by the provider itself.
                if action == Some(CallAction::End)
                    && matches!(self.tracks[index].state, CallState::Active)
                {
                    self.enter(index, CallState::Ending, now);
                    self.finish_pending(index, CommandEnd::Confirmed);
                } else {
                    self.confirm_pending(index);
                }
            }
            CommandOutcome::Rejected { code } => {
                let code = code.as_str().to_owned();
                self.finish_pending(index, CommandEnd::Rejected { code });
            }
            CommandOutcome::Unknown => self.unconfirmed(index, now),
        }
    }

    /// Resolves the pending command of track `index` when the track shows
    /// it happened.
    fn confirm_pending(&mut self, index: usize) {
        let track = &self.tracks[index];
        let Some(pending) = &track.pending else {
            return;
        };
        let done = match pending.action {
            CallAction::Answer => matches!(track.state, CallState::Connecting | CallState::Active),
            CallAction::Reject => matches!(
                track.state,
                CallState::Ended {
                    cause: EndCause::Rejected
                }
            ),
            CallAction::CancelOutgoing => matches!(track.state, CallState::Ended { .. }),
            CallAction::End => matches!(track.state, CallState::Ending | CallState::Ended { .. }),
            // Only the app's own report confirms: an inferred or
            // operating-system state never does.
            CallAction::MicToggle | CallAction::MicSet { .. } => {
                track.evidence == Evidence::AppApi
                    && pending.wanted.is_some_and(|w| track.mic == Tri::Known(w))
            }
            CallAction::CameraToggle | CallAction::CameraSet { .. } => {
                track.evidence == Evidence::AppApi
                    && pending
                        .wanted
                        .is_some_and(|w| track.camera == Tri::Known(w))
            }
        };
        if done {
            self.finish_pending(index, CommandEnd::Confirmed);
        } else if !track.state.is_live() {
            // The call ended another way: the command did not happen.
            self.finish_pending(index, CommandEnd::Unconfirmed);
        }
    }

    fn finish_pending(&mut self, index: usize, end: CommandEnd) {
        if let Some(pending) = self.tracks[index].pending.take() {
            self.emit(EngineEvent::CommandDone {
                token: pending.token,
                end,
            });
            self.generation += 1;
        }
    }

    fn unconfirmed(&mut self, index: usize, now: Instant) {
        if self.tracks[index].pending.is_some() {
            self.tracks[index].unconfirmed_until = Some(now.after(self.timeouts.unconfirmed_show));
            self.finish_pending(index, CommandEnd::Unconfirmed);
        }
    }

    fn fail_pending(&mut self, index: usize) {
        self.finish_pending(index, CommandEnd::Unconfirmed);
    }

    // ---------------------------------------------------------------- clock

    fn enter(&mut self, index: usize, to: CallState, now: Instant) {
        let track = &mut self.tracks[index];
        if track.state == to {
            return;
        }
        let from = std::mem::replace(&mut track.state, to);
        let to = track.state.clone();
        track.entered = now;
        if to == CallState::Active {
            track.connected_at = Some(now);
        }
        if !to.is_live() {
            track.ended_at = Some(now);
        }
        let event = EngineEvent::Transition {
            track: track.id,
            provider: track.provider.clone(),
            from,
            to: to.clone(),
        };
        self.emit(event);
        self.generation += 1;
        // Meeting to meeting: a newly active call ends the provider's other
        // active ones.
        if to == CallState::Active {
            let provider = self.tracks[index].provider.clone();
            let id = self.tracks[index].id;
            for other in 0..self.tracks.len() {
                let t = &self.tracks[other];
                if t.id != id
                    && t.provider == provider
                    && matches!(t.state, CallState::Active | CallState::Ending)
                {
                    self.fail_pending(other);
                    self.enter(
                        other,
                        CallState::Ended {
                            cause: EndCause::Left,
                        },
                        now,
                    );
                }
            }
        }
    }

    /// When a track's state times out.
    fn deadline(&self, track: &Track) -> Option<Instant> {
        let t = &self.timeouts;
        let after = |d: Duration| Some(track.entered.after(d));
        let state = match track.state {
            CallState::Incoming => after(t.ring_max),
            CallState::Outgoing => after(t.dial_max),
            CallState::Connecting => after(t.connect_max),
            CallState::Ending => after(t.ending_max),
            CallState::Active => Some(track.last_seen.after(t.active_max)),
            CallState::Unknown => after(if self.inferred_evidence(&track.provider) {
                t.inferred_max
            } else {
                t.stale_max
            }),
            CallState::Failed { .. } => after(t.failed_show),
            CallState::Ended { .. } => after(t.ended_show),
            CallState::Idle => None,
        };
        [
            state,
            track.pending.as_ref().map(|p| p.deadline),
            track.unconfirmed_until,
            track.signal_lost_at.map(|at| at.after(t.infer_off)),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Whether provider `provider`, connected, shows an active call from
    /// operating-system signals (an unknown call of it may still be going
    /// on). Another provider's signals say nothing about it.
    fn inferred_evidence(&self, provider: &ProviderId) -> bool {
        self.tracks.iter().any(|t| {
            &t.provider == provider
                && t.evidence == Evidence::OsSignal
                && t.state == CallState::Active
                && t.signal_lost_at.is_none()
                && self
                    .providers
                    .get(&t.provider)
                    .is_some_and(|p| p.session.is_some())
        })
    }

    /// When [`Engine::tick`] is due next.
    pub fn next_wake(&self) -> Option<Instant> {
        let candidates = self
            .candidates
            .values()
            .map(|c| c.first_seen.after(self.timeouts.infer_on));
        self.tracks
            .iter()
            .filter_map(|t| self.deadline(t))
            .chain(candidates)
            .min()
    }

    /// Applies every timeout due at `now`.
    pub fn tick(&mut self, now: Instant) {
        // Candidates that held long enough become active calls.
        let ready: Vec<(ProviderId, CallKey)> = self
            .candidates
            .iter()
            .filter(|(_, c)| now.since(c.first_seen) >= self.timeouts.infer_on)
            .map(|(k, _)| k.clone())
            .collect();
        for key in ready {
            if let Some(candidate) = self.candidates.remove(&key) {
                self.create(&key.0, candidate.observation, CallState::Active, now);
            }
        }
        let t = self.timeouts;
        let mut index = 0;
        while index < self.tracks.len() {
            let track = &self.tracks[index];
            if let Some(pending) = &track.pending
                && now >= pending.deadline
            {
                self.unconfirmed(index, now);
            }
            let track = &self.tracks[index];
            if track.unconfirmed_until.is_some_and(|u| now >= u) {
                self.tracks[index].unconfirmed_until = None;
                self.generation += 1;
            }
            let track = &self.tracks[index];
            if let Some(lost) = track.signal_lost_at
                && now >= lost.after(t.infer_off)
            {
                self.tracks[index].signal_lost_at = None;
                // The call went by itself: a command waiting on it did not
                // happen.
                self.fail_pending(index);
                self.enter(
                    index,
                    CallState::Ended {
                        cause: EndCause::Unknown,
                    },
                    now,
                );
            }
            let track = &self.tracks[index];
            let elapsed = now.since(track.entered);
            let next = match &track.state {
                CallState::Incoming if elapsed >= t.ring_max => Some(CallState::Ended {
                    cause: EndCause::Missed,
                }),
                CallState::Outgoing if elapsed >= t.dial_max => Some(CallState::Unknown),
                CallState::Connecting if elapsed >= t.connect_max => Some(CallState::Unknown),
                CallState::Active if now.since(track.last_seen) >= t.active_max => {
                    Some(CallState::Unknown)
                }
                CallState::Ending if elapsed >= t.ending_max => Some(CallState::Ended {
                    cause: EndCause::Left,
                }),
                CallState::Unknown => {
                    let limit = if self.inferred_evidence(&track.provider) {
                        t.inferred_max
                    } else {
                        t.stale_max
                    };
                    (elapsed >= limit).then_some(CallState::Ended {
                        cause: EndCause::Lost,
                    })
                }
                CallState::Ended { .. } if elapsed >= t.ended_show => {
                    self.drop_track(index);
                    self.generation += 1;
                    continue;
                }
                CallState::Failed { .. } if elapsed >= t.failed_show => {
                    self.drop_track(index);
                    self.generation += 1;
                    continue;
                }
                _ => None,
            };
            if let Some(next) = next {
                if !next.is_live() {
                    self.fail_pending(index);
                }
                self.enter(index, next, now);
                // The new state may itself be due (an ended call lingers):
                // look at this track again.
                continue;
            }
            index += 1;
        }
        self.after_change();
    }

    // -------------------------------------------------------------- reading

    /// Focus order (§ focus): incoming (newest first), then outgoing and
    /// connecting, then active and ending (most recently connected), then
    /// unknown, then ended and failed; ties by the latest observation, then
    /// the provider id.
    fn focus_index(&self) -> Option<usize> {
        fn class(state: &CallState) -> u8 {
            match state {
                CallState::Incoming => 0,
                CallState::Outgoing | CallState::Connecting => 1,
                CallState::Active | CallState::Ending => 2,
                CallState::Unknown => 3,
                CallState::Ended { .. } | CallState::Failed { .. } => 4,
                CallState::Idle => 5,
            }
        }
        self.tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.state != CallState::Idle)
            .min_by(|(_, a), (_, b)| {
                class(&a.state)
                    .cmp(&class(&b.state))
                    .then_with(|| match a.state {
                        CallState::Incoming => b.entered.cmp(&a.entered),
                        CallState::Active | CallState::Ending => {
                            b.connected_at.cmp(&a.connected_at)
                        }
                        _ => std::cmp::Ordering::Equal,
                    })
                    .then_with(|| b.observed_at.cmp(&a.observed_at))
                    .then_with(|| a.provider.cmp(&b.provider))
            })
            .map(|(index, _)| index)
    }

    /// The focused call's track id, if there is a call.
    pub fn focus(&self) -> Option<TrackId> {
        self.focus_index().map(|i| self.tracks[i].id)
    }

    /// The state of every track, oldest first.
    pub fn tracks(&self) -> Vec<(TrackId, CallState)> {
        self.tracks
            .iter()
            .map(|t| (t.id, t.state.clone()))
            .collect()
    }

    /// The state of one track; `None` once it is gone.
    pub fn track_state(&self, id: TrackId) -> Option<&CallState> {
        self.tracks.iter().find(|t| t.id == id).map(|t| &t.state)
    }

    /// Bumped on every change.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The focused call as the plugin, the faces and the IPC see it. The
    /// title only with the provider's opt-in (it is dropped from
    /// observations otherwise).
    pub fn snapshot(&self) -> CallSnapshot {
        // Controls only with the control consent, with or without a call.
        let shown = |p: Option<&Provider>| {
            p.map_or_else(
                || CapabilitySet::none(UnsupportedReason::ProviderHasNoApi),
                |p| {
                    if p.consent.control {
                        p.capabilities.clone()
                    } else {
                        p.capabilities
                            .without_control(UnsupportedReason::ConsentNotGiven)
                    }
                },
            )
        };
        let Some(index) = self.focus_index() else {
            let capabilities = shown(self.providers.values().next());
            return CallSnapshot::idle(capabilities, self.generation);
        };
        let track = &self.tracks[index];
        let provider = self.providers.get(&track.provider);
        let capabilities = shown(provider);
        let observation = CallObservation {
            key: track.key.clone(),
            kind: track.kind,
            phase: ObservedPhase::Unknown,
            direction: track.direction,
            mic_muted: track.mic,
            camera_on: track.camera,
            title: track.title.clone(),
            source: track.evidence,
            observed_at: track.observed_at,
        };
        // `since` is the state's entry on the engine clock; the agent maps
        // it to wall time. The title goes through the opt-in once more.
        let mut snapshot = CallSnapshot::from_observation(
            track.state.clone(),
            track.entered.as_millis(),
            &observation,
            capabilities,
            track.provider.clone(),
            provider.map(|p| p.consent).unwrap_or_default(),
            self.generation,
        );
        snapshot.connected = provider.is_some_and(|p| p.session.is_some());
        snapshot.pending = track.pending.as_ref().map(|p| p.action);
        snapshot.unconfirmed = track.unconfirmed_until.is_some();
        snapshot
    }

    /// Whether the focused call shows the overlay, and on which page.
    pub fn overlay(&self) -> OverlayIntent {
        match (self.overlay_session, self.overlay_page()) {
            (Some(session), Some(page)) => OverlayIntent::Shown {
                session,
                page,
                generation: self.generation,
            },
            _ => OverlayIntent::Hidden,
        }
    }

    fn overlay_page(&self) -> Option<OverlayPage> {
        let track = &self.tracks[self.focus_index()?];
        if !self.overlay_on_inferred && track.evidence == Evidence::OsSignal {
            return None;
        }
        match track.state {
            CallState::Incoming => Some(OverlayPage::Incoming),
            CallState::Outgoing | CallState::Connecting => Some(OverlayPage::Outgoing),
            CallState::Active | CallState::Ending | CallState::Unknown => Some(OverlayPage::Active),
            // The overlay goes on entering `ended`, not after its linger.
            CallState::Ended { .. } | CallState::Failed { .. } | CallState::Idle => None,
        }
    }

    /// Keeps the overlay session: a new one when a call needs the overlay
    /// and none did, none when no call does.
    fn after_change(&mut self) {
        match (self.overlay_page(), self.overlay_session) {
            (Some(_), None) => {
                self.overlay_session = Some(CallSession(self.next_session));
                self.next_session += 1;
            }
            (None, Some(_)) => self.overlay_session = None,
            _ => {}
        }
    }

    /// What happened since the last call, oldest first.
    pub fn drain_events(&mut self) -> Vec<EngineEvent> {
        std::mem::take(&mut self.events)
    }
}
