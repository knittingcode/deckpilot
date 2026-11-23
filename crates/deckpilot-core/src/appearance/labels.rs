//! Key labels: extra texts a key shows besides its title (`docs/key-text.md`,
//! "Labels").
//!
//! A look ([`KeyAppearance::labels`](super::KeyAppearance::labels)) holds at
//! most [`MAX_LABELS`] labels. Each is a display template (resolved like the
//! title, [`crate::variables::resolve_display_template`]) placed on the key
//! by a preset, an absolute point or an edge, drawn in one of three layers,
//! with an automatic contrast plate by default. Labels float: they never
//! move the title, the mark or the icon.
//!
//! Every measure is an integer (the look is part of the frame cache key):
//! sizes, offsets, padding and insets are **units** of a canvas whose
//! shorter side is 72 (points: pixels on a 72 px key), widths and heights
//! percent of the canvas, absolute points per mille. Every field but the
//! id and the text is optional, and a default is not written, so a later
//! field never changes what an older one meant.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{Color, TEXT_SIZE_POINTS, TextAlign};

/// Most labels on one key.
pub const MAX_LABELS: usize = 8;
/// Longest label template, in characters.
pub const MAX_LABEL_TEMPLATE: usize = 160;
/// Most grapheme clusters of a resolved label text that are laid out; the
/// rest is cut with `…` before measuring.
pub const MAX_LABEL_CLUSTERS: usize = 120;
/// Longest label id, in characters (`[a-z0-9-]`).
pub const MAX_LABEL_ID: usize = 16;
/// The fewest and most lines of a wrapped label.
pub const LABEL_LINES: (u8, u8) = (1, 4);
/// Lines of a wrapped label when none are set.
pub const DEFAULT_LABEL_LINES: u8 = 2;
/// A label's size when none is set, in points.
pub const DEFAULT_LABEL_POINTS: u8 = 10;
/// The smallest size a shrinking label goes to when none is set.
pub const DEFAULT_LABEL_MIN_POINTS: u8 = 6;
/// Smallest and largest padding and corner radius, in units.
pub const LABEL_SPACING: (u8, u8) = (0, 12);
/// Padding around a label's text when none is set, in units.
pub const DEFAULT_LABEL_PADDING: u8 = 2;
/// A plate's corner radius when none is set, in units.
pub const DEFAULT_LABEL_RADIUS: u8 = 2;
/// The opacity of an automatic plate when none is set, in percent.
pub const DEFAULT_PLATE_OPACITY: u8 = 60;
/// The opacity of a plate of the label's own colour when none is set.
pub const DEFAULT_COLOR_PLATE_OPACITY: u8 = 100;
/// Smallest and largest width or height, in percent of the canvas.
pub const LABEL_PERCENT: (u8, u8) = (5, 100);
/// Largest offset either way, in units.
pub const MAX_LABEL_OFFSET: i8 = 72;
/// Largest inset of an anchored label, in units.
pub const MAX_LABEL_INSET: u8 = 36;
/// An absolute position's largest coordinate (per mille of the canvas).
pub const PER_MILLE: u16 = 1000;
/// The shorter side of the label canvas, in units.
pub const CANVAS_UNITS: u16 = 72;

/// A label's id: stable, unique within one look, `[a-z0-9-]{1,16}`. The
/// editor gives a new label eight random hex digits. Dependencies, reports
/// and the editor's selection refer to it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LabelId {
    bytes: [u8; MAX_LABEL_ID],
    len: u8,
}

impl LabelId {
    pub fn new(id: &str) -> Result<Self, String> {
        let valid = !id.is_empty()
            && id.len() <= MAX_LABEL_ID
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !valid {
            return Err(format!(
                "“{id}” is not a label id; use 1 to {MAX_LABEL_ID} of a-z, 0-9 and -"
            ));
        }
        let mut bytes = [0; MAX_LABEL_ID];
        bytes[..id.len()].copy_from_slice(id.as_bytes());
        Ok(Self {
            bytes,
            len: u8::try_from(id.len()).unwrap_or(0),
        })
    }

