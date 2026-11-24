//! T-C1: capability discovery. A capability a provider leaves out is
//! unsupported, never assumed; a set always names every capability.

use super::*;

#[test]
fn a_missing_capability_is_unsupported_and_reported() {
    let (set, missing) = CapabilitySet::from_reported([
        (Capability::DetectMeeting, Support::Inferred),
        (Capability::MicReadState, Support::Inferred),
    ]);
    assert_eq!(set.get(Capability::DetectMeeting), Support::Inferred);
    assert_eq!(
        set.get(Capability::Answer),
        Support::Unsupported {
            reason: UnsupportedReason::ProviderHasNoApi
        }
    );
    assert_eq!(missing.len(), Capability::ALL.len() - 2);
    assert!(!missing.contains(&Capability::DetectMeeting));
    assert_eq!(set.iter().count(), Capability::ALL.len());
    assert!(!set.controls_anything());
}

#[test]
fn a_duplicate_never_widens_support() {
    let (set, missing) = CapabilitySet::from_reported(
        Capability::ALL
            .into_iter()
            .map(|c| (c, Support::Supported))
            .chain([(Capability::End, Support::ReadOnly)]),
    );
    assert_eq!(missing, []);
    assert_eq!(set.get(Capability::End), Support::ReadOnly);
    let (set, _) = CapabilitySet::from_reported([
        (Capability::End, Support::ReadOnly),
        (Capability::End, Support::Supported),
    ]);
    assert_eq!(set.get(Capability::End), Support::ReadOnly);
}

#[test]
fn only_supported_and_conditional_allow_commands() {
    let conditional = Support::Conditional {
        when: Condition::HostAllows,
    };
    assert!(Support::Supported.allows_command());
    assert!(conditional.allows_command());
    for s in [
        Support::ReadOnly,
        Support::Inferred,
        Support::Unsupported {
            reason: UnsupportedReason::ToggleOnly,
        },
    ] {
        assert!(!s.allows_command(), "{s:?}");
    }
    assert!(Support::Inferred.allows_observation());
    assert!(
        !Support::Unsupported {
            reason: UnsupportedReason::NeedsPairing
        }
        .allows_observation()
    );
}

#[test]
fn without_control_keeps_observation() {
    let (set, _) =
        CapabilitySet::from_reported(Capability::ALL.into_iter().map(|c| (c, Support::Supported)));
    assert!(set.controls_anything());
    let offline = set.without_control(UnsupportedReason::ConsentNotGiven);
    assert!(!offline.controls_anything());
    assert_eq!(offline.get(Capability::DetectMeeting), Support::Supported);
    assert_eq!(
        offline.get(Capability::MicSet),
        Support::Unsupported {
            reason: UnsupportedReason::ConsentNotGiven
        }
    );
}

#[test]
fn a_serialized_set_must_name_every_capability() {
    let (set, _) = CapabilitySet::from_reported([(Capability::DetectMeeting, Support::Inferred)]);
    let json = serde_json::to_value(&set).unwrap();
    assert_eq!(json["detectMeeting"]["kind"], "inferred");
    assert_eq!(json["answer"]["kind"], "unsupported");
    assert_eq!(json["answer"]["reason"], "providerHasNoApi");
    let back: CapabilitySet = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back, set);
    let mut partial = json;
    partial.as_object_mut().unwrap().remove("answer");
    assert!(serde_json::from_value::<CapabilitySet>(partial).is_err());
}

#[test]
fn ids_follow_their_rules() {
    for ok in [
        ProviderId::MOCK,
        ProviderId::OS_SIGNALS,
        ProviderId::GRAPH_PRESENCE,
        "zoom2",
    ] {
        assert_eq!(ProviderId::new(ok).unwrap().as_str(), ok);
    }
    for bad in ["", "Teams", "2zoom", "a--b", "a-", "a b", &"a".repeat(33)] {
        assert!(ProviderId::new(bad).is_err(), "{bad}");
    }
    assert!(serde_json::from_str::<ProviderId>("\"Mock\"").is_err());
    assert!(CallKey::new("").is_err());
    assert!(CallKey::new("a\nb").is_err());
    assert!(CallKey::new("x".repeat(CallKey::MAX_LEN + 1)).is_err());
}

