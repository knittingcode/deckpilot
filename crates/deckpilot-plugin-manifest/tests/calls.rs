//! T-P2: plugin API 1.16's Calls kinds (`docs/calls.md`): the `calls`
//! permission, `contributes.callOverlay` and `contributes.layoutTemplates`
//! are accepted only for the bundled plugin `deckpilot.calls` with
//! `^1.16`, refused for every other manifest, and described the same way by
//! the Rust model and the bundled JSON Schema.

use deckpilot_plugin_manifest::{
    ApiLevel, CALLS_API, CallPage, IssueCode, Manifest, PermissionKind, SCHEMA_JSON,
    TemplateControlKind, TemplateShape,
};
use serde_json::{Value, json};

const CALLS: &str = include_str!("fixtures/calls-manifest.json");

fn schema_ok(value: &Value) -> bool {
    let schema: Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap()
        .is_valid(value)
}

fn calls() -> Value {
    serde_json::from_str(CALLS).unwrap()
}

fn parse(value: &Value) -> Manifest {
    Manifest::from_json(&value.to_string()).unwrap()
}

/// The issues `validate_bundled` reports for `value`, as `(path, code)`.
fn bundled_issues(value: &Value) -> Vec<(String, IssueCode)> {
    match parse(value).validate_bundled() {
        Ok(()) => Vec::new(),
        Err(errors) => errors.0.into_iter().map(|i| (i.path, i.code)).collect(),
    }
}

#[track_caller]
fn refused(edit: impl FnOnce(&mut Value), path: &str, code: IssueCode) {
    let mut value = calls();
    edit(&mut value);
    let issues = bundled_issues(&value);
    assert!(
        issues.iter().any(|(p, c)| p == path && *c == code),
        "expected {code:?} at {path}, got {issues:?}"
    );
}

#[test]
fn the_bundled_calls_plugin_of_1_16_is_accepted() {
    assert_eq!(CALLS_API, ApiLevel::new(1, 16));
    let value = calls();
    assert!(schema_ok(&value));
    let manifest = parse(&value);
    manifest.validate_bundled().unwrap();
    assert!(PermissionKind::Calls.is_first_party_only());
    assert_eq!(PermissionKind::Calls.as_str(), "calls");
    let overlay = manifest.contributes.call_overlay.as_ref().unwrap();
    assert_eq!(overlay.pages, [CallPage::Incoming, CallPage::Active]);
    let templates = manifest.contributes.layout_templates();
    assert_eq!(templates[0].shape, TemplateShape::Grid);
    assert_eq!(
        templates[1].pages[0].controls[0].control,
        TemplateControlKind::Dial
    );
    // `calls:control` is optional: the plugin is enabled with the state
    // only, and the actions naming it wait for it.
    assert_eq!(
        manifest
            .optional_permissions()
            .into_iter()
            .collect::<Vec<_>>(),
        ["calls:control"]
    );
    // It round-trips, and a manifest without the kinds never writes them.
    let back = parse(&serde_json::from_str(&manifest.to_json_pretty()).unwrap());
    assert_eq!(back, manifest);
}

#[test]
fn every_calls_kind_is_a_privileged_usage() {
    let usages = parse(&calls()).privileged_usages();
    let paths: Vec<&str> = usages.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        paths,
        [
            "permissions[0]",
            "contributes.callOverlay",
            "contributes.layoutTemplates"
        ]
    );
}

#[test]
fn outside_the_bundle_every_calls_kind_is_refused() {
    let manifest = parse(&calls());
    let errors = manifest.validate_first_party().unwrap_err().0;
    let first_party_only: Vec<&str> = errors
        .iter()
        .filter(|i| i.code == IssueCode::FirstPartyOnly)
        .map(|i| i.path.as_str())
        .collect();
    assert_eq!(
        first_party_only,
        [
            "permissions[0]",
            "contributes.callOverlay",
            "contributes.layoutTemplates"
        ]
    );
    // A community plugin is refused by the plain rules too.
    let mut community = calls();
    community["id"] = json!("acme.calls");
    community["publisher"] = json!({"id": "acme", "name": "Acme"});
    let errors = parse(&community).validate().unwrap_err().0;
    assert!(errors.iter().any(|i| i.code == IssueCode::FirstPartyOnly));
}

