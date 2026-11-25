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

/// The Calls action (id without the plugin's prefix) of `action`.
fn calls_action(action: &deckpilot_core::ActionInstance) -> Option<&str> {
    action
        .action_type
        .as_str()
        .strip_prefix(OWNER)
        .and_then(|rest| rest.strip_prefix('.'))
}

/// A managed layout's call pages as the app shows them: each control with
/// its Calls action (support is filled in from the provider when the
/// status is built, [`with_support`]).
fn layout_pages(
    managed: &deckpilot_storage::managed_layouts::ManagedLayout,
) -> Vec<deckpilot_ipc::calls::CallsPage> {
    use deckpilot_calls::layouts::{Page, page_id};
    use deckpilot_ipc::calls::{CallsControl, CallsPage, CallsPageKind};
    [
        (Page::Incoming, CallsPageKind::Incoming),
        (Page::Outgoing, CallsPageKind::Outgoing),
        (Page::Active, CallsPageKind::Active),
    ]
    .into_iter()
    .filter_map(|(page, kind)| {
        let layout_page = managed.layout.page(page_id(managed.layout.id, page))?;
        let controls = layout_page
            .bindings
            .iter()
            .filter_map(|binding| {
                let id = binding.target.action_id()?;
                let action = managed.actions.iter().find(|a| a.id == id)?;
                Some(CallsControl {
                    control: binding.control,
                    action: calls_action(action)?.to_owned(),
                    supported: true,
                    reason: None,
                })
            })
            .collect();
        Some(CallsPage {
            page: kind,
            name: layout_page.name.clone(),
            controls,
        })
    })
    .collect()
}

/// `pages` with each control's support by a provider of `capabilities`
/// now (an action that controls no call is always supported).
fn with_support(
    mut pages: Vec<deckpilot_ipc::calls::CallsPage>,
    capabilities: &deckpilot_calls::provider::CapabilitySet,
    consent: ConsentState,
) -> Vec<deckpilot_ipc::calls::CallsPage> {
    for control in pages.iter_mut().flat_map(|p| p.controls.iter_mut()) {
        let refused =
            match deckpilot_calls::ops::control_support(&control.action, capabilities, consent) {
                Some(Err(reason)) => Some(reason.code().to_owned()),
                Some(Ok(())) | None => None,
            };
        control.supported = refused.is_none();
        control.reason = refused;
    }
    pages
}

/// The "Back to work" actions a managed layout binds (one in every
/// template; a user may have bound it to more keys, or to none).
fn back_actions(
    managed: &deckpilot_storage::managed_layouts::ManagedLayout,
) -> Vec<&deckpilot_core::ActionInstance> {
    let bound: BTreeSet<deckpilot_core::ActionInstanceId> =
        managed.layout.bound_actions().into_iter().collect();
    managed
        .actions
        .iter()
        .filter(|a| calls_action(a) == Some("return-to-work") && bound.contains(&a.id))
        .collect()
}

/// The engine's *Back to work* settings as the app sees them.
fn ipc_back(settings: &deckpilot_calls::ret::ReturnSettings) -> deckpilot_ipc::calls::BackToWork {
    use deckpilot_calls::ret::{AfterRun, ReturnMode};
    use deckpilot_ipc::calls as ipc;
    let seconds = |d: Duration| u32::try_from(d.as_secs()).unwrap_or(u32::MAX);
    let after = |a: AfterRun| match a {
        AfterRun::Resume => ipc::AfterRun::Resume,
        AfterRun::Stay => ipc::AfterRun::Stay,
    };
    ipc::BackToWork {
        mode: match settings.mode.clamped() {
            ReturnMode::Timer { after } => ipc::BackToWorkMode::Timer {
                seconds: seconds(after),
            },
            ReturnMode::Completion => ipc::BackToWorkMode::Completion,
            ReturnMode::CompletionOrTimeout { max } => ipc::BackToWorkMode::CompletionOrMax {
                max_seconds: seconds(max),
            },
        },
        on_failure: after(settings.on_failure),
        on_cancel: after(settings.on_cancel),
        all_devices: settings.all_devices,
    }
}

