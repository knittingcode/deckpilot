use super::*;
use crate::KeyAppearance;

fn id(text: &str) -> LabelId {
    LabelId::new(text).unwrap()
}

/// A label with every field set to a value that is not its default.
pub(crate) fn every_field() -> KeyLabel {
    KeyLabel {
        id: id("a1b2c3d4"),
        text: "{{ shared.team.name }} · {project.key}".into(),
        enabled: Some(false),
        visible: Some(LabelVisibility::WhenResolved),
        look: KeyLabelLook {
            position: LabelPosition::Absolute {
                x: 250,
                y: 900,
                origin: Some(Spot::BottomLeft),
            },
            offset_x: Some(-3),
            offset_y: Some(4),
            width: None,
            max_width: Some(80),
            height: Some(30),
            max_height: None,
            size: Some(12),
            weight: Some(LabelWeight::Bold),
            align: Some(TextAlign::Right),
            color: Some(Color([1, 2, 3])),
            background: Some(LabelBackground::Color(Color([250, 200, 100]))),
            background_opacity: Some(75),
            padding: Some(4),
            radius: Some(0),
            layer: Some(LabelLayer::OverTitle),
            overflow: Some(LabelOverflow::Wrap),
            max_lines: Some(3),
            min_size: Some(7),
        },
    }
}

#[test]
fn ids_are_short_lowercase_and_copy() {
    assert_eq!(id("bottom-1").as_str(), "bottom-1");
    for bad in ["", "Upper", "with space", "x".repeat(17).as_str(), "ż"] {
        assert!(LabelId::new(bad).is_err(), "{bad}");
    }
    let a = id("a");
    let b = a;
    assert_eq!(a, b);
    assert_eq!(serde_json::to_value(a).unwrap(), "a");
    assert!(serde_json::from_value::<LabelId>(serde_json::json!("A")).is_err());
}

#[test]
fn a_label_round_trips_with_every_field() {
    let label = every_field();
    let json = serde_json::to_value(&label).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "id": "a1b2c3d4",
            "text": "{{ shared.team.name }} · {project.key}",
            "enabled": false,
            "visible": "whenResolved",
            "position": { "mode": "absolute", "x": 250, "y": 900, "origin": "bottomLeft" },
            "offsetX": -3, "offsetY": 4,
            "maxWidth": 80, "height": 30,
            "size": 12, "weight": "bold", "align": "right",
            "color": "#010203", "background": "#fac864", "backgroundOpacity": 75,
            "padding": 4, "radius": 0,
            "layer": "overTitle", "overflow": "wrap", "maxLines": 3, "minSize": 7
        })
    );
    assert_eq!(serde_json::from_value::<KeyLabel>(json).unwrap(), label);
    for position in [
        LabelPosition::Preset { at: Spot::TopLeft },
        LabelPosition::Anchored {
            to: Spot::Bottom,
            inset: Some(2),
        },
    ] {
        let json = serde_json::to_value(position).unwrap();
        assert_eq!(
            serde_json::from_value::<LabelPosition>(json).unwrap(),
            position
        );
    }
    for (background, text) in [
        (LabelBackground::Auto, "auto"),
        (LabelBackground::None, "none"),
    ] {
        assert_eq!(serde_json::to_value(background).unwrap(), text);
        assert_eq!(
            serde_json::from_value::<LabelBackground>(text.into()).unwrap(),
            background
        );
    }
    assert!(serde_json::from_value::<LabelBackground>("red".into()).is_err());
}

#[test]
fn defaults_are_not_written() {
    let label = KeyLabel::new(id("x"), "Hi");
    assert_eq!(
        serde_json::to_value(&label).unwrap(),
        serde_json::json!({ "id": "x", "text": "Hi" })
    );
    // A minimal label reads with every default.
    let read: KeyLabel =
        serde_json::from_value(serde_json::json!({ "id": "x", "text": "Hi" })).unwrap();
    assert_eq!(read, label);
    assert_eq!(
        read.look.position,
        LabelPosition::Preset { at: Spot::Bottom }
    );
    assert_eq!(read.look.points(), DEFAULT_LABEL_POINTS);
    assert_eq!(read.look.background(), LabelBackground::Auto);
    assert_eq!(read.look.background_opacity(), DEFAULT_PLATE_OPACITY);
    assert_eq!(read.look.layer(), LabelLayer::OverIcon);
    assert!(read.is_enabled());
    // Fields a later version adds are ignored, not refused.
    let later: KeyLabel = serde_json::from_value(
        serde_json::json!({ "id": "x", "text": "Hi", "shadow": { "blur": 2 } }),
    )
    .unwrap();
    assert_eq!(later, label);
}

