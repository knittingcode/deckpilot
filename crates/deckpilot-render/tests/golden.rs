//! Golden images of key faces (`tests/golden/`).
//!
//! Each set is a list of faces drawn at every size of [`SIZES`]; a frame is
//! known by the SHA-256 of its RGB bytes (`<set>.sha256`), and a contact
//! sheet per size (`<set>-<w>x<h>.png`: the set's faces left to right, in
//! order) is kept next to it for review.
//!
//! - **G0** (`g0`): faces without key labels, generated from `main` at
//!   `a87116de`, before any label code existed. They prove that keys
//!   without labels render byte for byte as before; they are never blessed
//!   again by a change that adds to the renderer (`DECKPILOT_BLESS=g0` is
//!   the only way to rewrite them, on purpose).
//!
//! - **Labels** (`labels`): G1–G8 of `docs/key-text.md`, "Labels": every
//!   preset, ribbons over icons, absolute positions near the edges, eight
//!   labels in three layers, every overflow mode, plates, state marks over
//!   labels; the touch zone is one of the sizes.
//!
//! - **Open** (`open`): the automatic icon of Open keys
//!   (`docs/system-actions.md`, "The key's icon"): the document for a file
//!   and when nothing is known, the open folder for a folder, and the
//!   user's own icon drawn over either.
//!
//! `DECKPILOT_BLESS=1` rewrites every other set from what the renderer
//! draws now.
//!
//! Frames are compared exactly on macOS (where the sets are blessed). The
//! platform's `sin`/`cos` (gradients, the folder Back vector) may differ in
//! the last bit elsewhere, so other platforms accept a frame that differs
//! from the reviewed image by at most one level per channel; any larger
//! difference fails everywhere.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use deckpilot_core::{
    Accent, AssetId, AssetRef, Background, Color, DefaultAppearance, FaceLabel, FaceStyle,
    FitLines, FontSize, Glyph, IconFit, IconRef, Indicator, KeyAppearance, KeyFace, KeyLabelLook,
    LabelBackground, LabelId, LabelLayer, LabelOverflow, LabelPosition, LabelWeight, Spot,
    TextAlign, TextAppearance, TextMode, TextOverflow, TitleAlign,
};
use deckpilot_device::{Frame, KeyBackground};
use deckpilot_render::{Icons, LibraryAsset, render_for};
use sha2::{Digest, Sha256};

/// Every key and display size the sets are drawn at: the built-in models
/// (MK.2 72, XL 96, Plus key 120, Plus touch zone 200×100), synthetic
/// driver models of classes the catalog lacks (Mini-class 80; Neo-class
/// keys are 96, as XL), a non-square display and the app's 2× and 3×
/// previews of an MK.2 key.
const SIZES: [(u32, u32); 8] = [
    (72, 72),
    (96, 96),
    (120, 120),
    (200, 100),
    (80, 80),
    (144, 112),
    (144, 144),
    (216, 216),
];

/// One face of a set, drawn with `icons` on a model whose defaults are
/// `defaults`.
struct Case {
    name: &'static str,
    face: KeyFace,
    defaults: DefaultAppearance,
}

fn case(name: &'static str, face: KeyFace) -> Case {
    Case {
        name,
        face,
        defaults: DefaultAppearance::BUILTIN,
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "cases are written as literals"
)]
fn look(face: KeyFace, look: KeyAppearance) -> KeyFace {
    face.with_appearance(Some(&look))
}

fn action(title: &str) -> KeyFace {
    KeyFace::new(FaceStyle::Action, title)
}

fn text(mode: TextMode) -> TextAppearance {
    TextAppearance {
        mode,
        ..TextAppearance::default()
    }
}

