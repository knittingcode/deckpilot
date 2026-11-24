//! The repositories of a project (`docs/data-model.md`, "Repositories").
//!
//! A project lists any number of repositories. Each names a
//! [`ResourceKey`] whose machine binding (kind `directory`) holds its folder
//! on this computer: paths are never part of the portable model. The
//! repository role (`resources["repository"]`) that Git, Docker and the
//! Workspace actions resolve stays as it was; marking a repository as the
//! default also points that role at it ([`Project::set_default_repository`]),
//! and nothing else writes the role.
//!
//! A project with a `repository` role and no stored repositories (data an
//! older DeckPilot wrote, or an import of an older export) has one *derived*
//! repository ([`Project::derived_repository`]): key `main`, the project's
//! name, the role's resource, and an id derived from the project id, so
//! every computer, export and import agrees on it. It is stored only when
//! the project is saved with its repositories.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use unicode_segmentation::UnicodeSegmentation;
use uuid::Uuid;

use super::Project;
use crate::{ProjectId, ResourceKey};

/// The project role the default repository follows.
pub const REPOSITORY_ROLE: &str = "repository";

/// The key of the repository derived from the `repository` role.
pub const DERIVED_REPOSITORY_KEY: &str = "main";

/// A sanity bound for IPC and display, not a product limit.
pub const MAX_REPOSITORIES_PER_PROJECT: usize = 256;

/// Longest repository name, in characters.
pub const MAX_REPOSITORY_NAME_CHARS: usize = 64;

/// Longest repository description, in characters.
pub const MAX_REPOSITORY_DESCRIPTION_CHARS: usize = 500;

/// Longest repository key, in characters.
pub const MAX_REPOSITORY_KEY_CHARS: usize = 32;

const MAX_REMOTE_URL_CHARS: usize = 2048;
const MAX_REMOTE_ID_CHARS: usize = 64;
const MAX_CONNECTION_ID_CHARS: usize = 256;
const MAX_BRANCH_CHARS: usize = 255;

/// Namespace of the ids derived from project ids (the `repository` role's
/// repository, and the repositories of a copied project).
const REPOSITORY_NAMESPACE: Uuid = Uuid::from_u128(0xca56_ac7e_d700_4356_8d21_9bca_98e9_2d2b);

/// Stable identifier of a repository; never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepositoryId(Uuid);

impl RepositoryId {
    /// A new random identifier.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// The id of the repository derived from `project`'s `repository` role:
    /// `uuid_v5(namespace, "<project id>\0repository")`, the project id in
    /// its lowercase hyphenated form.
    pub fn derived(project: ProjectId) -> Self {
        let name = format!("{project}\0{REPOSITORY_ROLE}");
        Self(Uuid::new_v5(&REPOSITORY_NAMESPACE, name.as_bytes()))
    }