#[test]
fn normalizing_drops_defaults_and_empty_labels() {
    let mut label = KeyLabel::new(id("x"), "Hi");
    label.enabled = Some(true);
    label.visible = Some(LabelVisibility::Always);
    label.look.size = Some(DEFAULT_LABEL_POINTS);
    label.look.layer = Some(LabelLayer::OverIcon);
    label.look.background = Some(LabelBackground::Auto);
    label.look.background_opacity = Some(DEFAULT_PLATE_OPACITY);
    label.look.offset_x = Some(0);
    label.look.position = LabelPosition::Anchored {
        to: Spot::Top,
        inset: Some(0),
    };
    let normalized = label.normalized().unwrap();
    assert_eq!(normalized.enabled, None);
    assert_eq!(normalized.visible, None);
    assert_eq!(
        normalized.look,
        KeyLabelLook {
            position: LabelPosition::Anchored {
                to: Spot::Top,
                inset: None
            },
            ..KeyLabelLook::default()
        }
    );
    // An explicit colour plate keeps an opacity below 100, drops 100.
    let mut colored = KeyLabel::new(id("y"), "Hi");
    colored.look.background = Some(LabelBackground::Color(Color([0, 0, 0])));
    colored.look.background_opacity = Some(100);
    assert_eq!(colored.normalized().unwrap().look.background_opacity, None);
    assert!(KeyLabel::new(id("z"), "  ").normalized().is_none());

    let look = KeyAppearance {
        labels: vec![KeyLabel::new(id("a"), " "), KeyLabel::new(id("b"), "B")],
        ..KeyAppearance::default()
    };
    let normalized = look.normalized().unwrap();
    assert_eq!(normalized.labels.len(), 1);
    // Only empty labels: the look is the default and is not stored.
    let empty = KeyAppearance {
        labels: vec![KeyLabel::new(id("a"), "")],
        ..KeyAppearance::default()
    };
    assert_eq!(empty.normalized(), None);
}

