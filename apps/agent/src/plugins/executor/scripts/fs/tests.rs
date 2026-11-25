//! `scripts_fs_*` (design §8): every test works in its own temporary
//! folder, never in a real user folder; the fixture checks the folder is
//! not inside the real home, and the protected "home" is a folder inside
//! the temporary one.

use deckpilot_core::script_grants::{Destructive, FsAccess, FsRoot, FsScope, ScriptGrants};
use serde_json::{Value, json};

use super::super::tests::support::*;

fn repo_grant(key: &str, access: FsAccess, root_destructive: bool) -> FsScope {
    FsScope {
        root: FsRoot::Repository(key.into()),
        access,
        root_destructive,
    }
}

fn grants(scopes: Vec<FsScope>) -> ScriptGrants {
    ScriptGrants {
        fs: scopes,
        destructive: Destructive::Allow,
        ..ScriptGrants::default()
    }
}

/// A project `cms` at `w/cms` and the given grant on it.
fn setup(access: FsAccess) -> Fixture {
    let f = Fixture::new();
    f.project(&[("cms", "w/cms")]);
    f.grant(grants(vec![repo_grant("cms", access, false)]));
    f
}

fn rp(rel: &str) -> Value {
    json!({"repository":"cms","relative":rel})
}

fn op(name: &str, args: Value) -> Value {
    let mut v = json!({"op": name});
    v["args"] = args;
    v
}

// --- mkdir --------------------------------------------------------------

#[test]
fn scripts_fs_mkdir_creates_recursively_and_reports_exists() {
    let f = setup(FsAccess::Write);
    let mut e = f.loaded();
    ok(&call(
        &mut e,
        &op("fs.mkdir", json!({"path": rp("a/b/c"), "recursive": true})),
    ));
    assert!(f.path("w/cms/a/b/c").is_dir());
    ok(&call(
        &mut e,
        &op("fs.mkdir", json!({"path": rp("a/b/c"), "recursive": true})),
    ));
    assert_eq!(
        code(&call(
            &mut e,
            &op("fs.mkdir", json!({"path": rp("a/b/c"), "recursive": false}))
        )),
        "exists"
    );
    assert_eq!(
        code(&call(
            &mut e,
            &op("fs.mkdir", json!({"path": rp("x/y"), "recursive": false}))
        )),
        "notFound"
    );
}

#[test]
fn scripts_fs_mkdir_needs_write_and_stays_in_scope() {
    let f = setup(FsAccess::Read);
    let mut e = f.loaded();
    assert_eq!(
        code(&call(
            &mut e,
            &op("fs.mkdir", json!({"path": rp("a"), "recursive": true}))
        )),
        "notGranted"
    );
    f.grant(grants(vec![repo_grant("cms", FsAccess::Write, false)]));
    let outside = f.path("w/outside");
    let r = call(
        &mut e,
        &op(
            "fs.mkdir",
            json!({"path": {"path": outside.to_string_lossy()}, "recursive": true}),
        ),
    );
    assert_eq!(code(&r), "outsideScope");
    assert!(!exists(&outside));
    assert_eq!(
        code(&call(
            &mut e,
            &op(
                "fs.mkdir",
                json!({"path": rp("../outside"), "recursive": true})
            )
        )),
        "call:BadRequest"
    );
}

// --- remove (files) -----------------------------------------------------

#[test]
fn scripts_fs_remove_file_needs_delete() {
    let f = setup(FsAccess::Write);
    std::fs::write(f.path("w/cms/a.txt"), "x").unwrap();
    let mut e = f.loaded();
    let remove = op(
        "fs.remove",
        json!({"path": rp("a.txt"), "recursive": false, "dryRun": false}),
    );
    assert_eq!(code(&call(&mut e, &remove)), "notGranted");
    assert!(exists(&f.path("w/cms/a.txt")));
    // A dry run needs only read, and changes nothing.
    let plan = ok(&call(
        &mut e,
        &op(
            "fs.remove",
            json!({"path": rp("a.txt"), "recursive": false, "dryRun": true}),
        ),
    ));
    assert_eq!(
        (
            plan["files"].as_u64(),
            plan["dryRun"].as_bool(),
            plan["removed"].as_bool()
        ),
        (Some(1), Some(true), Some(false))
    );
    assert!(exists(&f.path("w/cms/a.txt")));
    f.grant(grants(vec![repo_grant("cms", FsAccess::Delete, false)]));
    assert_eq!(ok(&call(&mut e, &remove))["removed"], true);
    assert!(!exists(&f.path("w/cms/a.txt")));
    assert_eq!(code(&call(&mut e, &remove)), "notFound");
}

#[test]
fn scripts_fs_remove_file_protected_paths() {
    let f = setup(FsAccess::Delete);
    std::fs::create_dir_all(f.path("w/cms/.git")).unwrap();
    std::fs::write(f.path("w/cms/.git/config"), "x").unwrap();
    let mut e = f.loaded();
    for rel in [".git/config", ".GIT/config", ".git"] {
        let r = call(
            &mut e,
            &op(
                "fs.remove",
                json!({"path": rp(rel), "recursive": true, "dryRun": false}),
            ),
        );
        assert_eq!(code(&r), "protectedPath", "{rel}");
    }
    assert!(exists(&f.path("w/cms/.git/config")));
    // The repository's root itself.
    let r = call(
        &mut e,
        &op(
            "fs.remove",
            json!({"path": rp(""), "recursive": true, "dryRun": false}),
        ),
    );
    assert_eq!(code(&r), "protectedPath");
    // Writes into .git are refused as well (hooks).
    let r = call(
        &mut e,
        &op(
            "fs.writeFile",
            json!({"path": rp(".git/hooks/pre-commit"), "content": "x", "encoding": "utf8", "createOnly": false}),
        ),
    );
    assert_eq!(code(&r), "protectedPath");
}

// --- recursive removal --------------------------------------------------

fn tree(f: &Fixture, rel: &str) {
    let d = f.path(rel);
    std::fs::create_dir_all(d.join("a/b")).unwrap();
    std::fs::write(d.join("x.txt"), "123").unwrap();
    std::fs::write(d.join("a/y.txt"), "45").unwrap();
    std::fs::write(d.join("a/b/z.txt"), "6").unwrap();
}

#[test]
fn scripts_fs_remove_recursive_plans_and_removes() {
    let f = setup(FsAccess::Delete);
    tree(&f, "w/cms/build");
    let mut e = f.loaded();
    let dry = ok(&call(
        &mut e,
        &op(
            "fs.remove",
            json!({"path": rp("build"), "recursive": true, "dryRun": true}),
        ),
    ));
    assert_eq!(
        (
            dry["files"].as_u64(),
            dry["directories"].as_u64(),
            dry["bytes"].as_u64()
        ),
        (Some(3), Some(3), Some(6))
    );
    assert_eq!(dry["root"], "cms/build");
    assert!(exists(&f.path("w/cms/build/a/b/z.txt")));
    let r = call(
        &mut e,
        &op(
            "fs.remove",
            json!({"path": rp("build"), "recursive": false, "dryRun": false}),
        ),
    );
    assert_eq!(
        code(&r),
        "invalidArgument",
        "a folder that is not empty needs recursive"
    );
    ok(&call(
        &mut e,
        &op(
            "fs.remove",
            json!({"path": rp("build"), "recursive": true, "dryRun": false}),
        ),
    ));
    assert!(!exists(&f.path("w/cms/build")));
}

