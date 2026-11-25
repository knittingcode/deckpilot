//! What the variable picker lists: metadata of every reference available
//! in a context. Entries never carry values; values come from previews.

use serde::{Deserialize, Serialize};

use super::registry::{DeclaredScope, Deprecation, VariableOwner};
use super::{Shape, VariableRef, VariableScope, VariableType};
use crate::{ModuleId, ParameterKind, Project};

/// The picker's groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VariableGroup {
    /// Shared variables with a global value or default.
    Shared,
    /// System variables (`system.…`).
    System,
    /// Shared variables set for the context's profile.
    Profile,
    /// Shared variables set for the context's project.
    Project,
    /// The running workflow: `workflow.…`, its inputs and variables.
    Workflow,
    /// Outputs of earlier nodes.
    StepOutput,
    /// Variables a module declares (since protocol 9).
    Module,
}

/// Whether an entry can have a value in the context it was listed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReferenceAvailability {
    Available,
    /// Has a value only while the workflow runs.
    RuntimeOnly,
    /// Not in this context, e.g. a project variable without a project.
    NotInContext,
    /// Its owner cannot provide it: the module is disabled, unsupported or
    /// not installed (since protocol 9; older clients see `notInContext`).
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)] // independent wire flags, each optional
pub struct CatalogEntry {
    pub reference: VariableRef,
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub group: VariableGroup,
    pub value_type: VariableType,
    /// For shared variables: the scope whose value applies in the context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<VariableScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<ModuleId>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sensitive: bool,
    /// The value depends on the context or changes while running.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dynamic: bool,
    pub availability: ReferenceAvailability,
    /// Who owns the variable (since protocol 9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<VariableOwner>,
    /// Core and module variables cannot be changed (since protocol 9).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
    /// An older id kept as an alias; the picker inserts the canonical id
    /// instead (since protocol 9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<Deprecation>,
    /// For core and module variables: what the value depends on (since
    /// protocol 9). Shared variables have [`CatalogEntry::scope`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_scope: Option<DeclaredScope>,
    /// For a structured variable: the scalar members of one object of its
    /// shape (since protocol 18), those a client may offer (the title
    /// picker's catalog lists the display-safe ones only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<CatalogMember>,
    /// For a map variable: the keys of its objects in the context's
    /// project, in the user's order (since protocol 18). A client composes
    /// the reference of each member from them, `<reference>.<key>.<path>`
    /// ([`expand_members`]): the agent never lists one entry per member, so
    /// a catalog grows by one key per repository, not by an entry per
    /// member of each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub map_keys: Vec<String>,
    /// Listed for its members only: the variable itself is not offered
    /// (the title picker never offers a structured value). Since protocol
    /// 18; older clients do not receive such entries.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub members_only: bool,
    /// For the entry of one member of a structured variable
    /// (`….repositories.cms.name`), as a client composes it
    /// ([`expand_members`]): that variable's reference. The agent never
    /// sends such entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_of: Option<String>,
}

/// One scalar member of a shape, as the picker describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogMember {
    /// `name`, `remote.url`.
    pub path: String,
    pub label: String,
    pub value_type: VariableType,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub nullable: bool,
    /// May be shown on a key ([`super::Member::display_safe`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub display_safe: bool,
}

impl CatalogMember {
    /// The members of `shape`.
    pub fn of(shape: Shape) -> Vec<Self> {
        shape
            .members()
            .iter()
            .map(|member| Self {
                path: member.path.to_owned(),
                label: member.label.to_owned(),
                value_type: member.value_type,
                nullable: member.nullable,
                display_safe: member.display_safe,
            })
            .collect()
    }
}

