//! Plugin API 1.12: structured variable types (`object`, `map` of a shape)
//! and the repository host sources `host:project.repositories` and
//! `host:project.repository` (`docs/variables.md` §2.8). Privileged host
//! constructs: a bundled first-party plugin with `projectContext` only, at
//! `^1.12`, with exactly the source's type; a `^1.9` manifest validates
//! exactly as before.

use deckpilot_plugin_manifest::{
    IssueCode, Manifest, REPOSITORY_VARIABLES_API, SCHEMA_JSON, VariableHostSource, VariableShape,
    VariableValueKind,
};
use serde_json::{Value, json};

const EXAMPLE: &str = include_str!("fixtures/example-manifest.json");

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

/// The example as the first-party Projects plugin of plugin API `level`
/// with `projectContext` and the host variables `variables`.
fn projects(level: &str, variables: &[Value]) -> Value {
    let mut value: Value = serde_json::from_str(EXAMPLE).unwrap();
    value["id"] = json!("deckpilot.projects");
    value["publisher"]["id"] = json!("deckpilot");
    value["engines"]["pluginApi"] = json!(level);
    value["permissions"]
        .as_array_mut()
        .unwrap()
        .push(json!({"kind": "projectContext",
                     "justification": "Shows the active project's repositories on your keys."}));
    let list = value["contributes"]["variables"].as_array_mut().unwrap();
    list.extend(variables.iter().cloned());
    value
}

fn repositories() -> Value {
    json!({"id": "current.repositories", "name": "Repositories",
           "valueType": {"type": "map", "of": "repository"},
           "scope": "project", "source": "host:project.repositories"})
}

fn repository() -> Value {
    json!({"id": "current.repository", "name": "Default repository",
           "valueType": {"type": "object", "shape": "repository"},
           "scope": "project", "source": "host:project.repository"})
}

fn key() -> Value {
    json!({"id": "current.key", "name": "Project key", "valueType": {"type": "string"},
           "scope": "project", "source": "host:project.key"})
}

/// Index of the first added variable (the example declares its own).
fn first(value: &Value) -> usize {
    value["contributes"]["variables"].as_array().unwrap().len() - 2
}

#[test]
fn a_bundled_1_12_projects_plugin_declares_its_repositories() {
    assert_eq!(REPOSITORY_VARIABLES_API.to_string(), "1.12");
    let value = projects("^1.12", &[repositories(), repository()]);
    assert!(schema_ok(&value));
    let manifest = parse(&value);
    manifest.validate_bundled().unwrap();
    let variables = manifest.contributes.variables();
    let map = &variables[variables.len() - 2];
    assert_eq!(map.source, Some(VariableHostSource::ProjectRepositories));
    assert_eq!(map.value_type.kind, VariableValueKind::Map);
    assert_eq!(map.value_type.of, Some(VariableShape::Repository));
    let object = variables.last().unwrap();
    assert_eq!(object.value_type.shape, Some(VariableShape::Repository));
    // It round-trips as written.
    let back: Value = serde_json::to_value(map).unwrap();
    assert_eq!(
        back["valueType"],
        json!({"type": "map", "of": "repository"})
    );
}

#[test]
fn the_repository_sources_need_plugin_api_1_12() {
    for level in ["^1.9", "^1.11"] {
        let value = projects(level, &[repositories(), repository()]);
        let errors = parse(&value).validate_bundled().unwrap_err();
        let at = first(&value);
        for index in [at, at + 1] {
            assert!(
                errors.has(
                    &format!("contributes.variables[{index}].source"),
                    IssueCode::Conflict
                ),
                "{level}: {errors}"
            );
            assert!(
                errors.has(
                    &format!("contributes.variables[{index}].valueType"),
                    IssueCode::Conflict
                ),
                "{level}: {errors}"
            );
        }
        assert!(errors.to_string().contains("^1.12"), "{errors}");
    }
}

#[test]
fn a_1_9_projects_plugin_validates_exactly_as_before() {
    let value = projects("^1.9", &[key()]);
    assert!(schema_ok(&value));
    parse(&value).validate_bundled().unwrap();
    // Its string sources keep refusing every other type.
    let mut wrong = projects("^1.9", &[key()]);
    let at = first(&wrong) + 1;
    wrong["contributes"]["variables"][at]["valueType"] = json!({"type": "json"});
    let errors = parse(&wrong).validate_bundled().unwrap_err();
    assert!(errors.has(
        &format!("contributes.variables[{at}].valueType"),
        IssueCode::InvalidValue
    ));
    assert!(errors.to_string().contains("is a string"), "{errors}");
}

