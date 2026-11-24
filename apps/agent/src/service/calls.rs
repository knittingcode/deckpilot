//! Calls on the service task (`docs/calls.md`): the driver
//! ([`deckpilot_calls::driver::CallsDriver`]), its providers, the overlay
//! keys and the runs of a device that went "back to work".
//!
//! - **Off unless installed.** Without a provider (every build today: the
//!   detection-only Teams adapter and the plugin come later) nothing here
//!   runs and no device ever shows an overlay. Tests install the mock
//!   provider through the test hooks.
//! - **One clock.** The engine's clock is the runtime's folder clock (which
//!   tests move), counted from when Calls was installed. The loop looks at
//!   Calls whenever something is due and at most every
//!   [`POLL_INTERVAL`] while a provider is installed; nothing is spawned.
//! - **Overlay keys** go to the driver; a command it lets through goes to
//!   the provider, and a refusal back to the engine.
//! - **Runs of a suspended device** (Mode B and C): while a key of a
//!   device is handled, the device is noted; every run that key starts and
//!   that counts is followed to its real end, like the folder holds
//!   (`folders.rs`): an action run in place at once, a plugin's action at
//!   its `DetachedFinished`, a workflow at its `Execution::Finished`.

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use deckpilot_calls::clock::Instant;
use deckpilot_calls::driver::{CallsDriver, KeyOutcome};
use deckpilot_calls::engine::{Engine, Request};
use deckpilot_calls::layouts::{OWNER, Shape, Template, generate};
use deckpilot_calls::provider::{CallProvider, ConsentState, ProviderCx, ProviderSession};
use deckpilot_calls::ret::{RunEnd, RunKind, RunToken};
use deckpilot_core::{ActionResult, EngineError};
use deckpilot_core::{ExecutionId, ExecutionState, RuntimeOutcome};
use deckpilot_device::{DeviceId, DeviceModelId};
use tracing::{debug, info, warn};

use super::Agent;

/// How often an installed provider is polled at most apart.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// A provider, and a way to reach its concrete type (the test hooks
/// script the mock).
pub(crate) trait Provider: CallProvider {
    fn as_any(&mut self) -> &mut dyn Any;
}

impl<T: CallProvider + 'static> Provider for T {
    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

/// How a plugin's detached action ended, for a return session: a result
/// that says it was cancelled is a cancellation; an engine error (the
/// action could not run at all) is no completion; any other failure is a
/// failure.
pub(crate) fn detached_end(result: &Result<ActionResult, EngineError>) -> RunEnd {
    match result {
        Ok(result) if result.is_success() => RunEnd::Succeeded,
        Ok(result) if result.code.as_deref() == Some("cancelled") => RunEnd::Cancelled,
        Ok(_) => RunEnd::Failed,
        // It never ran: nothing completed (counts for nothing, like an
        // interrupted run).
        Err(_) => RunEnd::Interrupted,
    }
}

/// A run of a suspended device's key that counts for its return session.
#[derive(Debug, Clone)]
pub(crate) struct CallsRun {
    device: DeviceId,
    run: RunToken,
}

struct Service {
    driver: CallsDriver,
    provider: Box<dyn Provider>,
    origin: std::time::Instant,
    /// The templates of the managed layouts (the plugin's).
    templates: Vec<Template>,
    /// Models whose managed layout is loaded.
    seeded: BTreeSet<DeviceModelId>,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service")
            .field("driver", &self.driver)
            .finish_non_exhaustive()
    }
}

/// The Calls state of the agent.
#[derive(Debug, Default)]
pub(crate) struct AgentCalls {
    /// Boxed: the agent's state stays small (its loop is one future).
    service: Option<Box<Service>>,
    /// The device whose key is being handled.
    scope: Option<DeviceId>,
    /// Workflow runs that count for a return session, by execution.
    executions: BTreeMap<ExecutionId, CallsRun>,
    next_run: u64,
}

impl Agent {
    fn calls_now(&self) -> Option<Instant> {
        let origin = self.calls.service.as_ref()?.origin;
        let elapsed = self
            .runtime
            .folder_clock()
            .saturating_duration_since(origin);
        Some(Instant::from_millis(
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        ))
    }

