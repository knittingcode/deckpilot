//! `deckpilot.fs` (design §5.3, `docs/scripts-bridge.md`): every operation
//! inside one of the script's granted scopes, relative to the scope's open
//! root ([`resolve`]), with these rules:
//!
//! * **Reads** (`exists`, `stat`, `list`, `readFile`) may pass through a
//!   relative link that stays inside the scope; `cap-std` refuses one that
//!   leads out of it, and every absolute link (`symlinkEscape`). `exists`
//!   and `stat` never follow the final name; `list` lists the folder a
//!   relative link inside the scope leads to, and names entries without
//!   following them.
//! * **Private** folders of DeckPilot (its data, IPC token, logs,
//!   configuration, caches, the app) are never read or changed, also
//!   through a link ([`protected::Protected::forbids`]).
//! * **Changes** (`mkdir`, `writeFile`, `fs.write.*`, `copy`, `move`,
//!   `remove`, `removeContents`) never pass through a link at all: each
//!   folder of the path is opened without following it, so the place
//!   changed is the place named, below the root. A link is removed,
//!   replaced or copied as a link.
//! * **Protected** ([`protected`]): no change to a path inside a `.git`
//!   folder (OD-6); no change that removes, empties or replaces a
//!   protected root, a repository's folder or a tree holding a `.git`
//!   folder; a scope's root is never removed or replaced, and emptied only
//!   with `rootDestructive`.
//! * **Destructive** (`remove` of a folder, `removeContents`, `copy` or
//!   `move` over an existing target): `delete` access (`write` for an
//!   overwrite), the plan computed by the agent, and with
//!   `destructive: "ask"` the confirmation token of `confirm.rs`, only in
//!   a run the user started. `dryRun` returns the plan with read access
//!   and changes nothing. The plan is computed again when the operation
//!   runs, and refused when it grew.
//! * **Limits**: a file read or written is at most 4 MiB, `list` at most
//!   10,000 entries, a removal at most 100,000, `copy` at most 1 GiB per
//!   run, chunked writes at most 4 at once.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read as _, Seek as _, Write as _};

use cap_fs_ext::{DirExt as _, FollowSymlinks, OpenOptionsFollowExt as _};
use cap_std::fs::{Dir, File, OpenOptions};
use deckpilot_core::script_grants::{Destructive, FsAccess};
use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_abi::scripts::{
    Encoding, EntryKind, FsCopy, FsEntry, FsMkdir, FsMove, FsRead, FsReadFile, FsRemove,
    FsRemoveContents, FsStat, FsWriteChunk, FsWriteFile, FsWriteHandle, FsWriteOpen, FsWriteOpened,
    MAX_PLAN_SAMPLE, MAX_READ_CHUNK, MAX_SCRIPT_FILE, PathArg, RemovalPlan, ScriptFault, codes,
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::confirm::{grew, needs_confirmation};
use super::{Call, Notes, RunProject, fault, random_hex, to_value};

pub(crate) mod protected;
pub(crate) mod resolve;
#[cfg(test)]
mod tests;

use resolve::{Target, failure, resolve};

/// Entries one `fs.list` returns at most.
pub(crate) const MAX_LIST: usize = 10_000;
/// Entries one removal (or overwrite) removes at most.
pub(crate) const MAX_REMOVE: u64 = 100_000;
/// Bytes `fs.copy` copies per run at most.
pub(crate) const MAX_COPY_BYTES: u64 = 1 << 30;
/// Room the entries of one `fs.list` reply may take.
const LIST_REPLY_ROOM: usize = 900 << 10;
/// Chunked writes one run may have open.
pub(crate) const MAX_OPEN_WRITES: usize = 4;
/// Folder depth of a tree the agent walks.
const MAX_DEPTH: usize = 48;
/// Room the JSON of a read reply may take (the 1 MiB reply cap, less the
/// envelope).
const READ_REPLY_ROOM: usize = (1 << 20) - 4096;

/// Whether a name is `.git` (OD-6), as the file system compares it.
fn is_git(name: &str) -> bool {
    name.eq_ignore_ascii_case(".git")
}

fn kind_of(ft: cap_std::fs::FileType) -> EntryKind {
    if ft.is_symlink() {
        EntryKind::Symlink
    } else if ft.is_dir() {
        EntryKind::Directory
    } else if ft.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

fn modified_ms(meta: &cap_std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.into_std().duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The run's project, as the scopes need it.
fn project(call: &Call<'_>) -> Result<Option<RunProject>, ScriptFault> {
    call.project()
}

/// The fs operations.
pub(super) fn call(
    call: &Call<'_>,
    request: &CapabilityRequest,
    notes: &mut Notes,
) -> Result<Value, ScriptFault> {
    use CapabilityRequest as R;
    let project = project(call)?;
    let fs = Fs {
        call,
        project: project.as_ref(),
    };
    match request {
        R::FsExists(p) => fs.exists(&p.path, notes),
        R::FsStat(p) => fs.stat(&p.path, notes),
        R::FsList(p) => fs.list(&p.path, notes),
        R::FsMkdir(m) => fs.mkdir(m, notes),
        R::FsReadFile(r) => fs.read_file(r, notes),
        R::FsWriteFile(w) => fs.write_file(w, notes),
        R::FsWriteOpen(o) => fs.write_open(o, notes),
        R::FsWriteChunk(c) => fs.write_chunk(c, notes),
        R::FsWriteCommit(h) => fs.write_commit(h, notes),
        R::FsWriteAbort(h) => fs.write_abort(h),
        R::FsCopy(c) => fs.copy(c, notes),
        R::FsMove(m) => fs.move_(m, notes),
        R::FsRemove(r) => fs.remove(r, notes),
        R::FsRemoveContents(r) => fs.remove_contents(r, notes),
        other => Err(fault(
            codes::INVALID_ARGUMENT,
            format!("{} is not a file system operation", other.op()),
        )),
    }
}

struct Fs<'a> {
    call: &'a Call<'a>,
    project: Option<&'a RunProject>,
}

/// Opens `names` below `dir`, never following a link.
fn open_nofollow(dir: &Dir, names: &[String], shown: &str) -> Result<Dir, ScriptFault> {
    let mut current = dir.try_clone().map_err(|e| failure(&e, shown))?;
    for name in names {
        let meta = current
            .symlink_metadata(name)
            .map_err(|e| failure(&e, shown))?;
        if meta.file_type().is_symlink() {
            return Err(fault(
                codes::SYMLINK_ESCAPE,
                format!("“{shown}” goes through a link; changes never follow links"),
            ));
        }
        if !meta.is_dir() {
            return Err(fault(
                codes::INVALID_ARGUMENT,
                format!("“{shown}”: a part of the path is not a folder"),
            ));
        }
        current = current.open_dir_nofollow(name).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                failure(&e, shown)
            } else {
                fault(
                    codes::SYMLINK_ESCAPE,
                    format!("“{shown}” changed while it was opened"),
                )
            }
        })?;
    }
    Ok(current)
}

