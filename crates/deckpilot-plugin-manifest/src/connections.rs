//! Plugin API 1.14, connections (`docs/marketplace/sdk/manifest.md`,
//! "Connections"): the connection types a plugin declares
//! (`contributes.connectionTypes`), the record schemas of its `records`
//! outputs (`contributes.recordSchemas`) and the options of a parameter
//! that a read-only internal action feeds (`parameters[].options`).
//!
//! A **connection** is a host-managed instance of a declared type that the
//! user creates in the app: its own server (origin and path prefix), its
//! own consent, its own secret and its own status. The plugin names a
//! connection by id; it never sees the secret, and it cannot pick a
//! connection the invocation was not bound to. This crate only describes
//! and checks the contract; the host side lands separately, and until it
//! does every connection operation is answered `unsupported`.

use serde::{Deserialize, Serialize};

use crate::compat::ApiLevel;
use crate::model::CredentialScheme;

/// The plugin API level of connections: the `connections` permission,
/// `openUrl.connections`, `contributes.connectionTypes`,
/// `contributes.recordSchemas`, `connection` parameters, parameter
/// `options`, internal actions and `records` outputs.
pub const CONNECTIONS_API: ApiLevel = ApiLevel::new(1, 14);

/// Most connection types one plugin declares.
pub const MAX_CONNECTION_TYPES: usize = 8;
/// Longest connection type id.
pub const MAX_CONNECTION_TYPE_ID: usize = 32;
/// Most authentication methods of one connection type.
pub const MAX_CONNECTION_AUTH: usize = 4;
/// Most extensions one connection type declares.
pub const MAX_CONNECTION_EXTENSIONS: usize = 16;
/// Longest extension id (`<type>.<name>`).
pub const MAX_EXTENSION_ID: usize = 64;
/// Longest `selfHosted.apiPath`.
pub const MAX_API_PATH: usize = 64;
/// Most record schemas one plugin declares.
pub const MAX_RECORD_SCHEMAS: usize = 16;
/// Most fields of one record schema.
pub const MAX_RECORD_FIELDS: usize = 32;
/// Most parameters an `options` source depends on.
pub const MAX_OPTIONS_DEPENDS_ON: usize = 4;
/// Most records one `records` output carries.
pub const MAX_RECORDS: usize = 200;
/// Largest serialized `records` output, in bytes.
pub const MAX_RECORDS_BYTES: usize = 64 * 1024;
/// The `{web}` placeholder of a self-hosted `setupUrl`: the connection's
/// web origin, as the user entered it.
pub const WEB_PLACEHOLDER: &str = "{web}";

/// `contributes.connectionTypes[]`: a kind of server the user can connect
/// the plugin to (a GitHub, a GitLab), with its public cloud endpoints
/// and how a self-hosted one is reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionType {
    /// `[a-z][a-z0-9]*(-[a-z0-9]+)*`, at most [`MAX_CONNECTION_TYPE_ID`].
    pub id: String,
    /// Translatable, 1-48 characters.
    pub label: String,
    /// The provider's public service, at fixed origins. A cloud
    /// connection never has another host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud: Option<CloudEndpoints>,
    /// A server the user runs or their company runs, at the origin the user
    /// enters (with a port or a path prefix).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_hosted: Option<SelfHosted>,
    /// How a connection of the type signs in, in the order the app offers.
    pub auth: Vec<ConnectionAuth>,
    /// Provider-only operations, `<type id>.<name>` (`github.checks`). A
    /// connection reports which of them its server supports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Vec<String>>,
}

impl ConnectionType {
    /// `extensions`, empty when absent.
    pub fn extensions(&self) -> &[String] {
        self.extensions.as_deref().unwrap_or_default()
    }
}

/// The fixed endpoints of a provider's public service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudEndpoints {
    /// The API base: an `https` URL, optionally with a path
    /// (`https://gitlab.com/api/v4`).
    pub api: String,
    /// The web origin pages are opened on (`https://github.com`): an
    /// `https` origin without a path.
    pub web: String,
}

/// How the API of a self-hosted server is found from the web address the
/// user enters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelfHosted {
    /// Appended to the web address for the API base (`/api/v4`): starts
    /// with `/`, no trailing `/`, no `.` or `..` segment.
    pub api_path: String,
    /// What the app calls such a server ("GitHub Enterprise Server");
    /// translatable, 1-48 characters. Absent: the type's label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// How a connection signs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionAuthMethod {
    /// A token the user creates on the server and pastes into the app's own
    /// secret field. The plugin never receives it.
    Pat,
}

/// One way of signing in to a connection type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionAuth {
    pub method: ConnectionAuthMethod,
    /// How the host injects the secret: `bearer`, `basic` or `header`
    /// (never `query`: a secret in a URL ends up in logs).
    pub scheme: CredentialScheme,
    /// The header of the `header` scheme (`PRIVATE-TOKEN`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_name: Option<String>,
    /// Where the user creates the token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_url: Option<SetupUrls>,
}

/// Where a token is created, per deployment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetupUrls {
    /// An `https` URL on the cloud web origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud: Option<String>,
    /// `{web}/…`: a path on the server the user entered, never another
    /// host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_hosted: Option<String>,
}

/// `contributes.recordSchemas[]`: the shape of the records a `records`
/// output carries. Flat: typed fields only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordSchema {
    /// Like an action id.
    pub id: String,
    pub fields: Vec<RecordField>,
}

/// One field of a [`RecordSchema`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordField {
    /// lowerCamelCase.
    pub key: String,
    pub kind: RecordFieldKind,
    /// Text a user wrote (a title, a description, a person's name): kept
    /// out of caches and of scripts that were not granted it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitive: Option<bool>,
}

impl RecordField {
    pub fn sensitive(&self) -> bool {
        self.sensitive == Some(true)
    }
}

/// The type of a [`RecordField`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordFieldKind {
    Text,
    Integer,
    Boolean,
    /// An `https` URL.
    Url,
    /// Milliseconds since the Unix epoch.
    Timestamp,
    /// A short closed-set word (`running`, `success`).
    State,
}

/// `parameters[].options`: the values a text parameter's picker offers,
/// read from one of the plugin's own read-only internal actions. The stored
/// value stays plain text, so a typed value or a dynamic reference still
/// works.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParameterOptions {
    /// The id of an action of the plugin with `internal: true`, a
    /// `readOnly` node and a `records` output.
    pub from: String,
    /// Parameters of the same action whose values the source receives
    /// (under the same keys); the picker asks again when one changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<String>>,
    /// The record field stored as the value.
    pub value: String,
    /// The record field shown. Absent: `value`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Whether the picker passes what the user types as the source's
    /// `search` parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<bool>,
}

impl ParameterOptions {
    pub fn depends_on(&self) -> &[String] {
        self.depends_on.as_deref().unwrap_or_default()
    }

    pub fn search(&self) -> bool {
        self.search == Some(true)
    }
}
