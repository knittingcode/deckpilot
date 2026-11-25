//! The agent's side of plugin API 1.16's `call.state` and `call.control`
//! (`docs/calls.md` §Plugin operations).
//!
//! The broker allowed the call (the `calls` grant, `calls:control` for a
//! control, the plugin `deckpilot.calls`, a run the user started for a
//! control) and the executor checked the plugin and the run again
//! (`plugins/executor/calls.rs`). The executor then asks the agent's
//! thread ([`AgentCallsHost`]), which answers from the Calls driver:
//!
//! * **The plugin is checked a third time:** anything but
//!   `deckpilot.calls` is refused.
//! * **The user's own consents decide next**, as for the overlay keys:
//!   Calls runs only with the consent to observe calls (otherwise
//!   `calls-provider-offline`: nothing is installed), and a control passes
//!   the state engine's gate ([`deckpilot_calls::ops::gate`]): the call's
//!   state, the provider's connection, the consent to control calls
//!   (`calls-consent`) and the provider's capability now
//!   (`calls-unsupported: <reason>`; for detection-only Teams every control
//!   is `provider-has-no-api`). Nothing is ever guessed or sent ahead.
//! * **A control answers only when it is confirmed:** the command goes to
//!   the provider and the reply waits for the engine's `CommandDone`
//!   (confirmed, unconfirmed, rejected), at most [`CONTROL_WAIT`]. One
//!   waiting control per call (the engine's rule) and at most
//!   [`MAX_WAITING`] in all.
//! * **The state is sanitised** ([`CallStateReply`]): the state's name,
//!   the provider id, the microphone and camera as the app reports them,
//!   whether each control can work and why not, and the title only with
//!   the user's opt-in (the engine drops it otherwise). Never a
//!   participant, a meeting id or link, a process or an executable.
//! * **Nothing about devices or layouts:** neither operation names a
//!   device, a frame or a layout, so the plugin cannot drive an overlay,
//!   a managed layout or a profile. A confirmed command changes the call;
//!   the overlay follows the call as it always does.
//! * **Locked:** both are refused (`calls-locked`) while DeckPilot is
//!   locked, like the overlay, which a locked device never shows.

use std::collections::BTreeMap;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::time::Duration;

use deckpilot_calls::engine::{CommandEnd, EngineEvent, Request};
use deckpilot_calls::ops::{CALLS_PLUGIN_ID, CONTROL_ACTIONS, GateError, control_support};
use deckpilot_calls::provider::{CallAction, CommandToken, Tri};
use deckpilot_plugin_abi::calls::{
    self as abi, CallControlReply, CallStateReply, ControlSupport, codes,
};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, info};

use super::{Agent, Command, SecurityState};

/// How long an executor waits for the answer to `call.state`.
const STATE_WAIT: Duration = Duration::from_secs(2);

/// How long an executor waits for a control's confirmation: the engine's
/// own wait for a confirmation (5 s) and a margin for the agent's loop.
pub(crate) const CONTROL_WAIT: Duration = Duration::from_secs(8);

/// Most controls waiting for their confirmation at once.
pub(crate) const MAX_WAITING: usize = 8;

/// What the Calls plugin asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallsCall {
    State,
    Control(abi::CallAction),
}

/// The agent's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallsAnswer {
    State(Box<CallStateReply>),
    Control(CallControlReply),
}

/// Why the agent refused: a stable code ([`codes`]) and a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallsRefusal {
    pub code: &'static str,
    pub message: String,
}

impl CallsRefusal {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn of_gate(error: GateError) -> Self {
        match error {
            GateError::NotInCall => Self::new(
                codes::NOT_IN_CALL,
                "no call is in a state this action applies to",
            ),
            GateError::Unsupported(reason) => Self::new(codes::UNSUPPORTED, reason.code()),
            GateError::Unconfirmed => Self::new(
                codes::UNCONFIRMED,
                "the call app did not confirm it (or another command still waits)",
            ),
            GateError::ProviderOffline => {
                Self::new(codes::PROVIDER_OFFLINE, "the call app is not connected")
            }
            GateError::Consent => Self::new(
                codes::CONSENT,
                "the user has not consented to controlling calls",
            ),
        }
    }
}

pub(crate) type CallsReply = Result<CallsAnswer, CallsRefusal>;

/// The Calls operations as the executors reach them.
pub(crate) trait CallsHost: Send + Sync + std::fmt::Debug {
    /// Carries out `call` for `plugin` (already allowed by the broker and
    /// checked by the executor). Blocks the caller's thread until the
    /// answer, at most [`CONTROL_WAIT`].
    fn call(&self, plugin: &str, call: CallsCall) -> CallsReply;
}

/// [`CallsHost`] through the agent's command channel, answered on the
/// agent's thread. Called on a plugin's lane thread, never on the agent's.
#[derive(Debug, Clone)]
pub(crate) struct AgentCallsHost(UnboundedSender<Command>);