/// The parent folder of a change's target (no link followed) and its
/// final name.
fn parent_of(t: &Target) -> Result<(Dir, String), ScriptFault> {
    let Some((name, parents)) = t.rel.split_last() else {
        return Err(fault(
            codes::PROTECTED_PATH,
            format!(
                "“{}” is the root of a scope and is never replaced or removed",
                t.shown()
            ),
        ));
    };
    Ok((open_nofollow(&t.root, parents, &t.shown())?, name.clone()))
}

/// A tree's plan: what removing it removes.
#[derive(Debug, Default, Clone)]
struct Plan {
    files: u64,
    directories: u64,
    bytes: u64,
    sample: Vec<String>,
    /// One hash per entry (its path, kind, size, file id and time): the
    /// confirmation is bound to exactly these entries, not only to their
    /// counts (security review of #469, M3).
    entries: Vec<[u8; 32]>,
}

impl Plan {
    fn entries(&self) -> u64 {
        self.files + self.directories
    }

    fn count(
        &mut self,
        path: String,
        dir: bool,
        bytes: u64,
        identity: (u64, u64),
    ) -> Result<(), ScriptFault> {
        let mut hash = Sha256::new();
        hash.update(path.as_bytes());
        hash.update([0, u8::from(dir)]);
        hash.update(bytes.to_le_bytes());
        hash.update(identity.0.to_le_bytes());
        hash.update(identity.1.to_le_bytes());
        self.entries.push(hash.finalize().into());
        if dir {
            self.directories += 1;
        } else {
            self.files += 1;
            self.bytes = self.bytes.saturating_add(bytes);
        }
        if self.sample.len() < MAX_PLAN_SAMPLE {
            self.sample.push(path);
        }
        if self.entries() > MAX_REMOVE {
            return Err(fault(
                codes::LIMIT,
                format!("more than {MAX_REMOVE} files and folders; remove less at a time"),
            ));
        }
        Ok(())
    }

    /// What the confirmation is bound to: every entry, in a fixed order.
    fn fingerprint(&self) -> [u8; 32] {
        let mut entries = self.entries.clone();
        entries.sort_unstable();
        let mut hash = Sha256::new();
        for e in &entries {
            hash.update(e);
        }
        hash.finalize().into()
    }

    fn removal(self, root: String, dry_run: bool, removed: bool) -> RemovalPlan {
        RemovalPlan {
            dry_run,
            root,
            files: self.files,
            directories: self.directories,
            bytes: self.bytes,
            sample: self.sample,
            removed,
        }
    }
}

/// An entry's file id and modification time, as far as the system says.
fn identity(meta: &cap_std::fs::Metadata) -> (u64, u64) {
    #[cfg(unix)]
    let id = cap_fs_ext::MetadataExt::ino(meta);
    #[cfg(not(unix))]
    let id = 0;
    (id, modified_ms(meta))
}

fn name_of(name: &OsString) -> String {
    name.to_string_lossy().into_owned()
}

/// Walks the entries of `dir` (but `keep`), never following a link:
/// counts them, and refuses a tree that holds a `.git` folder. `keep` may
/// spare a `.git` only where `keep_git` says so (emptying a repository's
/// own root with `rootDestructive`).
fn plan_dir(
    dir: &Dir,
    prefix: &str,
    keep: &[String],
    keep_git: bool,
    plan: &mut Plan,
    depth: usize,
) -> Result<(), ScriptFault> {
    if depth > MAX_DEPTH {
        return Err(fault(codes::LIMIT, "the folders are nested too deeply"));
    }
    let entries = dir.entries().map_err(|e| failure(&e, prefix))?;
    for entry in entries {
        let entry = entry.map_err(|e| failure(&e, prefix))?;
        let name = name_of(&entry.file_name());
        if keep.contains(&name) && (keep_git || !is_git(&name)) {
            continue;
        }
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if is_git(&name) {
            return Err(fault(
                codes::PROTECTED_PATH,
                format!("“{path}” is a .git folder, which is never removed"),
            ));
        }
        let ft = entry.file_type().map_err(|e| failure(&e, &path))?;
        let meta = entry.metadata().map_err(|e| failure(&e, &path))?;
        if ft.is_dir() && !ft.is_symlink() {
            plan.count(path.clone(), true, 0, identity(&meta))?;
            let sub = dir
                .open_dir_nofollow(entry.file_name())
                .map_err(|e| failure(&e, &path))?;
            plan_dir(&sub, &path, &[], false, plan, depth + 1)?;
        } else {
            plan.count(path, false, meta.len(), identity(&meta))?;
        }
    }
    Ok(())
}

