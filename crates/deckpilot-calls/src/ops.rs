//! The host operations of the bundled Calls plugin and the gate of its
//! actions (`docs/calls.md` §Actions).
//!
//! - `call.state` answers the focused call's [`CallSnapshot`](crate::CallSnapshot).
//! - `call.control` asks for one [`CallAction`] on the focused call. The
//!   plugin never names a call or a provider: the host acts on the focused
//!   call, after [`gate`], and answers with the **confirmed** outcome.
//!
//! The broker grants both operations only to the plugin `deckpilot.calls`
//! with the `calls` permission (`call.control` also needs `calls:control`).
//! Their wire shapes are declared here; the broker and the plugin API level
//! that carries them come with the bundle (C9).

use serde::{Deserialize, Serialize};

use crate::provider::{
    CallAction, Capability, CapabilitySet, ConsentState, Tri, UnsupportedReason,
};
use crate::snapshot::CallState;

/// The operation that answers the focused call's snapshot.
pub const OP_CALL_STATE: &str = "call.state";
/// The operation that asks for one action on the focused call.
pub const OP_CALL_CONTROL: &str = "call.control";

/// The id of the only plugin the `call.*` operations exist for.
pub const CALLS_PLUGIN_ID: &str = "deckpilot.calls";

/// The arguments of `call.control`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallControlRequest {
    pub action: CallAction,
}

/// The answer of `call.control` when the gate let it through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CallControlResult {
    /// The provider confirmed it and an observation shows it.
    Confirmed,
    /// The call was already in the asked-for state (a set of a state it
    /// had): nothing was sent.
    AlreadyThere,
}

/// Why an action did not run, or ran without confirmation. Each has a
/// stable code the plugin's `errorCodes` translate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GateError {
    /// There is no call in a state the action applies to.
    #[error("calls-not-in-call")]
    NotInCall,
    #[error("calls-unsupported")]
    Unsupported(UnsupportedReason),
    /// Sent, but not confirmed in time (or the provider cannot confirm).
    #[error("calls-unconfirmed")]
    Unconfirmed,
    #[error("calls-provider-offline")]
    ProviderOffline,
    /// The user has not consented to controlling calls (`calls:control`).
    #[error("calls-consent")]
    Consent,
}

impl GateError {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotInCall => "calls-not-in-call",
            Self::Unsupported(_) => "calls-unsupported",
            Self::Unconfirmed => "calls-unconfirmed",
            Self::ProviderOffline => "calls-provider-offline",
            Self::Consent => "calls-consent",
        }
    }

    /// Every code, for the plugin manifest's `errorCodes`.
    pub const CODES: [&str; 5] = [
        "calls-not-in-call",
        "calls-unsupported",
        "calls-unconfirmed",
        "calls-provider-offline",
        "calls-consent",
    ];
}

/// The plugin action ids (`deckpilot.calls.<id>`) that control a call, and
/// the action each asks for.
pub const CONTROL_ACTIONS: [(&str, CallAction); 10] = [
    ("answer", CallAction::Answer),
    ("reject", CallAction::Reject),
    ("cancel-outgoing", CallAction::CancelOutgoing),
    ("end", CallAction::End),
    ("mic-toggle", CallAction::MicToggle),
    ("mic-mute", CallAction::MicSet { muted: true }),
    ("mic-unmute", CallAction::MicSet { muted: false }),
    ("camera-toggle", CallAction::CameraToggle),
    ("camera-on", CallAction::CameraSet { on: true }),
    ("camera-off", CallAction::CameraSet { on: false }),
];

/// The action a plugin action id asks for.
pub fn action_of(id: &str) -> Option<CallAction> {
    CONTROL_ACTIONS
        .iter()
        .find(|(name, _)| *name == id)
        .map(|(_, action)| *action)
}

/// What the gate decided to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Send this action (a set on a toggle-only provider becomes a toggle).
    Send(CallAction),
    /// The call already is in the asked-for state: send nothing.
    AlreadyThere,
}

