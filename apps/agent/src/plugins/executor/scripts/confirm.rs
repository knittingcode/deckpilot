//! The confirmation token of a destructive operation (`p6-plan.md` §2, the
//! contract in the Scripts plugin's README).
//!
//! * **Issued** by the agent for the plan **it** computed, bound to the op
//!   and a digest of its resolved arguments (scope root, relative path,
//!   options), in the run's own table: another run or script never sees
//!   it. 128 random bits.
//! * **`script.confirm {token}`** shows the stored plan (counts and a
//!   sample) in a dialog, only in a run the user started, once per token,
//!   within [`TTL`].
//! * **Used** by repeating the op with `confirmation: token`: the token is
//!   removed **before** anything else is checked, so it is single-use
//!   whatever happens next; op and digest must match and the answer must
//!   have been yes. The caller recomputes the plan and refuses one that
//!   grew.
//! * **Ends** with the run (the table is dropped with it).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use deckpilot_plugin_abi::CallResult;
use deckpilot_plugin_abi::dialog::{DialogRequest, DialogResult};
use deckpilot_plugin_abi::scripts::{
    Confirmation, Confirmed, RemovalPlan, ScriptConfirm, ScriptFault, codes,
};
use deckpilot_plugin_broker::PluginId;
use serde_json::Value;

use super::{Call, Notes, fault, random_hex, to_value};
use crate::plugins::executor::Executor;

/// How long a token waits for its confirmation and its use.
pub(super) const TTL: Duration = Duration::from_secs(120);
/// Tokens one run may hold at once.
pub(super) const MAX_PENDING: usize = 16;
/// Sample paths the dialog shows.
const SHOWN_SAMPLE: usize = 8;

/// The answer a token is at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    NotAsked,
    Yes,
    No,
}

#[derive(Debug, Clone)]
struct Pending {
    op: &'static str,
    digest: [u8; 32],
    plan: RemovalPlan,
    /// Every entry of the plan (`fs::Plan::fingerprint`).
    fingerprint: [u8; 32],
    /// What the dialog says the op does ("Delete", "Overwrite").
    action: &'static str,
    answer: Answer,
    issued: Instant,
}

/// A run's tokens.
#[derive(Debug, Default)]
pub(super) struct Tokens {
    pending: HashMap<String, Pending>,
}

impl Tokens {
    /// Tests: makes `token` older by `by`.
    #[cfg(test)]
    pub fn age(&mut self, token: &str, by: Duration) {
        if let Some(p) = self.pending.get_mut(token) {
            p.issued = p.issued.checked_sub(by).unwrap_or(p.issued);
        }
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }

    fn expire(&mut self) {
        self.pending.retain(|_, p| p.issued.elapsed() < TTL);
    }

    /// A new token for `plan` of `op` with `digest`.
    pub fn issue(
        &mut self,
        op: &'static str,
        digest: [u8; 32],
        plan: RemovalPlan,
        fingerprint: [u8; 32],
        action: &'static str,
    ) -> Result<String, ScriptFault> {
        self.expire();
        if self.pending.len() >= MAX_PENDING {
            return Err(fault(
                codes::LIMIT,
                "too many operations wait for a confirmation in this run",
            ));
        }
        let token = random_hex(16)
            .ok_or_else(|| fault(codes::HOST_ERROR, "no random source for a token"))?;
        self.pending.insert(
            token.clone(),
            Pending {
                op,
                digest,
                plan,
                fingerprint,
                action,
                answer: Answer::NotAsked,
                issued: Instant::now(),
            },
        );
        Ok(token)
    }

    /// The plan `script.confirm` shows; once per token.
    fn question(&mut self, token: &str) -> Result<(RemovalPlan, &'static str), ScriptFault> {
        self.expire();
        let pending = self.pending.get(token).ok_or_else(unknown)?;
        if pending.answer != Answer::NotAsked {
            return Err(fault(
                codes::INVALID_ARGUMENT,
                "this confirmation was already answered",
            ));
        }
        Ok((pending.plan.clone(), pending.action))
    }

    fn answer(&mut self, token: &str, yes: bool) {
        if let Some(p) = self.pending.get_mut(token) {
            p.answer = if yes { Answer::Yes } else { Answer::No };
        }
    }

