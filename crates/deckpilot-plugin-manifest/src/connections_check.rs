//! The rules of plugin API 1.14 (connections, [`crate::connections`]).
//!
//! [`check`] applies to every manifest: the shapes, the `^1.14`
//! requirement and the cross-field rules. The connection constructs (the
//! `connections` permission, `openUrl.connections`,
//! `contributes.connectionTypes` and `connection` parameters) are
//! privileged usages ([`usages`]): only a first-party bundled plugin may
//! declare them, and a signed release of one is admitted only with a
//! reviewed privileged policy that names them. The other constructs of the
//! level (parameter `options`, internal actions, `records` outputs and
//! `contributes.recordSchemas`) are first-party only too in 1.14
//! ([`gated`]), without being privileged.

use std::collections::BTreeSet;

use super::{Checker, IssueCode, Manifest, ParameterKind, Permission, camel_key};
use crate::connections::{
    CONNECTIONS_API, ConnectionAuth, ConnectionType, MAX_API_PATH, MAX_CONNECTION_AUTH,
    MAX_CONNECTION_EXTENSIONS, MAX_CONNECTION_TYPE_ID, MAX_CONNECTION_TYPES, MAX_EXTENSION_ID,
    MAX_OPTIONS_DEPENDS_ON, MAX_RECORD_FIELDS, MAX_RECORD_SCHEMAS, ParameterOptions,
    WEB_PLACEHOLDER,
};
use crate::identity::is_local_id;
use crate::model::{Action, CredentialScheme, Effect, HttpMethod, OutputKind, PermissionKind};

/// Headers a connection type may never name for its secret: the broker
/// refuses them in a plugin's request, and the host sets them itself.
const FORBIDDEN_AUTH_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
];

/// The connection constructs: privileged usages, in manifest order.
pub(crate) fn usages(m: &Manifest) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for (index, permission) in m.permissions.iter().enumerate() {
        if permission.opens_connections() {
            found.push((
                format!("permissions[{index}].connections"),
                "`openUrl.connections`".to_owned(),
            ));
        }
    }
    if m.contributes.connection_types.is_some() {
        found.push((
            "contributes.connectionTypes".into(),
            "`connectionTypes`".into(),
        ));
    }
    for (a, action) in m.contributes.actions().iter().enumerate() {
        for (p, parameter) in action.parameters.iter().enumerate() {
            if matches!(parameter.kind, ParameterKind::Connection { .. }) {
                found.push((
                    format!("contributes.actions[{a}].parameters[{p}].kind"),
                    "a `connection` parameter".to_owned(),
                ));
            }
        }
    }
    found
}

/// The other constructs of plugin API 1.14: first-party only, not
/// privileged.
pub(crate) fn gated(m: &Manifest) -> Vec<(String, String)> {
    let mut found = Vec::new();
    if m.contributes.record_schemas.is_some() {
        found.push(("contributes.recordSchemas".into(), "`recordSchemas`".into()));
    }
    for (a, action) in m.contributes.actions().iter().enumerate() {
        let path = format!("contributes.actions[{a}]");
        if action.internal.is_some() {
            found.push((format!("{path}.internal"), "`internal`".to_owned()));
        }
        for (p, parameter) in action.parameters.iter().enumerate() {
            if parameter.options.is_some() {
                found.push((
                    format!("{path}.parameters[{p}].options"),
                    "parameter `options`".to_owned(),
                ));
            }
        }
        for (o, output) in outputs(action).iter().enumerate() {
            if output.kind == Some(OutputKind::Records) || output.schema.is_some() {
                found.push((
                    format!("{path}.node.outputs[{o}]"),
                    "a `records` output".to_owned(),
                ));
            }
        }
    }
    found
}

fn outputs(action: &Action) -> &[crate::model::Output] {
    action
        .node
        .as_ref()
        .and_then(|n| n.outputs.as_deref())
        .unwrap_or_default()
}

