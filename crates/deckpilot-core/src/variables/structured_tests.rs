//! Structured variables end to end in the core: the repository shape,
//! member access through the registry (canonical ids and the legacy
//! `projects.current.…` spellings), the no-default rule, private paths,
//! profile isolation, display safety, previews and older protocols.

use std::sync::Arc;

use super::*;
use crate::resolution::{ContextResolver, NoSharedVariables};
use crate::{
    MachineConfig, MachineResourceBinding, ModuleId, Os, Profile, Project, ProjectKey, Repository,
    RepositoryId, RepositoryKey, RepositoryRemote, ResourceKey, ResourceKind,
};

const MAP: &str = "plugins.deckpilot.projects.current.repositories";
const OBJECT: &str = "plugins.deckpilot.projects.current.repository";

/// What the agent's host sources `host:project.repositories` and
/// `host:project.repository` give: the core's builders on the context's
/// project, folders looked at by `probe`.
pub(crate) struct Repositories {
    probe: Arc<dyn FolderProbe>,
}

impl Repositories {
    fn value(
        &self,
        declaration: &VariableDeclaration,
        member: Option<&MemberPath>,
        environment: &VariableEnvironment<'_>,
    ) -> Resolution {
        let project = match environment.project() {
            Ok(project) => project,
            Err(resolution) => return resolution,
        };
        let (machine, paths, probe) = (
            environment.machine,
            environment.private_paths,
            self.probe.as_ref(),
        );
        let value = match declaration.value_type {
            VariableType::Map { .. } => {
                VariableValue::Map(repositories_value(project, machine, paths, member, probe))
            }
            VariableType::Object { .. } => {
                match default_repository_value(project, machine, paths, member, probe) {
                    Ok(object) => VariableValue::Object(object),
                    Err(reason) => return Resolution::Missing(reason),
                }
            }
            _ => VariableValue::String(project.key.to_string()),
        };
        Resolution::Value {
            value,
            source: ValueSource::System,
            sensitive: false,
        }
    }
}

impl VariableSource for Repositories {
    fn resolve(
        &self,
        declaration: &VariableDeclaration,
        environment: &VariableEnvironment<'_>,
    ) -> Resolution {
        self.value(declaration, None, environment)
    }

    fn resolve_member(
        &self,
        declaration: &VariableDeclaration,
        member: &MemberPath,
        environment: &VariableEnvironment<'_>,
    ) -> Resolution {
        self.value(declaration, Some(member), environment)
    }
}

pub(crate) fn registry() -> VariableRegistry {
    registry_with(Arc::new(Filesystem))
}

/// [`registry`], its folders looked at by `probe`.
pub(crate) fn registry_with(probe: Arc<dyn FolderProbe>) -> VariableRegistry {
    let declare = |id: &str, value_type| {
        VariableDeclaration::new(id, id, value_type, DeclaredScope::Project)
            .requires(Requirement::Project)
    };
    let mut registry = VariableRegistry::new();
    registry
        .register_module(
            &ModuleId::new("deckpilot.projects").unwrap(),
            &[
                declare(
                    MAP,
                    VariableType::Map {
                        of: Shape::Repository,
                    },
                ),
                declare(
                    OBJECT,
                    VariableType::Object {
                        shape: Shape::Repository,
                    },
                ),
                declare(
                    "plugins.deckpilot.projects.current.key",
                    VariableType::String,
                ),
            ],
            Some(Arc::new(Repositories { probe })),
        )
        .unwrap();
    registry
}

fn repository(key: &str, name: &str, resource: &str) -> Repository {
    Repository {
        id: RepositoryId::new(),
        key: RepositoryKey::new(key).unwrap(),
        name: name.into(),
        resource: ResourceKey::new(resource).unwrap(),
        description: None,
        remotes: Vec::new(),
    }
}

pub(crate) struct Fixture {
    pub profile: Profile,
    pub machine: MachineConfig,
    pub registry: VariableRegistry,
}