#[test]
fn only_the_plugin_deckpilot_calls_declares_them() {
    refused(
        |v| v["id"] = json!("deckpilot.meetings"),
        "contributes.callOverlay",
        IssueCode::Conflict,
    );
    refused(
        |v| v["id"] = json!("deckpilot.meetings"),
        "permissions[0]",
        IssueCode::Conflict,
    );
}

#[test]
fn each_kind_needs_plugin_api_1_16() {
    for level in ["^1.9", "^1.15"] {
        let mut value = calls();
        value["engines"]["pluginApi"] = json!(level);
        let issues = bundled_issues(&value);
        for path in [
            "permissions[0]",
            "contributes.callOverlay",
            "contributes.layoutTemplates",
        ] {
            assert!(
                issues
                    .iter()
                    .any(|(p, c)| p == path && *c == IssueCode::Conflict),
                "{level} {path}: {issues:?}"
            );
        }
    }
}

#[test]
fn the_overlay_and_templates_need_the_calls_permission() {
    refused(
        |v| {
            v["permissions"] = json!([]);
            for action in v["contributes"]["actions"].as_array_mut().unwrap() {
                action
                    .as_object_mut()
                    .unwrap()
                    .remove("requiresPermissions");
            }
        },
        "contributes.callOverlay",
        IssueCode::MissingPermission,
    );
    refused(
        |v| {
            v["contributes"]
                .as_object_mut()
                .unwrap()
                .remove("callOverlay");
        },
        "contributes.layoutTemplates",
        IssueCode::MissingPermission,
    );
}

#[test]
fn calls_control_is_optional_only_when_declared() {
    // `control` absent: naming `calls:control` is refused.
    refused(
        |v| {
            v["permissions"][0]
                .as_object_mut()
                .unwrap()
                .remove("control");
        },
        "contributes.actions[0].requiresPermissions[0]",
        IssueCode::MissingPermission,
    );
}

#[test]
fn a_calls_plugin_has_no_way_out_but_the_host() {
    for (kind, permission) in [
        (
            "network",
            json!({"kind": "network", "hosts": ["graph.microsoft.com"], "justification": "x"}),
        ),
        (
            "openUrl",
            json!({"kind": "openUrl", "hosts": ["teams.microsoft.com"], "justification": "x"}),
        ),
        (
            "credentials",
            json!({"kind": "credentials", "refs": ["token"], "justification": "x"}),
        ),
        (
            "tools",
            json!({"kind": "tools", "tools": [{"executable": "git",
                   "subcommands": [{"name": "status"}]}], "justification": "x"}),
        ),
        (
            "connections",
            json!({"kind": "connections", "types": ["github"], "justification": "x"}),
        ),
        (
            "resources",
            json!({"kind": "resources", "open": true, "justification": "x"}),
        ),
        ("scripts", json!({"kind": "scripts", "justification": "x"})),
    ] {
        let mut value = calls();
        value["permissions"]
            .as_array_mut()
            .unwrap()
            .push(permission);
        let issues = bundled_issues(&value);
        assert!(
            issues
                .iter()
                .any(|(p, c)| p == "permissions[1]" && *c == IssueCode::Conflict),
            "{kind}: {issues:?}"
        );
    }
}

/// Its own variables could carry what it learns of a call into other
/// actions' parameters: a Calls plugin declares none.
#[test]
fn a_calls_plugin_declares_no_variables() {
    refused(
        |v| {
            v["contributes"]["variables"] = json!([{
                "id": "state",
                "name": "Call state",
                "valueType": {"type": "string"},
                "scope": "machine"
            }]);
        },
        "contributes.variables",
        IssueCode::Conflict,
    );
}

