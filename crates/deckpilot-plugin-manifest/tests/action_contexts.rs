//! `contributes.actions[].contexts` (plugin API 1.11): accepted by the Rust
//! model and the bundled JSON Schema alike; refused below plugin API 1.11,
//! when empty, unknown or repeated, and when the declaration does not fit
//! the action's controls or state; checked against gesture presets.
//! Without the field nothing changes.

use deckpilot_plugin_manifest::{
    ACTION_CONTEXTS, ACTION_CONTEXTS_API, ApiLevel, IssueCode, Manifest, ManifestError, SCHEMA_JSON,
};
use serde_json::{Value, json};

const EXAMPLE: &str = include_str!("fixtures/example-manifest.json");

fn schema_errors(instance: &Value) -> Vec<String> {
    let schema: Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    let validator = jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .expect("the bundled schema compiles");
    validator
        .iter_errors(instance)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect()
}

/// The example manifest at plugin API `level`, its `pr-count` action
/// (keys only, with a state) declaring `contexts`.
fn with(level: &str, contexts: &Value) -> Value {
    let mut value: Value = serde_json::from_str(EXAMPLE).unwrap();
    value["engines"]["pluginApi"] = json!(level);
    value["contributes"]["actions"][0]["contexts"] = contexts.clone();
    value
}

fn issues(value: &Value) -> Vec<(String, IssueCode, String)> {
    match Manifest::parse_and_validate(&value.to_string()) {
        Ok(_) => Vec::new(),
        Err(ManifestError::Invalid(errors)) => errors
            .0
            .into_iter()
            .map(|i| (i.path, i.code, i.message))
            .collect(),
        Err(other) => vec![(String::new(), IssueCode::InvalidValue, other.to_string())],
    }
}

const PATH: &str = "contributes.actions[0].contexts";

#[test]
fn contexts_parse_validate_and_round_trip() {
    assert_eq!(ACTION_CONTEXTS_API, ApiLevel::new(1, 11));
    assert_eq!(
        ACTION_CONTEXTS,
        ["key", "gesture", "display", "sequence", "workflow"]
    );
    let value = with("^1.11", &json!(ACTION_CONTEXTS));
    assert_eq!(schema_errors(&value), Vec::<String>::new());
    let manifest = Manifest::parse_and_validate(&value.to_string()).unwrap();
    assert_eq!(
        manifest.contributes.actions()[0].contexts.as_deref(),
        Some(&ACTION_CONTEXTS.map(String::from)[..])
    );
    assert_eq!(serde_json::to_value(&manifest).unwrap(), value);
    // Without the field: all five, exactly as before, at any level.
    let plain: Value = serde_json::from_str(EXAMPLE).unwrap();
    let manifest = Manifest::parse_and_validate(&plain.to_string()).unwrap();
    assert_eq!(manifest.contributes.actions()[0].contexts, None);
    let mut at_1_11 = plain.clone();
    at_1_11["engines"]["pluginApi"] = json!("^1.11");
    assert_eq!(issues(&at_1_11), Vec::new());
}

#[test]
fn contexts_need_plugin_api_1_11() {
    for level in ["^1.0", "^1.9", "^1.10"] {
        let found = issues(&with(level, &json!(["sequence", "workflow"])));
        assert!(
            found.iter().any(|(path, code, message)| path == PATH
                && *code == IssueCode::Conflict
                && message.contains("needs plugin API 1.11: set engines.pluginApi to `^1.11`")),
            "{level}: {found:?}"
        );
    }
}

/// `contexts` refused at `path` with `code` by the model and (when
/// `schema`) by the schema too.
fn refused(contexts: &Value, path: &str, code: IssueCode, schema: bool) {
    let value = with("^1.11", contexts);
    let found = issues(&value);
    assert!(
        found.iter().any(|(p, c, _)| p == path && *c == code),
        "{contexts}: expected {code:?} at {path}, got {found:?}"
    );
    if schema {
        assert!(
            !schema_errors(&value).is_empty(),
            "{contexts}: the schema should refuse it too"
        );
    }
}

#[test]
fn a_declaration_is_non_empty_known_and_unique() {
    refused(&json!([]), PATH, IssueCode::Count, true);
    refused(
        &json!(["key", "everywhere"]),
        &format!("{PATH}[1]"),
        IssueCode::InvalidValue,
        true,
    );
    refused(
        &json!(["key", "key"]),
        &format!("{PATH}[1]"),
        IssueCode::Duplicate,
        true,
    );
    refused(
        &json!(["Key"]),
        &format!("{PATH}[0]"),
        IssueCode::InvalidValue,
        true,
    );
}

#[test]
fn a_declaration_fits_the_controls_and_the_state() {
    // Keys with a press and a click only: no gesture there.
    let mut value = with("^1.11", &json!(["key", "gesture"]));
    value["contributes"]["actions"][0]["controls"] = json!({ "key": ["press", "click"] });
    let found = issues(&value);
    assert!(
        found.iter().any(|(path, code, message)| path == PATH
            && *code == IssueCode::Conflict
            && message.contains("`gesture` needs a double click or long press")),
        "{found:?}"
    );
    // Only a long press: no primary input for `key`.
    value["contributes"]["actions"][0]["controls"] = json!({ "key": ["long-press"] });
    let found = issues(&value);
    assert!(
        found.iter().any(|(path, _, message)| path == PATH
            && message.contains("`key` needs a press, release, click, rotation or touch")),
        "{found:?}"
    );
    // Without controls (keys with every key input) both fit.
    assert_eq!(
        issues(&with("^1.11", &json!(["key", "gesture"]))),
        Vec::new()
    );
    // `display` needs a live face: the action's state.
    assert_eq!(issues(&with("^1.11", &json!(["display"]))), Vec::new());
    let mut stateless = with("^1.11", &json!(["display"]));
    stateless["contributes"]["actions"][0]
        .as_object_mut()
        .unwrap()
        .remove("state");
    let found = issues(&stateless);
    assert!(
        found
            .iter()
            .any(|(path, _, message)| path == PATH && message.contains("`display` needs")),
        "{found:?}"
    );
}

#[test]
fn a_gesture_preset_uses_only_contexts_its_action_declares() {
    let mut value = with("^1.11", &json!(["key"]));
    value["contributes"]["gesturePresets"] = json!([{
        "id": "counts",
        "name": "Counts",
        "category": "GitHub",
        "gestures": [
            { "gesture": "click", "action": "pr-count", "label": "Count" },
            { "gesture": "longPress", "action": "pr-count", "label": "Count again" }
        ]
    }]);
    let found = issues(&value);
    assert_eq!(
        found
            .iter()
            .map(|(path, code, _)| (path.as_str(), *code))
            .collect::<Vec<_>>(),
        [(
            "contributes.gesturePresets[0].gestures[1].gesture",
            IssueCode::Conflict
        )],
        "{found:?}"
    );
    assert!(
        found[0]
            .2
            .contains("does not declare the `gesture` context")
    );
    // With `gesture` declared too it is accepted.
    value["contributes"]["actions"][0]["contexts"] = json!(["key", "gesture"]);
    assert_eq!(issues(&value), Vec::new());
}
