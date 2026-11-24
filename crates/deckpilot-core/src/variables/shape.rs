//! Structured values: the closed list of shapes, their members, and member
//! access (`docs/variables.md` §2.8).
//!
//! A variable of a structured type holds an object of one [`Shape`]
//! (`object<repository>`) or an ordered map of them by key
//! (`map<repository>`). Structured values are never text: they never render
//! into a template, a title or a text parameter. A reference reads one
//! scalar member of them (`{{ projects.current.repository.name }}`,
//! `{{ projects.current.repositories.cms.path }}`), or passes the whole
//! value where JSON is expected.
//!
//! The list of shapes is closed and versioned: a new shape is an addition.
//! Only the host produces structured values (a plugin's `variables.publish`
//! refuses them), so a shape is always the host's statement of what the
//! value holds.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::VariableType;
use super::valid_segment;

/// The shapes of structured values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Shape {
    /// One repository of a project (`docs/data-model.md`, Repository).
    Repository,
}

/// One scalar member of a shape: a row of a static table, whose flags are
/// independent facts about the member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct Member {
    /// The member path inside one object, `name` or `remote.url`.
    pub path: &'static str,
    /// `string`, `integer` or `boolean`.
    pub value_type: VariableType,
    /// The member may have no value (`null`).
    pub nullable: bool,
    /// The member may be shown on a key ([`super::display_safe`]). Ids and
    /// paths never are.
    pub display_safe: bool,
    /// An identifier, never text for people.
    pub identifier: bool,
    /// A folder path of this computer: readers that may not see paths get
    /// `null` (a plugin without `resources:paths`, a key title).
    pub private_path: bool,
    /// Shown in the picker.
    pub label: &'static str,
}

const fn member(path: &'static str, value_type: VariableType, label: &'static str) -> Member {
    Member {
        path,
        value_type,
        nullable: false,
        display_safe: true,
        identifier: false,
        private_path: false,
        label,
    }
}

/// The members of [`Shape::Repository`]. A Git Providers connection id and
/// the provider's repository id are never members (decision K4).
const REPOSITORY_MEMBERS: &[Member] = &[
    Member {
        display_safe: false,
        identifier: true,
        ..member("id", VariableType::String, "Id")
    },
    member("key", VariableType::String, "Key"),
    member("name", VariableType::String, "Name"),
    Member {
        nullable: true,
        ..member("description", VariableType::String, "Description")
    },
    member("isDefault", VariableType::Boolean, "Default"),
    // Not display-safe (OD-4): a key shows names, never this computer's
    // folders.
    Member {
        nullable: true,
        display_safe: false,
        private_path: true,
        ..member("path", VariableType::String, "Folder")
    },
    member("exists", VariableType::Boolean, "Folder exists"),
    // The primary remote (decision K1).
    Member {
        nullable: true,
        ..member("remote.name", VariableType::String, "Remote")
    },
    Member {
        nullable: true,
        display_safe: false,
        ..member("remote.url", VariableType::String, "Remote URL")
    },
    Member {
        nullable: true,
        ..member(
            "remote.defaultBranch",
            VariableType::String,
            "Default branch",
        )
    },
];

impl Shape {
    pub const ALL: [Self; 1] = [Self::Repository];

    pub fn name(self) -> &'static str {
        match self {
            Self::Repository => "repository",
        }
    }

    /// Every scalar member, in a stable order.
    pub fn members(self) -> &'static [Member] {
        match self {
            Self::Repository => REPOSITORY_MEMBERS,
        }
    }

    /// The scalar member at `path` (`remote.url`), if the shape has one.
    pub fn member(self, path: &str) -> Option<&'static Member> {
        self.members().iter().find(|member| member.path == path)
    }

    /// The groups of members (`remote`): read their members, never the
    /// group as a whole.
    pub fn is_group(self, path: &str) -> bool {
        self.members().iter().any(|member| {
            member
                .path
                .strip_prefix(path)
                .is_some_and(|rest| rest.starts_with('.'))
        })
    }
}

impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A value inside a structured value: a scalar, `null`, or a group of
/// members (`remote`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    Null,
    Boolean(bool),
    Integer(i64),
    String(String),
    Object(BTreeMap<String, Scalar>),
}

impl Scalar {
    /// `text` or `null`.
    pub fn text(text: Option<&str>) -> Self {
        text.map_or(Self::Null, |text| Self::String(text.to_owned()))
    }

    fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Boolean(flag) => (*flag).into(),
            Self::Integer(number) => (*number).into(),
            Self::String(text) => text.clone().into(),
            Self::Object(members) => members_json(members),
        }
    }
}

fn members_json(members: &BTreeMap<String, Scalar>) -> serde_json::Value {
    serde_json::Value::Object(
        members
            .iter()
            .map(|(key, value)| (key.clone(), value.to_json()))
            .collect(),
    )
}

/// The members of one object, by name; groups nest.
pub type Members = BTreeMap<String, Scalar>;

/// A value of type `object<shape>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectValue {
    pub shape: Shape,
    pub members: Members,
}

/// One entry of a [`MapValue`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapEntry {
    /// The entry's key: a repository's key.
    pub key: String,
    pub members: Members,
}

/// A value of type `map<shape>`: objects by key, in the user's order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapValue {
    pub shape: Shape,
    pub entries: Vec<MapEntry>,
}

/// What reading a member of a structured value gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberValue {
    /// A scalar (`string`, `integer`, `boolean`).
    Scalar(Scalar),
    /// A whole object (an entry of a map).
    Object(ObjectValue),
    /// The member has no value (`null`).
    Null,
    /// The shape has no such member.
    UnknownMember,
    /// The map has no entry with that key: no such repository in the
    /// project.
    NoEntry,
}

/// Looks `path` up in `members` (groups nest: `remote.url`).
fn lookup<'a>(members: &'a Members, path: &str) -> Option<&'a Scalar> {
    let mut segments = path.split('.');
    let mut current = members.get(segments.next()?)?;
    for segment in segments {
        match current {
            Scalar::Object(inner) => current = inner.get(segment)?,
            _ => return None,
        }
    }
    Some(current)
}

fn read(shape: Shape, members: &Members, path: &str) -> MemberValue {
    if shape.member(path).is_none() {
        return MemberValue::UnknownMember;
    }
    match lookup(members, path) {
        None | Some(Scalar::Null) => MemberValue::Null,
        // Only scalar members are listed, so a group never reaches here.
        Some(Scalar::Object(_)) => MemberValue::UnknownMember,
        Some(scalar) => MemberValue::Scalar(scalar.clone()),
    }
}

impl ObjectValue {
    /// The member at `path`.
    pub fn member(&self, path: &MemberPath) -> MemberValue {
        read(self.shape, &self.members, path.as_str())
    }

    pub fn to_json(&self) -> serde_json::Value {
        members_json(&self.members)
    }

    /// The same object with every private path (`path`) replaced by
    /// `null`: what a reader that may not see this computer's folders gets.
    #[must_use]
    pub fn without_paths(mut self) -> Self {
        hide_paths(self.shape, &mut self.members);
        self
    }
}

fn hide_paths(shape: Shape, members: &mut Members) {
    for member in shape.members().iter().filter(|m| m.private_path) {
        if let Some(value) = members.get_mut(member.path) {
            *value = Scalar::Null;
        }
    }
}

impl MapValue {
    /// The entry with `key`.
    pub fn get(&self, key: &str) -> Option<&MapEntry> {
        self.entries.iter().find(|entry| entry.key == key)
    }

