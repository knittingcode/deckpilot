//! The provider adapter contract (`docs/calls.md` §Providers).
//!
//! A provider observes calls or meetings of one app or mechanism and, when
//! that app offers a supported way, controls them. It reports what it can
//! do as a [`CapabilitySet`] that names **every** [`Capability`], each with
//! a [`Support`]: a capability is never assumed to work, and one the
//! provider leaves out is [`Support::Unsupported`] with
//! [`UnsupportedReason::ProviderHasNoApi`].
//!
//! Observations are full snapshots of one call ([`CallObservation`]), never
//! deltas, so a lost event is repaired by the next one. Commands never
//! change state by themselves: their outcome arrives later as
//! [`ProviderEvent::CommandOutcome`], and only an observation that confirms
//! it moves a call.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::clock::Instant;

#[cfg(test)]
#[path = "provider/tests.rs"]
mod tests;

/// The id of a provider: 1 to 32 lowercase letters, digits and single
/// hyphens, starting with a letter (`mock`, `os-signals`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ProviderId(String);

impl ProviderId {
    /// The scripted provider of the tests and the dev seed.
    pub const MOCK: &str = "mock";
    /// Read-only operating-system signals (microphone and camera in use).
    pub const OS_SIGNALS: &str = "os-signals";
    /// Microsoft Graph presence, read-only.
    pub const GRAPH_PRESENCE: &str = "graph-presence";

    pub fn new(id: &str) -> Result<Self, InvalidId> {
        let valid = (1..=32).contains(&id.len())
            && id.starts_with(|c: char| c.is_ascii_lowercase())
            && !id.ends_with('-')
            && !id.contains("--")
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if valid {
            Ok(Self(id.to_owned()))
        } else {
            Err(InvalidId)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ProviderId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::new(&text).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An id that does not follow its rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a valid id")]
pub struct InvalidId;

/// A provider-scoped opaque id of one call or meeting, synthetic when the
/// provider has none. It may be derived from an app's own id, so it is
/// never shown, logged or sent to the plugin: its `Debug` prints only its
/// length.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CallKey(String);

impl CallKey {
    /// The longest key: 128 bytes.
    pub const MAX_LEN: usize = 128;

    pub fn new(key: impl Into<String>) -> Result<Self, InvalidId> {
        let key = key.into();
        if (1..=Self::MAX_LEN).contains(&key.len()) && !key.chars().any(char::is_control) {
            Ok(Self(key))
        } else {
            Err(InvalidId)
        }
    }

    /// The key itself, for the adapter that made it.
    pub fn reveal(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CallKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CallKey(<{} bytes>)", self.0.len())
    }
}

/// The generation of one provider connection. Each successful connect
/// starts a new one; events and command outcomes of an older generation
/// are dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProviderSession(pub u64);

/// The token of one command: the provider session it was sent in and a
/// serial number. An outcome whose token does not match a pending command
/// of the current session is dropped (stale or forged).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CommandToken {
    pub session: ProviderSession,
    pub serial: u64,
}

/// Whether a provider can be used now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Availability {
    Available,
    /// The app is not installed.
    NotInstalled,
    /// The app is installed but not running.
    NotRunning,
    /// The app must be paired with DeckPilot first.
    NeedsPairing,
    /// The app's own setting for external control is off.
    DisabledInApp,
    /// The user declined this provider's consent.
    ConsentDenied,
    /// The provider cannot work here at all.
    Unsupported {
        reason: UnsupportedReason,
    },
    /// A stable error code (`[a-z0-9-]`, at most 48), never a message from
    /// the app.
    Error {
        code: String,
    },
}

/// What one call capability is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Capability {
    DetectIncoming,
    DetectOutgoing,
    DetectMeeting,
    ReadTitle,
    Answer,
    Reject,
    CancelOutgoing,
    End,
    MicToggle,
    MicSet,
    MicReadState,
    CameraToggle,
    CameraSet,
    CameraReadState,
}

impl Capability {
    /// Every capability, in a fixed order.
    pub const ALL: [Self; 14] = [
        Self::DetectIncoming,
        Self::DetectOutgoing,
        Self::DetectMeeting,
        Self::ReadTitle,
        Self::Answer,
        Self::Reject,
        Self::CancelOutgoing,
        Self::End,
        Self::MicToggle,
        Self::MicSet,
        Self::MicReadState,
        Self::CameraToggle,
        Self::CameraSet,
        Self::CameraReadState,
    ];

