//! T-L1 and T-L2: managed layouts are seeded once, never touch another
//! layout and never adopt one, are regenerated only while unedited, and
//! *Restore defaults* keeps the user's version first.

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, BindingTarget,
    ControlBinding, DeviceLayout, LayoutId, ParameterValue, Trigger,
};
use deckpilot_device::{ControlId, DeviceModelId, models};

use super::Seeded;
use crate::{SqliteStore, StorageError};

const OWNER: &str = "deckpilot.calls";

fn mk2() -> DeviceModelId {
    DeviceModelId::new(models::STREAM_DECK_MK2_ID)
}

/// A generated layout with fixed ids (the generator's are stable), its key
/// 0 running the microphone action with `mode`.
fn generated(
    layout_id: LayoutId,
    action_id: ActionInstanceId,
    mode: &str,
) -> (DeviceLayout, Vec<ActionInstance>) {
    let mut layout = DeviceLayout::new("Calls", mk2());
    layout.id = layout_id;
    let action = ActionInstance {
        id: action_id,
        name: "Microphone".into(),
        action_type: ActionTypeId::new("deckpilot.calls.mic-toggle").unwrap(),
        parameters: ActionParameters::new().with("mode", ParameterValue::Text(mode.into())),
        description: String::new(),
        icon: None,
        label: None,
    };
    let home = layout.home_page;
    layout
        .page_mut(home)
        .unwrap()
        .bindings
        .push(ControlBinding::new(
            ControlId::Key { index: 0 },
            Trigger::Press,
            BindingTarget::action(action.id),
        ));
    (layout, vec![action])
}

fn ids() -> (LayoutId, ActionInstanceId) {
    (LayoutId::new(), ActionInstanceId::new())
}

#[test]
fn seeding_is_idempotent_and_touches_no_other_layout() {
    let store = SqliteStore::open_in_memory().unwrap();
    let mut mine = DeviceLayout::new("Rozmowy", mk2());
    mine.name = "Rozmowy".into();
    store.create_shared_layout(&mine, None).unwrap();
    let before = store.shared_layout(mine.id).unwrap().unwrap();

    let (layout_id, action_id) = ids();
    let (layout, actions) = generated(layout_id, action_id, "toggle");
    assert_eq!(
        store
            .seed_managed_layout(OWNER, &layout, &actions, "1.0.0")
            .unwrap(),
        Seeded::Created
    );
    for _ in 0..3 {
        assert_eq!(
            store
                .seed_managed_layout(OWNER, &layout, &actions, "1.0.0")
                .unwrap(),
            Seeded::Unchanged
        );
    }
    let managed = store.managed_layout(OWNER, &mk2()).unwrap().unwrap();
    assert_eq!(managed.layout, layout);
    assert_eq!(managed.actions, actions);
    assert!(!managed.edited);
    assert_eq!(store.managed_layouts(OWNER).unwrap().len(), 1);
    // The user's layout named like it is the user's, untouched.
    assert_eq!(store.shared_layout(mine.id).unwrap().unwrap(), before);
    assert!(store.managed_layouts("deckpilot.other").unwrap().is_empty());
}

#[test]
fn a_layout_of_the_users_is_never_adopted() {
    let store = SqliteStore::open_in_memory().unwrap();
    let (layout_id, action_id) = ids();
    let (layout, actions) = generated(layout_id, action_id, "toggle");
    // The same id exists already, without being managed.
    let mut plain = DeviceLayout::new("Mine", mk2());
    plain.id = layout_id;
    store.create_shared_layout(&plain, None).unwrap();
    let result = store.seed_managed_layout(OWNER, &layout, &actions, "1.0.0");
    assert!(
        matches!(result, Err(StorageError::Invalid(_))),
        "{result:?}"
    );
    assert!(store.managed_layout(OWNER, &mk2()).unwrap().is_none());
}