    /// The entry at the first segment of `path`, or a member of it.
    pub fn member(&self, path: &MemberPath) -> MemberValue {
        let (key, rest) = match path.as_str().split_once('.') {
            Some((key, rest)) => (key, Some(rest)),
            None => (path.as_str(), None),
        };
        let Some(entry) = self.get(key) else {
            return MemberValue::NoEntry;
        };
        match rest {
            None => MemberValue::Object(ObjectValue {
                shape: self.shape,
                members: entry.members.clone(),
            }),
            Some(rest) => read(self.shape, &entry.members, rest),
        }
    }

    /// An object of the entries by key. JSON objects have no order; the
    /// user's order is each entry's position in [`MapValue::entries`].
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Object(
            self.entries
                .iter()
                .map(|entry| (entry.key.clone(), members_json(&entry.members)))
                .collect(),
        )
    }

    /// The same map with every private path replaced by `null`.
    #[must_use]
    pub fn without_paths(mut self) -> Self {
        for entry in &mut self.entries {
            hide_paths(self.shape, &mut entry.members);
        }
        self
    }
}

/// Most segments of a member path: `cms.remote.url` (a map key, a group,
/// a member).
pub const MAX_MEMBER_SEGMENTS: usize = 3;

/// The member part of a reference to a structured variable, 1 to
/// [`MAX_MEMBER_SEGMENTS`] segments of the reference grammar: `name`,
/// `remote.url`, or for a map `cms`, `cms.path`, `cms.remote.url`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemberPath(String);

