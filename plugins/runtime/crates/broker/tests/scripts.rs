//! The Scripts bridge (plugin API 1.13): only `deckpilot.scripts`, only
//! with the `scripts` permission, only within its `fs`, `process` and
//! `shell` sub-ceilings, revoked at once, on a rate of its own.

use std::sync::Arc;

use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_abi::capability::RunTrigger;
use deckpilot_plugin_broker::*;

fn broker() -> (Broker, Arc<MemoryAudit>) {
    let audit = Arc::new(MemoryAudit::new(10_000));
    let broker = Broker::new(
        audit.clone(),
        Arc::new(ManualClock::default()),
        RateLimits::default(),
    );
    (broker, audit)
}

fn grant(broker: &Broker, plugin: &PluginId, perms: &str) {
    broker
        .set_grants(plugin, &ApprovedPermissions::from_json(perms).unwrap())
        .unwrap();
}

fn req(json: &str) -> CapabilityRequest {
    serde_json::from_str(json).unwrap()
}

fn poll() -> CapabilityRequest {
    req(r#"{"op":"script.poll","args":{}}"#)
}
fn stat() -> CapabilityRequest {
    req(r#"{"op":"fs.stat","args":{"path":{"repository":"cms","relative":""}}}"#)
}
fn run() -> CapabilityRequest {
    req(r#"{"op":"process.run","args":{"command":"git","args":["status"],"env":{}}}"#)
}
fn shell() -> CapabilityRequest {
    req(r#"{"op":"shell.available","args":{}}"#)
}
fn list() -> CapabilityRequest {
    req(r#"{"op":"project.list","args":{}}"#)
}

fn key() -> RunContext {
    RunContext::direct(RunTrigger::Key)
}

fn scope(d: &Decision) -> Option<ScriptScope> {
    match d {
        Decision::Allow(Allowed::Script(call)) => Some(call.scope),
        _ => None,
    }
}

const ALL: &str = r#"[{"kind":"scripts","fs":true,"process":true,"shell":true}]"#;

#[test]
fn only_the_scripts_plugin_is_ever_allowed() {
    let (broker, _) = broker();
    let other = PluginId::new("acme.scripts").unwrap();
    grant(&broker, &other, ALL);
    let s = broker.session(other);
    for r in [poll(), stat(), run(), shell()] {
        assert_eq!(
            s.authorize_in(&r, Some(&key())).denial(),
            Some(DenyReason::NotGranted),
            "{}",
            r.op()
        );
    }
}

#[test]
fn the_ceiling_decides_each_sub_scope() {
    let (broker, _) = broker();
    let scripts = PluginId::new(SCRIPTS_PLUGIN).unwrap();
    grant(&broker, &scripts, r#"[{"kind":"scripts","fs":true}]"#);
    let s = broker.session(scripts.clone());
    assert_eq!(scope(&s.authorize(&poll())), Some(ScriptScope::Run));
    assert_eq!(scope(&s.authorize(&stat())), Some(ScriptScope::Fs));
    assert_eq!(scope(&s.authorize(&list())), Some(ScriptScope::Projects));
    assert_eq!(s.authorize(&run()).denial(), Some(DenyReason::NotGranted));
    assert_eq!(s.authorize(&shell()).denial(), Some(DenyReason::NotGranted));

    grant(&broker, &scripts, ALL);
    assert_eq!(scope(&s.authorize(&run())), Some(ScriptScope::Process));
    assert_eq!(scope(&s.authorize(&shell())), Some(ScriptScope::Shell));
    let Decision::Allow(Allowed::Script(call)) = s.authorize(&run()) else {
        panic!()
    };
    assert_eq!(call.request, run());
    assert!(call.ceiling.process && call.ceiling.fs && call.ceiling.shell);
}

#[test]
fn without_the_permission_or_after_revocation_every_op_is_refused() {
    let (broker, _) = broker();
    let scripts = PluginId::new(SCRIPTS_PLUGIN).unwrap();
    grant(&broker, &scripts, r#"[{"kind":"notifications"}]"#);
    let s = broker.session(scripts.clone());
    for r in [poll(), stat(), run(), shell()] {
        assert_eq!(s.authorize(&r).denial(), Some(DenyReason::NotGranted));
    }
    grant(&broker, &scripts, ALL);
    assert!(s.authorize(&run()).is_allowed());
    // Revoked: the next call is refused.
    grant(&broker, &scripts, "[]");
    assert_eq!(s.authorize(&run()).denial(), Some(DenyReason::NotGranted));
    broker.remove_plugin(&scripts);
    assert_eq!(
        s.authorize(&poll()).denial(),
        Some(DenyReason::UnknownPlugin)
    );
}

/// `project.list` stays plugin API 1.9's for a plugin with
/// `projectContext`, and is the bridge's for one with only `scripts`.
#[test]
fn project_list_belongs_to_the_grant_the_plugin_holds() {
    let (broker, _) = broker();
    let scripts = PluginId::new(SCRIPTS_PLUGIN).unwrap();
    grant(&broker, &scripts, ALL);
    let s = broker.session(scripts.clone());
    assert_eq!(scope(&s.authorize(&list())), Some(ScriptScope::Projects));
    let projects = PluginId::new("deckpilot.projects").unwrap();
    grant(&broker, &projects, r#"[{"kind":"projectContext"}]"#);
    assert_eq!(
        broker.session(projects).authorize(&list()),
        Decision::Allow(Allowed::ProjectList)
    );
}

/// A run polls and logs far more often than the shared 200 per second; the
/// bridge has its own rate, and a flood is still throttled.
#[test]
fn the_bridge_has_a_rate_of_its_own() {
    let (broker, _) = broker();
    let scripts = PluginId::new(SCRIPTS_PLUGIN).unwrap();
    grant(&broker, &scripts, ALL);
    let s = broker.session(scripts);
    for i in 0..2_000 {
        assert!(s.authorize(&poll()).is_allowed(), "{i}");
    }
    assert_eq!(s.authorize(&poll()).denial(), Some(DenyReason::RateLimited));
}

#[test]
fn a_duplicate_or_never_grantable_record_fails_the_derivation() {
    assert!(
        Grants::derive(
            &ApprovedPermissions::from_json(
                r#"[{"kind":"scripts","fs":true},{"kind":"scripts","process":true}]"#
            )
            .unwrap()
        )
        .is_err()
    );
    for kind in ["process", "shell", "fs", "filesystem"] {
        let json = format!(r#"[{{"kind":"{kind}"}}]"#);
        assert!(ApprovedPermissions::from_json(&json).is_err(), "{kind}");
    }
    assert!(ApprovedPermissions::from_json(r#"[{"kind":"scripts","exec":true}]"#).is_err());
}

#[test]
fn every_decision_is_audited() {
    let (broker, audit) = broker();
    let scripts = PluginId::new(SCRIPTS_PLUGIN).unwrap();
    grant(&broker, &scripts, r#"[{"kind":"scripts","fs":true}]"#);
    let s = broker.session(scripts);
    s.authorize(&stat());
    s.authorize(&run());
    let records = audit.records();
    let ops: Vec<_> = records.iter().map(|r| (r.op, r.decision.clone())).collect();
    assert_eq!(
        ops,
        vec![
            ("fs.stat", AuditDecision::Allowed),
            ("process.run", AuditDecision::Denied(DenyReason::NotGranted)),
        ]
    );
}