#[test]
fn scripts_fs_remove_recursive_never_holds_git_or_a_repository() {
    let f = Fixture::new();
    f.project(&[("cms", "w/cms"), ("web", "w/web")]);
    std::fs::create_dir_all(f.path("w/web/.git")).unwrap();
    std::fs::create_dir_all(f.path("w/cms/vendor/lib/.git")).unwrap();
    f.grant(grants(vec![
        FsScope {
            root: FsRoot::Directory {
                path: f.path("w"),
                label: "work".into(),
            },
            access: FsAccess::Delete,
            root_destructive: true,
        },
        repo_grant("cms", FsAccess::Delete, true),
    ]));
    let mut e = f.loaded();
    // A tree with a nested .git.
    let r = call(
        &mut e,
        &op(
            "fs.remove",
            json!({"path": rp("vendor"), "recursive": true, "dryRun": true}),
        ),
    );
    assert_eq!(code(&r), "protectedPath");
    // A repository's folder, through a directory scope.
    let web = f.path("w/web");
    let r = call(
        &mut e,
        &op(
            "fs.remove",
            json!({"path": {"path": web.to_string_lossy()}, "recursive": true, "dryRun": false}),
        ),
    );
    assert_eq!(code(&r), "protectedPath");
    // The scope's root (even with rootDestructive it is never removed).
    let r = call(
        &mut e,
        &op(
            "fs.remove",
            json!({"path": {"path": f.path("w").to_string_lossy()}, "recursive": true, "dryRun": false}),
        ),
    );
    assert_eq!(code(&r), "protectedPath");
    assert!(exists(&f.path("w/web/.git")) && exists(&f.path("w/cms/vendor/lib/.git")));
}

#[test]
fn scripts_fs_remove_contents_keeps_and_needs_root_destructive() {
    let f = setup(FsAccess::Delete);
    tree(&f, "w/cms");
    std::fs::create_dir_all(f.path("w/cms/.git")).unwrap();
    std::fs::write(f.path("w/cms/.gitkeep"), "").unwrap();
    let mut e = f.loaded();
    let contents = |keep: Value| {
        op(
            "fs.removeContents",
            json!({"path": rp(""), "dryRun": false, "keep": keep}),
        )
    };
    assert_eq!(
        code(&call(&mut e, &contents(json!([".git"])))),
        "protectedPath",
        "the root needs rootDestructive"
    );
    f.grant(grants(vec![repo_grant("cms", FsAccess::Delete, true)]));
    assert_eq!(
        code(&call(&mut e, &contents(json!([])))),
        "protectedPath",
        ".git is never removed"
    );
    let done = ok(&call(&mut e, &contents(json!([".git", ".gitkeep"]))));
    assert_eq!(done["removed"], true);
    let left: Vec<_> = std::fs::read_dir(f.path("w/cms"))
        .unwrap()
        .map(|d| d.unwrap().file_name())
        .collect();
    let mut left: Vec<_> = left
        .into_iter()
        .map(|n| n.to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, [".git", ".gitkeep"]);
}

#[test]
fn scripts_fs_remove_contents_of_a_sub_folder() {
    let f = setup(FsAccess::Delete);
    tree(&f, "w/cms/cache");
    let mut e = f.loaded();
    let plan = ok(&call(
        &mut e,
        &op(
            "fs.removeContents",
            json!({"path": rp("cache"), "dryRun": true, "keep": ["x.txt"]}),
        ),
    ));
    assert_eq!(
        (plan["files"].as_u64(), plan["directories"].as_u64()),
        (Some(2), Some(2))
    );
    ok(&call(
        &mut e,
        &op(
            "fs.removeContents",
            json!({"path": rp("cache"), "dryRun": false, "keep": ["x.txt"]}),
        ),
    ));
    assert!(exists(&f.path("w/cms/cache/x.txt")) && !exists(&f.path("w/cms/cache/a")));
}

// --- protected roots ----------------------------------------------------

#[test]
fn scripts_fs_protected_home_is_never_removed_or_emptied() {
    let f = Fixture::new();
    std::fs::write(f.home.join("notes.txt"), "keep").unwrap();
    f.grant(grants(vec![FsScope {
        root: FsRoot::Directory {
            path: f.root.clone(),
            label: "tmp".into(),
        },
        access: FsAccess::Delete,
        root_destructive: true,
    }]));
    let mut e = f.loaded();
    let home = f.home.to_string_lossy().into_owned();
    for request in [
        op(
            "fs.remove",
            json!({"path": {"path": home}, "recursive": true, "dryRun": false}),
        ),
        op(
            "fs.removeContents",
            json!({"path": {"path": home}, "dryRun": false, "keep": []}),
        ),
        op(
            "fs.move",
            json!({"src": {"path": home}, "dst": {"path": f.path("moved").to_string_lossy()}, "overwrite": false}),
        ),
    ] {
        assert_eq!(code(&call(&mut e, &request)), "protectedPath", "{request}");
    }
    // The folder that holds it is protected too ("contains").
    let r = call(
        &mut e,
        &op(
            "fs.removeContents",
            json!({"path": {"path": f.root.to_string_lossy()}, "dryRun": false, "keep": []}),
        ),
    );
    assert_eq!(code(&r), "protectedPath");
    assert!(exists(&f.home.join("notes.txt")));
    // Inside it, a file may go.
    ok(&call(
        &mut e,
        &op(
            "fs.remove",
            json!({"path": {"path": f.home.join("notes.txt").to_string_lossy()}, "recursive": false, "dryRun": false}),
        ),
    ));
}

// --- traversal ----------------------------------------------------------

#[test]
fn scripts_fs_traversal_never_leaves_a_scope() {
    let f = setup(FsAccess::Read);
    std::fs::write(f.path("w/secret.txt"), "s").unwrap();
    let mut e = f.loaded();
    let read = |path: Value| op("fs.readFile", json!({"path": path, "encoding": "utf8"}));
    // The ABI refuses non-canonical forms, the agent what normalizes out.
    for path in [
        rp("../secret.txt"),
        rp("a/../../secret.txt"),
        json!({"path": format!("{}/../secret.txt", f.path("w/cms").display())}),
        json!({"path": "relative/secret.txt"}),
    ] {
        let r = call(&mut e, &read(path.clone()));
        assert!(
            matches!(
                code(&r).as_str(),
                "call:BadRequest" | "outsideScope" | "invalidArgument"
            ),
            "{path}: {}",
            code(&r)
        );
    }
    // An absolute path outside every scope.
    let r = call(
        &mut e,
        &read(json!({"path": f.path("w/secret.txt").to_string_lossy()})),
    );
    assert_eq!(code(&r), "outsideScope");
    // An absolute path inside a repository needs repositories.paths.
    std::fs::write(f.path("w/cms/in.txt"), "in").unwrap();
    let inside = json!({"path": f.path("w/cms/in.txt").to_string_lossy()});
    assert_eq!(code(&call(&mut e, &read(inside.clone()))), "outsideScope");
    f.grant(ScriptGrants {
        repositories_paths: true,
        ..grants(vec![repo_grant("cms", FsAccess::Read, false)])
    });
    assert_eq!(ok(&call(&mut e, &read(inside)))["content"], "in");
    // A repository the project does not have.
    assert_eq!(
        code(&call(
            &mut e,
            &read(json!({"repository":"web","relative":"a"}))
        )),
        "notFound"
    );
}

