//! Calls in the IPC, protocol 19 (capability `calls`, `docs/calls.md`):
//! what the app shows about Calls and the settings it changes.
//!
//! - `getCallsStatus` answers [`CallsStatus`]; the agent pushes it again as
//!   the `callsChanged` event whenever it changes, to subscribed desktop
//!   clients of protocol 19 only ([`crate::AgentEvent::desktop_only`]).
//!   That is hygiene, not the boundary: the IPC token is, and any client
//!   holding it may ask `getCallsStatus`.
//! - `setCallsOverlay` turns the overlay on or off for one device.
//! - `restoreCallsLayout` is *Restore defaults* of one model's managed
//!   layout: the user's version is kept first as a layout of their own
//!   named `copyName`.
//!
//! - `setCallsBackToWork` sets the *Back to work* settings of one model's
//!   managed layout (they are the parameters of its "Back to work" action).
//!
//! Nothing here carries a meeting title, a participant, a process name or
//! a provider's own text: the state, the provider id and the capabilities
//! only.

use deckpilot_device::{ControlId, DeviceId, DeviceModelId};
use serde::{Deserialize, Serialize};

/// Longest name of a kept copy (`restoreCallsLayout`).
pub const MAX_COPY_NAME: usize = 80;

/// Longest device serial `setCallsOverlay` accepts.
pub const MAX_SERIAL: usize = 64;

/// Whether `name` may name a kept copy: 1 to [`MAX_COPY_NAME`] characters
/// once trimmed, with no control character, no bidirectional override or
/// isolate, no line or paragraph separator and no zero-width character
/// (each could make the name read as another).
pub fn valid_copy_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty()
        && trimmed.chars().count() <= MAX_COPY_NAME
        && !trimmed
            .chars()
            .any(|c| c.is_control() || is_bidi_control(c) || is_invisible(c))
}

fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{2028}' | '\u{2029}' | '\u{200B}'..='\u{200D}' | '\u{FEFF}'
    )
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}')
}

/// The Calls state the app shows.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallsStatus {
    /// Whether Calls runs: a provider is installed with the user's
    /// consent. Without it nothing else is filled in.
    pub installed: bool,
    /// The focused call's state name (`idle`, `incoming`, `outgoing`,
    /// `connecting`, `active`, `ending`, `ended`, `failed`, `unknown`).
    pub state: String,
    /// The provider of the focused call (`os-signals`, `mock`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Whether any control is possible (false for detection-only Teams).
    pub controls: bool,
    pub devices: Vec<CallsDevice>,
    pub layouts: Vec<CallsLayout>,
}

/// One connected device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallsDevice {
    pub device: DeviceId,
    /// The per-device setting (default: on for devices with key displays).
    pub overlay: bool,
    /// The device shows the overlay now.
    pub shown: bool,
    /// The device went "back to work" (its overlay is suspended).
    pub suspended: bool,
}

/// One model's managed layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallsLayout {
    pub model: DeviceModelId,
    pub layout: String,
    pub name: String,
    /// The user changed it since it was generated.
    pub edited: bool,
    pub template_version: String,
    /// Its call pages (incoming, outgoing, active), with each control and
    /// whether the provider supports it now.
    #[serde(default)]
    pub pages: Vec<CallsPage>,
    /// Its *Back to work* settings.
    #[serde(default)]
    pub back: BackToWork,
}

/// Which call page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CallsPageKind {
    Incoming,
    Outgoing,
    Active,
}

/// One call page of a managed layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallsPage {
    pub page: CallsPageKind,
    /// The page's name in the layout (the user may rename it).
    pub name: String,
    pub controls: Vec<CallsControl>,
}

/// One control of a call page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallsControl {
    pub control: ControlId,
    /// The Calls action (`answer`, `mic-toggle`, `return-to-work`, ...).
    pub action: String,
    /// Whether it can work with the provider now. Always true for the
    /// actions that control no call (`return-to-work`).
    pub supported: bool,
    /// Why not, when not: a stable code (`provider-has-no-api`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Shortest and longest *Back to work* timer (mode A), in seconds.
pub const TIMER_SECONDS: (u32, u32) = (5, 600);
/// Shortest and longest maximum of mode C, in seconds.
pub const MAX_SECONDS: (u32, u32) = (10, 1800);

/// When a device that went back to work shows the call again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BackToWorkMode {
    /// Mode A: after `seconds` ([`TIMER_SECONDS`]; presets 20, 30, 60).
    Timer { seconds: u32 },
    /// Mode B: when a run its key started ends.
    Completion,
    /// Mode C, the default: a run's end or `max_seconds`
    /// ([`MAX_SECONDS`]), whichever comes first.
    #[serde(rename_all = "camelCase")]
    CompletionOrMax { max_seconds: u32 },
}

impl Default for BackToWorkMode {
    fn default() -> Self {
        Self::CompletionOrMax { max_seconds: 60 }
    }
}

/// What happens when the run that counts fails or is cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AfterRun {
    /// Show the call again.
    Resume,
    /// Stay on the user's own layout.
    Stay,
}

/// The *Back to work* settings of a managed layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackToWork {
    pub mode: BackToWorkMode,
    /// Default: resume.
    pub on_failure: AfterRun,
    /// Default: stay (a cancelled run did not do what the user meant).
    pub on_cancel: AfterRun,
    /// Every device that shows the call goes back to work (default off).
    pub all_devices: bool,
}

impl Default for BackToWork {
    fn default() -> Self {
        Self {
            mode: BackToWorkMode::default(),
            on_failure: AfterRun::Resume,
            on_cancel: AfterRun::Stay,
            all_devices: false,
        }
    }
}

impl BackToWork {
    /// Whether the times are in their ranges.
    pub fn validate(&self) -> Result<(), String> {
        let within = |n: u32, (min, max): (u32, u32), what: &str| {
            if (min..=max).contains(&n) {
                Ok(())
            } else {
                Err(format!("{what} is {min} to {max} seconds"))
            }
        };
        match self.mode {
            BackToWorkMode::Timer { seconds } => within(seconds, TIMER_SECONDS, "the timer"),
            BackToWorkMode::Completion => Ok(()),
            BackToWorkMode::CompletionOrMax { max_seconds } => {
                within(max_seconds, MAX_SECONDS, "the maximum")
            }
        }
    }
}

/// Why a Calls request was refused.
pub mod reason {
    /// Calls is not installed (no provider, no consent).
    pub const NOT_INSTALLED: &str = "callsNotInstalled";
    /// The model has no managed layout.
    pub const NO_LAYOUT: &str = "callsNoLayout";
    /// The device is not connected.
    pub const UNKNOWN_DEVICE: &str = "callsUnknownDevice";
}
