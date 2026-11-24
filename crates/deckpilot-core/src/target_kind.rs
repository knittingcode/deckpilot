//! What the target of a key is on this computer, a file or a folder, for
//! the key's automatic icon (`docs/system-actions.md`, "The key's icon").
//!
//! Only filesystem metadata is used: the path is resolved as the action
//! resolves it ([`crate::paths::existing_inside`]: links followed by their
//! metadata, never out of the resource) and asked for its type with
//! `stat`. The target is never read, opened with its application or run
//! to find out. Whatever cannot be told safely is [`TargetKind::Unknown`]:
//! nothing chosen, an unbound key, a project role without a project, a
//! value that is not a local absolute path (a network path, an
//! application id), a missing file, a permission error, a link that leaves
//! its resource, or a lookup that takes too long.
//!
//! Answers are kept in a [`TargetCache`] until its owner clears it at the
//! events that can change them (a profile or layout loads, an action or a
//! resource binding changes); nothing is polled. A file replaced by a
//! folder underneath shows its new icon at the next such event.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::{MachineConfig, Project, ResourceKind, ResourceRef};

/// What a target is on this computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetKind {
    File,
    Directory,
    /// Not known: missing, unreadable, not a local path, or anything that
    /// cannot be told without guessing.
    Unknown,
}

/// Tells what a resource (or a path inside it) is on this computer.
pub trait TargetKinds {
    /// What `relative` inside the resource `reference` names, or without
    /// `relative` the resource itself.
    fn kind_of(&self, reference: &ResourceRef, relative: Option<&str>) -> TargetKind;
}

/// Knows nothing: every target is [`TargetKind::Unknown`]. Where no
/// machine is at hand (tests, previews without a store).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTargets;

impl TargetKinds for NoTargets {
    fn kind_of(&self, _reference: &ResourceRef, _relative: Option<&str>) -> TargetKind {
        TargetKind::Unknown
    }
}

/// Longest wait for one lookup: a path on a mount that does not answer
/// (a sleeping network drive) must not hold the device loop.
const LOOKUP_TIMEOUT: Duration = Duration::from_millis(250);

/// Longest total wait for lookups that timed out between two clears:
/// after it, the remaining targets are unknown until the next clear.
const TIMEOUT_BUDGET: Duration = Duration::from_secs(1);

/// The cache key: the resolved resource value and the relative path.
type Key = (String, Option<String>);

#[derive(Debug, Default)]
struct Cached {
    kinds: HashMap<Key, TargetKind>,
    /// Time spent on lookups that timed out since the last clear.
    waited: Duration,
}

/// Answers of [`MachineTargets`], by resolved path, until [`TargetCache::clear`].
#[derive(Debug, Default)]
pub struct TargetCache {
    cached: Mutex<Cached>,
    /// Lookups whose thread has not answered yet, kept across clears: a
    /// path that hangs is not looked up again (no second thread) until
    /// its first lookup returns.
    pending: Arc<Mutex<HashSet<Key>>>,
}

impl TargetCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets every answer: each target is looked at again when it is
    /// next asked for.
    pub fn clear(&self) {
        *self.lock() = Cached::default();
    }

    /// The targets of `machine`'s bindings, project roles resolved for
    /// `project`, answered through this cache.
    pub fn on<'a>(
        &'a self,
        machine: &'a MachineConfig,
        project: Option<&'a Project>,
    ) -> MachineTargets<'a> {
        MachineTargets {
            machine,
            project,
            cache: self,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Cached> {
        self.cached.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn kind(&self, base: &str, relative: Option<&str>) -> TargetKind {
        let key = (base.to_owned(), relative.map(str::to_owned));
        {
            let cached = self.lock();
            if let Some(kind) = cached.kinds.get(&key) {
                return *kind;
            }
            if cached.waited >= TIMEOUT_BUDGET {
                return TargetKind::Unknown;
            }
        }
        // Not under the lock: another caller may ask meanwhile.
        let (kind, timed_out) = lookup_within(&key, &self.pending, LOOKUP_TIMEOUT);
        let mut cached = self.lock();
        if timed_out {
            cached.waited += LOOKUP_TIMEOUT;
        }
        cached.kinds.insert(key, kind);
        kind
    }
}

/// The targets of one machine's bindings ([`TargetCache::on`]).
#[derive(Debug, Clone, Copy)]
pub struct MachineTargets<'a> {
    machine: &'a MachineConfig,
    project: Option<&'a Project>,
    cache: &'a TargetCache,
}

