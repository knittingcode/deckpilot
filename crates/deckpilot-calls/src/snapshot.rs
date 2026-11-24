//! The call state the plugin, the key faces and the IPC see.
//!
//! [`CallSnapshot`] is sanitised: no participant names, phone numbers,
//! meeting links or meeting ids, ever, and a title only when the user
//! opted in to it for the provider.

use serde::{Deserialize, Serialize};

use crate::provider::{CallAction, CallKind, CapabilitySet, Direction, ProviderId, Tri};

/// How a call ended, as the engine records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EndCause {
    Rejected,
    AnsweredElsewhere,
    Missed,
    Cancelled,
    Declined,
    NoAnswer,
    RemoteHangUp,
    Left,
    /// The provider lost sight of it and no evidence of the call came back
    /// in time.
    Lost,
    Unknown,
}

/// Why a call track failed: a stable code (`[a-z0-9-]`, at most 48 bytes).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FailureCode(String);

impl FailureCode {
    /// The provider broke its protocol.
    pub const PROTOCOL: &str = "provider-protocol";

    /// `code` if it is a stable code, else `provider-protocol`: a provider
    /// never puts its own text in front of the user.
    pub fn new(code: &str) -> Self {
        let valid = (1..=48).contains(&code.len())
            && code
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        Self(if valid { code } else { Self::PROTOCOL }.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The state of one call, and of a device's Calls presentation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CallState {
    Idle,
    Incoming,
    Outgoing,
    Connecting,
    Active,
    Ending,
    Ended {
        cause: EndCause,
    },
    Failed {
        code: FailureCode,
    },
    /// The provider lost sight of the call; it may come back.
    Unknown,
}

impl CallState {
    /// Whether the state is a call in progress (it has a timeout and may
    /// hold the overlay).
    pub fn is_live(&self) -> bool {
        matches!(
            self,
            Self::Incoming
                | Self::Outgoing
                | Self::Connecting
                | Self::Active
                | Self::Ending
                | Self::Unknown
        )
    }

    /// The stable name, for logs (`info`: state names only).
    pub fn name(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
            Self::Connecting => "connecting",
            Self::Active => "active",
            Self::Ending => "ending",
            Self::Ended { .. } => "ended",
            Self::Failed { .. } => "failed",
            Self::Unknown => "unknown",
        }
    }
}

/// The focused call as the plugin (`call.state`), the faces and the IPC
/// see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallSnapshot {
    pub state: CallState,
    pub kind: CallKind,
    pub direction: Option<Direction>,
    pub mic_muted: Tri,
    pub camera_on: Tri,
    pub capabilities: CapabilitySet,
    pub provider: Option<ProviderId>,
    /// When the call entered its state. The engine fills it in with its
    /// own clock; the agent maps it to Unix milliseconds before it leaves
    /// the agent.
    pub since: u64,
    /// Whether the call's provider is connected ("no connection to the
    /// provider" on the faces when not).
    #[serde(default)]
    pub connected: bool,
    /// A command sent and not confirmed yet: the key shows it as pending,
    /// never as done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<CallAction>,
    /// The last command was not confirmed in time: shown for a moment.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unconfirmed: bool,
    /// Only with the user's opt-in for the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Bumped on every change, so a face drawn from an older snapshot is
    /// redrawn.
    pub generation: u64,
}

impl CallSnapshot {
    /// No call, with the provider's current capabilities.
    pub fn idle(capabilities: CapabilitySet, generation: u64) -> Self {
        Self {
            state: CallState::Idle,
            kind: CallKind::Unknown,
            direction: None,
            mic_muted: Tri::Unknown,
            camera_on: Tri::Unknown,
            capabilities,
            provider: None,
            since: 0,
            connected: false,
            pending: None,
            unconfirmed: false,
            title: None,
            generation,
        }
    }
}
