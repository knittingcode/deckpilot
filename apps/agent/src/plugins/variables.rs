//! Plugin-owned variables (design §2.3 item 3, §3.7, §5.2 "own
//! variables").
//!
//! A plugin declares its variables in `contributes.variables` of its
//! manifest; they are registered under its namespace,
//! `plugins.<publisherCamel>.<nameCamel>.<id>`, as variables of its
//! module (`engine.rs`). Values come only from what the plugin published
//! with the `variables.publish` capability while it ran: the agent keeps
//! them per plugin, in memory ([`PublishedVariables`]), and
//! [`PluginVariableSource`] resolves from them. No plugin code runs during
//! a resolution.
//!
//! Values are not persisted. They are dropped when the plugin's host is
//! stopped for a lifecycle reason (disable, update, rollback, uninstall,
//! quarantine, crash loop, lock): references then resolve as unavailable
//! (the module is disabled or gone) or, once it runs again, as having no
//! value until it publishes again. Every drop starts a new generation, so
//! a publish from an invocation that began before it is refused.
//!
//! Host-published variables (plugin API 1.9, X7 in
//! `docs/marketplace/module-migration.md` §7): a variable declared with a
//! `source` (`host:project.id`, `.key`, `.name`) is the host's value, the
//! project in the resolution's context, computed on every resolution like
//! the built-in `projects.current.*`. The plugin never writes it
//! (`variables.publish` refuses its id, `executor`). It has a value only
//! while the plugin is enabled and holds `projectContext`
//! ([`PluginVariableSource::with_host_context`]); otherwise it is
//! unavailable with its module, or has no value.
//!
//! Plugin API 1.12 adds the repositories of that project:
//! `host:project.repositories` (`map<repository>`, every repository by
//! key) and `host:project.repository` (`object<repository>`, the default
//! one by decision OD-3), built by the core
//! ([`deckpilot_core::variables::repositories_value`]). A repository's
//! folder comes from this computer's bindings and is `null` for every
//! reader that may not see paths (`VariableEnvironment::private_paths`:
//! key titles, and the parameters of a plugin without `resources:paths`).
//! Structured values are the host's only: `variables.publish` refuses them
//! ([`typed`]).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, PoisonError, RwLock};

use deckpilot_core::variables::{
    DeclaredScope, Filesystem, FolderProbe, MemberPath, MissingReason, Requirement, Resolution,
    Shape, ValueSource, VariableDeclaration, VariableEnvironment, VariableId, VariableSource,
    VariableType, VariableValue, default_repository_value, repositories_value,
};
use deckpilot_plugin_manifest::{
    Variable, VariableHostSource, VariableRequirement, VariableScope, VariableShape,
    VariableValueKind, VariableValueType,
};

/// The values one plugin published.
#[derive(Debug, Default)]
struct Values {
    generation: u64,
    /// By relative id (`temperature`).
    values: BTreeMap<String, VariableValue>,
}

/// Every plugin's published values. Cheap to clone; every clone is the
/// same store.
#[derive(Debug, Clone, Default)]
pub(crate) struct PublishedVariables(Arc<RwLock<HashMap<String, Values>>>);

impl PublishedVariables {
    /// The current generation of `plugin`'s values: an invocation captures
    /// it when it is prepared, and may publish only while it holds.
    pub fn generation(&self, plugin: &str) -> u64 {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(plugin)
            .map_or(0, |v| v.generation)
    }

    /// Sets (`Some`) or clears (`None`) one value. `false` when the
    /// generation is stale (the plugin was stopped since): nothing changes.
    #[cfg(test)]
    pub fn set(
        &self,
        plugin: &str,
        generation: u64,
        id: &str,
        value: Option<VariableValue>,
    ) -> bool {
        self.update(plugin, generation, id, value).is_some()
    }

