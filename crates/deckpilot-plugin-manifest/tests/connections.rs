//! Plugin API 1.14, connections: the `connections` permission,
//! `openUrl.connections`, `contributes.connectionTypes` and
//! `contributes.recordSchemas`, `connection` parameters, parameter
//! `options`, internal actions and `records` outputs. Accepted only for a
//! first-party plugin shipped inside the app (`Manifest::validate_bundled`)
//! with `^1.14`, refused for every other manifest, and described the same
//! way by the Rust model and the bundled JSON Schema.

use deckpilot_plugin_manifest::{
    ApiLevel, CONNECTIONS_API, ConnectionAuthMethod, CredentialScheme, HttpMethod, IssueCode,
    Manifest, OutputKind, ParameterKind, Permission, PermissionKind, RecordFieldKind, SCHEMA_JSON,
};
use serde_json::{Value, json};

const FIXTURE: &str = include_str!("fixtures/connections-manifest.json");

fn schema() -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap()
}

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).unwrap()
}

fn parse(value: &Value) -> Manifest {
    Manifest::from_json(&value.to_string()).unwrap_or_else(|e| panic!("{e}"))
}

/// The bundled rules' issues at `path` (or below it).
fn issues_at(value: &Value, path: &str) -> Vec<(IssueCode, String)> {
    match parse(value).validate_bundled() {
        Ok(()) => Vec::new(),
        Err(errors) => errors
            .0
            .into_iter()
            .filter(|i| i.path.starts_with(path))
            .map(|i| (i.code, i.message))
            .collect(),
    }
}

fn refused(value: &Value, path: &str, code: IssueCode) {
    let found = issues_at(value, path);
    assert!(
        found.iter().any(|(c, _)| *c == code),
        "expected {code:?} at {path}, got {found:?}"
    );
}

#[test]
fn a_bundled_first_party_plugin_of_1_14_may_declare_connections() {
    assert_eq!(CONNECTIONS_API, ApiLevel::new(1, 14));
    let value = fixture();
    let errors: Vec<_> = schema()
        .iter_errors(&value)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "schema: {errors:?}");
    let manifest = parse(&value);
    manifest.validate_bundled().unwrap();
    assert_eq!(serde_json::to_value(&manifest).unwrap(), value);
}

#[test]
fn the_model_reads_connections() {
    let manifest = parse(&fixture());
    let Some(permission) = manifest.permission(PermissionKind::Connections) else {
        panic!("no connections permission");
    };
    assert!(PermissionKind::Connections.is_first_party_only());
    assert_eq!(PermissionKind::Connections.as_str(), "connections");
    assert_eq!(
        permission.connection_methods(),
        [HttpMethod::Get, HttpMethod::Post, HttpMethod::Put]
    );
    assert!(
        manifest
            .permissions
            .iter()
            .any(Permission::opens_connections)
    );
    let types = manifest.contributes.connection_types();
    assert_eq!(types[0].id, "github");
    assert_eq!(types[1].auth[0].scheme, CredentialScheme::Header);
    assert_eq!(types[1].auth[0].method, ConnectionAuthMethod::Pat);
    assert_eq!(types[0].extensions().len(), 3);
    assert_eq!(
        manifest.contributes.record_schemas()[1].fields[1].kind,
        RecordFieldKind::State
    );
    assert!(manifest.contributes.record_schemas()[1].fields[3].sensitive());
    let actions = manifest.contributes.actions();
    assert!(actions[0].is_internal());
    assert!(!actions[1].is_internal());
    assert!(matches!(
        &actions[1].parameters[0].kind,
        ParameterKind::Connection { capability: Some(c), .. } if c == "pipelines"
    ));
    let options = actions[1].parameters[1].options.as_ref().unwrap();
    assert_eq!(options.depends_on(), ["connection"]);
    assert!(options.search());
    let outputs = actions[0].node.as_ref().unwrap().outputs.as_ref().unwrap();
    assert_eq!(outputs[0].kind, Some(OutputKind::Records));
    // Absent methods mean GET only.
    let mut value = fixture();
    value["permissions"][0]
        .as_object_mut()
        .unwrap()
        .remove("methods");
    let manifest = parse(&value);
    assert_eq!(
        manifest
            .permission(PermissionKind::Connections)
            .unwrap()
            .connection_methods(),
        [HttpMethod::Get]
    );
}