/// What the gate looks at: the focused call and the provider.
#[derive(Debug, Clone, Copy)]
pub struct GateInput<'a> {
    pub state: &'a CallState,
    pub mic_muted: Tri,
    pub camera_on: Tri,
    pub capabilities: &'a CapabilitySet,
    pub connected: bool,
    pub consent: ConsentState,
}

/// The gate of a control action (§Actions): the call must be in a state the
/// action applies to, the provider connected, the user's `calls:control`
/// consent given, and the capability supported now. A set of the
/// microphone or camera on a provider that can only toggle reads the
/// **confirmed** state and toggles only when it differs; with an unknown
/// state it is refused, never guessed.
pub fn gate(input: GateInput<'_>, action: CallAction) -> Result<Plan, GateError> {
    use CallState as S;
    let applies = match action {
        CallAction::Answer | CallAction::Reject => matches!(input.state, S::Incoming),
        CallAction::CancelOutgoing => matches!(input.state, S::Outgoing | S::Connecting),
        CallAction::End => matches!(input.state, S::Active | S::Connecting),
        CallAction::MicToggle
        | CallAction::MicSet { .. }
        | CallAction::CameraToggle
        | CallAction::CameraSet { .. } => matches!(input.state, S::Active),
    };
    if matches!(input.state, S::Unknown) || (!input.connected && input.state.is_live()) {
        return Err(GateError::ProviderOffline);
    }
    if !applies {
        return Err(GateError::NotInCall);
    }
    if !input.consent.control {
        return Err(GateError::Consent);
    }
    let caps = input.capabilities;
    let support = caps.get(action.capability());
    // A set of the state the call already has (confirmed) sends nothing.
    let already = match action {
        CallAction::MicSet { muted } => input.mic_muted.known() == Some(muted),
        CallAction::CameraSet { on } => input.camera_on.known() == Some(on),
        _ => false,
    };
    // Only a state the provider reads from the app itself is trusted.
    if already
        && caps.get(read_of(action)).allows_command()
        && (support.allows_command() || caps.get(toggle_of(action)).allows_command())
    {
        return Ok(Plan::AlreadyThere);
    }
    if support.allows_command() {
        return Ok(Plan::Send(action));
    }
    let toggle =
        |capability: Capability, read: Capability, toggle: CallAction, state: Tri, wanted: bool| {
            if !caps.get(capability).allows_command() {
                return Err(unsupported(support));
            }
            // The state is trusted only when the provider reads it from the
            // app itself: an inferred or unreadable state would make a set
            // answer "already there" while the app says otherwise.
            if !caps.get(read).allows_command() {
                return Err(GateError::Unsupported(UnsupportedReason::ToggleOnly));
            }
            match state.known() {
                Some(current) if current == wanted => Ok(Plan::AlreadyThere),
                Some(_) => Ok(Plan::Send(toggle)),
                None => Err(GateError::Unsupported(UnsupportedReason::ToggleOnly)),
            }
        };
    match action {
        CallAction::MicSet { muted } => toggle(
            Capability::MicToggle,
            Capability::MicReadState,
            CallAction::MicToggle,
            input.mic_muted,
            muted,
        ),
        CallAction::CameraSet { on } => toggle(
            Capability::CameraToggle,
            Capability::CameraReadState,
            CallAction::CameraToggle,
            input.camera_on,
            on,
        ),
        _ => Err(unsupported(support)),
    }
}

/// The capability that reads the state a set changes.
fn read_of(action: CallAction) -> Capability {
    match action {
        CallAction::CameraSet { .. } => Capability::CameraReadState,
        _ => Capability::MicReadState,
    }
}

/// The toggle capability behind a set.
fn toggle_of(action: CallAction) -> Capability {
    match action {
        CallAction::CameraSet { .. } => Capability::CameraToggle,
        _ => Capability::MicToggle,
    }
}

