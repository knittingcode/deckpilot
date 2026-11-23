//! Keys and gestures that start workflows (`docs/data-model.md`,
//! "Workflows"):
//!
//! - **Scope.** With the profile, the agent learns which absent workflows
//!   belong to another profile sharing a layout ("Not in this profile")
//!   and which were deleted ("Workflow missing"), and whether the
//!   Workflows plugin (`deckpilot.workflows`) is disabled: then keys and
//!   gestures that start a *graph* workflow are kept and do not run, and
//!   show "Workflows off". Multi Actions (step workflows) are the core's
//!   and keep running. Enabling the plugin again restores the runs.
//! - **Refusals.** A key or gesture whose workflow does not start is told
//!   to the user, once per key and reason until the configuration is
//!   loaded again, with a stable code (`notification.code`), instead of
//!   only being logged.

use std::collections::BTreeSet;

use deckpilot_core::{ExecutionTrigger, PageId, WorkflowId, WorkflowRefusal, WorkflowScope};
use deckpilot_device::{ControlId, DeviceId};
use deckpilot_ipc::IpcError;
use tracing::{debug, warn};

use super::Agent;

/// The first-party plugin that owns the Workflows screens (decisions
/// D1/D6 of 2026-11-20) and, while disabled, keeps graph workflows from
/// running from keys.
pub(crate) const WORKFLOWS_PLUGIN: &str = "deckpilot.workflows";

/// Where a refused start came from: a key or gesture of a device.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct RefusedKey {
    pub device: DeviceId,
    pub page: Option<PageId>,
    pub control: ControlId,
    pub workflow: WorkflowId,
    pub refusal: WorkflowRefusal,
}

/// The refusals told to the user since the configuration was loaded.
#[derive(Debug, Default)]
pub(super) struct RefusedKeys(BTreeSet<RefusedKey>);

impl RefusedKeys {
    /// Whether `key` is refused for the first time.
    fn first(&mut self, key: RefusedKey) -> bool {
        self.0.insert(key)
    }

    pub(super) fn clear(&mut self) {
        self.0.clear();
    }
}

/// Whether `trigger` is a key's or gesture's: those do not start graph
/// workflows while the Workflows plugin is off.
pub(super) fn from_key(trigger: ExecutionTrigger) -> bool {
    matches!(
        trigger,
        ExecutionTrigger::Device | ExecutionTrigger::Simulated
    )
}

/// The refusal `error` carries, if a workflow start was refused for one
/// of [`WorkflowRefusal`]'s reasons.
pub(super) fn refusal_of(error: &IpcError) -> Option<WorkflowRefusal> {
    error.reason.as_deref().and_then(WorkflowRefusal::from_code)
}

impl Agent {
    /// Whether the Workflows plugin is installed and cannot run (disabled,
    /// or not available here): keys then do not start graph workflows. A
    /// build without plugins, plugins turned off, or a plugin store not
    /// read yet behaves as before the extraction: graph workflows run.
    pub(crate) fn graph_workflows_off(&self) -> bool {
        deckpilot_core::ModuleId::new(WORKFLOWS_PLUGIN)
            .is_ok_and(|id| self.modules.contains(&id) && !self.modules.is_enabled(&id))
    }

    /// Sets the runtime's [`WorkflowScope`] for the active profile: the
    /// workflows of the profiles that share its layouts and whether graph
    /// workflows run from keys. Refusals are told again after it.
    pub(super) fn refresh_workflow_scope(&mut self) {
        self.refused_keys.clear();
        let foreign = match (self.store.as_ref(), self.runtime.profile()) {
            (Some(store), Some(profile)) => match store.workflows_sharing_layouts(profile) {
                Ok(foreign) => Some(foreign),
                Err(error) => {
                    warn!(%error, "the workflows of profiles sharing a layout could not be read");
                    None
                }
            },
            _ => None,
        };
        let scope = WorkflowScope {
            foreign,
            graphs_off: self.graph_workflows_off(),
        };
        if scope.graphs_off {
            debug!("the Workflows plugin is off: keys do not start graph workflows");
        }
        self.runtime.set_workflow_scope(scope);
    }

    /// The refusal of a key's start of `workflow` on `device`, before the
    /// engine's own checks: missing or another profile's (decided by the
    /// layout the device shows), or a graph workflow while the Workflows
    /// plugin is off. `None`: it may start.
    pub(super) fn key_refusal(
        &self,
        device: &DeviceId,
        workflow: WorkflowId,
    ) -> Option<WorkflowRefusal> {
        let profile = self.runtime.profile()?;
        let layout = self.runtime.layout_of(device).map(|layout| layout.id);
        self.runtime
            .workflow_scope()
            .refusal(profile, layout, workflow)
    }

    /// Tells the user that the key `control` of `device` (on `page`) did
    /// not start `workflow`, for `error`'s reason: once per key, workflow
    /// and reason until the configuration is loaded again. Errors without
    /// a workflow reason (locked, no profile) are only logged.
    pub(super) fn workflow_not_started(
        &mut self,
        device: &DeviceId,
        page: Option<PageId>,
        control: ControlId,
        workflow: WorkflowId,
        error: &IpcError,
    ) {
        let Some(refusal) = refusal_of(error) else {
            warn!(%error, %device, %control, "cannot start the workflow");
            return;
        };
        let first = self.refused_keys.first(RefusedKey {
            device: device.clone(),
            page,
            control,
            workflow,
            refusal,
        });
        if !first {
            debug!(%device, %control, code = refusal.code(), "workflow not started again");
            return;
        }
        let name = self
            .runtime
            .profile()
            .and_then(|profile| profile.workflow(workflow))
            .map_or_else(|| "A workflow".to_owned(), |w| w.name.clone());
        warn!(%device, %control, workflow = %name, code = refusal.code(), "the key's workflow did not start");
        let mut notification = deckpilot_core::notify::Notification::new(
            &format!("{name} did not start"),
            &format!(
                "{control}: {}. The key is kept.",
                capitalized(refusal.message())
            ),
            deckpilot_core::notify::Severity::Warning,
        )
        .with_code(refusal.code());
        notification.desktop = false;
        let notifier = crate::notifications::AgentNotifier {
            events: self.handle.events.clone(),
            desktop_clients: self.handle.desktop_clients.clone(),
            native: None,
        };
        if let Err(problem) = deckpilot_core::notify::Notifier::notify(&notifier, &notification) {
            debug!(%problem, "the refused workflow was not shown");
        }
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}
