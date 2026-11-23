//! Key labels ([`FaceLabel`]s): laid out on a canvas of units and drawn in
//! three layers (`docs/key-text.md`, "Labels").
//!
//! The canvas's shorter side is [`CANVAS_UNITS`] (72) units whatever the
//! key's size, so every decision (line breaks, cuts, sizes, boxes, plates)
//! is made in units and comes out the same at 72, 96, 120 px and in a 2× or
//! 3× preview: the text engine has no hinting and its kerning scales
//! linearly, so a width in units is a width in pixels divided by the scale.
//! Only drawing multiplies by the scale; plate edges are snapped to whole
//! pixels. [`label_layout`] is what is drawn, and what the editor gets.
//!
//! Faces without labels never reach this module: their keys are drawn
//! exactly as before labels existed.

use deckpilot_core::appearance::labels::{CANVAS_UNITS, MAX_LABEL_CLUSTERS, PER_MILLE};
use deckpilot_core::{
    FaceLabel, IconFit, KeyAppearance, KeyFace, LabelBackground, LabelId, LabelLayer,
    LabelOverflow, LabelPosition, LabelWeight, Spot, TextAlign,
};
use deckpilot_device::Frame;
use serde::{Deserialize, Serialize};

use super::{KeyColors, OwnIcon, Request, background_layer, key_colors, layout, mark_of, own_icon};
use crate::icons::Icons;
use crate::text::{self, Style, Weight};

/// Steps a shrinking label tries between its size and its smallest size
/// (as the title's auto fit).
const SHRINK_STEPS: f32 = 48.0;
/// Corners and absolute labels: the widest they grow on their own, as a
/// share of the canvas width.
const CORNER_SHARE: f32 = 0.6;
/// The contrast below which an automatic plate is drawn (WCAG AA).
const MIN_CONTRAST: f32 = 4.5;

/// The size of the label canvas in units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Canvas {
    pub width: f32,
    pub height: f32,
}

impl Canvas {
    /// The canvas of a `width` × `height` pixel key, and its pixels per unit.
    pub fn of(width: u32, height: u32) -> (Self, f32) {
        let short = width.min(height).max(1) as f32;
        let scale = short / f32::from(CANVAS_UNITS);
        (
            Self {
                width: width as f32 / scale,
                height: height as f32 / scale,
            },
            scale,
        )
    }
}

/// Where the renderer put a label, in canvas units (`docs/key-text.md`,
/// "Labels"): what the editor draws its outlines and handles from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelBox {
    pub id: LabelId,
    /// The box (with padding) after it was moved into the canvas.
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub canvas: Canvas,
    pub layer: LabelLayer,
    /// Text was cut (`…`, clipped, or lines left out).
    pub truncated: bool,
    /// The box meets the title's text block.
    pub overlaps_title: bool,
    /// A plate is drawn behind the text.
    pub plate: bool,
    /// The size the text is drawn at, in points.
    pub font_points: f32,
    pub lines: usize,
}

/// A rectangle in units: left, top, right, bottom.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl Rect {
    fn meets(self, other: Self) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }
}

/// A label laid out: its box and lines, in units, and how it is painted.
#[derive(Debug, Clone)]
pub(crate) struct Placed {
    pub report: LabelBox,
    /// The lines: text and their top left corner.
    lines: Vec<(String, f32, f32)>,
    /// The em size, in units.
    size: f32,
    weight: Weight,
    /// Columns the lines draw in, for clipped labels.
    clip: Option<(f32, f32)>,
    ink: [u8; 3],
    /// The plate's colour and opacity (0–1), and its corner radius in units.
    plate: Option<([u8; 3], f32)>,
    radius: f32,
}

/// What a label is laid out against: the key's colours, the pictures under
/// it and the title's text block, all in units.
struct Scene {
    canvas: Canvas,
    colors: KeyColors,
    pictures: Vec<Rect>,
    title: Option<Rect>,
}

