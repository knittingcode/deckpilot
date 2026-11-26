//! The capability operations of plugin API 1.16's `calls` kind
//! (`docs/calls.md`): `call.state` and `call.control`.
//!
//! Only the bundled plugin `deckpilot.calls` may be granted them (the
//! manifest rules, the privileged policy and the broker each check it). A
//! guest never names a call, a provider, a device or a layout: `call.state`
//! answers the focused call, and `call.control` asks for one action on it.
//! The host decides what happens (the state engine's gate) and answers with
//! the **confirmed** outcome, never an optimistic one.
//!
//! What the guest receives is sanitised ([`CallStateReply`]): the state's
//! name, the microphone and camera as the call app reports them (or
//! unknown), whether each control can work now and why not, and the
//! meeting title only when the user opted in to it. Never a participant, a
//! number, a meeting link or id, a process or an executable name.
//!
//! Added compatibly within ABI 1: a guest built before them never sends
//! them, and a host or agent that predates them refuses the unknown op.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Longest meeting title a reply carries, in characters.
pub const MAX_TITLE: usize = 256;

/// One action on the focused call. The same JSON as the host's own
/// (`{"kind":"micSet","muted":true}`); an action with a field it does not
/// have (an `end` with `muted`, any action naming a call or a provider)
/// does not parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", try_from = "RawAction")]
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

/// What a [`CallAction`] is read from: every field an action may have, and
/// nothing else.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawAction {
    kind: String,
    #[serde(default)]
    muted: Option<bool>,
    #[serde(default)]
    on: Option<bool>,
}

impl TryFrom<RawAction> for CallAction {
    type Error = String;

    fn try_from(raw: RawAction) -> Result<Self, String> {
        Ok(match (raw.kind.as_str(), raw.muted, raw.on) {
            ("answer", None, None) => Self::Answer,
            ("reject", None, None) => Self::Reject,
            ("cancelOutgoing", None, None) => Self::CancelOutgoing,
            ("end", None, None) => Self::End,
            ("micToggle", None, None) => Self::MicToggle,
            ("micSet", Some(muted), None) => Self::MicSet { muted },
            ("cameraToggle", None, None) => Self::CameraToggle,
            ("cameraSet", None, Some(on)) => Self::CameraSet { on },
            (kind, _, _) => return Err(format!("not a call action with these fields: `{kind}`")),
        })
    }
}

/// The arguments of `call.control`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallControl {
    pub action: CallAction,
}

/// Whether one control can work now, and why not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlSupport {
    pub supported: bool,
    /// The stable reason when not (`provider-has-no-api`,
    /// `consent-not-given`, `toggle-only`…), which the plugin translates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The `ok` value of `call.state`. Unknown fields are **ignored** (host to
/// guest only), so the agent can add to it within ABI 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallStateReply {
    /// `idle`, `incoming`, `outgoing`, `connecting`, `active`, `ending`,
    /// `ended`, `failed` or `unknown`.
    pub state: String,
    /// The provider of the focused call (`os-signals`, `mock`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Whether the provider is connected.
    #[serde(default)]
    pub connected: bool,
    /// The microphone is muted, as the call app reports it; `None` when it
    /// is not known (never guessed: an app holding the microphone may be
    /// muted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mic_muted: Option<bool>,
    /// The camera is on, as the call app reports it; `None` when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_on: Option<bool>,
    /// A command waits for its confirmation: show it as pending, never as
    /// done.
    #[serde(default)]
    pub pending: bool,
    /// The last command was not confirmed in time.
    #[serde(default)]
    pub unconfirmed: bool,
    /// Per control action id (`answer`, `mic-toggle`…): whether it can work
    /// now with this provider and the user's consent.
    #[serde(default)]
    pub controls: BTreeMap<String, ControlSupport>,
    /// The meeting title, only with the user's opt-in for the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Bumped on every change, so a face drawn from an older reply is
    /// redrawn.
    #[serde(default)]
    pub generation: u64,
}

