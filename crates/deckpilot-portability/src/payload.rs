//! The decrypted content of an export.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use deckpilot_core::variables::{ScopeKey, VariableDefinition, VariableKey, VariableValue};
use deckpilot_core::{
    ActionParameters, ModuleRegistry, ParameterValue, Profile, ProfileId, ResourceKey,
    ResourceKind, ResourceRef,
};
use deckpilot_storage::crypto::{KdfParams, Password};
use deckpilot_storage::{ConfigStore, SqliteStore};
use deckpilot_update::policy::{PREFERENCES_SETTING, UpdateMode, UpdatePreferences};

use crate::{PortabilityError, archive};

/// Newest version of [`Payload`] this build reads and writes. Version 2
/// adds shared variables and may carry profile documents of document
/// version 5 (references, inline invocations); a DeckPilot that reads only
/// version 1 refuses it instead of silently dropping what it does not
/// know. Version 3 adds [`Payload::library`], layouts no exported profile
/// uses; an export without such layouts is written as version 2, which
/// releases up to 0.8.1 read (profiles carry their layouts, as before).
/// Version 4 marks an export whose layouts have key labels
/// (`KeyAppearance.labels`): a release that reads at most 3 refuses it
/// instead of importing the keys without their labels. An export without
/// labels is written as version 2 or 3, as before.
/// Version 5 marks an export with project repositories (`repositories`,
/// `defaultRepository`) other than the one the `repository` role stands
/// for: a release that reads at most 4 refuses it instead of dropping the
/// repositories. An export whose projects have only that derived
/// repository leaves it out (the importer derives it again) and is
/// written as before.
pub const PAYLOAD_VERSION: u32 = 5;

/// The version of an export without repositories whose layouts have key
/// labels.
pub const PAYLOAD_VERSION_WITH_LABELS: u32 = 4;

/// The version of an export without key labels whose
/// [`Payload::library`] is not empty.
pub const PAYLOAD_VERSION_WITH_LIBRARY: u32 = 3;

/// The version of an export without key labels whose
/// [`Payload::library`] is empty.
pub const PAYLOAD_VERSION_WITHOUT_LIBRARY: u32 = 2;

/// The version an export is written as: the oldest that holds it.
pub fn payload_version(profiles: &[Profile], library: &LibraryExport) -> u32 {
    let labelled = |layouts: &[deckpilot_core::DeviceLayout]| {
        layouts.iter().any(|layout| {
            layout
                .pages
                .iter()
                .flat_map(|page| &page.bindings)
                .chain(layout.recovered_bindings())
                .any(|binding| {
                    binding
                        .appearance
                        .as_ref()
                        .is_some_and(|look| !look.labels.is_empty())
                })
        })
    };
    let with_repositories = profiles
        .iter()
        .flat_map(|profile| &profile.projects)
        .any(|project| !project.repositories.is_empty() || project.default_repository.is_some());
    if with_repositories {
        PAYLOAD_VERSION
    } else if profiles.iter().any(|profile| labelled(&profile.layouts))
        || labelled(&library.layouts)
    {
        PAYLOAD_VERSION_WITH_LABELS
    } else if library.is_empty() {
        PAYLOAD_VERSION_WITHOUT_LIBRARY
    } else {
        PAYLOAD_VERSION_WITH_LIBRARY
    }
}

/// Portable settings of one module, in the module's own export format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleExport {
    /// The module's [`Portability::version`](deckpilot_core::Portability).
    pub version: u32,
    pub enabled: bool,
    #[serde(default)]
    pub settings: ActionParameters,
}

/// Application settings that mean the same on every computer. Settings of
/// this computer (resource bindings, security policy, brightness) and
/// runtime state (time of the last update check, confirmations) are never
/// exported: importing a security policy could silently weaken another
/// computer's protection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updates: Option<UpdateSettings>,
}

impl AppSettings {
    pub fn is_empty(&self) -> bool {
        self.updates.is_none()
    }
}

/// How updates are checked for and applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettings {
    pub mode: UpdateMode,
    pub automatic_checks: bool,
    pub check_interval_hours: u32,
}

/// Everything an export carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Payload {
    pub version: u32,
    /// Seconds since the Unix epoch.
    pub exported_at: i64,
    pub app_version: String,
    pub profiles: Vec<Profile>,
    /// Keyed by module id. Kept as text so that modules this build does not
    /// know are reported instead of failing the whole import.
    #[serde(default)]
    pub modules: BTreeMap<String, ModuleExport>,
    /// Logical resources the profiles use, with their kind where known.
    /// Machine-specific values are never exported.
    #[serde(default)]
    pub resources: BTreeMap<ResourceKey, Option<ResourceKind>>,
    /// Portable application settings; absent in exports of earlier builds.
    #[serde(default)]
    pub settings: AppSettings,
    /// Shared variables; absent in exports of earlier builds.
    #[serde(default, skip_serializing_if = "VariablesExport::is_empty")]
    pub variables: VariablesExport,
    /// Layouts no exported profile uses (an export of everything), with
    /// the actions and icons they use. Each profile carries the layouts it
    /// uses, in its order, with its default per model (`activeLayouts`);
    /// a layout two exported profiles use is in both with the same id and
    /// is shared again when both are imported. Payload version 3.
    #[serde(default, skip_serializing_if = "LibraryExport::is_empty")]
    pub library: LibraryExport,
}