/// A picture with hard edges, soft edges and translucency: quadrants of
/// colour, a diagonal ramp and a transparent border.
fn picture(width: u32, height: u32) -> image::RgbaImage {
    image::RgbaImage::from_fn(width, height, |x, y| {
        let border = x < 2 || y < 2 || x + 2 >= width || y + 2 >= height;
        if border {
            return image::Rgba([0, 0, 0, 0]);
        }
        let (left, top) = (x < width / 2, y < height / 2);
        let base = match (left, top) {
            (true, true) => [230, 57, 70],
            (false, true) => [42, 157, 143],
            (true, false) => [233, 196, 106],
            (false, false) => [38, 70, 83],
        };
        let ramp = u8::try_from((x + y) * 255 / (width + height).max(1)).unwrap();
        let alpha = if (x / 4 + y / 4) % 7 == 0 { 128 } else { 255 };
        image::Rgba([base[0], base[1], base[2].saturating_add(ramp / 4), alpha])
    })
}

const LOGO: &str = "acme.tools.logo";
const WIDE: &str = "acme.tools.wide";
const PHOTO: &str = "acme.tools.photo";

/// The images the sets use: plugin assets (a square logo, a wide image and
/// a photo-like background).
fn icons() -> Icons {
    let asset = |id: &str, image: image::RgbaImage| {
        (
            AssetId::parse(id).unwrap(),
            LibraryAsset::Image(Arc::new(image)),
        )
    };
    Icons::default().with_library([
        asset(LOGO, picture(64, 64)),
        asset(WIDE, picture(96, 48)),
        asset(PHOTO, picture(160, 120)),
    ])
}

#[expect(clippy::unnecessary_wraps, reason = "the look's field is optional")]
fn icon(id: &str, fit: IconFit) -> Option<IconRef> {
    Some(IconRef {
        id: AssetRef::parse(id).unwrap(),
        fit,
    })
}

#[expect(clippy::unnecessary_wraps, reason = "the look's field is optional")]
fn background(text: &str) -> Option<Background> {
    Some(Background::parse(text).unwrap())
}