#[test]
fn each_source_has_exactly_its_type() {
    let refused = |variable: Value, code: IssueCode, field: &str| {
        let value = projects("^1.12", &[variable, key()]);
        let at = first(&value);
        let errors = parse(&value).validate_bundled().unwrap_err();
        assert!(
            errors.has(&format!("contributes.variables[{at}].{field}"), code),
            "{errors}"
        );
    };
    let mut map_as_object = repositories();
    map_as_object["valueType"] = json!({"type": "object", "shape": "repository"});
    refused(map_as_object, IssueCode::InvalidValue, "valueType");
    let mut object_as_string = repository();
    object_as_string["valueType"] = json!({"type": "string"});
    refused(object_as_string, IssueCode::InvalidValue, "valueType");
    let mut key_as_map = key();
    key_as_map["valueType"] = json!({"type": "map", "of": "repository"});
    refused(key_as_map, IssueCode::InvalidValue, "valueType");
    let mut not_project = repositories();
    not_project["scope"] = json!("machine");
    refused(not_project, IssueCode::InvalidValue, "scope");
}

#[test]
fn structured_types_name_their_shape_and_belong_to_host_sources() {
    let shapes = [
        (json!({"type": "object"}), false),
        (json!({"type": "map"}), false),
        (json!({"type": "object", "of": "repository"}), false),
        (json!({"type": "map", "shape": "repository"}), false),
        (
            json!({"type": "map", "of": "repository", "shape": "repository"}),
            false,
        ),
        (json!({"type": "string", "shape": "repository"}), false),
        (json!({"type": "map", "of": "repository"}), true),
    ];
    for (value_type, schema) in shapes {
        let mut variable = repositories();
        variable["valueType"] = value_type.clone();
        let value = projects("^1.12", &[variable, key()]);
        assert_eq!(schema_ok(&value), schema, "{value_type}");
        let errors = parse(&value).validate_bundled();
        assert_eq!(errors.is_ok(), schema, "{value_type}: {errors:?}");
    }
    // An unknown shape is not a manifest.
    let mut unknown = repositories();
    unknown["valueType"] = json!({"type": "map", "of": "folder"});
    let value = projects("^1.12", &[unknown]);
    assert!(Manifest::from_json(&value.to_string()).is_err());
    assert!(!schema_ok(&value));
    // Without a host source: a plugin never publishes structured values.
    for mut variable in [repositories(), repository()] {
        variable.as_object_mut().unwrap().remove("source");
        variable["scope"] = json!("machine");
        let value = projects("^1.12", &[variable, key()]);
        let at = first(&value);
        let errors = parse(&value).validate_bundled().unwrap_err();
        assert!(
            errors.has(
                &format!("contributes.variables[{at}].valueType"),
                IssueCode::InvalidValue
            ),
            "{errors}"
        );
        assert!(errors.to_string().contains("host `source`"), "{errors}");
    }
}

#[test]
fn only_a_bundled_first_party_plugin_with_project_context_has_them() {
    // Every other manifest: refused, as each host source is.
    let value = projects("^1.12", &[repositories(), repository()]);
    let at = first(&value);
    let errors = parse(&value).validate().unwrap_err();
    for (index, source) in [
        (at, "host:project.repositories"),
        (at + 1, "host:project.repository"),
    ] {
        assert!(
            errors.has(
                &format!("contributes.variables[{index}].source"),
                IssueCode::FirstPartyOnly
            ),
            "{errors}"
        );
        assert!(errors.to_string().contains(source), "{errors}");
    }
    // Without `projectContext`, whose values they are.
    let mut value = projects("^1.12", &[repositories(), repository()]);
    value["permissions"]
        .as_array_mut()
        .unwrap()
        .retain(|p| p["kind"] != "projectContext");
    let errors = parse(&value).validate_bundled().unwrap_err();
    assert!(
        errors.has(
            &format!("contributes.variables[{at}].source"),
            IssueCode::MissingPermission
        ),
        "{errors}"
    );
}
