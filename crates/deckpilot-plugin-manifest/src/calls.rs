//! The Calls kinds of plugin API 1.16 (`docs/calls.md`): the `calls`
//! permission, `contributes.callOverlay` and `contributes.layoutTemplates`.
//!
//! All three are privileged: only the bundled Calls plugin
//! ([`CALLS_PLUGIN_ID`]) declares them, with `engines.pluginApi` `^1.16`.
//! The overlay and the templates are data: the host decides when a device
//! shows the overlay, generates the layouts, and draws every face.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::compat::ApiLevel;
use crate::model::ParameterValue;

/// The plugin API level that introduced the Calls kinds.
pub const CALLS_API: ApiLevel = ApiLevel::new(1, 16);

/// The only plugin that may declare the Calls kinds.
pub const CALLS_PLUGIN_ID: &str = "deckpilot.calls";

/// Most layout templates: one per [`TemplateShape`].
pub const MAX_LAYOUT_TEMPLATES: usize = 5;

/// Most controls one template page places.
pub const MAX_TEMPLATE_CONTROLS: usize = 32;

/// `contributes.callOverlay`: the pages of the overlay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallOverlay {
    /// 1 to 3 unique pages.
    pub pages: Vec<CallPage>,
}

/// A page of the overlay: one per call state the user acts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CallPage {
    Incoming,
    Outgoing,
    Active,
}

impl CallPage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
            Self::Active => "active",
        }
    }
}

impl fmt::Display for CallPage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The capability class a template is for. A generated layout is always
/// for one model; the host picks the template whose shape the model has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TemplateShape {
    /// At most 6 keys with displays.
    GridSmall,
    /// 7 to 15 keys with displays.
    Grid,
    /// More than 15 keys with displays.
    GridLarge,
    /// Keys and dials, with a touch strip.
    KeysDials,
    /// Keys without displays (a pedal).
    NoDisplay,
}

impl TemplateShape {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GridSmall => "grid-small",
            Self::Grid => "grid",
            Self::GridLarge => "grid-large",
            Self::KeysDials => "keys-dials",
            Self::NoDisplay => "no-display",
        }
    }

    /// The most controls of `kind` a model of this shape has.
    pub fn max_controls(self, kind: TemplateControlKind) -> u8 {
        match (self, kind) {
            (Self::GridSmall, TemplateControlKind::Key) => 6,
            (Self::Grid, TemplateControlKind::Key) => 15,
            (Self::GridLarge, TemplateControlKind::Key) => 32,
            (Self::KeysDials | Self::NoDisplay, TemplateControlKind::Key) => 8,
            (Self::KeysDials, TemplateControlKind::Dial | TemplateControlKind::Touch) => 4,
            _ => 0,
        }
    }
}

impl fmt::Display for TemplateShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `contributes.layoutTemplates[]`: the overlay's pages for one shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LayoutTemplate {
    pub shape: TemplateShape,
    /// The template's major version, 1 to 1000. A new major changes the
    /// generated layout's stable id; a plugin update with the same major
    /// regenerates an unedited layout in place.
    pub version: u32,
    pub pages: Vec<TemplatePage>,
}

/// One page of a template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplatePage {
    pub page: CallPage,
    pub controls: Vec<TemplateControl>,
}

/// What a template control is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TemplateControlKind {
    Key,
    Dial,
    Touch,
}

impl TemplateControlKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Dial => "dial",
            Self::Touch => "touch",
        }
    }
}

/// One of the plugin's own actions on one control of a page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateControl {
    pub control: TemplateControlKind,
    /// The control's position, from 0, below the shape's count.
    pub index: u8,
    /// The id of one of the plugin's own actions (`contributes.actions[].id`).
    pub action: String,
    /// Over the action's defaults, checked against its parameters.
    #[serde(
        default,
        deserialize_with = "crate::unique_map::deserialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub parameters: Option<BTreeMap<String, ParameterValue>>,
}

impl crate::Contributes {
    /// `layoutTemplates`, empty when absent.
    pub fn layout_templates(&self) -> &[LayoutTemplate] {
        self.layout_templates.as_deref().unwrap_or_default()
    }
}

impl crate::Permission {
    /// Whether a `calls` permission also asks for control.
    pub fn calls_control(&self) -> bool {
        matches!(
            self,
            Self::Calls {
                control: Some(true),
                ..
            }
        )
    }
}