impl TargetKinds for MachineTargets<'_> {
    fn kind_of(&self, reference: &ResourceRef, relative: Option<&str>) -> TargetKind {
        let Ok(binding) = self.machine.resolve(reference, self.project, None) else {
            return TargetKind::Unknown;
        };
        // A chosen file is bound as `other`, a folder as `directory`;
        // applications, URLs and the rest are not files to look at.
        if !matches!(binding.kind, ResourceKind::Directory | ResourceKind::Other) {
            return TargetKind::Unknown;
        }
        let base = binding.value.trim();
        if !is_local_absolute(base) {
            return TargetKind::Unknown;
        }
        let relative = relative.map(str::trim).filter(|r| !r.is_empty());
        // Text with a `{{ … }}` reference is known only when it runs.
        if relative.is_some_and(|r| r.contains("{{")) {
            return TargetKind::Unknown;
        }
        self.cache.kind(base, relative)
    }
}

/// An absolute path on this computer: not empty, not a network path
/// (`\\server\share`, `\\?\…`, `//server/…`), which could reach another
/// computer.
fn is_local_absolute(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with("\\\\")
        && !value.starts_with("//")
        && Path::new(value).is_absolute()
}

/// [`lookup`] on a thread of its own, waiting at most `timeout`; with
/// whether it timed out (the answer is then unknown, and the thread ends
/// whenever the system answers). A key whose earlier lookup has not
/// returned yet (`pending`) is unknown at once, without another thread.
fn lookup_within(
    key: &Key,
    pending: &Arc<Mutex<HashSet<Key>>>,
    timeout: Duration,
) -> (TargetKind, bool) {
    let claim = |pending: &Mutex<HashSet<Key>>| {
        pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone())
    };
    if !claim(pending) {
        return (TargetKind::Unknown, false);
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let owned = key.clone();
    let done = Arc::clone(pending);
    let spawned = std::thread::Builder::new()
        .name("deckpilot-target-kind".into())
        .spawn(move || {
            let kind = lookup(&owned.0, owned.1.as_deref());
            done.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&owned);
            let _ = sender.send(kind);
        });
    if spawned.is_err() {
        pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
        return (TargetKind::Unknown, false);
    }
    match receiver.recv_timeout(timeout) {
        Ok(kind) => (kind, false),
        Err(_) => (TargetKind::Unknown, true),
    }
}

/// What `relative` inside `base` (or `base` itself) is, by metadata only,
/// resolved as the Open action resolves it: links followed, never out of
/// the resource.
pub fn lookup(base: &str, relative: Option<&str>) -> TargetKind {
    let Ok(path) = crate::paths::existing_inside(base, relative, false) else {
        return TargetKind::Unknown;
    };
    // `metadata` is `stat`: links resolved by their metadata, nothing read.
    match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_dir() => {
            if opens_as_application(&path) {
                TargetKind::Unknown
            } else {
                TargetKind::Directory
            }
        }
        Ok(metadata) if metadata.is_file() => TargetKind::File,
        // Sockets, devices, pipes; errors (permission denied, gone).
        _ => TargetKind::Unknown,
    }
}

/// A macOS application bundle is a folder on disk but starts as an
/// application: neither a document nor a folder to show.
fn opens_as_application(path: &Path) -> bool {
    cfg!(target_os = "macos")
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
}

#[cfg(test)]
mod tests;
