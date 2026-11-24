//! The rules of plugin API 1.16's Calls kinds (`docs/calls.md`): the
//! `calls` permission, `contributes.callOverlay` and
//! `contributes.layoutTemplates`.
//!
//! - Each needs `engines.pluginApi` `^1.16`.
//! - Each is privileged ([`super::privileged::usages`] lists them, so the
//!   publisher gate refuses them outside a bundled first-party plugin), and
//!   only the plugin `deckpilot.calls` declares them.
//! - The overlay and the templates need the `calls` permission, the
//!   templates the overlay, and a template's pages are pages of the overlay.
//! - A plugin with `calls` declares no `network`, `credentials`, `openUrl`
//!   or `tools`: what it learns about calls leaves it only through the
//!   host's own faces.
//! - A template places only the plugin's own actions, with parameters
//!   those actions declare, on controls a model of its shape has.

use super::{Checker, IssueCode, Manifest, PermissionKind, default_value};
use crate::calls::{
    CALLS_API, CALLS_PLUGIN_ID, LayoutTemplate, MAX_LAYOUT_TEMPLATES, MAX_TEMPLATE_CONTROLS,
    TemplateControl, TemplateShape,
};

/// Every place a manifest uses a Calls kind, with its JSON path and what it
/// is.
pub(super) fn usages(m: &Manifest) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for (index, permission) in m.permissions.iter().enumerate() {
        if permission.kind() == PermissionKind::Calls {
            found.push((
                format!("permissions[{index}]"),
                "the `calls` permission".to_owned(),
            ));
        }
    }
    if m.contributes.call_overlay.is_some() {
        found.push(("contributes.callOverlay".into(), "`callOverlay`".into()));
    }
    if m.contributes.layout_templates.is_some() {
        found.push((
            "contributes.layoutTemplates".into(),
            "`layoutTemplates`".into(),
        ));
    }
    found
}

pub(super) fn check(c: &mut Checker, m: &Manifest) {
    let used = usages(m);
    if used.is_empty() {
        return;
    }
    let required = m.engines.plugin_api;
    if required.minimum < CALLS_API {
        for (path, what) in &used {
            c.error(
                path,
                IssueCode::Conflict,
                format!(
                    "{what} needs plugin API {CALLS_API}: set engines.pluginApi to \
                     `^{CALLS_API}` (it is `{required}`)"
                ),
            );
        }
    }
    if m.id.as_str() != CALLS_PLUGIN_ID {
        for (path, what) in &used {
            c.error(
                path,
                IssueCode::Conflict,
                format!("{what} is only for the plugin `{CALLS_PLUGIN_ID}`"),
            );
        }
    }
    let declared = m.permission(PermissionKind::Calls).is_some();
    if !declared {
        for (path, what) in used.iter().filter(|(p, _)| p.starts_with("contributes.")) {
            c.error(
                path,
                IssueCode::MissingPermission,
                format!("{what} needs the `calls` permission"),
            );
        }
    }
    if declared {
        for (index, permission) in m.permissions.iter().enumerate() {
            let kind = permission.kind();
            if matches!(
                kind,
                PermissionKind::Network
                    | PermissionKind::Credentials
                    | PermissionKind::OpenUrl
                    | PermissionKind::Tools
            ) {
                c.error(
                    format!("permissions[{index}]"),
                    IssueCode::Conflict,
                    format!(
                        "a plugin with `calls` cannot also declare `{}`: what it learns about \
                         calls leaves it only through the host's own faces",
                        kind.as_str()
                    ),
                );
            }
        }
    }
    overlay(c, m);
    templates(c, m);
}

fn overlay(c: &mut Checker, m: &Manifest) {
    let Some(overlay) = &m.contributes.call_overlay else {
        return;
    };
    c.count("contributes.callOverlay.pages", overlay.pages.len(), 1, 3);
    c.unique("contributes.callOverlay.pages", overlay.pages.iter());
}

fn templates(c: &mut Checker, m: &Manifest) {
    let Some(templates) = &m.contributes.layout_templates else {
        return;
    };
    let path = "contributes.layoutTemplates";
    if m.contributes.call_overlay.is_none() {
        c.error(
            path,
            IssueCode::MissingPermission,
            "`layoutTemplates` are the pages of `callOverlay`: declare it",
        );
    }
    c.count(path, templates.len(), 1, MAX_LAYOUT_TEMPLATES);
    c.unique(path, templates.iter().map(|t| t.shape));
    for (index, template) in templates.iter().enumerate() {
        self::template(c, m, &format!("{path}[{index}]"), template);
    }
}

fn template(c: &mut Checker, m: &Manifest, path: &str, template: &LayoutTemplate) {
    c.range(
        &format!("{path}.version"),
        u64::from(template.version),
        1,
        1000,
    );
    let pages = format!("{path}.pages");
    c.count(&pages, template.pages.len(), 1, 3);
    c.unique(&pages, template.pages.iter().map(|p| p.page));
    let overlay_pages = m
        .contributes
        .call_overlay
        .as_ref()
        .map(|o| o.pages.as_slice())
        .unwrap_or_default();
    for (p, page) in template.pages.iter().enumerate() {
        let at = format!("{pages}[{p}]");
        if m.contributes.call_overlay.is_some() && !overlay_pages.contains(&page.page) {
            c.error(
                format!("{at}.page"),
                IssueCode::UnknownReference,
                format!("`{}` is not a page of `callOverlay`", page.page),
            );
        }
        let controls = format!("{at}.controls");
        c.count(&controls, page.controls.len(), 0, MAX_TEMPLATE_CONTROLS);
        c.unique(
            &controls,
            page.controls
                .iter()
                .map(|k| format!("{} {}", k.control.as_str(), k.index)),
        );
        for (k, control) in page.controls.iter().enumerate() {
            self::control(c, m, &format!("{controls}[{k}]"), template.shape, control);
        }
    }
}

fn control(
    c: &mut Checker,
    m: &Manifest,
    path: &str,
    shape: TemplateShape,
    control: &TemplateControl,
) {
    let max = shape.max_controls(control.control);
    if control.index >= max {
        c.error(
            format!("{path}.index"),
            IssueCode::Range,
            if max == 0 {
                format!(
                    "a `{shape}` device has no {} controls",
                    control.control.as_str()
                )
            } else {
                format!(
                    "a `{shape}` device has {max} {} controls: the index is 0 to {}",
                    control.control.as_str(),
                    max - 1
                )
            },
        );
    }
    let Some(action) = m
        .contributes
        .actions()
        .iter()
        .find(|a| a.id == control.action)
    else {
        c.error(
            format!("{path}.action"),
            IssueCode::UnknownReference,
            format!(
                "`{}` is not one of this plugin's actions (contributes.actions[].id)",
                control.action
            ),
        );
        return;
    };
    let given = control.parameters.as_ref();
    for (key, value) in given.into_iter().flatten() {
        let field = format!("{path}.parameters.{key}");
        match action.parameters.iter().find(|p| &p.key == key) {
            Some(parameter) => default_value(c, &field, &parameter.kind, value),
            None => c.error(
                field,
                IssueCode::UnknownReference,
                format!("`{key}` is not a parameter of the action `{}`", action.id),
            ),
        }
    }
    for parameter in &action.parameters {
        let set = given.is_some_and(|p| p.contains_key(&parameter.key));
        if parameter.required && parameter.default.is_none() && !set {
            c.error(
                format!("{path}.parameters"),
                IssueCode::InvalidValue,
                format!(
                    "the required parameter `{}` of `{}` has no default: give it a value",
                    parameter.key, action.id
                ),
            );
        }
    }
}