    pub fn as_str(&self) -> &str {
        // Only ASCII is ever stored.
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or_default()
    }
}

impl fmt::Debug for LabelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LabelId({})", self.as_str())
    }
}

impl fmt::Display for LabelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for LabelId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for LabelId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        Self::new(&id).map_err(serde::de::Error::custom)
    }
}

/// A point of a box: its corners, edge centres and centre.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Spot {
    TopLeft,
    Top,
    TopRight,
    Left,
    #[default]
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Spot {
    pub const ALL: [Self; 9] = [
        Self::TopLeft,
        Self::Top,
        Self::TopRight,
        Self::Left,
        Self::Center,
        Self::Right,
        Self::BottomLeft,
        Self::Bottom,
        Self::BottomRight,
    ];

    /// Where the spot is across and down a box: 0 (left, top), 1 (centre)
    /// or 2 (right, bottom).
    pub fn grid(self) -> (u8, u8) {
        match self {
            Self::TopLeft => (0, 0),
            Self::Top => (1, 0),
            Self::TopRight => (2, 0),
            Self::Left => (0, 1),
            Self::Center => (1, 1),
            Self::Right => (2, 1),
            Self::BottomLeft => (0, 2),
            Self::Bottom => (1, 2),
            Self::BottomRight => (2, 2),
        }
    }

    /// A corner (not an edge centre or the centre).
    pub fn is_corner(self) -> bool {
        let (x, y) = self.grid();
        x != 1 && y != 1
    }
}

/// Where a label goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum LabelPosition {
    /// A readable default inside the key's text area: top, centre, bottom
    /// or a corner (left and right are refused).
    Preset { at: Spot },
    /// A point of the canvas in per mille of its width and height; `origin`
    /// is the point of the label's box placed there (default: its centre).
    Absolute {
        x: u16,
        y: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin: Option<Spot>,
    },
    /// Against an edge or a corner of the key itself (not the text area),
    /// `inset` units in from it (default 0): ribbons and corner tags.
    Anchored {
        to: Spot,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inset: Option<u8>,
    },
}

impl Default for LabelPosition {
    fn default() -> Self {
        Self::Preset { at: Spot::Bottom }
    }
}

impl LabelPosition {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    #[must_use]
    fn normalized(self) -> Self {
        match self {
            Self::Absolute {
                x,
                y,
                origin: Some(Spot::Center),
            } => Self::Absolute { x, y, origin: None },
            Self::Anchored { to, inset: Some(0) } => Self::Anchored { to, inset: None },
            other => other,
        }
    }
}

/// What is drawn behind a label's text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum LabelBackground {
    /// A dark or light plate when the text would not read otherwise: over
    /// a picture, or on a background without enough contrast.
    #[default]
    Auto,
    /// Never a plate.
    None,
    /// A plate of this colour.
    Color(Color),
}

impl Serialize for LabelBackground {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Auto => serializer.serialize_str("auto"),
            Self::None => serializer.serialize_str("none"),
            Self::Color(color) => color.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for LabelBackground {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "auto" => Ok(Self::Auto),
            "none" => Ok(Self::None),
            other => Color::parse(other).map(Self::Color).map_err(|_| {
                serde::de::Error::custom(format!(
                    "“{other}” is not a label background; use auto, none or #rrggbb"
                ))
            }),
        }
    }
}

/// Which band of the key a label is painted in; within a band, in list
/// order. State marks are always on top.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LabelLayer {
    /// Under the key's icon image (over the background).
    BehindIcon,
    /// Over the icon and the mark, under the title.
    #[default]
    OverIcon,
    /// Over the title.
    OverTitle,
}