/// The rules every manifest is checked against.
pub(super) fn check(c: &mut Checker, m: &Manifest) {
    let required = m.engines.plugin_api;
    if required.minimum < CONNECTIONS_API {
        let connections = m
            .permissions
            .iter()
            .enumerate()
            .filter(|(_, p)| p.kind() == PermissionKind::Connections)
            .map(|(i, _)| {
                (
                    format!("permissions[{i}]"),
                    "the `connections` permission".into(),
                )
            });
        for (path, what) in connections.chain(usages(m)).chain(gated(m)) {
            c.error(
                path,
                IssueCode::Conflict,
                format!(
                    "{what} needs plugin API {CONNECTIONS_API}: set engines.pluginApi to \
                     `^{CONNECTIONS_API}` (it is `{required}`)"
                ),
            );
        }
    }
    let types = connection_types(c, m);
    let permission = m.permissions.iter().enumerate().find_map(|(i, p)| match p {
        Permission::Connections { types, .. } => Some((i, types.as_slice())),
        _ => None,
    });
    match (&permission, m.contributes.connection_types.is_some()) {
        (None, true) => c.error(
            "contributes.connectionTypes",
            IssueCode::MissingPermission,
            "connection types need the `connections` permission",
        ),
        (Some((index, _)), false) => c.error(
            format!("permissions[{index}].types"),
            IssueCode::UnknownReference,
            "the `connections` permission needs `contributes.connectionTypes`",
        ),
        _ => {}
    }
    if let Some((index, listed)) = permission {
        let path = format!("permissions[{index}].types");
        for (i, id) in listed.iter().enumerate() {
            if !types.contains(id.as_str()) {
                c.error(
                    format!("{path}[{i}]"),
                    IssueCode::UnknownReference,
                    format!("`{id}` is not one of contributes.connectionTypes"),
                );
            }
        }
        for id in &types {
            if !listed.iter().any(|l| l == id) {
                c.error(
                    &path,
                    IssueCode::MissingPermission,
                    format!("the connection type `{id}` is not listed in the permission's types"),
                );
            }
        }
    }
    let granted: BTreeSet<&str> = permission
        .map(|(_, listed)| listed.iter().map(String::as_str).collect())
        .unwrap_or_default();
    for (index, p) in m.permissions.iter().enumerate() {
        if p.opens_connections() && permission.is_none() {
            c.error(
                format!("permissions[{index}].connections"),
                IssueCode::MissingPermission,
                "`openUrl.connections` needs the `connections` permission",
            );
        }
    }
    for (index, setting) in m.contributes.settings().iter().enumerate() {
        if matches!(setting.kind, ParameterKind::Connection { .. }) {
            c.error(
                format!("contributes.settings.fields[{index}].kind"),
                IssueCode::InvalidValue,
                "a setting cannot be a connection: the user picks one per action",
            );
        }
    }
    let schemas = record_schemas(c, m);
    for (index, action) in m.contributes.actions().iter().enumerate() {
        let path = format!("contributes.actions[{index}]");
        action_rules(c, m, &path, action, &granted, &schemas);
    }
}

/// `permissions[].kind == "connections"`: the shape alone.
pub(super) fn permission(c: &mut Checker, path: &str, permission: &Permission) {
    let Permission::Connections { types, methods, .. } = permission else {
        return;
    };
    let field = format!("{path}.types");
    c.count(&field, types.len(), 1, MAX_CONNECTION_TYPES);
    c.unique(&field, types);
    if let Some(methods) = methods {
        let field = format!("{path}.methods");
        c.count(&field, methods.len(), 1, usize::MAX);
        c.unique(&field, methods.iter().map(|x| format!("{x:?}")));
        if !methods.contains(&HttpMethod::Get) {
            c.error(
                field,
                IssueCode::InvalidValue,
                "must include GET: a connection is tested and read before anything else",
            );
        }
    }
}

/// `parameters[].kind` of type `connection`: the shape alone.
pub(super) fn parameter_kind(c: &mut Checker, path: &str, kind: &ParameterKind) {
    let ParameterKind::Connection { types, capability } = kind else {
        return;
    };
    let field = format!("{path}.types");
    c.count(&field, types.len(), 1, MAX_CONNECTION_TYPES);
    c.unique(&field, types);
    if let Some(capability) = capability {
        camel_key(c, &format!("{path}.capability"), capability);
    }
}

