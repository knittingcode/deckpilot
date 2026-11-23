use std::sync::Arc;

use super::*;
use crate::resolution::ContextResolver;
use crate::variables::doubles::{MemoryVariables, builtin_registry};
use crate::variables::{
    Requirement, ScopeKey, VariableDeclaration, VariableDefinition, VariableEnvironment,
    VariableKey, VariableScope, VariableSource,
};
use crate::{ModuleId, Os, Profile, Project, ProjectKey, render_label};

struct Fixture {
    profile: Profile,
    shared: MemoryVariables,
    registry: VariableRegistry,
}

fn key(text: &str) -> VariableKey {
    VariableKey::new(text).unwrap()
}

#[allow(clippy::too_many_lines)] // One shared fixture for every case.
fn fixture() -> Fixture {
    let mut profile = Profile::new("Work");
    profile.projects.push(Project::new(
        ProjectKey::new("SHOP").unwrap(),
        "Online shop",
    ));
    profile
        .projects
        .push(Project::new(ProjectKey::new("CMS").unwrap(), "Content"));
    let shop = profile.projects[0].id;
    let shared = MemoryVariables::new(profile.clone())
        .define(VariableDefinition::new(
            key("team.name"),
            VariableType::String,
            &[VariableScope::Global, VariableScope::Project],
        ))
        .set(
            &key("team.name"),
            ScopeKey::Global,
            VariableValue::String("Core".into()),
        )
        .set(
            &key("team.name"),
            ScopeKey::Project {
                profile: profile.id,
                project: shop,
            },
            VariableValue::String("Shop team".into()),
        )
        .define(VariableDefinition::new(
            key("team.size"),
            VariableType::Integer,
            &[VariableScope::Global],
        ))
        .set(
            &key("team.size"),
            ScopeKey::Global,
            VariableValue::Integer(7),
        )
        .define(VariableDefinition {
            sensitive: true,
            ..VariableDefinition::new(
                key("vault.pin"),
                VariableType::String,
                &[VariableScope::Global],
            )
        })
        .set(
            &key("vault.pin"),
            ScopeKey::Global,
            VariableValue::String("s3cr3t-pin".into()),
        )
        .define(VariableDefinition::new(
            key("vault.token"),
            VariableType::SecretRef,
            &[VariableScope::Global],
        ))
        .set(
            &key("vault.token"),
            ScopeKey::Global,
            VariableValue::SecretRef(crate::CredentialId::new("github-token").unwrap()),
        )
        .define(VariableDefinition::new(
            key("team.unset"),
            VariableType::String,
            &[VariableScope::Global],
        ));
    let mut registry = builtin_registry();
    let plugin = ModuleId::new("acme.weather").unwrap();
    registry
        .register_module(
            &plugin,
            &[
                VariableDeclaration::new(
                    "plugins.acme.weather.city",
                    "City",
                    VariableType::String,
                    DeclaredScope::Machine,
                ),
                VariableDeclaration {
                    sensitive: true,
                    ..VariableDeclaration::new(
                        "plugins.acme.weather.apiKey",
                        "Key",
                        VariableType::String,
                        DeclaredScope::Machine,
                    )
                },
                VariableDeclaration::new(
                    "plugins.acme.weather.device",
                    "Device",
                    VariableType::String,
                    DeclaredScope::Device,
                ),
                // Declared non-sensitive; its source says the value is.
                VariableDeclaration::new(
                    "plugins.acme.weather.leaky",
                    "Leaky",
                    VariableType::String,
                    DeclaredScope::Machine,
                ),
                // A `host:project.id` source: an identifier.
                VariableDeclaration {
                    identifier: true,
                    ..VariableDeclaration::new(
                        "plugins.acme.weather.project",
                        "Project",
                        VariableType::String,
                        DeclaredScope::Project,
                    )
                    .requires(Requirement::Project)
                },
            ],
            Some(Arc::new(Weather)),
        )
        .unwrap();
    // The Projects plugin's id, a plain string as its manifest declares it.
    registry
        .register_module(
            &ModuleId::new("deckpilot.projects").unwrap(),
            &[VariableDeclaration::new(
                "plugins.deckpilot.projects.current.id",
                "Project id",
                VariableType::String,
                DeclaredScope::Project,
            )],
            Some(Arc::new(Weather)),
        )
        .unwrap();
    Fixture {
        profile,
        shared,
        registry,
    }
}