/// Removes the entries of `dir` (but `keep`), never following a link, and
/// stops when more would go than `budget` (the confirmed plan) allows.
fn empty_dir(
    dir: &Dir,
    keep: &[String],
    keep_git: bool,
    budget: &mut u64,
    depth: usize,
) -> Result<(), ScriptFault> {
    if depth > MAX_DEPTH {
        return Err(fault(codes::LIMIT, "the folders are nested too deeply"));
    }
    let changed = || {
        fault(
            codes::LIMIT,
            "the folder changed while it was removed; nothing more was removed",
        )
    };
    let entries = dir.entries().map_err(|e| failure(&e, "a folder"))?;
    for entry in entries {
        let entry = entry.map_err(|e| failure(&e, "a folder"))?;
        let name = entry.file_name();
        let shown = name_of(&name);
        if keep.contains(&shown) && (keep_git || !is_git(&shown)) {
            continue;
        }
        if is_git(&shown) {
            return Err(fault(
                codes::PROTECTED_PATH,
                "a .git folder appeared; nothing more was removed",
            ));
        }
        *budget = budget.checked_sub(1).ok_or_else(changed)?;
        let ft = entry.file_type().map_err(|e| failure(&e, &shown))?;
        if ft.is_dir() && !ft.is_symlink() {
            let sub = dir
                .open_dir_nofollow(&name)
                .map_err(|e| failure(&e, &shown))?;
            empty_dir(&sub, &[], false, budget, depth + 1)?;
            // Closed first: Windows removes no folder that is open.
            drop(sub);
            dir.remove_dir(&name).map_err(|e| failure(&e, &shown))?;
        } else {
            dir.remove_file_or_symlink(&name)
                .map_err(|e| failure(&e, &shown))?;
        }
    }
    Ok(())
}

/// The plan of removing `name` inside `parent` (a file, a link, or a
/// folder with everything in it).
fn plan_entry(parent: &Dir, name: &str, shown: &str) -> Result<(Plan, bool), ScriptFault> {
    let meta = parent
        .symlink_metadata(name)
        .map_err(|e| failure(&e, shown))?;
    let mut plan = Plan::default();
    if meta.is_dir() && !meta.file_type().is_symlink() {
        let dir = parent
            .open_dir_nofollow(name)
            .map_err(|e| failure(&e, shown))?;
        plan_dir(&dir, "", &[], false, &mut plan, 0)?;
        plan.count(String::new(), true, 0, identity(&meta))?;
        plan.sample.retain(|s| !s.is_empty());
        Ok((plan, true))
    } else {
        plan.count(name.to_owned(), false, meta.len(), identity(&meta))?;
        Ok((plan, false))
    }
}

/// Removes `name` inside `parent` as planned (`budget`: its entries).
fn remove_entry(parent: &Dir, name: &str, dir: bool, mut budget: u64) -> Result<(), ScriptFault> {
    if dir {
        let sub = parent
            .open_dir_nofollow(name)
            .map_err(|e| failure(&e, name))?;
        empty_dir(&sub, &[], false, &mut budget, 0)?;
        drop(sub);
        parent.remove_dir(name).map_err(|e| failure(&e, name))
    } else {
        parent
            .remove_file_or_symlink(name)
            .map_err(|e| failure(&e, name))
    }
}

/// What a destructive operation is bound to: the op, the resolved paths
/// and the options.
fn digest(op: &str, parts: &[&str]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(op.as_bytes());
    for part in parts {
        hash.update([0]);
        hash.update(part.as_bytes());
    }
    hash.finalize().into()
}

fn key_of(t: &Target) -> String {
    t.absolute().to_string_lossy().into_owned()
}