/// `contributes.connectionTypes`: every type's shape; the declared ids.
fn connection_types<'m>(c: &mut Checker, m: &'m Manifest) -> BTreeSet<&'m str> {
    let types = m.contributes.connection_types();
    if m.contributes.connection_types.is_some() {
        c.count(
            "contributes.connectionTypes",
            types.len(),
            1,
            MAX_CONNECTION_TYPES,
        );
    }
    for (index, kind) in types.iter().enumerate() {
        connection_type(c, &format!("contributes.connectionTypes[{index}]"), kind);
    }
    c.unique("contributes.connectionTypes", types.iter().map(|t| &t.id));
    types.iter().map(|t| t.id.as_str()).collect()
}

fn connection_type(c: &mut Checker, path: &str, kind: &ConnectionType) {
    if !is_local_id(&kind.id, MAX_CONNECTION_TYPE_ID) {
        c.error(
            format!("{path}.id"),
            IssueCode::InvalidId,
            format!(
                "`{}` must be 1-{MAX_CONNECTION_TYPE_ID} lowercase letters and digits, starting \
                 with a letter, with single hyphens",
                kind.id
            ),
        );
    }
    c.text(&format!("{path}.label"), &kind.label, 1, 48);
    if kind.cloud.is_none() && kind.self_hosted.is_none() {
        c.error(
            path,
            IssueCode::Count,
            "needs `cloud`, `selfHosted` or both: a type nobody can connect to is no use",
        );
    }
    if let Some(cloud) = &kind.cloud {
        if let Err(problem) = check_base(&cloud.api, true) {
            c.error(format!("{path}.cloud.api"), IssueCode::InvalidUrl, problem);
        }
        if let Err(problem) = check_base(&cloud.web, false) {
            c.error(format!("{path}.cloud.web"), IssueCode::InvalidUrl, problem);
        }
    }
    if let Some(hosted) = &kind.self_hosted {
        if let Err(problem) = check_api_path(&hosted.api_path) {
            c.error(
                format!("{path}.selfHosted.apiPath"),
                IssueCode::InvalidPath,
                format!("`{}` {problem}", hosted.api_path),
            );
        }
        c.opt_text(
            &format!("{path}.selfHosted.label"),
            hosted.label.as_ref(),
            48,
        );
        if hosted.label.as_ref().is_some_and(String::is_empty) {
            c.error(
                format!("{path}.selfHosted.label"),
                IssueCode::Length,
                "must not be empty",
            );
        }
    }
    let field = format!("{path}.auth");
    c.count(&field, kind.auth.len(), 1, MAX_CONNECTION_AUTH);
    c.unique(
        &field,
        kind.auth
            .iter()
            .map(|a| format!("{:?}/{:?}", a.method, a.scheme)),
    );
    for (i, auth) in kind.auth.iter().enumerate() {
        connection_auth(c, &format!("{field}[{i}]"), kind, auth);
    }
    let field = format!("{path}.extensions");
    if let Some(extensions) = &kind.extensions {
        c.count(&field, extensions.len(), 1, MAX_CONNECTION_EXTENSIONS);
    }
    for (i, extension) in kind.extensions().iter().enumerate() {
        let name = extension
            .strip_prefix(kind.id.as_str())
            .and_then(|rest| rest.strip_prefix('.'));
        if extension.len() > MAX_EXTENSION_ID || !name.is_some_and(|n| is_local_id(n, 48)) {
            c.error(
                format!("{field}[{i}]"),
                IssueCode::InvalidId,
                format!(
                    "`{extension}` must be `{}.<name>`, the name like an action id",
                    kind.id
                ),
            );
        }
    }
    c.unique(&field, kind.extensions());
}