/// What happens to a label's text that does not fit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LabelOverflow {
    /// One line, cut with `…`.
    #[default]
    Ellipsis,
    /// One line, clipped at the box.
    Clip,
    /// Up to `max_lines` lines at word boundaries, the last cut with `…`.
    Wrap,
    /// One line, smaller down to `min_size`, then cut with `…`.
    Shrink,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LabelWeight {
    /// Inter Medium.
    #[default]
    Regular,
    /// Inter `SemiBold`.
    Bold,
}

/// When a label is shown: a closed set, decided by the core from what it
/// resolves anyway. No expressions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LabelVisibility {
    #[default]
    Always,
    /// Hidden while a reference of its template has no value or the text
    /// comes out empty.
    WhenResolved,
    /// Hidden while no project is active.
    WhenProject,
}

/// How a label looks and where it goes: everything of a [`KeyLabel`] but
/// its id, text, switch and visibility. What the renderer gets
/// ([`crate::FaceLabel`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyLabelLook {
    #[serde(default, skip_serializing_if = "LabelPosition::is_default")]
    pub position: LabelPosition,
    /// Moves the label right, in units (-72..=72).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset_x: Option<i8>,
    /// Moves the label down, in units (-72..=72).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset_y: Option<i8>,
    /// A fixed width, in percent of the canvas width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u8>,
    /// As wide as the text, up to this, in percent of the canvas width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<u8>,
    /// A fixed height, in percent of the canvas height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u8>,
    /// As high as the text, up to this, in percent of the canvas height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u8>,
    /// Points ([`TEXT_SIZE_POINTS`]). Default [`DEFAULT_LABEL_POINTS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<LabelWeight>,
    /// Default: by the position (left in a left corner, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<TextAlign>,
    /// Default: the key's text colour (or white or black on a plate it
    /// does not read on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<LabelBackground>,
    /// The plate's opacity in percent. Default 60 (automatic plate), 100
    /// (a colour).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_opacity: Option<u8>,
    /// Units around the text. Default [`DEFAULT_LABEL_PADDING`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<u8>,
    /// The plate's corner radius, in units. Default [`DEFAULT_LABEL_RADIUS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<LabelLayer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overflow: Option<LabelOverflow>,
    /// Wrap: most lines ([`LABEL_LINES`]). Default [`DEFAULT_LABEL_LINES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_lines: Option<u8>,
    /// Shrink: the smallest size, in points. Default
    /// [`DEFAULT_LABEL_MIN_POINTS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_size: Option<u8>,
}

/// One label of a key's look.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyLabel {
    pub id: LabelId,
    /// A display template, at most [`MAX_LABEL_TEMPLATE`] characters.
    pub text: String,
    /// `Some(false)` keeps the label without drawing it. Default: drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<LabelVisibility>,
    #[serde(flatten)]
    pub look: KeyLabelLook,
}

impl KeyLabelLook {
    /// The size in points.
    pub fn points(&self) -> u8 {
        self.size.unwrap_or(DEFAULT_LABEL_POINTS)
    }

    pub fn padding(&self) -> u8 {
        self.padding.unwrap_or(DEFAULT_LABEL_PADDING)
    }

    pub fn radius(&self) -> u8 {
        self.radius.unwrap_or(DEFAULT_LABEL_RADIUS)
    }

    pub fn layer(&self) -> LabelLayer {
        self.layer.unwrap_or_default()
    }

    pub fn overflow(&self) -> LabelOverflow {
        self.overflow.unwrap_or_default()
    }

    pub fn background(&self) -> LabelBackground {
        self.background.unwrap_or_default()
    }

    /// The plate's opacity in percent.
    pub fn background_opacity(&self) -> u8 {
        self.background_opacity
            .unwrap_or(match self.background() {
                LabelBackground::Color(_) => DEFAULT_COLOR_PLATE_OPACITY,
                LabelBackground::Auto | LabelBackground::None => DEFAULT_PLATE_OPACITY,
            })
            .min(100)
    }

    pub fn max_lines(&self) -> u8 {
        self.max_lines
            .unwrap_or(DEFAULT_LABEL_LINES)
            .clamp(LABEL_LINES.0, LABEL_LINES.1)
    }