impl Fs<'_> {
    fn resolve(
        &self,
        path: &PathArg,
        need: FsAccess,
        notes: &mut Notes,
    ) -> Result<Target, ScriptFault> {
        let target = resolve(
            path,
            &self.call.grants,
            self.project,
            need,
            &self.call.run.protected,
        )?;
        notes.add("scope", target.scope.label.clone());
        Ok(target)
    }

    /// The rules of every change of `t`.
    fn may_change(&self, t: &Target) -> Result<(), ScriptFault> {
        if t.rel.iter().any(|n| is_git(n)) {
            return Err(fault(
                codes::PROTECTED_PATH,
                format!(
                    "“{}” is inside a .git folder, which scripts never change",
                    t.shown()
                ),
            ));
        }
        if self.call.run.protected.guards(&t.absolute()) {
            return Err(fault(
                codes::PROTECTED_PATH,
                format!("“{}” is a protected folder", t.shown()),
            ));
        }
        Ok(())
    }

    /// The rules of a change that removes, empties or replaces `t` as a
    /// whole (`emptying`: `removeContents`, which may empty a scope's root
    /// with `rootDestructive`).
    fn may_destroy(&self, t: &Target, emptying: bool) -> Result<(), ScriptFault> {
        self.may_change(t)?;
        let target = t.absolute();
        if t.rel.is_empty() && !(emptying && t.scope.root_destructive) {
            return Err(fault(
                codes::PROTECTED_PATH,
                format!(
                    "“{}” is the root of a scope; it is never removed, and emptied only with rootDestructive",
                    t.shown()
                ),
            ));
        }
        let own_root = emptying && t.rel.is_empty() && t.scope.repository;
        let repository = self
            .project
            .iter()
            .flat_map(|p| &p.repositories)
            .filter_map(|r| r.folder.clone())
            .any(|folder| protected::starts_with(&protected::simplify(folder), &target));
        if repository && !own_root {
            return Err(fault(
                codes::PROTECTED_PATH,
                format!("“{}” is or holds a repository's folder", t.shown()),
            ));
        }
        Ok(())
    }

    /// The gate of a destructive operation with its `plan`: allowed, or a
    /// token to confirm, or refused.
    fn confirm(
        &self,
        op: &'static str,
        digest: [u8; 32],
        plan: &Plan,
        removal: &RemovalPlan,
        token: Option<&str>,
        action: &'static str,
    ) -> Result<(), ScriptFault> {
        if self.call.grants.destructive == Destructive::Allow {
            return Ok(());
        }
        self.unattended()?;
        let fingerprint = plan.fingerprint();
        let mut state = self.call.run.state();
        if let Some(token) = token {
            let (confirmed, bound) = state.tokens.take(token, op, &digest)?;
            if grew(&confirmed, removal) || bound != fingerprint {
                let token = state
                    .tokens
                    .issue(op, digest, removal.clone(), fingerprint, action)?;
                return Err(needs_confirmation(
                    token,
                    "the files changed since the confirmation; confirm again",
                ));
            }
            return Ok(());
        }
        let token = state
            .tokens
            .issue(op, digest, removal.clone(), fingerprint, action)?;
        Err(needs_confirmation(
            token,
            format!(
                "{action} {} files and {} folders: confirm first",
                removal.files, removal.directories
            ),
        ))
    }

    /// With `destructive: "ask"`, nothing is removed or replaced in a run
    /// nobody started, not even one file (OD-9; security review of #469,
    /// M2).
    fn unattended(&self) -> Result<(), ScriptFault> {
        if self.call.grants.destructive == Destructive::Ask && !self.call.user_initiated {
            return Err(fault(
                codes::NOT_USER_INITIATED,
                "this removes or replaces files: only in runs you start, or with destructive: allow",
            ));
        }
        Ok(())
    }

    fn exists(&self, path: &PathArg, notes: &mut Notes) -> Result<Value, ScriptFault> {
        let t = self.resolve(path, FsAccess::Read, notes)?;
        let exists = match t.root.symlink_metadata(t.rel_path()) {
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(failure(&e, &t.shown())),
        };
        Ok(json!({ "exists": exists }))
    }

    fn stat(&self, path: &PathArg, notes: &mut Notes) -> Result<Value, ScriptFault> {
        let t = self.resolve(path, FsAccess::Read, notes)?;
        let meta = if t.rel.is_empty() {
            t.root.dir_metadata()
        } else {
            t.root.symlink_metadata(t.rel_path())
        };
        let stat = match meta {
            Ok(meta) => Some(FsStat {
                kind: kind_of(meta.file_type()),
                size: meta.len(),
                modified_ms: modified_ms(&meta),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(failure(&e, &t.shown())),
        };
        Ok(json!({ "stat": stat }))
    }

    fn list(&self, path: &PathArg, notes: &mut Notes) -> Result<Value, ScriptFault> {
        let t = self.resolve(path, FsAccess::Read, notes)?;
        let dir = if t.rel.is_empty() {
            t.root.try_clone()
        } else {
            t.root.open_dir(t.rel_path())
        }
        .map_err(|e| failure(&e, &t.shown()))?;
        let mut entries = Vec::new();
        let mut bytes = 0usize;
        for entry in dir.entries().map_err(|e| failure(&e, &t.shown()))? {
            let entry = entry.map_err(|e| failure(&e, &t.shown()))?;
            // The reply stays within its cap, however long the names.
            if entries.len() == MAX_LIST || bytes > LIST_REPLY_ROOM {
                return Err(fault(
                    codes::LIMIT,
                    format!("“{}” holds more than {MAX_LIST} entries", t.shown()),
                ));
            }
            let kind = entry.file_type().map_or(EntryKind::Other, kind_of);
            let name = name_of(&entry.file_name());
            bytes += name.len() + 32;
            entries.push(FsEntry { name, kind });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        notes.add("entries", entries.len().to_string());
        Ok(json!({ "entries": entries }))
    }

    fn mkdir(&self, m: &FsMkdir, notes: &mut Notes) -> Result<Value, ScriptFault> {
        let t = self.resolve(&m.path, FsAccess::Write, notes)?;
        self.may_change(&t)?;
        let shown = t.shown();
        if t.rel.is_empty() {
            return if m.recursive {
                Ok(json!({}))
            } else {
                Err(fault(codes::EXISTS, format!("“{shown}” exists")))
            };
        }
        let (last, parents) = t.rel.split_last().expect("not empty");
        let parent = if m.recursive {
            // Each missing folder is made, never through a link.
            let mut dir = t.root.try_clone().map_err(|e| failure(&e, &shown))?;
            for name in parents {
                match dir.create_dir(name) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(failure(&e, &shown)),
                }
                dir = open_nofollow(&dir, std::slice::from_ref(name), &shown)?;
            }
            dir
        } else {
            open_nofollow(&t.root, parents, &shown)?
        };
        match parent.create_dir(last) {
            Ok(()) => Ok(json!({})),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let is_dir = parent
                    .symlink_metadata(last)
                    .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink());
                if m.recursive && is_dir {
                    Ok(json!({}))
                } else {
                    Err(fault(codes::EXISTS, format!("“{shown}” exists")))
                }
            }
            Err(e) => Err(failure(&e, &shown)),
        }
    }

    fn read_file(&self, r: &FsReadFile, notes: &mut Notes) -> Result<Value, ScriptFault> {
        let t = self.resolve(&r.path, FsAccess::Read, notes)?;
        let shown = t.shown();
        let mut options = OpenOptions::new();
        options.read(true);
        nonblocking(&mut options);
        let mut file = t
            .root
            .open_with(t.rel_path(), &options)
            .map_err(|e| failure(&e, &shown))?;
        let meta = file.metadata().map_err(|e| failure(&e, &shown))?;
        if !meta.is_file() {
            return Err(fault(
                codes::INVALID_ARGUMENT,
                format!("“{shown}” is not a file"),
            ));
        }
        let size = meta.len();
        let max = r.max_bytes.unwrap_or(MAX_SCRIPT_FILE).min(MAX_SCRIPT_FILE);
        if size > max {
            return Err(ScriptFault {
                path: Some(shown.clone()),
                ..fault(
                    codes::TOO_LARGE,
                    format!("“{shown}” is larger than {max} bytes"),
                )
            });
        }
        let offset = r.offset.unwrap_or(0).min(size);
        let length = match r.length {
            Some(length) => u64::from(length),
            None if size > u64::from(MAX_READ_CHUNK) => {
                return Err(fault(
                    codes::TOO_LARGE,
                    format!(
                        "“{shown}” is larger than one read; read it in chunks (offset, length)"
                    ),
                ));
            }
            None => size,
        };
        let want = length.min(size - offset);
        file.seek(std::io::SeekFrom::Start(offset))
            .map_err(|e| failure(&e, &shown))?;
        let mut bytes = Vec::with_capacity(usize::try_from(want).unwrap_or(0));
        (&mut file)
            .take(want)
            .read_to_end(&mut bytes)
            .map_err(|e| failure(&e, &shown))?;
        let reached_end = offset + bytes.len() as u64 >= size;
        let content = match r.encoding {
            Encoding::Base64 => {
                use base64::Engine as _;
                base64::engine::general_purpose::STANDARD.encode(&bytes)
            }
            Encoding::Utf8 => {
                let text = utf8_prefix(&bytes, reached_end).ok_or_else(|| {
                    fault(
                        codes::INVALID_ARGUMENT,
                        format!("“{shown}” is not UTF-8 text; read it as base64"),
                    )
                })?;
                fit_json(text).to_owned()
            }
        };
        let read = match r.encoding {
            Encoding::Base64 => bytes.len() as u64,
            Encoding::Utf8 => content.len() as u64,
        };
        notes.add("bytes", read.to_string());
        to_value(&FsRead {
            content,
            bytes: read,
            size,
            eof: offset + read >= size,
        })
    }

    fn write_file(&self, w: &FsWriteFile, notes: &mut Notes) -> Result<Value, ScriptFault> {
        let bytes = decode(&w.content, w.encoding)?;
        let (t, parent, name) = self.write_target(&w.path, notes)?;
        let mut pending = Pending::create(parent, name, w.create_only, &t)?;
        pending.write(&bytes)?;
        pending.commit()?;
        notes.add("bytes", bytes.len().to_string());
        Ok(json!({}))
    }

    /// A file to write: resolved with write access, its folder open, its
    /// name; never a folder.
    fn write_target(
        &self,
        path: &PathArg,
        notes: &mut Notes,
    ) -> Result<(Target, Dir, String), ScriptFault> {
        let t = self.resolve(path, FsAccess::Write, notes)?;
        self.may_change(&t)?;
        let (parent, name) = parent_of(&t)?;
        if let Ok(meta) = parent.symlink_metadata(&name) {
            if meta.is_dir() && !meta.file_type().is_symlink() {
                return Err(fault(codes::EXISTS, format!("“{}” is a folder", t.shown())));
            }
            // Replacing a file's content is a replacement too.
            self.unattended()?;
        }
        Ok((t, parent, name))
    }

    fn write_open(&self, o: &FsWriteOpen, notes: &mut Notes) -> Result<Value, ScriptFault> {
        let (t, parent, name) = self.write_target(&o.path, notes)?;
        let mut state = self.call.run.state();
        if state.writes.open.len() >= MAX_OPEN_WRITES {
            return Err(fault(
                codes::LIMIT,
                format!("at most {MAX_OPEN_WRITES} files are written at once"),
            ));
        }
        let pending = Pending::create(parent, name, o.create_only, &t)?;
        let handle = random_hex(16).ok_or_else(|| fault(codes::HOST_ERROR, "no random source"))?;
        state.writes.open.insert(
            handle.clone(),
            OpenWrite {
                pending,
                path: o.path.clone(),
            },
        );
        to_value(&FsWriteOpened { handle })
    }

    /// An open write, its grant checked again (the scope still allows
    /// writing the same file).
    fn with_write<T>(
        &self,
        handle: &str,
        notes: &mut Notes,
        f: impl FnOnce(&mut OpenWrite) -> Result<T, ScriptFault>,
    ) -> Result<T, ScriptFault> {
        let path = {
            let state = self.call.run.state();
            let open = state.writes.open.get(handle).ok_or_else(unknown_handle)?;
            open.path.clone()
        };
        let checked = self
            .resolve(&path, FsAccess::Write, notes)
            .and_then(|t| self.may_change(&t).map(|()| t));
        let mut state = self.call.run.state();
        match checked {
            Ok(t)
                if state
                    .writes
                    .open
                    .get(handle)
                    .is_some_and(|o| o.pending.key == key_of(&t)) =>
            {
                let open = state
                    .writes
                    .open
                    .get_mut(handle)
                    .ok_or_else(unknown_handle)?;
                f(open)
            }
            Ok(_) | Err(_) => {
                // Revoked or moved: the write is abandoned.
                if let Some(open) = state.writes.open.remove(handle) {
                    open.pending.abort();
                }
                Err(checked.err().unwrap_or_else(|| {
                    fault(
                        codes::OUTSIDE_SCOPE,
                        "the file's place changed during the write",
                    )
                }))
            }
        }
    }

    fn write_chunk(&self, c: &FsWriteChunk, notes: &mut Notes) -> Result<Value, ScriptFault> {
        let bytes = decode(&c.content, c.encoding)?;
        self.with_write(&c.handle, notes, |open| open.pending.write(&bytes))?;
        Ok(json!({}))
    }

    fn write_commit(&self, h: &FsWriteHandle, notes: &mut Notes) -> Result<Value, ScriptFault> {
        self.with_write(&h.handle, notes, |_| Ok(()))?;
        let open = self
            .call
            .run
            .state()
            .writes
            .open
            .remove(&h.handle)
            .ok_or_else(unknown_handle)?;
        notes.add("bytes", open.pending.written.to_string());
        open.pending.commit()?;
        Ok(json!({}))
    }

    fn write_abort(&self, h: &FsWriteHandle) -> Result<Value, ScriptFault> {
        let open = self
            .call
            .run
            .state()
            .writes
            .open
            .remove(&h.handle)
            .ok_or_else(unknown_handle)?;
        open.pending.abort();
        Ok(json!({}))
    }

    fn remove(&self, r: &FsRemove, notes: &mut Notes) -> Result<Value, ScriptFault> {
        let need = if r.dry_run {
            FsAccess::Read
        } else {
            FsAccess::Delete
        };
        let t = self.resolve(&r.path, need, notes)?;
        self.may_destroy(&t, false)?;
        let (parent, name) = parent_of(&t)?;
        let (plan, dir) = plan_entry(&parent, &name, &t.shown())?;
        if dir && !r.recursive && plan.entries() > 1 {
            return Err(fault(
                codes::INVALID_ARGUMENT,
                format!(
                    "“{}” is a folder that is not empty; pass recursive",
                    t.shown()
                ),
            ));
        }
        notes.add("files", plan.files.to_string());
        notes.add("directories", plan.directories.to_string());
        let removal = plan.clone().removal(t.shown(), r.dry_run, false);
        if r.dry_run {
            return to_value(&removal);
        }
        if dir && plan.entries() > 1 {
            let d = digest("fs.remove", &[&key_of(&t), "recursive"]);
            self.confirm(
                "fs.remove",
                d,
                &plan,
                &removal,
                r.confirmation.as_deref(),
                "Delete",
            )?;
        } else {
            self.unattended()?;
        }
        remove_entry(&parent, &name, dir, plan.entries())?;
        to_value(&plan.removal(t.shown(), false, true))
    }

    fn remove_contents(
        &self,
        r: &FsRemoveContents,
        notes: &mut Notes,
    ) -> Result<Value, ScriptFault> {
        let need = if r.dry_run {
            FsAccess::Read
        } else {
            FsAccess::Delete
        };
        let t = self.resolve(&r.path, need, notes)?;
        self.may_destroy(&t, true)?;
        let dir = open_nofollow(&t.root, &t.rel, &t.shown())?;
        let mut keep = r.keep.clone();
        keep.sort();
        keep.dedup();
        // `keep` spares a `.git` only at a repository's own root.
        let keep_git = t.rel.is_empty() && t.scope.repository;
        let mut plan = Plan::default();
        plan_dir(&dir, "", &keep, keep_git, &mut plan, 0)?;
        notes.add("files", plan.files.to_string());
        notes.add("directories", plan.directories.to_string());
        let removal = plan.clone().removal(t.shown(), r.dry_run, false);
        if r.dry_run || plan.entries() == 0 {
            return to_value(&removal);
        }
        let d = digest("fs.removeContents", &[&key_of(&t), &keep.join("\0")]);
        self.confirm(
            "fs.removeContents",
            d,
            &plan,
            &removal,
            r.confirmation.as_deref(),
            "Delete",
        )?;
        let mut budget = plan.entries();
        empty_dir(&dir, &keep, keep_git, &mut budget, 0)?;
        to_value(&plan.removal(t.shown(), false, true))
    }

    /// `src` and `dst` of a copy or a move, and what an overwrite of `dst`
    /// removes (`None`: nothing is there).
    fn pair(
        &self,
        src: &PathArg,
        dst: &PathArg,
        src_access: FsAccess,
        notes: &mut Notes,
    ) -> Result<Pair, ScriptFault> {
        let s = self.resolve(src, src_access, notes)?;
        let d = self.resolve(dst, FsAccess::Write, notes)?;
        self.may_change(&d)?;
        let src_parent = parent_of(&s)?;
        let dst_parent = parent_of(&d)?;
        if protected::starts_with(&d.absolute(), &s.absolute()) {
            return Err(fault(
                codes::INVALID_ARGUMENT,
                "the target is inside the source",
            ));
        }
        let existing = match dst_parent.0.symlink_metadata(&dst_parent.1) {
            Ok(_) => Some(plan_entry(&dst_parent.0, &dst_parent.1, &d.shown())?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(failure(&e, &d.shown())),
        };
        Ok((s, src_parent, d, dst_parent, existing))
    }

    /// An overwrite of `d`: allowed only with `overwrite`, under the rules
    /// of a removal, confirmed like one.
    fn overwrite(
        &self,
        op: &'static str,
        s: &Target,
        d: &Target,
        existing: &(Plan, bool),
        overwrite: bool,
        token: Option<&str>,
    ) -> Result<(), ScriptFault> {
        if !overwrite {
            return Err(ScriptFault {
                path: Some(d.shown()),
                ..fault(
                    codes::EXISTS,
                    format!("“{}” exists; pass overwrite", d.shown()),
                )
            });
        }
        self.may_destroy(d, false)?;
        // Replacing a folder that is not empty removes what it holds.
        if existing.1 && existing.0.entries() > 1 && d.scope.access < FsAccess::Delete {
            return Err(fault(
                codes::NOT_GRANTED,
                format!(
                    "replacing the folder “{}” deletes what it holds; that needs delete access",
                    d.shown()
                ),
            ));
        }
        let removal = existing.0.clone().removal(d.shown(), false, false);
        let dg = digest(op, &[&key_of(s), &key_of(d), "overwrite"]);
        self.confirm(op, dg, &existing.0, &removal, token, "Overwrite")
    }

    fn copy(&self, c: &FsCopy, notes: &mut Notes) -> Result<Value, ScriptFault> {
        let (s, (sp, sn), d, (dp, dn), existing) =
            self.pair(&c.src, &c.dst, FsAccess::Read, notes)?;
        let meta = sp
            .symlink_metadata(&sn)
            .map_err(|e| failure(&e, &s.shown()))?;
        if meta.is_dir() && !meta.file_type().is_symlink() && !c.recursive {
            return Err(fault(
                codes::INVALID_ARGUMENT,
                format!("“{}” is a folder; pass recursive", s.shown()),
            ));
        }
        // What the copy will write, checked before anything is removed.
        let mut plan = Plan::default();
        if meta.is_dir() && !meta.file_type().is_symlink() {
            let dir = sp
                .open_dir_nofollow(&sn)
                .map_err(|e| failure(&e, &s.shown()))?;
            plan_dir(&dir, "", &[], false, &mut plan, 0)?;
        } else {
            plan.bytes = meta.len();
        }
        let budget = MAX_COPY_BYTES.saturating_sub(self.call.run.state().copied);
        if plan.bytes > budget {
            return Err(fault(codes::LIMIT, "a run copies at most 1 GiB"));
        }
        if let Some(existing) = &existing {
            self.overwrite(
                "fs.copy",
                &s,
                &d,
                existing,
                c.overwrite,
                c.confirmation.as_deref(),
            )?;
            remove_entry(&dp, &dn, existing.1, existing.0.entries())?;
        }
        let mut copied = 0u64;
        let mut at: Vec<String> = d.rel.clone();
        at.pop();
        let result = copy_entry(&sp, &sn, &dp, &dn, &at, &mut copied, budget, 0);
        self.call.run.state().copied += copied;
        notes.add("bytes", copied.to_string());
        result.map(|()| json!({}))
    }

    fn move_(&self, m: &FsMove, notes: &mut Notes) -> Result<Value, ScriptFault> {
        let (s, (sp, sn), d, (dp, dn), existing) =
            self.pair(&m.src, &m.dst, FsAccess::Write, notes)?;
        // The source leaves its place: the rules of a removal.
        self.may_destroy(&s, false)?;
        let (src_plan, _) = plan_entry(&sp, &sn, &s.shown())?;
        notes.add("files", src_plan.files.to_string());
        if let Some(existing) = &existing {
            self.overwrite(
                "fs.move",
                &s,
                &d,
                existing,
                m.overwrite,
                m.confirmation.as_deref(),
            )?;
            remove_entry(&dp, &dn, existing.1, existing.0.entries())?;
        }
        sp.rename(&sn, &dp, &dn).map_err(|e| {
            if e.raw_os_error() == Some(18) || e.kind() == std::io::ErrorKind::CrossesDevices {
                fault(
                    codes::INVALID_ARGUMENT,
                    "the target is on another volume; copy and remove instead",
                )
            } else {
                failure(&e, &s.shown())
            }
        })?;
        Ok(json!({}))
    }
}