    /// Sets (`Some`) or clears (`None`) one value, saying whether it changed:
    /// `None` when the generation is stale, `Some(false)` when the value
    /// was already the one given (nothing to announce, nothing to redraw).
    pub fn update(
        &self,
        plugin: &str,
        generation: u64,
        id: &str,
        value: Option<VariableValue>,
    ) -> Option<bool> {
        let mut all = self.0.write().unwrap_or_else(PoisonError::into_inner);
        let values = all.entry(plugin.to_owned()).or_default();
        if values.generation != generation {
            return None;
        }
        if values.values.get(id) == value.as_ref() {
            return Some(false);
        }
        match value {
            Some(value) => {
                values.values.insert(id.to_owned(), value);
            }
            None => {
                values.values.remove(id);
            }
        }
        Some(true)
    }

    /// Clears every value of `plugin` within `generation`; the ids that
    /// had one. Nothing when the generation is stale.
    pub fn clear(&self, plugin: &str, generation: u64) -> Option<Vec<String>> {
        let mut all = self.0.write().unwrap_or_else(PoisonError::into_inner);
        let values = all.entry(plugin.to_owned()).or_default();
        (values.generation == generation)
            .then(|| std::mem::take(&mut values.values).into_keys().collect())
    }

    /// Drops every value of `plugin` and starts a new generation (its host
    /// was stopped). The ids that had a value.
    pub fn forget(&self, plugin: &str) -> Vec<String> {
        let mut all = self.0.write().unwrap_or_else(PoisonError::into_inner);
        let values = all.entry(plugin.to_owned()).or_default();
        values.generation += 1;
        std::mem::take(&mut values.values).into_keys().collect()
    }

    /// Drops every plugin's values (the store was closed).
    pub fn forget_all(&self) -> Vec<(String, Vec<String>)> {
        let plugins: Vec<String> = self
            .0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect();
        plugins
            .into_iter()
            .map(|plugin| {
                let ids = self.forget(&plugin);
                (plugin, ids)
            })
            .collect()
    }

    fn get(&self, plugin: &str, id: &str) -> Option<VariableValue> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(plugin)?
            .values
            .get(id)
            .cloned()
    }
}

/// The declarations of a plugin's variables, in its namespace (read-only,
/// like every module variable). A declaration that does not make a valid
/// id is left out (the manifest's validation prevents it).
pub(crate) fn declarations(namespace: &str, variables: &[Variable]) -> Vec<VariableDeclaration> {
    variables
        .iter()
        .filter_map(|variable| {
            let id = VariableId::new(format!("{namespace}.{}", variable.id)).ok()?;
            Some(VariableDeclaration {
                id,
                name: variable.name.clone(),
                description: variable.description.clone().unwrap_or_default(),
                value_type: value_type(&variable.value_type)?,
                scope: match variable.scope {
                    VariableScope::Machine => DeclaredScope::Machine,
                    VariableScope::Profile => DeclaredScope::Profile,
                    VariableScope::Project => DeclaredScope::Project,
                },
                requires: variable
                    .requires
                    .iter()
                    .flatten()
                    .map(|requirement| match requirement {
                        VariableRequirement::Project => Requirement::Project,
                    })
                    .chain(variable.source.map(|_| Requirement::Project))
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                sensitive: variable.sensitive.unwrap_or(false),
                read_only: true,
                // The host's project id is an id, never text on a key.
                identifier: variable.source == Some(VariableHostSource::ProjectId),
            })
        })
        .collect()
}

/// The core's type of a declared value type; `None` for a structured type
/// without its shape (the manifest's validation prevents it).
fn value_type(declared: &VariableValueType) -> Option<VariableType> {
    let shape = |shape: Option<VariableShape>| {
        shape.map(|shape| match shape {
            VariableShape::Repository => Shape::Repository,
        })
    };
    Some(match declared.kind {
        VariableValueKind::String => VariableType::String,
        VariableValueKind::Integer => VariableType::Integer,
        VariableValueKind::Boolean => VariableType::Boolean,
        VariableValueKind::Json => VariableType::Json,
        VariableValueKind::Object => VariableType::Object {
            shape: shape(declared.shape)?,
        },
        VariableValueKind::Map => VariableType::Map {
            of: shape(declared.of)?,
        },
    })
}

