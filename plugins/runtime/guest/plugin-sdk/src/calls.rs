//! Plugin API 1.16's operations (`docs/calls.md`): the focused call's
//! sanitised state and one action on it.
//!
//! Only the bundled plugin `deckpilot.calls` may use them: for every other
//! plugin the host denies both. [`Context::call_state`] needs `calls` and
//! works in a key's state call too; [`Context::call_control`] needs
//! `calls:control` and a run the user started, and answers only once the
//! call app confirmed the action (a refusal starts its message with a
//! stable code, [`deckpilot_plugin_abi::calls::codes`]). An agent that does
//! not carry them out answers [`CapabilityErrorKind::Unsupported`](crate::CapabilityErrorKind::Unsupported).

use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_abi::calls::{CallAction, CallControl, CallControlReply, CallStateReply};
use deckpilot_plugin_abi::capability::NoArgs;

use super::{CapabilityError, Context, malformed};

impl Context<'_> {
    /// The focused call (op `call.state`, plugin API 1.16): its state, the
    /// microphone and camera as the call app reports them, whether each
    /// control can work now and why not, and the title only with the
    /// user's opt-in.
    ///
    /// ```
    /// use deckpilot_plugin_sdk::testing::MockHost;
    ///
    /// let mut host = MockHost::allow_all();
    /// let mut ctx = host.context();
    /// assert_eq!(ctx.call_state().unwrap().state, "idle");
    /// ```
    pub fn call_state(&mut self) -> Result<CallStateReply, CapabilityError> {
        let value = self.call("call.state", &CapabilityRequest::CallState(NoArgs {}))?;
        serde_json::from_value(value).map_err(|_| malformed("call.state", "a call state"))
    }

    /// Asks for `action` on the focused call (op `call.control`, plugin
    /// API 1.16) and answers once it is confirmed, or that the call already
    /// was in that state. Never an optimistic success.
    pub fn call_control(
        &mut self,
        action: CallAction,
    ) -> Result<CallControlReply, CapabilityError> {
        let value = self.call(
            "call.control",
            &CapabilityRequest::CallControl(CallControl { action }),
        )?;
        serde_json::from_value(value).map_err(|_| malformed("call.control", "a call outcome"))
    }
}

#[cfg(test)]
mod tests {
    use deckpilot_plugin_abi::calls::{CallAction, CallControlReply};
    use deckpilot_plugin_abi::{CallErrorCode, CallResult, CapabilityRequest};

    use crate::CapabilityErrorKind;
    use crate::testing::MockHost;

    #[test]
    fn a_refusal_keeps_its_code() {
        let mut host = MockHost::new(|req| match req {
            CapabilityRequest::CallControl(_) => CallResult::error(
                CallErrorCode::Failed,
                "calls-unsupported: provider-has-no-api",
            ),
            CapabilityRequest::CallState(_) => CallResult::Ok(serde_json::json!({
                "state": "active",
                "controls": {"end": {"supported": false, "reason": "provider-has-no-api"}},
            })),
            _ => CallResult::Ok(serde_json::Value::Null),
        });
        let mut ctx = host.context();
        let state = ctx.call_state().unwrap();
        assert_eq!(state.state, "active");
        assert_eq!(
            state.controls["end"].reason.as_deref(),
            Some("provider-has-no-api")
        );
        let error = ctx.call_control(CallAction::End).unwrap_err();
        assert!(error.to_string().contains("calls-unsupported"), "{error}");
    }

    #[test]
    fn a_confirmed_action_says_so() {
        let mut host = MockHost::allow_all();
        let mut ctx = host.context();
        assert_eq!(
            ctx.call_control(CallAction::MicToggle).unwrap(),
            CallControlReply::Confirmed
        );
        let mut old =
            MockHost::new(|_| CallResult::error(CallErrorCode::UnsupportedOp, "not implemented"));
        assert_eq!(
            old.context().call_state().unwrap_err().kind(),
            CapabilityErrorKind::Unsupported
        );
    }
}
