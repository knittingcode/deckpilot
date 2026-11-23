use super::*;
use crate::face::render;
use deckpilot_core::{Color, FaceStyle, KeyLabelLook};

fn label(id: &str, text: &str, spec: KeyLabelLook) -> FaceLabel {
    FaceLabel {
        id: LabelId::new(id).unwrap(),
        text: text.into(),
        spec,
        unresolved: false,
        markers: Vec::new(),
    }
}

fn face(labels: Vec<FaceLabel>) -> KeyFace {
    KeyFace::new(FaceStyle::Action, "Mail").with_labels(labels)
}

fn boxes(labels: Vec<FaceLabel>, width: u32, height: u32) -> Vec<LabelBox> {
    label_layout(&face(labels), width, height, &Icons::default())
}

fn one(spec: KeyLabelLook) -> LabelBox {
    boxes(vec![label("a", "Hi", spec)], 72, 72).remove(0)
}

fn preset(at: Spot) -> KeyLabelLook {
    KeyLabelLook {
        position: LabelPosition::Preset { at },
        ..KeyLabelLook::default()
    }
}

#[test]
fn presets_sit_in_the_text_area() {
    let top = one(preset(Spot::Top));
    assert!((top.y - 6.0).abs() < 0.01, "{top:?}");
    assert!((top.x - 5.0).abs() < 0.01 && (top.width - 62.0).abs() < 0.01);
    let bottom = one(preset(Spot::Bottom));
    assert!((bottom.y + bottom.height - 66.0).abs() < 0.01);
    let center = one(preset(Spot::Center));
    assert!((center.y + center.height / 2.0 - 36.0).abs() < 0.01);
    // Corners hug the text.
    let corner = one(preset(Spot::TopRight));
    assert!((corner.x + corner.width - 67.0).abs() < 0.01);
    assert!(corner.width < 30.0, "{corner:?}");
    let left = one(preset(Spot::BottomLeft));
    assert!((left.x - 5.0).abs() < 0.01);
    assert_eq!(
        top.canvas,
        Canvas {
            width: 72.0,
            height: 72.0
        }
    );
}

#[test]
fn anchored_labels_use_the_whole_key() {
    let ribbon = one(KeyLabelLook {
        position: LabelPosition::Anchored {
            to: Spot::Bottom,
            inset: None,
        },
        ..KeyLabelLook::default()
    });
    assert_eq!((ribbon.x, ribbon.width), (0.0, 72.0));
    assert!((ribbon.y + ribbon.height - 72.0).abs() < 0.01);
    // About 16 units: a 10 pt line and 2 units of padding.
    assert!((12.0..17.0).contains(&ribbon.height), "{ribbon:?}");
    let inset = one(KeyLabelLook {
        position: LabelPosition::Anchored {
            to: Spot::TopLeft,
            inset: Some(3),
        },
        ..KeyLabelLook::default()
    });
    assert_eq!((inset.x, inset.y), (3.0, 3.0));
}

#[test]
fn absolute_positions_offsets_and_clamping() {
    let centred = one(KeyLabelLook {
        position: LabelPosition::Absolute {
            x: 500,
            y: 500,
            origin: None,
        },
        ..KeyLabelLook::default()
    });
    assert!((centred.x + centred.width / 2.0 - 36.0).abs() < 0.01);
    let moved = one(KeyLabelLook {
        position: LabelPosition::Absolute {
            x: 500,
            y: 500,
            origin: None,
        },
        offset_x: Some(4),
        offset_y: Some(-2),
        ..KeyLabelLook::default()
    });
    assert!((moved.x - centred.x - 4.0).abs() < 0.01);
    assert!((moved.y - centred.y + 2.0).abs() < 0.01);
    // Past the edge: moved back in, not cut.
    let out = one(KeyLabelLook {
        position: LabelPosition::Absolute {
            x: 1000,
            y: 1000,
            origin: Some(Spot::TopLeft),
        },
        offset_x: Some(72),
        ..KeyLabelLook::default()
    });
    assert!((out.x + out.width - 72.0).abs() < 0.01);
    assert!((out.y + out.height - 72.0).abs() < 0.01);
    // A touch zone: per mille of a 144 x 72 canvas.
    let zone = boxes(
        vec![label(
            "a",
            "Hi",
            KeyLabelLook {
                position: LabelPosition::Absolute {
                    x: 250,
                    y: 500,
                    origin: None,
                },
                ..KeyLabelLook::default()
            },
        )],
        200,
        100,
    )
    .remove(0);
    assert_eq!(
        zone.canvas,
        Canvas {
            width: 144.0,
            height: 72.0
        }
    );
    assert!((zone.x + zone.width / 2.0 - 36.0).abs() < 0.01);
}

#[test]
fn widths_and_heights() {
    let fixed = one(KeyLabelLook {
        width: Some(50),
        height: Some(25),
        ..preset(Spot::TopLeft)
    });
    assert_eq!((fixed.width, fixed.height), (36.0, 18.0));
    let capped = boxes(
        vec![label(
            "a",
            "A rather long label text",
            KeyLabelLook {
                max_width: Some(30),
                ..preset(Spot::TopLeft)
            },
        )],
        72,
        72,
    )
    .remove(0);
    assert!((capped.width - 21.6).abs() < 0.01);
    assert!(capped.truncated);
}