/// G0: faces without labels, across title modes, icons, backgrounds,
/// state marks and the special keys.
#[expect(clippy::too_many_lines, reason = "one case per entry")]
fn g0() -> Vec<Case> {
    let mut stateful = action("Build").with_detail("ok");
    stateful.badge = Some("3".into());
    stateful.toggle = Some(true);
    stateful.progress = Some(40);
    stateful.indicator = Some(Indicator::Ok);
    stateful.highlighted = true;
    let mut loading = action("Sync").with_symbol(Some("refresh-cw"));
    loading.indicator = Some(Indicator::Loading);
    loading.toggle = Some(false);
    let mut series = action("CPU").with_detail("23%");
    series.series = Some(vec![10, 40, 25, 80, 60, 95, 30, 50]);
    series.accent = Some(Accent::Blue);
    let mut glyph = action("Done").with_detail("2 of 3");
    glyph.glyph = Some(Glyph::Check);
    glyph.accent = Some(Accent::Green);
    let mut notice = KeyFace::new(FaceStyle::Notice, "No layout");
    notice.detail = Some("Open the app".into());
    vec![
        case("empty", KeyFace::empty()),
        case("title", action("Mail")),
        case("title-detail", action("Start Work").with_detail("SHOP")),
        case("title-long-wrap", action("Open the project dashboard now")),
        case("polish", action("Zażółć gęślą jaźń").with_detail("Łódź")),
        case(
            "symbol-title",
            action("Terminal").with_symbol(Some("terminal")),
        ),
        case("symbol-alone", action("").with_symbol(Some("cpu"))),
        case("glyph-accent", glyph),
        case(
            "single-line-ellipsis",
            look(
                action("Deploy to production servers"),
                KeyAppearance {
                    text: Some(TextAppearance {
                        overflow: Some(TextOverflow::Ellipsis),
                        ..text(TextMode::SingleLine)
                    }),
                    ..KeyAppearance::default()
                },
            ),
        ),
        case(
            "single-line-clip",
            look(
                action("Deploy to production servers"),
                KeyAppearance {
                    text: Some(TextAppearance {
                        overflow: Some(TextOverflow::Clip),
                        align: Some(TextAlign::Left),
                        ..text(TextMode::SingleLine)
                    }),
                    ..KeyAppearance::default()
                },
            ),
        ),
        case(
            "auto-fit-line",
            look(
                action("Release notes").with_symbol(Some("file-text")),
                KeyAppearance {
                    text: Some(TextAppearance {
                        fit: Some(FitLines::SingleLine),
                        ..text(TextMode::AutoFit)
                    }),
                    ..KeyAppearance::default()
                },
            ),
        ),
        case(
            "auto-fit-multiline",
            look(
                action("Restart the local development server"),
                KeyAppearance {
                    text: Some(TextAppearance {
                        fit: Some(FitLines::Multiline),
                        max_lines: Some(3),
                        min_size: Some(6),
                        ..text(TextMode::AutoFit)
                    }),
                    ..KeyAppearance::default()
                },
            ),
        ),
        case(
            "marquee-still",
            look(
                action("A very long scrolling title text"),
                KeyAppearance {
                    text: Some(text(TextMode::Marquee)),
                    ..KeyAppearance::default()
                },
            ),
        ),
        case(
            "top-unknown-symbol",
            look(
                action("Notes").with_symbol(Some("notebook")),
                KeyAppearance {
                    title_align: Some(TitleAlign::Top),
                    ..KeyAppearance::default()
                },
            ),
        ),
        case(
            "middle-drops-mark",
            look(
                action("Centre").with_symbol(Some("cpu")),
                KeyAppearance {
                    title_align: Some(TitleAlign::Middle),
                    ..KeyAppearance::default()
                },
            ),
        ),
        case(
            "typography",
            look(
                action("Bold italic").with_detail("value"),
                KeyAppearance {
                    bold: Some(true),
                    italic: Some(true),
                    underline: Some(true),
                    font_size: Some(FontSize::Large),
                    text: Some(TextAppearance {
                        size: Some(14),
                        align: Some(TextAlign::Left),
                        max_lines: Some(2),
                        ..text(TextMode::Wrap)
                    }),
                    ..KeyAppearance::default()
                },
            ),
        ),
        case(
            "colours",
            look(
                action("Colours").with_detail("own"),
                KeyAppearance {
                    background: Some(Background::Plain(KeyBackground::Color(Color([
                        240, 240, 235,
                    ])))),
                    text_color: Some(Color([20, 30, 120])),
                    font_size: Some(FontSize::Small),
                    ..KeyAppearance::default()
                },
            ),
        ),
        case(
            "icon-fill-title",
            look(
                action("Logo"),
                KeyAppearance {
                    icon: icon(LOGO, IconFit::Fill),
                    ..KeyAppearance::default()
                },
            ),
        ),
        case(
            "icon-fit-alone",
            look(
                action("Wide"),
                KeyAppearance {
                    icon: icon(WIDE, IconFit::Fit),
                    show_title: Some(false),
                    ..KeyAppearance::default()
                },
            ),
        ),
        case(
            "icon-crop-top",
            look(
                action("Cropped").with_detail("12:30"),
                KeyAppearance {
                    icon: icon(WIDE, IconFit::Crop),
                    title_align: Some(TitleAlign::Top),
                    ..KeyAppearance::default()
                },
            ),
        ),
        case(
            "library-symbol-icon",
            look(
                action("Library").with_symbol(Some("cpu")),
                KeyAppearance {
                    icon: icon("deckpilot.rocket", IconFit::Fit),
                    ..KeyAppearance::default()
                },
            ),
        ),
        case(
            "gradient",
            look(
                action("Midnight").with_symbol(Some("moon")),
                KeyAppearance {
                    background: background("asset:deckpilot.gradient-midnight"),
                    ..KeyAppearance::default()
                },
            ),
        ),
        case(
            "background-image",
            look(
                action("Photo"),
                KeyAppearance {
                    background: background(&format!("asset:{PHOTO}")),
                    ..KeyAppearance::default()
                },
            ),
        ),
        case("state-marks", stateful),
        case("loading-toggle-off", loading),
        case("series-accent", series),
        case(
            "folder-back",
            deckpilot_core::device_runtime::system_back_face(),
        ),
        case("notice", notice),
        Case {
            name: "model-defaults",
            face: action("Model").with_symbol(Some("monitor")),
            defaults: DefaultAppearance {
                key_background: KeyBackground::Color(Color([30, 60, 90])),
                text_color: Color([250, 220, 120]),
                display_background: Color([10, 10, 10]),
            },
        },
    ]
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn hash(frame: &Frame) -> String {
    let digest = Sha256::digest(frame.rgb());
    digest.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn key(case: &str, (w, h): (u32, u32)) -> String {
    format!("{case}@{w}x{h}")
}

/// The contact sheet of one size: the frames left to right with a 2 px
/// magenta gap.
fn sheet(frames: &[&Frame], (w, h): (u32, u32)) -> image::RgbImage {
    let gap = 2;
    let count = u32::try_from(frames.len()).unwrap();
    let mut out = image::RgbImage::from_pixel(
        count * (w + gap) + gap,
        h + 2 * gap,
        image::Rgb([255, 0, 255]),
    );
    for (index, frame) in (0..count).zip(frames) {
        let left = gap + index * (w + gap);
        for y in 0..h {
            for x in 0..w {
                out.put_pixel(left + x, gap + y, image::Rgb(frame.pixel(x, y).unwrap()));
            }
        }
    }
    out
}

/// The frame of sheet position `index` of `sheet`, back as RGB bytes.
fn from_sheet(sheet: &image::RgbImage, index: u32, (w, h): (u32, u32)) -> Vec<u8> {
    let gap = 2;
    let left = gap + index * (w + gap);
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            rgb.extend_from_slice(&sheet.get_pixel(left + x, gap + y).0);
        }
    }
    rgb
}