    /// Whether the capability changes a call (as opposed to observing).
    pub fn is_control(self) -> bool {
        matches!(
            self,
            Self::Answer
                | Self::Reject
                | Self::CancelOutgoing
                | Self::End
                | Self::MicToggle
                | Self::MicSet
                | Self::CameraToggle
                | Self::CameraSet
        )
    }
}

/// When a [`Support::Conditional`] capability works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Condition {
    /// Only while the meeting's host allows it (unmuting, the camera).
    HostAllows,
    /// Only while the user is in a call or meeting.
    InCall,
}

/// Why a capability is not supported: a closed list, each with an English
/// and a Polish text in the plugin, shown on the disabled key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnsupportedReason {
    /// The app offers no supported way (Microsoft Teams has none since its
    /// third-party device API was retired on 2026-06-30).
    ProviderHasNoApi,
    /// Not on this operating system (there is no Teams desktop on Linux).
    PlatformNotSupported,
    NotInMeeting,
    /// The meeting does not allow it now.
    NoPermissionInMeeting,
    /// The provider can only toggle, not set a state.
    ToggleOnly,
    NeedsPairing,
    NeedsOsPermission,
    ConsentNotGiven,
}

impl UnsupportedReason {
    /// The stable code: the plugin's text key and what diagnostics show.
    pub fn code(self) -> &'static str {
        match self {
            Self::ProviderHasNoApi => "provider-has-no-api",
            Self::PlatformNotSupported => "platform-not-supported",
            Self::NotInMeeting => "not-in-meeting",
            Self::NoPermissionInMeeting => "no-permission-in-meeting",
            Self::ToggleOnly => "toggle-only",
            Self::NeedsPairing => "needs-pairing",
            Self::NeedsOsPermission => "needs-os-permission",
            Self::ConsentNotGiven => "consent-not-given",
        }
    }
}

/// How far one capability is supported. Never a bare boolean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Support {
    /// The mechanism exists and the provider confirmed it now.
    Supported,
    /// Supported only while the condition holds.
    Conditional {
        when: Condition,
    },
    /// Can be observed, not controlled.
    ReadOnly,
    /// Derived from operating-system signals (the microphone or camera is
    /// in use), not reported by the app.
    Inferred,
    Unsupported {
        reason: UnsupportedReason,
    },
}

impl Support {
    /// Whether a command of this capability may be sent: `Supported`, or
    /// `Conditional` (the provider refuses or rejects it when the
    /// condition does not hold, which is then shown).
    pub fn allows_command(self) -> bool {
        matches!(self, Self::Supported | Self::Conditional { .. })
    }

    /// Whether the capability can be observed at all.
    pub fn allows_observation(self) -> bool {
        !matches!(self, Self::Unsupported { .. })
    }
}

/// A provider's support for every [`Capability`].
///
/// Built with [`CapabilitySet::from_reported`], which fills in every
/// capability the provider left out as unsupported, so a set always names
/// all of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "BTreeMap<Capability, Support>")]
pub struct CapabilitySet(BTreeMap<Capability, Support>);

impl CapabilitySet {
    /// Every capability unsupported, with `reason`.
    pub fn none(reason: UnsupportedReason) -> Self {
        Self(
            Capability::ALL
                .into_iter()
                .map(|c| (c, Support::Unsupported { reason }))
                .collect(),
        )
    }

    /// The set a provider reported, completed: a capability it left out is
    /// `Unsupported { ProviderHasNoApi }` (a contract violation the caller
    /// logs once; the second value names them). A capability reported twice
    /// keeps the narrower support (it is never widened by a duplicate).
    pub fn from_reported(
        reported: impl IntoIterator<Item = (Capability, Support)>,
    ) -> (Self, Vec<Capability>) {
        let mut map: BTreeMap<Capability, Support> = BTreeMap::new();
        for (capability, support) in reported {
            map.entry(capability)
                .and_modify(|old| *old = narrower(*old, support))
                .or_insert(support);
        }
        let mut missing = Vec::new();
        for capability in Capability::ALL {
            map.entry(capability).or_insert_with(|| {
                missing.push(capability);
                Support::Unsupported {
                    reason: UnsupportedReason::ProviderHasNoApi,
                }
            });
        }
        (Self(map), missing)
    }

