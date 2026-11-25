//! Plugin API 1.13: the Scripts plugin's kinds (`docs/scripts-bridge.md`).
//!
//! - the `scripts` permission (the bridge's ceiling, high tier);
//! - the `script` parameter kind (a script of the active profile, by id)
//!   and the `javascript` language of a `code` parameter;
//! - the `scripts` view (the app's Scripts page, a host view);
//! - the runtime constructs `runtime.limits.continuousMs` and
//!   `runtime.hostCalls`.
//!
//! Each needs `^1.13`. The permission, the `script` kind, the view and the
//! runtime constructs are privileged (first-party bundled plugins only, and
//! listed in the reviewed privileged policy), and only `deckpilot.scripts`
//! may declare the permission and the runtime constructs. `javascript` is
//! only a language for the editor and is open to every plugin at `^1.13`.

use super::{Checker, IssueCode, Manifest, ParameterKind, Permission, PermissionKind};
use crate::model::{
    CodeLanguage, MAX_CONTINUOUS_MS, MAX_RUNTIME_HOST_CALLS, MIN_CONTINUOUS_MS, SCRIPTS_API,
    SCRIPTS_PLUGIN, ViewKind,
};

/// Every place a manifest uses a kind of plugin API 1.13, with its JSON
/// path and what it is.
fn usages(m: &Manifest) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for (index, permission) in m.permissions.iter().enumerate() {
        if permission.kind() == PermissionKind::Scripts {
            found.push((
                format!("permissions[{index}]"),
                "the `scripts` permission".to_owned(),
            ));
        }
    }
    let actions = m.contributes.actions().iter().enumerate();
    for (a, action) in actions {
        for (p, parameter) in action.parameters.iter().enumerate() {
            let what = match parameter.kind {
                ParameterKind::Script {} => "a `script` parameter",
                ParameterKind::Code {
                    language: CodeLanguage::Javascript,
                    ..
                } => "the `javascript` language",
                _ => continue,
            };
            found.push((
                format!("contributes.actions[{a}].parameters[{p}].kind"),
                what.to_owned(),
            ));
        }
    }
    for (index, setting) in m.contributes.settings().iter().enumerate() {
        if matches!(
            setting.kind,
            ParameterKind::Code {
                language: CodeLanguage::Javascript,
                ..
            }
        ) {
            found.push((
                format!("contributes.settings.fields[{index}].kind"),
                "the `javascript` language".to_owned(),
            ));
        }
    }
    for (index, view) in m.contributes.views().iter().enumerate() {
        if view.kind == ViewKind::Scripts {
            found.push((
                format!("contributes.views[{index}].kind"),
                "the `scripts` view".to_owned(),
            ));
        }
    }
    if let Some(runtime) = &m.runtime {
        if runtime.limits.is_some() {
            found.push(("runtime.limits".into(), "`runtime.limits`".into()));
        }
        if runtime.host_calls.is_some() {
            found.push(("runtime.hostCalls".into(), "`runtime.hostCalls`".into()));
        }
    }
    found
}