#[test]
fn connection_constructs_are_privileged_usages_and_the_rest_first_party_only() {
    let manifest = parse(&fixture());
    let usages: Vec<String> = manifest
        .privileged_usages()
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    assert_eq!(
        usages,
        [
            "permissions[0]",
            "permissions[1].connections",
            "contributes.connectionTypes",
            "contributes.actions[0].parameters[0].kind",
            "contributes.actions[1].parameters[0].kind",
        ]
    );
    // Neither a registry package nor a first-party one from a registry.
    for errors in [
        manifest.validate().unwrap_err(),
        manifest.validate_first_party().unwrap_err(),
    ] {
        let gated: Vec<&str> = errors
            .0
            .iter()
            .filter(|i| i.code == IssueCode::FirstPartyOnly)
            .map(|i| i.path.as_str())
            .collect();
        for path in usages.iter().map(String::as_str).chain([
            "contributes.recordSchemas",
            "contributes.actions[0].internal",
            "contributes.actions[0].node.outputs[0]",
            "contributes.actions[1].parameters[1].options",
            "contributes.actions[1].node.outputs[1]",
        ]) {
            assert!(gated.contains(&path), "{path} not gated: {gated:?}");
        }
    }
}

#[test]
fn every_construct_needs_plugin_api_1_14() {
    let mut value = fixture();
    value["engines"]["pluginApi"] = json!("^1.13");
    let errors = parse(&value).validate_bundled().unwrap_err();
    let gated: Vec<&str> = errors
        .0
        .iter()
        .filter(|i| i.code == IssueCode::Conflict && i.message.contains("plugin API 1.14"))
        .map(|i| i.path.as_str())
        .collect();
    for path in [
        "permissions[0]",
        "permissions[1].connections",
        "contributes.connectionTypes",
        "contributes.recordSchemas",
        "contributes.actions[0].internal",
        "contributes.actions[0].parameters[0].kind",
        "contributes.actions[1].parameters[1].options",
        "contributes.actions[1].node.outputs[1]",
    ] {
        assert!(gated.contains(&path), "{path}: {gated:?}");
    }
}

#[test]
fn the_permission_and_the_types_name_each_other() {
    let mut value = fixture();
    value["permissions"][0]["types"] = json!(["github", "bitbucket"]);
    refused(
        &value,
        "permissions[0].types[1]",
        IssueCode::UnknownReference,
    );
    refused(&value, "permissions[0].types", IssueCode::MissingPermission);

    let mut value = fixture();
    value["permissions"].as_array_mut().unwrap().remove(0);
    value["permissions"][0]
        .as_object_mut()
        .unwrap()
        .remove("connections");
    value["permissions"][0]["hosts"] = json!(["github.com"]);
    refused(
        &value,
        "contributes.connectionTypes",
        IssueCode::MissingPermission,
    );

    let mut value = fixture();
    value["contributes"]
        .as_object_mut()
        .unwrap()
        .remove("connectionTypes");
    refused(&value, "permissions[0].types", IssueCode::UnknownReference);

    // `openUrl.connections` without the permission.
    let mut value = fixture();
    value["permissions"].as_array_mut().unwrap().remove(0);
    value["contributes"]
        .as_object_mut()
        .unwrap()
        .remove("connectionTypes");
    refused(
        &value,
        "permissions[0].connections",
        IssueCode::MissingPermission,
    );

    // A connection parameter of a type the permission does not list.
    let mut value = fixture();
    value["contributes"]["actions"][1]["parameters"][0]["kind"]["types"] = json!(["jira"]);
    refused(
        &value,
        "contributes.actions[1].parameters[0].kind.types[0]",
        IssueCode::MissingPermission,
    );

    // Methods always include GET.
    let mut value = fixture();
    value["permissions"][0]["methods"] = json!(["POST"]);
    refused(&value, "permissions[0].methods", IssueCode::InvalidValue);
    assert!(!schema().is_valid(&value));
}