/// A published JSON value as a value of `variable`'s declared type, or why
/// not. `null` clears the value (`Ok(None)`).
pub(crate) fn typed(
    variable: &Variable,
    value: &serde_json::Value,
) -> Result<Option<VariableValue>, String> {
    use serde_json::Value;
    if variable.value_type.is_structured() || variable.source.is_some() {
        return Err(format!(
            "`{}` is DeckPilot's to publish: a plugin cannot publish it",
            variable.id
        ));
    }
    let typed = match (variable.value_type.kind, value) {
        (_, Value::Null) => return Ok(None),
        (VariableValueKind::String, Value::String(text)) => VariableValue::String(text.clone()),
        (VariableValueKind::Integer, Value::Number(number)) if number.is_i64() => {
            VariableValue::Integer(number.as_i64().unwrap_or_default())
        }
        (VariableValueKind::Boolean, Value::Bool(flag)) => VariableValue::Boolean(*flag),
        (VariableValueKind::Json, other) => VariableValue::Json(other.clone()),
        (kind, _) => {
            return Err(format!(
                "`{}` is declared as {}; the value is not one",
                variable.id,
                match kind {
                    VariableValueKind::String => "a string",
                    VariableValueKind::Integer => "an integer",
                    VariableValueKind::Boolean => "a boolean",
                    VariableValueKind::Json => "JSON",
                    VariableValueKind::Object | VariableValueKind::Map => "a structured value",
                }
            ));
        }
    };
    Ok(Some(typed))
}

/// Resolves a plugin's variables from what it published (design §3.7),
/// and its host-published ones from the context: never runs plugin code.
pub(crate) struct PluginVariableSource {
    plugin: String,
    namespace: String,
    values: PublishedVariables,
    /// The host-published variables, by relative id (X7).
    host: BTreeMap<String, VariableHostSource>,
    /// The plugin is enabled and holds `projectContext`: only then do
    /// host-published variables have a value.
    host_context: bool,
    /// Tells whether a repository's folder exists: the filesystem, asked
    /// only for a read of `exists` or a whole value.
    probe: Arc<dyn FolderProbe>,
}

impl PluginVariableSource {
    pub fn new(plugin: String, namespace: String, values: PublishedVariables) -> Self {
        Self {
            plugin,
            namespace,
            values,
            host: BTreeMap::new(),
            host_context: false,
            probe: Arc::new(Filesystem),
        }
    }

    /// The same source, folders looked at by `probe`.
    #[cfg(test)]
    #[must_use]
    fn with_probe(mut self, probe: Arc<dyn FolderProbe>) -> Self {
        self.probe = probe;
        self
    }

    /// The host-published variables among `variables`, with a value only
    /// when `active` (the plugin is enabled and holds `projectContext`).
    #[must_use]
    pub fn with_host_context(mut self, variables: &[Variable], active: bool) -> Self {
        self.host = variables
            .iter()
            .filter_map(|v| Some((v.id.clone(), v.source?)))
            .collect();
        self.host_context = active;
        self
    }
}

/// The value of a host-published variable: the project in the context.
/// For a read of `member` of a structured one, only what it needs: one
/// repository, its folder looked at only for `exists`.
fn host_value(
    source: VariableHostSource,
    member: Option<&MemberPath>,
    probe: &dyn FolderProbe,
    environment: &VariableEnvironment<'_>,
) -> Resolution {
    let project = match environment.project() {
        Ok(project) => project,
        Err(resolution) => return resolution,
    };
    let (machine, paths) = (environment.machine, environment.private_paths);
    let value = match source {
        VariableHostSource::ProjectId => VariableValue::String(project.id.to_string()),
        VariableHostSource::ProjectKey => VariableValue::String(project.key.to_string()),
        VariableHostSource::ProjectName => VariableValue::String(project.name.clone()),
        VariableHostSource::ProjectRepositories => {
            VariableValue::Map(repositories_value(project, machine, paths, member, probe))
        }
        VariableHostSource::ProjectRepository => {
            match default_repository_value(project, machine, paths, member, probe) {
                Ok(object) => VariableValue::Object(object),
                Err(reason) => return Resolution::Missing(reason),
            }
        }
    };
    Resolution::Value {
        value,
        source: ValueSource::System,
        sensitive: false,
    }
}