fn scene(face: &KeyFace, width: u32, height: u32, icons: &Icons, colors: KeyColors) -> Scene {
    let (canvas, scale) = Canvas::of(width, height);
    let default_look = KeyAppearance::default();
    let look = face.appearance.as_ref().unwrap_or(&default_look);
    let own = own_icon(look, icons);
    let whole = Rect {
        left: 0.0,
        top: 0.0,
        right: canvas.width,
        bottom: canvas.height,
    };
    let mut pictures = Vec::new();
    // A background asset (image or gradient) and a series cover the key.
    if background_layer(look, icons).is_some() || face.series.is_some() {
        pictures.push(whole);
    }
    if let Some(OwnIcon::Image(image, fit)) = own {
        pictures.push(match fit {
            IconFit::Fill | IconFit::Crop => whole,
            IconFit::Fit => {
                let (w, h) = image.dimensions();
                let ratio = (canvas.width / w.max(1) as f32).min(canvas.height / h.max(1) as f32);
                let (w, h) = (w as f32 * ratio, h as f32 * ratio);
                Rect {
                    left: (canvas.width - w) / 2.0,
                    top: (canvas.height - h) / 2.0,
                    right: f32::midpoint(canvas.width, w),
                    bottom: f32::midpoint(canvas.height, h),
                }
            }
        });
    }
    // The title's block as the title layout puts it (in pixels, at this
    // size), across the text area.
    let placed = layout(
        &Request {
            face,
            look,
            colors,
            has_mark: mark_of(face, own).is_some(),
            over_icon: matches!(own, Some(OwnIcon::Image(..))),
        },
        width,
        height,
    );
    let title = placed.block.map(|(top, bottom)| {
        let (left, right) = placed
            .lines
            .iter()
            .fold((f32::MAX, f32::MIN), |(l, r), line| {
                (
                    l.min(line.left),
                    r.max(line.left + line.style.width(&line.text)),
                )
            });
        Rect {
            left: left / scale,
            top: top / scale,
            right: right / scale,
            bottom: bottom / scale,
        }
    });
    Scene {
        canvas,
        colors,
        pictures,
        title,
    }
}

/// The point `spot` of `rect`.
fn point(rect: Rect, spot: Spot) -> (f32, f32) {
    let (gx, gy) = spot.grid();
    let along = |from: f32, to: f32, g: u8| from + (to - from) * f32::from(g) / 2.0;
    (
        along(rect.left, rect.right, gx),
        along(rect.top, rect.bottom, gy),
    )
}

/// How wide a label is when it sets no width.
enum Width {
    /// This wide, whatever the text.
    Fixed(f32),
    /// As wide as the text, up to this.
    Auto(f32),
}

fn align_of(spot: Spot) -> TextAlign {
    match spot.grid().0 {
        0 => TextAlign::Left,
        2 => TextAlign::Right,
        _ => TextAlign::Center,
    }
}

/// `text` cut after [`MAX_LABEL_CLUSTERS`] grapheme clusters with `…`.
fn capped(text: &str) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    let clusters: Vec<&str> = text.graphemes(true).collect();
    if clusters.len() <= MAX_LABEL_CLUSTERS {
        return text.to_owned();
    }
    let mut cut = clusters[..MAX_LABEL_CLUSTERS - 1].concat();
    cut.push('…');
    cut
}