#[test]
fn connection_types_are_checked() {
    let cases: &[(&str, Value, &str, IssueCode)] = &[
        (
            "/cloud/web",
            json!("https://github.com/login"),
            "contributes.connectionTypes[0].cloud.web",
            IssueCode::InvalidUrl,
        ),
        (
            "/cloud/api",
            json!("http://api.github.com"),
            "contributes.connectionTypes[0].cloud.api",
            IssueCode::InvalidUrl,
        ),
        (
            "/cloud/api",
            json!("https://user@api.github.com"),
            "contributes.connectionTypes[0].cloud.api",
            IssueCode::InvalidUrl,
        ),
        (
            "/cloud/api",
            json!("https://api.github.com/a/../b"),
            "contributes.connectionTypes[0].cloud.api",
            IssueCode::InvalidUrl,
        ),
        (
            "/cloud/api",
            json!("https://localhost"),
            "contributes.connectionTypes[0].cloud.api",
            IssueCode::InvalidUrl,
        ),
        (
            "/selfHosted/apiPath",
            json!("/api/../v3"),
            "contributes.connectionTypes[0].selfHosted.apiPath",
            IssueCode::InvalidPath,
        ),
        (
            "/selfHosted/apiPath",
            json!("api/v3"),
            "contributes.connectionTypes[0].selfHosted.apiPath",
            IssueCode::InvalidPath,
        ),
        (
            "/selfHosted/apiPath",
            json!("/api/v3/"),
            "contributes.connectionTypes[0].selfHosted.apiPath",
            IssueCode::InvalidPath,
        ),
        (
            "/auth/0/scheme",
            json!("query"),
            "contributes.connectionTypes[0].auth[0].scheme",
            IssueCode::InvalidValue,
        ),
        (
            "/auth/0/setupUrl/selfHosted",
            json!("https://evil.example/settings/tokens"),
            "contributes.connectionTypes[0].auth[0].setupUrl.selfHosted",
            IssueCode::InvalidUrl,
        ),
        (
            "/auth/0/setupUrl/selfHosted",
            json!("{web}/../../evil"),
            "contributes.connectionTypes[0].auth[0].setupUrl.selfHosted",
            IssueCode::InvalidUrl,
        ),
        (
            "/auth/0/setupUrl/selfHosted",
            json!("{web}@evil.example/x"),
            "contributes.connectionTypes[0].auth[0].setupUrl.selfHosted",
            IssueCode::InvalidUrl,
        ),
        (
            "/auth/0/setupUrl/cloud",
            json!("https://evil.example/settings/tokens"),
            "contributes.connectionTypes[0].auth[0].setupUrl.cloud",
            IssueCode::InvalidUrl,
        ),
        (
            "/extensions/0",
            json!("gitlab.checks"),
            "contributes.connectionTypes[0].extensions[0]",
            IssueCode::InvalidId,
        ),
        (
            "/id",
            json!("Git_Hub"),
            "contributes.connectionTypes[0].id",
            IssueCode::InvalidId,
        ),
    ];
    for (pointer, replacement, path, code) in cases {
        let mut value = fixture();
        *value["contributes"]["connectionTypes"][0]
            .pointer_mut(pointer)
            .unwrap() = replacement.clone();
        refused(&value, path, *code);
    }
}

#[test]
fn connection_type_auth_headers_are_checked() {
    // The header scheme: a header name, never one the host owns.
    for name in [
        json!(null),
        json!("Authorization"),
        json!("Cookie"),
        json!("Proxy-Authorization"),
        json!("X Token"),
    ] {
        let mut value = fixture();
        let auth = &mut value["contributes"]["connectionTypes"][1]["auth"][0];
        if name.is_null() {
            auth.as_object_mut().unwrap().remove("headerName");
        } else {
            auth["headerName"] = name.clone();
        }
        let found = issues_at(&value, "contributes.connectionTypes[1].auth[0].headerName");
        assert!(!found.is_empty(), "{name}");
    }
    // A header name without the header scheme.
    let mut value = fixture();
    value["contributes"]["connectionTypes"][0]["auth"][0]["headerName"] = json!("X-Token");
    refused(
        &value,
        "contributes.connectionTypes[0].auth[0].headerName",
        IssueCode::Conflict,
    );
}