fn read_manifest(path: &Path) -> BTreeMap<String, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let (name, hash) = line.split_once(' ')?;
            Some((name.to_owned(), hash.trim().to_owned()))
        })
        .collect()
}

fn blessing(set: &str) -> bool {
    match std::env::var("DECKPILOT_BLESS").as_deref() {
        // G0 is the baseline from before labels: only on purpose.
        Ok("g0") => set == "g0",
        Ok("1") => set != "g0",
        _ => false,
    }
}

/// Draws `cases` at every size and compares them with the set `set`, or
/// rewrites it when blessing.
fn check_set(set: &str, header: &str, cases: &[Case], icons: &Icons) {
    let dir = golden_dir();
    let manifest_path = dir.join(format!("{set}.sha256"));
    let frames: Vec<Vec<Frame>> = SIZES
        .iter()
        .map(|&(w, h)| {
            cases
                .iter()
                .map(|case| render_for(&case.face, w, h, icons, &case.defaults))
                .collect()
        })
        .collect();
    let names: Vec<&str> = cases.iter().map(|case| case.name).collect();
    let unique: std::collections::BTreeSet<_> = names.iter().collect();
    assert_eq!(unique.len(), names.len(), "case names are unique");
    if blessing(set) {
        std::fs::create_dir_all(&dir).unwrap();
        let mut manifest =
            format!("{header}\n# <case>@<width>x<height> <sha256 of the RGB frame>\n");
        for (size, row) in SIZES.iter().zip(&frames) {
            for (case, frame) in cases.iter().zip(row) {
                let _ = writeln!(manifest, "{} {}", key(case.name, *size), hash(frame));
            }
            let refs: Vec<&Frame> = row.iter().collect();
            sheet(&refs, *size)
                .save(dir.join(format!("{set}-{}x{}.png", size.0, size.1)))
                .unwrap();
        }
        std::fs::write(&manifest_path, manifest).unwrap();
        eprintln!("blessed {set}: {} frames", cases.len() * SIZES.len());
        return;
    }
    let manifest = read_manifest(&manifest_path);
    assert!(
        !manifest.is_empty(),
        "{} is missing; bless it with DECKPILOT_BLESS=1",
        manifest_path.display()
    );
    let exact_required = cfg!(target_os = "macos");
    let mut failures = Vec::new();
    let mut near = 0;
    for (size, row) in SIZES.iter().zip(&frames) {
        let sheet_path = dir.join(format!("{set}-{}x{}.png", size.0, size.1));
        let reviewed = image::open(&sheet_path).ok().map(|image| image.to_rgb8());
        for ((index, case), frame) in (0_u32..).zip(cases).zip(row) {
            let name = key(case.name, *size);
            let Some(expected) = manifest.get(&name) else {
                failures.push(format!("{name}: not in {set}.sha256"));
                continue;
            };
            if &hash(frame) == expected {
                continue;
            }
            // Within one level of the reviewed image (and that image is
            // the blessed frame): another platform's libm, not a change.
            let close = !exact_required
                && reviewed.as_ref().is_some_and(|sheet| {
                    let stored = from_sheet(sheet, index, *size);
                    hash_bytes(&stored) == *expected
                        && stored
                            .iter()
                            .zip(frame.rgb())
                            .all(|(a, b)| a.abs_diff(*b) <= 1)
                });
            if close {
                near += 1;
            } else {
                failures.push(format!("{name}: the frame differs from the golden image"));
            }
        }
    }
    if near > 0 {
        eprintln!("{set}: {near} frames within one level of the golden image (not exact)");
    }
    let missing: Vec<_> = manifest
        .keys()
        .filter(|name| {
            !cases
                .iter()
                .any(|case| SIZES.iter().any(|size| &key(case.name, *size) == *name))
        })
        .collect();
    assert!(missing.is_empty(), "cases no longer drawn: {missing:?}");
    assert!(
        failures.is_empty(),
        "{} of {} frames of {set} differ:\n{}",
        failures.len(),
        cases.len() * SIZES.len(),
        failures.join("\n")
    );
}

