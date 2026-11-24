//! The key label contract with the editor
//! (`apps/desktop/src/lib/key-labels.ts`): every fixture of
//! `key-labels.contract.json` is what Rust reads and writes, byte for
//! byte as JSON values, and the limits are the core's.

use deckpilot_core::appearance::labels::{
    self, CANVAS_UNITS, DEFAULT_COLOR_PLATE_OPACITY, DEFAULT_COVER_PLATE_OPACITY,
    DEFAULT_LABEL_LINES, DEFAULT_LABEL_MIN_POINTS, DEFAULT_LABEL_PADDING, DEFAULT_LABEL_POINTS,
    DEFAULT_LABEL_RADIUS, DEFAULT_PLATE_OPACITY, LABEL_LINES, LABEL_PERCENT, LABEL_SPACING,
    MAX_LABEL_CLUSTERS, MAX_LABEL_ID, MAX_LABEL_INSET, MAX_LABEL_OFFSET, MAX_LABEL_TEMPLATE,
    MAX_LABELS, PER_MILLE,
};
use deckpilot_core::{FaceLabel, KeyLabel, TEXT_SIZE_POINTS};
use deckpilot_render::LabelBox;
use serde_json::{Value, json};

fn contract() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../apps/desktop/src/lib/key-labels.contract.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Numbers as `f64`, so `5` and `5.0` compare equal.
fn normal(value: &Value) -> Value {
    match value {
        Value::Number(n) => json!(n.as_f64().unwrap()),
        Value::Array(items) => Value::Array(items.iter().map(normal).collect()),
        Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), normal(v))).collect(),
        other => other.clone(),
    }
}

fn round_trips<T: serde::Serialize + serde::de::DeserializeOwned>(fixture: &Value) -> T {
    let read: T =
        serde_json::from_value(fixture.clone()).unwrap_or_else(|e| panic!("{e}: {fixture}"));
    assert_eq!(
        normal(&serde_json::to_value(&read).unwrap()),
        normal(fixture),
        "written back differently"
    );
    read
}

#[test]
fn limits_and_defaults_are_the_cores() {
    let contract = contract();
    let pair = |(a, b): (u8, u8)| json!([a, b]);
    assert_eq!(
        contract["limits"],
        json!({
            "labels": MAX_LABELS,
            "template": MAX_LABEL_TEMPLATE,
            "clusters": MAX_LABEL_CLUSTERS,
            "id": MAX_LABEL_ID,
            "lines": pair(LABEL_LINES),
            "size": pair(TEXT_SIZE_POINTS),
            "padding": pair(LABEL_SPACING),
            "radius": pair(LABEL_SPACING),
            "percent": pair(LABEL_PERCENT),
            "offset": MAX_LABEL_OFFSET,
            "inset": MAX_LABEL_INSET,
            "perMille": PER_MILLE,
            "canvas": CANVAS_UNITS,
        })
    );
    assert_eq!(
        contract["defaults"],
        json!({
            "size": DEFAULT_LABEL_POINTS,
            "lines": DEFAULT_LABEL_LINES,
            "minSize": DEFAULT_LABEL_MIN_POINTS,
            "padding": DEFAULT_LABEL_PADDING,
            "radius": DEFAULT_LABEL_RADIUS,
            "plateOpacity": DEFAULT_PLATE_OPACITY,
            "coverPlateOpacity": DEFAULT_COVER_PLATE_OPACITY,
            "colorPlateOpacity": DEFAULT_COLOR_PLATE_OPACITY,
        })
    );
}

#[test]
fn labels_faces_and_boxes_are_written_as_the_editor_reads_them() {
    let contract = contract();
    let labels: Vec<KeyLabel> = contract["keyLabels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|fixture| {
            // Written back as read; saving normalizes a default written
            // out (`visible: always`) away.
            let read: KeyLabel = round_trips(fixture);
            if fixture["visible"] == "always" {
                assert_eq!(read.clone().normalized().unwrap().visible, None);
            }
            read
        })
        .collect();
    labels::check_labels(&labels).unwrap();
    for fixture in contract["invalidKeyLabels"].as_array().unwrap() {
        let refused = serde_json::from_value::<KeyLabel>(fixture.clone())
            .map_err(|e| e.to_string())
            .and_then(|label| labels::check_labels(&[label]));
        assert!(refused.is_err(), "{fixture}");
    }
    for fixture in contract["faceLabels"].as_array().unwrap() {
        round_trips::<FaceLabel>(fixture);
    }
    for fixture in contract["labelBoxes"].as_array().unwrap() {
        round_trips::<LabelBox>(fixture);
    }
}
