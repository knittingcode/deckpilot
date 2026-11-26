# Devices: models, drivers and verification

How DeckPilot describes control-surface hardware, how drivers plug in, and
how far each model has been verified. The design keeps one rule: only a
driver knows a vendor's protocol; everything else (core, workflows, actions,
gestures, layouts, rendering, the configuration app) works with generic
models, capabilities and events.

## Audit (main at 592d5f1, 2026-11-13)

### Where Elgato-specific types and USB ids are used

| Place | What | Kind |
| ----- | ---- | ---- |
| `crates/deckpilot-hid` | `elgato_streamdeck::{Kind, StreamDeck, StreamDeckInput}`, `hidapi`, `model_for(Kind)` | The Elgato adapter. All Elgato protocol types are here and only here. |
| `crates/deckpilot-device/src/catalog.rs` | Built-in descriptors `elgato.streamdeck.{mk2,xl,plus}` with USB `0fd9:0080/006c/0084`; `ModelCatalog::find_by_usb` | Static model data. `find_by_usb` is used only by its own test; discovery by USB id happens in the adapter. |
| `apps/agent/src/service.rs` `default_options` | `deckpilot_hid::ElgatoBackend::new()` | The one place that wires the Elgato driver into the agent. |
| `apps/agent/src/main.rs` | `--mock-devices mk2,xl,plus` | Chooses built-in models for simulated devices. |
| `crates/deckpilot-ipc` | Doc comments name `elgato-hid` as an example backend name | Strings only. |
| `tools/dev` | `hardware-test` targets `elgato.streamdeck.mk2` | Test procedure for that model. |
| Tests (core, storage, portability, agent, desktop) | `models::stream_deck_mk2()`, `"elgato.streamdeck.mk2"` | Fixtures. |

`deckpilot-core`, `deckpilot-render`, `deckpilot-storage`,
`deckpilot-portability` and the modules have **no** dependency on
`deckpilot-hid`, on `elgato-streamdeck` or on USB ids. Core uses the model
catalog only by model id (`ModelCatalog::get`) to validate layouts and pages.

### What is already generic

- `DeviceModelId` (stable string id, e.g. `elgato.streamdeck.mk2`; stored in
  layouts), `DeviceId` (model + serial).
- `DeviceCapabilities`: optional `KeyGrid` (rows, columns, image spec),
  `EncoderSpec` (dials), `TouchSurface` (zones, image, gestures), with
  `controls()`, `has_control`, `has_display`, `validate_input`.
- `ControlId` (`Key`, `Encoder`, `TouchZone`), `ControlInput` (`Press`,
  `Release`, `Rotate`, `Touch`), `DeviceEvent` (`Connected`, `Disconnected`,
  `Input`), `DeviceRegistry`/`ConnectionState`.
- `Frame`: device-independent RGB image. `deckpilot-render` draws faces into
  frames of the size the model's `ImageSpec` asks for; the driver encodes and
  orients them.
- `KeyStateTracker`: turns key-state snapshots into press/release events.
- `MockBackend` (feature `mock`): simulated devices of any model.
- Core runtime, gesture engine, layout validation and the agent's render loop
  read only capabilities and generic events.

### The backend interface

```rust
pub trait DeviceBackend: Send {
    fn name(&self) -> &'static str;                       // driver id, e.g. "elgato-hid"
    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>, DeviceError>;
    fn open(&mut self, device: &DeviceId) -> Result<Box<dyn DeviceHandle>, DeviceError>;
    fn source(&self, device: &DeviceId) -> &'static str;  // which driver provides a device
}

pub trait DeviceHandle: Send {
    fn id(&self) -> &DeviceId;
    fn read_input(&mut self, timeout: Duration) -> Result<Vec<(ControlId, ControlInput)>, DeviceError>;
    fn set_image(&mut self, control: ControlId, frame: &Frame) -> Result<(), DeviceError>;
    fn clear_image(&mut self, control: ControlId) -> Result<(), DeviceError>;
    fn set_brightness(&mut self, percent: u8) -> Result<(), DeviceError>;
    fn flush(&mut self) -> Result<(), DeviceError>;
}
```

`CombinedBackend` merges several backends (USB plus simulated) and opens
each device with the backend that discovered it.

### Gaps found

- The support level was one value for all platforms (`HardwareSupport`), and
  the MK.2 still read `Unverified` although it was tested on hardware on
  macOS.
- No generic description of a *connected device* (vendor, driver, driver
  version) for code outside the agent; IPC carried only the model name and
  the backend name.
- Model descriptors cannot come from a driver: a driver for a new vendor would
  need a change to the built-in catalog.

## Target design

### Model registry (`deckpilot-device`, `ModelCatalog`)