#[test]
fn looks_without_labels_serialize_as_before() {
    let look = KeyAppearance {
        text_color: Some(Color([1, 2, 3])),
        ..KeyAppearance::default()
    };
    assert_eq!(
        serde_json::to_string(&look).unwrap(),
        r##"{"textColor":"#010203"}"##
    );
    let read: KeyAppearance = serde_json::from_str(r##"{"textColor":"#010203"}"##).unwrap();
    assert_eq!(read.labels, []);
    assert_eq!(read, look);
}

/// A 0.13 look, as an older agent reads it: labels are ignored, not
/// refused.
#[test]
fn an_older_reader_ignores_labels() {
    #[derive(serde::Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    #[allow(dead_code)]
    struct OlderLook {
        text_color: Option<Color>,
        show_title: Option<bool>,
    }
    let look = KeyAppearance {
        text_color: Some(Color([9, 9, 9])),
        labels: vec![every_field()],
        ..KeyAppearance::default()
    };
    let json = serde_json::to_string(&look).unwrap();
    let older: OlderLook = serde_json::from_str(&json).unwrap();
    assert_eq!(older.text_color, Some(Color([9, 9, 9])));
}

#[test]
fn limits_are_enforced() {
    let label = |n: usize| KeyLabel::new(id(&format!("l{n}")), "x");
    let look = |labels: Vec<KeyLabel>| KeyAppearance {
        labels,
        ..KeyAppearance::default()
    };
    assert!(look((0..MAX_LABELS).map(label).collect()).check().is_ok());
    let error = look((0..=MAX_LABELS).map(label).collect())
        .check()
        .unwrap_err();
    assert!(error.contains("at most 8"), "{error}");
    assert!(look(vec![label(1), label(1)]).check().is_err());
    let long = KeyLabel::new(id("x"), "x".repeat(MAX_LABEL_TEMPLATE + 1));
    assert!(look(vec![long]).check().is_err());
    let fits = KeyLabel::new(id("x"), "ż".repeat(MAX_LABEL_TEMPLATE));
    assert!(look(vec![fits]).check().is_ok());

    let with = |f: fn(&mut KeyLabelLook)| {
        let mut label = KeyLabel::new(id("x"), "x");
        f(&mut label.look);
        look(vec![label]).check()
    };
    assert!(with(|l| l.position = LabelPosition::Preset { at: Spot::Left }).is_err());
    assert!(with(|l| l.position = LabelPosition::Preset { at: Spot::Right }).is_err());
    assert!(with(|l| l.position = LabelPosition::Preset { at: Spot::Center }).is_ok());
    assert!(
        with(|l| l.position = LabelPosition::Anchored {
            to: Spot::Left,
            inset: None
        })
        .is_ok()
    );
    assert!(
        with(|l| l.position = LabelPosition::Anchored {
            to: Spot::Top,
            inset: Some(37)
        })
        .is_err()
    );
    assert!(
        with(|l| l.position = LabelPosition::Absolute {
            x: 1001,
            y: 0,
            origin: None
        })
        .is_err()
    );
    assert!(with(|l| l.offset_x = Some(73)).is_err());
    assert!(with(|l| l.offset_y = Some(-72)).is_ok());
    assert!(with(|l| l.width = Some(4)).is_err());
    assert!(with(|l| l.max_height = Some(101)).is_err());
    assert!(
        with(|l| {
            l.width = Some(50);
            l.max_width = Some(60);
        })
        .is_err()
    );
    assert!(
        with(|l| {
            l.height = Some(50);
            l.max_height = Some(60);
        })
        .is_err()
    );
    assert!(with(|l| l.size = Some(5)).is_err());
    assert!(with(|l| l.size = Some(32)).is_ok());
    assert!(with(|l| l.size = Some(33)).is_err());
    assert!(with(|l| l.max_lines = Some(0)).is_err());
    assert!(with(|l| l.max_lines = Some(5)).is_err());
    assert!(with(|l| l.min_size = Some(11)).is_err(), "above the size");
    assert!(with(|l| l.padding = Some(13)).is_err());
    assert!(with(|l| l.radius = Some(12)).is_ok());
    assert!(with(|l| l.background_opacity = Some(101)).is_err());
}

#[test]
fn secret_and_run_references_are_refused_unknown_ones_are_not() {
    let look = |text: &str| KeyAppearance {
        labels: vec![KeyLabel::new(id("x"), text)],
        ..KeyAppearance::default()
    };
    for ok in [
        "Plain",
        "{project.name}",
        "{{ system.project.current.key }}",
        "{{ shared.team.name }}",
        "{page.number}/{page.count}",
        // A module's variable that is not here (its module disabled).
        "{{ projects.current.key }}",
        "{{ not a reference }}",
    ] {
        assert!(look(ok).check().is_ok(), "{ok}");
    }
    for refused in [
        "{{ steps.fetch.output.token }}",
        "{{ workflow.execution.id }}",
    ] {
        let error = look(refused).check().unwrap_err();
        assert!(error.contains("never shows"), "{refused}: {error}");
    }
    let registry = crate::variables::VariableRegistry::new();
    assert_eq!(
        template_issues("{{ not a reference }} {{ steps.a.output.b }}", &registry),
        [
            TemplateIssue::Invalid {
                text: "not a reference".into()
            },
            TemplateIssue::NotDisplayable {
                reference: "steps.a.output.b".into()
            }
        ]
    );
}

#[test]
fn spots_and_positions() {
    assert!(Spot::TopLeft.is_corner());
    assert!(!Spot::Top.is_corner());
    assert!(!Spot::Center.is_corner());
    assert_eq!(Spot::BottomRight.grid(), (2, 2));
    assert_eq!(
        LabelPosition::default(),
        LabelPosition::Preset { at: Spot::Bottom }
    );
}

/// With the agent's variables (a plugin's declarations): a reference the
/// plugin declares sensitive, or an identifier, is refused; one of a plugin
/// that is not here is not (it shows the marker until its plugin declares
/// it display-safe).
#[test]
fn the_agents_variables_refuse_sensitive_and_identifier_references() {
    use crate::variables::{
        DeclaredScope, Resolution, VariableDeclaration, VariableEnvironment, VariableRegistry,
        VariableSource, VariableType,
    };
    struct Nothing;
    impl VariableSource for Nothing {
        fn resolve(&self, _: &VariableDeclaration, _: &VariableEnvironment<'_>) -> Resolution {
            Resolution::UnavailableUntilRuntime
        }
    }
    let mut registry = VariableRegistry::new();
    registry
        .register_module(
            &crate::ModuleId::new("acme.api").unwrap(),
            &[
                VariableDeclaration {
                    sensitive: true,
                    ..VariableDeclaration::new(
                        "plugins.acme.api.token",
                        "Token",
                        VariableType::String,
                        DeclaredScope::Machine,
                    )
                },
                VariableDeclaration {
                    identifier: true,
                    ..VariableDeclaration::new(
                        "plugins.acme.api.account",
                        "Account",
                        VariableType::String,
                        DeclaredScope::Machine,
                    )
                },
                VariableDeclaration::new(
                    "plugins.acme.api.city",
                    "City",
                    VariableType::String,
                    DeclaredScope::Machine,
                ),
            ],
            Some(std::sync::Arc::new(Nothing)),
        )
        .unwrap();
    let labels = |text: &str| vec![KeyLabel::new(id("x"), text)];
    for refused in [
        "{{ plugins.acme.api.token }}",
        "Account {{ plugins.acme.api.account }}",
    ] {
        let error = check_labels_with(&labels(refused), &registry).unwrap_err();
        assert!(error.contains("never shows"), "{refused}: {error}");
        // The core alone does not know the plugin: allowed there.
        assert!(check_labels(&labels(refused)).is_ok());
    }
    assert!(check_labels_with(&labels("{{ plugins.acme.api.city }}"), &registry).is_ok());
    assert!(check_labels_with(&labels("{{ plugins.other.thing.name }}"), &registry).is_ok());
}