#[test]
fn private_values_and_call_keys_never_print() {
    let title = Private::new("Board meeting: layoffs".to_owned());
    assert_eq!(format!("{title:?}"), "Private(<redacted>)");
    assert_eq!(title.reveal(), "Board meeting: layoffs");
    let key = CallKey::new("19:meeting_abc@thread.v2").unwrap();
    let printed = format!("{key:?}");
    assert!(!printed.contains("meeting_abc"), "{printed}");
    let observation = CallObservation {
        key,
        kind: CallKind::Meeting,
        phase: ObservedPhase::Connected,
        direction: None,
        mic_muted: Tri::Unknown,
        camera_on: Tri::Known(true),
        title: Some(title),
        source: Evidence::AppApi,
        observed_at: Instant::ZERO,
    };
    let printed = format!("{observation:?}");
    assert!(
        !printed.contains("layoffs") && !printed.contains("meeting_abc"),
        "{printed}"
    );
}

#[test]
fn every_action_needs_a_control_capability() {
    for (_, action) in crate::ops::CONTROL_ACTIONS {
        assert!(action.capability().is_control(), "{action:?}");
    }
    let controls: Vec<Capability> = Capability::ALL
        .into_iter()
        .filter(|c| c.is_control())
        .collect();
    assert_eq!(controls.len(), 8);
}

#[test]
fn reasons_have_stable_codes() {
    let all = [
        UnsupportedReason::ProviderHasNoApi,
        UnsupportedReason::PlatformNotSupported,
        UnsupportedReason::NotInMeeting,
        UnsupportedReason::NoPermissionInMeeting,
        UnsupportedReason::ToggleOnly,
        UnsupportedReason::NeedsPairing,
        UnsupportedReason::NeedsOsPermission,
        UnsupportedReason::ConsentNotGiven,
    ];
    let mut codes: Vec<&str> = all.iter().map(|r| r.code()).collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), all.len());
}

/// A provider that implements the contract with nothing but detection, as
/// a detection-only adapter does: every control is unsupported with a
/// reason, and a command is refused before anything is sent.
struct DetectionOnly;

impl CallProvider for DetectionOnly {
    fn id(&self) -> ProviderId {
        ProviderId::new(ProviderId::OS_SIGNALS).unwrap()
    }

    fn discover(&mut self, _now: Instant) -> Discovery {
        let (capabilities, _) = CapabilitySet::from_reported([
            (Capability::DetectMeeting, Support::Inferred),
            (Capability::MicReadState, Support::Inferred),
            (Capability::CameraReadState, Support::Inferred),
        ]);
        Discovery {
            availability: Availability::Available,
            capabilities,
            app_version: None,
        }
    }

    fn connect(&mut self, cx: &ProviderCx) -> Result<(), ProviderError> {
        if cx.consent.observe {
            Ok(())
        } else {
            Err(ProviderError::ConsentNotGiven)
        }
    }

    fn disconnect(&mut self) {}

    fn poll(&mut self, _now: Instant) -> Vec<ProviderEvent> {
        Vec::new()
    }

    fn command(
        &mut self,
        _command: CallCommand,
        _token: CommandToken,
    ) -> Result<(), CommandRefusal> {
        Err(CommandRefusal::Unsupported(
            UnsupportedReason::ProviderHasNoApi,
        ))
    }
}

#[test]
fn a_detection_only_provider_controls_nothing() {
    let mut provider = DetectionOnly;
    let discovery = provider.discover(Instant::ZERO);
    assert!(!discovery.capabilities.controls_anything());
    for capability in Capability::ALL.into_iter().filter(|c| c.is_control()) {
        assert_eq!(
            discovery.capabilities.get(capability),
            Support::Unsupported {
                reason: UnsupportedReason::ProviderHasNoApi
            }
        );
    }
    let cx = ProviderCx {
        consent: ConsentState::default(),
        session: ProviderSession(1),
    };
    assert_eq!(provider.connect(&cx), Err(ProviderError::ConsentNotGiven));
    let refused = provider.command(
        CallCommand {
            key: CallKey::new("k").unwrap(),
            action: CallAction::End,
        },
        CommandToken {
            session: ProviderSession(1),
            serial: 1,
        },
    );
    assert_eq!(
        refused,
        Err(CommandRefusal::Unsupported(
            UnsupportedReason::ProviderHasNoApi
        ))
    );
}