Static data per model: id, vendor, name, capabilities, the
**verification status per platform** with evidence date and note, and
optionally the colours of keys that set none (`defaultAppearance`: key
background, which may be `transparent`, text colour and display background;
[data-model.md](data-model.md#key-appearance)). Models without them, such
as one from an older driver, use the built-in defaults, which are the
Stream Deck's: transparent keys on a black display, white text. Model ids
never change; stored layouts refer to them.

### Verification levels

| Level | Meaning |
| ----- | ------- |
| `unsupported` | Not supported on this platform. |
| `experimental` | A work-in-progress definition; may be incomplete or wrong. |
| `defined` | The model is described; layouts can be designed. No driver. |
| `driverImplemented` | A driver exists and builds for the platform. |
| `automatedTests` | The driver's automated tests (protocol checks, simulated devices) run on the platform. No physical device. |
| `physicallyVerified` | Tested on a physical device on this platform. |

Rules:

- Only the built-in registry can record `physicallyVerified`
  (`PlatformVerification::physically_verified` is crate-private; the public
  constructor caps at `automatedTests`). Simulated devices, tests and drivers
  registered at run time cannot raise a model to it.
- A model is **supported** when it is physically verified on at least one
  platform. Other models are **experimental**: they stay in the registry
  (layouts can be designed for them) but are not listed as supported.
- A platform without an entry reads as `defined`.
- The legacy `support` field (`HardwareSupport`) is kept for clients that
  read it; it is `verified` when the model is physically verified anywhere.

Current data (recorded 2026-11-13):

| Model | macOS | Windows | Linux |
| ----- | ----- | ------- | ----- |
| Stream Deck MK.2 (`elgato.streamdeck.mk2`) | physically verified (2026-11-13: guided `deckpilot-dev hardware-test` and real use: display, keys, brightness, profile switching, USB reconnect, gestures, Work Session timer face) | automated tests | automated tests |
| Stream Deck XL (`elgato.streamdeck.xl`) | defined | defined | defined |
| Stream Deck + (`elgato.streamdeck.plus`) | defined | defined | defined |

"Automated tests" for the MK.2 on Windows and Linux means: the driver builds
and its tests run in CI there (descriptor checked against the protocol
library, simulated devices). Nobody has used a physical MK.2 on those
platforms with DeckPilot.

### Device descriptor (the generic device context)

`deckpilot_device::DeviceDescriptor` describes one connected (or simulated)
device without vendor types. It is the stable device context for code
outside the drivers, including variable resolution:

| Field | Example |
| ----- | ------- |
| `vendorId` | `elgato` (first segment of the model id) |
| `vendorName` | `Elgato` |
| `modelId` | `elgato.streamdeck.mk2` |
| `displayName` | `Stream Deck MK.2` |
| `serial` | hardware identity reported by the device |
| `capabilities` | `DeviceCapabilities` (keys, dials, touch strip) |
| `driver` | `{ "id": "elgato-hid", "version": "0.6.0" }`, or `null` when unknown |
| `verification` | per-platform list of `{ platform, level, evidenceDate, evidence }` |

How to get one:

- From a `DeviceId` and the catalog: `ModelCatalog::describe(&device, driver)`.
- In the agent: `Agent::device_descriptor(&device)` (catalog + the driver of
  the backend that provides the device).
- Over IPC: `AgentStatus.devices[].descriptor` (optional, additive field;
  older agents omit it, older clients ignore it; no protocol bump).

Fields are only ever added.

### Drivers (adapter, strategy, registry)

- **Adapter**: a vendor driver implements `DeviceBackend` (discover, open,
  `name` = driver id, `version`, `models`) and `DeviceHandle` (read input,
  set/clear images, brightness, flush). It wraps the vendor's HID/protocol
  layer, translates native reports into `ControlId` + `ControlInput`, and
  accepts generic `Frame`s. Image operations are queued and take effect on
  `flush`, in order; a clear is queued like an image.
- **Strategy**: model-specific behaviour is chosen per model by the adapter.
  In `deckpilot-hid`, `SUPPORTED` maps each library device kind to its
  DeckPilot descriptor, whose `DeviceCapabilities`/`ImageSpec` (grid, image
  size, orientation, encoding) drive rendering and input translation.
- **Registry/factory**: drivers are registered with the agent's backend
  (`CombinedBackend::with`); `default_options` registers the Elgato driver and,
  in development, the simulated one. Each driver can contribute model
  descriptors (`DeviceBackend::models`); the agent adds them to its catalog
  at start unless a built-in model has the same id (`register_missing`), so
  built-in data and its verification always win. Discovery yields
  `DeviceId`s; `ModelCatalog::describe` turns them into descriptors.
- **Capabilities**: `Capability` names the families core drives generically:
  `keys`, `keyDisplays`, `dials`, `touchStrip`, `brightness`
  (`DeviceCapabilities::supports`, `families`). The agent sends brightness
  only to devices whose model has it.

Adding a vendor needs no change to core: implement the two traits, return
the model descriptors from `models`, and register the driver. The test
`apps/agent/tests/fake_vendor.rs` does exactly that with a driver defined only
in the test: it is discovered, described, its native reports drive a layout
through generic events, and it receives frames at its model's image size.

### Device states (DP-USB-01, IPC protocol 21)

`ConnectionState` stays `connected` or `disconnected` for older clients.
`DeviceInfo.availability` (`DeviceAvailability` in `deckpilot-device`) says
why a device is or is not usable:

| State | When | Detected by |
| ----- | ---- | ----------- |
| `connected` | a session is open | the agent |
| `notConnected` | the system does not list it | discovery |
| `usbPresentHidUnavailable` | the USB bus lists a supported Stream Deck without a `hidraw` interface (e.g. a hub that could not configure it, kernel `-28`) | Linux only: sysfs `idVendor`/`idProduct`/`serial` and a `hidraw` directory below the device, read-only |
| `permissionDenied` | the open was refused (no udev rule; macOS `not permitted`) | hidapi's open error |
| `unsupported` | an Elgato model without a driver, or an open that says so | discovery, open error |
| `busy` | another program holds the device (Linux `EBUSY`, macOS exclusive access, Windows access denied or sharing violation) | hidapi's open error |
| `communicationError` | another open error, or a device that keeps dropping right after it connects (flapping) | open error, retry state |
| `reconnecting` | the connection was lost while the device is still listed; it is opened again shortly | the agent |
| `unknown` | discovery fails, so devices not open cannot be told apart; also a newer state in an older client | the agent |

hidapi reports open errors only as text; `deckpilot-hid` classifies the
platforms' wordings and codes (`classify_open_error`). On Linux the text is
in the desktop's language once GTK has set the locale (tray mode), so it is
also compared with the process's own `strerror` of `EACCES`, `EPERM`,
`EBUSY`, `ENOENT`, `ENODEV` and `ENXIO`. "USB present, HID unavailable" is
reported only when two scans in a row see it, since sysfs lists a device
briefly before its `hidraw` node is bound. While one driver fails to
enumerate and another succeeds, devices not open are `unknown`. A device that is
present is never `notConnected`: one that is listed but cannot be opened,
or that the USB bus lists without HID, appears in the device list with its
reason even though it never connected (`DeviceInfo::never_opened`, kept in
the agent). Clients before protocol 21 receive neither the field nor those
devices, so their list is what it was. The desktop's device rows and the
tray's Connected Devices show the state, with a hint where the user can act
(en and pl). Nothing here resets, re-binds or writes to a device.

**Device access guide (DP-INST-02).** Two of these states have a one-time,
dismissible guide in the desktop app (`components/device-access-guide.tsx`).
It is built from the states above and the package format the app already
reports (`UpdateState.packageFormat`), so it needs no IPC change:

- `permissionDenied` on Linux: the udev fix for the package type.
  - `.deb`/`.rpm`: replug the device first (the package installs the rule
    and reloads udev). If that does not help, reinstall the package file.
  - AppImage, Flatpak or unknown: install the rule by hand, with the
    commands the rule file documents (download, `install -m 0644` into
    `/etc/udev/rules.d/`, `udevadm control --reload-rules`), then replug.
- `usbPresentHidUnavailable`: replug the device, connect it without a hub
  or dock, or power-cycle the hub. Links to
  [the USB hub troubleshooting page](troubleshooting/usb-hub-resets.md).

The guide shows text and links to copy: the app cannot open a browser.
It never runs a command and never asks for administrator rights. Each guide
is shown once; closing it is remembered in the webview's storage
(`deckpilot.deviceAccessGuide.dismissed`). The device rows keep showing the
state and its hint.

There is no macOS guide. The agent opens a Stream Deck through hidapi's
IOKit backend, outside the App Sandbox. macOS's Input Monitoring rule
refuses keyboard and pointer interfaces only; vendor and consumer
interfaces open without it
(`experiments/distribution/macos-sandbox-hid`), and the GitHub build has
opened a Stream Deck MK.2 on macOS without asking for it. That experiment
had no Stream Deck attached, so this is an inference, not a measurement.
Windows needs no permission: the device uses the built-in HID driver.

### Where vendor code lives

- Elgato protocol types (`elgato_streamdeck`, `hidapi`) are only in
  `deckpilot-hid`, the Elgato adapter. `crates/deckpilot-core/tests/vendor_neutral.rs`
  checks that core does not depend on them or on USB ids.
- The built-in Stream Deck descriptors (and their USB ids, shown in the app)
  stay in the static model registry in `deckpilot-device`: stored layouts
  refer to these model ids, and the configuration app designs layouts for
  them without loading any driver. Discovery by USB id is the adapter's job;
  the generic catalog no longer offers `find_by_usb`.