fn connection_auth(c: &mut Checker, path: &str, kind: &ConnectionType, auth: &ConnectionAuth) {
    match auth.scheme {
        CredentialScheme::Query => c.error(
            format!("{path}.scheme"),
            IssueCode::InvalidValue,
            "a connection's secret is never sent in the URL: use bearer, basic or header",
        ),
        CredentialScheme::Header => match &auth.header_name {
            None => c.error(
                format!("{path}.headerName"),
                IssueCode::Conflict,
                "the `header` scheme needs headerName",
            ),
            Some(name) => {
                let token = (1..=64).contains(&name.len())
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
                let lower = name.to_ascii_lowercase();
                if !token
                    || FORBIDDEN_AUTH_HEADERS.contains(&lower.as_str())
                    || lower.starts_with("proxy-")
                    || lower.starts_with("sec-")
                {
                    c.error(
                        format!("{path}.headerName"),
                        IssueCode::InvalidValue,
                        format!("`{name}` is not a header a connection's secret may use"),
                    );
                }
            }
        },
        CredentialScheme::Bearer | CredentialScheme::Basic => {}
    }
    if auth.scheme != CredentialScheme::Header && auth.header_name.is_some() {
        c.error(
            format!("{path}.headerName"),
            IssueCode::Conflict,
            "only the `header` scheme takes headerName",
        );
    }
    let Some(setup) = &auth.setup_url else {
        return;
    };
    if let Some(url) = &setup.cloud {
        c.https_url(&format!("{path}.setupUrl.cloud"), url);
        let web = kind.cloud.as_ref().map(|cloud| cloud.web.as_str());
        if web.is_none_or(|web| !same_origin_path(url, web)) {
            c.error(
                format!("{path}.setupUrl.cloud"),
                IssueCode::InvalidUrl,
                "must be a page on the type's cloud web origin",
            );
        }
    }
    if let Some(url) = &setup.self_hosted {
        let rest = url.strip_prefix(WEB_PLACEHOLDER);
        let ok = kind.self_hosted.is_some()
            && rest.is_some_and(|rest| {
                rest.starts_with('/')
                    && rest.len() <= 256
                    && !rest.contains("//")
                    && !rest.contains(['{', '}', '\\', '#', '@'])
                    && !rest.chars().any(|ch| ch.is_control() || ch.is_whitespace())
                    && !rest
                        .split('/')
                        .any(|segment| segment == "." || segment == "..")
            });
        if !ok {
            c.error(
                format!("{path}.setupUrl.selfHosted"),
                IssueCode::InvalidUrl,
                format!(
                    "must be `{WEB_PLACEHOLDER}/…`, a page on the server the user entered (and the \
                     type needs `selfHosted`)"
                ),
            );
        }
    }
}

/// Whether `url` is on the origin of `web` (scheme, host and port).
fn same_origin_path(url: &str, web: &str) -> bool {
    match (url::Url::parse(url), url::Url::parse(web)) {
        (Ok(url), Ok(web)) => url.origin() == web.origin(),
        _ => false,
    }
}

/// A cloud API base (`with_path`) or web origin: `https`, a host the
/// manifest rules accept, no user name, query or fragment, and a web
/// origin without a path.
fn check_base(text: &str, with_path: bool) -> Result<(), String> {
    let rest = text
        .strip_prefix("https://")
        .ok_or_else(|| format!("`{text}` must start with https://"))?;
    if text.chars().count() > 256 {
        return Err("must be at most 256 characters".into());
    }
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    if authority.contains('@') || text.contains(['?', '#']) {
        return Err(format!(
            "`{text}` must not have a user name, a query or a fragment"
        ));
    }
    super::check_host(authority).map_err(|problem| format!("`{authority}` {problem}"))?;
    if authority.starts_with("*.") {
        return Err(format!("`{authority}` must be an exact host"));
    }
    if path.is_empty() || path == "/" {
        return Ok(());
    }
    if !with_path {
        return Err(format!("`{text}` must be an origin, without a path"));
    }
    check_api_path(path.trim_end_matches('/')).map_err(|problem| format!("`{text}` {problem}"))
}

/// `/segment[/segment…]`: unreserved characters only, no `.` or `..`, no
/// trailing `/`.
fn check_api_path(path: &str) -> Result<(), &'static str> {
    if path.len() > MAX_API_PATH {
        return Err("is too long (at most 64 characters)");
    }
    let Some(rest) = path.strip_prefix('/') else {
        return Err("must start with /");
    };
    let segments_ok = rest.split('/').all(|segment| {
        !segment.is_empty()
            && segment != "."
            && segment != ".."
            && segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~'))
    });
    if rest.is_empty() || !segments_ok {
        return Err(
            "must be one or more path segments of letters, digits, `-`, `_`, `.` or `~`, \
             without `.`, `..` or a trailing /",
        );
    }
    Ok(())
}

