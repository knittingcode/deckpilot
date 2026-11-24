//! Action types of installed plugins, as the agent reported them
//! (experimental; IPC protocol 10, capability `plugins`,
//! `docs/marketplace/agent-integration.md`).
//!
//! The agent is the source of truth: every answer of the plugin commands
//! that lists or changes installed plugins is recorded here (see
//! `commands::plugins`), and nothing else adds to it. Plugin actions never
//! run in this process; the definitions only let the editors validate and
//! save actions, workflow nodes and keys that use them, exactly as the
//! agent's engine will (`apps/agent/src/plugins/engine.rs`). Until the
//! frontend asks for the installed plugins (only builds with
//! `VITE_DECKPILOT_PLUGINS=1` do) this stays empty and the registry is
//! the built-in modules' alone.

use std::collections::BTreeMap;

use deckpilot_core::{
    ActionHandler, ActionParameters, ActionRegistry, ActionResult, ActionType, AliasHandler,
    ExecutionContext, ModuleId,
};
use deckpilot_ipc::plugins::{PluginState, PluginSummary};

/// `definition` with its editor navigation (plugin API 1.7) only when the
/// host's own rules accept it: an allowlisted operation whose target is one
/// of the action's text parameters. The editor maps it to its own
/// navigation; nothing of the plugin runs for it.
fn checked(definition: &ActionType) -> ActionType {
    let mut definition = definition.clone();
    if definition
        .editor_navigation
        .as_ref()
        .is_some_and(|navigation| navigation.check(&definition.parameters).is_err())
    {
        definition.editor_navigation = None;
    }
    definition
}

/// The installed plugins of the last answer, by plugin id.
#[derive(Debug, Default)]
pub struct PluginActions {
    plugins: BTreeMap<String, PluginSummary>,
    /// Whether the frontend ever asked for plugins (only builds with
    /// `VITE_DECKPILOT_PLUGINS=1` do). Until then the features plugins
    /// took over keep their behaviour from before the extraction
    /// ([`PluginActions::feature`]).
    answered: bool,
}

/// Whether a feature a first-party plugin owns (`deckpilot.workflows`,
/// `deckpilot.projects`) is on, as far as this process knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureState {
    /// No plugin answer yet: a build without plugins, where the feature
    /// behaves as before the extraction.
    Unknown,
    /// The plugin is installed and enabled.
    Enabled,
    /// Plugins were listed, and this one is not enabled (or not there).
    Off,
}

impl PluginActions {
    /// The installed plugins are exactly `plugins` (a `listPlugins` answer).
    pub fn replace(&mut self, plugins: &[PluginSummary]) {
        self.answered = true;
        self.plugins = plugins
            .iter()
            .map(|plugin| (plugin.id.clone(), plugin.clone()))
            .collect();
    }

    /// One plugin changed (the answer to `getPlugin`, an install, enable…).
    pub fn upsert(&mut self, plugin: &PluginSummary) {
        self.answered = true;
        self.plugins.insert(plugin.id.clone(), plugin.clone());
    }

    /// The plugin was uninstalled.
    pub fn remove(&mut self, plugin: &str) {
        self.answered = true;
        self.plugins.remove(plugin);
    }

    /// Plugins cannot be asked for (no agent, plugins off, locked): none
    /// is known, so their actions are unknown action types again, and the
    /// features plugins own are off.
    pub fn clear(&mut self) {
        self.answered = true;
        self.plugins.clear();
    }