/// Layouts of no profile in an export ([`Payload::library`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryExport {
    #[serde(default)]
    pub layouts: Vec<deckpilot_core::DeviceLayout>,
    #[serde(default)]
    pub actions: Vec<deckpilot_core::ActionInstance>,
    #[serde(default)]
    pub icons: Vec<deckpilot_core::Icon>,
}

impl LibraryExport {
    pub fn is_empty(&self) -> bool {
        self.layouts.is_empty() && self.actions.is_empty() && self.icons.is_empty()
    }
}

/// Shared variables in an export (decision D5): the user's definitions and
/// the values of the exported profiles and their projects; global values
/// only together with the application settings. Values of sensitive
/// variables — and their defaults — are never exported. A `secretRef`
/// value is a credential name; the credential itself is never exported.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariablesExport {
    #[serde(default)]
    pub definitions: Vec<VariableDefinition>,
    #[serde(default)]
    pub values: Vec<ExportedValue>,
}

impl VariablesExport {
    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty() && self.values.is_empty()
    }
}

/// One exported value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedValue {
    pub key: VariableKey,
    pub scope: ScopeKey,
    pub value: VariableValue,
}

/// The shared variables to export with `profiles`.
fn export_variables(
    store: &SqliteStore,
    profiles: &[Profile],
    global_values: bool,
) -> Result<VariablesExport, PortabilityError> {
    let stored = store.shared_variables()?;
    // Values are exported only for variables this build can read and that
    // are not sensitive; module variables travel as values only (the
    // module declares the definition on the other computer).
    let exportable: BTreeSet<VariableKey> = stored
        .iter()
        .filter(|s| !s.definition.sensitive)
        .map(|s| s.definition.key.clone())
        .collect();
    let definitions = stored
        .into_iter()
        .filter(|s| s.definition.module.is_none())
        .map(|s| {
            let mut definition = s.definition;
            if definition.sensitive {
                definition.default = None;
            }
            definition
        })
        .collect();
    let mut values = Vec::new();
    let mut add = |stored: Vec<deckpilot_storage::StoredValue>| {
        values.extend(
            stored
                .into_iter()
                .filter(|v| exportable.contains(&v.key))
                .map(|v| ExportedValue {
                    key: v.key,
                    scope: v.scope,
                    value: v.value,
                }),
        );
    };
    if global_values {
        add(store.global_variable_values()?);
    }
    for profile in profiles {
        add(store.profile_variable_values(profile.id)?);
    }
    Ok(VariablesExport {
        definitions,
        values,
    })
}

/// The projects of `profile` as an export carries them: a project whose
/// only repository is the one its `repository` role stands for (and is its
/// default) leaves it out, since the importer derives the same one again;
/// remotes leave out `connectionId`, which only means something on this
/// computer. Paths are never in a project (they are machine bindings).
pub fn portable_projects(profile: &mut Profile) {
    for project in &mut profile.projects {
        let derived = project.derived_repository();
        if derived.is_some_and(|derived| {
            project.repositories == [derived.clone()]
                && project.default_repository == Some(derived.id)
        }) {
            project.repositories.clear();
            project.default_repository = None;
        }
        for repository in &mut project.repositories {
            for remote in &mut repository.remotes {
                *remote = remote.portable();
            }
        }
    }
}

