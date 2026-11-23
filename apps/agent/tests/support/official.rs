//! The official plugins (`deckpilot.*` of `plugins/first-party/bundled.json`)
//! exactly as a release bundles them: the signed release artifacts that
//! `plugins/first-party/artifacts.json` locks, checked and unpacked by
//! `plugins/first-party/build.sh` (size and sha256, then `deckpilot-registry
//! release verify` with the **pinned** publisher keys, never `--keys`,
//! `released-pins.json`, the revocations and the audit log). The tests run
//! the files an installer ships, never a build of their own.
//!
//! `DECKPILOT_E2E_OFFICIAL_PLUGINS` names the `bundled/` directory a
//! `build.sh --out DIR` run wrote (`DIR/bundled`; CI stages it once per
//! job). Without it the first test that needs a plugin runs `build.sh`
//! itself into `target/official-plugins/<digest of the lock>/` (downloads
//! the artifacts unless `target/official-plugins/cache` has them; builds
//! `deckpilot-registry` unless `DECKPILOT_E2E_REGISTRY_TOOL` names one), and
//! later runs reuse it.
//!
//! Each release's files but its module are also committed, as its contract
//! (`plugins/first-party/contracts/<id>/`), for the tests that need them at
//! compile time (the module contracts, the privileged policy): [`dir`]
//! checks they are exactly what the release unpacked.
#![allow(dead_code)] // Each test binary uses part of it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde_json::Value;
use sha2::{Digest, Sha256};

/// The variable naming a staged `bundled/` directory.
pub const ENV: &str = "DECKPILOT_E2E_OFFICIAL_PLUGINS";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn json(path: &Path) -> Value {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `plugins/first-party/artifacts.json`.
pub fn lock() -> Value {
    json(&repo().join("plugins/first-party/artifacts.json"))
}

/// The version `artifacts.json` locks for `id`.
pub fn locked_version(id: &str) -> String {
    lock()["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == id)
        .unwrap_or_else(|| panic!("artifacts.json locks no release of {id}"))["version"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// The release bundles `id` from its locked artifact, with `class`.
pub fn assert_released(id: &str, class: &str) {
    let released = json(&repo().join("plugins/first-party/bundled.json"));
    let expected = serde_json::json!({
        "id": id,
        "version": locked_version(id),
        "class": class,
        "source": "artifact",
    });
    assert!(
        released["plugins"].as_array().unwrap().contains(&expected),
        "the release bundles {id} from its release artifact as a {class}-class plugin: {released}"
    );
}

/// The staged `bundled/` directory (see the module documentation).
pub fn bundle() -> &'static Path {
    static BUNDLE: OnceLock<PathBuf> = OnceLock::new();
    BUNDLE.get_or_init(|| {
        if let Some(dir) = std::env::var_os(ENV) {
            let dir = PathBuf::from(dir);
            assert!(
                dir.join("pins.json").is_file(),
                "{ENV}={} is not a bundled/ directory plugins/first-party/build.sh wrote",
                dir.display()
            );
            return dir;
        }
        stage()
    })
}

/// `build.sh` into `target/official-plugins/<digest>/`, once per lock.
fn stage() -> PathBuf {
    let repo = repo();
    let mut digest = Sha256::new();
    for file in ["artifacts.json", "bundled.json", "released-pins.json"] {
        digest.update(std::fs::read(repo.join("plugins/first-party").join(file)).unwrap());
    }
    let digest = digest
        .finalize()
        .iter()
        .take(8)
        .fold(String::new(), |mut hex, b| {
            use std::fmt::Write as _;
            let _ = write!(hex, "{b:02x}");
            hex
        });
    let root = repo.join("target/official-plugins");
    let out = root.join(&digest);
    if out.join("bundled/pins.json").is_file() {
        return out.join("bundled");
    }
    // Into a directory of this process, renamed into place when complete:
    // another test binary staging at the same time never sees half of it.
    let work = root.join(format!("{digest}.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    let mut command = Command::new("bash");
    command
        .current_dir(&repo)
        .arg("plugins/first-party/build.sh")
        .arg("--out")
        .arg(&work)
        .arg("--artifacts-dir")
        .arg(root.join("cache"))
        .arg("--target-dir")
        .arg(root.join("build"))
        .env_remove("DECKPILOT_TEST_ONLY_PUBLISHER_KEYS")
        .env_remove("CARGO_TARGET_DIR");
    if let Some(tool) = std::env::var_os("DECKPILOT_E2E_REGISTRY_TOOL") {
        command.arg("--registry-tool").arg(tool);
    }
    let status = command
        .status()
        .expect("bash runs plugins/first-party/build.sh");
    assert!(
        status.success(),
        "plugins/first-party/build.sh could not stage the official plugins"
    );
    assert!(
        !work.join("bundled/NOT-FOR-RELEASE").exists(),
        "the official plugins were verified with test-only keys"
    );
    if std::fs::rename(&work, &out).is_err() {
        // Staged by another test binary meanwhile.
        let _ = std::fs::remove_dir_all(&work);
    }
    out.join("bundled")
}

/// `bundled/<id>/`: the verified files of the release `artifacts.json`
/// locks for `id`. They must be exactly its contract,
/// `plugins/first-party/contracts/<id>/` (every file but the module, which
/// the compile-time contract and policy tests read), besides `plugin.wasm`.
pub fn dir(id: &str) -> PathBuf {
    let dir = bundle().join(id);
    let manifest = json(&dir.join("manifest.json"));
    assert_eq!(manifest["id"], id, "{}", dir.display());
    assert_eq!(
        manifest["version"].as_str().unwrap(),
        locked_version(id),
        "{} is not the locked release of {id}: stage it again",
        dir.display()
    );
    let contract = repo().join("plugins/first-party/contracts").join(id);
    let mut released = files(&dir);
    released.retain(|(name, _)| name != "plugin.wasm");
    assert!(
        released == files(&contract),
        "{} differs from the release {} unpacked: update the contract with the release",
        contract.display(),
        dir.display()
    );
    dir
}

/// Every file under `root`: its path relative to `root` (with `/`) and
/// its bytes, sorted.
fn files(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.push((name, std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

/// The manifest of `id`'s locked release.
pub fn manifest(id: &str) -> Value {
    json(&dir(id).join("manifest.json"))
}

/// The module of `id`'s locked release.
pub fn module(id: &str) -> PathBuf {
    dir(id).join("plugin.wasm")
}

/// Every file of `id`'s locked release, copied into `target` (created).
pub fn copy_into(id: &str, target: &Path) {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                copy(&path, &to.join(entry.file_name()));
            } else {
                std::fs::copy(&path, to.join(entry.file_name())).unwrap();
            }
        }
    }
    copy(&dir(id), target);
}