    /// The state of the feature `plugin` owns.
    pub fn feature(&self, plugin: &str) -> FeatureState {
        if !self.answered {
            FeatureState::Unknown
        } else if self
            .plugins
            .get(plugin)
            .is_some_and(|p| p.state == PluginState::Enabled)
        {
            FeatureState::Enabled
        } else {
            FeatureState::Off
        }
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Adds the plugin action types to `registry`: available ones as
    /// definitions (they never run here), the others as unavailable with
    /// the agent's reason, worded as the agent's engine words it. Entries
    /// outside their plugin's namespace are ignored, so an answer can never
    /// replace or hide a built-in action type.
    pub fn extend(&self, registry: &mut ActionRegistry) {
        for plugin in self.plugins.values() {
            let Ok(module) = ModuleId::new(plugin.id.clone()) else {
                continue;
            };
            if !module.is_plugin() {
                continue;
            }
            let name = display_name(plugin);
            for action in &plugin.actions {
                let Some(definition) = &action.definition else {
                    continue;
                };
                if definition.id.as_str() != action.action_type
                    || ModuleId::owner_of(&definition.id).as_ref() != Some(&module)
                {
                    continue;
                }
                if action.available {
                    // A duplicate cannot happen inside one namespace; if it
                    // did, the first definition stays.
                    let _ = registry.register_for_module(
                        module.clone(),
                        &[],
                        Box::new(AgentAction {
                            definition: checked(definition),
                        }),
                    );
                } else {
                    let why = action
                        .unavailable_reason
                        .as_deref()
                        .unwrap_or("the plugin cannot run now");
                    registry.mark_unavailable(
                        [definition.id.clone()],
                        format!("{name} ({}): {why}", plugin.id),
                    );
                }
            }
        }
        self.extend_legacy(registry);
    }

    /// Legacy built-in action types a reported first-party plugin now
    /// serves ([`deckpilot_core::alias::ACTION_ALIASES`]), as the agent's
    /// registry routes them: under the legacy id with the plugin's
    /// definition, marked deprecated (an [`AliasHandler`] around the
    /// plugin's type), so keys, actions and workflow steps stored with the
    /// legacy id are drawn, validated and saved as their plugin action.
    /// While the plugin's action cannot run, the legacy type is unavailable
    /// with the plugin's reason. A legacy type whose built-in is still
    /// compiled in keeps it here; a plugin that is not reported leaves the
    /// core's reason (not installed) in place.
    ///
    /// Before this, the app knew the legacy types of the removed built-ins
    /// (`projects`, `git`, …) only as "not installed", whatever the agent
    /// ran, so their keys were previewed as unavailable after the plugin
    /// took them over.
    fn extend_legacy(&self, registry: &mut ActionRegistry) {
        for alias in deckpilot_core::alias::ACTION_ALIASES {
            let legacy = alias.legacy_id();
            if registry.definition(&legacy).is_some() {
                continue;
            }
            let Some(plugin) = self.plugins.get(alias.plugin) else {
                continue;
            };
            let Some(action) = plugin
                .actions
                .iter()
                .find(|action| action.action_type == alias.target)
            else {
                let name = display_name(plugin);
                registry.mark_unavailable(
                    [legacy],
                    format!("{name} ({}) does not provide `{}`", plugin.id, alias.target),
                );
                continue;
            };
            match &action.definition {
                Some(definition) if action.available && definition.id.as_str() == alias.target => {
                    let handler = AliasHandler::new(
                        alias,
                        Box::new(AgentAction {
                            definition: checked(definition),
                        }),
                        None,
                        None,
                    );
                    let _ = registry.register_for_module(alias.plugin_id(), &[], Box::new(handler));
                }
                _ => {
                    let why = action
                        .unavailable_reason
                        .as_deref()
                        .unwrap_or("the plugin cannot run now");
                    registry.mark_unavailable(
                        [legacy],
                        format!("{} ({}): {why}", display_name(plugin), plugin.id),
                    );
                }
            }
        }
    }
}

/// The plugin's name, else its id.
fn display_name(plugin: &PluginSummary) -> &str {
    if plugin.name.is_empty() {
        plugin.id.as_str()
    } else {
        plugin.name.as_str()
    }
}

/// A plugin action type for validation only: the agent runs it.
struct AgentAction {
    definition: ActionType,
}

impl ActionHandler for AgentAction {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        _parameters: &ActionParameters,
        _context: &ExecutionContext<'_>,
    ) -> ActionResult {
        ActionResult::failure("plugin actions run in the DeckPilot agent")
    }
}