/// WCAG relative luminance of an sRGB colour.
fn luminance([r, g, b]: [u8; 3]) -> f32 {
    let channel = |c: u8| {
        let c = f32::from(c) / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Lays out one label against `scene`.
#[expect(clippy::too_many_lines, reason = "one label's layout, top to bottom")]
fn place(label: &FaceLabel, scene: &Scene) -> Placed {
    let spec = &label.spec;
    let canvas = scene.canvas;
    let (cw, ch) = (canvas.width, canvas.height);
    let text = capped(&label.text);
    let padding = f32::from(spec.padding());
    let size = f32::from(spec.points());
    let weight = match spec.weight.unwrap_or_default() {
        LabelWeight::Regular => Weight::Medium,
        LabelWeight::Bold => Weight::SemiBold,
    };
    let style = Style::new(weight, size);
    // The text area of the title: 5 units in from the sides, a twelfth of
    // the height from the top and the bottom.
    let safe = Rect {
        left: 5.0,
        top: ch / 12.0,
        right: cw - 5.0,
        bottom: ch - ch / 12.0,
    };
    let whole = Rect {
        left: 0.0,
        top: 0.0,
        right: cw,
        bottom: ch,
    };
    let (anchor, origin, default_width, default_align) = match spec.position {
        LabelPosition::Preset { at } => {
            let width = if at.is_corner() {
                Width::Auto(cw * CORNER_SHARE)
            } else {
                Width::Fixed(safe.right - safe.left)
            };
            (point(safe, at), at, width, align_of(at))
        }
        LabelPosition::Anchored { to, inset } => {
            let inset = f32::from(inset.unwrap_or(0));
            let (gx, gy) = to.grid();
            let (x, y) = point(whole, to);
            let toward = |g: u8| match g {
                0 => inset,
                2 => -inset,
                _ => 0.0,
            };
            let edge = gx == 1 && gy != 1;
            let width = if edge {
                Width::Fixed(cw - 2.0 * inset)
            } else {
                Width::Auto(cw * CORNER_SHARE)
            };
            ((x + toward(gx), y + toward(gy)), to, width, align_of(to))
        }
        LabelPosition::Absolute { x, y, origin } => (
            (
                f32::from(x.min(PER_MILLE)) / f32::from(PER_MILLE) * cw,
                f32::from(y.min(PER_MILLE)) / f32::from(PER_MILLE) * ch,
            ),
            origin.unwrap_or(Spot::Center),
            Width::Auto(cw),
            TextAlign::Center,
        ),
    };
    let anchor = (
        anchor.0 + f32::from(spec.offset_x.unwrap_or(0)),
        anchor.1 + f32::from(spec.offset_y.unwrap_or(0)),
    );
    let percent = |value: u8, of: f32| f32::from(value.min(100)) / 100.0 * of;
    let width_rule = match (spec.width, spec.max_width) {
        (Some(width), _) => Width::Fixed(percent(width, cw)),
        (None, Some(max)) => Width::Auto(percent(max, cw)),
        (None, None) => default_width,
    };
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let width = match width_rule {
        Width::Fixed(width) => width,
        Width::Auto(limit) => (style.width(&one_line) + 2.0 * padding).min(limit),
    }
    .clamp(2.0 * padding + 1.0, cw);
    let content_width = (width - 2.0 * padding).max(1.0);
    // The most height the text may take.
    let max_box_height = match (spec.height, spec.max_height) {
        (Some(height), _) | (None, Some(height)) => percent(height, ch),
        (None, None) => ch,
    };
    let text_room = (max_box_height - 2.0 * padding).max(0.0);
    let (lines, style, truncated, clip) = match spec.overflow() {
        LabelOverflow::Ellipsis => {
            let shown = style.fit_halving(&one_line, content_width);
            let cut = shown != one_line;
            (vec![shown], style, cut, None)
        }
        LabelOverflow::Clip => {
            let wide = style.width(&one_line) > content_width;
            (vec![one_line.clone()], style, wide, Some(()))
        }
        LabelOverflow::Shrink => {
            let least = f32::from(spec.min_points()).min(size);
            let step = ((size - least) / SHRINK_STEPS).max(0.25);
            let found =
                std::iter::successors(Some(size), |s| (*s - step > least).then_some(*s - step))
                    .chain(std::iter::once(least))
                    .map(|s| style.with_size(s))
                    .find(|s| s.width(&one_line) <= content_width);
            if let Some(fitting) = found {
                (vec![one_line.clone()], fitting, false, None)
            } else {
                let small = style.with_size(least);
                (
                    vec![small.fit_halving(&one_line, content_width)],
                    small,
                    true,
                    None,
                )
            }
        }
        LabelOverflow::Wrap => {
            let max_lines = usize::from(spec.max_lines());
            let wrapped = style.wrapped_halving(&text, content_width, max_lines);
            let fit = (text_room / style.line_height()).floor().max(1.0) as usize;
            if wrapped.lines.len() > fit {
                // Lines that do not fit the height: the last one kept ends
                // with `…`.
                let mut lines = wrapped.lines[..fit - 1].to_vec();
                let rest = wrapped.lines[fit - 1..].join(" ");
                lines.push(style.fit_halving(&rest, content_width));
                (lines, style, true, None)
            } else {
                (wrapped.lines, style, wrapped.truncated, None)
            }
        }
    };
    let line_height = style.line_height();
    let content_height = lines.len() as f32 * line_height;
    let height = match (spec.height, spec.max_height) {
        (Some(height), _) => percent(height, ch),
        (None, Some(max)) => (content_height + 2.0 * padding).min(percent(max, ch)),
        (None, None) => content_height + 2.0 * padding,
    }
    .min(ch);
    // The origin point of the box at the anchor, then moved (not cut) into
    // the canvas.
    let (gx, gy) = origin.grid();
    let x = (anchor.0 - width * f32::from(gx) / 2.0).clamp(0.0, (cw - width).max(0.0));
    let y = (anchor.1 - height * f32::from(gy) / 2.0).clamp(0.0, (ch - height).max(0.0));
    let bounds = Rect {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    };
    let align = spec.align.unwrap_or(default_align);
    let content_left = x + padding;
    let first_top = y + (height - content_height) / 2.0;
    let placed_lines: Vec<(String, f32, f32)> = lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let wide = style.width(&line);
            let left = match (align, clip) {
                (_, Some(())) if wide > content_width => content_left,
                (TextAlign::Left, _) => content_left,
                (TextAlign::Center, _) => content_left + (content_width - wide) / 2.0,
                (TextAlign::Right, _) => content_left + content_width - wide,
            };
            (line, left, first_top + index as f32 * line_height)
        })
        .collect();
    // The plate and the ink.
    let opacity = f32::from(spec.background_opacity()) / 100.0;
    let own_ink = spec.color.map(deckpilot_core::Color::rgb);
    let key_ink = own_ink.unwrap_or(scene.colors.text);
    let plate = match spec.background() {
        LabelBackground::None => None,
        LabelBackground::Color(color) => Some((color.rgb(), opacity)),
        LabelBackground::Auto => {
            let over_picture = scene.pictures.iter().any(|picture| picture.meets(bounds));
            let low_contrast = contrast(key_ink, scene.colors.background) < MIN_CONTRAST;
            (over_picture || low_contrast).then(|| {
                let dark = if luminance(key_ink) >= 0.5 {
                    [0, 0, 0]
                } else {
                    [255, 255, 255]
                };
                (dark, opacity)
            })
        }
    };
    let ink = match (own_ink, plate) {
        (Some(ink), _) => ink,
        (None, Some((color, opacity))) => {
            let under = text::mix(scene.colors.background, color, opacity);
            if contrast(key_ink, under) >= MIN_CONTRAST {
                key_ink
            } else if contrast([255, 255, 255], under) >= contrast([0, 0, 0], under) {
                [255, 255, 255]
            } else {
                [0, 0, 0]
            }
        }
        (None, None) => key_ink,
    };
    let line_count = placed_lines.len();
    Placed {
        report: LabelBox {
            id: label.id,
            x,
            y,
            width,
            height,
            canvas,
            layer: spec.layer(),
            truncated,
            overlaps_title: scene.title.is_some_and(|title| title.meets(bounds)),
            plate: plate.is_some(),
            font_points: style.size,
            lines: line_count,
        },
        lines: placed_lines,
        size: style.size,
        weight,
        clip: clip.map(|()| (content_left, content_left + content_width)),
        ink,
        plate,
        radius: f32::from(spec.radius()),
    }
}