/// The `ok` value of `call.control`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CallControlReply {
    /// The provider confirmed it and an observation shows it.
    Confirmed,
    /// The call already was in the asked-for state: nothing was sent.
    AlreadyThere,
}

/// The reason codes a refused Calls operation starts its message with
/// (`calls-unsupported: provider-has-no-api`), which the plugin's
/// `errorCodes` translate.
pub mod codes {
    /// No call is in a state the action applies to.
    pub const NOT_IN_CALL: &str = "calls-not-in-call";
    /// The provider cannot do it (the reason follows).
    pub const UNSUPPORTED: &str = "calls-unsupported";
    /// Sent, but not confirmed in time.
    pub const UNCONFIRMED: &str = "calls-unconfirmed";
    /// The provider is not connected, or Calls does not run.
    pub const PROVIDER_OFFLINE: &str = "calls-provider-offline";
    /// The user did not consent (to observing, or to controlling).
    pub const CONSENT: &str = "calls-consent";
    /// DeckPilot is locked.
    pub const LOCKED: &str = "calls-locked";
    /// A control in a run the user did not start (a schedule, an event,
    /// the agent's start).
    pub const NOT_USER_STARTED: &str = "calls-not-user-started";
    /// Too many controls: in one run, or waiting at once.
    pub const THROTTLED: &str = "calls-throttled";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{CapabilityRequest, NoArgs};

    #[test]
    fn the_operations_have_closed_shapes() {
        let state: CapabilityRequest =
            serde_json::from_str(r#"{"op":"call.state","args":{}}"#).unwrap();
        assert_eq!(state, CapabilityRequest::CallState(NoArgs {}));
        assert_eq!(state.op(), "call.state");
        let control: CapabilityRequest = serde_json::from_str(
            r#"{"op":"call.control","args":{"action":{"kind":"micSet","muted":true}}}"#,
        )
        .unwrap();
        assert_eq!(
            control,
            CapabilityRequest::CallControl(CallControl {
                action: CallAction::MicSet { muted: true }
            })
        );
        assert_eq!(control.op(), "call.control");
        assert!(control.validate().is_ok());
        // Never a call, a provider, a device or a plugin.
        for refused in [
            r#"{"op":"call.control","args":{"action":{"kind":"end"},"provider":"mock"}}"#,
            r#"{"op":"call.control","args":{"action":{"kind":"end","call":"x"}}}"#,
            r#"{"op":"call.control","args":{"action":{"kind":"micSet"}}}"#,
            r#"{"op":"call.control","args":{"action":{"kind":"dial"}}}"#,
            r#"{"op":"call.control","args":{"action":{"kind":"end"},"plugin":"x"}}"#,
            r#"{"op":"call.state","args":{"device":"x"}}"#,
        ] {
            assert!(
                serde_json::from_str::<CapabilityRequest>(refused).is_err(),
                "{refused}"
            );
        }
    }

    #[test]
    fn every_action_round_trips() {
        for action in [
            CallAction::Answer,
            CallAction::Reject,
            CallAction::CancelOutgoing,
            CallAction::End,
            CallAction::MicToggle,
            CallAction::MicSet { muted: false },
            CallAction::CameraToggle,
            CallAction::CameraSet { on: true },
        ] {
            let json = serde_json::to_string(&action).unwrap();
            assert_eq!(serde_json::from_str::<CallAction>(&json).unwrap(), action);
        }
    }

    #[test]
    fn a_reply_ignores_fields_added_later() {
        let reply: CallStateReply =
            serde_json::from_str(r#"{"state":"active","later":1}"#).unwrap();
        assert_eq!(reply.state, "active");
        assert_eq!(reply.mic_muted, None);
        assert!(reply.controls.is_empty());
        assert_eq!(
            serde_json::to_value(CallControlReply::AlreadyThere).unwrap(),
            "alreadyThere"
        );
    }
}