/// A copy's or a move's source and target, each with its open folder and
/// name, and what an overwrite of the target removes (`None`: nothing is
/// there; `bool`: a folder).
type Pair = (
    Target,
    (Dir, String),
    Target,
    (Dir, String),
    Option<(Plan, bool)>,
);

/// Whether a link `target`, placed in the folder `at` (names below the
/// scope's root), leads to a place inside the scope: relative, and never
/// above the root.
fn link_stays_inside(at: &[String], target: &std::path::Path) -> bool {
    use std::path::Component;
    // Only `../../name/…`: every `..` before the first name, so no earlier
    // link can make a later `..` climb more than counted.
    let mut ups = 0usize;
    let mut named = false;
    for component in target.components() {
        match component {
            Component::Normal(_) => named = true,
            Component::ParentDir if !named => ups += 1,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    ups <= at.len()
}

/// Opens without blocking: a FIFO or a device never blocks the run.
fn nonblocking(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt as _;
        #[cfg(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))]
        const O_NONBLOCK: i32 = 0x0004;
        #[cfg(all(target_os = "linux", any(target_arch = "mips", target_arch = "mips64")))]
        const O_NONBLOCK: i32 = 0o200;
        #[cfg(not(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly",
            all(target_os = "linux", any(target_arch = "mips", target_arch = "mips64"))
        )))]
        const O_NONBLOCK: i32 = 0o4000;
        options.custom_flags(O_NONBLOCK);
    }
    #[cfg(not(unix))]
    let _ = options;
}

