//! Draft actions: configurations saved before their required parameters
//! are filled in.
//!
//! An action instance, a key's or gesture's binding, a Multi Action step
//! or a workflow node may be saved while a required parameter of its type
//! has no value ([`crate::ActionType::normalize_draft`]). Every other
//! problem is still refused when saving. Such a use is *incomplete*: it is
//! kept and reported here, and it never runs. Before any handler is
//! called, the engine checks completeness on the effective parameters (the
//! instance's, the overrides, resolved references) and refuses the run
//! with [`CONFIGURATION_INCOMPLETE`] (`crate::resolution`). A workflow graph
//! with an incomplete node does not start (its issue stays an error,
//! marked with this code, and only saving ignores it).
//!
//! Incomplete is derived from the type's schema whenever it is needed and
//! never stored, so nothing in a profile changes shape: agents before
//! 0.14 load such profiles as they are and refuse the same runs, before
//! the handler, as invalid configuration (`docs/action-architecture.md`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use deckpilot_device::ControlId;

use crate::graph::{NodeId, NodeKind};
use crate::{
    ActionInstanceId, ActionParameters, ActionRegistry, BindingTarget, DeviceLayout, LayoutId,
    PageId, Profile, Trigger, Workflow, WorkflowId,
};

/// The result code of a run refused because required parameters have no
/// value; [`crate::ActionResult::data`] `missing` holds their keys,
/// separated by commas. Also the code of such a graph issue.
pub const CONFIGURATION_INCOMPLETE: &str = "configuration-incomplete";

/// Where an incomplete action is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum IncompleteSite {
    /// A key's, dial's or touch zone's binding, for any trigger (press,
    /// click, double click, long press, release, rotation, touch).
    Binding {
        layout: LayoutId,
        page: PageId,
        control: ControlId,
        trigger: Trigger,
    },
    /// A step of a Multi Action (0-based).
    Step { workflow: WorkflowId, index: usize },
    /// An action node of a workflow graph.
    Node { workflow: WorkflowId, node: NodeId },
}

/// A use of an action whose effective parameters leave required ones
/// without a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IncompleteUse {
    pub site: IncompleteSite,
    pub action: ActionInstanceId,
    /// Parameter keys, in the type's order.
    pub missing: Vec<String>,
}

/// The missing parameters of `action` of `profile` with `overrides` over
/// its own values. Empty for a complete use, and for an action that is not
/// there or whose type cannot run here (reported as such elsewhere).
pub fn missing_for(
    profile: &Profile,
    registry: &ActionRegistry,
    action: ActionInstanceId,
    overrides: &ActionParameters,
) -> Vec<String> {
    let Some(instance) = profile.action(action) else {
        return Vec::new();
    };
    let Some(definition) = registry.definition(&instance.action_type) else {
        return Vec::new();
    };
    if overrides.is_empty() {
        definition.missing(&instance.parameters)
    } else {
        definition.missing(&instance.parameters.overlay(overrides))
    }
}

/// Every action of `profile` that needs configuration, with its missing
/// parameters (its own values, without any use's overrides).
pub fn incomplete_actions(
    profile: &Profile,
    registry: &ActionRegistry,
) -> BTreeMap<ActionInstanceId, Vec<String>> {
    profile
        .actions
        .iter()
        .filter_map(|instance| {
            let missing = missing_for(profile, registry, instance.id, &ActionParameters::new());
            (!missing.is_empty()).then_some((instance.id, missing))
        })
        .collect()
}

/// The incomplete bindings of `layout`, an action of `profile` each.
pub fn layout_incomplete(
    layout: &DeviceLayout,
    profile: &Profile,
    registry: &ActionRegistry,
) -> Vec<IncompleteUse> {
    let mut found = Vec::new();
    for page in &layout.pages {
        for binding in &page.bindings {
            let BindingTarget::Action { action, overrides } = &binding.target else {
                continue;
            };
            let missing = missing_for(profile, registry, *action, overrides);
            if !missing.is_empty() {
                found.push(IncompleteUse {
                    site: IncompleteSite::Binding {
                        layout: layout.id,
                        page: page.id,
                        control: binding.control,
                        trigger: binding.trigger,
                    },
                    action: *action,
                    missing,
                });
            }
        }
    }
    found
}

/// The incomplete steps or nodes of `workflow` in `profile`. A node's
/// parameters filled by a mapping are not missing: they get their value
/// when the node runs, and are checked then.
pub fn workflow_incomplete(
    workflow: &Workflow,
    profile: &Profile,
    registry: &ActionRegistry,
) -> Vec<IncompleteUse> {
    let mut found = Vec::new();
    match &workflow.graph {
        Some(graph) => {
            for node in &graph.nodes {
                let NodeKind::Action {
                    action,
                    overrides,
                    mappings,
                    ..
                } = &node.kind
                else {
                    continue;
                };
                let missing: Vec<String> = missing_for(profile, registry, *action, overrides)
                    .into_iter()
                    .filter(|key| !mappings.contains_key(key))
                    .collect();
                if !missing.is_empty() {
                    found.push(IncompleteUse {
                        site: IncompleteSite::Node {
                            workflow: workflow.id,
                            node: node.id.clone(),
                        },
                        action: *action,
                        missing,
                    });
                }
            }
        }
        None => {
            for (index, step) in workflow.steps.iter().enumerate() {
                let missing = missing_for(profile, registry, step.action, &step.overrides);
                if !missing.is_empty() {
                    found.push(IncompleteUse {
                        site: IncompleteSite::Step {
                            workflow: workflow.id,
                            index,
                        },
                        action: step.action,
                        missing,
                    });
                }
            }
        }
    }
    found
}

/// Every incomplete use in `profile`: its layouts' bindings, then its
/// workflows' steps and nodes.
pub fn incomplete_uses(profile: &Profile, registry: &ActionRegistry) -> Vec<IncompleteUse> {
    let mut found: Vec<IncompleteUse> = profile
        .layouts
        .iter()
        .flat_map(|layout| layout_incomplete(layout, profile, registry))
        .collect();
    for workflow in &profile.workflows {
        found.extend(workflow_incomplete(workflow, profile, registry));
    }
    found
}

#[cfg(test)]
mod tests;