// --- symlinks -----------------------------------------------------------

#[cfg(unix)]
mod symlinks {
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn scripts_fs_symlink_to_outside_is_an_escape() {
        let f = setup(FsAccess::Delete);
        std::fs::write(f.path("w/secret.txt"), "s").unwrap();
        symlink(f.path("w/secret.txt"), f.path("w/cms/link.txt")).unwrap();
        symlink(f.path("w"), f.path("w/cms/up")).unwrap();
        let mut e = f.loaded();
        let r = call(
            &mut e,
            &op(
                "fs.readFile",
                json!({"path": rp("link.txt"), "encoding": "utf8"}),
            ),
        );
        assert_eq!(code(&r), "symlinkEscape");
        let r = call(
            &mut e,
            &op(
                "fs.readFile",
                json!({"path": rp("up/secret.txt"), "encoding": "utf8"}),
            ),
        );
        assert_eq!(code(&r), "symlinkEscape");
        let r = call(&mut e, &op("fs.list", json!({"path": rp("up")})));
        assert_eq!(code(&r), "symlinkEscape");
        // stat and exists see the link itself.
        assert_eq!(
            ok(&call(
                &mut e,
                &op("fs.stat", json!({"path": rp("link.txt")}))
            ))["stat"]["kind"],
            "symlink"
        );
        // Writing through a link in the path is refused; nothing outside changes.
        let r = call(
            &mut e,
            &op(
                "fs.writeFile",
                json!({"path": rp("up/secret.txt"), "content": "pwned", "encoding": "utf8", "createOnly": false}),
            ),
        );
        assert_eq!(code(&r), "symlinkEscape");
        assert_eq!(
            std::fs::read_to_string(f.path("w/secret.txt")).unwrap(),
            "s"
        );
        // Removing the link removes the link, never its target.
        ok(&call(
            &mut e,
            &op(
                "fs.remove",
                json!({"path": rp("up"), "recursive": true, "dryRun": false}),
            ),
        ));
        assert!(!exists(&f.path("w/cms/up")) && exists(&f.path("w/secret.txt")));
        // Writing over a link replaces the link, never its target.
        ok(&call(
            &mut e,
            &op(
                "fs.writeFile",
                json!({"path": rp("link.txt"), "content": "new", "encoding": "utf8", "createOnly": false}),
            ),
        ));
        assert_eq!(
            std::fs::read_to_string(f.path("w/secret.txt")).unwrap(),
            "s"
        );
        assert_eq!(
            std::fs::read_to_string(f.path("w/cms/link.txt")).unwrap(),
            "new"
        );
    }

    #[test]
    fn scripts_fs_symlink_to_home_inside_a_tree_is_not_followed() {
        let f = setup(FsAccess::Delete);
        std::fs::write(f.home.join("precious.txt"), "p").unwrap();
        std::fs::create_dir_all(f.path("w/cms/build")).unwrap();
        symlink(&f.home, f.path("w/cms/build/home")).unwrap();
        let mut e = f.loaded();
        let plan = ok(&call(
            &mut e,
            &op(
                "fs.remove",
                json!({"path": rp("build"), "recursive": true, "dryRun": true}),
            ),
        ));
        assert_eq!(
            (plan["files"].as_u64(), plan["directories"].as_u64()),
            (Some(1), Some(1)),
            "the link counts as one entry"
        );
        ok(&call(
            &mut e,
            &op(
                "fs.remove",
                json!({"path": rp("build"), "recursive": true, "dryRun": false}),
            ),
        ));
        assert!(!exists(&f.path("w/cms/build")));
        assert_eq!(
            std::fs::read_to_string(f.home.join("precious.txt")).unwrap(),
            "p"
        );
    }

    #[test]
    fn scripts_fs_symlink_root_replaced_after_grant() {
        let f = Fixture::new();
        std::fs::create_dir_all(f.path("out")).unwrap();
        std::fs::create_dir_all(f.path("elsewhere")).unwrap();
        std::fs::write(f.path("elsewhere/x.txt"), "x").unwrap();
        f.grant(grants(vec![FsScope {
            root: FsRoot::Directory {
                path: f.path("out"),
                label: "Out".into(),
            },
            access: FsAccess::Delete,
            root_destructive: true,
        }]));
        let mut e = f.loaded();
        ok(&call(
            &mut e,
            &op(
                "fs.list",
                json!({"path": {"path": f.path("out").to_string_lossy()}}),
            ),
        ));
        // The granted folder is swapped for a link to another one.
        std::fs::remove_dir(f.path("out")).unwrap();
        symlink(f.path("elsewhere"), f.path("out")).unwrap();
        let r = call(
            &mut e,
            &op(
                "fs.removeContents",
                json!({"path": {"path": f.path("out").to_string_lossy()}, "dryRun": false, "keep": []}),
            ),
        );
        assert_eq!(code(&r), "outsideScope", "the scope moved");
        assert!(exists(&f.path("elsewhere/x.txt")));
    }

    #[test]
    fn scripts_fs_symlink_in_the_middle_of_a_path() {
        let f = setup(FsAccess::Delete);
        std::fs::create_dir_all(f.path("w/cms/real/deep")).unwrap();
        std::fs::write(f.path("w/cms/real/deep/f.txt"), "f").unwrap();
        symlink("real", f.path("w/cms/alias")).unwrap();
        symlink(f.path("w/cms/real"), f.path("w/cms/absolute")).unwrap();
        let mut e = f.loaded();
        // A read may pass a relative link that stays inside the scope.
        assert_eq!(
            ok(&call(
                &mut e,
                &op(
                    "fs.readFile",
                    json!({"path": rp("alias/deep/f.txt"), "encoding": "utf8"})
                )
            ))["content"],
            "f"
        );
        // An absolute link is never followed, even to inside the scope.
        let r = call(
            &mut e,
            &op(
                "fs.readFile",
                json!({"path": rp("absolute/deep/f.txt"), "encoding": "utf8"}),
            ),
        );
        assert_eq!(code(&r), "symlinkEscape");
        // A change never does.
        let r = call(
            &mut e,
            &op(
                "fs.remove",
                json!({"path": rp("alias/deep"), "recursive": true, "dryRun": false}),
            ),
        );
        assert_eq!(code(&r), "symlinkEscape");
        assert!(exists(&f.path("w/cms/real/deep/f.txt")));
    }

