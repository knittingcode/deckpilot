use std::path::PathBuf;

use super::*;
use crate::{MachineResourceBinding, Os, ProjectKey, ResourceKey};

/// A fresh folder of its own under the system's temporary folder.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "deckpilot-target-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn key(value: &str) -> ResourceKey {
    ResourceKey::new(value).unwrap()
}

fn fixed(value: &str) -> ResourceRef {
    ResourceRef::Fixed { key: key(value) }
}

fn machine(bindings: &[(&str, ResourceKind, &str)]) -> MachineConfig {
    let mut machine = MachineConfig::new(Os::current());
    for (k, kind, value) in bindings {
        machine.set_binding(MachineResourceBinding {
            key: key(k),
            kind: *kind,
            value: (*value).to_owned(),
        });
    }
    machine
}

#[test]
fn files_and_folders_are_told_apart_by_their_metadata() {
    let dir = scratch("kinds");
    let file = dir.join("report.pdf");
    std::fs::write(&file, b"%PDF").unwrap();
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(dir.join("docs/README.md"), b"# docs").unwrap();
    let folder = dir.to_string_lossy().into_owned();
    let file = file.to_string_lossy().into_owned();
    let machine = machine(&[
        ("file.report", ResourceKind::Other, &file),
        ("folder.work", ResourceKind::Directory, &folder),
    ]);
    let cache = TargetCache::new();
    let targets = cache.on(&machine, None);
    assert_eq!(
        targets.kind_of(&fixed("file.report"), None),
        TargetKind::File
    );
    assert_eq!(
        targets.kind_of(&fixed("folder.work"), None),
        TargetKind::Directory
    );
    // A path inside a folder resource, as Open resolves it.
    assert_eq!(
        targets.kind_of(&fixed("folder.work"), Some("docs")),
        TargetKind::Directory
    );
    assert_eq!(
        targets.kind_of(&fixed("folder.work"), Some("docs/README.md")),
        TargetKind::File
    );
    // Empty and blank relative paths are the resource itself.
    assert_eq!(
        targets.kind_of(&fixed("folder.work"), Some("  ")),
        TargetKind::Directory
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn what_cannot_be_told_safely_is_unknown() {
    let dir = scratch("unknown");
    let folder = dir.to_string_lossy().into_owned();
    let gone = dir.join("moved.pdf").to_string_lossy().into_owned();
    let machine = machine(&[
        ("file.gone", ResourceKind::Other, &gone),
        ("folder.work", ResourceKind::Directory, &folder),
        ("app.safari", ResourceKind::Application, "com.apple.Safari"),
        ("file.token", ResourceKind::Other, "token-42"),
        (
            "file.share",
            ResourceKind::Other,
            "\\\\server\\share\\a.txt",
        ),
        ("file.slashes", ResourceKind::Other, "//server/share/a.txt"),
        ("site.docs", ResourceKind::Url, "https://example.com"),
    ]);
    let cache = TargetCache::new();
    let targets = cache.on(&machine, None);
    for (reference, relative) in [
        // A missing file or folder.
        (fixed("file.gone"), None),
        (fixed("folder.work"), Some("missing/inside.txt")),
        // Not bound on this computer.
        (fixed("file.unbound"), None),
        // A project role without a project, or nothing chosen.
        (
            ResourceRef::ProjectRole {
                role: "repository".into(),
            },
            None,
        ),
        (
            ResourceRef::ProjectRole {
                role: String::new(),
            },
            None,
        ),
        // Not a file or folder binding, or not a local absolute path.
        (fixed("app.safari"), None),
        (fixed("file.token"), None),
        (fixed("file.share"), None),
        (fixed("file.slashes"), None),
        (fixed("site.docs"), None),
        // A path that leaves its resource, or is absolute.
        (fixed("folder.work"), Some("../outside")),
        (fixed("folder.work"), Some("/etc")),
        // An unresolved variable: known only when the action runs.
        (fixed("folder.work"), Some("{{ shared.paths.docs }}")),
    ] {
        assert_eq!(
            targets.kind_of(&reference, relative),
            TargetKind::Unknown,
            "{reference:?} {relative:?}"
        );
    }
    assert_eq!(
        NoTargets.kind_of(&fixed("folder.work"), None),
        TargetKind::Unknown
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn project_roles_resolve_for_the_project() {
    let dir = scratch("roles");
    let folder = dir.to_string_lossy().into_owned();
    let machine = machine(&[("alpha.repository", ResourceKind::Directory, &folder)]);
    let mut project = Project::new(ProjectKey::new("ALPHA").unwrap(), "Alpha");
    project
        .resources
        .insert("repository".into(), key("alpha.repository"));
    let cache = TargetCache::new();
    let role = ResourceRef::ProjectRole {
        role: "repository".into(),
    };
    assert_eq!(
        cache.on(&machine, Some(&project)).kind_of(&role, None),
        TargetKind::Directory
    );
    assert_eq!(
        cache.on(&machine, None).kind_of(&role, None),
        TargetKind::Unknown
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn answers_are_kept_until_cleared_never_polled() {
    let dir = scratch("cache");
    let target = dir.join("target");
    std::fs::write(&target, b"file first").unwrap();
    let value = target.to_string_lossy().into_owned();
    let machine = machine(&[("file.target", ResourceKind::Other, &value)]);
    let cache = TargetCache::new();
    let targets = cache.on(&machine, None);
    assert_eq!(
        targets.kind_of(&fixed("file.target"), None),
        TargetKind::File
    );
    // Replaced by a folder underneath: the kept answer until an event
    // clears the cache, then the new kind.
    std::fs::remove_file(&target).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    assert_eq!(
        targets.kind_of(&fixed("file.target"), None),
        TargetKind::File
    );
    cache.clear();
    assert_eq!(
        targets.kind_of(&fixed("file.target"), None),
        TargetKind::Directory
    );
    // And back to a file; then removed: unknown.
    std::fs::remove_dir_all(&target).unwrap();
    std::fs::write(&target, b"file again").unwrap();
    cache.clear();
    assert_eq!(
        targets.kind_of(&fixed("file.target"), None),
        TargetKind::File
    );
    std::fs::remove_file(&target).unwrap();
    cache.clear();
    assert_eq!(
        targets.kind_of(&fixed("file.target"), None),
        TargetKind::Unknown
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn links_are_followed_by_their_metadata_inside_the_resource_only() {
    let dir = scratch("links");
    let base = dir.join("repo");
    let outside = dir.join("outside");
    std::fs::create_dir_all(base.join("sub")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(base.join("sub/inside.txt"), b"in").unwrap();
    std::os::unix::fs::symlink(base.join("sub"), base.join("to-sub")).unwrap();
    std::os::unix::fs::symlink(base.join("sub/inside.txt"), base.join("to-file")).unwrap();
    std::os::unix::fs::symlink(&outside, base.join("out")).unwrap();
    std::os::unix::fs::symlink(base.join("sub/inside.txt"), dir.join("picked-link")).unwrap();
    let folder = base.to_string_lossy().into_owned();
    let picked = dir.join("picked-link").to_string_lossy().into_owned();
    let machine = machine(&[
        ("folder.repo", ResourceKind::Directory, &folder),
        ("file.link", ResourceKind::Other, &picked),
    ]);
    let cache = TargetCache::new();
    let targets = cache.on(&machine, None);
    assert_eq!(
        targets.kind_of(&fixed("folder.repo"), Some("to-sub")),
        TargetKind::Directory
    );
    assert_eq!(
        targets.kind_of(&fixed("folder.repo"), Some("to-file")),
        TargetKind::File
    );
    // Open refuses a link out of its resource: no icon is guessed for it.
    assert_eq!(
        targets.kind_of(&fixed("folder.repo"), Some("out")),
        TargetKind::Unknown
    );
    // A picked file that is a link: what it points to.
    assert_eq!(targets.kind_of(&fixed("file.link"), None), TargetKind::File);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(target_os = "macos")]
#[test]
fn application_bundles_are_neither_documents_nor_folders() {
    let dir = scratch("bundle");
    let bundle = dir.join("Editor.app");
    std::fs::create_dir_all(bundle.join("Contents")).unwrap();
    assert_eq!(lookup(&bundle.to_string_lossy(), None), TargetKind::Unknown);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_lookup_that_does_not_answer_in_time_is_unknown() {
    // The timeout itself, without a hanging mount: a zero wait cannot
    // see the answer of a thread that has to start first, or it does and
    // the answer is right; never a wrong kind.
    let dir = scratch("timeout");
    let key: Key = (dir.to_string_lossy().into_owned(), None);
    let pending = Arc::new(Mutex::new(HashSet::new()));
    let (kind, timed_out) = lookup_within(&key, &pending, Duration::ZERO);
    if timed_out {
        assert_eq!(kind, TargetKind::Unknown);
    } else {
        assert_eq!(kind, TargetKind::Directory);
    }
    // A path whose lookup has not returned yet is not looked up again.
    let busy: Key = ("/still/waiting".into(), None);
    pending.lock().unwrap().insert(busy.clone());
    assert_eq!(
        lookup_within(&busy, &pending, Duration::from_secs(5)),
        (TargetKind::Unknown, false)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