    pub fn get(&self, capability: Capability) -> Support {
        self.0
            .get(&capability)
            .copied()
            .unwrap_or(Support::Unsupported {
                reason: UnsupportedReason::ProviderHasNoApi,
            })
    }

    pub fn iter(&self) -> impl Iterator<Item = (Capability, Support)> + '_ {
        self.0.iter().map(|(c, s)| (*c, *s))
    }

    /// Whether any control capability allows a command.
    pub fn controls_anything(&self) -> bool {
        self.iter()
            .any(|(c, s)| c.is_control() && s.allows_command())
    }

    /// The same set with every capability `Unsupported { reason }` where it
    /// allowed commands: what is shown while the provider is offline or the
    /// control consent is missing.
    #[must_use]
    pub fn without_control(&self, reason: UnsupportedReason) -> Self {
        Self(
            self.0
                .iter()
                .map(|(c, s)| {
                    let s = if c.is_control() {
                        Support::Unsupported { reason }
                    } else {
                        *s
                    };
                    (*c, s)
                })
                .collect(),
        )
    }
}

impl TryFrom<BTreeMap<Capability, Support>> for CapabilitySet {
    type Error = String;

    /// A deserialized set must name every capability, as a provider must.
    fn try_from(map: BTreeMap<Capability, Support>) -> Result<Self, String> {
        match Capability::ALL.into_iter().find(|c| !map.contains_key(c)) {
            Some(missing) => Err(format!("the capability set does not name {missing:?}")),
            None => Ok(Self(map)),
        }
    }
}

/// The rank of a support level: lower is narrower.
fn rank(support: Support) -> u8 {
    match support {
        Support::Unsupported { .. } => 0,
        Support::Inferred => 1,
        Support::ReadOnly => 2,
        Support::Conditional { .. } => 3,
        Support::Supported => 4,
    }
}

fn narrower(a: Support, b: Support) -> Support {
    if rank(b) < rank(a) { b } else { a }
}

/// What a provider can do now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    pub availability: Availability,
    pub capabilities: CapabilitySet,
    /// For diagnostics only, never logged together with a title.
    pub app_version: Option<String>,
}

/// The consents the user gave the Calls plugin and this provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConsentState {
    /// `calls:observe` and the provider's own enable.
    pub observe: bool,
    /// `calls:control`.
    pub control: bool,
    /// The provider's `showTitleOnDevice` setting (off by default): only
    /// then may an observation carry a title.
    pub titles: bool,
}

/// What a provider is given when it connects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCx {
    pub consent: ConsentState,
    /// The session the connection will report events in.
    pub session: ProviderSession,
}

/// Why a provider could not connect.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    #[error("the provider is not available: {0:?}")]
    NotAvailable(Availability),
    #[error("the user has not consented to this provider")]
    ConsentNotGiven,
    /// A stable code (`[a-z0-9-]`), never a message from the app.
    #[error("the provider failed: {code}")]
    Failed { code: String },
}

/// Why a provider connection ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisconnectReason {
    AppExited,
    Network,
    /// The user or the app revoked the pairing or the consent.
    Revoked,
    Protocol,
    /// The agent stopped the provider (disabled, consent withdrawn).
    Paused,
}

/// What the call is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CallKind {
    Meeting,
    OneToOne,
    Group,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    Incoming,
    Outgoing,
}

/// How a call ended, as the provider saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservedEnd {
    /// The user rejected an incoming call.
    Rejected,
    AnsweredElsewhere,
    /// It stopped ringing without being answered.
    Missed,
    /// The user cancelled an outgoing call.
    Cancelled,
    /// The other side declined an outgoing call.
    Declined,
    NoAnswer,
    RemoteHangUp,
    /// The user left or ended it.
    Left,
    Unknown,
}

/// The phase of one call, as the provider saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ObservedPhase {
    Ringing,
    Dialing,
    Connecting,
    Connected,
    Ending,
    Ended { cause: ObservedEnd },
    Unknown,
}

/// A state that is known or not. Never defaulted: a provider that cannot
/// tell says `Unknown`, and a face shows "unknown", not a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Tri {
    #[default]
    Unknown,
    Known(bool),
}

impl Tri {
    pub fn known(self) -> Option<bool> {
        match self {
            Self::Known(value) => Some(value),
            Self::Unknown => None,
        }
    }
}

