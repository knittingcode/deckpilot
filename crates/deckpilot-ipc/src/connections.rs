//! Connections of plugins (plugin API 1.14, IPC protocol
//! [`CONNECTIONS_PROTOCOL`](crate::CONNECTIONS_PROTOCOL);
//! `docs/marketplace/sdk/permissions.md` "Connections").
//!
//! A connection is a server the user connects a plugin to, of one of the
//! connection types the plugin declares: its own address, consent, token
//! and status. The app creates, tests, renames, turns off and removes them;
//! the agent keeps them in the plugin database.
//!
//! **Secrets travel one way.** A token goes from the app to the agent in
//! [`Request::CreatePluginConnection`](crate::Request::CreatePluginConnection)
//! or [`Request::SetPluginConnectionSecret`](crate::Request::SetPluginConnectionSecret)
//! as a [`PluginSecret`] (never shown in `Debug`), and nothing ever sends it
//! back: a [`PluginConnection`] says only whether one is stored and since
//! when.

use serde::{Deserialize, Serialize};

use crate::plugins::{PluginSecret, check_text};

/// Longest connection display name, in characters.
pub const MAX_CONNECTION_NAME: usize = 64;
/// Longest server address the app sends.
pub const MAX_CONNECTION_URL: usize = 1024;

/// Where a connection's server runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionDeployment {
    /// The provider's public service (GitHub.com, GitLab.com).
    Cloud,
    /// A server the user entered (GitHub Enterprise Server, a self-managed
    /// GitLab).
    SelfHosted,
}

/// A connection's status, as the app shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionState {
    /// The last test or request worked.
    Ok,
    /// The server refused the token.
    Unauthorized,
    /// The token lacks a permission an operation needed.
    InsufficientScope,
    /// The server could not be reached.
    Unreachable,
    /// The server's certificate is not trusted by the system.
    TlsFailed,
    /// The user must enter a token again.
    NeedsReconnect,
    /// Never tested since its token was stored.
    Untested,
    /// Turned off by the user.
    Disabled,
    /// A state this app does not know.
    #[serde(other)]
    Other,
}

/// One connection. Never its secret.
#[expect(
    clippy::struct_excessive_bools,
    reason = "a wire record: each flag is one independent fact the app shows"
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginConnection {
    /// `cn_` and 26 characters; stable, never reused.
    pub id: String,
    /// One of the plugin's connection types (`github`, `gitlab`).
    #[serde(rename = "type")]
    pub type_id: String,
    pub display_name: String,
    pub deployment: ConnectionDeployment,
    /// The API base requests go to (`https://host[:port][/prefix]`).
    pub api_url: String,
    /// Where its pages are (`https://host[:port]`).
    pub web_url: String,
    /// The user allowed changes (creating merge requests, starting
    /// pipelines): methods beyond `GET`.
    pub allow_changes: bool,
    /// The user's consent to a server on a private network.
    pub private_network: bool,
    pub enabled: bool,
    pub state: ConnectionState,
    /// A code for the state (`dns-failed`, `token-expired`), never text of
    /// the server's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_detail: Option<String>,
    /// The account the last test found (`octo`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// What the last test found the server and token can do (the plugin's
    /// own record, no secrets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<serde_json::Value>,
    /// The default connection of its type.
    pub is_default: bool,
    /// A token is stored.
    pub has_secret: bool,
    /// When the token was stored (seconds since the epoch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_updated_at: Option<i64>,
    pub created_at: i64,
}

/// How a connection type signs in, as the app explains it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTypeAuth {
    /// `pat`: a token the user creates on the server.
    pub method: String,
    /// Where the token is created on the public service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_setup_url: Option<String>,
    /// Where it is created on a self-hosted server: a path template with
    /// `{web}`, the server the user entered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_hosted_setup_url: Option<String>,
}

/// One connection type a plugin declares, in the app's language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginConnectionType {
    pub id: String,
    pub label: String,
    /// The public service's addresses, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_api_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_web_url: Option<String>,
    /// Whether the user can enter their own server, and what it is called.
    pub self_hosted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_hosted_label: Option<String>,
    /// Appended to the address the user enters for the API (`/api/v4`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_hosted_api_path: Option<String>,
    pub auth: Vec<ConnectionTypeAuth>,
}

/// The connections of one plugin and the types it declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginConnections {
    pub plugin: String,
    /// Whether the plugin's `connections` permission is granted: without
    /// it no connection can be created and none is used.
    pub granted: bool,
    /// Whether the plugin may connect to a server on a private network
    /// (each one consented to on its own).
    pub private_network_allowed: bool,
    /// Whether the plugin may be allowed changes at all.
    pub changes_allowed: bool,
    pub types: Vec<PluginConnectionType>,
    pub connections: Vec<PluginConnection>,
}

/// What a test of a connection found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTestOutcome {
    pub ok: bool,
    /// A code (`unauthorized`, `tls-failed`, `dns-failed`, `no-token`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// What went wrong, for people; never the token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Whether `id` is a connection id: `cn_` and 26 characters of Crockford