    pub fn min_points(&self) -> u8 {
        self.min_size
            .unwrap_or(DEFAULT_LABEL_MIN_POINTS)
            .min(self.points())
    }

    /// This look without values that are the defaults.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        fn drop_default<T: PartialEq>(value: &mut Option<T>, default: &T) {
            if value.as_ref() == Some(default) {
                *value = None;
            }
        }
        self.position = self.position.normalized();
        drop_default(&mut self.offset_x, &0);
        drop_default(&mut self.offset_y, &0);
        drop_default(&mut self.size, &DEFAULT_LABEL_POINTS);
        drop_default(&mut self.weight, &LabelWeight::Regular);
        drop_default(&mut self.background, &LabelBackground::Auto);
        drop_default(&mut self.padding, &DEFAULT_LABEL_PADDING);
        drop_default(&mut self.radius, &DEFAULT_LABEL_RADIUS);
        drop_default(&mut self.layer, &LabelLayer::OverIcon);
        drop_default(&mut self.overflow, &LabelOverflow::Ellipsis);
        drop_default(&mut self.max_lines, &DEFAULT_LABEL_LINES);
        drop_default(&mut self.min_size, &DEFAULT_LABEL_MIN_POINTS);
        let opacity = match self.background() {
            LabelBackground::Color(_) => DEFAULT_COLOR_PLATE_OPACITY,
            LabelBackground::Auto | LabelBackground::None => DEFAULT_PLATE_OPACITY,
        };
        drop_default(&mut self.background_opacity, &opacity);
        self
    }

    /// Checks ranges and combinations (not the text).
    pub fn check(&self) -> Result<(), String> {
        let range = |name: &str, value: Option<u8>, (low, high): (u8, u8)| match value {
            Some(value) if !(low..=high).contains(&value) => {
                Err(format!("label {name} {value} is not within {low}–{high}"))
            }
            _ => Ok(()),
        };
        match self.position {
            LabelPosition::Preset {
                at: at @ (Spot::Left | Spot::Right),
            } => {
                return Err(format!(
                    "a label preset is top, center, bottom or a corner, not {at:?}"
                ));
            }
            LabelPosition::Absolute { x, y, .. } if x > PER_MILLE || y > PER_MILLE => {
                return Err(format!(
                    "a label's absolute position is 0–{PER_MILLE} per mille, not {x}, {y}"
                ));
            }
            LabelPosition::Anchored {
                inset: Some(inset), ..
            } if inset > MAX_LABEL_INSET => {
                return Err(format!(
                    "a label's inset is at most {MAX_LABEL_INSET} units, not {inset}"
                ));
            }
            _ => {}
        }
        for (name, offset) in [("x", self.offset_x), ("y", self.offset_y)] {
            if let Some(offset) = offset
                && !(-MAX_LABEL_OFFSET..=MAX_LABEL_OFFSET).contains(&offset)
            {
                return Err(format!(
                    "a label's {name} offset is within ±{MAX_LABEL_OFFSET} units, not {offset}"
                ));
            }
        }
        range("width", self.width, LABEL_PERCENT)?;
        range("max width", self.max_width, LABEL_PERCENT)?;
        range("height", self.height, LABEL_PERCENT)?;
        range("max height", self.max_height, LABEL_PERCENT)?;
        if self.width.is_some() && self.max_width.is_some() {
            return Err("a label has a fixed width or a largest width, not both".into());
        }
        if self.height.is_some() && self.max_height.is_some() {
            return Err("a label has a fixed height or a largest height, not both".into());
        }
        range("size", self.size, TEXT_SIZE_POINTS)?;
        range("smallest size", self.min_size, TEXT_SIZE_POINTS)?;
        if let Some(min) = self.min_size
            && min > self.points()
        {
            return Err(format!(
                "a label's smallest size ({min}) is above its size ({})",
                self.points()
            ));
        }
        range("line count", self.max_lines, LABEL_LINES)?;
        range("padding", self.padding, LABEL_SPACING)?;
        range("corner radius", self.radius, LABEL_SPACING)?;
        range("background opacity", self.background_opacity, (0, 100))?;
        Ok(())
    }
}

