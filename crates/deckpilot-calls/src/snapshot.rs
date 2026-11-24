//! The call state the plugin, the key faces and the IPC see.
//!
//! [`CallSnapshot`] is sanitised: no participant names, phone numbers,
//! meeting links or meeting ids, ever, and a title only when the user
//! opted in to it for the provider.

use serde::{Deserialize, Serialize};

use crate::provider::{
    CallAction, CallKind, CallObservation, CapabilitySet, ConsentState, Direction, Private,
    ProviderId, Tri,
};

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
///
/// Deserializing one goes through [`FailureCode::new`], so a code read from
/// anywhere is a stable code.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FailureCode(#[serde(deserialize_with = "stable_code")] String);

fn stable_code<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let text = String::deserialize(deserializer)?;
    Ok(FailureCode::new(&text).0)
}

impl std::fmt::Display for FailureCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

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
///
/// Its `Debug` never prints the title (only whether there is one), so a
/// snapshot can be logged.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl std::fmt::Debug for CallSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallSnapshot")
            .field("state", &self.state)
            .field("kind", &self.kind)
            .field("direction", &self.direction)
            .field("mic_muted", &self.mic_muted)
            .field("camera_on", &self.camera_on)
            .field("capabilities", &self.capabilities)
            .field("provider", &self.provider)
            .field("since", &self.since)
            .field("connected", &self.connected)
            .field("pending", &self.pending)
            .field("unconfirmed", &self.unconfirmed)
            .field("title", &self.title.as_ref().map(|_| "<redacted>"))
            .field("generation", &self.generation)
            .finish()
    }
}

impl CallSnapshot {
    /// The snapshot of a call as `observation` shows it, in `state`. The
    /// title is taken only with the user's opt-in (`consent.titles`); it is
    /// dropped otherwise, whatever the provider sent.
    pub fn from_observation(
        state: CallState,
        observation: &CallObservation,
        capabilities: CapabilitySet,
        provider: ProviderId,
        consent: ConsentState,
        generation: u64,
    ) -> Self {
        Self {
            state,
            kind: observation.kind,
            direction: observation.direction,
            mic_muted: observation.mic_muted,
            camera_on: observation.camera_on,
            capabilities,
            provider: Some(provider),
            since: observation.observed_at.as_millis(),
            connected: true,
            pending: None,
            unconfirmed: false,
            title: Self::title_with_consent(observation.title.as_ref(), consent),
            generation,
        }
    }

    /// A title for the snapshot: only with the user's opt-in.
    pub fn title_with_consent(
        title: Option<&Private<String>>,
        consent: ConsentState,
    ) -> Option<String> {
        if consent.titles {
            title.map(|t| t.reveal().clone())
        } else {
            None
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Instant;
    use crate::provider::{CallKey, Evidence, ObservedPhase, UnsupportedReason};

    fn observation() -> CallObservation {
        CallObservation {
            key: CallKey::new("k").unwrap(),
            kind: CallKind::Meeting,
            phase: ObservedPhase::Connected,
            direction: None,
            mic_muted: Tri::Known(true),
            camera_on: Tri::Unknown,
            title: Some(Private::new("Board: layoffs".to_owned())),
            source: Evidence::AppApi,
            observed_at: Instant::from_secs(3),
        }
    }

    fn snapshot(titles: bool) -> CallSnapshot {
        CallSnapshot::from_observation(
            CallState::Active,
            &observation(),
            CapabilitySet::none(UnsupportedReason::ProviderHasNoApi),
            ProviderId::new("mock").unwrap(),
            ConsentState {
                observe: true,
                control: false,
                titles,
            },
            7,
        )
    }

    #[test]
    fn the_title_needs_the_opt_in() {
        let without = snapshot(false);
        assert_eq!(without.title, None);
        assert!(!serde_json::to_string(&without).unwrap().contains("layoffs"));
        assert_eq!(snapshot(true).title.as_deref(), Some("Board: layoffs"));
        assert_eq!(snapshot(true).mic_muted, Tri::Known(true));
    }

    #[test]
    fn debug_never_prints_the_title() {
        let printed = format!("{:?}", snapshot(true));
        assert!(!printed.contains("layoffs"), "{printed}");
        assert!(printed.contains("<redacted>"), "{printed}");
    }

    #[test]
    fn failure_codes_are_stable_codes_however_they_arrive() {
        assert_eq!(FailureCode::new("busy").as_str(), "busy");
        for bad in ["", "Some Text", "x".repeat(49).as_str(), "a\nb"] {
            assert_eq!(
                FailureCode::new(bad).as_str(),
                FailureCode::PROTOCOL,
                "{bad}"
            );
        }
        let read: FailureCode = serde_json::from_str("\"<script>\"").unwrap();
        assert_eq!(read.as_str(), FailureCode::PROTOCOL);
    }
}