#[test]
fn overlay_pages_are_one_to_three_unique() {
    refused(
        |v| v["contributes"]["callOverlay"]["pages"] = json!([]),
        "contributes.callOverlay.pages",
        IssueCode::Count,
    );
    let mut twice = calls();
    twice["contributes"]["callOverlay"]["pages"] = json!(["active", "active", "incoming"]);
    assert!(
        bundled_issues(&twice)
            .iter()
            .any(|(p, c)| p == "contributes.callOverlay.pages[1]" && *c == IssueCode::Duplicate)
    );
    let mut unknown = calls();
    unknown["contributes"]["callOverlay"]["pages"] = json!(["ringing"]);
    assert!(Manifest::from_json(&unknown.to_string()).is_err());
    assert!(!schema_ok(&unknown));
}

#[test]
fn a_template_places_own_actions_on_controls_its_shape_has() {
    let page = "contributes.layoutTemplates[0].pages[1].controls";
    // Key 15 does not exist on a 15-key grid.
    refused(
        |v| v["contributes"]["layoutTemplates"][0]["pages"][1]["controls"][1]["index"] = json!(15),
        &format!("{page}[1].index"),
        IssueCode::Range,
    );
    // A grid has no dials.
    refused(
        |v| {
            v["contributes"]["layoutTemplates"][0]["pages"][1]["controls"][0]["control"] =
                json!("dial");
        },
        &format!("{page}[0].index"),
        IssueCode::Range,
    );
    // Only the plugin's own actions.
    refused(
        |v| {
            v["contributes"]["layoutTemplates"][0]["pages"][1]["controls"][0]["action"] =
                json!("open-teams");
        },
        &format!("{page}[0].action"),
        IssueCode::UnknownReference,
    );
    // Parameters the action declares, with values that fit.
    refused(
        |v| {
            v["contributes"]["layoutTemplates"][0]["pages"][1]["controls"][1]["parameters"] =
                json!({"delay": {"type": "integer", "value": 30}});
        },
        &format!("{page}[1].parameters.delay"),
        IssueCode::UnknownReference,
    );
    // One action per control.
    refused(
        |v| v["contributes"]["layoutTemplates"][0]["pages"][1]["controls"][1]["index"] = json!(0),
        &format!("{page}[1]"),
        IssueCode::Duplicate,
    );
}

#[test]
fn templates_are_one_per_shape_with_overlay_pages() {
    refused(
        |v| v["contributes"]["layoutTemplates"][1]["shape"] = json!("grid"),
        "contributes.layoutTemplates[1]",
        IssueCode::Duplicate,
    );
    refused(
        |v| v["contributes"]["layoutTemplates"][0]["pages"][0]["page"] = json!("outgoing"),
        "contributes.layoutTemplates[0].pages[0].page",
        IssueCode::UnknownReference,
    );
    refused(
        |v| v["contributes"]["layoutTemplates"][0]["version"] = json!(0),
        "contributes.layoutTemplates[0].version",
        IssueCode::Range,
    );
    let mut shape = calls();
    shape["contributes"]["layoutTemplates"][0]["shape"] = json!("mk2");
    assert!(Manifest::from_json(&shape.to_string()).is_err());
    assert!(!schema_ok(&shape));
}

#[test]
fn the_model_and_the_schema_refuse_unknown_fields_alike() {
    for edit in [
        json!({"kind": "calls", "justification": "x", "providers": ["teams"]}),
        json!({"kind": "calls", "justification": "x", "control": "yes"}),
    ] {
        let mut value = calls();
        value["permissions"][0] = edit.clone();
        assert!(Manifest::from_json(&value.to_string()).is_err(), "{edit}");
        assert!(!schema_ok(&value), "{edit}");
    }
    let mut value = calls();
    value["contributes"]["callOverlay"]["priority"] = json!(1000);
    assert!(Manifest::from_json(&value.to_string()).is_err());
    assert!(!schema_ok(&value));
}