#[test]
fn connection_types_need_a_deployment_and_unique_ids() {
    // Neither cloud nor self-hosted.
    let mut value = fixture();
    let kind = value["contributes"]["connectionTypes"][1]
        .as_object_mut()
        .unwrap();
    kind.remove("cloud");
    kind.remove("selfHosted");
    refused(&value, "contributes.connectionTypes[1]", IssueCode::Count);
    assert!(!schema().is_valid(&value));
    // A self-hosted setup URL needs a self-hosted deployment.
    let mut value = fixture();
    value["contributes"]["connectionTypes"][0]
        .as_object_mut()
        .unwrap()
        .remove("selfHosted");
    refused(
        &value,
        "contributes.connectionTypes[0].auth[0].setupUrl.selfHosted",
        IssueCode::InvalidUrl,
    );
    // Ids are unique.
    let mut value = fixture();
    value["contributes"]["connectionTypes"][1]["id"] = json!("github");
    refused(
        &value,
        "contributes.connectionTypes[1]",
        IssueCode::Duplicate,
    );
}

#[test]
fn connection_parameters_are_picked_never_defaulted_and_never_settings() {
    let mut value = fixture();
    value["contributes"]["actions"][1]["parameters"][0]["default"] =
        json!({"type": "text", "value": "cn_01JAAAAAAAAAAAAAAAAAAAAAAA"});
    refused(
        &value,
        "contributes.actions[1].parameters[0].default",
        IssueCode::InvalidDefault,
    );

    let mut value = fixture();
    value["contributes"]["settings"] = json!({"fields": [{
        "key": "account", "label": "Account", "required": false,
        "kind": {"type": "connection", "types": ["github"]}
    }]});
    refused(
        &value,
        "contributes.settings.fields[0].kind",
        IssueCode::InvalidValue,
    );

    let mut value = fixture();
    value["contributes"]["actions"][1]["parameters"][0]["kind"]["types"] = json!([]);
    refused(
        &value,
        "contributes.actions[1].parameters[0].kind.types",
        IssueCode::Count,
    );
    assert!(!schema().is_valid(&value));
}

#[test]
fn options_come_from_a_read_only_internal_action_with_records() {
    let at = "contributes.actions[1].parameters[1].options";
    // The source is not internal.
    let mut value = fixture();
    value["contributes"]["actions"][0]
        .as_object_mut()
        .unwrap()
        .remove("internal");
    refused(&value, &format!("{at}.from"), IssueCode::Conflict);
    // The source modifies something.
    let mut value = fixture();
    value["contributes"]["actions"][0]["node"]["effect"] = json!("modifies");
    refused(&value, &format!("{at}.from"), IssueCode::Conflict);
    // An unknown source.
    let mut value = fixture();
    value["contributes"]["actions"][1]["parameters"][1]["options"]["from"] = json!("nope");
    refused(&value, &format!("{at}.from"), IssueCode::UnknownReference);
    // A field the schema does not have.
    let mut value = fixture();
    value["contributes"]["actions"][1]["parameters"][1]["options"]["label"] = json!("name");
    refused(&value, &format!("{at}.label"), IssueCode::UnknownReference);
    // A dependency the source does not take.
    let mut value = fixture();
    value["contributes"]["actions"][1]["parameters"][1]["options"]["dependsOn"] = json!(["ref"]);
    refused(
        &value,
        &format!("{at}.dependsOn[0]"),
        IssueCode::UnknownReference,
    );
    // Itself.
    let mut value = fixture();
    value["contributes"]["actions"][1]["parameters"][1]["options"]["dependsOn"] =
        json!(["repository"]);
    refused(
        &value,
        &format!("{at}.dependsOn[0]"),
        IssueCode::UnknownReference,
    );
    // `search` without a `search` parameter on the source.
    let mut value = fixture();
    value["contributes"]["actions"][0]["parameters"]
        .as_array_mut()
        .unwrap()
        .pop();
    refused(&value, &format!("{at}.search"), IssueCode::UnknownReference);
    // Only on a single-line text parameter.
    let mut value = fixture();
    value["contributes"]["actions"][1]["parameters"][1]["kind"] =
        json!({"type": "text", "multiline": true});
    refused(&value, at, IssueCode::Conflict);
}