/// The keys of the objects a map of `shape` has in `project`, in order:
/// the project's repositories.
fn map_keys(shape: Shape, project: Option<&Project>) -> Vec<String> {
    match shape {
        Shape::Repository => project
            .map(|project| {
                project
                    .effective_repositories()
                    .iter()
                    .map(|repository| repository.key.as_str().to_owned())
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// `entries` with each structured module variable's shape: its `members`
/// and, for a map, the keys of `project` (the context's project) in
/// `map_keys`. Never one entry per member: a client composes them
/// ([`expand_members`]), so that a catalog of a project with many
/// repositories still fits one message.
pub fn with_members(entries: Vec<CatalogEntry>, project: Option<&Project>) -> Vec<CatalogEntry> {
    entries
        .into_iter()
        .map(|entry| {
            if !matches!(entry.reference, VariableRef::Module(_)) {
                return entry;
            }
            match entry.value_type {
                VariableType::Object { shape } => CatalogEntry {
                    members: CatalogMember::of(shape),
                    ..entry
                },
                VariableType::Map { of } => CatalogEntry {
                    members: CatalogMember::of(of),
                    map_keys: map_keys(of, project),
                    ..entry
                },
                _ => entry,
            }
        })
        .collect()
}

/// What `listVariables` answers: `entries` ([`with_members`] of
/// `project`) that match `filter`, a structured variable also when one of
/// its members does.
pub fn catalog_listing(
    entries: Vec<CatalogEntry>,
    project: Option<&Project>,
    filter: &CatalogFilter,
) -> Vec<CatalogEntry> {
    with_members(entries, project)
        .into_iter()
        .filter(|entry| filter.matches_listed(entry))
        .collect()
}

/// `entries` as a client lists them: after each structured variable, one
/// entry per scalar member it lists (`….repository.name`), and for a map
/// one per key (`….repositories.cms.name`), each naming its variable in
/// `member_of`; a variable listed for its members only is left out. The
/// structured variable and its members are kept only when they match
/// `filter` (the agent kept the variable when it or one of its members
/// does); every other entry is kept as the agent listed it.
pub fn expand_members(entries: Vec<CatalogEntry>, filter: &CatalogFilter) -> Vec<CatalogEntry> {
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.members.is_empty() || !entry.value_type.is_structured() {
            out.push(entry);
            continue;
        }
        let members = entry.member_entries();
        if !entry.members_only && filter.matches(&entry) {
            out.push(entry);
        }
        out.extend(members.into_iter().filter(|member| filter.matches(member)));
    }
    out
}

impl CatalogEntry {
    /// The entries of the members this structured variable lists, as a
    /// client composes them ([`expand_members`]): none for another entry.
    pub fn member_entries(&self) -> Vec<CatalogEntry> {
        let prefixes: Vec<(String, String)> = match self.value_type {
            VariableType::Object { .. } => vec![(self.reference.to_string(), self.label.clone())],
            VariableType::Map { .. } => self
                .map_keys
                .iter()
                .map(|key| {
                    (
                        format!("{}.{key}", self.reference),
                        format!("{} › {key}", self.label),
                    )
                })
                .collect(),
            _ => Vec::new(),
        };
        self.members
            .iter()
            .flat_map(|member| {
                prefixes.iter().filter_map(move |(prefix, label)| {
                    let reference =
                        VariableRef::parse(&format!("{prefix}.{}", member.path)).ok()?;
                    Some(CatalogEntry {
                        reference,
                        label: format!("{label} › {}", member.label),
                        description: String::new(),
                        value_type: member.value_type,
                        dynamic: true,
                        deprecated: None,
                        members: Vec::new(),
                        map_keys: Vec::new(),
                        members_only: false,
                        member_of: Some(self.reference.to_string()),
                        ..self.clone()
                    })
                })
            })
            .collect()
    }
}

impl CatalogEntry {
    /// The entry as a client of `protocol` before 18 knows it: structured
    /// types are `json` without members or keys, and an entry listed for
    /// its members only (or a member's) is left out.
    pub fn for_protocol(self, protocol: u32) -> Option<Self> {
        if protocol >= 18 {
            return Some(self);
        }
        if self.member_of.is_some() || self.members_only {
            return None;
        }
        Some(Self {
            value_type: self.value_type.for_protocol(protocol),
            members: Vec::new(),
            map_keys: Vec::new(),
            ..self
        })
    }

    /// The entry as a client before protocol 9 knows it, or `None` if such
    /// a client could not read it (a module root).
    pub fn for_protocol_8(self) -> Option<Self> {
        if matches!(self.reference, VariableRef::Module(_)) || self.group == VariableGroup::Module {
            return None;
        }
        Some(Self {
            availability: match self.availability {
                ReferenceAvailability::Unavailable => ReferenceAvailability::NotInContext,
                other => other,
            },
            owner: None,
            read_only: false,
            deprecated: None,
            declared_scope: None,
            ..self
        })
    }
}

/// What the picker asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CatalogFilter {
    /// Words that must all occur in the reference, label, description,
    /// scope, type or module, ignoring case.
    pub query: String,
    /// Empty: every group.
    pub groups: Vec<VariableGroup>,
    /// Only entries whose type can fill a parameter of this kind.
    pub compatible_with: Option<ParameterKind>,
    /// Also entries that are [`ReferenceAvailability::NotInContext`].
    pub include_unavailable: bool,
}

impl CatalogFilter {
    /// Whether the agent lists `entry`: it matches (unless it is listed
    /// for its members only), or one of the members a client composes
    /// from it does ([`expand_members`]).
    pub fn matches_listed(&self, entry: &CatalogEntry) -> bool {
        (!entry.members_only && self.matches(entry))
            || entry
                .member_entries()
                .iter()
                .any(|member| self.matches(member))
    }

    pub fn matches(&self, entry: &CatalogEntry) -> bool {
        if !self.groups.is_empty() && !self.groups.contains(&entry.group) {
            return false;
        }
        if let Some(kind) = &self.compatible_with
            && !entry.value_type.assignable_to(kind)
        {
            return false;
        }
        if !self.include_unavailable
            && matches!(
                entry.availability,
                ReferenceAvailability::NotInContext | ReferenceAvailability::Unavailable
            )
        {
            return false;
        }
        let haystack = [
            entry.reference.to_string(),
            entry.label.clone(),
            entry.description.clone(),
            entry.scope.map(|s| s.name().to_owned()).unwrap_or_default(),
            entry.value_type.name().to_owned(),
            entry
                .module
                .as_ref()
                .or_else(|| entry.owner.as_ref().and_then(VariableOwner::module))
                .map(ToString::to_string)
                .unwrap_or_default(),
            entry
                .deprecated
                .as_ref()
                .map(|d| d.canonical.to_string())
                .unwrap_or_default(),
        ]
        .join("\n")
        .to_lowercase();
        self.query
            .split_whitespace()
            .all(|word| haystack.contains(&word.to_lowercase()))
    }
}
