//! `contributes.gesturePresets` (plugin API 1.10): accepted by the Rust
//! model and the bundled JSON Schema alike, refused with a precise error
//! for an unknown action, gesture or parameter, a value that does not fit,
//! a release next to a double or long press, and below plugin API 1.10;
//! localized like every other manifest text.

use deckpilot_plugin_manifest::locale::LocaleTag;
use deckpilot_plugin_manifest::{
    ApiLevel, GESTURE_PRESETS_API, IssueCode, Manifest, ManifestError, SCHEMA_JSON,
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

/// The example manifest at plugin API `level` with one preset over its
/// `pr-count` action (a `filter` choice with a default, keys only).
fn with(level: &str, preset: &Value) -> Value {
    let mut value: Value = serde_json::from_str(EXAMPLE).unwrap();
    value["engines"]["pluginApi"] = json!(level);
    value["contributes"]["gesturePresets"] = json!([preset]);
    value
}

fn reviews() -> Value {
    json!({
        "id": "review-counts",
        "name": "Review counts",
        "description": "Yours on a single press, waiting for you on a long press.",
        "category": "GitHub",
        "allowPartial": true,
        "gestures": [
            {
                "gesture": "click",
                "action": "pr-count",
                "parameters": { "filter": { "type": "choice", "value": "author-me" } },
                "label": "Count my pull requests"
            },
            {
                "gesture": "longPress",
                "action": "pr-count",
                "label": "Count reviews for me"
            }
        ]
    })
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

#[test]
fn a_preset_parses_validates_and_round_trips() {
    assert_eq!(GESTURE_PRESETS_API, ApiLevel::new(1, 10));
    let value = with("^1.10", &reviews());
    assert_eq!(schema_errors(&value), Vec::<String>::new());
    let manifest = Manifest::parse_and_validate(&value.to_string()).unwrap();
    let preset = &manifest.contributes.gesture_presets()[0];
    assert_eq!(preset.id, "review-counts");
    assert_eq!(preset.allow_partial, Some(true));
    assert_eq!(preset.gestures.len(), 2);
    assert_eq!(serde_json::to_value(&manifest).unwrap(), value);
    // No presets: nothing changes for older manifests.
    let plain: Value = serde_json::from_str(EXAMPLE).unwrap();
    let manifest = Manifest::parse_and_validate(&plain.to_string()).unwrap();
    assert_eq!(manifest.contributes.gesture_presets().len(), 0);
}

#[test]
fn presets_need_plugin_api_1_10() {
    for level in ["^1.0", "^1.9"] {
        let found = issues(&with(level, &reviews()));
        assert!(
            found
                .iter()
                .any(|(path, code, message)| path == "contributes.gesturePresets"
                    && *code == IssueCode::Conflict
                    && message.contains("needs plugin API 1.10: set engines.pluginApi to `^1.10`")),
            "{level}: {found:?}"
        );
    }
}

/// `preset` changed by `f`, refused at `path` with `code` by the model and
/// (when `schema`) by the schema too.
fn refused(f: impl FnOnce(&mut Value), path: &str, code: IssueCode, schema: bool) {
    let mut preset = reviews();
    f(&mut preset);
    let value = with("^1.10", &preset);
    let found = issues(&value);
    assert!(
        found.iter().any(|(p, c, _)| p == path && *c == code),
        "{path}: {found:?}"
    );
    if schema {
        assert_ne!(
            schema_errors(&value),
            Vec::<String>::new(),
            "{path}: schema"
        );
    }
}

#[test]
fn broken_presets_are_manifest_errors() {
    // An action the plugin does not declare.
    refused(
        |p| p["gestures"][0]["action"] = json!("pr-merge"),
        "contributes.gesturePresets[0].gestures[0].action",
        IssueCode::UnknownReference,
        false,
    );
    // A gesture DeckPilot does not know.
    refused(
        |p| p["gestures"][1]["gesture"] = json!("triplePress"),
        "contributes.gesturePresets[0].gestures[1].gesture",
        IssueCode::InvalidValue,
        true,
    );
    // A parameter the action does not have.
    refused(
        |p| p["gestures"][0]["parameters"]["state"] = json!({"type": "text", "value": "open"}),
        "contributes.gesturePresets[0].gestures[0].parameters.state",
        IssueCode::UnknownReference,
        false,
    );
    // A value outside the choice's options, and of the wrong type.
    refused(
        |p| p["gestures"][0]["parameters"]["filter"]["value"] = json!("everyone"),
        "contributes.gesturePresets[0].gestures[0].parameters.filter",
        IssueCode::InvalidDefault,
        false,
    );
    refused(
        |p| {
            p["gestures"][0]["parameters"]["repository"] = json!({"type": "integer", "value": 3});
        },
        "contributes.gesturePresets[0].gestures[0].parameters.repository",
        IssueCode::InvalidDefault,
        false,
    );
    // The same gesture twice.
    refused(
        |p| p["gestures"][1]["gesture"] = json!("click"),
        "contributes.gesturePresets[0].gestures[1]",
        IssueCode::Duplicate,
        false,
    );
    // A release next to a long press.
    refused(
        |p| {
            p["gestures"]
                .as_array_mut()
                .unwrap()
                .push(json!({"gesture": "release", "action": "pr-count", "label": "Release"}));
        },
        "contributes.gesturePresets[0].gestures",
        IssueCode::Conflict,
        false,
    );
    // A rotation the action (keys only) cannot run from.
    refused(
        |p| p["gestures"][1]["gesture"] = json!("rotateClockwise"),
        "contributes.gesturePresets[0].gestures[1].gesture",
        IssueCode::Conflict,
        false,
    );
    // Texts, ids and counts.
    refused(
        |p| p["id"] = json!("Review Counts"),
        "contributes.gesturePresets[0].id",
        IssueCode::InvalidId,
        true,
    );
    refused(
        |p| p["gestures"][0]["label"] = json!(""),
        "contributes.gesturePresets[0].gestures[0].label",
        IssueCode::Length,
        true,
    );
    refused(
        |p| p["gestures"] = json!([]),
        "contributes.gesturePresets[0].gestures",
        IssueCode::Count,
        true,
    );
    // Presets grant nothing: there is no field for a permission.
    let mut value = with("^1.10", &reviews());
    value["contributes"]["gesturePresets"][0]["permissions"] = json!(["network"]);
    assert!(Manifest::parse_and_validate(&value.to_string()).is_err());
    assert_ne!(schema_errors(&value), Vec::<String>::new());
}

#[test]
fn preset_texts_are_localized_with_the_fallback_chain() {
    let mut value = with("^1.10", &reviews());
    value["locales"] = json!(["en", "pl"]);
    value["contributes"]["gesturePresets"][0]["name"] = json!("t:presets.reviews.name");
    value["contributes"]["gesturePresets"][0]["gestures"][0]["label"] =
        json!("t:presets.reviews.mine");
    value["contributes"]["gesturePresets"][0]["gestures"][1]["label"] =
        json!("t:presets.reviews.waiting");
    let manifest = Manifest::parse_and_validate(&value.to_string()).unwrap();
    let file = |entries: &[(&str, &str)]| {
        let mut nested = serde_json::Map::new();
        for (key, text) in entries {
            nested.insert((*key).to_owned(), json!(text));
        }
        json!({ "presets": { "reviews": nested } })
            .to_string()
            .into_bytes()
    };
    let english = file(&[
        ("name", "Review counts"),
        ("mine", "Count my pull requests"),
        ("waiting", "Count reviews for me"),
    ]);
    // Polish lacks one label: English fills it in.
    let polish = file(&[("name", "Liczniki recenzji"), ("mine", "Policz moje PR")]);
    let catalog = manifest
        .catalog(|path| match path {
            "locales/en.json" => Some(english.clone()),
            "locales/pl.json" => Some(polish.clone()),
            _ => None,
        })
        .unwrap();
    let mut missing = |_: &str| {};
    let polish = manifest.localized(
        &catalog,
        Some(&LocaleTag::parse("pl-PL").unwrap()),
        &mut missing,
    );
    let preset = &polish.contributes.gesture_presets()[0];
    assert_eq!(preset.name, "Liczniki recenzji");
    assert_eq!(preset.gestures[0].label, "Policz moje PR");
    assert_eq!(preset.gestures[1].label, "Count reviews for me");
    let english = manifest.localized(&catalog, None, &mut missing);
    assert_eq!(
        english.contributes.gesture_presets()[0].name,
        "Review counts"
    );
}