/// `back` over `base` (what `back` does not name, such as whether
/// navigation counts, stays as it was).
fn engine_back(
    back: &deckpilot_ipc::calls::BackToWork,
    base: deckpilot_calls::ret::ReturnSettings,
) -> deckpilot_calls::ret::ReturnSettings {
    use deckpilot_calls::ret::{AfterRun, ReturnMode};
    use deckpilot_ipc::calls as ipc;
    let after = |a: ipc::AfterRun| match a {
        ipc::AfterRun::Resume => AfterRun::Resume,
        ipc::AfterRun::Stay => AfterRun::Stay,
    };
    deckpilot_calls::ret::ReturnSettings {
        mode: match back.mode {
            ipc::BackToWorkMode::Timer { seconds } => ReturnMode::Timer {
                after: Duration::from_secs(u64::from(seconds)),
            },
            ipc::BackToWorkMode::Completion => ReturnMode::Completion,
            ipc::BackToWorkMode::CompletionOrMax { max_seconds } => {
                ReturnMode::CompletionOrTimeout {
                    max: Duration::from_secs(u64::from(max_seconds)),
                }
            }
        }
        .clamped(),
        on_failure: after(back.on_failure),
        on_cancel: after(back.on_cancel),
        all_devices: back.all_devices,
        ..base
    }
}

/// How often an installed provider is polled at most apart.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// The first wait before a managed layout that could not be stored is
/// tried again; it doubles up to [`SEED_RETRY_MAX`].
const SEED_RETRY_FIRST: Duration = Duration::from_secs(1);
const SEED_RETRY_MAX: Duration = Duration::from_secs(60);

/// A model whose managed layout could not be stored because the database
/// was busy: when it is tried again (Calls clock), and the wait so far.
#[derive(Debug, Clone, Copy)]
struct SeedRetry {
    not_before: Instant,
    wait: Duration,
}