/// A plugin's variables: every one has a value, the sensitive one a secret.
struct Weather;

impl VariableSource for Weather {
    fn resolve(
        &self,
        declaration: &VariableDeclaration,
        _environment: &VariableEnvironment<'_>,
    ) -> Resolution {
        let name = declaration.id.name();
        Resolution::Value {
            value: VariableValue::String(match name {
                "acme.weather.apiKey" | "acme.weather.leaky" => "plugin-secret".into(),
                "deckpilot.projects.current.id" | "acme.weather.project" => "7f1c2a9e-uuid".into(),
                _ => "Gdańsk".into(),
            }),
            source: super::super::ValueSource::System,
            sensitive: name == "acme.weather.leaky",
        }
    }
}

impl Fixture {
    fn resolve(&self, project: Option<usize>, template: &str) -> DisplayText {
        let project = project.map(|index| self.profile.projects[index].id);
        let context = display_context(self.profile.id, project);
        let resolver = ContextResolver {
            profile: &self.profile,
            os: Os::Linux,
            shared: &self.shared,
            outputs: None,
            registry: &self.registry,
            live: &super::super::NoLiveState,
        };
        resolve_display_template(
            &DisplayEnvironment {
                context: &context,
                resolver: &resolver,
                registry: &self.registry,
            },
            template,
        )
    }
}

#[test]
fn a_static_title_is_unchanged() {
    let fixture = fixture();
    for title in [
        "Build",
        "",
        "Two\nlines",
        "{ not a placeholder }",
        "a } b { c",
        "Zażółć",
    ] {
        let text = fixture.resolve(Some(0), title);
        assert_eq!(text, DisplayText::plain(title), "{title:?}");
        assert!(!text.is_dynamic());
    }
}

#[test]
fn one_variable_several_and_mixed_text() {
    let fixture = fixture();
    assert_eq!(
        fixture.resolve(Some(0), "{{ projects.current.key }}").text,
        "SHOP"
    );
    assert_eq!(
        fixture.resolve(Some(0), "{{projects.current.name}}").text,
        "Online shop"
    );
    assert_eq!(
        fixture
            .resolve(Some(0), "Projekt: {{ projects.current.key }}")
            .text,
        "Projekt: SHOP"
    );
    assert_eq!(
        fixture
            .resolve(
                Some(0),
                "{{ system.profile.current.name }} — {{ projects.current.name }} ({{ shared.team.size }})"
            )
            .text,
        "Work — Online shop (7)"
    );
    // A project value wins over the global one, for that project only.
    assert_eq!(
        fixture.resolve(Some(0), "{{ shared.team.name }}").text,
        "Shop team"
    );
    assert_eq!(
        fixture.resolve(Some(1), "{{ shared.team.name }}").text,
        "Core"
    );
    // A plugin's variable.
    assert_eq!(
        fixture
            .resolve(None, "{{ plugins.acme.weather.city }}")
            .text,
        "Gdańsk"
    );
    // Multiline is kept: the renderer breaks there.
    assert_eq!(
        fixture
            .resolve(
                Some(0),
                "{{ projects.current.key }}\n{{ shared.team.size }}"
            )
            .text,
        "SHOP\n7"
    );
}

#[test]
fn legacy_placeholders_resolve_through_the_engine_as_before() {
    let fixture = fixture();
    let shop = &fixture.profile.projects[0];
    for template in [
        "Git {project.key}",
        "{project.name}",
        "{profile}: {project.key}",
        "{profile} — {project.name}",
        "Projekt: {project.key}",
        "Plain",
        "{page.number}",
        "{{profile}}",
    ] {
        assert_eq!(
            fixture.resolve(Some(0), template).text,
            render_label(template, "Work", Some(shop)),
            "{template}"
        );
        assert_eq!(
            fixture.resolve(None, template).text,
            render_label(template, "Work", None),
            "{template}"
        );
    }
    let text = fixture.resolve(Some(0), "{profile} {project.key}");
    assert!(text.dependencies.contains(&DisplayDependency::Profile));
    assert!(text.dependencies.contains(&DisplayDependency::Project));
}