#[test]
fn records_outputs_name_a_schema_and_internal_actions_stay_off_keys() {
    let mut value = fixture();
    value["contributes"]["actions"][0]["node"]["outputs"][0]
        .as_object_mut()
        .unwrap()
        .remove("schema");
    refused(
        &value,
        "contributes.actions[0].node.outputs[0].schema",
        IssueCode::Conflict,
    );
    let mut value = fixture();
    value["contributes"]["actions"][0]["node"]["outputs"][0]["schema"] = json!("issue");
    refused(
        &value,
        "contributes.actions[0].node.outputs[0].schema",
        IssueCode::UnknownReference,
    );
    let mut value = fixture();
    value["contributes"]["actions"][1]["node"]["outputs"][0]["schema"] = json!("pipeline");
    refused(
        &value,
        "contributes.actions[1].node.outputs[0].schema",
        IssueCode::Conflict,
    );
    let mut value = fixture();
    value["contributes"]["actions"][0]["state"] =
        json!({"refresh": "every", "intervalSeconds": 60});
    value["contributes"]["actions"][0]["controls"] = json!({"key": ["press"]});
    refused(&value, "contributes.actions[0].state", IssueCode::Conflict);
    refused(
        &value,
        "contributes.actions[0].controls",
        IssueCode::Conflict,
    );
    let mut value = fixture();
    value["contributes"]["recordSchemas"][0]["fields"][1]["key"] = json!("id");
    refused(
        &value,
        "contributes.recordSchemas[0].fields[1]",
        IssueCode::Duplicate,
    );
}

#[test]
fn connection_type_labels_are_translatable() {
    let manifest = parse(&fixture());
    let keys: Vec<(String, String)> = manifest.translation_keys();
    for (path, key) in [
        ("contributes.connectionTypes[0].label", "types.github"),
        (
            "contributes.connectionTypes[0].selfHosted.label",
            "types.ghes",
        ),
        ("contributes.connectionTypes[1].label", "types.gitlab"),
    ] {
        assert!(
            keys.iter().any(|(p, k)| p == path && k == key),
            "{path}: {keys:?}"
        );
    }
}

#[test]
fn an_older_agent_refuses_the_fields_and_a_1_9_manifest_is_unchanged() {
    // The structure is new: nothing of it hides in a manifest without it.
    let plain: Value =
        serde_json::from_str(include_str!("fixtures/example-manifest.json")).unwrap();
    let manifest = parse(&plain);
    assert_eq!(manifest.contributes.connection_types().len(), 0);
    assert_eq!(manifest.contributes.record_schemas().len(), 0);
    assert_eq!(manifest.privileged_usages().len(), 0);
    manifest.validate().unwrap();
    // Unknown members of the new objects are refused, like everywhere.
    let mut value = fixture();
    value["contributes"]["connectionTypes"][0]["insecure"] = json!(true);
    assert!(Manifest::from_json(&value.to_string()).is_err());
    let mut value = fixture();
    value["permissions"][0]["hosts"] = json!(["evil.example"]);
    assert!(Manifest::from_json(&value.to_string()).is_err());
}

#[test]
fn an_action_is_never_replaced_by_an_internal_one() {
    let mut value = fixture();
    value["contributes"]["actions"][1]["deprecated"] = json!(true);
    value["contributes"]["actions"][1]["replacedBy"] = json!("list-repositories");
    refused(
        &value,
        "contributes.actions[1].replacedBy",
        IssueCode::Conflict,
    );
}

#[test]
fn an_internal_action_declares_no_contexts() {
    // Contexts (1.11) place an action on a deck; an internal one is never
    // there, so even an otherwise valid list is refused.
    let mut value = fixture();
    value["contributes"]["actions"][0]["contexts"] = json!(["key"]);
    refused(
        &value,
        "contributes.actions[0].contexts",
        IssueCode::Conflict,
    );
    // The same list on the plugin's ordinary action is fine.
    let mut value = fixture();
    value["contributes"]["actions"][1]["contexts"] = json!(["key"]);
    parse(&value).validate_bundled().unwrap();
}

#[test]
fn naming_the_new_open_url_members_at_all_needs_their_level() {
    // `connections: false` is still a 1.14 field.
    let mut value = fixture();
    value["engines"]["pluginApi"] = json!("^1.9");
    value["permissions"][1]["connections"] = json!(false);
    value["permissions"][1]["hosts"] = json!(["gitlab.com"]);
    let errors = parse(&value).validate_bundled().unwrap_err();
    assert!(
        errors
            .0
            .iter()
            .any(|i| i.path == "permissions[1].connections" && i.message.contains("1.14")),
        "{errors}"
    );
    // `fromParams: false` is still a 1.9 field.
    let plain: Value =
        serde_json::from_str(include_str!("fixtures/example-manifest.json")).unwrap();
    let mut value = plain;
    value["permissions"].as_array_mut().unwrap().push(json!({
        "kind": "openUrl", "hosts": ["github.com"], "fromParams": false,
        "justification": "Opens pull requests in the browser."
    }));
    let errors = parse(&value).validate().unwrap_err();
    assert!(
        errors
            .0
            .iter()
            .any(|i| i.path.ends_with(".fromParams") && i.message.contains("1.9")),
        "{errors}"
    );
}

