//! The capability operations of plugin API 1.14, connections
//! (`docs/marketplace/sdk/capabilities.md`, "Connections"):
//! `connections.get`, `connections.reportStatus`, `time.sleep`, and the
//! `connection` member of `http.fetch`.
//!
//! A connection is a host-managed instance of one of the plugin's declared
//! connection types: its own server, consent, secret and status. A guest
//! names a connection only by its opaque id; it never sees the secret, an
//! auth header or another connection's origin, and the host refuses an id
//! the invocation is not bound to. Only first-party plugins shipped inside
//! the app may declare the `connections` permission.
//!
//! Added compatibly within ABI 1: a guest built before them never sends
//! them, and a host or agent that predates them refuses the unknown op.
//! An agent that knows the operations but does not carry them out yet
//! answers [`CallErrorCode::UnsupportedOp`](crate::CallErrorCode::UnsupportedOp).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capability::{HttpMethod, RequestError};

/// The prefix of a connection id.
pub const CONNECTION_ID_PREFIX: &str = "cn_";
/// Characters after [`CONNECTION_ID_PREFIX`]: a ULID in Crockford base32.
pub const CONNECTION_ID_BODY: usize = 26;
/// Longest `detail` code of `connections.reportStatus`.
pub const MAX_STATUS_DETAIL: usize = 48;
/// Longest single `time.sleep`, in milliseconds. The host also keeps
/// one second of the invocation's deadline free.
pub const MAX_SLEEP_MS: u32 = 30_000;

/// Whether `id` is a connection id: `cn_` and 26 characters of Crockford
/// base32 (digits and upper-case letters without `I`, `L`, `O` and `U`).
pub fn is_connection_id(id: &str) -> bool {
    id.strip_prefix(CONNECTION_ID_PREFIX).is_some_and(|body| {
        body.len() == CONNECTION_ID_BODY
            && body
                .bytes()
                .all(|b| b.is_ascii_digit() || (b.is_ascii_uppercase() && !b"ILOU".contains(&b)))
    })
}

/// `connections.get`: one connection the invocation is bound to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionRef {
    pub connection: String,
}

impl ConnectionRef {
    pub fn validate(&self) -> Result<(), RequestError> {
        if is_connection_id(&self.connection) {
            Ok(())
        } else {
            Err(RequestError::Invalid("connection"))
        }
    }
}

/// Where a connection's server runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Deployment {
    /// The provider's public service, at the manifest's fixed origins.
    Cloud,
    /// A server the user entered.
    SelfHosted,
    /// A deployment this guest does not know yet.
    #[serde(other)]
    Other,
}

/// A connection's status, as the host shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionStatus {
    /// The last request or test worked.
    Ok,
    /// The server refused the secret (401).
    Unauthorized,
    /// The secret lacks a permission an operation needed (403).
    InsufficientScope,
    /// The server could not be reached (DNS, connection, time limit).
    Unreachable,
    /// The server's certificate is not trusted by the system.
    TlsFailed,
    /// The user must sign in again (host only).
    NeedsReconnect,
    /// The user turned the connection off (host only).
    Disabled,
    /// Never tested (host only).
    Untested,
    /// A status this guest does not know yet.
    #[serde(other)]
    Other,
}

impl ConnectionStatus {
    /// The statuses a plugin may report: what a request found. The host's
    /// own states (disabled, untested, needs reconnect) are never set by
    /// a plugin.
    pub const REPORTABLE: [Self; 5] = [
        Self::Ok,
        Self::Unauthorized,
        Self::InsufficientScope,
        Self::Unreachable,
        Self::TlsFailed,
    ];

    pub fn is_reportable(self) -> bool {
        Self::REPORTABLE.contains(&self)
    }
}

/// The `ok` value of `connections.get`: everything about the connection but
/// its secret. Unknown fields are **ignored** (host to guest only), so the
/// agent can add to it within ABI 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    pub id: String,
    /// The id of one of the manifest's `contributes.connectionTypes`.
    #[serde(rename = "type")]
    pub type_id: String,
    pub deployment: Deployment,
    /// The normalized API base: `https://host[:port][/prefix]`, no
    /// trailing `/`.
    pub api_base: String,
    /// The web origin pages are opened on.
    pub web_base: String,
    /// What the connection may send: `GET` until the user allows changes.
    #[serde(default)]
    pub methods: Vec<HttpMethod>,
    pub status: ConnectionStatus,
    /// What the plugin's last test reported for it (no secrets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Value>,
}