impl KeyLabel {
    /// A label with `text` at the default position (bottom).
    pub fn new(id: LabelId, text: impl Into<String>) -> Self {
        Self {
            id,
            text: text.into(),
            enabled: None,
            visible: None,
            look: KeyLabelLook::default(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub fn visibility(&self) -> LabelVisibility {
        self.visible.unwrap_or_default()
    }

    /// `None` when its template is empty; otherwise the label without
    /// values that are the defaults.
    pub fn normalized(mut self) -> Option<Self> {
        if self.text.trim().is_empty() {
            return None;
        }
        if self.enabled == Some(true) {
            self.enabled = None;
        }
        if self.visible == Some(LabelVisibility::Always) {
            self.visible = None;
        }
        self.look = self.look.normalized();
        Some(self)
    }
}

/// What is wrong with a reference in a label's template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TemplateIssue {
    /// Not a reference (kept as typed): a warning.
    Invalid { text: String },
    /// A secret, a sensitive or a non-display source, or a run's value:
    /// never shown on a key, refused on save.
    NotDisplayable { reference: String },
}

impl TemplateIssue {
    /// Whether the template cannot be saved with it.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::NotDisplayable { .. })
    }
}

/// The issues of the references of `template`, judged by the display
/// allow-list ([`crate::variables::display_safe`]) with the variables of
/// `registry`. A module's variable that is not registered is not an issue
/// here (its module may be disabled); it shows the unresolved mark.
pub fn template_issues(
    template: &str,
    registry: &crate::variables::VariableRegistry,
) -> Vec<TemplateIssue> {
    let mut issues = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            break;
        };
        let raw = after[..end].trim();
        match crate::variables::VariableRef::parse(raw) {
            Ok(reference) => {
                if crate::variables::display_safe(&reference, registry).is_err() {
                    issues.push(TemplateIssue::NotDisplayable {
                        reference: raw.to_owned(),
                    });
                }
            }
            Err(_) => issues.push(TemplateIssue::Invalid {
                text: raw.to_owned(),
            }),
        }
        rest = &after[end + 2..];
    }
    issues
}

/// Checks a look's labels with the core's variables: the count, unique ids,
/// each label's values, the template's length and that it shows nothing
/// that is not displayable.
pub fn check_labels(labels: &[KeyLabel]) -> Result<(), String> {
    static CORE: std::sync::LazyLock<crate::variables::VariableRegistry> =
        std::sync::LazyLock::new(crate::variables::VariableRegistry::new);
    check_labels_with(labels, &CORE)
}

/// [`check_labels`] with the variables of `registry` (the agent's, with
/// the modules' and plugins').
pub fn check_labels_with(
    labels: &[KeyLabel],
    registry: &crate::variables::VariableRegistry,
) -> Result<(), String> {
    if labels.len() > MAX_LABELS {
        return Err(format!(
            "a key has at most {MAX_LABELS} labels, not {}",
            labels.len()
        ));
    }
    for (index, label) in labels.iter().enumerate() {
        if labels[..index].iter().any(|other| other.id == label.id) {
            return Err(format!("two labels of a key have the id “{}”", label.id));
        }
        let length = label.text.chars().count();
        if length > MAX_LABEL_TEMPLATE {
            return Err(format!(
                "label “{}” is {length} characters long; at most {MAX_LABEL_TEMPLATE}",
                label.id
            ));
        }
        label
            .look
            .check()
            .map_err(|error| format!("label “{}”: {error}", label.id))?;
        if let Some(TemplateIssue::NotDisplayable { reference }) =
            template_issues(&label.text, registry)
                .into_iter()
                .find(TemplateIssue::is_error)
        {
            return Err(format!(
                "label “{}” uses {reference}, which a key never shows (a secret, a sensitive or a run's value)",
                label.id
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