/// base32 (digits and upper-case letters without `I`, `L`, `O`, `U`).
pub fn is_connection_id(id: &str) -> bool {
    id.strip_prefix("cn_").is_some_and(|body| {
        body.len() == 26
            && body
                .bytes()
                .all(|b| b.is_ascii_digit() || (b.is_ascii_uppercase() && !b"ILOU".contains(&b)))
    })
}

pub(crate) fn check_connection_id(id: &str) -> Result<(), String> {
    if is_connection_id(id) {
        Ok(())
    } else {
        Err("connection must be a connection id".into())
    }
}

pub(crate) fn check_name(name: &str) -> Result<(), String> {
    let chars = name.chars().count();
    if chars == 0 || chars > MAX_CONNECTION_NAME || name.chars().any(char::is_control) {
        return Err(format!(
            "a connection's name is 1 to {MAX_CONNECTION_NAME} characters without control \
             characters"
        ));
    }
    Ok(())
}

pub(crate) fn check_url(url: &str) -> Result<(), String> {
    if url.is_empty() || url.len() > MAX_CONNECTION_URL || url.chars().any(char::is_control) {
        return Err(format!(
            "a server address is 1 to {MAX_CONNECTION_URL} characters"
        ));
    }
    Ok(())
}

pub(crate) fn check_secret(secret: &PluginSecret) -> Result<(), String> {
    crate::plugins::check_secret(secret)
}

pub(crate) fn check_request(request: &crate::Request) -> Result<(), String> {
    use crate::Request;
    match request {
        Request::ListPluginConnections { plugin } => check_text("plugin", plugin),
        Request::CreatePluginConnection {
            plugin,
            connection_type,
            url,
            display_name,
            secret,
            ..
        } => {
            check_text("plugin", plugin)?;
            check_text("connection type", connection_type)?;
            if let Some(url) = url {
                check_url(url)?;
            }
            check_name(display_name)?;
            secret.as_ref().map_or(Ok(()), check_secret)
        }
        Request::SetPluginConnectionSecret {
            plugin,
            connection,
            secret,
        } => {
            check_text("plugin", plugin)?;
            check_connection_id(connection)?;
            check_secret(secret)
        }
        Request::UpdatePluginConnection {
            plugin,
            connection,
            display_name,
            ..
        } => {
            check_text("plugin", plugin)?;
            check_connection_id(connection)?;
            display_name.as_deref().map_or(Ok(()), check_name)
        }
        Request::TestPluginConnection { plugin, connection }
        | Request::RemovePluginConnection { plugin, connection } => {
            check_text("plugin", plugin)?;
            check_connection_id(connection)
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Request;

    /// A token is never in a request's `Debug`, every request is checked,
    /// and the answers carry no secret.
    #[test]
    fn requests_are_checked_and_never_show_the_token() {
        let create = Request::CreatePluginConnection {
            plugin: "deckpilot.git-providers".into(),
            connection_type: "gitlab".into(),
            deployment: ConnectionDeployment::SelfHosted,
            url: Some("https://code.example.com".into()),
            display_name: "Work".into(),
            allow_changes: false,
            private_network: false,
            secret: Some(PluginSecret::new("dp-test-secret-value")),
        };
        assert!(!format!("{create:?}").contains("dp-test-secret-value"));
        create.validate().unwrap();
        assert_eq!(create.since_protocol(), crate::CONNECTIONS_PROTOCOL);
        let json = serde_json::to_value(&create).unwrap();
        assert_eq!(json["type"], "createPluginConnection");
        assert_eq!(json["connectionType"], "gitlab");
        for bad in [
            Request::TestPluginConnection {
                plugin: "deckpilot.git-providers".into(),
                connection: "github".into(),
            },
            Request::UpdatePluginConnection {
                plugin: "deckpilot.git-providers".into(),
                connection: "cn_01JD7Q0000000000000000000A".into(),
                display_name: Some(String::new()),
                enabled: None,
                allow_changes: None,
                make_default: false,
            },
            Request::CreatePluginConnection {
                plugin: "deckpilot.git-providers".into(),
                connection_type: "gitlab".into(),
                deployment: ConnectionDeployment::SelfHosted,
                url: Some("x".repeat(MAX_CONNECTION_URL + 1)),
                display_name: "Work".into(),
                allow_changes: false,
                private_network: false,
                secret: None,
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
        let fields: Vec<String> = serde_json::to_value(PluginConnection {
            id: "cn_01JD7Q0000000000000000000A".into(),
            type_id: "gitlab".into(),
            display_name: "Work".into(),
            deployment: ConnectionDeployment::SelfHosted,
            api_url: "https://code.example.com/api/v4".into(),
            web_url: "https://code.example.com".into(),
            allow_changes: false,
            private_network: false,
            enabled: true,
            state: ConnectionState::Untested,
            state_detail: None,
            account: None,
            capabilities: None,
            is_default: true,
            has_secret: true,
            secret_updated_at: Some(1),
            created_at: 1,
        })
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
        assert!(
            fields.iter().all(|k| k == "hasSecret"
                || k == "secretUpdatedAt"
                || !k.to_ascii_lowercase().contains("secret")
                    && !k.to_ascii_lowercase().contains("token")),
            "{fields:?}"
        );
    }
}