#[test]
fn options_depend_on_parameters_of_the_same_kind() {
    let at = "contributes.actions[1].parameters[1].options.dependsOn[0]";
    // The source takes the connection as text.
    let mut value = fixture();
    value["contributes"]["actions"][0]["parameters"][0]["kind"] = json!({"type": "text"});
    refused(&value, at, IssueCode::Conflict);
    // The source takes fewer connection types than the dependent offers.
    let mut value = fixture();
    value["contributes"]["actions"][0]["parameters"][0]["kind"]["types"] = json!(["gitlab"]);
    refused(&value, at, IssueCode::Conflict);
    // More types on the source side are fine.
    let mut value = fixture();
    value["contributes"]["actions"][1]["parameters"][0]["kind"]["types"] = json!(["gitlab"]);
    assert_eq!(issues_at(&value, at), []);
}

#[test]
fn cloud_urls_and_api_paths_have_one_spelling() {
    for (pointer, replacement, path, code) in [
        (
            "/cloud/api",
            json!("https://api.github.com:443"),
            "contributes.connectionTypes[0].cloud.api",
            IssueCode::InvalidUrl,
        ),
        (
            "/cloud/api",
            json!("https://api.github.com/"),
            "contributes.connectionTypes[0].cloud.api",
            IssueCode::InvalidUrl,
        ),
        (
            "/cloud/web",
            json!("https://github.com/"),
            "contributes.connectionTypes[0].cloud.web",
            IssueCode::InvalidUrl,
        ),
        (
            "/selfHosted/apiPath",
            json!("/api/./v3"),
            "contributes.connectionTypes[0].selfHosted.apiPath",
            IssueCode::InvalidPath,
        ),
    ] {
        let mut value = fixture();
        *value["contributes"]["connectionTypes"][0]
            .pointer_mut(pointer)
            .unwrap() = replacement.clone();
        refused(&value, path, code);
        // The schema says the same, except the default port, which only the
        // validator refuses.
        if !replacement.as_str().unwrap().ends_with(":443") {
            assert!(!schema().is_valid(&value), "{replacement}");
        }
    }
    // The schema refuses `.` and `..` segments in a cloud API path too.
    let mut value = fixture();
    value["contributes"]["connectionTypes"][1]["cloud"]["api"] =
        json!("https://gitlab.com/api/../v4");
    assert!(!schema().is_valid(&value));
    // Segments that only start with dots are fine.
    let mut value = fixture();
    value["contributes"]["connectionTypes"][1]["selfHosted"]["apiPath"] = json!("/.well/api");
    assert!(schema().is_valid(&value));
    assert_eq!(
        issues_at(&value, "contributes.connectionTypes[1].selfHosted"),
        []
    );
}

#[test]
fn gesture_presets_never_name_internal_actions_or_set_connections() {
    let preset = |action: &str, parameters: Value| {
        json!([{
            "id": "quick", "name": "Quick", "category": "Git Providers",
            "gestures": [{"gesture": "click", "action": action,
                          "parameters": parameters, "label": "Run"}]
        }])
    };
    // An internal action on a key, through a preset.
    let mut value = fixture();
    value["contributes"]["gesturePresets"] = preset(
        "list-repositories",
        json!({"connection": {"type": "text", "value": "cn_01JD7Q0000000000000000000A"}}),
    );
    refused(
        &value,
        "contributes.gesturePresets[0].gestures[0].action",
        IssueCode::Conflict,
    );
    // A literal value for a connection parameter is never allowed.
    let mut value = fixture();
    value["contributes"]["gesturePresets"] = preset(
        "pipeline-status",
        json!({
            "connection": {"type": "text", "value": "cn_01JD7Q0000000000000000000A"},
            "repository": {"type": "text", "value": "gl:4711"}
        }),
    );
    refused(
        &value,
        "contributes.gesturePresets[0].gestures[0].parameters.connection",
        IssueCode::InvalidDefault,
    );
}
