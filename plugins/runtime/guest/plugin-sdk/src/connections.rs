//! Plugin API 1.14, connections: [`Context::connection`],
//! [`Context::report_connection_status`], [`Context::sleep`] and
//! [`Request::connection`](super::Request::connection).
//!
//! A connection is a server the user connected in the app, with its own
//! secret. The plugin names it by the id its `connection` parameter holds;
//! the agent sends the request with that connection's secret, only inside
//! its origin, path prefix and methods, and only when the invocation is
//! bound to it. The secret never reaches the plugin. Only DeckPilot's own
//! plugins shipped inside the app can declare the `connections`
//! permission. An agent that does not carry the operations out yet
//! answers [`CapabilityErrorKind::Unsupported`].

use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_abi::connections::{
    ConnectionInfo, ConnectionRef, ConnectionStatus, ConnectionStatusReport, Sleep,
};

use super::{CapabilityError, CapabilityErrorKind, Context, malformed};

fn bad_request(op: &'static str, problem: &impl std::fmt::Display) -> CapabilityError {
    CapabilityError {
        op,
        kind: CapabilityErrorKind::BadRequest,
        detail: format!("the request's {problem}"),
        target: None,
        network: None,
    }
}

impl Context<'_> {
    /// The connection `id`, without its secret (op `connections.get`,
    /// plugin API 1.14): its type, deployment, API base and web origin,
    /// methods, status and what the last test found. Only a connection
    /// the invocation is bound to.
    ///
    /// ```
    /// use deckpilot_plugin_sdk::testing::MockHost;
    /// use deckpilot_plugin_sdk::{CallResult, CapabilityRequest, HttpMethod};
    ///
    /// let mut host = MockHost::new(|req| match req {
    ///     CapabilityRequest::ConnectionGet(c) => CallResult::Ok(serde_json::json!({
    ///         "id": c.connection, "type": "gitlab", "deployment": "selfHosted",
    ///         "apiBase": "https://code.example.com:8443/gitlab/api/v4",
    ///         "webBase": "https://code.example.com:8443/gitlab",
    ///         "methods": ["GET"], "status": "ok"
    ///     })),
    ///     _ => CallResult::Ok(serde_json::Value::Null),
    /// });
    /// let mut ctx = host.context();
    /// let c = ctx.connection("cn_01JD7Q0000000000000000000A").unwrap();
    /// assert_eq!(c.type_id, "gitlab");
    /// assert!(!c.allows(HttpMethod::Post));
    /// ```
    pub fn connection(&mut self, id: &str) -> Result<ConnectionInfo, CapabilityError> {
        let request = CapabilityRequest::ConnectionGet(ConnectionRef {
            connection: id.to_owned(),
        });
        if let Err(problem) = request.validate() {
            return Err(bad_request("connections.get", &problem));
        }
        let value = self.call("connections.get", &request)?;
        serde_json::from_value(value).map_err(|_| malformed("connections.get", "a connection"))
    }

    /// Tells the agent what a request found about the bound connection
    /// `id` (op `connections.reportStatus`): `ok` after a success, or
    /// `unauthorized`, `insufficientScope`, `unreachable`, `tlsFailed`,
    /// with an optional `detail` code (never a text of the server's). The
    /// app shows it on the connection.
    pub fn report_connection_status(
        &mut self,
        id: &str,
        status: ConnectionStatus,
        detail: Option<&str>,
    ) -> Result<(), CapabilityError> {
        let request = CapabilityRequest::ConnectionReportStatus(ConnectionStatusReport {
            connection: id.to_owned(),
            status,
            detail: detail.map(str::to_owned),
        });
        if let Err(problem) = request.validate() {
            return Err(bad_request("connections.reportStatus", &problem));
        }
        self.call("connections.reportStatus", &request).map(drop)
    }

    /// Waits `ms` milliseconds, at most
    /// [`MAX_SLEEP_MS`](deckpilot_plugin_abi::connections::MAX_SLEEP_MS)
    /// (op `time.sleep`, plugin API 1.14), and less when the invocation's
    /// deadline is near. Cancelling the run wakes it with an error, so a
    /// waiting action ends at once.
    pub fn sleep(&mut self, ms: u32) -> Result<(), CapabilityError> {
        let request = CapabilityRequest::TimeSleep(Sleep { ms });
        if let Err(problem) = request.validate() {
            return Err(bad_request("time.sleep", &problem));
        }
        self.call("time.sleep", &request).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use deckpilot_plugin_abi::connections::ConnectionStatus;
    use deckpilot_plugin_abi::{CallErrorCode, CallResult, CapabilityRequest};

    use crate::testing::MockHost;
    use crate::{CapabilityErrorKind, Request};

    const ID: &str = "cn_01JD7Q0000000000000000000A";

    #[test]
    fn bad_requests_never_reach_the_host() {
        let mut host = MockHost::allow_all();
        let mut ctx = host.context();
        for error in [
            ctx.connection("github").unwrap_err(),
            ctx.connection("cn_01jd7q0000000000000000000a").unwrap_err(),
            ctx.report_connection_status(ID, ConnectionStatus::Disabled, None)
                .unwrap_err(),
            ctx.report_connection_status(ID, ConnectionStatus::Ok, Some("Token was ghp_x"))
                .unwrap_err(),
            ctx.sleep(0).unwrap_err(),
            ctx.sleep(30_001).unwrap_err(),
        ] {
            assert_eq!(error.kind(), CapabilityErrorKind::BadRequest, "{error}");
        }
        assert_eq!(ctx.calls(), 0);
    }

    #[test]
    fn a_fetch_names_its_connection_and_never_a_slot_too() {
        let mut host = MockHost::new(|req| match req {
            CapabilityRequest::HttpFetch(f) if f.connection.as_deref() == Some(ID) => {
                assert!(f.auth.is_none());
                CallResult::Ok(serde_json::json!({"status": 200, "body": "[]"}))
            }
            _ => CallResult::error(CallErrorCode::NotPermitted, "not bound"),
        });
        let mut ctx = host.context();
        let r = ctx
            .fetch(Request::get("https://gitlab.example.com/api/v4/projects").connection(ID))
            .unwrap();
        assert_eq!(r.status, 200);
        assert!(
            ctx.http_get("https://gitlab.example.com/api/v4/projects")
                .unwrap_err()
                .is_denied()
        );
    }

    #[test]
    fn an_agent_without_the_operations_says_so() {
        let mut host =
            MockHost::new(|_| CallResult::error(CallErrorCode::UnsupportedOp, "not implemented"));
        let mut ctx = host.context();
        assert_eq!(
            ctx.connection(ID).unwrap_err().kind(),
            CapabilityErrorKind::Unsupported
        );
        assert_eq!(
            ctx.sleep(1_000).unwrap_err().kind(),
            CapabilityErrorKind::Unsupported
        );
        assert_eq!(
            ctx.report_connection_status(ID, ConnectionStatus::Unreachable, Some("dns-failed"))
                .unwrap_err()
                .kind(),
            CapabilityErrorKind::Unsupported
        );
    }
}