impl ConnectionInfo {
    /// Whether the connection may send `method`.
    pub fn allows(&self, method: HttpMethod) -> bool {
        self.methods.contains(&method)
    }
}

/// `connections.reportStatus`: what a request found about a bound
/// connection, shown by the host on the connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionStatusReport {
    pub connection: String,
    pub status: ConnectionStatus,
    /// A code (`[a-z][a-z0-9-]*`, at most [`MAX_STATUS_DETAIL`]), never a
    /// text of the server's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ConnectionStatusReport {
    pub fn validate(&self) -> Result<(), RequestError> {
        if !is_connection_id(&self.connection) {
            return Err(RequestError::Invalid("connection"));
        }
        if !self.status.is_reportable() {
            return Err(RequestError::Invalid("status"));
        }
        if let Some(detail) = &self.detail {
            let mut bytes = detail.bytes();
            let ok = detail.len() <= MAX_STATUS_DETAIL
                && bytes.next().is_some_and(|b| b.is_ascii_lowercase())
                && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            if !ok {
                return Err(RequestError::Invalid("detail"));
            }
        }
        Ok(())
    }
}

/// `time.sleep`: waits, woken early when the invocation is cancelled
/// (then answered with an error).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sleep {
    /// 1 to [`MAX_SLEEP_MS`].
    pub ms: u32,
}