/// A profile whose project `SHOP` has `cms` (the default, its folder
/// exists, a primary remote) and `api` (unbound), and a project `SOLO`
/// with one repository and no default.
pub(crate) fn fixture() -> Fixture {
    let mut profile = Profile::new("Work");
    let mut shop = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
    let mut cms = repository("cms", "Storefront CMS", "shop.cms");
    cms.description = Some("Content".into());
    cms.remotes = vec![
        RepositoryRemote {
            name: "upstream".into(),
            ..RepositoryRemote::default()
        },
        RepositoryRemote {
            name: "origin".into(),
            primary: true,
            connection_id: Some("conn-1".into()),
            remote_repository_id: Some("gh:123".into()),
            remote_url: Some("https://github.com/acme/cms".into()),
            default_branch: Some("main".into()),
        },
    ];
    let api = repository("api", "API", "shop.api");
    shop.default_repository = Some(cms.id);
    shop.repositories = vec![cms, api];
    let mut solo = Project::new(ProjectKey::new("SOLO").unwrap(), "Solo");
    solo.repositories = vec![repository("main", "Solo", "solo.main")];
    profile.projects = vec![shop, solo];
    let mut machine = MachineConfig::new(Os::current());
    machine.set_binding(MachineResourceBinding {
        key: ResourceKey::new("shop.cms").unwrap(),
        kind: ResourceKind::Directory,
        value: std::env::temp_dir().to_string_lossy().into_owned(),
    });
    Fixture {
        profile,
        machine,
        registry: registry(),
    }
}

impl Fixture {
    fn project(&self, key: &str) -> &Project {
        self.profile
            .projects
            .iter()
            .find(|p| p.key.as_str() == key)
            .unwrap()
    }

    fn context(&self, key: &str) -> ResolutionContext {
        ResolutionContext::preview(self.profile.id)
            .with_project(Some(self.project(key).id), ProjectSource::Application)
    }