#[test]
fn a_template_update_regenerates_only_an_unedited_layout() {
    let store = SqliteStore::open_in_memory().unwrap();
    let (layout_id, action_id) = ids();
    let (layout, actions) = generated(layout_id, action_id, "toggle");
    store
        .seed_managed_layout(OWNER, &layout, &actions, "1.0.0")
        .unwrap();
    // Unedited: the new template version is written in place.
    let (newer, newer_actions) = generated(layout_id, action_id, "set");
    assert_eq!(
        store
            .seed_managed_layout(OWNER, &newer, &newer_actions, "1.1.0")
            .unwrap(),
        Seeded::Regenerated
    );
    let managed = store.managed_layout(OWNER, &mk2()).unwrap().unwrap();
    assert_eq!(managed.actions, newer_actions);
    assert_eq!(managed.template_version, "1.1.0");
    assert!(!managed.edited);

    // The user edits it: it is never rewritten.
    store
        .rename_shared_layout(layout_id, "Moje Rozmowy")
        .unwrap();
    assert!(store.managed_layout(OWNER, &mk2()).unwrap().unwrap().edited);
    assert_eq!(
        store
            .seed_managed_layout(OWNER, &layout, &actions, "1.2.0")
            .unwrap(),
        Seeded::UpdateAvailable
    );
    assert_eq!(
        store
            .seed_managed_layout(OWNER, &newer, &newer_actions, "1.1.0")
            .unwrap(),
        Seeded::Unchanged
    );
    let managed = store.managed_layout(OWNER, &mk2()).unwrap().unwrap();
    assert_eq!(managed.layout.name, "Moje Rozmowy");
    assert_eq!(managed.template_version, "1.1.0");
}

#[test]
fn restore_defaults_keeps_the_users_version_first() {
    let store = SqliteStore::open_in_memory().unwrap();
    let (layout_id, action_id) = ids();
    let (layout, actions) = generated(layout_id, action_id, "toggle");
    store
        .seed_managed_layout(OWNER, &layout, &actions, "1.0.0")
        .unwrap();
    // Nothing of the user's to keep: no copy.
    assert_eq!(
        store
            .restore_managed_layout(OWNER, &layout, &actions, "1.0.0", "copy")
            .unwrap(),
        None
    );
    store.rename_shared_layout(layout_id, "Edited").unwrap();
    let copy = store
        .restore_managed_layout(OWNER, &layout, &actions, "1.0.0", "Rozmowy (moja wersja)")
        .unwrap()
        .unwrap();
    // The managed layout is the generated one again.
    let managed = store.managed_layout(OWNER, &mk2()).unwrap().unwrap();
    assert_eq!(managed.layout, layout);
    assert!(!managed.edited);
    // The user's version is an ordinary layout of its own, with copies of
    // its actions: regenerating never changes it.
    let kept = store.shared_layout(copy).unwrap().unwrap();
    assert_eq!(kept.layout.name, "Rozmowy (moja wersja)");
    assert!(kept.profiles.is_empty());
    let (kept_actions, _) = store.layout_library(&kept.layout).unwrap();
    assert_eq!(kept_actions.len(), 1);
    assert_ne!(kept_actions[0].id, action_id);
    assert_eq!(kept_actions[0].parameters, actions[0].parameters);
}

#[test]
fn removing_takes_the_unedited_unless_asked_and_imports_never_do() {
    let store = SqliteStore::open_in_memory().unwrap();
    let (layout_id, action_id) = ids();
    let (layout, actions) = generated(layout_id, action_id, "toggle");
    store
        .seed_managed_layout(OWNER, &layout, &actions, "1.0.0")
        .unwrap();
    // Deleted as an ordinary layout: refused.
    assert!(matches!(
        store.delete_shared_layout(layout_id, &[]),
        Err(StorageError::Invalid(_))
    ));
    store.rename_shared_layout(layout_id, "Edited").unwrap();
    assert!(
        store
            .remove_managed_layouts(OWNER, false)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store.remove_managed_layouts(OWNER, true).unwrap(),
        [layout_id]
    );
    assert!(store.managed_layout(OWNER, &mk2()).unwrap().is_none());
    assert!(store.shared_layout(layout_id).unwrap().is_none());
    // Seeded again from scratch afterwards.
    assert_eq!(
        store
            .seed_managed_layout(OWNER, &layout, &actions, "1.0.0")
            .unwrap(),
        Seeded::Created
    );
}

#[test]
fn a_layout_must_bring_the_actions_it_binds() {
    let store = SqliteStore::open_in_memory().unwrap();
    let (layout_id, action_id) = ids();
    let (layout, _) = generated(layout_id, action_id, "toggle");
    assert!(matches!(
        store.seed_managed_layout(OWNER, &layout, &[], "1.0.0"),
        Err(StorageError::Invalid(_))
    ));
}