impl Sleep {
    pub fn validate(&self) -> Result<(), RequestError> {
        if self.ms == 0 || self.ms > MAX_SLEEP_MS {
            Err(RequestError::TooLarge("ms"))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CapabilityRequest;

    const ID: &str = "cn_01JD7Q0000000000000000000A";

    fn parse(json: &str) -> Result<CapabilityRequest, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn connection_ids_are_opaque_ulids() {
        assert!(is_connection_id(ID));
        for bad in [
            "",
            "cn_",
            "github",
            "cn_01jd7q0000000000000000000a",
            "cn_01JD7Q000000000000000000OA",
            "cn_01JD7Q0000000000000000000",
            "cn_01JD7Q0000000000000000000AB",
            "xx_01JD7Q0000000000000000000A",
            "cn_01JD7Q0000000000000000000/",
        ] {
            assert!(!is_connection_id(bad), "{bad}");
        }
    }

    #[test]
    fn the_operations_are_strict_compatible_additions() {
        for (json, op) in [
            (
                r#"{"op":"connections.get","args":{"connection":"cn_01JD7Q0000000000000000000A"}}"#,
                "connections.get",
            ),
            (
                r#"{"op":"connections.reportStatus","args":{"connection":"cn_01JD7Q0000000000000000000A","status":"unauthorized","detail":"token-expired"}}"#,
                "connections.reportStatus",
            ),
            (r#"{"op":"time.sleep","args":{"ms":15000}}"#, "time.sleep"),
        ] {
            let r = parse(json).unwrap();
            assert_eq!(r.op(), op);
            r.validate().unwrap();
            assert_eq!(serde_json::to_string(&r).unwrap(), json);
        }
        // No plugin id, no secret, no URL, no other members.
        for bad in [
            r#"{"op":"connections.get","args":{"connection":"cn_01JD7Q0000000000000000000A","plugin":"x.y"}}"#,
            r#"{"op":"connections.get","args":{"connection":"cn_01JD7Q0000000000000000000A","secret":true}}"#,
            r#"{"op":"connections.list","args":{}}"#,
            r#"{"op":"connections.reportStatus","args":{"connection":"cn_01JD7Q0000000000000000000A"}}"#,
            r#"{"op":"time.sleep","args":{}}"#,
            r#"{"op":"time.sleep","args":{"ms":-1}}"#,
            r#"{"op":"time.sleep","args":{"ms":10,"until":"never"}}"#,
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn bounds_are_checked() {
        let get = |id: &str| {
            CapabilityRequest::ConnectionGet(ConnectionRef {
                connection: id.into(),
            })
        };
        assert_eq!(
            get("github").validate(),
            Err(RequestError::Invalid("connection"))
        );
        let report = |status, detail: Option<&str>| {
            CapabilityRequest::ConnectionReportStatus(ConnectionStatusReport {
                connection: ID.into(),
                status,
                detail: detail.map(str::to_owned),
            })
            .validate()
        };
        for status in ConnectionStatus::REPORTABLE {
            report(status, None).unwrap();
        }
        // The host's own states are never the plugin's to set.
        for status in [
            ConnectionStatus::Disabled,
            ConnectionStatus::Untested,
            ConnectionStatus::NeedsReconnect,
            ConnectionStatus::Other,
        ] {
            assert_eq!(
                report(status, None),
                Err(RequestError::Invalid("status")),
                "{status:?}"
            );
        }
        // A detail is a code, never the server's text.
        report(ConnectionStatus::Unreachable, Some("dns-failed")).unwrap();
        for detail in [
            "",
            "Bad credentials",
            "ghp_fake",
            "Token-1",
            &"a".repeat(49),
        ] {
            assert_eq!(
                report(ConnectionStatus::Unauthorized, Some(detail)),
                Err(RequestError::Invalid("detail")),
                "{detail}"
            );
        }
        let sleep = |ms| CapabilityRequest::TimeSleep(Sleep { ms }).validate();
        sleep(1).unwrap();
        sleep(MAX_SLEEP_MS).unwrap();
        assert_eq!(sleep(0), Err(RequestError::TooLarge("ms")));
        assert_eq!(sleep(MAX_SLEEP_MS + 1), Err(RequestError::TooLarge("ms")));
    }

    #[test]
    fn a_fetch_names_a_connection_or_a_slot_never_both() {
        let plain =
            r#"{"op":"http.fetch","args":{"method":"GET","url":"https://api.example.com/"}}"#;
        let r = parse(plain).unwrap();
        // Without a connection: byte-identical to the earlier form.
        assert_eq!(serde_json::to_string(&r).unwrap(), plain);
        let with = r#"{"op":"http.fetch","args":{"method":"GET","url":"https://code.example.com:8443/gitlab/api/v4/projects","connection":"cn_01JD7Q0000000000000000000A"}}"#;
        let r = parse(with).unwrap();
        r.validate().unwrap();
        assert_eq!(serde_json::to_string(&r).unwrap(), with);
        let both = r#"{"op":"http.fetch","args":{"method":"GET","url":"https://a.example.com/","auth":{"credential":"token","scheme":"bearer"},"connection":"cn_01JD7Q0000000000000000000A"}}"#;
        assert_eq!(
            parse(both).unwrap().validate(),
            Err(RequestError::Invalid("auth"))
        );
        let bad = r#"{"op":"http.fetch","args":{"method":"GET","url":"https://a.example.com/","connection":"other-plugin:cn_1"}}"#;
        assert_eq!(
            parse(bad).unwrap().validate(),
            Err(RequestError::Invalid("connection"))
        );
    }

    #[test]
    fn connection_info_has_no_secret_and_tolerates_additions() {
        let info: ConnectionInfo = serde_json::from_str(
            r#"{"id":"cn_01JD7Q0000000000000000000A","type":"github","deployment":"selfHosted",
                "apiBase":"https://ghe.example.com:8443/api/v3","webBase":"https://ghe.example.com:8443",
                "methods":["GET","POST"],"status":"ok","capabilities":{"pipelines":true},
                "addedLater":1}"#,
        )
        .unwrap();
        assert_eq!(info.deployment, Deployment::SelfHosted);
        assert!(info.allows(crate::HttpMethod::Post));
        assert!(!info.allows(crate::HttpMethod::Delete));
        let json = serde_json::to_value(&info).unwrap();
        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert!(
            keys.iter()
                .all(|k| !k.contains("secret") && !k.contains("token")),
            "{keys:?}"
        );
        let later: ConnectionInfo = serde_json::from_str(
            r#"{"id":"cn_01JD7Q0000000000000000000A","type":"gitlab","deployment":"dataCenter",
                "apiBase":"https://a.example.com","webBase":"https://a.example.com","status":"expiring"}"#,
        )
        .unwrap();
        assert_eq!(later.deployment, Deployment::Other);
        assert_eq!(later.status, ConnectionStatus::Other);
        assert!(later.methods.is_empty());
    }
}