impl AgentCallsHost {
    pub fn new(commands: UnboundedSender<Command>) -> Self {
        Self(commands)
    }
}

impl CallsHost for AgentCallsHost {
    fn call(&self, plugin: &str, call: CallsCall) -> CallsReply {
        let (reply, answer) = sync_channel(1);
        self.0
            .send(Command::PluginCall {
                plugin: plugin.to_owned(),
                call,
                reply,
            })
            .map_err(|_| {
                CallsRefusal::new(codes::PROVIDER_OFFLINE, "the agent is not answering")
            })?;
        let wait = match call {
            CallsCall::State => STATE_WAIT,
            CallsCall::Control(_) => CONTROL_WAIT,
        };
        answer.recv_timeout(wait).unwrap_or_else(|_| {
            Err(match call {
                CallsCall::State => {
                    CallsRefusal::new(codes::PROVIDER_OFFLINE, "the agent is not answering")
                }
                CallsCall::Control(_) => {
                    CallsRefusal::new(codes::UNCONFIRMED, "no confirmation in time")
                }
            })
        })
    }
}

/// A control waiting for its confirmation.
#[derive(Debug)]
pub(crate) struct Waiting {
    reply: SyncSender<CallsReply>,
    /// On the engine's clock: answered unconfirmed after it.
    until: deckpilot_calls::clock::Instant,
}

/// The controls waiting for their confirmation, by command.
#[derive(Debug, Default)]
pub(crate) struct WaitingControls(BTreeMap<CommandToken, Waiting>);

impl WaitingControls {
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

/// The host's action of the plugin's.
fn action_of(action: abi::CallAction) -> CallAction {
    match action {
        abi::CallAction::Answer => CallAction::Answer,
        abi::CallAction::Reject => CallAction::Reject,
        abi::CallAction::CancelOutgoing => CallAction::CancelOutgoing,
        abi::CallAction::End => CallAction::End,
        abi::CallAction::MicToggle => CallAction::MicToggle,
        abi::CallAction::MicSet { muted } => CallAction::MicSet { muted },
        abi::CallAction::CameraToggle => CallAction::CameraToggle,
        abi::CallAction::CameraSet { on } => CallAction::CameraSet { on },
    }
}

fn known(tri: Tri) -> Option<bool> {
    tri.known()
}

impl Agent {
    /// `call.state` or `call.control` of `plugin`, on the agent's thread.
    /// A confirmed or refused answer goes to `reply` now; a control the
    /// provider was sent waits for its confirmation
    /// ([`Agent::calls_answer_waiting`]).
    pub(super) fn plugin_call(
        &mut self,
        plugin: &str,
        call: CallsCall,
        reply: SyncSender<CallsReply>,
    ) {
        let refused = |reply: &SyncSender<CallsReply>, refusal: CallsRefusal| {
            let _ = reply.try_send(Err(refusal));
        };
        if plugin != CALLS_PLUGIN_ID {
            // The broker and the executor refuse it already.
            return refused(
                &reply,
                CallsRefusal::new(codes::CONSENT, "only the Calls plugin may use calls"),
            );
        }
        if self.security != SecurityState::Unlocked {
            return refused(
                &reply,
                CallsRefusal::new(codes::LOCKED, "DeckPilot is locked"),
            );
        }
        let Some(now) = self.calls_now() else {
            return refused(
                &reply,
                CallsRefusal::new(
                    codes::PROVIDER_OFFLINE,
                    "Calls is not running: no call app is enabled with the user's consent",
                ),
            );
        };
        match call {
            CallsCall::State => {
                let answer = self.calls_state_reply();
                let _ = reply.try_send(answer.map(|s| CallsAnswer::State(Box::new(s))));
            }
            CallsCall::Control(action) => self.calls_control(action_of(action), now, reply),
        }
    }

    /// The focused call as the plugin sees it.
    fn calls_state_reply(&self) -> Result<CallStateReply, CallsRefusal> {
        let service =
            self.calls.service.as_deref().ok_or_else(|| {
                CallsRefusal::new(codes::PROVIDER_OFFLINE, "Calls is not running")
            })?;
        let consent = service.consent;
        if !consent.observe {
            return Err(CallsRefusal::new(
                codes::CONSENT,
                "the user has not consented to observing calls",
            ));
        }
        let snapshot = service.driver.engine().snapshot();
        let controls = CONTROL_ACTIONS
            .iter()
            .filter_map(|(id, _)| {
                control_support(id, &snapshot.capabilities, consent).map(|support| {
                    let support = match support {
                        Ok(()) => ControlSupport {
                            supported: true,
                            reason: None,
                        },
                        Err(reason) => ControlSupport {
                            supported: false,
                            reason: Some(reason.code().to_owned()),
                        },
                    };
                    ((*id).to_owned(), support)
                })
            })
            .collect();
        Ok(CallStateReply {
            state: snapshot.state.name().to_owned(),
            provider: snapshot.provider.as_ref().map(ToString::to_string),
            connected: snapshot.connected,
            mic_muted: known(snapshot.mic_muted),
            camera_on: known(snapshot.camera_on),
            pending: snapshot.pending.is_some(),
            unconfirmed: snapshot.unconfirmed,
            controls,
            // Only with the opt-in: the engine keeps no title otherwise.
            title: snapshot
                .title
                .filter(|_| consent.titles)
                .map(|t| t.chars().take(abi::MAX_TITLE).collect()),
            generation: snapshot.generation,
        })
    }