impl VariableSource for PluginVariableSource {
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

impl PluginVariableSource {
    fn value(
        &self,
        declaration: &VariableDeclaration,
        member: Option<&MemberPath>,
        environment: &VariableEnvironment<'_>,
    ) -> Resolution {
        let relative = declaration
            .id
            .as_str()
            .strip_prefix(&self.namespace)
            .and_then(|rest| rest.strip_prefix('.'));
        if let Some(source) = relative.and_then(|id| self.host.get(id)) {
            return if self.host_context {
                host_value(*source, member, self.probe.as_ref(), environment)
            } else {
                Resolution::Missing(MissingReason::NoValue)
            };
        }
        match relative.and_then(|id| self.values.get(&self.plugin, id)) {
            Some(value) => Resolution::Value {
                value,
                source: ValueSource::System,
                sensitive: false,
            },
            None => Resolution::Missing(MissingReason::NoValue),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variable(id: &str, kind: &str, scope: &str) -> Variable {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": id, "valueType": {"type": kind}, "scope": scope
        }))
        .unwrap()
    }

    #[test]
    fn values_are_kept_per_plugin_and_generation() {
        let store = PublishedVariables::default();
        let generation = store.generation("acme.a");
        assert!(store.set("acme.a", generation, "x", Some(VariableValue::Integer(1))));
        assert_eq!(store.get("acme.a", "x"), Some(VariableValue::Integer(1)));
        assert_eq!(store.get("acme.b", "x"), None);
        assert_eq!(store.forget("acme.a"), vec!["x".to_owned()]);
        assert_eq!(store.get("acme.a", "x"), None);
        // An invocation from before the stop publishes nothing.
        assert!(!store.set("acme.a", generation, "x", Some(VariableValue::Integer(2))));
        assert_eq!(store.clear("acme.a", generation), None);
        let next = store.generation("acme.a");
        assert_ne!(next, generation);
        assert!(store.set("acme.a", next, "x", Some(VariableValue::Integer(3))));
        assert!(store.set("acme.a", next, "y", Some(VariableValue::Boolean(true))));
        assert!(store.set("acme.a", next, "y", None));
        // The same value again changes nothing (and is not announced).
        assert_eq!(
            store.update("acme.a", next, "x", Some(VariableValue::Integer(3))),
            Some(false)
        );
        assert_eq!(store.update("acme.a", next, "y", None), Some(false));
        assert_eq!(
            store.update("acme.a", next, "x", Some(VariableValue::Integer(4))),
            Some(true)
        );
        assert_eq!(
            store.update("acme.a", generation, "x", Some(VariableValue::Integer(5))),
            None
        );
        assert_eq!(store.clear("acme.a", next), Some(vec!["x".to_owned()]));
        assert_eq!(store.forget_all().len(), 1);
    }

    #[test]
    fn values_must_have_the_declared_type() {
        use serde_json::json;
        let count = variable("count", "integer", "machine");
        assert_eq!(
            typed(&count, &json!(42)),
            Ok(Some(VariableValue::Integer(42)))
        );
        assert_eq!(typed(&count, &json!(null)), Ok(None));
        for wrong in [json!("42"), json!(4.2), json!(true), json!([1])] {
            assert!(typed(&count, &wrong).is_err(), "{wrong}");
        }
        let text = variable("status", "string", "machine");
        assert!(typed(&text, &json!(1)).is_err());
        let flag = variable("online", "boolean", "machine");
        assert_eq!(
            typed(&flag, &json!(false)),
            Ok(Some(VariableValue::Boolean(false)))
        );
        let any = variable("raw", "json", "machine");
        assert_eq!(
            typed(&any, &json!({"a": 1})),
            Ok(Some(VariableValue::Json(json!({"a": 1}))))
        );
    }

    #[test]
    fn declarations_are_read_only_in_the_plugins_namespace() {
        let declared = declarations(
            "plugins.acme.sdkTest",
            &[
                variable("status", "string", "machine"),
                variable("per.project", "json", "project"),
            ],
        );
        assert_eq!(declared.len(), 2);
        assert_eq!(declared[0].id.as_str(), "plugins.acme.sdkTest.status");
        assert_eq!(declared[0].scope, DeclaredScope::Machine);
        assert!(declared.iter().all(|d| d.read_only));
        assert_eq!(declared[1].value_type, VariableType::Json);
        assert_eq!(declared[1].scope, DeclaredScope::Project);
    }

