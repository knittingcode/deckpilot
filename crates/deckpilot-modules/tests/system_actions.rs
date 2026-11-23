//! Website, Open, Open Application and Close Application in the registry
//! the agent and the app build (`docs/system-actions.md`): their contexts,
//! their capabilities, and "Quit on long press" as an ordinary gesture
//! binding of the existing gesture model.

use deckpilot_core::{ActionContext, Capability, Os, Trigger};
use deckpilot_device::{ControlId, models};
use deckpilot_modules::{builtin, system};

fn definition(id: &str) -> deckpilot_core::ActionType {
    let registry = builtin(Os::current());
    registry
        .action_registry()
        .action_types()
        .find(|t| t.id.as_str() == id)
        .unwrap_or_else(|| panic!("{id} is registered"))
        .clone()
}

#[test]
fn the_four_actions_are_registered_once_each() {
    let registry = builtin(Os::current());
    for id in [
        system::actions::OPEN_URL,
        system::actions::OPEN_FILE,
        system::actions::OPEN_APP,
        system::actions::CLOSE_APP,
    ] {
        let count = registry
            .action_registry()
            .action_types()
            .filter(|t| t.id.as_str() == id)
            .count();
        assert_eq!(count, 1, "{id}");
        let found = definition(id);
        assert!(!found.deprecated, "{id}");
        assert_eq!(
            found
                .contexts
                .as_ref()
                .map(deckpilot_core::ActionContexts::to_vec),
            Some(vec![
                ActionContext::Key,
                ActionContext::Gesture,
                ActionContext::Sequence,
                ActionContext::Workflow,
            ]),
            "{id}"
        );
    }
    assert_eq!(definition(system::actions::OPEN_URL).name, "Website");
    assert_eq!(definition(system::actions::OPEN_FILE).name, "Open");
    assert_eq!(
        definition(system::actions::OPEN_APP).name,
        "Open Application"
    );
    assert_eq!(
        definition(system::actions::CLOSE_APP).name,
        "Close Application"
    );
}

#[test]
fn quit_on_long_press_is_an_ordinary_gesture_binding() {
    let key = models::stream_deck_mk2()
        .capabilities
        .control_capabilities(ControlId::Key { index: 0 })
        .unwrap();
    let open = definition(system::actions::OPEN_APP);
    let close = definition(system::actions::CLOSE_APP);
    for (action, trigger) in [
        (&open, Trigger::Press),
        (&open, Trigger::Click),
        (&close, Trigger::LongPress),
        (&close, Trigger::DoubleClick),
    ] {
        assert!(
            action.compatibility(&key, trigger).is_compatible(),
            "{} on {trigger:?}",
            action.id
        );
        assert!(
            action
                .context_compatibility(ActionContext::of_trigger(trigger))
                .is_compatible(),
            "{} on {trigger:?}",
            action.id
        );
    }
    // The long-press action takes the same application parameter, so the
    // editor gives it the key's application as it is.
    let parameter = |action: &deckpilot_core::ActionType| {
        action
            .parameters
            .iter()
            .find(|p| p.key == "application")
            .cloned()
            .unwrap()
    };
    assert_eq!(parameter(&open), parameter(&close));
}

#[test]
fn closing_is_a_capability_of_the_system_module_only() {
    let registry = builtin(Os::current());
    for info in registry.modules() {
        let declares = info
            .manifest
            .capabilities
            .contains(&Capability::ProcessControl);
        assert_eq!(
            declares,
            info.manifest.id.as_str() == system::MODULE_ID,
            "{}",
            info.manifest.id
        );
    }
}