/// The labels of `face` laid out at `width` × `height`, in paint order.
pub(crate) fn place_all(
    face: &KeyFace,
    width: u32,
    height: u32,
    icons: &Icons,
    colors: KeyColors,
) -> Vec<Placed> {
    if face.labels.is_empty() || face.is_empty() || width == 0 || height == 0 {
        return Vec::new();
    }
    let scene = scene(face, width, height, icons, colors);
    face.labels
        .iter()
        .map(|label| place(label, &scene))
        .collect()
}

/// Where the labels of `face` go at `width` × `height` pixels, in canvas
/// units (the same at every size of the key), in list order. Empty for a
/// face without labels. The renderer draws exactly these boxes.
pub fn label_layout(face: &KeyFace, width: u32, height: u32, icons: &Icons) -> Vec<LabelBox> {
    let colors = key_colors(face, &deckpilot_core::DefaultAppearance::BUILTIN);
    place_all(face, width, height, icons, colors)
        .into_iter()
        .map(|placed| placed.report)
        .collect()
}

/// [`label_layout`] for a model whose defaults are `defaults` (the plates
/// depend on the key's colours).
pub fn label_layout_for(
    face: &KeyFace,
    width: u32,
    height: u32,
    icons: &Icons,
    defaults: &deckpilot_core::DefaultAppearance,
) -> Vec<LabelBox> {
    place_all(face, width, height, icons, key_colors(face, defaults))
        .into_iter()
        .map(|placed| placed.report)
        .collect()
}