pub(super) fn check(c: &mut Checker, m: &Manifest) {
    let required = m.engines.plugin_api;
    if required.minimum < SCRIPTS_API {
        for (path, what) in usages(m) {
            c.error(
                path,
                IssueCode::Conflict,
                format!(
                    "{what} needs plugin API {SCRIPTS_API}: set engines.pluginApi to \
                     `^{SCRIPTS_API}` (it is `{required}`)"
                ),
            );
        }
    }
    let scripts_plugin = m.id.as_str() == SCRIPTS_PLUGIN;
    let only_scripts = |c: &mut Checker, path: String, what: &str| {
        c.error(
            path,
            IssueCode::InvalidValue,
            format!("{what} is only for DeckPilot's Scripts plugin (`{SCRIPTS_PLUGIN}`)"),
        );
    };
    for (index, permission) in m.permissions.iter().enumerate() {
        if let Permission::Scripts {
            fs, process, shell, ..
        } = permission
        {
            if !scripts_plugin {
                only_scripts(
                    c,
                    format!("permissions[{index}]"),
                    "the `scripts` permission",
                );
            }
            if ![fs, process, shell].iter().any(|v| **v == Some(true)) {
                c.error(
                    format!("permissions[{index}]"),
                    IssueCode::InvalidValue,
                    "the `scripts` permission names at least one of fs, process and shell",
                );
            }
        }
    }
    runtime(c, m, scripts_plugin);
    for (index, view) in m.contributes.views().iter().enumerate() {
        if view.kind == ViewKind::Scripts && !scripts_plugin {
            only_scripts(
                c,
                format!("contributes.views[{index}].kind"),
                "the `scripts` view",
            );
        }
    }
    // A `script` parameter names a script the `scripts` permission runs.
    let declared = m.permission(PermissionKind::Scripts).is_some();
    for (a, action) in m.contributes.actions().iter().enumerate() {
        for (p, parameter) in action.parameters.iter().enumerate() {
            if matches!(parameter.kind, ParameterKind::Script {}) && !declared {
                c.error(
                    format!("contributes.actions[{a}].parameters[{p}].kind"),
                    IssueCode::MissingPermission,
                    "a `script` parameter needs the `scripts` permission",
                );
            }
        }
    }
    for (index, setting) in m.contributes.settings().iter().enumerate() {
        if matches!(setting.kind, ParameterKind::Script {}) {
            c.error(
                format!("contributes.settings.fields[{index}].kind"),
                IssueCode::InvalidValue,
                "a setting cannot be a script: a script is chosen per action",
            );
        }
    }
    // No exfiltration channel next to the bridge's process and file
    // authority (common rule 6, as for `tools`).
    if declared {
        for (index, permission) in m.permissions.iter().enumerate() {
            let kind = permission.kind();
            if matches!(
                kind,
                PermissionKind::Network | PermissionKind::Credentials | PermissionKind::OpenUrl
            ) {
                c.error(
                    format!("permissions[{index}]"),
                    IssueCode::Conflict,
                    format!(
                        "a plugin with `scripts` cannot also declare `{}`",
                        kind.as_str()
                    ),
                );
            }
        }
    }
}

/// `runtime.limits` and `runtime.hostCalls`: `deckpilot.scripts` only,
/// within their bounds.
fn runtime(c: &mut Checker, m: &Manifest, scripts_plugin: bool) {
    let only_scripts = |c: &mut Checker, path: &str, what: &str| {
        c.error(
            path,
            IssueCode::InvalidValue,
            format!("{what} is only for DeckPilot's Scripts plugin (`{SCRIPTS_PLUGIN}`)"),
        );
    };
    if let Some(runtime) = &m.runtime {
        if let Some(limits) = &runtime.limits {
            if !scripts_plugin {
                only_scripts(c, "runtime.limits", "`runtime.limits`");
            }
            if !(MIN_CONTINUOUS_MS..=MAX_CONTINUOUS_MS).contains(&limits.continuous_ms) {
                c.error(
                    "runtime.limits.continuousMs",
                    IssueCode::InvalidValue,
                    format!(
                        "continuousMs is {MIN_CONTINUOUS_MS} to {MAX_CONTINUOUS_MS} milliseconds"
                    ),
                );
            }
        }
        if let Some(calls) = runtime.host_calls {
            if !scripts_plugin {
                only_scripts(c, "runtime.hostCalls", "`runtime.hostCalls`");
            }
            if !(1..=MAX_RUNTIME_HOST_CALLS).contains(&calls) {
                c.error(
                    "runtime.hostCalls",
                    IssueCode::InvalidValue,
                    format!("hostCalls is 1 to {MAX_RUNTIME_HOST_CALLS}"),
                );
            }
        }
    }
}