fn unsupported(support: crate::provider::Support) -> GateError {
    match support {
        crate::provider::Support::Unsupported { reason } => GateError::Unsupported(reason),
        // Observable only: the app offers no way to control it.
        _ => GateError::Unsupported(UnsupportedReason::ProviderHasNoApi),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Support;

    fn caps(supported: &[Capability]) -> CapabilitySet {
        CapabilitySet::from_reported(Capability::ALL.into_iter().map(|c| {
            let s = if supported.contains(&c) {
                Support::Supported
            } else {
                Support::Unsupported {
                    reason: UnsupportedReason::ProviderHasNoApi,
                }
            };
            (c, s)
        }))
        .0
    }

    fn input<'a>(state: &'a CallState, caps: &'a CapabilitySet, mic: Tri) -> GateInput<'a> {
        GateInput {
            state,
            mic_muted: mic,
            camera_on: Tri::Unknown,
            capabilities: caps,
            connected: true,
            consent: ConsentState {
                observe: true,
                control: true,
                titles: false,
            },
        }
    }

    #[test]
    fn every_control_action_id_maps_to_its_action() {
        for (id, action) in CONTROL_ACTIONS {
            assert_eq!(action_of(id), Some(action));
        }
        assert_eq!(action_of("show-calls"), None);
        assert_eq!(action_of("return-to-work"), None);
    }

    #[test]
    fn the_state_decides_which_actions_apply() {
        let all = caps(&Capability::ALL);
        let active = CallState::Active;
        let incoming = CallState::Incoming;
        let idle = CallState::Idle;
        assert_eq!(
            gate(input(&incoming, &all, Tri::Unknown), CallAction::Answer),
            Ok(Plan::Send(CallAction::Answer))
        );
        assert_eq!(
            gate(input(&active, &all, Tri::Unknown), CallAction::Answer),
            Err(GateError::NotInCall)
        );
        assert_eq!(
            gate(input(&idle, &all, Tri::Unknown), CallAction::MicToggle),
            Err(GateError::NotInCall)
        );
        assert_eq!(
            gate(
                input(&CallState::Connecting, &all, Tri::Unknown),
                CallAction::End
            ),
            Ok(Plan::Send(CallAction::End))
        );
        assert_eq!(
            gate(
                input(&CallState::Outgoing, &all, Tri::Unknown),
                CallAction::CancelOutgoing
            ),
            Ok(Plan::Send(CallAction::CancelOutgoing))
        );
    }

    #[test]
    fn an_unsupported_capability_is_refused_with_its_reason() {
        let none = CapabilitySet::none(UnsupportedReason::ProviderHasNoApi);
        let active = CallState::Active;
        assert_eq!(
            gate(
                input(&active, &none, Tri::Known(false)),
                CallAction::MicToggle
            ),
            Err(GateError::Unsupported(UnsupportedReason::ProviderHasNoApi))
        );
        // Read-only (observed, not controlled) is not a control either.
        let (read_only, _) =
            CapabilitySet::from_reported([(Capability::MicToggle, Support::ReadOnly)]);
        assert_eq!(
            gate(
                input(&active, &read_only, Tri::Known(false)),
                CallAction::MicToggle
            ),
            Err(GateError::Unsupported(UnsupportedReason::ProviderHasNoApi))
        );
    }

    /// T-S8: a set on a provider that can only toggle reads the confirmed
    /// state; with an unknown state it is refused instead of guessed.
    #[test]
    fn a_set_on_a_toggle_only_provider_needs_the_confirmed_state() {
        let toggle_only = caps(&[
            Capability::MicToggle,
            Capability::MicReadState,
            Capability::CameraToggle,
            Capability::CameraReadState,
        ]);
        let active = CallState::Active;
        let mute = CallAction::MicSet { muted: true };
        assert_eq!(
            gate(input(&active, &toggle_only, Tri::Unknown), mute),
            Err(GateError::Unsupported(UnsupportedReason::ToggleOnly))
        );
        assert_eq!(
            gate(input(&active, &toggle_only, Tri::Known(false)), mute),
            Ok(Plan::Send(CallAction::MicToggle))
        );
        assert_eq!(
            gate(input(&active, &toggle_only, Tri::Known(true)), mute),
            Ok(Plan::AlreadyThere)
        );
        let settable = caps(&[Capability::MicSet]);
        assert_eq!(
            gate(input(&active, &settable, Tri::Unknown), mute),
            Ok(Plan::Send(mute))
        );
        let neither = caps(&[]);
        assert_eq!(
            gate(input(&active, &neither, Tri::Known(false)), mute),
            Err(GateError::Unsupported(UnsupportedReason::ProviderHasNoApi))
        );
    }

    /// A state that is inferred (operating-system signals) or not readable
    /// is never trusted for a set: "already there" or a toggle would be a
    /// guess, so it is refused.
    #[test]
    fn a_set_via_toggle_needs_a_state_read_from_the_app() {
        let active = CallState::Active;
        let unmute = CallAction::MicSet { muted: false };
        for read in [
            Support::Inferred,
            Support::ReadOnly,
            Support::Unsupported {
                reason: UnsupportedReason::ProviderHasNoApi,
            },
        ] {
            let (set, _) = CapabilitySet::from_reported([
                (Capability::MicToggle, Support::Supported),
                (Capability::MicReadState, read),
            ]);
            for state in [Tri::Known(true), Tri::Known(false), Tri::Unknown] {
                assert_eq!(
                    gate(input(&active, &set, state), unmute),
                    Err(GateError::Unsupported(UnsupportedReason::ToggleOnly)),
                    "{read:?} {state:?}"
                );
            }
        }
        let (conditional, _) = CapabilitySet::from_reported([
            (Capability::MicToggle, Support::Supported),
            (
                Capability::MicReadState,
                Support::Conditional {
                    when: crate::provider::Condition::InCall,
                },
            ),
        ]);
        assert_eq!(
            gate(input(&active, &conditional, Tri::Known(true)), unmute),
            Ok(Plan::Send(CallAction::MicToggle))
        );
    }

    #[test]
    fn consent_and_connection_come_before_capabilities() {
        let all = caps(&Capability::ALL);
        let active = CallState::Active;
        let mut no_consent = input(&active, &all, Tri::Known(false));
        no_consent.consent.control = false;
        assert_eq!(gate(no_consent, CallAction::End), Err(GateError::Consent));
        let mut offline = input(&active, &all, Tri::Known(false));
        offline.connected = false;
        assert_eq!(
            gate(offline, CallAction::End),
            Err(GateError::ProviderOffline)
        );
        let unknown = CallState::Unknown;
        assert_eq!(
            gate(input(&unknown, &all, Tri::Unknown), CallAction::End),
            Err(GateError::ProviderOffline)
        );
    }

    #[test]
    fn codes_are_stable() {
        let all = [
            GateError::NotInCall,
            GateError::Unsupported(UnsupportedReason::ToggleOnly),
            GateError::Unconfirmed,
            GateError::ProviderOffline,
            GateError::Consent,
        ];
        let codes: Vec<&str> = all.iter().map(|e| e.code()).collect();
        assert_eq!(codes, GateError::CODES);
        for e in all {
            assert_eq!(e.to_string(), e.code());
        }
    }

    #[test]
    fn the_control_request_has_a_closed_shape() {
        let request: CallControlRequest =
            serde_json::from_str(r#"{"action":{"kind":"micSet","muted":true}}"#).unwrap();
        assert_eq!(request.action, CallAction::MicSet { muted: true });
        // The plugin never names a call or a provider.
        assert!(
            serde_json::from_str::<CallControlRequest>(
                r#"{"action":{"kind":"end"},"provider":"mock"}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<CallControlRequest>(r#"{"action":{"kind":"dial"}}"#).is_err()
        );
        // Nor a field an action does not have.
        for extra in [
            r#"{"action":{"kind":"micSet","muted":true,"call":"x"}}"#,
            r#"{"action":{"kind":"cameraSet","on":true,"provider":"mock"}}"#,
            r#"{"action":{"kind":"end","call":"x"}}"#,
        ] {
            assert!(
                serde_json::from_str::<CallControlRequest>(extra).is_err(),
                "{extra}"
            );
        }
    }
}