/// Paints the labels of `layer`, in list order.
pub(crate) fn draw_layer(frame: &mut Frame, placed: &[Placed], layer: LabelLayer) {
    if placed.is_empty() {
        return;
    }
    let (_, scale) = Canvas::of(frame.width(), frame.height());
    for label in placed.iter().filter(|label| label.report.layer == layer) {
        if let Some((color, opacity)) = label.plate {
            plate(frame, &label.report, label.radius, color, opacity, scale);
        }
        let style = Style::new(label.weight, label.size * scale);
        let clip = label.clip.map(|(from, to)| (from * scale, to * scale));
        for (line, left, top) in &label.lines {
            style.draw_clipped(frame, line, (left * scale, top * scale), label.ink, clip);
        }
    }
}

/// A rounded plate over the box, its edges on whole pixels, its corners
/// anti-aliased.
fn plate(
    frame: &mut Frame,
    report: &LabelBox,
    radius: f32,
    color: [u8; 3],
    opacity: f32,
    scale: f32,
) {
    if opacity <= 0.0 {
        return;
    }
    let snap = |units: f32| (units * scale).round();
    let (x0, y0) = (snap(report.x).max(0.0), snap(report.y).max(0.0));
    let (x1, y1) = (
        snap(report.x + report.width).min(frame.width() as f32),
        snap(report.y + report.height).min(frame.height() as f32),
    );
    let r = (radius * scale)
        .min((x1 - x0) / 2.0)
        .min((y1 - y0) / 2.0)
        .max(0.0);
    let mut y = y0;
    while y < y1 {
        let mut x = x0;
        while x < x1 {
            // Distance into a corner's circle, from the pixel's centre.
            let (cx, cy) = (x + 0.5, y + 0.5);
            let dx = (x0 + r - cx).max(cx - (x1 - r)).max(0.0);
            let dy = (y0 + r - cy).max(cy - (y1 - r)).max(0.0);
            let coverage = if r > 0.0 && dx > 0.0 && dy > 0.0 {
                (r + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0)
            } else {
                1.0
            };
            if let Some(under) = frame.pixel(x as u32, y as u32) {
                frame.put(
                    x as u32,
                    y as u32,
                    text::mix(under, color, opacity * coverage),
                );
            }
            x += 1.0;
        }
        y += 1.0;
    }
}

#[cfg(test)]
mod tests;
