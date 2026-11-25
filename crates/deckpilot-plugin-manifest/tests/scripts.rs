//! Plugin API 1.13: the Scripts plugin's kinds (`docs/scripts-bridge.md`).
//! The `scripts` permission, the `script` parameter kind, the `javascript`
//! language, the `scripts` view, and the runtime constructs
//! `runtime.limits` and `runtime.hostCalls`: each at `^1.13`; all but the
//! language privileged; the permission and the runtime constructs for
//! `deckpilot.scripts` only.

use deckpilot_plugin_manifest::{
    CodeLanguage, IssueCode, Manifest, ParameterKind, Permission, SCHEMA_JSON, SCRIPTS_API,
    ViewKind,
};
use serde_json::{Value, json};

/// The Scripts plugin's manifest as `knittingcode/deckpilot-plugins`
/// `official/scripts` declares it (P5), but for the `check` action's
/// `"contexts": []`, which plugin API 1.11 refuses (an empty list): the
/// guest leaves the field out instead (a follow-up in the plugins
/// repository).
const SCRIPTS: &str = include_str!("fixtures/scripts-manifest.json");

fn scripts() -> Value {
    serde_json::from_str(SCRIPTS).unwrap()
}

fn schema_ok(value: &Value) -> bool {
    let schema: Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap()
        .is_valid(value)
}

fn parse(value: &Value) -> Manifest {
    Manifest::from_json(&value.to_string()).unwrap()
}

#[test]
fn the_scripts_plugin_s_manifest_validates_as_bundled() {
    assert_eq!(SCRIPTS_API.to_string(), "1.13");
    let value = scripts();
    assert!(schema_ok(&value));
    let manifest = parse(&value);
    manifest.validate_bundled().unwrap();
    let runtime = manifest.runtime.as_ref().unwrap();
    assert_eq!(runtime.limits.unwrap().continuous_ms, 2000);
    assert_eq!(runtime.host_calls, Some(4096));
    assert!(manifest.permissions.iter().any(|p| matches!(
        p,
        Permission::Scripts {
            fs: Some(true),
            process: Some(true),
            shell: Some(true),
            ..
        }
    )));
    let run = &manifest.contributes.actions()[0];
    assert_eq!(run.parameters[0].kind, ParameterKind::Script {});
    let check = &manifest.contributes.actions()[1];
    assert!(matches!(
        check.parameters[0].kind,
        ParameterKind::Code {
            language: CodeLanguage::Javascript,
            ..
        }
    ));
    assert_eq!(manifest.contributes.views()[0].kind, ViewKind::Scripts);
    // It round-trips as written.
    let back = serde_json::to_value(&manifest).unwrap();
    assert_eq!(back["runtime"], value["runtime"]);
    assert_eq!(back["permissions"][0], value["permissions"][0]);
}

#[test]
fn every_kind_needs_plugin_api_1_13() {
    let mut value = scripts();
    value["engines"]["pluginApi"] = json!("^1.12");
    let errors = parse(&value).validate_bundled().unwrap_err();
    for path in [
        "permissions[0]",
        "contributes.actions[0].parameters[0].kind",
        "contributes.actions[1].parameters[0].kind",
        "contributes.views[0].kind",
        "runtime.limits",
        "runtime.hostCalls",
    ] {
        assert!(errors.has(path, IssueCode::Conflict), "{path}: {errors}");
    }
    assert!(errors.to_string().contains("^1.13"), "{errors}");
}

#[test]
fn only_the_scripts_plugin_declares_its_permission_and_runtime() {
    let mut value = scripts();
    value["id"] = json!("deckpilot.other");
    let errors = parse(&value).validate_bundled().unwrap_err();
    for path in ["permissions[0]", "runtime.limits", "runtime.hostCalls"] {
        assert!(
            errors.has(path, IssueCode::InvalidValue),
            "{path}: {errors}"
        );
    }
}

#[test]
fn they_are_first_party_only() {
    let errors = parse(&scripts()).validate().unwrap_err();
    for path in [
        "permissions[0]",
        "contributes.actions[0].parameters[0].kind",
        "contributes.views[0].kind",
        "runtime.limits",
        "runtime.hostCalls",
    ] {
        assert!(
            errors.has(path, IssueCode::FirstPartyOnly),
            "{path}: {errors}"
        );
    }
}

#[test]
fn bounds_and_rules() {
    let refused = |edit: &dyn Fn(&mut Value), path: &str, code: IssueCode| {
        let mut value = scripts();
        edit(&mut value);
        let errors = parse(&value).validate_bundled().unwrap_err();
        assert!(errors.has(path, code), "{path}: {errors}");
    };
    refused(
        &|v| v["runtime"]["limits"]["continuousMs"] = json!(10_001),
        "runtime.limits.continuousMs",
        IssueCode::InvalidValue,
    );
    refused(
        &|v| v["runtime"]["limits"]["continuousMs"] = json!(100),
        "runtime.limits.continuousMs",
        IssueCode::InvalidValue,
    );
    refused(
        &|v| v["runtime"]["hostCalls"] = json!(4097),
        "runtime.hostCalls",
        IssueCode::InvalidValue,
    );
    refused(
        &|v| {
            v["permissions"][0] =
                json!({"kind": "scripts", "justification": "Runs your scripts on this computer."});
        },
        "permissions[0]",
        IssueCode::InvalidValue,
    );
    // No exfiltration channel beside it.
    refused(
        &|v| {
            v["permissions"].as_array_mut().unwrap().push(json!({
                "kind": "network", "hosts": ["api.example.com"],
                "justification": "Talks to the example API."
            }));
        },
        "permissions[2]",
        IssueCode::Conflict,
    );
    // A `script` parameter needs the permission.
    refused(
        &|v| {
            v["permissions"].as_array_mut().unwrap().remove(0);
        },
        "contributes.actions[0].parameters[0].kind",
        IssueCode::MissingPermission,
    );
    // Unknown members are refused by the model.
    let mut value = scripts();
    value["runtime"]["limits"]["fuel"] = json!(1);
    assert!(Manifest::from_json(&value.to_string()).is_err());
    assert!(!schema_ok(&value));
}

#[test]
fn a_manifest_without_them_validates_as_before() {
    let example: Value =
        serde_json::from_str(include_str!("fixtures/example-manifest.json")).unwrap();
    assert!(schema_ok(&example));
    parse(&example).validate().unwrap();
}
