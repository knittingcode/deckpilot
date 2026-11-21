//! `package`: builds a `.dpplugin` from a plugin directory and its compiled
//! module (`docs/marketplace/artifact-contract.md`).
//!
//! The package holds `manifest.json`, `LICENSE` and every file the manifest
//! names, read from the plugin directory, plus the module as
//! `plugin.wasm`. Nothing else in the directory (sources, build output) is
//! packaged. The result is deterministic and checked by the same rules the
//! client applies (`deckpilot-plugin-package`), the module included.

use std::path::Path;

use deckpilot_plugin_manifest::Manifest;
use deckpilot_plugin_package::{
    ARTIFACT, BuiltPackage, LICENSE, MANIFEST, PackageBuilder, check_entry_path,
};

use super::{PublishError, Result};
use crate::privileged_policy::{self, Privileged, PrivilegedGate};
use crate::publisher::PublisherError;

fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| PublishError(format!("{}: {e}", path.display())))
}

/// Builds the package of the plugin in `plugin_dir` with `module` as its
/// executable artifact. `first_party` checks the manifest with the
/// first-party rules (a `deckpilot.*` plugin, `--first-party`); without it
/// the reserved publisher `deckpilot` is refused like every other.
///
/// The first-party rules refuse plugin API 1.9's privileged kinds while the
/// compiled gate is closed ([`PrivilegedGate::compiled`]). With it open, a
/// manifest that is exactly the reviewed privileged policy's entry for its
/// id and version is packaged with the bundled rules; any other privileged
/// manifest is refused ([`privileged_policy::check`]).
pub fn package(plugin_dir: &Path, module: &Path, first_party: bool) -> Result<BuiltPackage> {
    package_with_gate(plugin_dir, module, first_party, PrivilegedGate::compiled())
}

/// [`package`] with the given gate (tests of the opened gate only; the CLI
/// always uses the compiled one).
pub fn package_with_gate(
    plugin_dir: &Path,
    module: &Path,
    first_party: bool,
    gate: PrivilegedGate<'_>,
) -> Result<BuiltPackage> {
    let manifest_bytes = read(&plugin_dir.join(MANIFEST))?;
    let manifest = Manifest::from_slice(&manifest_bytes)
        .map_err(|e| PublishError(format!("{MANIFEST}: {e}")))?;
    let mut builder = PackageBuilder::new();
    let add = |builder: &mut PackageBuilder, path: &str, bytes: Vec<u8>| {
        builder
            .add(path, bytes)
            .map(|_| ())
            .map_err(|e| PublishError(e.to_string()))
    };
    add(&mut builder, MANIFEST, manifest_bytes.clone())?;
    add(&mut builder, LICENSE, read(&plugin_dir.join(LICENSE))?)?;
    add(&mut builder, ARTIFACT, read(module)?)?;
    for file in manifest.referenced_files() {
        // The path rules first: a path such as `assets/../../x` must be
        // refused before anything outside the plugin directory is read.
        let file: &str = &file;
        check_entry_path(file).map_err(|e| PublishError(e.to_string()))?;
        add(&mut builder, file, read(&plugin_dir.join(file))?)?;
    }
    if first_party {
        if manifest.id.publisher() != deckpilot_plugin_manifest::FIRST_PARTY_PUBLISHER {
            return Err(PublishError(format!(
                "--first-party is for deckpilot.* plugins, not {}",
                manifest.id
            )));
        }
        match privileged_policy::check(gate, &manifest_bytes, &manifest.id, &manifest.version) {
            Ok(Privileged::Reviewed { .. }) => {
                builder.reviewed_privileged();
            }
            // Closed: the first-party rules refuse the privileged kinds
            // with their own message (one issue per usage).
            Ok(Privileged::None) | Err(PublisherError::PrivilegedGateClosed { .. }) => {
                builder.first_party();
            }
            Err(e) => return Err(PublishError(format!("refusing to package: {e}"))),
        }
    }
    builder.build().map_err(|e| PublishError(e.to_string()))
}