#[test]
fn unresolved_references_show_the_marker_and_are_reported() {
    let fixture = fixture();
    let text = fixture.resolve(
        None,
        "{{ projects.current.key }}/{{ shared.team.unset }}/{{ shared.nope.nope }}/{project.key}",
    );
    assert_eq!(text.text, "-/-/-/-");
    let names: Vec<(&str, &UnresolvedReason)> = text
        .unresolved
        .iter()
        .map(|u| (u.reference.as_str(), &u.reason))
        .collect();
    assert_eq!(
        names,
        [
            (
                "projects.current.key",
                &UnresolvedReason::Missing {
                    missing: MissingReason::NoProject
                }
            ),
            (
                "shared.team.unset",
                &UnresolvedReason::Missing {
                    missing: MissingReason::NoValue
                }
            ),
            (
                "shared.nope.nope",
                &UnresolvedReason::Missing {
                    missing: MissingReason::UnknownVariable
                }
            ),
            (
                "project.key",
                &UnresolvedReason::Missing {
                    missing: MissingReason::NoProject
                }
            ),
        ]
    );
    // Each name once; the dependencies stay, so a value that appears later
    // is shown.
    let twice = fixture.resolve(None, "{{ shared.team.unset }} {{ shared.team.unset }}");
    assert_eq!(twice.unresolved.len(), 1);
    assert!(twice.dependencies.contains(&DisplayDependency::Variable {
        reference: "shared.team.unset".into()
    }));
}

#[test]
fn text_that_is_not_a_reference_stays_as_typed() {
    let fixture = fixture();
    let text = fixture.resolve(Some(0), "{{ env.home }} and {{ unclosed");
    assert_eq!(text.text, "{{ env.home }} and {{ unclosed");
    assert_eq!(text.invalid, ["env.home"]);
    assert_eq!(text.unresolved, []);
}

#[test]
fn secrets_and_unsafe_sources_never_resolve() {
    let fixture = fixture();
    let cases = [
        ("shared.vault.pin", UnresolvedReason::NotDisplayable),
        ("shared.vault.token", UnresolvedReason::NotDisplayable),
        (
            "plugins.acme.weather.apiKey",
            UnresolvedReason::NotDisplayable,
        ),
        (
            "plugins.acme.weather.device",
            UnresolvedReason::NotDisplayable,
        ),
        ("system.device.current.id", UnresolvedReason::NotDisplayable),
        (
            "system.profile.current.id",
            UnresolvedReason::NotDisplayable,
        ),
        ("workflow.id", UnresolvedReason::NotDisplayable),
        ("workflow.execution.id", UnresolvedReason::RuntimeOnly),
        ("steps.a.output.b", UnresolvedReason::RuntimeOnly),
        ("workSession.state", UnresolvedReason::NotDisplayable),
        // A source that marks its value sensitive, against its declaration.
        (
            "plugins.acme.weather.leaky",
            UnresolvedReason::NotDisplayable,
        ),
        // Ids: by declaration (`host:project.id`) and by name.
        (
            "plugins.acme.weather.project",
            UnresolvedReason::NotDisplayable,
        ),
        (
            "plugins.deckpilot.projects.current.id",
            UnresolvedReason::NotDisplayable,
        ),
        ("projects.current.id", UnresolvedReason::NotDisplayable),
        (
            "system.project.current.id",
            UnresolvedReason::NotDisplayable,
        ),
        (
            "system.work.session.state",
            UnresolvedReason::NotDisplayable,
        ),
    ];
    for (reference, reason) in cases {
        let text = fixture.resolve(Some(0), &format!("A {{{{ {reference} }}}} B"));
        assert_eq!(text.text, "A - B", "{reference}");
        assert_eq!(
            text.unresolved,
            [UnresolvedReference {
                reference: reference.to_owned(),
                reason
            }],
            "{reference}"
        );
    }
    let all = fixture.resolve(
        Some(0),
        "{{ shared.vault.pin }} {{ shared.vault.token }} {{ plugins.acme.weather.apiKey }} \
         {{ plugins.acme.weather.leaky }} {{ plugins.acme.weather.project }} \
         {{ plugins.deckpilot.projects.current.id }} {{ projects.current.id }}",
    );
    let json = serde_json::to_string(&all).unwrap();
    let debug = format!("{all:?}");
    // The message names the case by its position, never the secret.
    for (case, secret) in ["s3cr3t-pin", "github-token", "plugin-secret", "7f1c2a9e"]
        .into_iter()
        .enumerate()
    {
        let leaked = json.contains(secret) || debug.contains(secret);
        assert!(!leaked, "secret case {case} reached the output");
    }
}