    /// A thread swaps a folder for a link to the protected home while
    /// removals run: nothing outside the scope is ever removed.
    #[test]
    fn scripts_fs_symlink_race_never_escapes() {
        let f = setup(FsAccess::Delete);
        for i in 0..20 {
            std::fs::write(f.home.join(format!("h{i}.txt")), "h").unwrap();
        }
        let target = f.path("w/cms/victim");
        let home = f.home.clone();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        let swapper = std::thread::spawn(move || {
            while !flag.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = std::fs::remove_file(&target);
                let _ = std::fs::create_dir_all(target.join("sub"));
                let _ = std::fs::write(target.join("sub/f.txt"), "f");
                let _ = std::fs::remove_dir_all(&target);
                let _ = symlink(&home, &target);
            }
        });
        let mut e = f.loaded();
        for _ in 0..200 {
            let _ = call(
                &mut e,
                &op(
                    "fs.remove",
                    json!({"path": rp("victim"), "recursive": true, "dryRun": false}),
                ),
            );
            let _ = call(
                &mut e,
                &op(
                    "fs.removeContents",
                    json!({"path": rp("victim"), "dryRun": false, "keep": []}),
                ),
            );
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        swapper.join().unwrap();
        let left = std::fs::read_dir(&f.home).unwrap().count();
        assert_eq!(left, 20, "nothing in the protected home was removed");
    }
}

// --- reads, writes, sizes -----------------------------------------------

#[test]
fn scripts_fs_read_file_in_chunks() {
    let f = setup(FsAccess::Read);
    // 600 KiB with a two-byte character across the 512 KiB boundary.
    let mut text = "a".repeat((512 << 10) - 1);
    text.push('é');
    text.push_str(&"b".repeat(88 << 10));
    std::fs::write(f.path("w/cms/big.txt"), &text).unwrap();
    let mut e = f.loaded();
    let whole = call(
        &mut e,
        &op(
            "fs.readFile",
            json!({"path": rp("big.txt"), "encoding": "utf8"}),
        ),
    );
    assert_eq!(
        code(&whole),
        "tooLarge",
        "a whole read is one chunk at most"
    );
    let first = ok(&call(
        &mut e,
        &op(
            "fs.readFile",
            json!({"path": rp("big.txt"), "encoding": "utf8", "offset": 0, "length": 524_288}),
        ),
    ));
    assert_eq!(first["eof"], false);
    let n = first["bytes"].as_u64().unwrap();
    assert_eq!(n, 524_287, "the cut character waits for the next read");
    let second = ok(&call(
        &mut e,
        &op(
            "fs.readFile",
            json!({"path": rp("big.txt"), "encoding": "utf8", "offset": n, "length": 524_288,
                   "ifVersion": first["version"]}),
        ),
    ));
    assert_eq!(second["eof"], true);
    let joined = format!(
        "{}{}",
        first["content"].as_str().unwrap(),
        second["content"].as_str().unwrap()
    );
    assert_eq!(joined, text);
    let r = call(
        &mut e,
        &op(
            "fs.readFile",
            json!({"path": rp("big.txt"), "encoding": "utf8", "maxBytes": 1024, "offset": 0, "length": 1024}),
        ),
    );
    assert_eq!(code(&r), "tooLarge");
}

#[test]
fn scripts_fs_read_reply_fits_even_when_json_escapes() {
    let f = setup(FsAccess::Read);
    std::fs::write(f.path("w/cms/ctl.txt"), vec![1u8; 512 << 10]).unwrap();
    let mut e = f.loaded();
    let r = ok(&call(
        &mut e,
        &op(
            "fs.readFile",
            json!({"path": rp("ctl.txt"), "encoding": "utf8", "offset": 0, "length": 524_288}),
        ),
    ));
    let json = serde_json::to_vec(&r).unwrap();
    assert!(json.len() < 1 << 20, "{}", json.len());
    assert!(r["bytes"].as_u64().unwrap() < 512 << 10);
    assert_eq!(r["eof"], false);
}