/// Where an observation comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Evidence {
    /// The app reported it through a supported interface.
    AppApi,
    /// Derived from operating-system signals: always
    /// [`Support::Inferred`].
    OsSignal,
    /// A presence service (Graph), up to about a minute late.
    Presence,
}

/// A private value (a meeting title): no `Display`, no `Serialize`, and a
/// `Debug` that prints nothing of it. Read only through
/// [`Private::reveal`], in the two places that may show it (the device face
/// with the user's opt-in, and the `call.state` answer with it).
#[derive(Clone, PartialEq, Eq)]
pub struct Private<T>(T);

impl<T> Private<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub fn reveal(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Private<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Private(<redacted>)")
    }
}

/// A full snapshot of one call or meeting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallObservation {
    pub key: CallKey,
    pub kind: CallKind,
    pub phase: ObservedPhase,
    pub direction: Option<Direction>,
    pub mic_muted: Tri,
    pub camera_on: Tri,
    /// Only when the provider can read it **and** the user opted in
    /// ([`ConsentState::titles`]).
    pub title: Option<Private<String>>,
    pub source: Evidence,
    pub observed_at: Instant,
}

/// What a provider reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
    Connected {
        session: ProviderSession,
    },
    Disconnected {
        reason: DisconnectReason,
    },
    /// The capabilities changed (a meeting's permissions, an OS permission
    /// revoked). Faces follow.
    Capabilities(CapabilitySet),
    Call(CallObservation),
    /// The call is no longer seen.
    CallGone {
        key: CallKey,
    },
    CommandOutcome {
        token: CommandToken,
        outcome: CommandOutcome,
    },
}

/// The outcome of a command, as the provider confirmed it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CommandOutcome {
    Confirmed,
    /// A stable code (`[a-z0-9-]`).
    Rejected {
        code: String,
    },
    /// Sent, but whether it worked cannot be known (a blind toggle). Shown
    /// as unconfirmed, never as success.
    Unknown,
}

/// What a command does to a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CallAction {
    Answer,
    Reject,
    CancelOutgoing,
    End,
    MicToggle,
    MicSet { muted: bool },
    CameraToggle,
    CameraSet { on: bool },
}

impl CallAction {
    /// The capability the action needs.
    pub fn capability(self) -> Capability {
        match self {
            Self::Answer => Capability::Answer,
            Self::Reject => Capability::Reject,
            Self::CancelOutgoing => Capability::CancelOutgoing,
            Self::End => Capability::End,
            Self::MicToggle => Capability::MicToggle,
            Self::MicSet { .. } => Capability::MicSet,
            Self::CameraToggle => Capability::CameraToggle,
            Self::CameraSet { .. } => Capability::CameraSet,
        }
    }
}

/// One command to a provider, for one of its calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallCommand {
    pub key: CallKey,
    pub action: CallAction,
}

/// Why a provider refused a command before sending it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandRefusal {
    #[error("not supported: {}", .0.code())]
    Unsupported(UnsupportedReason),
    #[error("the provider is not connected")]
    NotConnected,
    #[error("the provider does not know this call")]
    UnknownCall,
    #[error("the provider is busy with another command")]
    Busy,
}

/// The adapter contract. Implemented in the agent (and by the mock
/// provider of the tests), driven by the Calls service loop, never by the
/// plugin.
///
/// Every method is non-blocking. Nothing reads a clock: `now` is given.
pub trait CallProvider: Send {
    fn id(&self) -> ProviderId;

    /// What the provider can do now. Cheap and idempotent; may be called at
    /// any time, also before [`CallProvider::connect`].
    fn discover(&mut self, now: Instant) -> Discovery;

    /// Starts the connection; [`ProviderEvent::Connected`] follows from
    /// [`CallProvider::poll`].
    fn connect(&mut self, cx: &ProviderCx) -> Result<(), ProviderError>;

    fn disconnect(&mut self);

    /// The events since the last poll, oldest first.
    fn poll(&mut self, now: Instant) -> Vec<ProviderEvent>;

    /// Sends a command. Its outcome arrives later as
    /// [`ProviderEvent::CommandOutcome`] with `token`.
    fn command(&mut self, command: CallCommand, token: CommandToken) -> Result<(), CommandRefusal>;
}