#[test]
fn a_disabled_or_missing_plugin_shows_the_marker_without_a_stale_value() {
    let mut fixture = fixture();
    let template = "Weather: {{ plugins.acme.weather.city }}";
    assert_eq!(fixture.resolve(None, template).text, "Weather: Gdańsk");
    let plugin = ModuleId::new("acme.weather").unwrap();
    fixture
        .registry
        .set_available(&plugin, Some("disabled".into()));
    let disabled = fixture.resolve(None, template);
    assert_eq!(disabled.text, "Weather: -");
    assert_eq!(
        disabled.unresolved[0].reason,
        UnresolvedReason::Unavailable {
            owner: "acme.weather".into()
        }
    );
    fixture.registry.unregister_module(&plugin);
    let gone = fixture.resolve(None, template);
    assert_eq!(gone.text, "Weather: -");
    assert!(matches!(
        gone.unresolved[0].reason,
        UnresolvedReason::Unavailable { .. }
    ));
    // A plugin that was never installed.
    assert_eq!(
        fixture
            .resolve(None, "{{ plugins.other.thing.value }}")
            .text,
        "-"
    );
}

#[test]
fn values_never_cross_profiles() {
    let fixture = fixture();
    let other = Profile::new("Other");
    let context = display_context(other.id, None);
    let resolver = ContextResolver {
        profile: &fixture.profile,
        os: Os::Linux,
        shared: &fixture.shared,
        outputs: None,
        registry: &fixture.registry,
        live: &super::super::NoLiveState,
    };
    let text = resolve_display_template(
        &DisplayEnvironment {
            context: &context,
            resolver: &resolver,
            registry: &fixture.registry,
        },
        "{{ shared.team.name }} {profile}",
    );
    assert_eq!(text.text, "- -");
    assert!(text.unresolved.iter().all(|u| u.reason
        == UnresolvedReason::Missing {
            missing: MissingReason::OtherProfile
        }));
}

#[test]
fn long_values_are_shortened() {
    let mut fixture = fixture();
    fixture.shared = fixture.shared.set(
        &key("team.name"),
        ScopeKey::Global,
        VariableValue::String("x".repeat(1000)),
    );
    let text = fixture.resolve(None, "{{ shared.team.name }}").text;
    assert_eq!(text.chars().count(), MAX_DISPLAY_VALUE);
    assert!(text.ends_with('…'));
}

#[test]
fn dependencies_name_what_announces_a_change() {
    let fixture = fixture();
    let text = fixture.resolve(
        Some(0),
        "{{ shared.team.name }} {project.key} {{ projects.current.name }} {{ plugins.acme.weather.city }}",
    );
    let changed = |key: &str| text.dependencies.iter().any(|d| d.changed_by(key));
    // Shared variables are announced by their key.
    assert!(changed("team.name"));
    // Legacy ids by their canonical id and the projects plugin's.
    assert!(changed("projects.current.key"));
    assert!(changed("plugins.deckpilot.projects.current.name"));
    assert!(changed("plugins.acme.weather.city"));
    assert!(!changed("team.size"));
    assert!(!changed("plugins.acme.weather.apiKey"));
    assert!(text.dependencies.contains(&DisplayDependency::Project));
}