    /// The id a copy of `project` (now `copy`) gives `repository`:
    /// `uuid_v5(namespace, "<copy id>\0repository\0<repository id>")`, so a
    /// copy made twice the same way agrees with itself.
    fn copied(copy: ProjectId, repository: Self) -> Self {
        let name = format!("{copy}\0{REPOSITORY_ROLE}\0{repository}");
        Self(Uuid::new_v5(&REPOSITORY_NAMESPACE, name.as_bytes()))
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for RepositoryId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RepositoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for RepositoryId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

/// The key of a repository within its project, such as `cms`: a lowercase
/// ASCII letter, then up to 31 lowercase letters, digits or `_`
/// (`^[a-z][a-z0-9_]{0,31}$`). It is the variable segment of
/// `projects.current.repositories.<key>`, so it has a single spelling.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RepositoryKey(String);

impl RepositoryKey {
    pub fn new(key: impl Into<String>) -> Result<Self, RepositoryError> {
        let key = key.into();
        let valid = !key.is_empty()
            && key.len() <= MAX_REPOSITORY_KEY_CHARS
            && key.starts_with(|c: char| c.is_ascii_lowercase())
            && key
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if valid {
            Ok(Self(key))
        } else {
            Err(RepositoryError::InvalidKey(key))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RepositoryKey {
    type Error = RepositoryError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RepositoryKey> for String {
    fn from(value: RepositoryKey) -> Self {
        value.0
    }
}

impl fmt::Display for RepositoryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Most remotes per repository (decision K1).
pub const MAX_REMOTES_PER_REPOSITORY: usize = 8;

const MAX_REMOTE_NAME_CHARS: usize = 64;

/// One remote of a repository (shared contract C1 with Git Providers, as
/// decided in K1–K3): the git remote `name`, whether it is the `primary`
/// one (at most one is), and its link. Only the app writes remotes
/// ([`Project::set_repository_remotes`], after the user confirms); every
/// editor save keeps them as they are. Exports leave out `connectionId`,
/// whose meaning is local to this computer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryRemote {
    /// The git remote name (`origin`, `upstream`).
    pub name: String,
    /// The remote the variable member `remote` resolves to.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub primary: bool,
    /// Opaque id of a Git Providers connection on this computer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    /// The provider's immutable id, prefixed with the provider:
    /// `^(gh|gl|bb):[0-9A-Za-z{}_-]{1,64}$` (decision K2). Names such as
    /// `group/project` are refused; they can change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_repository_id: Option<String>,
    /// An `https://`, `ssh://` or `user@host:path` URL, without a password,
    /// query or fragment (and without a user name for `https`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
    /// A branch name in Git's ref name grammar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
}

impl RepositoryRemote {
    /// Checks every field ([`RepositoryError::InvalidRemote`]).
    pub fn check(&self) -> Result<(), RepositoryError> {
        let invalid = |reason| Err(RepositoryError::InvalidRemote(reason));
        let plain = |value: &str, max: usize| {
            !value.is_empty()
                && value.chars().count() <= max
                && !value.chars().any(char::is_control)
                && value.trim() == value
        };
        if !valid_remote_name(&self.name) {
            return invalid("name");
        }
        if let Some(id) = &self.connection_id
            && !plain(id, MAX_CONNECTION_ID_CHARS)
        {
            return invalid("connectionId");
        }
        if let Some(id) = &self.remote_repository_id
            && !valid_remote_repository_id(id)
        {
            return invalid("remoteRepositoryId");
        }
        if let Some(url) = &self.remote_url
            && !(plain(url, MAX_REMOTE_URL_CHARS) && valid_remote_url(url))
        {
            return invalid("remoteUrl");
        }
        if let Some(branch) = &self.default_branch
            && !valid_branch(branch)
        {
            return invalid("defaultBranch");
        }
        Ok(())
    }

    /// The remote without what only this computer understands
    /// (`connectionId`), for an export.
    #[must_use]
    pub fn portable(&self) -> Self {
        Self {
            connection_id: None,
            ..self.clone()
        }
    }
}

/// A git remote name: a letter or digit, then up to 63 letters, digits,
/// `.`, `_` or `-`; never `..` or a `.lock` ending (stricter than Git, so it
/// is always one safe ref component).
fn valid_remote_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_REMOTE_NAME_CHARS
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !name.contains("..")
        // Git refs are case-sensitive: only `.lock` itself is refused.
        && name.strip_suffix(".lock").is_none()
        && !name.ends_with('.')
}

/// `^(gh|gl|bb):[0-9A-Za-z{}_-]{1,64}$` (decision K2).
fn valid_remote_repository_id(id: &str) -> bool {
    let Some((provider, rest)) = id.split_once(':') else {
        return false;
    };
    matches!(provider, "gh" | "gl" | "bb")
        && !rest.is_empty()
        && rest.len() <= MAX_REMOTE_ID_CHARS
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '{' | '}' | '_' | '-'))
}

/// `https://host/path`, `ssh://[user@]host[:port]/path` or the scp-like
/// `user@host:path`. Never a password, a query or a fragment; a user name
/// only for SSH (`git@`), where it is not a secret.
fn valid_remote_url(value: &str) -> bool {
    if value.chars().any(char::is_whitespace) || value.contains(['?', '#']) {
        return false;
    }
    if value.starts_with("https://") || value.starts_with("ssh://") {
        let Ok(url) = url::Url::parse(value) else {
            return false;
        };
        let host = url.host_str().is_some_and(|h| !h.is_empty());
        let user = url.username().is_empty() || url.scheme() == "ssh";
        return host && user && url.password().is_none();
    }
    // scp-like: `user@host:path` (Git reads a `:` before any `/` this way).
    let Some((authority, path)) = value.split_once(':') else {
        return false;
    };
    let Some((user, host)) = authority.split_once('@') else {
        return false;
    };
    let word = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    };
    word(user) && word(host) && !path.is_empty() && !path.starts_with('/')
}

/// A branch name as `git check-ref-format --branch` accepts it.
fn valid_branch(name: &str) -> bool {
    if name.is_empty()
        || name.len() > MAX_BRANCH_CHARS
        || name == "@"
        || name.starts_with('-')
        || name.starts_with('/')
        || name.ends_with('/')
        || name.ends_with('.')
        || name.contains("..")
        || name.contains("//")
        || name.contains("@{")
    {
        return false;
    }
    if name
        .chars()
        .any(|c| c.is_ascii_control() || c == ' ' || "~^:?*[\\".contains(c) || c == '\u{7f}')
    {
        return false;
    }
    name.split('/')
        .all(|part| !part.starts_with('.') && part.strip_suffix(".lock").is_none())
}

/// One repository of a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    pub id: RepositoryId,
    pub key: RepositoryKey,
    /// 1 to 64 characters, trimmed; the user's text.
    pub name: String,
    /// The machine binding (kind `directory`) that holds its folder.
    pub resource: ResourceKey,
    /// Up to 500 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Its remotes (K1), at most [`MAX_REMOTES_PER_REPOSITORY`]; see
    /// [`RepositoryRemote`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remotes: Vec<RepositoryRemote>,
}

impl Repository {
    /// Checks the name, the description and the remote link.
    pub fn check(&self) -> Result<(), RepositoryError> {
        let name = self.name.chars().count();
        if name == 0
            || name > MAX_REPOSITORY_NAME_CHARS
            || self.name.trim() != self.name
            || self.name.chars().any(char::is_control)
        {
            return Err(RepositoryError::InvalidName(self.key.to_string()));
        }
        if self
            .description
            .as_ref()
            .is_some_and(|d| d.chars().count() > MAX_REPOSITORY_DESCRIPTION_CHARS)
        {
            return Err(RepositoryError::DescriptionTooLong(self.key.to_string()));
        }
        self.check_remotes()
    }

