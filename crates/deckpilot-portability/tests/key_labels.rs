//! Key labels in exports (`docs/key-text.md`, "Labels"): an export with
//! labels is payload version 4 and keeps them through an import, an export
//! without labels keeps its version and bytes, and an import refuses a
//! label template that would show a secret, a sensitive or a run's value.

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, BindingTarget,
    ControlBinding, DeviceLayout, KeyAppearance, KeyLabel, LabelId, Os, ParameterValue, Profile,
    Trigger,
};
use deckpilot_device::{ControlId, DeviceModelId, ModelCatalog, models};
use deckpilot_portability::import::validate;
use deckpilot_portability::payload::{self, ExportOptions, Payload};
use deckpilot_portability::{
    ImportMode, ImportOptions, PAYLOAD_VERSION, PAYLOAD_VERSION_WITH_LABELS,
    PAYLOAD_VERSION_WITHOUT_LIBRARY, PortabilityError, plan,
};
use deckpilot_storage::crypto::{KdfParams, Password};
use deckpilot_storage::{ConfigStore, SqliteStore};

const OPTIONS: ExportOptions = ExportOptions {
    module_settings: false,
    app_settings: false,
};

fn modules() -> deckpilot_core::ModuleRegistry {
    deckpilot_modules::builtin(Os::Linux)
}

fn password() -> Password {
    let mut bytes = [0u8; 16];
    deckpilot_storage::crypto::random_bytes(&mut bytes).unwrap();
    Password::new(format!("test-export-{:032x}", u128::from_le_bytes(bytes)))
}

/// A profile whose key 0 has `labels` (none: no look at all).
fn store_with(labels: Vec<KeyLabel>) -> (SqliteStore, Profile) {
    let store = SqliteStore::open_in_memory().unwrap();
    let hello = ActionInstance {
        id: ActionInstanceId::new(),
        name: "Hello".into(),
        action_type: ActionTypeId::new("general.log").unwrap(),
        parameters: ActionParameters::new().with("message", ParameterValue::Text("hi".into())),
        description: String::new(),
        icon: None,
        label: None,
    };
    let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
    let home = layout.home_page;
    let mut binding = ControlBinding::new(
        ControlId::Key { index: 0 },
        Trigger::Press,
        BindingTarget::action(hello.id),
    );
    if !labels.is_empty() {
        binding.appearance = Some(KeyAppearance {
            labels,
            ..KeyAppearance::default()
        });
    }
    layout.page_mut(home).unwrap().bindings.push(binding);
    let mut profile = Profile::new("Work");
    profile.actions.push(hello);
    profile.layouts.push(layout.clone());
    profile.activate_layout(layout.id).unwrap();
    store.save_profile(&profile).unwrap();
    let profile = store.load_profile(profile.id).unwrap().unwrap();
    (store, profile)
}

fn exported(store: &SqliteStore, profile: &Profile) -> Payload {
    let password = password();
    let payload =
        payload::export(store, &modules(), Some(&[profile.id]), OPTIONS, 1, "test").unwrap();
    let bytes = payload::seal(&payload, &password, KdfParams::for_tests()).unwrap();
    payload::open(&bytes, &password).unwrap()
}

fn label(text: &str) -> KeyLabel {
    KeyLabel::new(LabelId::new("tag").unwrap(), text)
}

#[test]
fn an_export_with_labels_is_version_4_and_keeps_them() {
    let (store, profile) = store_with(vec![label("{project.key}")]);
    let payload = exported(&store, &profile);
    assert_eq!(payload.version, PAYLOAD_VERSION_WITH_LABELS);
    assert_eq!(PAYLOAD_VERSION_WITH_LABELS, 4);
    validate(&payload, &ModelCatalog::builtin()).unwrap();
    let target = SqliteStore::open_in_memory().unwrap();
    let options = ImportOptions {
        mode: ImportMode::Replace,
        profiles: vec![profile.id],
        ..ImportOptions::default()
    };
    let (changes, _) = plan(&payload, &options, &target, &modules()).unwrap();
    target.apply_changes(&changes).unwrap();
    let imported = target.load_profile(profile.id).unwrap().unwrap();
    assert_eq!(imported, profile);
    let look = imported.layouts[0].pages[0].bindings[0]
        .appearance
        .as_ref()
        .unwrap();
    assert_eq!(look.labels, [label("{project.key}")]);
}

#[test]
fn an_export_without_labels_keeps_its_version_and_bytes() {
    let (store, profile) = store_with(Vec::new());
    let payload = exported(&store, &profile);
    assert_eq!(payload.version, PAYLOAD_VERSION_WITHOUT_LIBRARY);
    let json = serde_json::to_string(&payload).unwrap();
    assert!(!json.contains("\"labels\""), "{json}");
}

#[test]
fn an_import_refuses_labels_that_would_show_what_a_key_never_shows() {
    let (store, profile) = store_with(vec![label("Plain")]);
    let mut payload = exported(&store, &profile);
    // A crafted export: the label shows a run's value.
    let look = payload.profiles[0].layouts[0].pages[0].bindings[0]
        .appearance
        .as_mut()
        .unwrap();
    look.labels[0].text = "{{ steps.fetch.output.token }}".into();
    let refused = validate(&payload, &ModelCatalog::builtin());
    assert!(
        matches!(&refused, Err(PortabilityError::Invalid(message)) if message.contains("never shows")),
        "{refused:?}"
    );
    // Nine labels: over the limit.
    let look = payload.profiles[0].layouts[0].pages[0].bindings[0]
        .appearance
        .as_mut()
        .unwrap();
    look.labels = (0..9)
        .map(|n| KeyLabel::new(LabelId::new(&format!("l{n}")).unwrap(), "x"))
        .collect();
    assert!(validate(&payload, &ModelCatalog::builtin()).is_err());
}

#[test]
fn a_later_version_is_refused() {
    let (store, profile) = store_with(vec![label("x")]);
    let mut payload =
        payload::export(&store, &modules(), Some(&[profile.id]), OPTIONS, 1, "t").unwrap();
    payload.version = PAYLOAD_VERSION + 1;
    let password = password();
    let bytes = payload::seal(&payload, &password, KdfParams::for_tests()).unwrap();
    assert!(matches!(
        payload::open(&bytes, &password),
        Err(PortabilityError::UnsupportedVersion(7))
    ));
}