fn hash_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// G0: keys without labels draw exactly as on `main` at `a87116de`.
#[test]
fn g0_keys_without_labels_are_unchanged() {
    check_set(
        "g0",
        "# G0: key faces without labels, generated from main at a87116de (before key labels).\n# Never re-blessed by label changes (DECKPILOT_BLESS=g0 only).",
        &g0(),
        &icons(),
    );
}

fn label(id: &str, text: &str, spec: KeyLabelLook) -> FaceLabel {
    FaceLabel {
        id: LabelId::new(id).unwrap(),
        text: text.into(),
        spec,
        unresolved: false,
        markers: Vec::new(),
    }
}

fn at(spot: Spot) -> KeyLabelLook {
    KeyLabelLook {
        position: LabelPosition::Preset { at: spot },
        ..KeyLabelLook::default()
    }
}

fn anchored(to: Spot) -> KeyLabelLook {
    KeyLabelLook {
        position: LabelPosition::Anchored { to, inset: None },
        ..KeyLabelLook::default()
    }
}

fn absolute(x: u16, y: u16, origin: Spot) -> KeyLabelLook {
    KeyLabelLook {
        position: LabelPosition::Absolute {
            x,
            y,
            origin: Some(origin),
        },
        ..KeyLabelLook::default()
    }
}

fn labelled(face: KeyFace, labels: Vec<FaceLabel>) -> KeyFace {
    face.with_labels(labels)
}

fn iconed(title: &str, fit: IconFit) -> KeyFace {
    look(
        action(title),
        KeyAppearance {
            icon: icon(LOGO, fit),
            ..KeyAppearance::default()
        },
    )
}

const POLISH: &str = "Zażółć gęślą jaźń — źdźbło łąki";