#[test]
fn scripts_fs_write_file_and_chunks_replace_atomically() {
    let f = setup(FsAccess::Write);
    let mut e = f.loaded();
    ok(&call(
        &mut e,
        &op(
            "fs.writeFile",
            json!({"path": rp("a.txt"), "content": "one", "encoding": "utf8", "createOnly": false}),
        ),
    ));
    assert_eq!(
        std::fs::read_to_string(f.path("w/cms/a.txt")).unwrap(),
        "one"
    );
    let r = call(
        &mut e,
        &op(
            "fs.writeFile",
            json!({"path": rp("a.txt"), "content": "two", "encoding": "utf8", "createOnly": true}),
        ),
    );
    assert_eq!(code(&r), "exists");
    let opened = ok(&call(
        &mut e,
        &op(
            "fs.write.open",
            json!({"path": rp("a.txt"), "createOnly": false}),
        ),
    ));
    let h = opened["handle"].as_str().unwrap().to_owned();
    for _ in 0..3 {
        ok(&call(
            &mut e,
            &op(
                "fs.write.chunk",
                json!({"handle": h, "content": "x".repeat(32 << 10), "encoding": "utf8"}),
            ),
        ));
    }
    assert_eq!(
        std::fs::read_to_string(f.path("w/cms/a.txt")).unwrap(),
        "one",
        "unchanged until the commit"
    );
    ok(&call(&mut e, &op("fs.write.commit", json!({"handle": h}))));
    assert_eq!(
        std::fs::read(f.path("w/cms/a.txt")).unwrap().len(),
        3 * (32 << 10)
    );
    assert_eq!(
        code(&call(&mut e, &op("fs.write.commit", json!({"handle": h})))),
        "invalidArgument"
    );
    // No temporary file is left behind.
    let names: Vec<_> = std::fs::read_dir(f.path("w/cms"))
        .unwrap()
        .map(|d| d.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    // A folder is never written over.
    std::fs::create_dir(f.path("w/cms/dir")).unwrap();
    let r = call(
        &mut e,
        &op(
            "fs.writeFile",
            json!({"path": rp("dir"), "content": "x", "encoding": "utf8", "createOnly": false}),
        ),
    );
    assert_eq!(code(&r), "exists");
}

#[test]
fn scripts_fs_chunked_writes_are_bounded_and_end_with_the_run() {
    let f = setup(FsAccess::Write);
    let mut e = f.loaded();
    let open = |e: &mut _, rel: &str| {
        call(
            e,
            &op(
                "fs.write.open",
                json!({"path": rp(rel), "createOnly": false}),
            ),
        )
    };
    let mut handles = Vec::new();
    for i in 0..4 {
        handles.push(
            ok(&open(&mut e, &format!("f{i}")))["handle"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    assert_eq!(code(&open(&mut e, "f5")), "limit");
    // 4 MiB at most.
    let chunk = "y".repeat(32 << 10);
    let mut refused = false;
    for _ in 0..130 {
        let r = call(
            &mut e,
            &op(
                "fs.write.chunk",
                json!({"handle": handles[0], "content": chunk, "encoding": "utf8"}),
            ),
        );
        if matches!(r, deckpilot_plugin_abi::CallResult::Error(_)) {
            assert_eq!(code(&r), "tooLarge");
            refused = true;
            break;
        }
    }
    assert!(refused);
    // Revoked between chunks: the write is abandoned.
    f.grant(grants(vec![repo_grant("cms", FsAccess::Read, false)]));
    let r = call(
        &mut e,
        &op(
            "fs.write.chunk",
            json!({"handle": handles[1], "content": "z", "encoding": "utf8"}),
        ),
    );
    assert_eq!(code(&r), "notGranted");
    assert_eq!(
        code(&call(
            &mut e,
            &op("fs.write.commit", json!({"handle": handles[1]}))
        )),
        "invalidArgument"
    );
    // The run ends: every temporary file goes.
    drop(e);
    let names: Vec<_> = std::fs::read_dir(f.path("w/cms")).unwrap().collect();
    assert!(names.is_empty(), "{names:?}");
}

// --- copy and move --------------------------------------------------------

#[test]
fn scripts_fs_copy_and_move_with_overwrite_rules() {
    // Replacing a folder needs delete access.
    let f = setup(FsAccess::Delete);
    tree(&f, "w/cms/src");
    let mut e = f.loaded();
    let copy = |overwrite: bool| {
        op(
            "fs.copy",
            json!({"src": rp("src"), "dst": rp("dst"), "recursive": true, "overwrite": overwrite}),
        )
    };
    ok(&call(&mut e, &copy(false)));
    assert_eq!(
        std::fs::read_to_string(f.path("w/cms/dst/a/b/z.txt")).unwrap(),
        "6"
    );
    assert_eq!(code(&call(&mut e, &copy(false))), "exists");
    // Overwriting is destructive: allowed here (destructive: allow), with write access.
    ok(&call(&mut e, &copy(true)));
    assert_eq!(
        code(&call(
            &mut e,
            &op(
                "fs.copy",
                json!({"src": rp("src"), "dst": rp("src/a/inner"), "recursive": true, "overwrite": false})
            )
        )),
        "invalidArgument"
    );
    ok(&call(
        &mut e,
        &op(
            "fs.move",
            json!({"src": rp("dst"), "dst": rp("moved"), "overwrite": false}),
        ),
    ));
    assert!(!exists(&f.path("w/cms/dst")) && exists(&f.path("w/cms/moved/x.txt")));
    // The scope's root never moves.
    assert_eq!(
        code(&call(
            &mut e,
            &op(
                "fs.move",
                json!({"src": rp(""), "dst": rp("x"), "overwrite": false})
            )
        )),
        "protectedPath"
    );
}

#[test]
fn scripts_fs_overwrite_asks_in_ask_mode() {
    let f = setup(FsAccess::Write);
    f.grant(ScriptGrants {
        destructive: Destructive::Ask,
        ..grants(vec![repo_grant("cms", FsAccess::Write, false)])
    });
    std::fs::write(f.path("w/cms/a.txt"), "a").unwrap();
    std::fs::write(f.path("w/cms/b.txt"), "b").unwrap();
    let mut e = f.loaded();
    let r = call(
        &mut e,
        &op(
            "fs.copy",
            json!({"src": rp("a.txt"), "dst": rp("b.txt"), "recursive": false, "overwrite": true}),
        ),
    );
    assert_eq!(code(&r), "needsConfirmation");
    assert_eq!(std::fs::read_to_string(f.path("w/cms/b.txt")).unwrap(), "b");
}

// --- revocation -----------------------------------------------------------

#[test]
fn scripts_fs_revocation_applies_to_the_next_call() {
    let f = setup(FsAccess::Read);
    std::fs::write(f.path("w/cms/a.txt"), "a").unwrap();
    let mut e = f.loaded();
    let read = op(
        "fs.readFile",
        json!({"path": rp("a.txt"), "encoding": "utf8"}),
    );
    ok(&call(&mut e, &read));
    f.revoke();
    assert_eq!(code(&call(&mut e, &read)), "outsideScope", "no scope left");
    f.grant(grants(vec![repo_grant("cms", FsAccess::Read, false)]));
    ok(&call(&mut e, &read));
    // The plugin's ceiling revoked: the broker refuses before the executor.
    f.broker
        .set_grants(
            &deckpilot_plugin_broker::PluginId::new(PLUGIN).unwrap(),
            &deckpilot_plugin_broker::ApprovedPermissions::from_json(
                r#"[{"kind":"scripts","process":true}]"#,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(code(&call(&mut e, &read)), "call:NotPermitted");
}

#[test]
fn scripts_fs_list_and_stat() {
    let f = setup(FsAccess::Read);
    tree(&f, "w/cms");
    let mut e = f.loaded();
    let list = ok(&call(&mut e, &op("fs.list", json!({"path": rp("")}))));
    let names: Vec<_> = list["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["name"].as_str().unwrap().to_owned(),
                e["kind"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        names,
        [
            ("a".to_owned(), "directory".to_owned()),
            ("x.txt".to_owned(), "file".to_owned())
        ]
    );
    let stat = ok(&call(&mut e, &op("fs.stat", json!({"path": rp("x.txt")}))));
    assert_eq!(stat["stat"]["size"], 3);
    assert_eq!(
        ok(&call(&mut e, &op("fs.stat", json!({"path": rp("nope")}))))["stat"],
        json!(null)
    );
    assert_eq!(
        ok(&call(&mut e, &op("fs.exists", json!({"path": rp("nope")}))))["exists"],
        false
    );
}

// --- the security review of #469 ------------------------------------------

#[test]
fn scripts_fs_private_folders_are_never_read_or_written() {
    let f = Fixture::new();
    let data = f.path("appdata");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("agent.token"), "secret").unwrap();
    f.grant(grants(vec![FsScope {
        root: FsRoot::Directory {
            path: f.root.clone(),
            label: "tmp".into(),
        },
        access: FsAccess::Delete,
        root_destructive: false,
    }]));
    let mut e = f.loaded();
    let token = json!({"path": data.join("agent.token").to_string_lossy()});
    let r = call(
        &mut e,
        &op("fs.readFile", json!({"path": token, "encoding": "utf8"})),
    );
    assert_eq!(code(&r), "protectedPath");
    let r = call(
        &mut e,
        &op(
            "fs.writeFile",
            json!({"path": {"path": data.join("x").to_string_lossy()}, "content": "x", "encoding": "utf8", "createOnly": false}),
        ),
    );
    assert_eq!(code(&r), "protectedPath");
    let r = call(
        &mut e,
        &op("fs.list", json!({"path": {"path": data.to_string_lossy()}})),
    );
    assert_eq!(code(&r), "protectedPath");
    #[cfg(unix)]
    {
        // Nor through a link inside the scope.
        std::os::unix::fs::symlink("appdata", f.path("alias")).unwrap();
        let r = call(
            &mut e,
            &op(
                "fs.readFile",
                json!({"path": {"path": f.path("alias/agent.token").to_string_lossy()}, "encoding": "utf8"}),
            ),
        );
        assert_eq!(code(&r), "protectedPath");
    }
}

#[test]
fn scripts_fs_ask_refuses_every_removal_in_a_run_nobody_started() {
    let f = setup(FsAccess::Delete);
    f.grant(ScriptGrants {
        destructive: Destructive::Ask,
        ..grants(vec![repo_grant("cms", FsAccess::Delete, false)])
    });
    std::fs::write(f.path("w/cms/one.txt"), "1").unwrap();
    let schedule = deckpilot_plugin_abi::capability::RunContext::workflow(
        deckpilot_plugin_abi::capability::RunCause::Schedule,
        None,
    );
    let mut e = f.loaded_in(schedule);
    let r = call(
        &mut e,
        &op(
            "fs.remove",
            json!({"path": rp("one.txt"), "recursive": false, "dryRun": false}),
        ),
    );
    assert_eq!(code(&r), "notUserInitiated");
    assert!(exists(&f.path("w/cms/one.txt")));
    // A dry run still works.
    ok(&call(
        &mut e,
        &op(
            "fs.remove",
            json!({"path": rp("one.txt"), "recursive": false, "dryRun": true}),
        ),
    ));
}

#[test]
fn scripts_fs_a_token_is_bound_to_the_entries_not_their_count() {
    let f = setup(FsAccess::Delete);
    f.grant(ScriptGrants {
        destructive: Destructive::Ask,
        ..grants(vec![repo_grant("cms", FsAccess::Delete, false)])
    });
    std::fs::create_dir_all(f.path("w/cms/build")).unwrap();
    std::fs::write(f.path("w/cms/build/a.txt"), "aaaa").unwrap();
    let mut e = f.loaded();
    let remove = |t: Option<&str>| {
        let mut args = json!({"path": rp("build"), "recursive": true, "dryRun": false});
        if let Some(t) = t {
            args["confirmation"] = json!(t);
        }
        op("fs.remove", args)
    };
    let t = token(&call(&mut e, &remove(None)));
    Fixture::answer(&e, true);
    ok(&call(&mut e, &op("script.confirm", json!({"token": t}))));
    // Another file of the same size in its place.
    std::fs::remove_file(f.path("w/cms/build/a.txt")).unwrap();
    std::fs::write(f.path("w/cms/build/b.txt"), "bbbb").unwrap();
    let r = call(&mut e, &remove(Some(&t)));
    assert_eq!(code(&r), "needsConfirmation");
    assert!(exists(&f.path("w/cms/build/b.txt")));
}

#[test]
fn scripts_fs_keep_never_spares_a_nested_git() {
    let f = setup(FsAccess::Delete);
    std::fs::create_dir_all(f.path("w/cms/vendor/lib/.git")).unwrap();
    std::fs::write(f.path("w/cms/vendor/lib/work.txt"), "uncommitted").unwrap();
    let mut e = f.loaded();
    let r = call(
        &mut e,
        &op(
            "fs.removeContents",
            json!({"path": rp("vendor/lib"), "dryRun": false, "keep": [".git"]}),
        ),
    );
    assert_eq!(code(&r), "protectedPath");
    assert!(exists(&f.path("w/cms/vendor/lib/work.txt")));
}

#[test]
fn scripts_fs_replacing_a_folder_needs_delete() {
    let f = setup(FsAccess::Write);
    tree(&f, "w/cms/src");
    tree(&f, "w/cms/dst");
    let mut e = f.loaded();
    let r = call(
        &mut e,
        &op(
            "fs.copy",
            json!({"src": rp("src"), "dst": rp("dst"), "recursive": true, "overwrite": true}),
        ),
    );
    assert_eq!(code(&r), "notGranted");
    assert!(exists(&f.path("w/cms/dst/a/b/z.txt")));
}

#[cfg(unix)]
#[test]
fn scripts_fs_copy_never_plants_a_link_that_leads_out() {
    let f = setup(FsAccess::Write);
    std::fs::create_dir_all(f.path("w/cms/src/bin")).unwrap();
    std::os::unix::fs::symlink("../pkg/tool", f.path("w/cms/src/bin/tool")).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", f.path("w/cms/src/abs")).unwrap();
    std::fs::create_dir_all(f.path("w/cms/deep")).unwrap();
    let mut e = f.loaded();
    // A relative link that stays inside, from its new place too.
    ok(&call(
        &mut e,
        &op(
            "fs.copy",
            json!({"src": rp("src/bin"), "dst": rp("deep/bin"), "recursive": true, "overwrite": false}),
        ),
    ));
    assert!(
        f.path("w/cms/deep/bin/tool")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
    // The same link copied to the root would lead above it.
    let r = call(
        &mut e,
        &op(
            "fs.copy",
            json!({"src": rp("src/bin/tool"), "dst": rp("tool"), "recursive": false, "overwrite": false}),
        ),
    );
    assert_eq!(code(&r), "symlinkEscape");
    let r = call(
        &mut e,
        &op(
            "fs.copy",
            json!({"src": rp("src/abs"), "dst": rp("abs"), "recursive": false, "overwrite": false}),
        ),
    );
    assert_eq!(code(&r), "symlinkEscape");
    assert!(!exists(&f.path("w/cms/tool")) && !exists(&f.path("w/cms/abs")));
}

#[test]
fn scripts_fs_the_run_keeps_the_project_it_started_with() {
    let f = setup(FsAccess::Read);
    std::fs::write(f.path("w/cms/a.txt"), "a").unwrap();
    let mut e = f.loaded();
    // The active project changes to one with another `cms`.
    f.project(&[("cms", "w/other")]);
    f.host.project.lock().unwrap().as_mut().unwrap().id = "p2".into();
    let r = call(
        &mut e,
        &op(
            "fs.readFile",
            json!({"path": rp("a.txt"), "encoding": "utf8"}),
        ),
    );
    // The pinned project is gone from the store: its repository with it,
    // and never the new project's `cms`.
    assert_eq!(code(&r), "notFound");
}

// --- the coordinator's re-check of #469 -------------------------------------

#[cfg(unix)]
#[test]
fn scripts_fs_move_never_carries_a_link_out_of_its_scope() {
    let f = setup(FsAccess::Write);
    std::fs::create_dir_all(f.path("w/cms/a/b/c")).unwrap();
    // Inside from its place, three levels above the root once moved up.
    std::os::unix::fs::symlink("../../y", f.path("w/cms/a/b/c/x")).unwrap();
    std::fs::create_dir_all(f.path("w/cms/a/b/d")).unwrap();
    std::os::unix::fs::symlink("/etc", f.path("w/cms/a/b/d/abs")).unwrap();
    let mut e = f.loaded();
    let mv = |src: &str, dst: &str| {
        op(
            "fs.move",
            json!({"src": rp(src), "dst": rp(dst), "overwrite": false}),
        )
    };
    assert_eq!(code(&call(&mut e, &mv("a/b/c/x", "x"))), "symlinkEscape");
    assert_eq!(
        code(&call(&mut e, &mv("a/b/c", "c"))),
        "symlinkEscape",
        "a folder holding one"
    );
    assert_eq!(
        code(&call(&mut e, &mv("a/b/d", "a/d"))),
        "symlinkEscape",
        "an absolute link"
    );
    assert!(exists(&f.path("w/cms/a/b/c/x")) && !exists(&f.path("w/cms/x")));
    // Moved where it still leads inside, it moves.
    ok(&call(&mut e, &mv("a/b/c", "a/b/c2")));
}

#[test]
fn scripts_fs_copied_links_put_every_parent_first() {
    use super::link_stays_inside;
    let at = |n: usize| vec!["d".to_owned(); n];
    assert!(link_stays_inside(&at(2), std::path::Path::new("../../x")));
    assert!(!link_stays_inside(&at(1), std::path::Path::new("../../x")));
    assert!(
        !link_stays_inside(&at(3), std::path::Path::new("a/../..")),
        "a name before .."
    );
    assert!(!link_stays_inside(&at(3), std::path::Path::new("sub/../x")));
    assert!(!link_stays_inside(&at(3), std::path::Path::new("/etc")));
}

#[test]
fn scripts_fs_ask_never_writes_over_or_moves_in_a_run_nobody_started() {
    let f = setup(FsAccess::Delete);
    f.grant(ScriptGrants {
        destructive: Destructive::Ask,
        ..grants(vec![repo_grant("cms", FsAccess::Delete, false)])
    });
    std::fs::write(f.path("w/cms/a.txt"), "keep").unwrap();
    let schedule = deckpilot_plugin_abi::capability::RunContext::workflow(
        deckpilot_plugin_abi::capability::RunCause::Schedule,
        None,
    );
    let mut e = f.loaded_in(schedule);
    let write = |rel: &str| {
        op(
            "fs.writeFile",
            json!({"path": rp(rel), "content": "new", "encoding": "utf8", "createOnly": false}),
        )
    };
    assert_eq!(code(&call(&mut e, &write("a.txt"))), "notUserInitiated");
    let mv = op(
        "fs.move",
        json!({"src": rp("a.txt"), "dst": rp("a.bak"), "overwrite": false}),
    );
    assert_eq!(code(&call(&mut e, &mv)), "notUserInitiated");
    // A chunked write whose target appears after it opened never replaces it.
    let h = ok(&call(
        &mut e,
        &op(
            "fs.write.open",
            json!({"path": rp("b.txt"), "createOnly": false}),
        ),
    ))["handle"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(&call(
        &mut e,
        &op(
            "fs.write.chunk",
            json!({"handle": h, "content": "new", "encoding": "utf8"}),
        ),
    ));
    std::fs::write(f.path("w/cms/b.txt"), "theirs").unwrap();
    assert_eq!(
        code(&call(&mut e, &op("fs.write.commit", json!({"handle": h})))),
        "exists"
    );
    assert_eq!(
        std::fs::read_to_string(f.path("w/cms/b.txt")).unwrap(),
        "theirs"
    );
    assert_eq!(
        std::fs::read_to_string(f.path("w/cms/a.txt")).unwrap(),
        "keep"
    );
    // A new file is fine.
    ok(&call(&mut e, &write("c.txt")));
}

#[test]
fn scripts_fs_a_private_folder_is_known_by_its_identity() {
    use super::protected::Protected;
    let f = Fixture::new();
    let data = f.path("appdata");
    std::fs::create_dir_all(data.join("sub")).unwrap();
    let p = Protected::default().with_private([data.clone()]);
    assert!(p.forbids_identity(&data));
    assert!(p.forbids_identity(&data.join("sub")));
    assert!(
        p.forbids_identity(&data.join("not-yet/file")),
        "below it, not there yet"
    );
    assert!(p.forbids_identity(&f.root), "holds it");
    assert!(
        !p.forbids_identity(&f.root.join("other-new-file")),
        "a new file beside it"
    );
    // The same folder under another name: macOS's firmlink of the data
    // volume.
    #[cfg(target_os = "macos")]
    {
        let other =
            std::path::Path::new("/System/Volumes/Data").join(data.strip_prefix("/").unwrap());
        if other.is_dir() {
            assert!(!p.forbids(&other), "the text does not match");
            assert!(p.forbids_identity(&other), "the identity does");
        }
    }
}

#[test]
fn scripts_fs_shares_of_this_computer_are_refused() {
    use std::ffi::OsStr;
    let local = |server: &str, share: &str| {
        super::resolve::is_local_share(OsStr::new(server), OsStr::new(share))
    };
    assert!(local("localhost", "Users"));
    assert!(local("127.0.0.1", "c"));
    assert!(local("fileserver", "C$"));
    assert!(local("fileserver", "ADMIN$"));
    assert!(!local("fileserver", "projects"));
    // P7 blocker 3 of #469: every other spelling of this computer.
    for server in [
        "0.0.0.0",
        "2130706433",
        "0x7f000001",
        "10.0.0.5",
        "192.168.1.20",
        "fe80--1.ipv6-literal.net",
        "localhost.",
        "::1",
    ] {
        assert!(local(server, "projects"), "{server}");
    }
    if let Ok(name) = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")) {
        assert!(local(&name, "projects"));
        assert!(local(&format!("{name}.local"), "projects"));
        assert!(local(&format!("{name}."), "projects"));
    }
    assert!(!local("files.example.com", "projects"));
}

/// A repository folder in Windows' verbatim form (`\\?\C:\…`, what
/// `canonicalize` gives) resolves and stays protected.
#[cfg(windows)]
#[test]
fn scripts_fs_a_verbatim_repository_folder_resolves() {
    let f = setup(FsAccess::Delete);
    std::fs::write(f.path("w/cms/a.txt"), "a").unwrap();
    {
        let mut project = f.host.project.lock().unwrap();
        let repo = &mut project.as_mut().unwrap().repositories[0];
        repo.folder = Some(std::fs::canonicalize(f.path("w/cms")).unwrap());
        assert!(
            repo.folder
                .as_ref()
                .unwrap()
                .to_string_lossy()
                .starts_with(r"\\?\")
        );
    }
    let mut e = f.loaded();
    let r = ok(&call(
        &mut e,
        &op(
            "fs.readFile",
            json!({"path": rp("a.txt"), "encoding": "utf8"}),
        ),
    ));
    assert_eq!(r["content"], "a");
    // The repository's own folder is still never removed.
    let r = call(
        &mut e,
        &op(
            "fs.remove",
            json!({"path": rp(""), "recursive": true, "dryRun": false}),
        ),
    );
    assert_eq!(code(&r), "protectedPath");
}

#[test]
fn scripts_fs_a_private_folder_made_after_the_start_is_known() {
    use super::protected::Protected;
    let f = Fixture::new();
    let data = f.path("later");
    let p = Protected::default().with_private([data.clone()]);
    std::fs::create_dir_all(&data).unwrap();
    let dir = cap_std::fs::Dir::open_ambient_dir(&data, cap_std::ambient_authority()).unwrap();
    assert!(p.forbids_handle(&dir, false));
    assert!(p.forbids_above(&dir, &data));
    let root = cap_std::fs::Dir::open_ambient_dir(&f.root, cap_std::ambient_authority()).unwrap();
    assert!(p.forbids_handle(&root, true), "holds it");
    assert!(!p.forbids_handle(&root, false));
}

/// The private folder reached through this computer's own share: refused
/// by its identity whatever the text says (needs the administrative share
/// the CI runner has).
#[cfg(windows)]
#[test]
fn scripts_fs_a_private_folder_through_localhost_is_refused_by_identity() {
    use super::protected::Protected;
    let f = Fixture::new();
    let data = f.path("appdata");
    std::fs::create_dir_all(&data).unwrap();
    let p = Protected::default().with_private([data.clone()]);
    let text = data.to_string_lossy().into_owned();
    let (drive, rest) = text.split_at(2);
    let unc = std::path::PathBuf::from(format!(r"\\localhost\{}${rest}", &drive[..1]));
    if cap_std::fs::Dir::open_ambient_dir(&unc, cap_std::ambient_authority()).is_err() {
        eprintln!("no administrative share here; skipped");
        return;
    }
    assert!(!p.forbids(&unc), "the text does not match");
    assert!(p.forbids_identity(&unc), "the identity does");
}

/// #475's review, H1: a link flipped between the check and the open never
/// reaches a private folder, because every link is followed by hand,
/// folder by folder, each checked on its handle.
#[cfg(unix)]
#[test]
fn scripts_fs_a_flipping_link_never_reaches_a_private_folder() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.path("appdata")).unwrap();
    std::fs::write(f.path("appdata/agent.token"), "SECRET").unwrap();
    std::fs::write(f.path("harmless"), "fine").unwrap();
    f.grant(grants(vec![FsScope {
        root: FsRoot::Directory {
            path: f.root.clone(),
            label: "tmp".into(),
        },
        access: FsAccess::Read,
        root_destructive: false,
    }]));
    let link = f.path("x");
    std::os::unix::fs::symlink("harmless", &link).unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = stop.clone();
    let flipping = link.clone();
    let flipper = std::thread::spawn(move || {
        let mut i = 0u32;
        while !flag.load(std::sync::atomic::Ordering::Relaxed) {
            let target = if i.is_multiple_of(2) {
                "appdata/agent.token"
            } else {
                "harmless"
            };
            let tmp = flipping.with_extension("new");
            let _ = std::fs::remove_file(&tmp);
            let _ = std::os::unix::fs::symlink(target, &tmp);
            let _ = std::fs::rename(&tmp, &flipping);
            i += 1;
        }
    });
    let mut e = f.loaded();
    let read = op(
        "fs.readFile",
        json!({"path": {"path": link.to_string_lossy()}, "encoding": "utf8"}),
    );
    for _ in 0..300 {
        if let deckpilot_plugin_abi::CallResult::Ok(v) = call(&mut e, &read) {
            assert_ne!(v["content"], "SECRET", "a private file was read");
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    flipper.join().unwrap();
}

/// Links that climb within the scope (pnpm's `node_modules`) are read.
#[cfg(unix)]
#[test]
fn scripts_fs_a_link_that_climbs_within_the_scope_is_read() {
    let f = setup(FsAccess::Read);
    std::fs::create_dir_all(f.path("w/cms/.pnpm/pkg")).unwrap();
    std::fs::write(f.path("w/cms/.pnpm/pkg/index.js"), "ok").unwrap();
    std::fs::create_dir_all(f.path("w/cms/node_modules")).unwrap();
    std::os::unix::fs::symlink("../.pnpm/pkg", f.path("w/cms/node_modules/pkg")).unwrap();
    let mut e = f.loaded();
    let r = ok(&call(
        &mut e,
        &op(
            "fs.readFile",
            json!({"path": rp("node_modules/pkg/index.js"), "encoding": "utf8"}),
        ),
    ));
    assert_eq!(r["content"], "ok");
    // One that climbs above the scope's root does not.
    std::os::unix::fs::symlink("../../../..", f.path("w/cms/node_modules/up")).unwrap();
    let r = call(
        &mut e,
        &op("fs.list", json!({"path": rp("node_modules/up")})),
    );
    let refused = code(&r);
    assert!(
        matches!(refused.as_str(), "symlinkEscape" | "protectedPath"),
        "{refused}"
    );
}

// --- the Scripts guest review of plugins #100 -------------------------------

fn chunk(rel: &str, offset: u64, version: Option<&Value>) -> Value {
    let mut args = json!({"path": rp(rel), "encoding": "base64", "offset": offset, "length": 4});
    if let Some(v) = version {
        args["ifVersion"] = v.clone();
    }
    op("fs.readFile", args)
}

#[test]
fn scripts_fs_a_chunked_read_never_joins_two_versions_of_a_file() {
    let f = setup(FsAccess::Read);
    std::fs::write(f.path("w/cms/a.bin"), b"AAAABBBB").unwrap();
    let mut e = f.loaded();
    // Unchanged: the later chunk comes from the same file.
    let first = ok(&call(&mut e, &chunk("a.bin", 0, None)));
    let second = ok(&call(&mut e, &chunk("a.bin", 4, Some(&first["version"]))));
    assert_eq!(second["content"], "QkJCQg==");
    assert_eq!(second["eof"], true);
    // Rewritten in place at the same length between two chunks.
    let first = ok(&call(&mut e, &chunk("a.bin", 0, None)));
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(f.path("w/cms/a.bin"), b"CCCCDDDD").unwrap();
    let r = call(&mut e, &chunk("a.bin", 4, Some(&first["version"])));
    assert_eq!(code(&r), "fileChanged");
    // Replaced by another file at the same path.
    let first = ok(&call(&mut e, &chunk("a.bin", 0, None)));
    std::fs::write(f.path("w/cms/b.bin"), b"EEEEFFFF").unwrap();
    std::fs::rename(f.path("w/cms/b.bin"), f.path("w/cms/a.bin")).unwrap();
    let r = call(&mut e, &chunk("a.bin", 4, Some(&first["version"])));
    assert_eq!(code(&r), "fileChanged");
    // A version is for one read of one file, and used up at its end.
    let first = ok(&call(&mut e, &chunk("a.bin", 0, None)));
    std::fs::write(f.path("w/cms/c.bin"), b"GGGGHHHH").unwrap();
    let r = call(&mut e, &chunk("c.bin", 4, Some(&first["version"])));
    assert_eq!(code(&r), "fileChanged");
    let r = call(&mut e, &chunk("a.bin", 4, Some(&first["version"])));
    assert_eq!(code(&r), "invalidArgument", "the refused one is gone");
    // A later chunk without a version is refused before the agent.
    let r = call(&mut e, &chunk("a.bin", 4, None));
    assert_eq!(code(&r), "call:BadRequest");
}