/// Copies `name` of `src` to `to` in `dst`, never following a link (a link
/// is copied as a link on Unix), at most `budget` bytes.
#[expect(clippy::too_many_arguments, reason = "one step of a recursive copy")]
fn copy_entry(
    src: &Dir,
    name: &str,
    dst: &Dir,
    to: &str,
    at: &[String],
    copied: &mut u64,
    budget: u64,
    depth: usize,
) -> Result<(), ScriptFault> {
    if depth > MAX_DEPTH {
        return Err(fault(codes::LIMIT, "the folders are nested too deeply"));
    }
    if is_git(name) {
        return Err(fault(
            codes::PROTECTED_PATH,
            "a .git folder is never copied",
        ));
    }
    let meta = src.symlink_metadata(name).map_err(|e| failure(&e, name))?;
    let ft = meta.file_type();
    if ft.is_symlink() {
        #[cfg(unix)]
        {
            let target = src
                .read_link_contents(name)
                .map_err(|e| failure(&e, name))?;
            // Only a relative link that still leads inside the target's
            // scope from its new place (security review of #469, M1).
            if !link_stays_inside(at, &target) {
                return Err(fault(
                    codes::SYMLINK_ESCAPE,
                    format!(
                        "“{name}” is a link that would lead outside its scope; it is not copied"
                    ),
                ));
            }
            return dst
                .symlink_contents(target, to)
                .map_err(|e| failure(&e, to));
        }
        #[cfg(not(unix))]
        return Err(fault(
            codes::INVALID_ARGUMENT,
            "links are not copied on this system",
        ));
    }
    if ft.is_dir() {
        dst.create_dir(to).map_err(|e| failure(&e, to))?;
        let from = src.open_dir_nofollow(name).map_err(|e| failure(&e, name))?;
        let into = dst.open_dir_nofollow(to).map_err(|e| failure(&e, to))?;
        let mut inside = at.to_vec();
        inside.push(to.to_owned());
        for entry in from.entries().map_err(|e| failure(&e, name))? {
            let entry = entry.map_err(|e| failure(&e, name))?;
            let child = name_of(&entry.file_name());
            copy_entry(
                &from,
                &child,
                &into,
                &child,
                &inside,
                copied,
                budget,
                depth + 1,
            )?;
        }
        return Ok(());
    }
    if !ft.is_file() {
        return Err(fault(
            codes::INVALID_ARGUMENT,
            format!("“{name}” is not a file or folder"),
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    nonblocking(&mut options);
    let mut input = src
        .open_with(name, &options)
        .map_err(|e| failure(&e, name))?;
    // A FIFO swapped in since the check is never read.
    if !input.metadata().is_ok_and(|m| m.is_file()) {
        return Err(fault(
            codes::INVALID_ARGUMENT,
            format!("“{name}” is not a file"),
        ));
    }
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let mut output = dst.open_with(to, &options).map_err(|e| failure(&e, to))?;
    let left = budget.saturating_sub(*copied);
    let n = std::io::copy(&mut (&mut input).take(left + 1), &mut output)
        .map_err(|e| failure(&e, to))?;
    if n > left {
        return Err(fault(codes::LIMIT, "a run copies at most 1 GiB"));
    }
    *copied += n;
    Ok(())
}

fn unknown_handle() -> ScriptFault {
    fault(codes::INVALID_ARGUMENT, "not an open write of this run")
}

fn decode(content: &str, encoding: Encoding) -> Result<Vec<u8>, ScriptFault> {
    match encoding {
        Encoding::Utf8 => Ok(content.as_bytes().to_vec()),
        Encoding::Base64 => {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(content.as_bytes())
                .map_err(|_| fault(codes::INVALID_ARGUMENT, "the content is not base64"))
        }
    }
}

/// The longest UTF-8 prefix of `bytes`: a character cut by the chunk's end
/// is left for the next read (unless the file ends there).
fn utf8_prefix(bytes: &[u8], at_end: bool) -> Option<&str> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text),
        Err(e) if e.error_len().is_none() && !at_end && bytes.len() - e.valid_up_to() < 4 => {
            std::str::from_utf8(&bytes[..e.valid_up_to()]).ok()
        }
        Err(_) => None,
    }
}