    /// Installs Calls with `provider` and the consent the user gave
    /// (`calls:observe`, `calls:control`, the title opt-in; from the
    /// plugin's grants: never assumed here). The managed layouts are seeded
    /// for the models that connect (`calls_seed_connected`).
    pub(crate) fn install_calls(
        &mut self,
        mut provider: Box<dyn Provider>,
        consent: ConsentState,
        templates: Vec<Template>,
    ) {
        if !consent.observe {
            info!("calls not installed: no consent to observe calls");
            return;
        }
        let now = Instant::ZERO;
        let discovery = provider.discover(now);
        let id = provider.id();
        let mut engine = Engine::default();
        engine.add_provider(id.clone(), consent, discovery.capabilities);
        let driver = CallsDriver::new(engine);
        if let Err(error) = provider.connect(&ProviderCx {
            consent,
            session: ProviderSession(1),
        }) {
            warn!(%error, "the Calls provider did not connect");
        }
        info!(provider = %id, "calls installed");
        self.calls.service = Some(Box::new(Service {
            driver,
            provider,
            origin: self.runtime.folder_clock(),
            templates,
            seeded: BTreeSet::new(),
        }));
        self.calls_wake();
    }

    /// Seeds the managed layout of every connected model that has none
    /// loaded yet (`managed_layouts`: created once, regenerated while
    /// unedited, an edited one kept) and loads the stored one into the
    /// driver, so the user's edits are what the overlay shows. While the
    /// configuration is locked nothing is seeded; it is tried again.
    fn calls_seed_connected(&mut self) {
        let Some(service) = self.calls.service.as_mut() else {
            return;
        };
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let models: BTreeSet<DeviceModelId> = self
            .runtime
            .devices()
            .connected()
            .map(|d| d.model.clone())
            .filter(|m| !service.seeded.contains(m))
            .collect();
        for model_id in models {
            service.seeded.insert(model_id.clone());
            let Some(model) = self.runtime.catalog().get(&model_id) else {
                continue;
            };
            let shape = Shape::of(&model.capabilities);
            let Some(template) = service.templates.iter().find(|t| t.shape == shape) else {
                continue;
            };
            let mut generated = match generate(template, &model.id, &model.capabilities) {
                Ok(generated) => generated,
                Err(error) => {
                    debug!(model = %model.id, %error, "no Calls layout for the model");
                    continue;
                }
            };
            let version = format!("{}.0.0", template.version);
            match store
                .seed_managed_layout(OWNER, &generated.layout, &generated.actions, &version)
                .and_then(|_| store.managed_layout(OWNER, &model.id))
            {
                Ok(Some(stored)) => {
                    generated.layout = stored.layout;
                    generated.actions = stored.actions;
                }
                Ok(None) => {}
                Err(error) => {
                    warn!(model = %model.id, %error, "the Calls layout could not be stored");
                    continue;
                }
            }
            service.driver.set_layout(model.id.clone(), generated);
        }
    }

    /// Handles what is due and says how long until Calls next has to be
    /// looked at (`None`: Calls is not installed).
    pub(super) fn calls_wake(&mut self) -> Option<Duration> {
        let now = self.calls_now()?;
        self.calls_seed_connected();
        let service = self.calls.service.as_mut()?;
        let id = service.provider.id();
        let events = service.provider.poll(now);
        let changed = !events.is_empty();
        for event in events {
            service
                .driver
                .on_provider_event(&mut self.runtime, &id, event, now);
        }
        let due = service.driver.next_wake().is_some_and(|wake| wake <= now);
        if due || changed {
            service.driver.tick(&mut self.runtime, now);
        }
        if due || changed {
            self.render_all();
        }
        let next = self
            .calls
            .service
            .as_ref()
            .and_then(|s| s.driver.next_wake())
            .map(|wake| Duration::from_millis(wake.as_millis().saturating_sub(now.as_millis())));
        Some(next.map_or(POLL_INTERVAL, |n| n.min(POLL_INTERVAL)))
    }