/// `contributes.recordSchemas`: shapes and the declared ids.
fn record_schemas<'m>(c: &mut Checker, m: &'m Manifest) -> BTreeSet<&'m str> {
    let schemas = m.contributes.record_schemas();
    if m.contributes.record_schemas.is_some() {
        c.count(
            "contributes.recordSchemas",
            schemas.len(),
            1,
            MAX_RECORD_SCHEMAS,
        );
    }
    for (index, schema) in schemas.iter().enumerate() {
        let path = format!("contributes.recordSchemas[{index}]");
        if !is_local_id(&schema.id, 48) {
            c.error(
                format!("{path}.id"),
                IssueCode::InvalidId,
                format!(
                    "`{}` must be 1-48 lowercase letters and digits, starting with a letter, \
                     with single hyphens",
                    schema.id
                ),
            );
        }
        let field = format!("{path}.fields");
        c.count(&field, schema.fields.len(), 1, MAX_RECORD_FIELDS);
        for (i, f) in schema.fields.iter().enumerate() {
            camel_key(c, &format!("{field}[{i}].key"), &f.key);
        }
        c.unique(&field, schema.fields.iter().map(|f| &f.key));
    }
    c.unique("contributes.recordSchemas", schemas.iter().map(|s| &s.id));
    let used: BTreeSet<&str> = m
        .contributes
        .actions()
        .iter()
        .flat_map(outputs)
        .filter_map(|o| o.schema.as_deref())
        .collect();
    for (index, schema) in schemas.iter().enumerate() {
        if !used.contains(schema.id.as_str()) {
            c.warn(
                format!("contributes.recordSchemas[{index}]"),
                format!("no output uses the record schema `{}`", schema.id),
            );
        }
    }
    schemas.iter().map(|s| s.id.as_str()).collect()
}

/// One action's 1.14 rules: its connection parameters, options, outputs
/// and, when internal, what an internal action cannot have.
fn action_rules(
    c: &mut Checker,
    m: &Manifest,
    path: &str,
    action: &Action,
    granted: &BTreeSet<&str>,
    schemas: &BTreeSet<&str>,
) {
    for (i, parameter) in action.parameters.iter().enumerate() {
        let field = format!("{path}.parameters[{i}]");
        if let ParameterKind::Connection { types, .. } = &parameter.kind {
            for (t, id) in types.iter().enumerate() {
                if !granted.contains(id.as_str()) {
                    c.error(
                        format!("{field}.kind.types[{t}]"),
                        IssueCode::MissingPermission,
                        format!("`{id}` is not a type of the `connections` permission"),
                    );
                }
            }
        }
        if let Some(options) = &parameter.options {
            parameter_options(
                c,
                m,
                &format!("{field}.options"),
                action,
                parameter,
                options,
            );
        }
    }
    for (o, output) in outputs(action).iter().enumerate() {
        let field = format!("{path}.node.outputs[{o}]");
        match (output.kind == Some(OutputKind::Records), &output.schema) {
            (true, None) => c.error(
                format!("{field}.schema"),
                IssueCode::Conflict,
                "a `records` output needs a schema (an id of contributes.recordSchemas)",
            ),
            (true, Some(schema)) if !schemas.contains(schema.as_str()) => c.error(
                format!("{field}.schema"),
                IssueCode::UnknownReference,
                format!("`{schema}` is not one of contributes.recordSchemas"),
            ),
            (false, Some(_)) => c.error(
                format!("{field}.schema"),
                IssueCode::Conflict,
                "only a `records` output takes a schema",
            ),
            _ => {}
        }
    }
    if !action.is_internal() {
        return;
    }
    let mut never = |member: &str, present: bool| {
        if present {
            c.error(
                format!("{path}.{member}"),
                IssueCode::Conflict,
                format!("an internal action is never on a key, so it has no `{member}`"),
            );
        }
    };
    never("controls", action.controls.is_some());
    never("state", action.state.is_some());
    never("title", action.title.is_some());
    never("editorNavigation", action.editor_navigation.is_some());
    never("replacedBy", action.replaced_by.is_some());
    if action.changes_context() {
        c.error(
            format!("{path}.node.changesContext"),
            IssueCode::Conflict,
            "an internal action never changes the run's project",
        );
    }
}