impl MemberPath {
    pub fn new(path: &str) -> Option<Self> {
        let segments: Vec<&str> = path.split('.').collect();
        ((1..=MAX_MEMBER_SEGMENTS).contains(&segments.len())
            && segments.iter().all(|segment| valid_segment(segment)))
        .then(|| Self(path.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MemberPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a member path of a variable of `value_type` names, without a
/// value: the member, or why it names none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberTarget {
    /// One scalar member.
    Scalar(&'static Member),
    /// A whole object (a map's entry).
    Object(Shape),
    /// Not a member of the shape (or a group of members, which is read
    /// member by member).
    Unknown,
}

/// The target of `path` in a value of `value_type`; `Unknown` for a type
/// without members.
pub fn member_target(value_type: VariableType, path: &MemberPath) -> MemberTarget {
    let (shape, path) = match value_type {
        VariableType::Object { shape } => (shape, path.as_str()),
        VariableType::Map { of } => match path.as_str().split_once('.') {
            None => return MemberTarget::Object(of),
            Some((_, rest)) => (of, rest),
        },
        _ => return MemberTarget::Unknown,
    };
    shape
        .member(path)
        .map_or(MemberTarget::Unknown, MemberTarget::Scalar)
}

/// The members of `repository` of `project` (`docs/variables.md` §2.8):
/// its id, key, name, description, whether it is the project's default,
/// its folder from `machine` (the binding of its resource, a directory)
/// and whether that folder exists, and its primary remote's name, URL and
/// default branch. The folder is `null` without `private_paths` (whether
/// it exists is still told), and when the resource is unbound or bound to
/// something other than a directory. No connection or provider id
/// (decision K4).
pub fn repository_members(
    project: &crate::Project,
    repository: &crate::Repository,
    machine: Option<&crate::MachineConfig>,
    private_paths: bool,
) -> Members {
    let folder = machine
        .and_then(|machine| machine.binding(&repository.resource))
        .filter(|binding| binding.kind == crate::ResourceKind::Directory)
        .map(|binding| binding.value.as_str());
    let exists = folder.is_some_and(|folder| std::path::Path::new(folder).is_dir());
    let default = project.default_repository().map(|r| r.id);
    let remote = repository.primary_remote().map_or(Scalar::Null, |remote| {
        Scalar::Object(BTreeMap::from([
            ("name".to_owned(), Scalar::String(remote.name.clone())),
            ("url".to_owned(), Scalar::text(remote.remote_url.as_deref())),
            (
                "defaultBranch".to_owned(),
                Scalar::text(remote.default_branch.as_deref()),
            ),
        ]))
    });
    BTreeMap::from([
        ("id".to_owned(), Scalar::String(repository.id.to_string())),
        (
            "key".to_owned(),
            Scalar::String(repository.key.as_str().to_owned()),
        ),
        ("name".to_owned(), Scalar::String(repository.name.clone())),
        (
            "description".to_owned(),
            Scalar::text(repository.description.as_deref()),
        ),
        (
            "isDefault".to_owned(),
            Scalar::Boolean(default == Some(repository.id)),
        ),
        (
            "path".to_owned(),
            Scalar::text(folder.filter(|_| private_paths)),
        ),
        ("exists".to_owned(), Scalar::Boolean(exists)),
        ("remote".to_owned(), remote),
    ])
}

/// Every repository of `project` (the stored ones, or the one its
/// `repository` role stands for), by key in the user's order:
/// `projects.current.repositories`.
pub fn repositories_value(
    project: &crate::Project,
    machine: Option<&crate::MachineConfig>,
    private_paths: bool,
) -> MapValue {
    MapValue {
        shape: Shape::Repository,
        entries: project
            .effective_repositories()
            .iter()
            .map(|repository| MapEntry {
                key: repository.key.as_str().to_owned(),
                members: repository_members(project, repository, machine, private_paths),
            })
            .collect(),
    }
}

/// The default repository of `project` (decision OD-3):
/// `projects.current.repository`. The one marked as default, else the only
/// one when there is exactly one; never the first by position, so
/// reordering never changes what a key does. Without one: no repository at
/// all ([`super::MissingReason::NoValue`]), or several and none the default
/// ([`super::MissingReason::NoDefaultRepository`]).
pub fn default_repository_value(
    project: &crate::Project,
    machine: Option<&crate::MachineConfig>,
    private_paths: bool,
) -> Result<ObjectValue, super::MissingReason> {
    match project.default_repository() {
        Some(repository) => Ok(ObjectValue {
            shape: Shape::Repository,
            members: repository_members(project, &repository, machine, private_paths),
        }),
        None if project.effective_repositories().is_empty() => Err(super::MissingReason::NoValue),
        None => Err(super::MissingReason::NoDefaultRepository),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository(key: &str, path: Option<&str>) -> Members {
        BTreeMap::from([
            ("id".into(), Scalar::String(format!("id-{key}"))),
            ("key".into(), Scalar::String(key.into())),
            ("name".into(), Scalar::String(key.to_uppercase())),
            ("description".into(), Scalar::Null),
            ("isDefault".into(), Scalar::Boolean(false)),
            ("path".into(), Scalar::text(path)),
            ("exists".into(), Scalar::Boolean(path.is_some())),
            (
                "remote".into(),
                Scalar::Object(BTreeMap::from([
                    ("name".into(), Scalar::String("origin".into())),
                    ("url".into(), Scalar::Null),
                    ("defaultBranch".into(), Scalar::String("main".into())),
                ])),
            ),
        ])
    }

    fn path(text: &str) -> MemberPath {
        MemberPath::new(text).unwrap()
    }

    #[test]
    fn every_member_is_a_valid_scalar_path_and_ids_and_paths_are_never_shown() {
        for shape in Shape::ALL {
            for member in shape.members() {
                assert!(MemberPath::new(member.path).is_some(), "{}", member.path);
                assert!(
                    matches!(
                        member.value_type,
                        VariableType::String | VariableType::Integer | VariableType::Boolean
                    ),
                    "{}",
                    member.path
                );
                if member.identifier || member.private_path {
                    assert!(!member.display_safe, "{}", member.path);
                }
            }
        }
        let repository = Shape::Repository;
        assert!(!repository.member("path").unwrap().display_safe);
        assert!(!repository.member("id").unwrap().display_safe);
        assert!(!repository.member("remote.url").unwrap().display_safe);
        assert!(repository.member("name").unwrap().display_safe);
        assert!(
            repository
                .member("remote.defaultBranch")
                .unwrap()
                .display_safe
        );
        // K4: no connection or provider ids.
        assert!(repository.member("remote.connectionId").is_none());
        assert!(repository.member("remote.remoteRepositoryId").is_none());
        assert!(repository.is_group("remote"));
        assert!(!repository.is_group("name"));
    }

    #[test]
    fn member_paths_have_one_to_three_segments() {
        assert!(MemberPath::new("cms.remote.url").is_some());
        assert!(MemberPath::new("a.b.c.d").is_none());
        assert!(MemberPath::new("").is_none());
        assert!(MemberPath::new("Cms").is_none());
        assert!(MemberPath::new("cms..path").is_none());
    }

    #[test]
    fn objects_and_maps_read_their_members() {
        let object = ObjectValue {
            shape: Shape::Repository,
            members: repository("cms", Some("/work/cms")),
        };
        assert_eq!(
            object.member(&path("name")),
            MemberValue::Scalar(Scalar::String("CMS".into()))
        );
        assert_eq!(
            object.member(&path("remote.defaultBranch")),
            MemberValue::Scalar(Scalar::String("main".into()))
        );
        assert_eq!(object.member(&path("remote.url")), MemberValue::Null);
        assert_eq!(object.member(&path("description")), MemberValue::Null);
        assert_eq!(object.member(&path("remote")), MemberValue::UnknownMember);
        assert_eq!(object.member(&path("nope")), MemberValue::UnknownMember);
        assert_eq!(
            object.clone().without_paths().member(&path("path")),
            MemberValue::Null
        );

        let map = MapValue {
            shape: Shape::Repository,
            entries: vec![
                MapEntry {
                    key: "cms".into(),
                    members: repository("cms", Some("/work/cms")),
                },
                MapEntry {
                    key: "api".into(),
                    members: repository("api", None),
                },
            ],
        };
        assert_eq!(
            map.member(&path("cms.path")),
            MemberValue::Scalar(Scalar::String("/work/cms".into()))
        );
        assert_eq!(map.member(&path("api.path")), MemberValue::Null);
        assert_eq!(map.member(&path("web.name")), MemberValue::NoEntry);
        assert_eq!(map.member(&path("cms.nope")), MemberValue::UnknownMember);
        assert!(matches!(map.member(&path("api")), MemberValue::Object(_)));
        assert_eq!(
            map.clone().without_paths().member(&path("cms.path")),
            MemberValue::Null
        );
        let json = map.to_json();
        assert_eq!(json["cms"]["remote"]["defaultBranch"], "main");
        assert_eq!(json["api"]["path"], serde_json::Value::Null);
    }

    #[test]
    fn member_targets_come_from_the_type_alone() {
        let object = VariableType::Object {
            shape: Shape::Repository,
        };
        let map = VariableType::Map {
            of: Shape::Repository,
        };
        assert!(matches!(
            member_target(object, &path("path")),
            MemberTarget::Scalar(m) if m.private_path
        ));
        assert_eq!(
            member_target(map, &path("cms")),
            MemberTarget::Object(Shape::Repository)
        );
        assert!(matches!(
            member_target(map, &path("cms.name")),
            MemberTarget::Scalar(m) if m.display_safe
        ));
        assert_eq!(member_target(map, &path("cms.x")), MemberTarget::Unknown);
        assert_eq!(
            member_target(VariableType::String, &path("x")),
            MemberTarget::Unknown
        );
    }

    #[test]
    fn values_round_trip_as_json() {
        let map = MapValue {
            shape: Shape::Repository,
            entries: vec![MapEntry {
                key: "cms".into(),
                members: repository("cms", None),
            }],
        };
        let json = serde_json::to_value(&map).unwrap();
        assert_eq!(json["shape"], "repository");
        assert_eq!(json["entries"][0]["key"], "cms");
        let back: MapValue = serde_json::from_value(json).unwrap();
        assert_eq!(back, map);
    }
}