    /// An overlay key's outcome: handled here (`true`) or not an overlay's.
    pub(super) fn calls_outcome(&mut self, device: &DeviceId, outcome: &RuntimeOutcome) -> bool {
        let Some(now) = self.calls_now() else {
            return false;
        };
        let Some(service) = self.calls.service.as_mut() else {
            return false;
        };
        match outcome {
            RuntimeOutcome::OverlayAction {
                session,
                action_type,
                parameters,
                ..
            } => {
                let handled = service.driver.overlay_key(
                    &mut self.runtime,
                    device,
                    *session,
                    action_type.as_str(),
                    parameters,
                    now,
                );
                match handled {
                    KeyOutcome::Send(Request::Send { command, token, .. }) => {
                        if let Err(refusal) = service.provider.command(command, token) {
                            service.driver.engine_mut().command_refused(token, &refusal);
                        }
                    }
                    KeyOutcome::Refused(error) => {
                        debug!(%device, code = error.code(), "call action refused");
                    }
                    _ => {}
                }
                service.driver.sync(&mut self.runtime, now);
                true
            }
            RuntimeOutcome::OverlayDismissed { session, .. } => {
                service.driver.dismissed(device, *session);
                true
            }
            _ => false,
        }
    }

    /// A device connected: it shows the overlay if the call needs it.
    pub(super) fn calls_device_connected(&mut self) {
        let Some(now) = self.calls_now() else {
            return;
        };
        // A new model gets its managed layout first.
        self.calls_seed_connected();
        if let Some(service) = self.calls.service.as_mut() {
            service.driver.device_connected(&mut self.runtime, now);
        }
    }

    /// Whether no key is being handled (a key inside another's handling
    /// belongs to the outer one).
    pub(super) fn calls_scope_is_free(&self) -> bool {
        self.calls.scope.is_none()
    }

    /// Notes that a key of `device` is being handled (`None`: done).
    pub(super) fn calls_scope(&mut self, device: Option<&DeviceId>) {
        self.calls.scope = device.cloned();
    }

    /// A run of the key being handled starts and ends later: it counts for
    /// the device's return session if the device is suspended and `kind`
    /// qualifies.
    pub(super) fn calls_hold_for_run(&mut self, kind: RunKind) -> Option<CallsRun> {
        let device = self.calls.scope.clone()?;
        let service = self.calls.service.as_mut()?;
        self.calls.next_run += 1;
        let run = RunToken(self.calls.next_run);
        service
            .driver
            .run_started(&device, run, kind)
            .then_some(CallsRun { device, run })
    }

    /// The run of `run` reached its end.
    pub(super) fn calls_run_ended(&mut self, run: Option<CallsRun>, end: RunEnd) {
        let (Some(run), Some(now)) = (run, self.calls_now()) else {
            return;
        };
        if let Some(service) = self.calls.service.as_mut() {
            service
                .driver
                .run_finished(&mut self.runtime, &run.device, run.run, end, now);
        }
        self.render_all();
    }

    /// A key ran something in place, or only navigated: over at once.
    pub(super) fn calls_ran_in_place(&mut self, kind: RunKind, succeeded: bool) {
        let run = self.calls_hold_for_run(kind);
        let end = if succeeded {
            RunEnd::Succeeded
        } else {
            RunEnd::Failed
        };
        self.calls_run_ended(run, end);
    }

    /// A workflow run started from the key being handled.
    pub(super) fn calls_execution_started(&mut self, execution: ExecutionId) {
        if let Some(run) = self.calls_hold_for_run(RunKind::Workflow) {
            self.calls.executions.insert(execution, run);
        }
    }

    /// A workflow run is over with `state`.
    pub(super) fn calls_execution_finished(
        &mut self,
        execution: ExecutionId,
        state: ExecutionState,
    ) {
        let run = self.calls.executions.remove(&execution);
        let end = match state {
            ExecutionState::Succeeded => RunEnd::Succeeded,
            ExecutionState::Failed => RunEnd::Failed,
            ExecutionState::Cancelled => RunEnd::Cancelled,
            _ => RunEnd::Interrupted,
        };
        self.calls_run_ended(run, end);
    }

    /// The driver, for the test hooks.
    #[cfg(feature = "test-hooks")]
    pub(crate) fn calls_driver(&self) -> Option<&CallsDriver> {
        self.calls.service.as_ref().map(|s| &s.driver)
    }