    fn resolver(&self, private_paths: bool) -> ContextResolver<'_> {
        ContextResolver {
            profile: &self.profile,
            os: Os::current(),
            shared: &NoSharedVariables,
            outputs: None,
            registry: &self.registry,
            live: &NoLiveState,
            machine: Some(&self.machine),
            private_paths,
        }
    }

    fn resolve(&self, reference: &str, project: &str, private_paths: bool) -> Resolution {
        self.resolver(private_paths).resolve(
            &VariableRef::parse(reference).unwrap(),
            &self.context(project),
        )
    }

    fn display(&self, template: &str, project: &str) -> DisplayText {
        let context = display_context(self.profile.id, Some(self.project(project).id));
        // The engine's display resolver hides paths (`display_templates`).
        let resolver = self.resolver(false);
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

/// `text` as it appears inside a JSON string (a Windows folder's `\` is
/// escaped there).
fn in_json(text: &str) -> String {
    let quoted = serde_json::to_string(text).unwrap();
    quoted[1..quoted.len() - 1].to_owned()
}

fn text(resolution: &Resolution) -> Option<&str> {
    match resolution {
        Resolution::Value {
            value: VariableValue::String(text),
            ..
        } => Some(text),
        _ => None,
    }
}

fn missing(reason: MissingReason) -> Resolution {
    Resolution::Missing(reason)
}

#[test]
fn the_types_serialize_with_their_shape_and_never_render_as_text() {
    let map = VariableType::Map {
        of: Shape::Repository,
    };
    let object = VariableType::Object {
        shape: Shape::Repository,
    };
    assert_eq!(
        serde_json::to_value(map).unwrap(),
        serde_json::json!({"type": "map", "of": "repository"})
    );
    assert_eq!(
        serde_json::to_value(object).unwrap(),
        serde_json::json!({"type": "object", "shape": "repository"})
    );
    let json = crate::ParameterKind::Code {
        language: "json".into(),
        max_length: None,
    };
    let text = crate::ParameterKind::Text {
        multiline: true,
        max_length: None,
    };
    for value_type in [map, object] {
        assert!(value_type.is_structured());
        assert!(value_type.assignable_to(&json));
        assert!(!value_type.assignable_to(&text));
        assert!(!value_type.assignable_to(&crate::ParameterKind::Choice { options: vec![] }));
        assert_eq!(value_type.for_protocol(17), VariableType::Json);
        assert_eq!(value_type.for_protocol(18), value_type);
    }
    let f = fixture();
    let Resolution::Value { value, .. } = f.resolve(MAP, "SHOP", true) else {
        panic!("a map");
    };
    assert_eq!(value.render_text(), None);
    assert_eq!(value.to_parameter(&text), None);
    let Some(crate::ParameterValue::Text(passed)) = value.to_parameter(&json) else {
        panic!("JSON");
    };
    let passed: serde_json::Value = serde_json::from_str(&passed).unwrap();
    assert_eq!(passed["cms"]["name"], "Storefront CMS");
    assert_eq!(passed["cms"]["remote"]["defaultBranch"], "main");
    // K4: neither id of a remote is part of the value.
    assert!(!passed.to_string().contains("conn-1"));
    assert!(!passed.to_string().contains("gh:123"));
}

#[test]
fn a_map_reads_each_repository_by_key_in_the_users_order() {
    let f = fixture();
    let Resolution::Value {
        value: VariableValue::Map(map),
        ..
    } = f.resolve(MAP, "SHOP", true)
    else {
        panic!("a map");
    };
    let keys: Vec<&str> = map.entries.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(keys, ["cms", "api"]);
    assert_eq!(
        text(&f.resolve(&format!("{MAP}.cms.name"), "SHOP", true)),
        Some("Storefront CMS")
    );
    assert_eq!(
        text(&f.resolve(&format!("{MAP}.cms.path"), "SHOP", true)),
        Some(f.machine.bindings[0].value.as_str())
    );
    assert_eq!(
        text(&f.resolve(&format!("{MAP}.cms.remote.url"), "SHOP", true)),
        Some("https://github.com/acme/cms")
    );
    assert_eq!(
        text(&f.resolve(&format!("{MAP}.cms.remote.name"), "SHOP", true)),
        Some("origin")
    );
    let flag = |reference: String| match f.resolve(&reference, "SHOP", true) {
        Resolution::Value {
            value: VariableValue::Boolean(flag),
            ..
        } => flag,
        other => panic!("{other:?}"),
    };
    assert!(flag(format!("{MAP}.cms.isDefault")));
    assert!(!flag(format!("{MAP}.api.isDefault")));
    assert!(flag(format!("{MAP}.cms.exists")));
    assert!(!flag(format!("{MAP}.api.exists")));
    // Unbound: no folder; no description, no remote: no value.
    for absent in ["api.path", "api.description", "api.remote.url"] {
        assert_eq!(
            f.resolve(&format!("{MAP}.{absent}"), "SHOP", true),
            missing(MissingReason::NoValue),
            "{absent}"
        );
    }
    // One entry whole: an object of the shape.
    assert!(matches!(
        f.resolve(&format!("{MAP}.api"), "SHOP", true),
        Resolution::Value {
            value: VariableValue::Object(ObjectValue {
                shape: Shape::Repository,
                ..
            }),
            ..
        }
    ));
}

#[test]
fn the_default_repository_follows_the_no_default_rule() {
    let mut f = fixture();
    assert_eq!(
        text(&f.resolve(&format!("{OBJECT}.key"), "SHOP", true)),
        Some("cms")
    );
    // Exactly one repository: that one, without a default.
    assert_eq!(
        text(&f.resolve(&format!("{OBJECT}.key"), "SOLO", true)),
        Some("main")
    );
    // Several and no default: no value, never the first by position.
    f.profile.projects[0].default_repository = None;
    assert_eq!(
        f.resolve(&format!("{OBJECT}.key"), "SHOP", true),
        missing(MissingReason::NoDefaultRepository)
    );
    assert_eq!(
        f.resolve(OBJECT, "SHOP", true),
        missing(MissingReason::NoDefaultRepository)
    );
    f.profile.projects[0].repositories.reverse();
    assert_eq!(
        f.resolve(&format!("{OBJECT}.key"), "SHOP", true),
        missing(MissingReason::NoDefaultRepository)
    );
    // None at all: no value.
    f.profile.projects[0].repositories.clear();
    assert_eq!(
        f.resolve(&format!("{OBJECT}.key"), "SHOP", true),
        missing(MissingReason::NoValue)
    );
    // The repository role alone (R3): the derived repository `main`.
    f.profile.projects[0]
        .resources
        .insert("repository".into(), ResourceKey::new("shop.cms").unwrap());
    assert_eq!(
        text(&f.resolve(&format!("{OBJECT}.key"), "SHOP", true)),
        Some("main")
    );
    assert!(text(&f.resolve(&format!("{OBJECT}.path"), "SHOP", true)).is_some());
}

#[test]
fn the_legacy_spellings_read_the_same_values() {
    let f = fixture();
    for (legacy, canonical) in [
        (
            "projects.current.repositories.cms.name",
            format!("{MAP}.cms.name"),
        ),
        (
            "projects.current.repositories.cms.path",
            format!("{MAP}.cms.path"),
        ),
        ("projects.current.repository.name", format!("{OBJECT}.name")),
        (
            "projects.current.repository.remote.defaultBranch",
            format!("{OBJECT}.remote.defaultBranch"),
        ),
        ("projects.current.repositories", MAP.to_owned()),
        ("projects.current.repository", OBJECT.to_owned()),
    ] {
        let a = f.resolve(legacy, "SHOP", true);
        assert!(matches!(a, Resolution::Value { .. }), "{legacy}: {a:?}");
        assert_eq!(a, f.resolve(&canonical, "SHOP", true), "{legacy}");
        // Parsed and printed as written.
        assert_eq!(VariableRef::parse(legacy).unwrap().to_string(), legacy);
    }
    let id = VariableId::reference("projects.current.repositories.cms.remote.url").unwrap();
    let (base, member) = f.registry.split_member(&id).unwrap();
    assert_eq!(base.as_str(), "projects.current.repositories");
    assert_eq!(member.as_str(), "cms.remote.url");
    // The scalar ids resolve as before: nothing to split.
    let key = VariableId::new("plugins.deckpilot.projects.current.key").unwrap();
    assert_eq!(f.registry.split_member(&key), None);
    assert_eq!(
        text(&f.resolve("plugins.deckpilot.projects.current.key", "SHOP", true)),
        Some("SHOP")
    );
}

#[test]
fn invalid_references_resolve_to_nothing() {
    let f = fixture();
    let cases = [
        (
            format!("{MAP}.web.name"),
            MissingReason::RepositoryNotInProject,
        ),
        (format!("{MAP}.web"), MissingReason::RepositoryNotInProject),
        (format!("{MAP}.cms.nope"), MissingReason::UnknownMember),
        // A group is read member by member.
        (format!("{MAP}.cms.remote"), MissingReason::UnknownMember),
        (format!("{OBJECT}.remote"), MissingReason::UnknownMember),
        (format!("{OBJECT}.nope"), MissingReason::UnknownMember),
        // K4: never members.
        (
            format!("{OBJECT}.remote.connectionId"),
            MissingReason::UnknownMember,
        ),
        (
            format!("{OBJECT}.remote.remoteRepositoryId"),
            MissingReason::UnknownMember,
        ),
        // A scalar has no members: no such variable.
        (
            "plugins.deckpilot.projects.current.key.x".to_owned(),
            MissingReason::UnknownVariable,
        ),
        (
            "plugins.deckpilot.projects.current.other.a.b.c".to_owned(),
            MissingReason::UnknownVariable,
        ),
    ];
    for (reference, reason) in cases {
        assert_eq!(
            f.resolve(&reference, "SHOP", true),
            missing(reason),
            "{reference}"
        );
    }
    // A key of another project of the profile is not in this one.
    assert_eq!(
        f.resolve(&format!("{MAP}.main.name"), "SHOP", true),
        missing(MissingReason::RepositoryNotInProject)
    );
    // A member path longer than three segments names nothing.
    assert_eq!(
        f.resolve(&format!("{MAP}.cms.remote.url.x"), "SHOP", true),
        missing(MissingReason::UnknownVariable)
    );
    // Too long, or not segments: not a reference.
    for bad in [
        format!("{MAP}.cms.remote.url.x.y"),
        "projects.current.repositories.Cms.name".to_owned(),
        "projects.current.repositories.cms-x.name".to_owned(),
    ] {
        assert!(VariableRef::parse(&bad).is_err(), "{bad}");
    }
    // Without the plugin: unavailable, as any of its variables.
    let none = VariableRegistry::new();
    let resolver = ContextResolver {
        registry: &none,
        ..f.resolver(true)
    };
    assert_eq!(
        resolver.resolve(
            &VariableRef::parse(&format!("{MAP}.cms.name")).unwrap(),
            &f.context("SHOP")
        ),
        Resolution::Unavailable {
            owner: "deckpilot.projects".into()
        }
    );
}

/// A source that gives folders to every reader.
struct Leaky;

impl VariableSource for Leaky {
    fn resolve(&self, d: &VariableDeclaration, e: &VariableEnvironment<'_>) -> Resolution {
        let paths = VariableEnvironment {
            private_paths: true,
            ..*e
        };
        Repositories {
            probe: Arc::new(Filesystem),
        }
        .resolve(d, &paths)
    }
}

#[test]
fn a_reader_that_may_not_see_paths_reads_null() {
    let f = fixture();
    assert_eq!(
        f.resolve(&format!("{MAP}.cms.path"), "SHOP", false),
        missing(MissingReason::NoValue)
    );
    assert_eq!(
        f.resolve(&format!("{OBJECT}.path"), "SHOP", false),
        missing(MissingReason::NoValue)
    );
    // Whether it exists is still told; everything else is unchanged.
    assert!(matches!(
        f.resolve(&format!("{OBJECT}.exists"), "SHOP", false),
        Resolution::Value {
            value: VariableValue::Boolean(true),
            ..
        }
    ));
    let folder = f.machine.bindings[0].value.clone();
    for whole in [MAP.to_owned(), OBJECT.to_owned(), format!("{MAP}.cms")] {
        let Resolution::Value { value, .. } = f.resolve(&whole, "SHOP", false) else {
            panic!("{whole}");
        };
        let json = value.structured_json().unwrap().to_string();
        assert!(!json.contains(&in_json(&folder)), "{whole}: {json}");
        let Resolution::Value { value, .. } = f.resolve(&whole, "SHOP", true) else {
            panic!("{whole}");
        };
        assert!(
            value
                .structured_json()
                .unwrap()
                .to_string()
                .contains(&in_json(&folder))
        );
    }
    // Even a source that gives paths regardless is cut down by the
    // registry for such a reader.
    let mut leaky = VariableRegistry::new();
    leaky
        .register_module(
            &ModuleId::new("deckpilot.projects").unwrap(),
            &[VariableDeclaration::new(
                MAP,
                "Repositories",
                VariableType::Map {
                    of: Shape::Repository,
                },
                DeclaredScope::Project,
            )],
            Some(Arc::new(Leaky)),
        )
        .unwrap();
    let resolver = ContextResolver {
        registry: &leaky,
        ..f.resolver(false)
    };
    let context = f.context("SHOP");
    assert_eq!(
        resolver.resolve(
            &VariableRef::parse(&format!("{MAP}.cms.path")).unwrap(),
            &context
        ),
        missing(MissingReason::NoValue)
    );
    let Resolution::Value { value, .. } =
        resolver.resolve(&VariableRef::parse(MAP).unwrap(), &context)
    else {
        panic!("a map");
    };
    assert!(
        !value
            .structured_json()
            .unwrap()
            .to_string()
            .contains(&in_json(&folder))
    );
}

#[test]
fn values_never_cross_profiles() {
    let f = fixture();
    let other = Profile::new("Home");
    // A context of another profile, or a project of another profile.
    let context = ResolutionContext::preview(other.id)
        .with_project(Some(f.project("SHOP").id), ProjectSource::Application);
    for reference in [
        MAP.to_owned(),
        format!("{MAP}.cms.path"),
        format!("{OBJECT}.name"),
    ] {
        assert_eq!(
            f.resolver(true)
                .resolve(&VariableRef::parse(&reference).unwrap(), &context),
            missing(MissingReason::OtherProfile),
            "{reference}"
        );
    }
    let foreign = Project::new(ProjectKey::new("X").unwrap(), "X");
    let context = ResolutionContext::preview(f.profile.id)
        .with_project(Some(foreign.id), ProjectSource::Application);
    assert_eq!(
        f.resolver(true).resolve(
            &VariableRef::parse(&format!("{MAP}.cms.name")).unwrap(),
            &context
        ),
        missing(MissingReason::OtherProfile)
    );
    // A run's project is not previewed with the active one.
    let run = ResolutionContext::preview(f.profile.id)
        .with_project(Some(f.project("SHOP").id), ProjectSource::Execution);
    assert_eq!(
        f.resolver(true).resolve(
            &VariableRef::parse(&format!("{MAP}.cms.name")).unwrap(),
            &run
        ),
        Resolution::UnavailableUntilRuntime
    );
}

#[test]
fn keys_show_display_safe_members_only() {
    let f = fixture();
    let shown = f.display(
        "{{ projects.current.repository.name }} on {{ projects.current.repositories.cms.remote.defaultBranch }}",
        "SHOP",
    );
    assert_eq!(shown.text, "Storefront CMS on main");
    assert_eq!(shown.unresolved, Vec::new());
    for refused in [
        "projects.current.repository.path",
        "projects.current.repositories.cms.path",
        "projects.current.repository.id",
        "projects.current.repositories.cms.remote.url",
        // Whole values never render.
        "projects.current.repositories",
        "projects.current.repository",
        "projects.current.repositories.cms",
        MAP,
    ] {
        let shown = f.display(&format!("[{{{{ {refused} }}}}]"), "SHOP");
        assert_eq!(shown.text, "[-]", "{refused}");
        assert_eq!(
            shown.unresolved[0].reason,
            UnresolvedReason::NotDisplayable,
            "{refused}"
        );
        assert!(!shown.text.contains(&f.machine.bindings[0].value));
        assert_eq!(
            display_safe(&VariableRef::parse(refused).unwrap(), &f.registry),
            Err(UnresolvedReason::NotDisplayable),
            "{refused}"
        );
    }
    // A repository the project does not have, an unknown member.
    let shown = f.display("{{ projects.current.repositories.web.name }}", "SHOP");
    assert_eq!(shown.text, "-");
    assert_eq!(
        shown.unresolved[0].reason,
        UnresolvedReason::Missing {
            missing: MissingReason::RepositoryNotInProject
        }
    );
    assert_eq!(
        display_safe(
            &VariableRef::parse("projects.current.repository.nope").unwrap(),
            &f.registry
        ),
        Err(UnresolvedReason::Missing {
            missing: MissingReason::UnknownMember
        })
    );
    // Several repositories and no default: `-`, never the first.
    let mut f = fixture();
    f.profile.projects[0].default_repository = None;
    let shown = f.display("{{ projects.current.repository.name }}", "SHOP");
    assert_eq!(shown.text, "-");
    assert_eq!(
        shown.unresolved[0].reason,
        UnresolvedReason::Missing {
            missing: MissingReason::NoDefaultRepository
        }
    );
}

#[test]
#[allow(clippy::too_many_lines)] // both catalogs, both protocols
fn the_title_picker_lists_display_safe_members_and_never_structured_values() {
    let f = fixture();
    let context = f.context("SHOP");
    let project = Some(f.project("SHOP"));
    let reference = |entry: &CatalogEntry| entry.reference.to_string();
    let all = CatalogFilter::default();

    // The configuration catalog: each structured variable once, with its
    // shape's members and, for the map, the repositories' keys; never one
    // entry per member.
    let configuration = catalog_listing(f.registry.catalog(&context, false), project, &all);
    let map = configuration.iter().find(|e| reference(e) == MAP).unwrap();
    assert_eq!(map.members.len(), Shape::Repository.members().len());
    assert_eq!(map.map_keys, ["cms", "api"]);
    assert!(map.member_of.is_none() && !map.members_only);
    let object = configuration
        .iter()
        .find(|e| reference(e) == OBJECT)
        .unwrap();
    assert_eq!(object.map_keys, Vec::<String>::new());
    assert!(configuration.iter().all(|e| e.member_of.is_none()));
    // K4: never a connection or provider id.
    for entry in [map, object] {
        assert!(
            entry
                .members
                .iter()
                .all(|m| !m.path.contains("connectionId") && !m.path.contains("RepositoryId"))
        );
    }
    // The app composes the member entries.
    let expanded: Vec<String> = expand_members(configuration.clone(), &all)
        .iter()
        .map(reference)
        .collect();
    for expected in [
        MAP.to_owned(),
        format!("{MAP}.cms.name"),
        format!("{MAP}.api.path"),
        format!("{MAP}.cms.remote.url"),
        OBJECT.to_owned(),
        format!("{OBJECT}.exists"),
    ] {
        assert!(expanded.contains(&expected), "{expected}: {expanded:?}");
    }
    let members = Shape::Repository.members().len();
    assert_eq!(
        expanded.len(),
        configuration.len() + 2 * members + members,
        "{expanded:?}"
    );

    // The title picker's catalog: the structured variables for their
    // display-safe members only.
    let display = display_listing(
        f.registry.catalog(&context, false),
        project,
        &f.registry,
        &all,
    );
    let map = display.iter().find(|e| reference(e) == MAP).unwrap();
    assert!(map.members_only);
    assert_eq!(map.map_keys, ["cms", "api"]);
    assert!(map.members.iter().all(|m| m.display_safe));
    assert!(
        !map.members
            .iter()
            .any(|m| m.path == "path" || m.path == "id")
    );
    let listed: Vec<String> = expand_members(display.clone(), &all)
        .iter()
        .map(reference)
        .collect();
    for expected in [
        format!("{MAP}.cms.name"),
        format!("{MAP}.api.key"),
        format!("{MAP}.cms.remote.defaultBranch"),
        format!("{OBJECT}.name"),
        format!("{OBJECT}.exists"),
    ] {
        assert!(listed.contains(&expected), "{expected}: {listed:?}");
    }
    for never in [
        MAP.to_owned(),
        OBJECT.to_owned(),
        format!("{MAP}.cms.path"),
        format!("{MAP}.cms.id"),
        format!("{MAP}.cms.remote.url"),
        format!("{OBJECT}.path"),
        format!("{OBJECT}.id"),
    ] {
        assert!(!listed.contains(&never), "{never}");
    }
    // Every listed member resolves on the key.
    for entry in &listed {
        let shown = f.display(&format!("{{{{ {entry} }}}}"), "SHOP");
        assert!(
            shown.unresolved.is_empty()
                || matches!(shown.unresolved[0].reason, UnresolvedReason::Missing { .. }),
            "{entry}: {shown:?}"
        );
    }
    // Composed member entries name their variable.
    let composed = expand_members(display.clone(), &all);
    let member = composed
        .iter()
        .find(|e| reference(e) == format!("{MAP}.cms.name"))
        .unwrap();
    assert_eq!(member.member_of.as_deref(), Some(MAP));

    // Protocol 17: the configuration catalog's variables are `json`
    // without members or keys; the title picker's are left out.
    let map = configuration.iter().find(|e| reference(e) == MAP).unwrap();
    let old = map.clone().for_protocol(17).unwrap();
    assert_eq!(old.value_type, VariableType::Json);
    assert_eq!(old.members, Vec::new());
    assert_eq!(old.map_keys, Vec::<String>::new());
    assert_eq!(map.clone().for_protocol(18).unwrap(), *map);
    assert!(
        display
            .iter()
            .filter(|e| e.value_type.is_structured())
            .all(|e| e.clone().for_protocol(17).is_none())
    );
    assert_eq!(member.clone().for_protocol(17), None);
}

/// A search matches a structured variable by its members too: the agent
/// lists it, and the app keeps only what matches.
#[test]
fn a_search_finds_members_and_the_app_keeps_only_the_matches() {
    let f = fixture();
    let context = f.context("SHOP");
    let project = Some(f.project("SHOP"));
    let reference = |entry: &CatalogEntry| entry.reference.to_string();
    let filter = CatalogFilter {
        query: "api default branch".into(),
        ..CatalogFilter::default()
    };
    let listed = catalog_listing(f.registry.catalog(&context, false), project, &filter);
    assert_eq!(listed.iter().map(reference).collect::<Vec<_>>(), [MAP]);
    let shown: Vec<String> = expand_members(listed, &filter)
        .iter()
        .map(reference)
        .collect();
    assert_eq!(shown, [format!("{MAP}.api.remote.defaultBranch")]);
    // Only text: the map itself is not, its names and keys are.
    let text = CatalogFilter {
        compatible_with: Some(crate::ParameterKind::Text {
            multiline: false,
            max_length: None,
        }),
        ..CatalogFilter::default()
    };
    let listed = catalog_listing(f.registry.catalog(&context, false), project, &text);
    let shown: Vec<String> = expand_members(listed, &text)
        .iter()
        .map(reference)
        .collect();
    assert!(!shown.contains(&MAP.to_owned()), "{shown:?}");
    assert!(shown.contains(&format!("{MAP}.cms.name")), "{shown:?}");
}

/// Counts the folders it is asked about.
#[derive(Default)]
struct CountingProbe(std::sync::atomic::AtomicUsize);

impl CountingProbe {
    fn take(&self) -> usize {
        self.0.swap(0, std::sync::atomic::Ordering::SeqCst)
    }
}

impl FolderProbe for CountingProbe {
    fn is_dir(&self, folder: &str) -> bool {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::path::Path::new(folder).is_dir()
    }
}

/// A member read builds only what it needs: no folder is looked at for any
/// member but `exists`, and then only the one repository's (finding of
/// the review of #460: a key title must not stall on an unmounted
/// network volume).
#[test]
fn a_member_read_looks_at_a_folder_only_for_exists() {
    let probe = Arc::new(CountingProbe::default());
    let mut f = fixture();
    f.registry = registry_with(probe.clone());
    // Both repositories of SHOP bound, so a whole read looks at two.
    f.machine.set_binding(MachineResourceBinding {
        key: ResourceKey::new("shop.api").unwrap(),
        kind: ResourceKind::Directory,
        value: std::env::temp_dir().to_string_lossy().into_owned(),
    });
    for member in Shape::Repository.members() {
        let path = member.path;
        let expected = usize::from(path == "exists");
        for private_paths in [true, false] {
            let _ = f.resolve(&format!("{MAP}.cms.{path}"), "SHOP", private_paths);
            assert_eq!(probe.take(), expected, "{MAP}.cms.{path}");
            let _ = f.resolve(&format!("{OBJECT}.{path}"), "SHOP", private_paths);
            assert_eq!(probe.take(), expected, "{OBJECT}.{path}");
        }
    }
    // On a key: names never touch the filesystem.
    for template in [
        "{{ projects.current.repositories.cms.name }}",
        "{{ projects.current.repository.name }}",
        "{{ projects.current.repositories.web.name }}",
    ] {
        let _ = f.display(template, "SHOP");
        assert_eq!(probe.take(), 0, "{template}");
    }
    assert_eq!(
        f.display("{{ projects.current.repositories.cms.exists }}", "SHOP")
            .text,
        "true"
    );
    assert_eq!(probe.take(), 1);
    // The values are those of a whole read.
    let whole = |reference: &str| match f.resolve(reference, "SHOP", true) {
        Resolution::Value { value, .. } => value.structured_json().unwrap(),
        other => panic!("{other:?}"),
    };
    let map = whole(MAP);
    assert_eq!(probe.take(), 2);
    assert_eq!(
        text(&f.resolve(&format!("{MAP}.api.name"), "SHOP", true)),
        map["api"]["name"].as_str()
    );
    assert_eq!(whole(&format!("{MAP}.cms")), map["cms"]);
    assert_eq!(probe.take(), 1);
    assert_eq!(
        f.resolve(&format!("{MAP}.web.name"), "SHOP", true),
        missing(MissingReason::RepositoryNotInProject)
    );
    assert_eq!(probe.take(), 0);
}

#[test]
fn previews_summarise_structured_values() {
    let f = fixture();
    let status = preview(f.resolve(MAP, "SHOP", true), None);
    assert_eq!(
        status,
        ReferenceStatus::Structured {
            value_type: VariableType::Map {
                of: Shape::Repository
            },
            entries: 2,
            source: ValueSource::System,
        }
    );
    let json = serde_json::to_string(&status).unwrap();
    assert!(
        !json.contains(&in_json(&f.machine.bindings[0].value)),
        "{json}"
    );
    assert_eq!(
        status.for_protocol(17),
        ReferenceStatus::Redacted {
            value_type: VariableType::Json,
            source: ValueSource::System,
        }
    );
    // A member is a scalar preview.
    assert!(matches!(
        preview(f.resolve(&format!("{OBJECT}.name"), "SHOP", true), None),
        ReferenceStatus::Resolved { .. }
    ));
    // Older clients know none of the new reasons.
    let missing = preview(f.resolve(&format!("{MAP}.web.name"), "SHOP", true), None);
    assert_eq!(
        missing.clone().for_protocol(17),
        ReferenceStatus::Missing {
            reason: MissingReason::NoValue
        }
    );
    assert_eq!(missing.clone().for_protocol(18), missing);
    assert_eq!(
        MissingReason::UnknownMember.for_protocol(17),
        MissingReason::UnknownVariable
    );
    // A text parameter cannot take a whole structured value.
    let text = crate::ParameterKind::Text {
        multiline: false,
        max_length: None,
    };
    assert!(matches!(
        preview(f.resolve(MAP, "SHOP", true), Some(&text)),
        ReferenceStatus::TypeMismatch { .. }
    ));
}

#[test]
fn user_variables_cannot_be_structured() {
    let mut definition = VariableDefinition::new(
        VariableKey::new("team.repos").unwrap(),
        VariableType::Map {
            of: Shape::Repository,
        },
        &[VariableScope::Global],
    );
    assert!(
        definition
            .issues()
            .contains(&DefinitionIssue::Structured(definition.key.clone()))
    );
    definition.value_type = VariableType::Json;
    let value = VariableValue::Map(MapValue {
        shape: Shape::Repository,
        entries: Vec::new(),
    });
    assert_eq!(
        definition.check_value(&ScopeKey::Global, &value),
        Err(DefinitionIssue::Structured(definition.key.clone()))
    );
}

#[test]
fn templates_never_interpolate_structured_values() {
    let f = fixture();
    let resolver = f.resolver(true);
    let context = f.context("SHOP");
    let mut redactor = crate::resolution::Redactor::default();
    let text = crate::ParameterKind::Text {
        multiline: false,
        max_length: None,
    };
    for reference in [MAP, OBJECT] {
        let parsed = VariableRef::parse(reference).unwrap();
        let resolution = resolver.resolve(&parsed, &context);
        assert!(matches!(
            bind("x", &text, &parsed, resolution),
            Err(ReferenceError {
                problem: ReferenceProblem::TypeMismatch { .. },
                ..
            })
        ));
    }
    // A member is text as any string.
    let parsed = VariableRef::parse("projects.current.repository.name").unwrap();
    assert_eq!(
        bind("x", &text, &parsed, resolver.resolve(&parsed, &context)),
        Ok(crate::ParameterValue::Text("Storefront CMS".into()))
    );
    // A sensitive structured value is redacted member by member.
    redactor.add_value(&VariableValue::Object(ObjectValue {
        shape: Shape::Repository,
        members: std::collections::BTreeMap::from([(
            "name".into(),
            Scalar::String("hidden-name".into()),
        )]),
    }));
    assert_eq!(redactor.redact("a hidden-name b"), "a [redacted] b");
}
