//! Calls in the IPC, protocol 19 (capability `calls`, `docs/calls.md`):
//! what the app shows about Calls and the settings it changes.
//!
//! - `getCallsStatus` answers [`CallsStatus`]; the agent pushes it again as
//!   the `callsChanged` event whenever it changes (to desktop clients of
//!   protocol 19 only).
//! - `setCallsOverlay` turns the overlay on or off for one device.
//! - `restoreCallsLayout` is *Restore defaults* of one model's managed
//!   layout: the user's version is kept first as a layout of their own
//!   named `copyName`.
//!
//! Nothing here carries a meeting title, a participant, a process name or
//! a provider's own text: the state, the provider id and the capabilities
//! only.

use deckpilot_device::{DeviceId, DeviceModelId};
use serde::{Deserialize, Serialize};

/// Longest name of a kept copy (`restoreCallsLayout`).
pub const MAX_COPY_NAME: usize = 80;

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
}

/// Why a Calls request was refused.
pub mod reason {
    /// Calls is not installed (no provider, no consent).
    pub const NOT_INSTALLED: &str = "callsNotInstalled";
    /// The model has no managed layout.
    pub const NO_LAYOUT: &str = "callsNoLayout";
}