/// `text`, shortened (on a character boundary) until its JSON fits the
/// reply: text that JSON escapes heavily is read in smaller pieces.
fn fit_json(mut text: &str) -> &str {
    while serde_json::to_string(text).map_or(usize::MAX, |j| j.len()) > READ_REPLY_ROOM {
        let mut end = text.len() / 2;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text = &text[..end];
    }
    text
}

/// A file being written: a temporary file next to its target, renamed
/// over it (or linked to it, `createOnly`) on commit, removed otherwise.
pub(crate) struct Pending {
    parent: Dir,
    name: String,
    temp: String,
    file: Option<File>,
    written: u64,
    create_only: bool,
    /// The target's absolute path, to recognize it again.
    key: String,
}

impl Pending {
    fn create(
        parent: Dir,
        name: String,
        create_only: bool,
        t: &Target,
    ) -> Result<Self, ScriptFault> {
        if create_only && parent.symlink_metadata(&name).is_ok() {
            return Err(fault(codes::EXISTS, format!("“{}” exists", t.shown())));
        }
        let suffix = random_hex(6).ok_or_else(|| fault(codes::HOST_ERROR, "no random source"))?;
        let short: String = name.chars().take(64).collect();
        let temp = format!(".{short}.deckpilot-{suffix}.tmp");
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let file = parent
            .open_with(&temp, &options)
            .map_err(|e| failure(&e, &t.shown()))?;
        Ok(Self {
            parent,
            name,
            temp,
            file: Some(file),
            written: 0,
            create_only,
            key: key_of(t),
        })
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), ScriptFault> {
        if self.written + bytes.len() as u64 > MAX_SCRIPT_FILE {
            return Err(fault(
                codes::TOO_LARGE,
                "a file written by a script is at most 4 MiB",
            ));
        }
        let file = self.file.as_mut().ok_or_else(unknown_handle)?;
        file.write_all(bytes).map_err(|e| failure(&e, &self.name))?;
        self.written += bytes.len() as u64;
        Ok(())
    }