/// Resource keys of the repositories of `profile` (with the derived one).
fn repository_keys(profile: &Profile) -> BTreeSet<ResourceKey> {
    profile
        .projects
        .iter()
        .flat_map(|project| {
            project
                .effective_repositories()
                .iter()
                .map(|r| r.resource.clone())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Resource keys a profile refers to: project resources, the folders of
/// its repositories, and fixed resource parameters (including step
/// overrides).
pub fn resource_keys(profile: &Profile) -> BTreeSet<ResourceKey> {
    let mut keys: BTreeSet<ResourceKey> = profile
        .projects
        .iter()
        .flat_map(|project| project.resources.values().cloned())
        .collect();
    keys.extend(repository_keys(profile));
    let fixed = |parameters: &ActionParameters| -> Vec<ResourceKey> {
        parameters
            .iter()
            .filter_map(|(_, value)| match value {
                ParameterValue::Resource(ResourceRef::Fixed { key }) => Some(key.clone()),
                _ => None,
            })
            .collect()
    };
    for action in &profile.actions {
        keys.extend(fixed(&action.parameters));
    }
    for step in profile.workflows.iter().flat_map(|w| &w.steps) {
        keys.extend(fixed(&step.overrides));
    }
    keys
}

/// Collects the portable configuration. `profiles`: the profiles to export,
/// all when `None`. `include_modules`: export module settings.
/// What an export contains besides the selected profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportOptions {
    pub module_settings: bool,
    pub app_settings: bool,
}

/// The portable application settings stored in `store`.
pub fn app_settings(store: &SqliteStore) -> Result<AppSettings, PortabilityError> {
    let updates = store
        .setting(PREFERENCES_SETTING)?
        .and_then(|text| serde_json::from_str::<UpdatePreferences>(&text).ok())
        .map(|preferences| {
            let preferences = preferences.sanitized();
            UpdateSettings {
                mode: preferences.mode,
                automatic_checks: preferences.automatic_checks,
                check_interval_hours: preferences.check_interval_hours,
            }
        });
    Ok(AppSettings { updates })
}

pub fn export(
    store: &SqliteStore,
    modules: &ModuleRegistry,
    profiles: Option<&[ProfileId]>,
    options: ExportOptions,
    exported_at: i64,
    app_version: &str,
) -> Result<Payload, PortabilityError> {
    let include_modules = options.module_settings;
    let mut selected = Vec::new();
    for summary in store.list_profiles()? {
        if profiles.is_some_and(|wanted| !wanted.contains(&summary.id)) {
            continue;
        }
        if let Some(mut profile) = store.load_profile(summary.id)? {
            portable_projects(&mut profile);
            selected.push(profile);
        }
    }
    if let Some(wanted) = profiles
        && wanted.len() != selected.len()
    {
        return Err(PortabilityError::Invalid(
            "a selected profile does not exist".into(),
        ));
    }
    let machine = store.load_machine_config()?;
    let mut resources = BTreeMap::new();
    let folders: BTreeSet<ResourceKey> = selected.iter().flat_map(repository_keys).collect();
    for key in selected.iter().flat_map(resource_keys) {
        // A repository's resource is a folder, also where it is not bound.
        let kind = machine
            .as_ref()
            .and_then(|machine| machine.binding(&key))
            .map(|binding| binding.kind)
            .or_else(|| folders.contains(&key).then_some(ResourceKind::Directory));
        resources.insert(key, kind);
    }
    let mut exported_modules = BTreeMap::new();
    if include_modules {
        for info in modules.modules() {
            if let Some((version, enabled, settings)) = modules.export_settings(&info.manifest.id) {
                exported_modules.insert(
                    info.manifest.id.to_string(),
                    ModuleExport {
                        version,
                        enabled,
                        settings,
                    },
                );
            }
        }
    }
    let variables = export_variables(store, &selected, options.app_settings)?;
    // Everything is exported: also the layouts no profile uses.
    let mut library = LibraryExport::default();
    if profiles.is_none() {
        for stored in store.shared_layouts()? {
            if !stored.profiles.is_empty() {
                continue;
            }
            let (actions, icons) = store.layout_library(&stored.layout)?;
            for action in actions {
                if !library.actions.iter().any(|a| a.id == action.id) {
                    library.actions.push(action);
                }
            }
            for icon in icons {
                if !library.icons.iter().any(|i| i.id == icon.id) {
                    library.icons.push(icon);
                }
            }
            library.layouts.push(stored.layout);
        }
    }
    Ok(Payload {
        variables,
        version: payload_version(&selected, &library),
        library,
        exported_at,
        app_version: app_version.into(),
        profiles: selected,
        modules: exported_modules,
        resources,
        settings: if options.app_settings {
            app_settings(store)?
        } else {
            AppSettings::default()
        },
    })
}

/// Encrypts a payload into archive bytes.
pub fn seal(
    payload: &Payload,
    password: &Password,
    kdf: KdfParams,
) -> Result<Vec<u8>, PortabilityError> {
    let plaintext = zeroize::Zeroizing::new(serde_json::to_vec(payload)?);
    archive::seal(
        &plaintext,
        password,
        kdf,
        payload.exported_at,
        &payload.app_version,
    )
}

/// Decrypts archive bytes and parses the payload.
pub fn open(bytes: &[u8], password: &Password) -> Result<Payload, PortabilityError> {
    let (_, plaintext) = archive::open(bytes, password)?;
    let payload: Payload = serde_json::from_slice(&plaintext)?;
    if payload.version == 0 || payload.version > PAYLOAD_VERSION {
        return Err(PortabilityError::UnsupportedVersion(payload.version));
    }
    Ok(payload)
}
