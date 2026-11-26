use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{DeviceError, DeviceEvent, DeviceId};

/// Connection state of a known device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionState {
    Connected,
    Disconnected,
}

/// Why a device is, or is not, usable: finer than [`ConnectionState`],
/// which stays `connected` / `disconnected` for older clients
/// (`docs/devices.md` "Device states"). A device that is physically
/// present is never reported [`DeviceAvailability::NotConnected`] only
/// because it could not be opened or enumeration failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceAvailability {
    /// Open and working.
    Connected,
    /// Not seen by the system.
    NotConnected,
    /// The USB bus lists it, but it has no HID interface (Linux only; e.g.
    /// a hub that could not configure it). Nothing can talk to it until it
    /// is replugged or the hub is power-cycled.
    UsbPresentHidUnavailable,
    /// The operating system refused access (a missing udev rule on Linux).
    PermissionDenied,
    /// A model DeckPilot has no driver for.
    Unsupported,
    /// Another program has it open.
    Busy,
    /// It opens but fails, or keeps dropping right after it connects.
    CommunicationError,
    /// The connection was lost while the device is still listed; it is
    /// opened again soon.
    Reconnecting,
    /// Not known, e.g. while device enumeration fails. Also what a newer
    /// state reads as in an older client.
    #[serde(other)]
    Unknown,
}

impl DeviceAvailability {
    /// The state of a listed device whose open failed with `error`.
    pub fn of_open_error(error: &DeviceError) -> Self {
        match error {
            DeviceError::PermissionDenied(_) => Self::PermissionDenied,
            DeviceError::Busy(_) => Self::Busy,
            DeviceError::Unsupported(_) => Self::Unsupported,
            // Listed, then gone when opened: it is coming or going.
            DeviceError::Disconnected => Self::Reconnecting,
            DeviceError::Io(_) | DeviceError::InvalidControl(_) | DeviceError::InvalidFrame(_) => {
                Self::CommunicationError
            }
        }
    }

    /// Whether a device in this state is physically there (seen by the
    /// system), whether or not DeckPilot can use it.
    pub fn is_present(self) -> bool {
        !matches!(self, Self::NotConnected | Self::Unknown)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStatus {
    pub device: DeviceId,
    pub state: ConnectionState,
}

/// Tracks every device seen during a session, including disconnected ones, so
/// that hotplug and multiple simultaneous devices are handled uniformly.
#[derive(Debug, Clone, Default)]
pub struct DeviceRegistry {
    devices: BTreeMap<DeviceId, ConnectionState>,
}

impl DeviceRegistry {
    /// Applies a device event. Returns `true` when the connection state changed.
    pub fn apply(&mut self, event: &DeviceEvent) -> bool {
        let new_state = match event {
            DeviceEvent::Connected { .. } => ConnectionState::Connected,
            DeviceEvent::Disconnected { .. } => ConnectionState::Disconnected,
            DeviceEvent::Input { .. } => return false,
        };
        let previous = self.devices.insert(event.device().clone(), new_state);
        previous != Some(new_state)
    }

    pub fn state(&self, device: &DeviceId) -> Option<ConnectionState> {
        self.devices.get(device).copied()
    }

    pub fn is_connected(&self, device: &DeviceId) -> bool {
        self.state(device) == Some(ConnectionState::Connected)
    }

    pub fn statuses(&self) -> Vec<DeviceStatus> {
        self.devices
            .iter()
            .map(|(device, state)| DeviceStatus {
                device: device.clone(),
                state: *state,
            })
            .collect()
    }

    pub fn connected(&self) -> impl Iterator<Item = &DeviceId> {
        self.devices
            .iter()
            .filter(|(_, state)| **state == ConnectionState::Connected)
            .map(|(device, _)| device)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ControlId, ControlInput, DeviceModelId};

    fn device(serial: &str) -> DeviceId {
        DeviceId {
            model: DeviceModelId::new("test.model"),
            serial: serial.into(),
        }
    }

    #[test]
    fn tracks_hotplug_for_multiple_devices() {
        let mut registry = DeviceRegistry::default();
        assert!(registry.apply(&DeviceEvent::Connected {
            device: device("a")
        }));
        assert!(registry.apply(&DeviceEvent::Connected {
            device: device("b")
        }));
        assert!(!registry.apply(&DeviceEvent::Connected {
            device: device("a")
        }));
        assert_eq!(registry.connected().count(), 2);

        assert!(registry.apply(&DeviceEvent::Disconnected {
            device: device("a")
        }));
        assert_eq!(
            registry.state(&device("a")),
            Some(ConnectionState::Disconnected)
        );
        assert!(registry.is_connected(&device("b")));
        assert_eq!(registry.statuses().len(), 2);
    }

    #[test]
    fn open_errors_map_to_states() {
        use DeviceAvailability as A;
        let cases = [
            (
                DeviceError::PermissionDenied("EACCES".into()),
                A::PermissionDenied,
            ),
            (DeviceError::Busy("exclusive".into()), A::Busy),
            (DeviceError::Unsupported("model".into()), A::Unsupported),
            (DeviceError::Disconnected, A::Reconnecting),
            (DeviceError::Io("pipe".into()), A::CommunicationError),
            (
                DeviceError::InvalidFrame("size".into()),
                A::CommunicationError,
            ),
        ];
        for (error, state) in cases {
            assert_eq!(DeviceAvailability::of_open_error(&error), state, "{error}");
            assert!(state.is_present());
        }
        assert!(!A::NotConnected.is_present());
        assert!(!A::Unknown.is_present());
    }

    #[test]
    fn availability_is_camel_case_and_unknown_values_read_as_unknown() {
        assert_eq!(
            serde_json::to_value(DeviceAvailability::UsbPresentHidUnavailable).unwrap(),
            "usbPresentHidUnavailable"
        );
        assert_eq!(
            serde_json::to_value(DeviceAvailability::NotConnected).unwrap(),
            "notConnected"
        );
        let newer: DeviceAvailability = serde_json::from_value("somethingNew".into()).unwrap();
        assert_eq!(newer, DeviceAvailability::Unknown);
    }

    #[test]
    fn input_events_do_not_change_state() {
        let mut registry = DeviceRegistry::default();
        let changed = registry.apply(&DeviceEvent::Input {
            device: device("a"),
            control: ControlId::Key { index: 0 },
            input: ControlInput::Press,
        });
        assert!(!changed);
        assert_eq!(registry.state(&device("a")), None);
    }
}