    fn commit(mut self) -> Result<(), ScriptFault> {
        let file = self.file.take().ok_or_else(unknown_handle)?;
        file.sync_all().map_err(|e| failure(&e, &self.name))?;
        drop(file);
        let result = if self.create_only {
            // Atomic and never over an existing file.
            match self.parent.hard_link(&self.temp, &self.parent, &self.name) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    Err(fault(codes::EXISTS, format!("“{}” exists", self.name)))
                }
                // A volume without hard links (FAT, some shares): checked,
                // then renamed (a file made in between is replaced).
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::Unsupported | std::io::ErrorKind::PermissionDenied
                    ) && self.parent.symlink_metadata(&self.name).is_err() =>
                {
                    self.parent
                        .rename(&self.temp, &self.parent, &self.name)
                        .map_err(|e| failure(&e, &self.name))
                }
                Err(e) => Err(failure(&e, &self.name)),
            }
        } else {
            self.parent
                .rename(&self.temp, &self.parent, &self.name)
                .map_err(|e| failure(&e, &self.name))
        };
        let _ = self.parent.remove_file(&self.temp);
        self.temp.clear();
        result
    }

    fn abort(mut self) {
        self.file = None;
        if !self.temp.is_empty() {
            let _ = self.parent.remove_file(&self.temp);
            self.temp.clear();
        }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if !self.temp.is_empty() {
            self.file = None;
            let _ = self.parent.remove_file(&self.temp);
        }
    }
}

/// A chunked write of a run.
pub(crate) struct OpenWrite {
    pending: Pending,
    path: PathArg,
}

/// A run's open chunked writes.
#[derive(Default)]
pub(crate) struct Writes {
    open: HashMap<String, OpenWrite>,
}

impl Writes {
    /// Abandons every open write (the run ended).
    pub fn abort_all(&mut self) {
        for (_, open) in self.open.drain() {
            open.pending.abort();
        }
    }
}