    fn host_variable(id: &str, source: &str) -> Variable {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": id, "valueType": {"type": "string"}, "scope": "project",
            "source": source
        }))
        .unwrap()
    }

    /// X7: the host's value, the project in the context, only while the
    /// plugin is enabled and holds `projectContext`; what the plugin
    /// published never stands in for it.
    #[test]
    fn the_hosts_project_id_is_an_identifier_never_shown_on_a_key() {
        let declared = declarations(
            "plugins.deckpilot.projects",
            &[
                host_variable("current.id", "host:project.id"),
                host_variable("current.key", "host:project.key"),
                variable("other", "string", "machine"),
            ],
        );
        let flags: Vec<(&str, bool)> = declared
            .iter()
            .map(|d| (d.id.name(), d.identifier))
            .collect();
        assert_eq!(
            flags,
            [
                ("deckpilot.projects.current.id", true),
                ("deckpilot.projects.current.key", false),
                ("deckpilot.projects.other", false),
            ]
        );
    }

    #[test]
    fn host_variables_have_the_contexts_project_only_while_the_plugin_may_have_them() {
        use deckpilot_core::variables::{NoLiveState, ProjectSource, ResolutionContext};
        use deckpilot_core::{Os, Profile, Project, ProjectKey};

        let mut profile = Profile::new("Work");
        profile
            .projects
            .push(Project::new(ProjectKey::new("SHOP").unwrap(), "Shop"));
        let shop = profile.projects[0].id;
        let variables = [
            host_variable("current.id", "host:project.id"),
            host_variable("current.key", "host:project.key"),
            host_variable("current.name", "host:project.name"),
        ];
        let namespace = "plugins.deckpilot.projects";
        let declared = declarations(namespace, &variables);
        assert!(
            declared
                .iter()
                .all(|d| d.requires == vec![Requirement::Project] && d.read_only)
        );
        let store = PublishedVariables::default();
        // Even a value the plugin got into the store is not used.
        let generation = store.generation("deckpilot.projects");
        assert!(store.set(
            "deckpilot.projects",
            generation,
            "current.key",
            Some(VariableValue::String("FORGED".into()))
        ));
        let source = |active| {
            PluginVariableSource::new("deckpilot.projects".into(), namespace.into(), store.clone())
                .with_host_context(&variables, active)
        };
        let with_project = ResolutionContext::preview(profile.id)
            .with_project(Some(shop), ProjectSource::Application);
        let without = ResolutionContext::preview(profile.id);
        let resolve = |source: &PluginVariableSource, context, index: usize| {
            source.resolve(
                &declared[index],
                &VariableEnvironment {
                    context,
                    profile: &profile,
                    os: Os::MacOs,
                    live: &NoLiveState,
                    machine: None,
                    private_paths: true,
                },
            )
        };
        let value = |text: &str| Resolution::Value {
            value: VariableValue::String(text.into()),
            source: ValueSource::System,
            sensitive: false,
        };
        let enabled = source(true);
        assert_eq!(
            resolve(&enabled, &with_project, 0),
            value(&shop.to_string())
        );
        assert_eq!(resolve(&enabled, &with_project, 1), value("SHOP"));
        assert_eq!(resolve(&enabled, &with_project, 2), value("Shop"));
        assert_eq!(
            resolve(&enabled, &without, 1),
            Resolution::Missing(MissingReason::NoProject)
        );
        // Disabled, or without `projectContext`: withdrawn.
        let disabled = source(false);
        for index in 0..3 {
            assert_eq!(
                resolve(&disabled, &with_project, index),
                Resolution::Missing(MissingReason::NoValue)
            );
        }
        // Enabled again: back.
        assert_eq!(resolve(&source(true), &with_project, 1), value("SHOP"));
    }

    fn repository_variable(id: &str, source: &str, value_type: &serde_json::Value) -> Variable {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": id, "valueType": value_type, "scope": "project",
            "source": source
        }))
        .unwrap()
    }

    /// Plugin API 1.12: the context's project's repositories, by key, and
    /// its default one, with folders from this computer's bindings only
    /// for a reader that may see paths; never published by the plugin.
    #[test]
    #[allow(clippy::too_many_lines)] // one fixture, every reader
    fn host_repositories_hide_folders_from_readers_without_paths() {
        use deckpilot_core::variables::{NoLiveState, ProjectSource, ResolutionContext};
        use deckpilot_core::{
            MachineConfig, MachineResourceBinding, Os, Profile, Project, ProjectKey, Repository,
            RepositoryId, RepositoryKey, ResourceKey, ResourceKind,
        };

        let variables = [
            repository_variable(
                "current.repositories",
                "host:project.repositories",
                &serde_json::json!({"type": "map", "of": "repository"}),
            ),
            repository_variable(
                "current.repository",
                "host:project.repository",
                &serde_json::json!({"type": "object", "shape": "repository"}),
            ),
        ];
        let namespace = "plugins.deckpilot.projects";
        let declared = declarations(namespace, &variables);
        assert_eq!(
            declared[0].value_type,
            VariableType::Map {
                of: Shape::Repository
            }
        );
        assert_eq!(
            declared[1].value_type,
            VariableType::Object {
                shape: Shape::Repository
            }
        );
        assert!(
            declared
                .iter()
                .all(|d| d.requires == vec![Requirement::Project])
        );
        // Never the plugin's to publish.
        for variable in &variables {
            assert!(typed(variable, &serde_json::json!({"cms": {}})).is_err());
        }

        let mut profile = Profile::new("Work");
        let mut shop = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
        let make = |key: &str| Repository {
            id: RepositoryId::new(),
            key: RepositoryKey::new(key).unwrap(),
            name: key.to_uppercase(),
            resource: ResourceKey::new(format!("shop.{key}")).unwrap(),
            description: None,
            remotes: Vec::new(),
        };
        shop.repositories = vec![make("cms"), make("api")];
        profile.projects.push(shop);
        let mut machine = MachineConfig::new(Os::current());
        let folder = std::env::temp_dir().to_string_lossy().into_owned();
        machine.set_binding(MachineResourceBinding {
            key: ResourceKey::new("shop.cms").unwrap(),
            kind: ResourceKind::Directory,
            value: folder.clone(),
        });
        let context = ResolutionContext::preview(profile.id)
            .with_project(Some(profile.projects[0].id), ProjectSource::Application);
        let source = PluginVariableSource::new(
            "deckpilot.projects".into(),
            namespace.into(),
            PublishedVariables::default(),
        )
        .with_host_context(&variables, true);
        let resolve = |index: usize, private_paths: bool| {
            source.resolve(
                &declared[index],
                &VariableEnvironment {
                    context: &context,
                    profile: &profile,
                    os: Os::current(),
                    live: &NoLiveState,
                    machine: Some(&machine),
                    private_paths,
                },
            )
        };
        let json = |resolution: Resolution| match resolution {
            Resolution::Value { value, .. } => value.structured_json().unwrap(),
            other => panic!("{other:?}"),
        };
        let shown = json(resolve(0, true));
        assert_eq!(shown["cms"]["path"], serde_json::json!(folder));
        assert_eq!(shown["cms"]["exists"], serde_json::json!(true));
        assert_eq!(shown["api"]["path"], serde_json::Value::Null);
        let hidden = json(resolve(0, false));
        assert_eq!(hidden["cms"]["path"], serde_json::Value::Null);
        assert_eq!(hidden["cms"]["exists"], serde_json::json!(true));
        let quoted = serde_json::to_string(&folder).unwrap();
        assert!(!hidden.to_string().contains(&quoted[1..quoted.len() - 1]));
        // Two repositories and no default: no value (OD-3).
        assert_eq!(
            resolve(1, true),
            Resolution::Missing(MissingReason::NoDefaultRepository)
        );
        // Without `projectContext`, or disabled: withdrawn.
        let withdrawn = PluginVariableSource::new(
            "deckpilot.projects".into(),
            namespace.into(),
            PublishedVariables::default(),
        )
        .with_host_context(&variables, false);
        assert_eq!(
            withdrawn.resolve(
                &declared[0],
                &VariableEnvironment {
                    context: &context,
                    profile: &profile,
                    os: Os::current(),
                    live: &NoLiveState,
                    machine: Some(&machine),
                    private_paths: true,
                },
            ),
            Resolution::Missing(MissingReason::NoValue)
        );
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

    fn repository_variables() -> [Variable; 2] {
        [
            repository_variable(
                "current.repositories",
                "host:project.repositories",
                &serde_json::json!({"type": "map", "of": "repository"}),
            ),
            repository_variable(
                "current.repository",
                "host:project.repository",
                &serde_json::json!({"type": "object", "shape": "repository"}),
            ),
        ]
    }

    /// A profile whose one project has `count` repositories with the
    /// longest keys and names, each bound to an existing folder; the first
    /// is the default.
    fn many_repositories(count: usize) -> (deckpilot_core::Profile, deckpilot_core::MachineConfig) {
        use deckpilot_core::project::{MAX_REPOSITORY_KEY_CHARS, MAX_REPOSITORY_NAME_CHARS};
        use deckpilot_core::{
            MachineConfig, MachineResourceBinding, Os, Profile, Project, ProjectKey, Repository,
            RepositoryId, RepositoryKey, ResourceKey, ResourceKind,
        };
        let mut profile = Profile::new("Work");
        let mut project = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
        let mut machine = MachineConfig::new(Os::current());
        let folder = std::env::temp_dir().to_string_lossy().into_owned();
        for index in 0..count {
            let mut key = format!("r{index:03}_");
            key.push_str(&"k".repeat(MAX_REPOSITORY_KEY_CHARS - key.len()));
            let resource = ResourceKey::new(format!("shop.{key}")).unwrap();
            machine.set_binding(MachineResourceBinding {
                key: resource.clone(),
                kind: ResourceKind::Directory,
                value: folder.clone(),
            });
            project.repositories.push(Repository {
                id: RepositoryId::new(),
                key: RepositoryKey::new(key).unwrap(),
                name: format!("{index:03} {}", "N".repeat(MAX_REPOSITORY_NAME_CHARS - 4)),
                resource,
                description: Some("d".repeat(200)),
                remotes: Vec::new(),
            });
        }
        project.default_repository = project.repositories.first().map(|r| r.id);
        profile.projects.push(project);
        (profile, machine)
    }

    /// A member read builds only what it needs (review of #460): one
    /// repository, and its folder looked at only for `exists`, so a key
    /// title never stalls on an unmounted network volume.
    #[test]
    fn a_member_read_looks_at_no_folder_but_for_exists() {
        use deckpilot_core::variables::Shape;
        use deckpilot_core::variables::{
            NoLiveState, ProjectSource, ResolutionContext, VariableRegistry,
        };
        use deckpilot_core::{ModuleId, Os};

        let (profile, machine) = many_repositories(8);
        let variables = repository_variables();
        let namespace = "plugins.deckpilot.projects";
        let probe = Arc::new(CountingProbe::default());
        let source = PluginVariableSource::new(
            "deckpilot.projects".into(),
            namespace.into(),
            PublishedVariables::default(),
        )
        .with_host_context(&variables, true)
        .with_probe(probe.clone());
        let mut registry = VariableRegistry::new();
        registry
            .register_module(
                &ModuleId::new("deckpilot.projects").unwrap(),
                &declarations(namespace, &variables),
                Some(Arc::new(source)),
            )
            .unwrap();
        let context = ResolutionContext::preview(profile.id)
            .with_project(Some(profile.projects[0].id), ProjectSource::Application);
        let key = profile.projects[0].repositories[3].key.as_str().to_owned();
        for private_paths in [true, false] {
            let environment = VariableEnvironment {
                context: &context,
                profile: &profile,
                os: Os::current(),
                live: &NoLiveState,
                machine: Some(&machine),
                private_paths,
            };
            let resolve = |id: &str| {
                registry.resolve(
                    &VariableId::reference(format!("{namespace}.{id}")).unwrap(),
                    &environment,
                )
            };
            for member in Shape::Repository.members() {
                let path = member.path;
                let expected = usize::from(path == "exists");
                let _ = resolve(&format!("current.repositories.{key}.{path}"));
                assert_eq!(probe.take(), expected, "repositories.{key}.{path}");
                let _ = resolve(&format!("current.repository.{path}"));
                assert_eq!(probe.take(), expected, "repository.{path}");
            }
            assert!(matches!(
                resolve(&format!("current.repositories.{key}.name")),
                Resolution::Value { value: VariableValue::String(name), .. }
                    if name == profile.projects[0].repositories[3].name
            ));
            // The whole map looks at every folder once; one entry at its own.
            let _ = resolve("current.repositories");
            assert_eq!(probe.take(), 8);
            let _ = resolve(&format!("current.repositories.{key}"));
            assert_eq!(probe.take(), 1);
        }
    }

    /// Both catalogs of a project with the most repositories, the longest
    /// keys and names, fit one IPC message (review of #460: one entry per
    /// member of each repository did not, from about a dozen on, and the
    /// whole picker came back empty). The app composes every member entry.
    #[test]
    fn the_catalogs_of_the_most_repositories_fit_one_message() {
        use deckpilot_core::ModuleId;
        use deckpilot_core::project::MAX_REPOSITORIES_PER_PROJECT;
        use deckpilot_core::variables::Shape;
        use deckpilot_core::variables::{
            CatalogFilter, ProjectSource, ResolutionContext, VariableRegistry, catalog_listing,
            display_context, display_listing, expand_members,
        };
        use deckpilot_ipc::framing::MAX_MESSAGE_BYTES;
        use deckpilot_ipc::{Response, ServerMessage};

        let wire = |entries: Vec<deckpilot_core::variables::CatalogEntry>| {
            serde_json::to_vec(&ServerMessage::Response {
                id: u64::MAX,
                response: Response::Variables { entries },
            })
            .unwrap()
            .len()
        };
        let (profile, _) = many_repositories(MAX_REPOSITORIES_PER_PROJECT);
        let project = &profile.projects[0];
        let variables = repository_variables();
        let namespace = "plugins.deckpilot.projects";
        let mut registry = VariableRegistry::new();
        registry
            .register_module(
                &ModuleId::new("deckpilot.projects").unwrap(),
                &declarations(namespace, &variables),
                Some(Arc::new(
                    PluginVariableSource::new(
                        "deckpilot.projects".into(),
                        namespace.into(),
                        PublishedVariables::default(),
                    )
                    .with_host_context(&variables, true),
                )),
            )
            .unwrap();
        let all = CatalogFilter::default();
        let members = Shape::Repository.members().len();

        // `listVariables`, as `Agent::list_variables` answers it.
        let context = ResolutionContext::preview(profile.id)
            .with_project(Some(project.id), ProjectSource::Application);
        let listed = catalog_listing(registry.catalog(&context, false), Some(project), &all);
        let size = wire(listed.clone());
        assert!(size <= MAX_MESSAGE_BYTES, "listVariables: {size} bytes");
        let expanded = expand_members(listed, &all);
        let composed = expanded.iter().filter(|e| e.member_of.is_some()).count();
        assert_eq!(composed, (MAX_REPOSITORIES_PER_PROJECT + 1) * members);
        // One entry per member, as before, would not have fitted.
        assert!(wire(expanded) > MAX_MESSAGE_BYTES);

        // `listDisplayVariables`, as `Agent::list_display_variables`.
        let context = display_context(profile.id, Some(project.id));
        let listed = display_listing(
            registry.catalog(&context, false),
            Some(project),
            &registry,
            &all,
        );
        let size = wire(listed.clone());
        assert!(
            size <= MAX_MESSAGE_BYTES,
            "listDisplayVariables: {size} bytes"
        );
        let safe = Shape::Repository
            .members()
            .iter()
            .filter(|m| m.display_safe)
            .count();
        let expanded = expand_members(listed, &all);
        assert!(expanded.iter().all(|e| !e.value_type.is_structured()));
        assert_eq!(
            expanded.iter().filter(|e| e.member_of.is_some()).count(),
            (MAX_REPOSITORIES_PER_PROJECT + 1) * safe
        );
        assert!(expanded.iter().all(|e| {
            let reference = e.reference.to_string();
            let member = reference.rsplit('.').next().unwrap_or_default();
            member != "path" && member != "id" && member != "url"
        }));
    }
}