#[test]
fn overflow_modes() {
    let long = "Zażółć gęślą jaźń źdźbło łąki";
    let with = |overflow: LabelOverflow, extra: KeyLabelLook| {
        boxes(
            vec![label(
                "a",
                long,
                KeyLabelLook {
                    overflow: Some(overflow),
                    ..extra
                },
            )],
            72,
            72,
        )
        .remove(0)
    };
    let ellipsis = with(LabelOverflow::Ellipsis, preset(Spot::Top));
    assert!(ellipsis.truncated && ellipsis.lines == 1);
    let clip = with(LabelOverflow::Clip, preset(Spot::Top));
    assert!(clip.truncated && clip.lines == 1);
    let wrap = with(LabelOverflow::Wrap, preset(Spot::Top));
    assert_eq!(wrap.lines, 2);
    let four = with(
        LabelOverflow::Wrap,
        KeyLabelLook {
            max_lines: Some(4),
            ..preset(Spot::Top)
        },
    );
    assert!(four.lines >= 3, "{four:?}");
    let short = with(
        LabelOverflow::Wrap,
        KeyLabelLook {
            max_lines: Some(4),
            max_height: Some(30),
            ..preset(Spot::Top)
        },
    );
    assert!(short.lines < four.lines && short.truncated, "{short:?}");
    assert!(short.height <= 21.6 + 0.01);
    let smallest = with(LabelOverflow::Shrink, preset(Spot::Top));
    assert!((smallest.font_points - 6.0).abs() < 0.01 && smallest.truncated);
    let shrunk = boxes(
        vec![label(
            "a",
            "Kraków",
            KeyLabelLook {
                overflow: Some(LabelOverflow::Shrink),
                size: Some(24),
                ..preset(Spot::Top)
            },
        )],
        72,
        72,
    )
    .remove(0);
    assert!(shrunk.font_points < 24.0 && shrunk.font_points > 6.0 && !shrunk.truncated);
}

#[test]
fn long_texts_are_cut_to_120_clusters() {
    let text = "ą".repeat(500);
    assert_eq!(capped(&text).chars().count(), MAX_LABEL_CLUSTERS);
    assert_eq!(capped("short"), "short");
}

#[test]
fn automatic_plates_follow_the_layers_and_contrast() {
    // White on black: no plate.
    assert!(!one(preset(Spot::Top)).plate);
    // White on a white key: a plate, and the text reads on it.
    let mut light = face(vec![label("a", "Hi", preset(Spot::Top))]);
    light.appearance = Some(KeyAppearance {
        background: Some(deckpilot_core::Background::Plain(
            deckpilot_core::KeyBackground::Color(Color([255, 255, 255])),
        )),
        ..KeyAppearance::default()
    });
    let placed = place_all(
        &light,
        72,
        72,
        &Icons::default(),
        key_colors(&light, &deckpilot_core::DefaultAppearance::BUILTIN),
    );
    assert!(placed[0].report.plate);
    assert_eq!(placed[0].plate, Some(([0, 0, 0], 0.6)));
    // Over a series: a plate.
    let mut charted = face(vec![label("a", "Hi", preset(Spot::Top))]);
    charted.series = Some(vec![1, 2, 3]);
    assert!(label_layout(&charted, 72, 72, &Icons::default())[0].plate);
    // `none` never; a colour always.
    assert!(
        !one(KeyLabelLook {
            background: Some(LabelBackground::None),
            ..preset(Spot::Top)
        })
        .plate
    );
    assert!(
        one(KeyLabelLook {
            background: Some(LabelBackground::Color(Color([0, 0, 0]))),
            ..preset(Spot::Top)
        })
        .plate
    );
    assert!(contrast([255, 255, 255], [0, 0, 0]) > 20.0);
}

#[test]
fn layers_paint_in_order() {
    // A red plate behind the title, a blue one over it, at the same place.
    let plate = |id: &str, color: [u8; 3], layer: LabelLayer| {
        label(
            id,
            " ",
            KeyLabelLook {
                position: LabelPosition::Anchored {
                    to: Spot::Center,
                    inset: None,
                },
                width: Some(100),
                height: Some(100),
                background: Some(LabelBackground::Color(Color(color))),
                layer: Some(layer),
                ..KeyLabelLook::default()
            },
        )
    };
    let red = [200, 0, 0];
    let blue = [0, 0, 200];
    let over = face(vec![
        plate("blue", blue, LabelLayer::OverTitle),
        plate("red", red, LabelLayer::OverIcon),
    ]);
    // The title is under the blue plate: the key is all blue.
    let frame = render(&over, 72, 72);
    assert_eq!(frame.pixel(36, 60), Some(blue));
    assert_eq!(frame.pixel(1, 1), Some(blue));
    // Within a layer the later label is on top.
    let order = face(vec![
        plate("red", red, LabelLayer::OverIcon),
        plate("blue", blue, LabelLayer::OverIcon),
    ]);
    // The title is drawn over both.
    let frame = render(&order, 72, 72);
    assert_eq!(frame.pixel(1, 1), Some(blue));
    // State marks stay on top.
    let mut marked = face(vec![plate("red", red, LabelLayer::OverTitle)]);
    marked.progress = Some(100);
    let frame = render(&marked, 72, 72);
    assert_ne!(frame.pixel(36, 71), Some(red));
    assert_eq!(frame.pixel(36, 36), Some(red));
}

#[test]
fn the_title_report_is_reported_per_label() {
    let over = one(preset(Spot::Center));
    assert!(
        over.overlaps_title,
        "the centre preset meets a middle title"
    );
    let away = one(preset(Spot::Top));
    assert!(!away.overlaps_title);
}

#[test]
fn empty_faces_and_faces_without_labels_have_no_boxes() {
    assert_eq!(boxes(Vec::new(), 72, 72), []);
    let empty = KeyFace::empty().with_labels(vec![label("a", "x", KeyLabelLook::default())]);
    assert_eq!(label_layout(&empty, 72, 72, &Icons::default()), []);
}