    /// Asks the engine for `action` on the focused call and sends what it
    /// let through to the provider; the reply waits for the confirmation.
    fn calls_control(
        &mut self,
        action: CallAction,
        now: deckpilot_calls::clock::Instant,
        reply: SyncSender<CallsReply>,
    ) {
        if self.calls.waiting.len() >= MAX_WAITING {
            let _ = reply.try_send(Err(CallsRefusal::new(
                codes::UNCONFIRMED,
                "too many call actions wait for their confirmation",
            )));
            return;
        }
        let Some(service) = self.calls.service.as_mut() else {
            return;
        };
        let requested = service.driver.engine_mut().request(action, now);
        let request = match requested {
            Err(error) => {
                debug!(code = error.code(), "a Calls plugin action was refused");
                let _ = reply.try_send(Err(CallsRefusal::of_gate(error)));
                return;
            }
            Ok(Request::AlreadyThere) => {
                let _ = reply.try_send(Ok(CallsAnswer::Control(CallControlReply::AlreadyThere)));
                return;
            }
            Ok(request) => request,
        };
        let Request::Send { command, token, .. } = request else {
            return;
        };
        let until =
            now.after(service.driver.engine().timeouts().command_max + Duration::from_secs(1));
        self.calls.waiting.0.insert(token, Waiting { reply, until });
        if let Err(refusal) = service.provider.command(command, token) {
            service.driver.engine_mut().command_refused(token, &refusal);
        }
        info!(?action, "the Calls plugin asked for a call action");
        service.driver.sync(&mut self.runtime, now);
        self.calls_answer_waiting(now);
        self.render_all();
        self.calls_publish();
    }

    /// Answers the waiting controls whose command is done, and those that
    /// waited too long (a call that went while one waited).
    pub(super) fn calls_answer_waiting(&mut self, now: deckpilot_calls::clock::Instant) {
        let Some(service) = self.calls.service.as_mut() else {
            // Calls went: nothing will confirm them.
            for (_, waiting) in std::mem::take(&mut self.calls.waiting.0) {
                let _ = waiting.reply.try_send(Err(CallsRefusal::new(
                    codes::PROVIDER_OFFLINE,
                    "Calls stopped",
                )));
            }
            return;
        };
        for event in service.driver.engine_mut().drain_events() {
            let EngineEvent::CommandDone { token, end } = event else {
                continue;
            };
            let Some(waiting) = self.calls.waiting.0.remove(&token) else {
                continue;
            };
            let answer = match end {
                CommandEnd::Confirmed => Ok(CallsAnswer::Control(CallControlReply::Confirmed)),
                CommandEnd::Unconfirmed => Err(CallsRefusal::new(
                    codes::UNCONFIRMED,
                    "the call app did not confirm it in time",
                )),
                CommandEnd::Rejected { code } => Err(rejected(&code)),
            };
            let _ = waiting.reply.try_send(answer);
        }
        let late: Vec<CommandToken> = self
            .calls
            .waiting
            .0
            .iter()
            .filter(|(_, w)| now >= w.until)
            .map(|(token, _)| *token)
            .collect();
        for token in late {
            if let Some(waiting) = self.calls.waiting.0.remove(&token) {
                let _ = waiting.reply.try_send(Err(CallsRefusal::new(
                    codes::UNCONFIRMED,
                    "the call app did not confirm it in time",
                )));
            }
        }
    }
}

/// A provider's refusal of a sent command: its reason when it is one of
/// the closed unsupported reasons, else unconfirmed with its code.
fn rejected(code: &str) -> CallsRefusal {
    use deckpilot_calls::provider::UnsupportedReason as R;
    let reasons = [
        R::ProviderHasNoApi,
        R::PlatformNotSupported,
        R::NotInMeeting,
        R::NoPermissionInMeeting,
        R::ToggleOnly,
        R::NeedsPairing,
        R::NeedsOsPermission,
        R::ConsentNotGiven,
    ];
    match reasons.into_iter().find(|r| r.code() == code) {
        Some(reason) => CallsRefusal::new(codes::UNSUPPORTED, reason.code()),
        None => CallsRefusal::new(
            codes::UNCONFIRMED,
            format!(
                "the call app refused it ({})",
                deckpilot_calls::snapshot::FailureCode::new(code)
            ),
        ),
    }
}