#[test]
fn display_text_serializes_for_the_editor() {
    let fixture = fixture();
    let text = fixture.resolve(None, "{{ projects.current.key }}");
    assert_eq!(
        serde_json::to_value(&text).unwrap(),
        serde_json::json!({
            "text": "-",
            "unresolved": [{"reference": "projects.current.key", "reason": "missing", "missing": "noProject"}],
            "markers": [{"start": 0, "end": 1, "reference": "projects.current.key"}],
            "dependencies": [{"kind": "project"}, {"kind": "variable", "reference": "projects.current.key"}],
        })
    );
    assert_eq!(
        serde_json::to_value(DisplayText::plain("A")).unwrap(),
        serde_json::json!({"text": "A"})
    );
    let key = TextKey {
        page: crate::PageId::new(),
        control: ControlId::Key { index: 0 },
        trigger: Trigger::Press,
        slot: TextSlot::Title,
    };
    let json = serde_json::to_value(key).unwrap();
    assert_eq!(json["slot"], serde_json::json!({"kind": "title"}));
}

#[test]
fn markers_are_told_apart_from_values_that_read_like_one() {
    let mut fixture = fixture();
    fixture.shared = fixture.shared.set(
        &key("team.name"),
        ScopeKey::Global,
        VariableValue::String("-".into()),
    );
    // "Zespół: " is 8 UTF-16 units; the value "-" is not a marker.
    let text = fixture.resolve(
        None,
        "Zespół: {{ shared.team.name }} {{ shared.team.unset }}",
    );
    assert_eq!(text.text, "Zespół: - -");
    assert_eq!(
        text.markers,
        [MarkerSpan {
            start: 10,
            end: 11,
            reference: "shared.team.unset".into()
        }]
    );
    let twice = fixture.resolve(None, "{{ shared.team.unset }}{{ shared.team.unset }}");
    assert_eq!(twice.markers.len(), 2);
    assert_eq!(twice.unresolved.len(), 1);
}

#[test]
fn the_picker_never_offers_ids_or_secrets() {
    use crate::variables::{CatalogEntry, ReferenceAvailability, VariableGroup};
    let fixture = fixture();
    let context = display_context(fixture.profile.id, Some(fixture.profile.projects[0].id));
    let listed: Vec<String> = fixture
        .registry
        .catalog(&context, false)
        .into_iter()
        .filter(|entry| display_listed(entry, &fixture.registry))
        .map(|entry| entry.reference.to_string())
        .collect();
    assert!(
        listed.contains(&"plugins.acme.weather.city".to_owned()),
        "{listed:?}"
    );
    assert!(
        listed.contains(&"system.project.current.key".to_owned()),
        "{listed:?}"
    );
    for refused in [
        "plugins.acme.weather.apiKey",
        "plugins.acme.weather.project",
        "plugins.deckpilot.projects.current.id",
        "projects.current.id",
        "system.project.current.id",
        "system.profile.current.id",
        "workSession.id",
    ] {
        assert!(
            !listed.contains(&refused.to_owned()),
            "{refused}: {listed:?}"
        );
    }
    // A shared variable is judged by its entry.
    let shared = |sensitive: bool, value_type: VariableType| CatalogEntry {
        reference: VariableRef::parse("shared.team.name").unwrap(),
        label: String::new(),
        description: String::new(),
        group: VariableGroup::Shared,
        value_type,
        scope: None,
        module: None,
        sensitive,
        dynamic: false,
        availability: ReferenceAvailability::Available,
        owner: None,
        read_only: false,
        deprecated: None,
        declared_scope: None,
    };
    assert!(display_listed(
        &shared(false, VariableType::String),
        &fixture.registry
    ));
    assert!(!display_listed(
        &shared(true, VariableType::String),
        &fixture.registry
    ));
    assert!(!display_listed(
        &shared(false, VariableType::SecretRef),
        &fixture.registry
    ));
    assert!(!display_listed(
        &shared(false, VariableType::ProjectId),
        &fixture.registry
    ));
}