    /// Takes the token for `op` with `digest`: removed first, then checked.
    /// The confirmed plan and its entries, to compare with the ones
    /// recomputed now.
    pub fn take(
        &mut self,
        token: &str,
        op: &str,
        digest: &[u8; 32],
    ) -> Result<(RemovalPlan, [u8; 32]), ScriptFault> {
        let pending = self.pending.remove(token).ok_or_else(unknown)?;
        if pending.issued.elapsed() >= TTL {
            return Err(fault(
                codes::NEEDS_CONFIRMATION,
                "the confirmation expired; ask again",
            ));
        }
        if pending.op != op || &pending.digest != digest {
            return Err(fault(
                codes::NOT_GRANTED,
                "the confirmation was given for another operation",
            ));
        }
        match pending.answer {
            Answer::Yes => Ok((pending.plan, pending.fingerprint)),
            Answer::No => Err(fault(codes::CANCELLED, "the operation was not confirmed")),
            Answer::NotAsked => Err(fault(
                codes::NEEDS_CONFIRMATION,
                "the operation was not confirmed yet (script.confirm)",
            )),
        }
    }
}

fn unknown() -> ScriptFault {
    fault(
        codes::INVALID_ARGUMENT,
        "not a confirmation this run issued (or it was used or expired)",
    )
}

/// The fault that asks for a confirmation of `token`.
pub(super) fn needs_confirmation(token: String, message: impl Into<String>) -> ScriptFault {
    ScriptFault {
        confirmation: Some(Confirmation { token }),
        ..fault(codes::NEEDS_CONFIRMATION, message)
    }
}

/// Whether `now` is larger than the confirmed plan in anything.
pub(super) fn grew(confirmed: &RemovalPlan, now: &RemovalPlan) -> bool {
    now.files > confirmed.files
        || now.directories > confirmed.directories
        || now.bytes > confirmed.bytes
}

fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["bytes", "KB", "MB", "GB"];
    #[expect(clippy::cast_precision_loss, reason = "a size shown with one decimal")]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// The dialog's text: what the agent computed, never the script's words.
pub(super) fn message(plan: &RemovalPlan, action: &str) -> String {
    let mut text = format!(
        "{action} {} files and {} folders ({}) in “{}”.",
        plan.files,
        plan.directories,
        size(plan.bytes),
        plan.root
    );
    if !plan.sample.is_empty() {
        text.push_str("\n\nFor example:");
        for path in plan.sample.iter().take(SHOWN_SAMPLE) {
            let line = format!("\n• {}", super::run::loggable(path));
            if text.chars().count() + line.chars().count() > limits_message() - 20 {
                break;
            }
            text.push_str(&line);
        }
    }
    text.chars().take(limits_message()).collect()
}

fn limits_message() -> usize {
    deckpilot_plugin_abi::dialog::MAX_DIALOG_MESSAGE
}

impl Executor {
    /// `script.confirm`: the agent's own plan for `token`, in a dialog.
    pub(super) fn script_confirm(
        &self,
        plugin: &PluginId,
        call: &Call<'_>,
        confirm: &ScriptConfirm,
        notes: &mut Notes,
    ) -> Result<Value, ScriptFault> {
        if !call.user_initiated {
            return Err(fault(
                codes::NOT_USER_INITIATED,
                "confirmations are asked only in runs you started",
            ));
        }
        let (plan, action) = call.run.state().tokens.question(&confirm.token)?;
        // The scope's label, never the path below it.
        notes.add(
            "scope",
            plan.root.split('/').next().unwrap_or_default().to_owned(),
        );
        notes.add("files", plan.files.to_string());
        notes.add("directories", plan.directories.to_string());
        let yes = self.ask(plugin, call, &plan, action)?;
        call.run.state().tokens.answer(&confirm.token, yes);
        notes.add("confirmed", yes.to_string());
        to_value(&Confirmed { confirmed: yes })
    }

    fn ask(
        &self,
        plugin: &PluginId,
        call: &Call<'_>,
        plan: &RemovalPlan,
        action: &'static str,
    ) -> Result<bool, ScriptFault> {
        #[cfg(test)]
        if let Some(answer) = *call
            .run
            .answer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            return Ok(answer);
        }
        let request = DialogRequest::Confirm {
            title: format!("{action} files?")
                .chars()
                .take(limits_title())
                .collect(),
            message: Some(message(plan, action)),
            confirm_label: Some(action.to_owned()),
            cancel_label: None,
            destructive: true,
            timeout_seconds: Some(60),
        };
        match self.dialog(plugin, &request, call.interrupt) {
            CallResult::Ok(value) => {
                let result: DialogResult = serde_json::from_value(value)
                    .map_err(|e| fault(codes::HOST_ERROR, e.to_string()))?;
                Ok(result.is_confirmed())
            }
            CallResult::Error(e) => Err(fault(
                codes::UNAVAILABLE,
                format!("the confirmation could not be shown: {}", e.message),
            )),
        }
    }
}

fn limits_title() -> usize {
    deckpilot_plugin_abi::dialog::MAX_DIALOG_TITLE
}