/// G1–G8.
#[expect(clippy::too_many_lines, reason = "one case per entry")]
fn labels() -> Vec<Case> {
    let symbol = || action("Mail").with_symbol(Some("mail"));
    let mut cases: Vec<Case> = [
        ("g1-top", Spot::Top),
        ("g1-center", Spot::Center),
        ("g1-bottom", Spot::Bottom),
        ("g1-top-left", Spot::TopLeft),
        ("g1-top-right", Spot::TopRight),
        ("g1-bottom-left", Spot::BottomLeft),
        ("g1-bottom-right", Spot::BottomRight),
    ]
    .into_iter()
    .map(|(name, spot)| {
        case(
            name,
            labelled(symbol(), vec![label("a", "SHOP-12", at(spot))]),
        )
    })
    .collect();
    let ribbon = KeyLabelLook {
        padding: Some(2),
        ..anchored(Spot::Bottom)
    };
    cases.extend([
        case(
            "g2-ribbon-over-icon",
            labelled(
                look(
                    action(""),
                    KeyAppearance {
                        icon: icon(LOGO, IconFit::Fill),
                        show_title: Some(false),
                        ..KeyAppearance::default()
                    },
                ),
                vec![label("ribbon", "Shop front", ribbon.clone())],
            ),
        ),
        case(
            "g2-ribbon-icon-title",
            labelled(
                iconed("Deploy", IconFit::Fit),
                vec![label(
                    "ribbon",
                    "staging",
                    KeyLabelLook {
                        position: LabelPosition::Anchored {
                            to: Spot::Top,
                            inset: Some(2),
                        },
                        ..ribbon.clone()
                    },
                )],
            ),
        ),
        case(
            "g3-absolute-edges",
            labelled(
                action("Edges"),
                vec![
                    label(
                        "tl",
                        "TL",
                        KeyLabelLook {
                            offset_x: Some(-10),
                            offset_y: Some(-10),
                            ..absolute(0, 0, Spot::Center)
                        },
                    ),
                    label("br", "BR", absolute(1000, 1000, Spot::Center)),
                    label(
                        "mid",
                        "pushed",
                        KeyLabelLook {
                            offset_y: Some(30),
                            ..absolute(500, 900, Spot::Top)
                        },
                    ),
                    label(
                        "wide",
                        "A wide absolute label",
                        KeyLabelLook {
                            width: Some(100),
                            offset_x: Some(40),
                            ..absolute(500, 400, Spot::Center)
                        },
                    ),
                ],
            ),
        ),
        case(
            "g4-eight-labels",
            labelled(
                iconed("Eight", IconFit::Fit),
                vec![
                    label(
                        "behind",
                        "BEHIND",
                        KeyLabelLook {
                            layer: Some(LabelLayer::BehindIcon),
                            background: Some(LabelBackground::Color(Color([200, 30, 30]))),
                            ..anchored(Spot::Center)
                        },
                    ),
                    label("tl", "1", at(Spot::TopLeft)),
                    label("tr", "2", at(Spot::TopRight)),
                    label("bl", "3", at(Spot::BottomLeft)),
                    label("br", "4", at(Spot::BottomRight)),
                    label(
                        "top",
                        "top",
                        KeyLabelLook {
                            weight: Some(LabelWeight::Bold),
                            ..anchored(Spot::Top)
                        },
                    ),
                    label(
                        "over",
                        "OVER",
                        KeyLabelLook {
                            layer: Some(LabelLayer::OverTitle),
                            color: Some(Color([255, 220, 0])),
                            background: Some(LabelBackground::None),
                            size: Some(14),
                            ..anchored(Spot::Bottom)
                        },
                    ),
                    label(
                        "under",
                        "under title",
                        KeyLabelLook {
                            size: Some(7),
                            ..absolute(500, 700, Spot::Center)
                        },
                    ),
                ],
            ),
        ),
    ]);
    let overflow = |mode: LabelOverflow, extra: KeyLabelLook| KeyLabelLook {
        overflow: Some(mode),
        ..extra
    };
    cases.extend([
        case(
            "g5-ellipsis",
            labelled(
                action(""),
                vec![label(
                    "a",
                    POLISH,
                    overflow(LabelOverflow::Ellipsis, at(Spot::Center)),
                )],
            ),
        ),
        case(
            "g5-clip",
            labelled(
                action(""),
                vec![label(
                    "a",
                    POLISH,
                    overflow(LabelOverflow::Clip, at(Spot::Center)),
                )],
            ),
        ),
        case(
            "g5-wrap-2",
            labelled(
                action(""),
                vec![label(
                    "a",
                    POLISH,
                    overflow(LabelOverflow::Wrap, at(Spot::Center)),
                )],
            ),
        ),
        case(
            "g5-wrap-4-height",
            labelled(
                action(""),
                vec![label(
                    "a",
                    POLISH,
                    overflow(
                        LabelOverflow::Wrap,
                        KeyLabelLook {
                            max_lines: Some(4),
                            max_height: Some(40),
                            ..at(Spot::Top)
                        },
                    ),
                )],
            ),
        ),
        case(
            "g5-shrink",
            labelled(
                action(""),
                vec![
                    label(
                        "a",
                        "Łódź Kaliska",
                        overflow(
                            LabelOverflow::Shrink,
                            KeyLabelLook {
                                size: Some(16),
                                ..at(Spot::Top)
                            },
                        ),
                    ),
                    label(
                        "b",
                        POLISH,
                        overflow(LabelOverflow::Shrink, at(Spot::Bottom)),
                    ),
                ],
            ),
        ),
        case(
            "g6-solid-no-plate",
            labelled(
                action("Solid"),
                vec![label("a", "no plate", anchored(Spot::Top))],
            ),
        ),
        case(
            "g6-low-contrast",
            labelled(
                look(
                    action("Light"),
                    KeyAppearance {
                        background: Some(Background::Plain(
                            deckpilot_device::KeyBackground::Color(Color([235, 235, 230])),
                        )),
                        text_color: Some(Color([20, 20, 20])),
                        ..KeyAppearance::default()
                    },
                ),
                vec![label(
                    "a",
                    "pale",
                    KeyLabelLook {
                        color: Some(Color([250, 250, 250])),
                        ..anchored(Spot::Top)
                    },
                )],
            ),
        ),
        case(
            "g6-picture",
            labelled(
                look(
                    action("Photo"),
                    KeyAppearance {
                        background: background(&format!("asset:{PHOTO}")),
                        ..KeyAppearance::default()
                    },
                ),
                vec![
                    label("auto", "auto", anchored(Spot::TopLeft)),
                    label(
                        "none",
                        "none",
                        KeyLabelLook {
                            background: Some(LabelBackground::None),
                            ..anchored(Spot::TopRight)
                        },
                    ),
                    label(
                        "own",
                        "own",
                        KeyLabelLook {
                            background: Some(LabelBackground::Color(Color([20, 90, 200]))),
                            background_opacity: Some(50),
                            radius: Some(6),
                            ..absolute(500, 500, Spot::Center)
                        },
                    ),
                ],
            ),
        ),
        case(
            "g6-gradient",
            labelled(
                look(
                    action("Midnight"),
                    KeyAppearance {
                        background: background("asset:deckpilot.gradient-midnight"),
                        ..KeyAppearance::default()
                    },
                ),
                vec![label("a", "gradient", at(Spot::Top))],
            ),
        ),
    ]);
    let mut marked = labelled(
        action("Build").with_detail("ok"),
        vec![
            label("top", "TOP RIBBON", anchored(Spot::Top)),
            label("bottom", "BOTTOM RIBBON", anchored(Spot::Bottom)),
            label("corner", "9", anchored(Spot::TopRight)),
            label("left", "L", anchored(Spot::TopLeft)),
        ],
    );
    marked.badge = Some("3".into());
    marked.toggle = Some(true);
    marked.progress = Some(60);
    marked.indicator = Some(Indicator::Error);
    marked.highlighted = true;
    cases.push(case("g7-state-marks-on-top", marked));
    cases
}