    /// At most [`MAX_REMOTES_PER_REPOSITORY`] remotes with unique names, at
    /// most one primary, each valid.
    pub fn check_remotes(&self) -> Result<(), RepositoryError> {
        if self.remotes.len() > MAX_REMOTES_PER_REPOSITORY {
            return Err(RepositoryError::TooManyRemotes);
        }
        let mut names = std::collections::HashSet::new();
        for remote in &self.remotes {
            remote.check()?;
            if !names.insert(remote.name.as_str()) {
                return Err(RepositoryError::DuplicateRemote(remote.name.clone()));
            }
        }
        if self.remotes.iter().filter(|r| r.primary).count() > 1 {
            return Err(RepositoryError::SeveralPrimaryRemotes);
        }
        Ok(())
    }

    /// The primary remote, which the variable member `remote` resolves to.
    pub fn primary_remote(&self) -> Option<&RepositoryRemote> {
        self.remotes.iter().find(|r| r.primary)
    }
}

/// Why a project's repositories are not valid.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RepositoryError {
    #[error(
        "repository key `{0}` is not valid: use a lowercase letter, then up to 31 lowercase letters, digits or `_`"
    )]
    InvalidKey(String),
    #[error("repository key `{0}` is used more than once in this project")]
    DuplicateKey(String),
    #[error("a repository id is used more than once in this project")]
    DuplicateId,
    #[error("the name of repository `{0}` must be 1 to 64 characters")]
    InvalidName(String),
    #[error("the description of repository `{0}` is longer than 500 characters")]
    DescriptionTooLong(String),
    #[error("a project can have at most {max} repositories", max = MAX_REPOSITORIES_PER_PROJECT)]
    TooMany,
    #[error("the default repository is not a repository of this project")]
    UnknownDefault,
    #[error("the remote link of a repository is not valid ({0})")]
    InvalidRemote(&'static str),
    #[error("a repository can have at most {max} remotes", max = MAX_REMOTES_PER_REPOSITORY)]
    TooManyRemotes,
    #[error("remote `{0}` is listed more than once")]
    DuplicateRemote(String),
    #[error("only one remote of a repository can be the primary one")]
    SeveralPrimaryRemotes,
    #[error("the repository does not exist in this project")]
    NotFound,
    /// The last repository of a project with the `repository` role: the
    /// role would bring it back as the derived one (R3).
    #[error(
        "the last repository cannot be removed while the project has the repository role; remove the role first"
    )]
    LastWithRole,
}

impl RepositoryError {
    /// Stable code of the error, for translations (`repository.<code>`).
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidKey(_) => "invalidKey",
            Self::DuplicateKey(_) => "duplicateKey",
            Self::DuplicateId => "duplicateId",
            Self::InvalidName(_) => "invalidName",
            Self::DescriptionTooLong(_) => "descriptionTooLong",
            Self::TooMany => "tooMany",
            Self::UnknownDefault => "unknownDefault",
            Self::InvalidRemote(_) => "invalidRemote",
            Self::TooManyRemotes => "tooManyRemotes",
            Self::DuplicateRemote(_) => "duplicateRemote",
            Self::SeveralPrimaryRemotes => "severalPrimaryRemotes",
            Self::NotFound => "notFound",
            Self::LastWithRole => "lastWithRole",
        }
    }
}

/// `name` cut to [`MAX_REPOSITORY_NAME_CHARS`] at a grapheme boundary and
/// trimmed; `fallback` when nothing is left.
fn repository_name(name: &str, fallback: &str) -> String {
    let mut out = String::new();
    let mut count = 0;
    for grapheme in name.trim().graphemes(true) {
        let chars = grapheme.chars().count();
        if count + chars > MAX_REPOSITORY_NAME_CHARS {
            break;
        }
        count += chars;
        out.push_str(grapheme);
    }
    let out = out.trim();
    if out.is_empty() || out.chars().any(char::is_control) {
        fallback.to_owned()
    } else {
        out.to_owned()
    }
}

impl Project {
    /// The repository the `repository` role stands for when the project
    /// stores none (rule R3): key `main`, the project's name (cut to 64
    /// characters), the role's resource and [`RepositoryId::derived`].
    pub fn derived_repository(&self) -> Option<Repository> {
        let resource = self.resources.get(REPOSITORY_ROLE)?;
        Some(Repository {
            id: RepositoryId::derived(self.id),
            key: RepositoryKey(DERIVED_REPOSITORY_KEY.to_owned()),
            name: repository_name(&self.name, DERIVED_REPOSITORY_KEY),
            resource: resource.clone(),
            description: None,
            remotes: Vec::new(),
        })
    }