#[cfg(test)]
mod tests {
    use deckpilot_core::{ActionTypeId, ParameterValue, RegistryError};
    use deckpilot_ipc::plugins::{PluginActionInfo, PluginOrigin};

    use super::*;

    fn definition(id: &str) -> ActionType {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": "Greet",
            "description": "Says hello.",
            "category": "Fun",
            "parameters": [
                {"key": "name", "label": "Name", "kind": {"type": "text"}, "required": true}
            ],
            "requiresProject": false,
        }))
        .unwrap()
    }

    fn plugin(id: &str, available: bool) -> PluginSummary {
        let action_type = format!("{id}.greet");
        PluginSummary {
            id: id.into(),
            name: "Hello Weather".into(),
            publisher: "acme".into(),
            version: "1.0.0".into(),
            state: if available {
                PluginState::Enabled
            } else {
                PluginState::Disabled
            },
            state_reason: None,
            origin: PluginOrigin::Sideload,
            previous_version: None,
            staged_version: None,
            actions: vec![PluginActionInfo {
                action: "greet".into(),
                action_type: action_type.clone(),
                name: "Greet".into(),
                description: None,
                available,
                unavailable_reason: (!available).then(|| "the plugin is disabled".into()),
                definition: Some(definition(&action_type)),
            }],
            action_count: None,
            health: None,
            navigation: Vec::new(),
            installed_for: None,
            visual_assets: Vec::new(),
            status_items: Vec::new(),
            dashboard_cards: Vec::new(),
            gesture_presets: Vec::new(),
        }
    }

    fn greet() -> ActionTypeId {
        ActionTypeId::new("acme.hello-weather.greet").unwrap()
    }

    fn registry(actions: &PluginActions) -> ActionRegistry {
        let mut registry =
            deckpilot_modules::builtin(deckpilot_core::Os::current()).action_registry();
        actions.extend(&mut registry);
        registry
    }

    /// Plugin API 1.7: a plugin's editor navigation reaches the catalog
    /// only as the host's rules allow; the plugin never runs for it.
    #[test]
    fn editor_navigation_reaches_the_catalog_only_when_it_checks_out() {
        use deckpilot_core::{EditorNavigation, NavigationOperation};
        let with = |navigation: serde_json::Value| {
            let mut summary = plugin("acme.hello-weather", true);
            let mut definition = serde_json::to_value(definition(&greet().to_string())).unwrap();
            definition["editorNavigation"] = navigation;
            summary.actions[0].definition = Some(serde_json::from_value(definition).unwrap());
            let mut actions = PluginActions::default();
            actions.replace(&[summary]);
            registry(&actions)
                .definition(&greet())
                .unwrap()
                .editor_navigation
                .clone()
        };
        assert_eq!(
            with(serde_json::json!({"operation": "folder.open", "targetParameter": "name"})),
            Some(EditorNavigation::with_target(
                NavigationOperation::FolderOpen,
                "name"
            ))
        );
        assert_eq!(
            with(serde_json::json!({"operation": "folder.open", "targetParameter": "nope"})),
            None
        );
        assert_eq!(
            with(serde_json::json!({"operation": "page.next", "targetParameter": "name"})),
            None
        );
    }

    #[test]
    fn nothing_known_changes_nothing() {
        let before = deckpilot_modules::builtin(deckpilot_core::Os::current()).action_registry();
        let after = registry(&PluginActions::default());
        assert_eq!(
            before.action_types().map(|t| &t.id).collect::<Vec<_>>(),
            after.action_types().map(|t| &t.id).collect::<Vec<_>>()
        );
        assert!(after.definition(&greet()).is_none());
        assert!(after.unavailable_reason(&greet()).is_none());
    }

    #[test]
    fn available_actions_validate_like_in_the_agent() {
        let mut actions = PluginActions::default();
        actions.replace(&[plugin("acme.hello-weather", true)]);
        let registry = registry(&actions);
        assert_eq!(
            registry.module_of(&greet()).map(ModuleId::as_str),
            Some("acme.hello-weather")
        );
        let normalized = registry
            .normalize(
                &greet(),
                &ActionParameters::new().with("name", ParameterValue::Text("Ada".into())),
            )
            .unwrap();
        assert_eq!(
            normalized.get("name"),
            Some(&ParameterValue::Text("Ada".into()))
        );
        // A required parameter is required here as in the agent.
        assert!(matches!(
            registry.normalize(&greet(), &ActionParameters::new()),
            Err(RegistryError::InvalidParameters(_))
        ));
    }

    #[test]
    fn unavailable_actions_say_why_and_cannot_be_created() {
        let mut actions = PluginActions::default();
        actions.replace(&[plugin("acme.hello-weather", false)]);
        let registry = registry(&actions);
        assert!(registry.definition(&greet()).is_none());
        assert_eq!(
            registry.unavailable_reason(&greet()),
            Some("Hello Weather (acme.hello-weather): the plugin is disabled")
        );
        assert!(matches!(
            registry.create_instance(
                "Greet",
                greet(),
                &ActionParameters::new().with("name", ParameterValue::Text("Ada".into()))
            ),
            Err(RegistryError::Unavailable(..))
        ));
    }

    #[test]
    fn changes_follow_the_agent() {
        let mut actions = PluginActions::default();
        actions.replace(&[plugin("acme.hello-weather", true)]);
        actions.upsert(&plugin("acme.hello-weather", false));
        assert!(registry(&actions).definition(&greet()).is_none());
        actions.remove("acme.hello-weather");
        assert!(actions.is_empty());
        let registry = registry(&actions);
        assert!(registry.definition(&greet()).is_none());
        assert!(registry.unavailable_reason(&greet()).is_none());
        actions.upsert(&plugin("acme.hello-weather", true));
        actions.clear();
        assert!(actions.is_empty());
    }

    /// The first-party Projects plugin as the agent reports it, with its
    /// `next` action available or not.
    fn projects_plugin(available: bool, reason: Option<&str>) -> PluginSummary {
        let mut summary = plugin("deckpilot.projects", available);
        summary.name = "Projects".into();
        summary.publisher = "deckpilot".into();
        summary.origin = PluginOrigin::Bundled;
        summary.actions[0].action = "next".into();
        summary.actions[0].action_type = "deckpilot.projects.next".into();
        summary.actions[0].name = "Next project".into();
        summary.actions[0].unavailable_reason = reason.map(Into::into);
        let mut definition = definition("deckpilot.projects.next");
        definition.parameters.clear();
        definition.name = "Next project".into();
        summary.actions[0].definition = Some(definition);
        summary
    }

    fn legacy_next() -> ActionTypeId {
        ActionTypeId::new("projects.next").unwrap()
    }

    /// The live defect of 2026-11-21: keys stored with a legacy type of a
    /// removed built-in (`projects.next`) were previewed as unavailable
    /// after the Projects plugin took them over, as the app's registry
    /// never routed the legacy type to the plugin the agent reported.
    #[test]
    fn legacy_types_follow_the_plugin_that_serves_them() {
        // No plugin reported: the core's reason, as before.
        let routed = registry(&PluginActions::default());
        assert!(routed.definition(&legacy_next()).is_none());
        assert!(
            routed
                .unavailable_reason(&legacy_next())
                .is_some_and(|reason| reason.contains("not installed")),
            "{:?}",
            routed.unavailable_reason(&legacy_next())
        );

        // Enabled and consented: the legacy id runs as the plugin's action.
        let mut actions = PluginActions::default();
        actions.replace(&[projects_plugin(true, None)]);
        let routed = registry(&actions);
        let definition = routed.definition(&legacy_next()).expect("routed");
        assert!(definition.deprecated, "not offered for new actions");
        assert_eq!(definition.name, "Next project");
        assert_eq!(
            routed.module_of(&legacy_next()).map(ModuleId::as_str),
            Some("deckpilot.projects")
        );
        assert!(routed.unavailable_reason(&legacy_next()).is_none());

        // A permission not granted: unavailable with the plugin's reason.
        let mut actions = PluginActions::default();
        actions.replace(&[projects_plugin(
            false,
            Some("the permission `projectContext:select` is not granted"),
        )]);
        let routed = registry(&actions);
        assert!(routed.definition(&legacy_next()).is_none());
        assert_eq!(
            routed.unavailable_reason(&legacy_next()),
            Some(
                "Projects (deckpilot.projects): the permission `projectContext:select` is not granted"
            )
        );

        // Uninstalled, then installed again: back without any change to
        // what is stored.
        actions.remove("deckpilot.projects");
        assert!(registry(&actions).definition(&legacy_next()).is_none());
        actions.upsert(&projects_plugin(true, None));
        assert!(registry(&actions).definition(&legacy_next()).is_some());
    }

    /// The editor's preview of a key bound to a legacy type: unavailable
    /// (the compact missing-plugin face, its title kept) without the
    /// plugin, its own face again once the plugin serves it.
    #[test]
    fn previews_of_legacy_keys_follow_the_plugin() {
        use deckpilot_core::device_runtime::page_faces;
        use deckpilot_core::{BindingTarget, ControlBinding, Profile, Trigger};
        use deckpilot_device::ControlId;
        let model = deckpilot_device::models::stream_deck_mk2();
        let mut profile = Profile::new("Work");
        let mut layout = deckpilot_core::DeviceLayout::new("Main", model.id.clone());
        let instance = deckpilot_core::ActionInstance {
            id: deckpilot_core::ActionInstanceId::new(),
            name: "Next".into(),
            action_type: legacy_next(),
            parameters: ActionParameters::new(),
            description: String::new(),
            icon: None,
            label: None,
        };
        let id = instance.id;
        profile.actions.push(instance);
        let home = layout.home_page;
        layout
            .page_mut(home)
            .unwrap()
            .bindings
            .push(ControlBinding {
                label: Some("Now".into()),
                ..ControlBinding::new(
                    ControlId::Key { index: 0 },
                    Trigger::Press,
                    BindingTarget::action(id),
                )
            });
        let face = |actions: &PluginActions| {
            page_faces(
                &registry(actions),
                &layout,
                home,
                &profile,
                None,
                &[ControlId::Key { index: 0 }],
                Some(&model.capabilities),
            )
            .remove(0)
            .1
        };
        let missing = face(&PluginActions::default());
        assert!(missing.unavailable);
        assert_eq!(missing.title, "Now");
        assert_eq!(missing.detail, None, "no warning text on the key");
        let mut actions = PluginActions::default();
        actions.replace(&[projects_plugin(true, None)]);
        let served = face(&actions);
        assert!(!served.unavailable);
        assert_eq!(served.style, deckpilot_core::FaceStyle::Action);
        assert_eq!(served.title, "Now");
    }

    #[test]
    fn answers_cannot_reach_outside_their_namespace() {
        // A built-in type claimed by a plugin, a plugin id that is not a
        // plugin id, and a type of another plugin: all ignored.
        let mut claims_builtin = plugin("acme.hello-weather", false);
        claims_builtin.actions[0].action_type = "system.cpu".into();
        claims_builtin.actions[0].definition = Some(definition("system.cpu"));
        let mut not_a_plugin = plugin("system", true);
        not_a_plugin.actions[0].action_type = "system.greet".into();
        not_a_plugin.actions[0].definition = Some(definition("system.greet"));
        let mut other = plugin("acme.hello-weather", true);
        other.id = "acme.other".into();
        let mut actions = PluginActions::default();
        actions.replace(&[claims_builtin, not_a_plugin, other]);
        let registry = registry(&actions);
        let next = ActionTypeId::new("system.cpu").unwrap();
        assert!(registry.definition(&next).is_some());
        assert!(registry.unavailable_reason(&next).is_none());
        assert!(
            registry
                .definition(&ActionTypeId::new("system.greet").unwrap())
                .is_none()
        );
        assert!(registry.definition(&greet()).is_none());
    }
}