/// G1–G8 (`docs/key-text.md`, "Labels").
#[test]
fn labels_are_drawn_as_reviewed() {
    check_set(
        "labels",
        "# Key labels G1–G8 (docs/key-text.md, \"Labels\"). DECKPILOT_BLESS=1 rewrites them.",
        &labels(),
        &icons(),
    );
}

/// Every label case breaks its lines and places its boxes the same at
/// every size (in units).
#[test]
fn label_layout_is_the_same_at_every_size() {
    /// Every field of two boxes agrees (numbers within a hundredth of a
    /// unit).
    fn same(a: &deckpilot_render::LabelBox, b: &deckpilot_render::LabelBox) -> bool {
        let close = |x: f32, y: f32| (x - y).abs() < 0.01;
        a.id == b.id
            && a.layer == b.layer
            && close(a.canvas.width, b.canvas.width)
            && close(a.canvas.height, b.canvas.height)
            && close(a.x, b.x)
            && close(a.y, b.y)
            && close(a.width, b.width)
            && close(a.height, b.height)
            && close(a.font_points, b.font_points)
            && a.lines == b.lines
            && a.truncated == b.truncated
            && a.plate == b.plate
            && a.overlaps_title == b.overlaps_title
    }
    let icons = icons();
    for case in labels() {
        let at = |w: u32, h: u32| deckpilot_render::label_layout(&case.face, w, h, &icons);
        // Keys and their previews, and the touch zone and its previews.
        for (base, sizes) in [
            (
                (72, 72),
                &[(96, 96), (120, 120), (144, 144), (216, 216), (80, 80)][..],
            ),
            ((200, 100), &[(400, 200), (600, 300)][..]),
        ] {
            let base_boxes = at(base.0, base.1);
            assert!(!base_boxes.is_empty(), "{}", case.name);
            for (w, h) in sizes {
                let other = at(*w, *h);
                assert_eq!(other.len(), base_boxes.len(), "{} {w}x{h}", case.name);
                for (a, b) in base_boxes.iter().zip(&other) {
                    assert!(same(a, b), "{} {w}x{h}: {a:?} != {b:?}", case.name);
                }
            }
        }
    }
}