    /// The repositories as they are used: the stored ones, or the derived
    /// one when none are stored (R3).
    pub fn effective_repositories(&self) -> Cow<'_, [Repository]> {
        if self.repositories.is_empty()
            && let Some(derived) = self.derived_repository()
        {
            Cow::Owned(vec![derived])
        } else {
            Cow::Borrowed(&self.repositories)
        }
    }

    /// The stored default, or the derived repository when none are stored.
    pub fn effective_default_repository_id(&self) -> Option<RepositoryId> {
        if self.repositories.is_empty() {
            self.derived_repository().map(|r| r.id)
        } else {
            self.default_repository
        }
    }

    /// The default repository (decision OD-3): the one marked as default,
    /// else the only repository when there is exactly one, else none. Never
    /// the first by position.
    pub fn default_repository(&self) -> Option<Repository> {
        let repositories = self.effective_repositories();
        match self.effective_default_repository_id() {
            Some(id) => repositories.iter().find(|r| r.id == id).cloned(),
            None if repositories.len() == 1 => repositories.first().cloned(),
            None => None,
        }
    }

    /// The repository with `key`, among the effective ones.
    pub fn repository_by_key(&self, key: &str) -> Option<Repository> {
        self.effective_repositories()
            .iter()
            .find(|r| r.key.as_str() == key)
            .cloned()
    }

    /// Whether the stored repositories are exactly the derived one as it
    /// stands now (as the migration stored it, never edited).
    pub fn has_only_derived_repository(&self) -> bool {
        self.derived_repository().is_some_and(|derived| {
            self.default_repository == Some(derived.id) && self.repositories == [derived]
        })
    }

    /// Stores the derived repository (R3) when none are stored; `true` when
    /// it did.
    pub fn materialize_repositories(&mut self) -> bool {
        if !self.repositories.is_empty() {
            return false;
        }
        let Some(derived) = self.derived_repository() else {
            return false;
        };
        self.default_repository = Some(derived.id);
        self.repositories.push(derived);
        true
    }

    /// This project with the derived repository filled in (R3), for showing
    /// it; nothing is stored.
    #[must_use]
    pub fn with_derived_repositories(&self) -> Self {
        let mut project = self.clone();
        project.materialize_repositories();
        project
    }

    /// Unique ids and keys, a default that is one of them, at most
    /// [`MAX_REPOSITORIES_PER_PROJECT`], and each repository's own checks.
    pub fn validate_repositories(&self) -> Result<(), RepositoryError> {
        if self.repositories.len() > MAX_REPOSITORIES_PER_PROJECT {
            return Err(RepositoryError::TooMany);
        }
        let mut ids = std::collections::HashSet::new();
        let mut keys = std::collections::HashSet::new();
        for repository in &self.repositories {
            if !ids.insert(repository.id) {
                return Err(RepositoryError::DuplicateId);
            }
            if !keys.insert(repository.key.as_str()) {
                return Err(RepositoryError::DuplicateKey(repository.key.to_string()));
            }
            repository.check()?;
        }
        if self.default_repository.is_some_and(|id| !ids.contains(&id)) {
            return Err(RepositoryError::UnknownDefault);
        }
        Ok(())
    }

    /// Marks `id` as the default repository (rule R2) and points the
    /// `repository` role at its resource, adding the role when it is
    /// missing; `None` clears the default and leaves the role as it is.
    /// The derived repository is stored first (R3). Returns whether the
    /// role changed.
    pub fn set_default_repository(
        &mut self,
        id: Option<RepositoryId>,
    ) -> Result<bool, RepositoryError> {
        self.materialize_repositories();
        let Some(id) = id else {
            self.default_repository = None;
            return Ok(false);
        };
        let resource = self
            .repositories
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.resource.clone())
            .ok_or(RepositoryError::NotFound)?;
        self.default_repository = Some(id);
        let changed = self.resources.get(REPOSITORY_ROLE) != Some(&resource);
        if changed {
            self.resources.insert(REPOSITORY_ROLE.to_owned(), resource);
        }
        Ok(changed)
    }

    /// Removes a repository entry. Its binding and the project's roles are
    /// never touched (nothing is deleted automatically); when it was the
    /// default, the project has no default afterwards and the
    /// `repository` role keeps its old resource.
    ///
    /// The last repository is not removed while the project has the
    /// `repository` role ([`RepositoryError::LastWithRole`]): the role would
    /// stand for it again (R3).
    pub fn remove_repository(&mut self, id: RepositoryId) -> Result<Repository, RepositoryError> {
        self.materialize_repositories();
        if self.repositories.len() == 1
            && self.repositories[0].id == id
            && self.resources.contains_key(REPOSITORY_ROLE)
        {
            return Err(RepositoryError::LastWithRole);
        }
        let index = self
            .repositories
            .iter()
            .position(|r| r.id == id)
            .ok_or(RepositoryError::NotFound)?;
        if self.default_repository == Some(id) {
            self.default_repository = None;
        }
        Ok(self.repositories.remove(index))
    }

    /// Replaces the remotes of repository `id` (decision K3: only the app
    /// calls this, after the user confirmed the link). The remotes are
    /// checked first; nothing changes when they are not valid.
    pub fn set_repository_remotes(
        &mut self,
        id: RepositoryId,
        remotes: Vec<RepositoryRemote>,
    ) -> Result<(), RepositoryError> {
        self.materialize_repositories();
        let repository = self
            .repositories
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or(RepositoryError::NotFound)?;
        let candidate = Repository {
            remotes,
            ..repository.clone()
        };
        candidate.check_remotes()?;
        *repository = candidate;
        Ok(())
    }

    /// A resource key for a new repository `key`: `<project key>.<key>`,
    /// the project key lowercased (characters outside `a-z 0-9 _ -` become
    /// `_`), with `-2`, `-3`, … appended while `taken` says the key is
    /// already used (by another project or a binding).
    ///
    /// # Panics
    ///
    /// Never: a project key and a repository key use only characters a
    /// resource key allows.
    pub fn new_repository_resource_key(
        &self,
        key: &RepositoryKey,
        taken: impl Fn(&ResourceKey) -> bool,
    ) -> ResourceKey {
        let project: String = self
            .key
            .as_str()
            .chars()
            .map(|c| {
                let c = c.to_ascii_lowercase();
                if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let base = format!("{project}.{key}");
        // A project key and a repository key are never empty and use only
        // characters a resource key allows, so these always parse.
        let candidate = |suffix: usize| {
            let text = if suffix < 2 {
                base.clone()
            } else {
                format!("{base}-{suffix}")
            };
            ResourceKey::new(text).expect("a repository resource key is a valid resource key")
        };
        let mut suffix = 1;
        loop {
            let key = candidate(suffix);
            if !taken(&key) {
                return key;
            }
            suffix += 1;
        }
    }

    /// A copy of this project with the id `id`. Its repositories get ids
    /// derived from the new project id: the derived repository the copy's
    /// derived id, the others `uuid_v5` of the new project id and their old
    /// id. The default follows.
    #[must_use]
    pub fn copy_with_id(&self, id: ProjectId) -> Self {
        let mut copy = self.clone();
        copy.id = id;
        let old_derived = RepositoryId::derived(self.id);
        let remap = |old: RepositoryId| {
            if old == old_derived {
                RepositoryId::derived(id)
            } else {
                RepositoryId::copied(id, old)
            }
        };
        for repository in &mut copy.repositories {
            repository.id = remap(repository.id);
        }
        copy.default_repository = copy.default_repository.map(remap);
        copy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProjectKey;

    fn project(roles: &[(&str, &str)]) -> Project {
        let mut project = Project::new(ProjectKey::new("SHOP").unwrap(), "Online Shop");
        for (role, key) in roles {
            project
                .resources
                .insert((*role).to_owned(), ResourceKey::new(*key).unwrap());
        }
        project
    }

    fn repository(key: &str, resource: &str) -> Repository {
        Repository {
            id: RepositoryId::new(),
            key: RepositoryKey::new(key).unwrap(),
            name: key.to_owned(),
            resource: ResourceKey::new(resource).unwrap(),
            description: None,
            remotes: Vec::new(),
        }
    }

    fn remote(name: &str) -> RepositoryRemote {
        RepositoryRemote {
            name: name.to_owned(),
            ..RepositoryRemote::default()
        }
    }

    #[test]
    fn keys_follow_the_variable_segment_grammar() {
        for good in [
            "main",
            "cms",
            "a",
            "api_2",
            "x".repeat(32).as_str(),
            "default",
        ] {
            assert!(RepositoryKey::new(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "Main",
            "2nd",
            "_x",
            "with-dash",
            "with space",
            "dot.ted",
            "żółw",
            "x".repeat(33).as_str(),
        ] {
            assert_eq!(
                RepositoryKey::new(bad),
                Err(RepositoryError::InvalidKey(bad.to_owned())),
                "{bad}"
            );
        }
        // Also when read: a stored or imported key is checked.
        assert!(serde_json::from_str::<RepositoryKey>("\"Bad\"").is_err());
    }

    #[test]
    fn the_derived_repository_is_deterministic() {
        let shop = project(&[("repository", "shop.repository"), ("docs", "shop.docs")]);
        let derived = shop.derived_repository().unwrap();
        assert_eq!(derived.id, RepositoryId::derived(shop.id));
        assert_eq!(derived.id, shop.derived_repository().unwrap().id);
        assert_eq!(derived.key.as_str(), "main");
        assert_eq!(derived.name, "Online Shop");
        assert_eq!(derived.resource.as_str(), "shop.repository");
        assert_ne!(
            RepositoryId::derived(shop.id),
            RepositoryId::derived(ProjectId::new())
        );
        // A fixed project id always derives the same repository id.
        let fixed: ProjectId = "6f7c3a2e-1d4b-4f5a-9e8d-7c6b5a493827".parse().unwrap();
        assert_eq!(
            RepositoryId::derived(fixed).to_string(),
            RepositoryId::derived(fixed).to_string()
        );
        assert_eq!(RepositoryId::derived(fixed).as_uuid().get_version_num(), 5);

        // R3: shown, but not stored.
        assert_eq!(shop.repositories, []);
        assert_eq!(
            shop.effective_repositories().as_ref(),
            std::slice::from_ref(&derived)
        );
        assert_eq!(shop.effective_default_repository_id(), Some(derived.id));
        assert_eq!(shop.default_repository(), Some(derived.clone()));
        assert_eq!(shop.repository_by_key("main"), Some(derived.clone()));
        let shown = shop.with_derived_repositories();
        assert_eq!(shown.repositories, std::slice::from_ref(&derived));
        assert_eq!(shown.default_repository, Some(derived.id));
        assert_eq!(shown.resources, shop.resources);

        // Without the role there is nothing to derive.
        let bare = project(&[("docs", "shop.docs")]);
        assert_eq!(bare.derived_repository(), None);
        assert!(bare.effective_repositories().is_empty());
        assert_eq!(bare.default_repository(), None);
    }

    #[test]
    fn a_long_project_name_is_cut_for_the_derived_repository() {
        let mut shop = project(&[("repository", "shop.repository")]);
        shop.name = format!("  {}  ", "ż".repeat(80));
        let derived = shop.derived_repository().unwrap();
        assert_eq!(derived.name.chars().count(), 64);
        derived.check().unwrap();
        shop.name = "   ".into();
        assert_eq!(shop.derived_repository().unwrap().name, "main");
    }

    #[test]
    fn one_and_several_repositories_validate() {
        let mut shop = project(&[]);
        shop.repositories.push(repository("api", "shop.api"));
        shop.validate_repositories().unwrap();
        // OD-3: exactly one repository is the default without a mark.
        assert_eq!(shop.default_repository().unwrap().key.as_str(), "api");

        shop.repositories.push(repository("cms", "shop.cms"));
        shop.repositories.push(repository("web", "shop.web"));
        shop.validate_repositories().unwrap();
        // Several and no mark: no default, never the first by position.
        assert_eq!(shop.default_repository(), None);
        shop.default_repository = Some(shop.repositories[2].id);
        assert_eq!(shop.default_repository().unwrap().key.as_str(), "web");
        assert_eq!(
            shop.repository_by_key("cms").unwrap().resource.as_str(),
            "shop.cms"
        );
    }

    #[test]
    fn duplicates_and_bad_entries_are_refused() {
        let mut shop = project(&[]);
        shop.repositories.push(repository("api", "shop.api"));
        shop.repositories.push(repository("api", "shop.other"));
        assert_eq!(
            shop.validate_repositories(),
            Err(RepositoryError::DuplicateKey("api".into()))
        );

        let mut shop = project(&[]);
        let api = repository("api", "shop.api");
        let mut twin = repository("cms", "shop.cms");
        twin.id = api.id;
        shop.repositories = vec![api, twin];
        assert_eq!(
            shop.validate_repositories(),
            Err(RepositoryError::DuplicateId)
        );

        let mut shop = project(&[]);
        shop.repositories.push(repository("api", "shop.api"));
        shop.default_repository = Some(RepositoryId::new());
        assert_eq!(
            shop.validate_repositories(),
            Err(RepositoryError::UnknownDefault)
        );

        for name in ["", " padded", "x".repeat(65).as_str(), "line\nbreak"] {
            let mut shop = project(&[]);
            let mut api = repository("api", "shop.api");
            api.name = name.to_owned();
            shop.repositories.push(api);
            assert_eq!(
                shop.validate_repositories(),
                Err(RepositoryError::InvalidName("api".into())),
                "{name:?}"
            );
        }
        let mut shop = project(&[]);
        let mut api = repository("api", "shop.api");
        api.name = "ż".repeat(64);
        api.description = Some("d".repeat(500));
        shop.repositories.push(api);
        shop.validate_repositories().unwrap();
        shop.repositories[0].description = Some("d".repeat(501));
        assert_eq!(
            shop.validate_repositories(),
            Err(RepositoryError::DescriptionTooLong("api".into()))
        );

        let mut shop = project(&[]);
        for index in 0..=MAX_REPOSITORIES_PER_PROJECT {
            shop.repositories
                .push(repository(&format!("r{index}"), &format!("shop.r{index}")));
        }
        assert_eq!(shop.validate_repositories(), Err(RepositoryError::TooMany));
        shop.repositories.pop();
        shop.validate_repositories().unwrap();
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One contract, field by field.
    fn remotes_are_checked_and_round_trip() {
        let origin = RepositoryRemote {
            name: "origin".into(),
            primary: true,
            connection_id: Some("conn-1".into()),
            remote_repository_id: Some("gh:R_kgDOabc123".into()),
            remote_url: Some("https://git.example.com/group/project.git".into()),
            default_branch: Some("release/1.x".into()),
        };
        origin.check().unwrap();
        let mut api = repository("api", "shop.api");
        api.remotes = vec![origin.clone(), remote("upstream")];
        api.check().unwrap();
        let json = serde_json::to_value(&api).unwrap();
        assert_eq!(
            json["remotes"],
            serde_json::json!([{
                "name": "origin",
                "primary": true,
                "connectionId": "conn-1",
                "remoteRepositoryId": "gh:R_kgDOabc123",
                "remoteUrl": "https://git.example.com/group/project.git",
                "defaultBranch": "release/1.x",
            }, {"name": "upstream"}])
        );
        let back: Repository = serde_json::from_value(json).unwrap();
        assert_eq!(back, api);
        assert_eq!(api.primary_remote(), Some(&origin));
        assert_eq!(
            origin.portable(),
            RepositoryRemote {
                connection_id: None,
                ..origin.clone()
            }
        );

        // Without remotes, nothing is written.
        let plain = serde_json::to_value(repository("api", "shop.api")).unwrap();
        assert!(plain.get("remotes").is_none());
        assert!(plain.get("description").is_none());

        // K1: unique names, at most one primary, at most 8.
        let mut twice = repository("api", "shop.api");
        twice.remotes = vec![remote("origin"), remote("origin")];
        assert_eq!(
            twice.check(),
            Err(RepositoryError::DuplicateRemote("origin".into()))
        );
        let mut two_primary = repository("api", "shop.api");
        two_primary.remotes = vec![
            RepositoryRemote {
                primary: true,
                ..remote("a")
            },
            RepositoryRemote {
                primary: true,
                ..remote("b")
            },
        ];
        assert_eq!(
            two_primary.check(),
            Err(RepositoryError::SeveralPrimaryRemotes)
        );
        let mut many = repository("api", "shop.api");
        many.remotes = (0..8).map(|i| remote(&format!("r{i}"))).collect();
        many.check().unwrap();
        assert_eq!(many.primary_remote(), None);
        many.remotes.push(remote("r8"));
        assert_eq!(many.check(), Err(RepositoryError::TooManyRemotes));

        for name in ["origin", "up-stream", "fork.2", "A_b"] {
            assert_eq!(remote(name).check(), Ok(()), "{name}");
        }
        for name in [
            "",
            "-x",
            ".x",
            "a..b",
            "a/b",
            "a b",
            "x.lock",
            "a.",
            &"n".repeat(65),
        ] {
            assert_eq!(
                remote(name).check(),
                Err(RepositoryError::InvalidRemote("name")),
                "{name}"
            );
        }

        // K2: provider-prefixed immutable ids only.
        for id in [
            "gh:R_kgDOabc",
            "gl:12345",
            "bb:{1b2c-3d4e}",
            &format!("gh:{}", "a".repeat(64)),
        ] {
            let entry = RepositoryRemote {
                remote_repository_id: Some(id.to_owned()),
                ..remote("origin")
            };
            assert_eq!(entry.check(), Ok(()), "{id}");
        }
        for id in [
            "group/proj",
            "12345",
            "gh:",
            "gh:group/proj",
            "gx:123",
            "GH:123",
            "gh:a b",
            "gh:a.b",
            &format!("gh:{}", "a".repeat(65)),
        ] {
            let entry = RepositoryRemote {
                remote_repository_id: Some(id.to_owned()),
                ..remote("origin")
            };
            assert_eq!(
                entry.check(),
                Err(RepositoryError::InvalidRemote("remoteRepositoryId")),
                "{id}"
            );
        }

        for url in [
            "https://github.com/org/repo.git",
            "ssh://git@github.com/org/repo.git",
            "ssh://git@host:2222/org/repo",
            "git@github.com:org/repo.git",
        ] {
            let entry = RepositoryRemote {
                remote_url: Some(url.into()),
                ..remote("origin")
            };
            assert_eq!(entry.check(), Ok(()), "{url}");
        }
        for url in [
            "https://user:secret@github.com/org/repo.git",
            "https://token@github.com/org/repo.git",
            "ssh://git:secret@github.com/org/repo.git",
            "https://github.com/org/repo.git?private_token=x",
            "https://github.com/org/repo.git#frag",
            "http://github.com/org/repo.git",
            "file:///etc/passwd",
            "/local/path",
            "github.com:org/repo",
            "git@github.com:/abs",
            "https://github.com/org/repo with space",
            "",
        ] {
            let entry = RepositoryRemote {
                remote_url: Some(url.into()),
                ..remote("origin")
            };
            assert_eq!(
                entry.check(),
                Err(RepositoryError::InvalidRemote("remoteUrl")),
                "{url}"
            );
        }
        let long = RepositoryRemote {
            remote_url: Some(format!("https://h.example/{}", "a".repeat(2048))),
            ..remote("origin")
        };
        assert!(long.check().is_err());

        for branch in ["main", "release/1.x", "feature/a-b_c", "v1.2"] {
            assert!(valid_branch(branch), "{branch}");
        }
        for branch in [
            "", "-x", "a..b", "a b", "a~1", "a^", "a:b", "a?", "a*", "a[", "a\\b", "a/", "/a",
            "a//b", ".hidden", "a/.b", "x.lock", "a.", "@", "a@{1}",
        ] {
            assert!(!valid_branch(branch), "{branch}");
        }
        assert!(!valid_branch(&"b".repeat(256)));

        let bad_connection = RepositoryRemote {
            connection_id: Some(String::new()),
            ..remote("origin")
        };
        assert_eq!(
            bad_connection.check(),
            Err(RepositoryError::InvalidRemote("connectionId"))
        );
        // A project checks the remotes of its repositories.
        let mut shop = project(&[]);
        shop.repositories.push(twice);
        assert!(shop.validate_repositories().is_err());
    }

    #[test]
    fn only_valid_remotes_are_set() {
        let mut shop = project(&[("repository", "shop.repository")]);
        let main = RepositoryId::derived(shop.id);
        let origin = RepositoryRemote {
            primary: true,
            remote_url: Some("git@github.com:org/shop.git".into()),
            ..remote("origin")
        };
        shop.set_repository_remotes(main, vec![origin.clone()])
            .unwrap();
        // The derived repository was stored with them.
        assert_eq!(shop.repositories[0].remotes, std::slice::from_ref(&origin));
        assert_eq!(
            shop.set_repository_remotes(main, vec![remote("a"), remote("a")]),
            Err(RepositoryError::DuplicateRemote("a".into()))
        );
        assert_eq!(shop.repositories[0].remotes, [origin]);
        assert_eq!(
            shop.set_repository_remotes(RepositoryId::new(), Vec::new()),
            Err(RepositoryError::NotFound)
        );
        shop.set_repository_remotes(main, Vec::new()).unwrap();
        assert_eq!(shop.repositories[0].remotes, []);
    }

    #[test]
    fn rename_remove_and_change_the_default() {
        let mut shop = project(&[("repository", "shop.repository"), ("docs", "shop.docs")]);
        let roles = shop.resources.clone();
        assert!(shop.materialize_repositories());
        assert!(!shop.materialize_repositories());
        let main = shop.repositories[0].id;
        assert_eq!(shop.default_repository, Some(main));
        assert_eq!(shop.resources, roles);

        let cms = repository("cms", "shop.cms");
        let cms_id = cms.id;
        shop.repositories.push(cms);
        shop.validate_repositories().unwrap();

        // Rename: name and key change, the id stays.
        shop.repositories[1].name = "Content".into();
        shop.repositories[1].key = RepositoryKey::new("content").unwrap();
        shop.validate_repositories().unwrap();
        assert_eq!(shop.repository_by_key("content").unwrap().id, cms_id);

        // R2: the default follows into the role, one way.
        assert_eq!(shop.set_default_repository(Some(cms_id)), Ok(true));
        assert_eq!(shop.default_repository, Some(cms_id));
        assert_eq!(shop.resources["repository"].as_str(), "shop.cms");
        assert_eq!(shop.resources["docs"].as_str(), "shop.docs");
        assert_eq!(shop.set_default_repository(Some(cms_id)), Ok(false));
        assert_eq!(
            shop.set_default_repository(Some(RepositoryId::new())),
            Err(RepositoryError::NotFound)
        );
        // Clearing the default leaves the role.
        assert_eq!(shop.set_default_repository(None), Ok(false));
        assert_eq!(shop.default_repository, None);
        assert_eq!(shop.resources["repository"].as_str(), "shop.cms");

        // Removing the default: no default, the role keeps its old key.
        shop.set_default_repository(Some(cms_id)).unwrap();
        let removed = shop.remove_repository(cms_id).unwrap();
        assert_eq!(removed.key.as_str(), "content");
        assert_eq!(shop.default_repository, None);
        assert_eq!(shop.resources["repository"].as_str(), "shop.cms");
        assert_eq!(
            shop.remove_repository(cms_id),
            Err(RepositoryError::NotFound)
        );
        // One repository left: it is the default again by OD-3.
        assert_eq!(shop.default_repository().unwrap().id, main);
        // It cannot be removed while the role would bring it back (R3).
        assert_eq!(
            shop.remove_repository(main),
            Err(RepositoryError::LastWithRole)
        );
        assert!(shop.has_only_derived_repository() || shop.repositories.len() == 1);
        let mut without_role = shop.clone();
        without_role.resources.remove("repository");
        without_role.remove_repository(main).unwrap();
        assert_eq!(without_role.repositories, []);

        // A project without the role gets it when a default is marked.
        let mut bare = project(&[]);
        let api = repository("api", "bare.api");
        let api_id = api.id;
        bare.repositories.push(api);
        assert_eq!(bare.set_default_repository(Some(api_id)), Ok(true));
        assert_eq!(bare.resources["repository"].as_str(), "bare.api");
    }

    #[test]
    fn new_resource_keys_never_reuse_a_taken_one() {
        let shop = project(&[]);
        let cms = RepositoryKey::new("cms").unwrap();
        assert_eq!(
            shop.new_repository_resource_key(&cms, |_| false).as_str(),
            "shop.cms"
        );
        let taken = ["shop.cms", "shop.cms-2"];
        assert_eq!(
            shop.new_repository_resource_key(&cms, |k| taken.contains(&k.as_str()))
                .as_str(),
            "shop.cms-3"
        );
        let odd = Project::new(ProjectKey::new("My-Proj_2").unwrap(), "x");
        assert_eq!(
            odd.new_repository_resource_key(&cms, |_| false).as_str(),
            "my-proj_2.cms"
        );
    }

    #[test]
    fn copies_get_ids_of_their_own() {
        let mut shop = project(&[("repository", "shop.repository")]);
        shop.materialize_repositories();
        let mut cms = repository("cms", "shop.cms");
        cms.remotes.push(RepositoryRemote {
            remote_url: Some("https://example.com/cms.git".into()),
            primary: true,
            ..remote("origin")
        });
        shop.repositories.push(cms);
        shop.default_repository = Some(shop.repositories[1].id);

        let new_id = ProjectId::new();
        let copy = shop.copy_with_id(new_id);
        assert_eq!(copy.id, new_id);
        assert_eq!(copy.repositories[0].id, RepositoryId::derived(new_id));
        assert_ne!(copy.repositories[1].id, shop.repositories[1].id);
        assert_eq!(copy.default_repository, Some(copy.repositories[1].id));
        assert_eq!(copy.repositories[1].remotes, shop.repositories[1].remotes);
        assert_eq!(copy.resources, shop.resources);
        copy.validate_repositories().unwrap();
        // The same copy made again agrees with itself.
        assert_eq!(shop.copy_with_id(new_id), copy);
    }
}
