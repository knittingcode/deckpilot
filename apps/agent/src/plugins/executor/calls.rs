//! Plugin API 1.16's `call.state` and `call.control` (`docs/calls.md`
//! §Plugin operations).
//!
//! The broker decided (the plugin `deckpilot.calls`, the `calls` grant,
//! `calls:control` and a run the user started for a control, 4 controls
//! per second). The executor checks the plugin and the run once more, then
//! the agent's thread answers (`service/plugin_calls.rs`): the user's
//! consents and the state engine's gate decide there, and a control is
//! answered only once the call app confirmed it.
//!
//! Every call is audited ([`super::PrivilegedRecord`]): the operation, the
//! grant, the outcome and, for a control, the action's kind. Never the
//! state, the title or anything of the call. A refusal carries its reason
//! code at the start of the message (`calls-unsupported: …`).

use std::sync::Arc;
use std::time::Instant;

use deckpilot_calls::ops::CALLS_PLUGIN_ID;
use deckpilot_plugin_abi::calls::codes;
use deckpilot_plugin_abi::{CallErrorCode, CallResult};
use deckpilot_plugin_broker::{Allowed, DenyReason, PluginId};
use tracing::warn;

use super::{Executor, PrivilegedRecord};
use crate::service::plugin_calls::{CallsAnswer, CallsCall, CallsHost, CallsRefusal};

impl Executor {
    /// The executor of an invocation that reaches Calls through `calls`.
    pub fn with_calls(mut self, calls: Option<Arc<dyn CallsHost>>) -> Self {
        self.calls = calls;
        self
    }

    /// `call.state` or `call.control`, which the broker allowed.
    pub(super) fn calls(&self, plugin: &PluginId, allowed: &Allowed) -> CallResult {
        let started = Instant::now();
        let (op, grant, call) = match allowed {
            Allowed::CallState => ("call.state", "calls", CallsCall::State),
            Allowed::CallControl { action } => {
                ("call.control", "calls:control", CallsCall::Control(*action))
            }
            _ => return CallResult::error(CallErrorCode::Failed, "not a Calls operation"),
        };
        let mut record = PrivilegedRecord {
            plugin: plugin.to_string(),
            op,
            grant,
            outcome: None,
            elapsed: std::time::Duration::ZERO,
            details: Vec::new(),
        };
        if let CallsCall::Control(action) = call {
            let kind = serde_json::to_value(action)
                .ok()
                .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_owned))
                .unwrap_or_default();
            record.details.push(("action", kind));
        }
        let control = matches!(call, CallsCall::Control(_));
        let answer = if plugin.as_str() != CALLS_PLUGIN_ID {
            // The broker refuses it already.
            warn!(plugin = %plugin, op, "a Calls operation of another plugin was refused");
            Err(CallsRefusal::new(
                codes::CONSENT,
                "only the Calls plugin may use calls",
            ))
        } else if control && !self.run.is_user_initiated() {
            // The broker refuses it already: checked again so that nothing
            // but a run the user started controls a call.
            Err(CallsRefusal::new(
                codes::CONSENT,
                DenyReason::PrivilegedNotUserInitiated.to_string(),
            ))
        } else {
            match &self.calls {
                Some(host) => host.call(plugin.as_str(), call),
                None => Err(CallsRefusal::new(
                    codes::PROVIDER_OFFLINE,
                    "Calls is not available in this agent",
                )),
            }
        };
        let result = match answer {
            Ok(CallsAnswer::State(state)) => serde_json::to_value(*state).map_or_else(
                |e| CallResult::error(CallErrorCode::Failed, e.to_string()),
                CallResult::Ok,
            ),
            Ok(CallsAnswer::Control(outcome)) => serde_json::to_value(outcome).map_or_else(
                |e| CallResult::error(CallErrorCode::Failed, e.to_string()),
                CallResult::Ok,
            ),
            Err(refusal) => {
                record.outcome = Some(refusal.code);
                CallResult::error(
                    code(refusal.code),
                    format!("{}: {}", refusal.code, refusal.message),
                )
            }
        };
        record.elapsed = started.elapsed();
        self.audit_privileged(&record);
        result
    }
}

/// The ABI code of a refusal's reason code.
fn code(reason: &str) -> CallErrorCode {
    match reason {
        codes::CONSENT => CallErrorCode::NotPermitted,
        codes::PROVIDER_OFFLINE | codes::LOCKED => CallErrorCode::Unavailable,
        _ => CallErrorCode::Failed,
    }
}

#[cfg(test)]
mod tests;