/// A key's own pixels outside its labels are those of the same key
/// without labels: labels float, the title stays where it was.
#[test]
#[expect(
    clippy::cast_precision_loss,
    reason = "pixel coordinates of a 72 px key"
)]
fn labels_do_not_move_the_title() {
    let icons = icons();
    let plain = action("Mail").with_symbol(Some("mail"));
    let with = labelled(
        plain.clone(),
        vec![label("a", "x", anchored(Spot::TopLeft))],
    );
    let frame = render_for(&plain, 72, 72, &icons, &DefaultAppearance::BUILTIN);
    let labelled_frame = render_for(&with, 72, 72, &icons, &DefaultAppearance::BUILTIN);
    let boxes = deckpilot_render::label_layout(&with, 72, 72, &icons);
    let b = &boxes[0];
    let mut differing_outside = 0;
    for y in 0..72 {
        for x in 0..72 {
            let inside = (x as f32) >= b.x.floor() - 1.0
                && (x as f32) <= b.x + b.width + 1.0
                && (y as f32) >= b.y.floor() - 1.0
                && (y as f32) <= b.y + b.height + 1.0;
            if !inside && frame.pixel(x, y) != labelled_frame.pixel(x, y) {
                differing_outside += 1;
            }
        }
    }
    assert_eq!(differing_outside, 0);
    assert!(!b.overlaps_title);
}

/// Open keys as the core computes them: `file` for a file (and for a
/// target that is not chosen or cannot be told), `folder-open` for a
/// folder; the key's own icon over either.
fn open_keys() -> Vec<Case> {
    let open = |symbol: &str| action("Open").with_symbol(Some(symbol));
    let custom = KeyAppearance {
        icon: icon("deckpilot.house", IconFit::Fit),
        ..KeyAppearance::default()
    };
    let untitled = |symbol: &str| KeyFace::new(FaceStyle::Action, "").with_symbol(Some(symbol));
    vec![
        case("file", open("file")),
        case("folder", open("folder-open")),
        case("file-untitled", untitled("file")),
        case("folder-untitled", untitled("folder-open")),
        case("custom-over-file", look(open("file"), custom.clone())),
        case(
            "custom-over-folder",
            look(open("folder-open"), custom.clone()),
        ),
        case(
            "image-over-folder",
            look(
                open("folder-open"),
                KeyAppearance {
                    icon: icon(LOGO, IconFit::Fit),
                    ..KeyAppearance::default()
                },
            ),
        ),
    ]
}

/// The Open set (`docs/system-actions.md`, "The key's icon").
#[test]
fn open_keys_draw_their_automatic_icon_as_reviewed() {
    let cases = open_keys();
    check_set(
        "open",
        "# Open keys' automatic icon (docs/system-actions.md). DECKPILOT_BLESS=1 rewrites them.",
        &cases,
        &icons(),
    );
    // The user's icon wins: the same frame whatever the target.
    let draw = |name: &str| {
        let case = cases.iter().find(|c| c.name == name).unwrap();
        render_for(&case.face, 72, 72, &icons(), &case.defaults)
    };
    assert_eq!(draw("custom-over-file"), draw("custom-over-folder"));
    assert_ne!(draw("file"), draw("folder"));
}