    /// The installed provider, for the test hooks.
    #[cfg(feature = "test-hooks")]
    pub(crate) fn calls_provider(&mut self) -> Option<&mut dyn Any> {
        self.calls.service.as_mut().map(|s| s.provider.as_any())
    }
}

/// The reference templates with `back` as the parameters of "Back to
/// work" (tests).
#[cfg(feature = "test-hooks")]
pub(crate) fn templates_with_back(back: &deckpilot_core::ActionParameters) -> Vec<Template> {
    let mut templates = deckpilot_calls::layouts::builtin_templates();
    for template in &mut templates {
        for (_, controls) in &mut template.pages {
            for control in controls.iter_mut().filter(|c| c.action == "return-to-work") {
                control.parameters = back.clone();
            }
        }
    }
    templates
}

/// What the test hooks see of Calls.
#[cfg(feature = "test-hooks")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallsView {
    pub state: deckpilot_calls::snapshot::CallState,
    /// Per device with an overlay frame: shown now, and the name of the
    /// overlay page (`In a call`, `Incoming call`, `Calling`).
    pub devices: Vec<(DeviceId, bool, Option<String>)>,
    /// The name of the overlay layout each device shows (the stored
    /// managed layout's).
    pub layouts: Vec<String>,
    pub scheduled_returns: usize,
}

/// An edit of the installed mock provider (test hooks).
#[cfg(feature = "test-hooks")]
pub type MockEdit = Box<dyn FnOnce(&mut deckpilot_calls::testing::MockProvider, Instant) + Send>;

/// An edit of the installed mock provider (test hooks), as a command.
#[cfg(feature = "test-hooks")]
pub struct CallsMockEdit(pub MockEdit);

#[cfg(feature = "test-hooks")]
impl std::fmt::Debug for CallsMockEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CallsMockEdit")
    }
}

#[cfg(feature = "test-hooks")]
impl Agent {
    pub(super) fn calls_view(&self) -> CallsView {
        let state = self
            .calls_driver()
            .map_or(deckpilot_calls::snapshot::CallState::Idle, |d| {
                d.engine().snapshot().state
            });
        let devices = self
            .runtime
            .all_overlays()
            .into_iter()
            .map(|(device, views)| {
                let shown = views.iter().any(|v| v.visible);
                let page = self
                    .runtime
                    .overlay_page(&device)
                    .and_then(|(layout, page)| layout.page(page).map(|p| p.name.clone()));
                (device, shown, page)
            })
            .collect();
        let layouts = self
            .runtime
            .all_overlays()
            .into_iter()
            .filter_map(|(device, _)| {
                self.runtime
                    .overlay_page(&device)
                    .map(|(l, _)| l.name.clone())
            })
            .collect();
        CallsView {
            state,
            devices,
            layouts,
            scheduled_returns: self.calls_driver().map_or(0, |d| d.returns().scheduled()),
        }
    }

    /// Applies a test edit to the installed mock (with the engine clock's
    /// now), then looks at what is due.
    pub(super) fn calls_edit_mock(&mut self, edit: CallsMockEdit) {
        let now = self.calls_now().unwrap_or(Instant::ZERO);
        if let Some(mock) = self
            .calls_provider()
            .and_then(|p| p.downcast_mut::<deckpilot_calls::testing::MockProvider>())
        {
            (edit.0)(mock, now);
        }
        self.calls_wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_detached_runs_end_counts_as_it_ended() {
        assert_eq!(
            detached_end(&Ok(ActionResult::success("ok"))),
            RunEnd::Succeeded
        );
        assert_eq!(
            detached_end(&Ok(ActionResult::failure("no"))),
            RunEnd::Failed
        );
        assert_eq!(
            detached_end(&Ok(ActionResult::error("cancelled", "stopped"))),
            RunEnd::Cancelled
        );
        assert_eq!(
            detached_end(&Ok(ActionResult::error("plugin-host-failed", "crashed"))),
            RunEnd::Failed
        );
        assert_eq!(
            detached_end(&Err(EngineError::UnknownAction(
                deckpilot_core::ActionInstanceId::new()
            ))),
            RunEnd::Interrupted,
            "could not run: no completion"
        );
    }
}