/// `parameters[].options`: a text parameter, fed by a read-only internal
/// action of the plugin with a `records` output whose schema has the
/// `value` and `label` fields, and receiving only parameters both actions
/// have.
fn parameter_options(
    c: &mut Checker,
    m: &Manifest,
    path: &str,
    action: &Action,
    parameter: &crate::model::Parameter,
    options: &ParameterOptions,
) {
    if !matches!(
        parameter.kind,
        ParameterKind::Text {
            multiline: None | Some(false),
            ..
        }
    ) {
        c.error(
            path,
            IssueCode::Conflict,
            "only a single-line text parameter takes `options`",
        );
    }
    let source = m
        .contributes
        .actions()
        .iter()
        .find(|a| a.id == options.from);
    let Some(source) = source else {
        c.error(
            format!("{path}.from"),
            IssueCode::UnknownReference,
            format!("`{}` is not one of the plugin's actions", options.from),
        );
        return;
    };
    if source.id == action.id {
        c.error(
            format!("{path}.from"),
            IssueCode::Conflict,
            "an action cannot feed its own parameter's options",
        );
    }
    let read_only = source
        .node
        .as_ref()
        .is_none_or(|n| n.effect() == Effect::ReadOnly);
    if !source.is_internal() || !read_only {
        c.error(
            format!("{path}.from"),
            IssueCode::Conflict,
            format!(
                "`{}` must be an internal, readOnly action to feed options",
                options.from
            ),
        );
    }
    let field = format!("{path}.dependsOn");
    c.count(
        &field,
        options.depends_on().len(),
        0,
        MAX_OPTIONS_DEPENDS_ON,
    );
    c.unique(&field, options.depends_on());
    for (i, key) in options.depends_on().iter().enumerate() {
        let here = action.parameters.iter().any(|p| &p.key == key);
        let there = source.parameters.iter().any(|p| &p.key == key);
        if key == &parameter.key || !here || !there {
            c.error(
                format!("{field}[{i}]"),
                IssueCode::UnknownReference,
                format!(
                    "`{key}` must be another parameter of this action and a parameter of `{}`",
                    options.from
                ),
            );
        }
    }
    if options.search()
        && !source
            .parameters
            .iter()
            .any(|p| p.key == "search" && matches!(p.kind, ParameterKind::Text { .. }))
    {
        c.error(
            format!("{path}.search"),
            IssueCode::UnknownReference,
            format!("`{}` has no text parameter `search`", options.from),
        );
    }
    options_fields(c, m, path, source, options);
}

/// `options.value` and `options.label`: fields of the source's record
/// schema.
fn options_fields(
    c: &mut Checker,
    m: &Manifest,
    path: &str,
    source: &Action,
    options: &ParameterOptions,
) {
    let schema = outputs(source)
        .iter()
        .filter(|o| o.kind == Some(OutputKind::Records))
        .find_map(|o| o.schema.as_deref())
        .and_then(|id| m.contributes.record_schemas().iter().find(|s| s.id == id));
    let Some(schema) = schema else {
        c.error(
            format!("{path}.from"),
            IssueCode::Conflict,
            format!("`{}` has no `records` output with a schema", options.from),
        );
        return;
    };
    for (member, key) in [
        ("value", Some(&options.value)),
        ("label", options.label.as_ref()),
    ] {
        let Some(key) = key else { continue };
        if !schema.fields.iter().any(|f| &f.key == key) {
            c.error(
                format!("{path}.{member}"),
                IssueCode::UnknownReference,
                format!(
                    "`{key}` is not a field of the record schema `{}`",
                    schema.id
                ),
            );
        }
    }
}