/// Whether `error` may pass by itself (another connection holds the
/// database): only then is seeding tried again.
fn passes(error: &deckpilot_storage::StorageError) -> bool {
    matches!(
        error,
        deckpilot_storage::StorageError::Database(rusqlite::Error::SqliteFailure(e, _))
            if matches!(e.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}

/// A provider, and a way to reach its concrete type (the test hooks
/// script the mock).
pub(crate) trait Provider: CallProvider {
    fn as_any(&mut self) -> &mut dyn Any;
}

impl std::fmt::Debug for dyn Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Provider({})", self.id())
    }
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

pub(super) struct Service {
    pub(super) driver: CallsDriver,
    pub(super) provider: Box<dyn Provider>,
    origin: std::time::Instant,
    /// The templates of the managed layouts (the plugin's).
    templates: Vec<Template>,
    /// Models whose managed layout is loaded, or that have none (no
    /// template, or an error that does not pass).
    seeded: BTreeSet<DeviceModelId>,
    /// Models to be tried again once the database is free.
    retries: BTreeMap<DeviceModelId, SeedRetry>,
    /// The consent the user gave (from the plugin's grants).
    pub(super) consent: ConsentState,
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
    pub(super) service: Option<Box<Service>>,
    /// The Calls plugin's controls waiting for their confirmation
    /// (`plugin_calls.rs`).
    #[cfg(feature = "plugins")]
    pub(super) waiting: super::plugin_calls::WaitingControls,
    /// The device whose key is being handled.
    scope: Option<DeviceId>,
    /// Workflow runs that count for a return session, by execution.
    executions: BTreeMap<ExecutionId, CallsRun>,
    next_run: u64,
    /// What `callsChanged` last sent.
    published: Option<Box<deckpilot_ipc::calls::CallsStatus>>,
    /// The status last answered or sent, and its revision
    /// ([`deckpilot_ipc::calls::CallsStatus::revision`]): counted up when
    /// the content changes.
    revision: Option<Box<deckpilot_ipc::calls::CallsStatus>>,
    /// The managed layouts as the app sees them, read from the store once
    /// and again only after [`Agent::calls_layouts_changed`] (never on the
    /// Calls tick). `None`: to be read.
    layouts: Option<Vec<deckpilot_ipc::calls::CallsLayout>>,
    /// How often the layouts were read from the store (test hooks).
    layouts_reads: u64,
    /// The editor's Calls page previews: by device, the connection that
    /// asked (one per device).
    previews: BTreeMap<DeviceId, u64>,
    /// Test hooks: the next seeds fail as if the database were busy.
    #[cfg(feature = "test-hooks")]
    fail_seeds: u32,
    /// Test hooks: how often a managed layout was stored or tried.
    #[cfg(feature = "test-hooks")]
    seed_attempts: u64,
}

fn not_installed() -> deckpilot_ipc::IpcError {
    deckpilot_ipc::IpcError::new(deckpilot_ipc::ErrorCode::NotFound, "Calls is not installed")
        .with_reason(deckpilot_ipc::calls::reason::NOT_INSTALLED)
}

fn unknown_device() -> deckpilot_ipc::IpcError {
    deckpilot_ipc::IpcError::new(
        deckpilot_ipc::ErrorCode::NotFound,
        "the device is not connected",
    )
    .with_reason(deckpilot_ipc::calls::reason::UNKNOWN_DEVICE)
}

fn no_layout() -> deckpilot_ipc::IpcError {
    deckpilot_ipc::IpcError::new(
        deckpilot_ipc::ErrorCode::NotFound,
        "this device model has no Calls layout",
    )
    .with_reason(deckpilot_ipc::calls::reason::NO_LAYOUT)
}

impl Agent {
    pub(super) fn calls_now(&self) -> Option<Instant> {
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
            retries: BTreeMap::new(),
            consent,
        }));
        self.calls_wake();
    }

    /// Seeds the managed layout of every connected model that has none
    /// loaded yet (`managed_layouts`: created once, regenerated while
    /// unedited, an edited one kept) and loads the stored one into the
    /// driver, so the user's edits are what the overlay shows. While the
    /// configuration is locked nothing is seeded; it is tried again.
    fn calls_seed_connected(&mut self) {
        let Some(now) = self.calls_now() else {
            return;
        };
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
            // Busy before: not before its wait is over.
            .filter(|m| service.retries.get(m).is_none_or(|r| now >= r.not_before))
            .collect();
        for model_id in models {
            let Some(model) = self.runtime.catalog().get(&model_id) else {
                continue;
            };
            let shape = Shape::of(&model.capabilities);
            let Some(template) = service.templates.iter().find(|t| t.shape == shape) else {
                // No template for this class: nothing to try again.
                service.seeded.insert(model_id.clone());
                continue;
            };
            let generated = match generate(template, &model.id, &model.capabilities) {
                Ok(generated) => generated,
                Err(error) => {
                    debug!(model = %model.id, %error, "no Calls layout for the model");
                    service.seeded.insert(model_id.clone());
                    continue;
                }
            };
            let version = format!("{}.0.0", template.version);
            #[cfg(feature = "test-hooks")]
            let failed = {
                self.calls.seed_attempts += 1;
                (self.calls.fail_seeds > 0).then(|| {
                    self.calls.fail_seeds -= 1;
                    deckpilot_storage::StorageError::Database(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                        None,
                    ))
                })
            };
            #[cfg(not(feature = "test-hooks"))]
            let failed: Option<deckpilot_storage::StorageError> = None;
            let stored = match failed {
                Some(error) => Err(error),
                None => store
                    .seed_managed_layout(OWNER, &generated.layout, &generated.actions, &version)
                    .and_then(|_| store.managed_layout(OWNER, &model.id)),
            };
            match stored {
                Ok(Some(stored)) => {
                    // Seeded: done for this model. The stored layout is
                    // used, found by its own ids (an edited one of an
                    // earlier template version too).
                    if service.retries.remove(&model_id).is_some() {
                        info!(model = %model.id, "the Calls layout is stored now");
                    }
                    service.seeded.insert(model_id.clone());
                    let used = generated.with_stored(stored.layout, stored.actions);
                    service.driver.set_layout(model.id.clone(), used);
                    self.calls.layouts = None;
                }
                // Seeded, yet not there to read: nothing to show for the
                // model, and nothing to gain by writing it again.
                Ok(None) => {
                    warn!(model = %model.id, "the Calls layout was stored but cannot be read");
                    service.seeded.insert(model_id.clone());
                }
                // The database is busy: tried again later, with a wait that
                // doubles up to a minute. Logged once per streak.
                Err(error) if passes(&error) => {
                    let wait = service
                        .retries
                        .get(&model_id)
                        .map_or(SEED_RETRY_FIRST, |r| (r.wait * 2).min(SEED_RETRY_MAX));
                    if !service.retries.contains_key(&model_id) {
                        warn!(model = %model.id, %error, "the Calls layout could not be stored yet; tried again later");
                    }
                    service.retries.insert(
                        model_id.clone(),
                        SeedRetry {
                            not_before: now.after(wait),
                            wait,
                        },
                    );
                }
                // Anything else does not pass by itself: not tried again
                // until the next agent start (the overlay is not shown on
                // the model meanwhile).
                Err(error) => {
                    warn!(
                        model = %model.id,
                        %error,
                        "the Calls layout could not be stored; not tried again until the agent restarts"
                    );
                    service.retries.remove(&model_id);
                    service.seeded.insert(model_id.clone());
                }
            }
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
        // The Calls plugin's controls whose command is done (or that
        // waited too long).
        #[cfg(feature = "plugins")]
        self.calls_answer_waiting(now);
        if due || changed {
            self.calls_prune_previews();
            self.render_all();
            self.calls_publish();
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
                // A dismissed preview is over, and the app is told.
                if !service.driver.previews().any(|d| d == device)
                    && self.calls.previews.remove(device).is_some()
                {
                    self.calls_publish();
                }
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
        self.calls_publish();
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
        self.calls_publish();
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

    /// A device went: the app's device list changes, and its preview ends.
    pub(super) fn calls_device_disconnected(&mut self, device: &DeviceId) {
        if self.calls.previews.remove(device).is_some()
            && let Some(service) = self.calls.service.as_mut()
        {
            service.driver.end_preview(&mut self.runtime, device);
        }
        self.calls_publish();
    }

    /// `previewCallsPage` and `endCallsPreview` of the desktop connection
    /// `viewer` (`server.rs` refuses any other client).
    pub(super) fn calls_preview(
        &mut self,
        viewer: u64,
        request: &deckpilot_ipc::Request,
    ) -> Result<deckpilot_ipc::Response, deckpilot_ipc::IpcError> {
        use deckpilot_calls::driver::PreviewError;
        use deckpilot_calls::layouts::Page;
        use deckpilot_ipc::calls::CallsPageKind;
        use deckpilot_ipc::{ErrorCode, IpcError, Request, Response};
        match request {
            Request::PreviewCallsPage { device, page } => {
                if self.store.is_none() {
                    return Err(IpcError::new(
                        ErrorCode::Locked,
                        "the configuration is locked",
                    ));
                }
                if !self.runtime.devices().connected().any(|d| d == device) {
                    return Err(unknown_device());
                }
                let service = self.calls.service.as_mut().ok_or_else(not_installed)?;
                let page = match page {
                    CallsPageKind::Incoming => Page::Incoming,
                    CallsPageKind::Outgoing => Page::Outgoing,
                    CallsPageKind::Active => Page::Active,
                };
                service
                    .driver
                    .preview(&mut self.runtime, device, page)
                    .map_err(|error| match error {
                        PreviewError::InCall => IpcError::new(
                            ErrorCode::Unsupported,
                            "a call shows on the device; it always wins over a preview",
                        )
                        .with_reason(deckpilot_ipc::calls::reason::PREVIEW_IN_CALL),
                        PreviewError::NoLayout => no_layout(),
                        PreviewError::Refused => {
                            IpcError::new(ErrorCode::Failed, "the device took no preview")
                                .with_reason(deckpilot_ipc::calls::reason::PREVIEW_REFUSED)
                        }
                    })?;
                // One per device: a new one replaces the old, whoever asked.
                self.calls.previews.insert(device.clone(), viewer);
                self.render_all();
                self.calls_publish();
                Ok(Response::Ok)
            }
            Request::EndCallsPreview { device } => {
                if self.calls.previews.get(device) == Some(&viewer) {
                    self.calls.previews.remove(device);
                    if let Some(service) = self.calls.service.as_mut() {
                        service.driver.end_preview(&mut self.runtime, device);
                    }
                    self.render_all();
                    self.calls_publish();
                }
                Ok(Response::Ok)
            }
            _ => Err(IpcError::new(
                ErrorCode::InvalidRequest,
                "not a Calls preview request",
            )),
        }
    }

    /// The connection `viewer` closed: its previews end, no frame stays.
    pub(super) fn calls_preview_closed(&mut self, viewer: u64) {
        let devices: Vec<DeviceId> = self
            .calls
            .previews
            .iter()
            .filter(|(_, v)| **v == viewer)
            .map(|(d, _)| d.clone())
            .collect();
        for device in &devices {
            self.calls.previews.remove(device);
            if let Some(service) = self.calls.service.as_mut() {
                service.driver.end_preview(&mut self.runtime, device);
            }
        }
        if !devices.is_empty() {
            self.render_all();
            self.calls_publish();
        }
    }

    /// Every preview ends (the configuration locked).
    pub(super) fn calls_end_previews(&mut self) {
        self.calls.previews.clear();
        if let Some(service) = self.calls.service.as_mut() {
            service.driver.end_previews(&mut self.runtime);
        }
        self.calls_publish();
    }

    /// Forgets the previews the driver ended by itself (a real call came,
    /// a dismiss on the device).
    fn calls_prune_previews(&mut self) {
        if let Some(service) = self.calls.service.as_ref() {
            let live: BTreeSet<DeviceId> = service.driver.previews().cloned().collect();
            self.calls
                .previews
                .retain(|device, _| live.contains(device));
        }
    }

    /// The managed layouts may have changed (seeding, *Restore defaults*,
    /// a configuration reload, a lock): they are read again and the app is
    /// told.
    pub(super) fn calls_layouts_changed(&mut self) {
        self.calls.layouts = None;
        self.calls_publish();
    }

    /// The managed layouts, from the store only when not read yet.
    fn calls_layouts(&mut self) -> Vec<deckpilot_ipc::calls::CallsLayout> {
        if let Some(layouts) = &self.calls.layouts {
            return layouts.clone();
        }
        let Some(store) = self.store.as_ref() else {
            // Locked: nothing to show, and read again once unlocked.
            return Vec::new();
        };
        self.calls.layouts_reads += 1;
        let layouts: Vec<deckpilot_ipc::calls::CallsLayout> = store
            .managed_layouts(OWNER)
            .unwrap_or_default()
            .into_iter()
            .map(|m| deckpilot_ipc::calls::CallsLayout {
                pages: layout_pages(&m),
                back: back_actions(&m)
                    .first()
                    .map(|a| ipc_back(&deckpilot_calls::driver::settings_of(&a.parameters))),
                model: m.model,
                layout: m.layout.id.to_string(),
                name: m.layout.name,
                edited: m.edited,
                template_version: m.template_version,
            })
            .collect();
        self.calls.layouts = Some(layouts.clone());
        layouts
    }

    /// Calls as the app shows it (`getCallsStatus`, `callsChanged`), with
    /// its revision: the last one's when nothing changed, else one more.
    pub(super) fn calls_status(&mut self) -> deckpilot_ipc::calls::CallsStatus {
        let mut status = self.calls_status_content();
        let last = self.calls.revision.as_deref();
        status.revision = last.map_or(0, |l| l.revision);
        if last != Some(&status) {
            status.revision += 1;
            self.calls.revision = Some(Box::new(status.clone()));
        }
        status
    }

    /// [`Agent::calls_status`] without its revision.
    fn calls_status_content(&mut self) -> deckpilot_ipc::calls::CallsStatus {
        if self.calls.service.is_none() {
            return deckpilot_ipc::calls::CallsStatus::default();
        }
        let layouts = self.calls_layouts();
        let Some(service) = self.calls.service.as_ref() else {
            return deckpilot_ipc::calls::CallsStatus::default();
        };
        let snapshot = service.driver.engine().snapshot();
        let overlays = self.runtime.all_overlays();
        let devices = self
            .runtime
            .devices()
            .connected()
            .map(|device| {
                let views = overlays
                    .iter()
                    .find(|(d, _)| d == device)
                    .map(|(_, v)| v.as_slice())
                    .unwrap_or_default();
                deckpilot_ipc::calls::CallsDevice {
                    device: device.clone(),
                    overlay: service.driver.overlay_enabled(&self.runtime, device),
                    shown: views.iter().any(|v| v.visible),
                    suspended: views.iter().any(|v| v.suspended),
                    preview: service.driver.preview_page(device).map(|page| match page {
                        deckpilot_calls::layouts::Page::Incoming => {
                            deckpilot_ipc::calls::CallsPageKind::Incoming
                        }
                        deckpilot_calls::layouts::Page::Outgoing => {
                            deckpilot_ipc::calls::CallsPageKind::Outgoing
                        }
                        deckpilot_calls::layouts::Page::Active => {
                            deckpilot_ipc::calls::CallsPageKind::Active
                        }
                    }),
                }
            })
            .collect();
        // The controls' support changes with the provider, not with the
        // stored layout: filled in now, not cached.
        let layouts = layouts
            .into_iter()
            .map(|mut layout| {
                layout.pages = with_support(layout.pages, &snapshot.capabilities, service.consent);
                layout
            })
            .collect();
        deckpilot_ipc::calls::CallsStatus {
            installed: true,
            state: snapshot.state.name().to_owned(),
            provider: snapshot.provider.map(|p| p.to_string()),
            // Nothing controls without the control consent.
            controls: service.consent.control && snapshot.capabilities.controls_anything(),
            devices,
            layouts,
            revision: 0,
        }
    }

    /// Sends `callsChanged` when what the app shows changed.
    pub(super) fn calls_publish(&mut self) {
        if self.calls.service.is_none() {
            return;
        }
        let status = self.calls_status();
        if self.calls.published.as_deref() == Some(&status) {
            return;
        }
        self.calls.published = Some(Box::new(status.clone()));
        let _ = self
            .handle
            .events
            .send(deckpilot_ipc::AgentEvent::CallsChanged {
                status: Box::new(status),
            });
    }

    /// `setCallsOverlay`.
    pub(super) fn set_calls_overlay(
        &mut self,
        device: &DeviceId,
        on: bool,
    ) -> Result<deckpilot_ipc::Response, deckpilot_ipc::IpcError> {
        let now = self.calls_now().ok_or_else(not_installed)?;
        // Only a connected device: the setting map does not grow with
        // serials that name nothing.
        if !self.runtime.devices().connected().any(|d| d == device) {
            return Err(unknown_device());
        }
        let service = self.calls.service.as_mut().ok_or_else(not_installed)?;
        service
            .driver
            .set_device_enabled(&mut self.runtime, device, on, now);
        self.render_all();
        self.calls_publish();
        Ok(deckpilot_ipc::Response::CallsStatus {
            status: Box::new(self.calls_status()),
        })
    }

    /// `setCallsBackToWork`: the *Back to work* settings of `model`'s
    /// managed layout, written as the parameters of its "Back to work"
    /// action (a user edit: the layout counts as edited), loaded into the
    /// driver and used from the next press.
    pub(super) fn set_calls_back_to_work(
        &mut self,
        model: &DeviceModelId,
        back: &deckpilot_ipc::calls::BackToWork,
    ) -> Result<deckpilot_ipc::Response, deckpilot_ipc::IpcError> {
        use deckpilot_ipc::{ErrorCode, IpcError};
        // Refused, never clamped: the app shows what it set.
        back.validate()
            .map_err(|message| IpcError::new(ErrorCode::InvalidRequest, message))?;
        if self.calls.service.is_none() {
            return Err(not_installed());
        }
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| IpcError::new(ErrorCode::Locked, "the configuration is locked"))?;
        let managed = store
            .managed_layout(OWNER, model)
            .map_err(|error| IpcError::new(ErrorCode::Failed, error.to_string()))?
            .ok_or_else(no_layout)?;
        let actions = back_actions(&managed);
        if actions.is_empty() {
            return Err(IpcError::new(
                ErrorCode::NotFound,
                "this Calls layout has no Back to work key",
            )
            .with_reason(deckpilot_ipc::calls::reason::NO_BACK_TO_WORK));
        }
        // Every "Back to work" key of the layout gets the settings, in one
        // transaction; each keeps the parameters these do not name (merged,
        // not replaced).
        let changes: Vec<_> = actions
            .into_iter()
            .map(|action| {
                let settings = engine_back(
                    back,
                    deckpilot_calls::driver::settings_of(&action.parameters),
                );
                let mut parameters = action.parameters.clone();
                for (key, value) in deckpilot_calls::driver::parameters_of(&settings).iter() {
                    parameters = parameters.with(key.clone(), value.clone());
                }
                (action.id, parameters)
            })
            .collect();
        store
            .set_managed_actions_parameters(OWNER, model, &changes)
            .map_err(|error| match error {
                deckpilot_storage::StorageError::UnknownLayout => no_layout(),
                other => IpcError::new(ErrorCode::Failed, other.to_string()),
            })?;
        // Loaded again from the store (the edited layout is kept as it is).
        if let Some(service) = self.calls.service.as_mut() {
            service.seeded.remove(model);
        }
        self.calls_seed_connected();
        if let Some(now) = self.calls_now()
            && let Some(service) = self.calls.service.as_mut()
        {
            service.driver.sync(&mut self.runtime, now);
        }
        self.render_all();
        self.calls_layouts_changed();
        Ok(deckpilot_ipc::Response::CallsStatus {
            status: Box::new(self.calls_status()),
        })
    }

    /// `restoreCallsLayout`: the user's version kept as `copy_name`, the
    /// layout generated again and shown.
    pub(super) fn restore_calls_layout(
        &mut self,
        model: &DeviceModelId,
        copy_name: &str,
    ) -> Result<deckpilot_ipc::Response, deckpilot_ipc::IpcError> {
        use deckpilot_ipc::{ErrorCode, IpcError};
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| IpcError::new(ErrorCode::Locked, "the configuration is locked"))?;
        let service = self.calls.service.as_mut().ok_or_else(not_installed)?;
        let Some(catalog_model) = self.runtime.catalog().get(model) else {
            return Err(no_layout());
        };
        let shape = Shape::of(&catalog_model.capabilities);
        let template = service
            .templates
            .iter()
            .find(|t| t.shape == shape)
            .ok_or_else(no_layout)?;
        let generated =
            generate(template, model, &catalog_model.capabilities).map_err(|_| no_layout())?;
        let version = format!("{}.0.0", template.version);
        let copy = store
            .restore_managed_layout(
                OWNER,
                &generated.layout,
                &generated.actions,
                &version,
                copy_name.trim(),
            )
            .map_err(|error| match error {
                deckpilot_storage::StorageError::UnknownLayout => no_layout(),
                other => IpcError::new(ErrorCode::Failed, other.to_string()),
            })?;
        let used = match store.managed_layout(OWNER, model) {
            Ok(Some(stored)) => generated.with_stored(stored.layout, stored.actions),
            _ => generated,
        };
        service.driver.set_layout(model.clone(), used);
        if let Some(now) = self.calls_now()
            && let Some(service) = self.calls.service.as_mut()
        {
            service.driver.sync(&mut self.runtime, now);
        }
        self.render_all();
        self.calls_layouts_changed();
        Ok(deckpilot_ipc::Response::CallsLayoutRestored {
            copy: copy.map(|id| id.to_string()),
        })
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
    /// The devices with an editor preview.
    pub previews: usize,
    /// How many Calls overlay frames are stacked, on every device.
    pub frames: usize,
    pub scheduled_returns: usize,
    /// How often the managed layouts were read from the store for the app.
    pub layouts_reads: u64,
    /// How often a managed layout was stored or tried.
    pub seed_attempts: u64,
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
        let frames = self
            .runtime
            .all_overlays()
            .into_iter()
            .flat_map(|(_, views)| views)
            .filter(|view| view.owner == deckpilot_core::OverlayOwner::Calls)
            .count();
        CallsView {
            previews: self.calls.previews.len(),
            frames,
            state,
            devices,
            layouts,
            scheduled_returns: self.calls_driver().map_or(0, |d| d.returns().scheduled()),
            layouts_reads: self.calls.layouts_reads,
            seed_attempts: self.calls.seed_attempts,
        }
    }

    /// The next `count` seeds of a managed layout fail as if another
    /// connection held the database.
    pub(super) fn calls_fail_seeds(&mut self, count: u32) {
        self.calls.fail_seeds = count;
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
