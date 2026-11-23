//! What key labels cost to draw (`docs/key-text.md`, "Labels"; design §5):
//!
//! ```text
//! cargo test --release -p deckpilot-render --test label_cost -- --nocapture
//! ```
//!
//! In a release build it asserts the budgets: the worst-case key (8 labels,
//! wrapping on 4 lines, an icon image, automatic plates), drawn and
//! encoded as JPEG, in at most 1 ms at 120 px; a label text change with
//! the scaled icon cached in at most 0.3 ms at 72 px. A debug build only
//! prints the numbers.

use std::sync::Arc;
use std::time::{Duration, Instant};

use deckpilot_core::{
    AssetId, AssetRef, DefaultAppearance, FaceLabel, FaceStyle, IconFit, IconRef, KeyAppearance,
    KeyFace, KeyLabelLook, LabelId, LabelLayer, LabelOverflow, LabelPosition, Spot,
};
use deckpilot_render::{Icons, LibraryAsset, render_for};

const LOGO: &str = "acme.tools.logo";

fn icons() -> Icons {
    let image = image::RgbaImage::from_fn(256, 256, |x, y| {
        image::Rgba([
            u8::try_from(x % 256).unwrap(),
            u8::try_from(y % 256).unwrap(),
            128,
            255,
        ])
    });
    Icons::default().with_library([(
        AssetId::parse(LOGO).unwrap(),
        LibraryAsset::Image(Arc::new(image)),
    )])
}

fn worst(text: &str) -> KeyFace {
    let spots = [
        Spot::TopLeft,
        Spot::Top,
        Spot::TopRight,
        Spot::Left,
        Spot::Center,
        Spot::Right,
        Spot::BottomLeft,
        Spot::Bottom,
    ];
    let layers = [
        LabelLayer::BehindIcon,
        LabelLayer::OverIcon,
        LabelLayer::OverTitle,
    ];
    let labels = spots
        .iter()
        .enumerate()
        .map(|(index, spot)| FaceLabel {
            id: LabelId::new(&format!("l{index}")).unwrap(),
            text: format!("{text} {index} zażółć gęślą jaźń źdźbło łąki pociąg"),
            spec: KeyLabelLook {
                position: LabelPosition::Anchored {
                    to: *spot,
                    inset: None,
                },
                overflow: Some(LabelOverflow::Wrap),
                max_lines: Some(4),
                layer: Some(layers[index % 3]),
                ..KeyLabelLook::default()
            },
            unresolved: false,
            markers: Vec::new(),
        })
        .collect();
    KeyFace::new(FaceStyle::Action, "Deploy")
        .with_appearance(Some(&KeyAppearance {
            icon: Some(IconRef {
                id: AssetRef::parse(LOGO).unwrap(),
                fit: IconFit::Fill,
            }),
            ..KeyAppearance::default()
        }))
        .with_labels(labels)
}

fn jpeg(frame: &deckpilot_device::Frame) -> usize {
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
        .encode(
            frame.rgb(),
            frame.width(),
            frame.height(),
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    out.len()
}

/// The median of `runs` timings of `f`.
fn median(runs: usize, mut f: impl FnMut(usize)) -> Duration {
    let mut times: Vec<Duration> = (0..runs)
        .map(|run| {
            let start = Instant::now();
            f(run);
            start.elapsed()
        })
        .collect();
    times.sort();
    times[runs / 2]
}

#[test]
fn label_cost() {
    let icons = icons();
    let defaults = DefaultAppearance::BUILTIN;
    // A debug build (`cargo test --workspace`) only checks that it runs.
    let runs = if cfg!(debug_assertions) { 5 } else { 200 };
    // Every run a new text (a cache miss of the frame), the icon scaled
    // once per size.
    let worst_120 = median(runs, |run| {
        let frame = render_for(&worst(&run.to_string()), 120, 120, &icons, &defaults);
        std::hint::black_box(jpeg(&frame));
    });
    let worst_72 = median(runs, |run| {
        let frame = render_for(&worst(&run.to_string()), 72, 72, &icons, &defaults);
        std::hint::black_box(jpeg(&frame));
    });
    let layout_120 = median(runs, |run| {
        std::hint::black_box(deckpilot_render::label_layout(
            &worst(&run.to_string()),
            120,
            120,
            &icons,
        ));
    });
    let no_jpeg_120 = median(runs, |run| {
        std::hint::black_box(render_for(
            &worst(&run.to_string()),
            120,
            120,
            &icons,
            &defaults,
        ));
    });
    println!("  layout only at 120 px: {layout_120:?}; render without JPEG: {no_jpeg_120:?}");
    let one_label = |text: &str| {
        let mut face = worst(text);
        face.labels.truncate(1);
        face.labels[0].spec = KeyLabelLook {
            position: LabelPosition::Anchored {
                to: Spot::Bottom,
                inset: None,
            },
            ..KeyLabelLook::default()
        };
        face
    };
    let change_72 = median(runs, |run| {
        let frame = render_for(&one_label(&format!("{run}%")), 72, 72, &icons, &defaults);
        std::hint::black_box(jpeg(&frame));
    });
    let mut plain = one_label("x");
    plain.labels.clear();
    let plain_72 = median(runs, |_| {
        let frame = render_for(&plain, 72, 72, &icons, &defaults);
        std::hint::black_box(jpeg(&frame));
    });
    // Without the scaled image cache: a new set of icons every run.
    let uncached_72 = median(runs.min(50), |run| {
        let fresh = icons.clone().with_library([]);
        let frame = render_for(&one_label(&format!("{run}%")), 72, 72, &fresh, &defaults);
        std::hint::black_box(jpeg(&frame));
    });
    println!(
        "label_cost (median of {runs}, render + JPEG):\n  \
         worst case (8 labels, wrap 4, icon, plates) at 120 px: {worst_120:?}\n  \
         worst case at 72 px: {worst_72:?}\n  \
         one label text change at 72 px, scaled icon cached: {change_72:?}\n  \
         the same without the scaled image cache: {uncached_72:?}\n  \
         the key without labels at 72 px: {plain_72:?}\n  \
         icons scaled: {}",
        icons.scaled_count()
    );
    // Two sizes, one scale each: the rest came from the cache.
    assert_eq!(icons.scaled_count(), 2);
    if cfg!(debug_assertions) {
        println!("  (debug build: budgets not checked)");
        return;
    }
    assert!(worst_120 <= Duration::from_millis(1), "{worst_120:?}");
    assert!(change_72 <= Duration::from_micros(300), "{change_72:?}");
}
