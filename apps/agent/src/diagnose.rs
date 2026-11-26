//! `deckpilot-agent diagnose [--json]`: a read-only report for support.
//!
//! The report says which versions are installed and how, whether an agent
//! runs, and which Elgato devices the system sees, so that a problem such
//! as "the Stream Deck is plugged in but DeckPilot does not see it" can be
//! told apart from a permission or HID problem. Its JSON form is versioned
//! ([`SCHEMA`], [`SCHEMA_VERSION`]): fields are only ever added within a
//! version (docs/background-agent.md, "Diagnostics").
//!
//! It is read-only:
//!
//! - it never starts an agent and never connects to one: the agent's state
//!   comes from its instance lock (probed with a shared lock on the existing
//!   file, opened read-only and released at once; a starting agent retries
//!   its lock for `STARTUP_LOCK_WAIT`, and the app's `wait_until_free`
//!   polls, so the moment never makes either give up) and from whether its
//!   socket file exists;
//! - it never opens the database, so it works while the configuration is
//!   locked, and it never reads the IPC token;
//! - on Linux it opens no device: HID interfaces come from
//!   `/sys/class/hidraw`, USB devices from `/sys/bus/usb/devices`, and a
//!   node's permission from `access(2)`. On macOS and Windows `hidapi`
//!   enumerates the HID devices (on Windows by opening each with no access
//!   rights, to read its attributes); no report is read or written;
//! - it creates, writes and resets nothing, and runs no other program.
//!
//! Redacted: serial numbers become a pseudonym (`sha256:` and 16 hex
//! digits of SHA-256 over a fixed prefix, on Linux the machine ID, and the
//! serial): stable on one machine, so two reports can be compared, but a
//! pseudonym, not an irreversible secret (without the machine ID, serials
//! can be guessed and checked against it). The home directory becomes `~`
//! and the user name `<user>` in paths and messages. No environment value,
//! token or configuration value is part of the report.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{self, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

use deckpilot_device::{DeviceBackend, DeviceModel, ModelCatalog};
use deckpilot_ipc::AgentPaths;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The report's schema name.
pub const SCHEMA: &str = "deckpilot.diagnose";
/// The report's schema version. Raised only when a field changes meaning
/// or goes away; new fields keep the version.
pub const SCHEMA_VERSION: u32 = 1;

/// Elgato's USB vendor ID.
const ELGATO_VENDOR: u16 = 0x0fd9;
/// The rule the `.deb` and `.rpm` install, and where a user installs it by
/// hand (AppImage, Flatpak).
const UDEV_RULE: &str = "70-deckpilot-streamdeck.rules";
const UDEV_RULE_DIRS: [&str; 3] = [
    "usr/lib/udev/rules.d",
    "lib/udev/rules.d",
    "etc/udev/rules.d",
];
/// The deb package's file list as dpkg keeps it.
const DPKG_LIST: &str = "var/lib/dpkg/info/deck-pilot.list";
/// What makes serial pseudonyms of this report differ from a plain hash.
const SERIAL_PREFIX: &str = "deckpilot-diagnose-v1:";

/// Where the HID interfaces come from.
#[derive(Debug, Clone)]
pub enum HidSource {
    /// `<root>/sys/class/hidraw` (Linux), linked to the USB devices.
    Sysfs,
    /// A list from `hidapi` (macOS, Windows), not linked to USB devices.
    Listed(Result<Vec<deckpilot_hid::HidInterface>, String>),
}

/// Everything the report reads, so that tests can give it a fixture tree.
#[derive(Debug, Clone)]
pub struct Environment {
    /// The file system root (`/`): `sys`, `dev`, `etc`, `proc`, `usr` and
    /// `var` are read below it.
    pub root: PathBuf,
    pub platform: &'static str,
    pub arch: &'static str,
    pub agent_version: &'static str,
    pub distribution: &'static deckpilot_distribution::Distribution,
    pub paths: AgentPaths,
    /// Whether `DECKPILOT_DATA_DIR` chose the data directory.
    pub data_dir_overridden: bool,
    /// This executable.
    pub exe: Option<PathBuf>,
    /// The AppImage this agent runs from (`deckpilot_ipc::paths::appimage`).
    pub appimage: Option<PathBuf>,
    pub home: Option<PathBuf>,
    pub user: Option<String>,
    /// Linux `/etc/machine-id`: keys the serial pseudonyms. Never reported.
    pub machine_id: Option<String>,
    pub hid: HidSource,
}

impl Environment {
    /// The running system.
    pub fn current() -> io::Result<Self> {
        let paths = AgentPaths::resolve()?;
        let data_dir_overridden = std::env::var_os(deckpilot_ipc::paths::DATA_DIR_ENV)
            .is_some_and(|value| !value.is_empty());
        let hid = if cfg!(target_os = "linux") {
            HidSource::Sysfs
        } else {
            HidSource::Listed(deckpilot_hid::elgato_hid_interfaces())
        };
        Ok(Self {
            root: PathBuf::from("/"),
            platform: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            agent_version: crate::service::VERSION,
            distribution: deckpilot_distribution::current(),
            paths,
            data_dir_overridden,
            exe: std::env::current_exe().ok(),
            appimage: deckpilot_ipc::paths::appimage(),
            home: dirs_home(),
            user: std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .ok()
                .filter(|user| !user.is_empty()),
            machine_id: read_trimmed(Path::new("/etc/machine-id")).filter(|id| !id.is_empty()),
            hid,
        })
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .filter(|home| home.is_absolute() && home.components().count() > 1)
}

/// Removes the home directory and the user name from paths and messages.
#[derive(Debug, Clone)]
struct Redactor {
    home: Option<PathBuf>,
    user: Option<String>,
    /// Paths compare without case (Windows).
    fold_case: bool,
}

impl Redactor {
    fn path(&self, path: &Path) -> String {
        let text = match &self.home {
            Some(home) => match strip_home(path, home, self.fold_case) {
                Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
                Some(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
                None => path.display().to_string(),
            },
            None => path.display().to_string(),
        };
        self.text(&text)
    }

    /// The user name wherever it stands as a word of its own (between
    /// characters that are not letters or digits), in any text.
    fn text(&self, text: &str) -> String {
        let Some(user) = self.user.as_deref().filter(|user| user.chars().count() > 1) else {
            return text.to_owned();
        };
        let fold = |c: char| {
            if self.fold_case {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                c
            }
        };
        let chars: Vec<char> = text.chars().collect();
        let name: Vec<char> = user.chars().map(fold).collect();
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        while i < chars.len() {
            let matches = chars.len() - i >= name.len()
                && chars[i..i + name.len()]
                    .iter()
                    .map(|c| fold(*c))
                    .eq(name.iter().copied())
                && (i == 0 || !chars[i - 1].is_alphanumeric())
                && chars
                    .get(i + name.len())
                    .is_none_or(|c| !c.is_alphanumeric());
            if matches {
                out.push_str("<user>");
                i += name.len();
            } else {
                out.push(chars[i]);
                i += 1;
            }
        }
        out
    }
}

/// `path` below `home`, component by component (so a trailing separator
/// does not matter), without case on Windows.
fn strip_home(path: &Path, home: &Path, fold_case: bool) -> Option<PathBuf> {
    let key = |c: std::path::Component<'_>| {
        let text = c.as_os_str().to_string_lossy().into_owned();
        if fold_case { text.to_lowercase() } else { text }
    };
    let mut rest = path.components();
    for part in home.components() {
        let next = rest.next()?;
        if key(next) != key(part) {
            return None;
        }
    }
    Some(rest.as_path().to_path_buf())
}

/// A stable pseudonym for a serial number: the same device gives the same
/// value in every report, and the serial itself is not in the report.
pub fn serial_pseudonym(serial: &str, machine_id: Option<&str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(SERIAL_PREFIX.as_bytes());
    if let Some(id) = machine_id {
        hasher.update(id.as_bytes());
        hasher.update(b"\0");
    }
    hasher.update(serial.as_bytes());
    let digest = hasher.finalize();
    let mut text = String::from("sha256:");
    for byte in &digest[..8] {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// The report.
pub fn collect(env: &Environment) -> Value {
    let redactor = Redactor {
        home: env.home.clone(),
        user: env.user.clone(),
        fold_case: env.platform == "windows",
    };
    let catalog = ModelCatalog::builtin();
    let driven: Vec<(u16, u16)> = deckpilot_hid::ElgatoBackend::new()
        .models()
        .iter()
        .filter_map(|model| model.usb.map(|usb| (usb.vendor_id, usb.product_id)))
        .collect();
    let devices = devices(env, &catalog, &driven, &redactor);
    json!({
        "schema": SCHEMA,
        "schemaVersion": SCHEMA_VERSION,
        "versions": versions(env),
        "os": os(env),
        "install": install(env, &redactor),
        "ipc": {
            "protocol": deckpilot_ipc::PROTOCOL_VERSION,
            "minimumProtocol": deckpilot_ipc::MIN_PROTOCOL_VERSION,
            "minimumCompatibleAgentProtocol": deckpilot_ipc::MIN_COMPATIBLE_AGENT_PROTOCOL,
        },
        "agent": agent(env, &redactor),
        "udevRules": udev_rules(env),
        "devices": devices,
    })
}

fn versions(env: &Environment) -> Value {
    let next_to_agent = |name: &str| {
        env.exe
            .as_ref()
            .and_then(|exe| exe.parent())
            .is_some_and(|dir| {
                dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
                    .is_file()
            })
    };
    // The app, the agent and the plugin host ship in one package and are
    // replaced together: next to the agent, they are of its version.
    let bundled = |present: bool| {
        json!({
            "present": present,
            "version": present.then_some(env.agent_version),
            // Not read from the file: what ships next to the agent.
            "versionSource": present.then_some("same-package"),
        })
    };
    json!({
        "agent": env.agent_version,
        "desktop": bundled(next_to_agent("deckpilot-desktop")),
        "pluginHost": bundled(next_to_agent("deckpilot-plugin-host")),
    })
}

fn os(env: &Environment) -> Value {
    let name = match env.platform {
        "linux" => read_trimmed(&env.root.join("etc/os-release"))
            .or_else(|| read_trimmed(&env.root.join("usr/lib/os-release")))
            .and_then(|release| {
                release.lines().find_map(|line| {
                    line.strip_prefix("PRETTY_NAME=")
                        .map(|value| value.trim_matches('"').to_owned())
                })
            }),
        "macos" => read_trimmed(
            &env.root
                .join("System/Library/CoreServices/SystemVersion.plist"),
        )
        .and_then(|plist| plist_string(&plist, "ProductVersion"))
        .map(|version| format!("macOS {version}")),
        _ => None,
    };
    let kernel = (env.platform == "linux")
        .then(|| read_trimmed(&env.root.join("proc/sys/kernel/osrelease")))
        .flatten();
    json!({
        "platform": env.platform,
        "arch": env.arch,
        "name": name,
        "kernel": kernel,
    })
}

/// The `<string>` after `<key>name</key>` in a property list.
fn plist_string(plist: &str, key: &str) -> Option<String> {
    let after = plist.split(&format!("<key>{key}</key>")).nth(1)?;
    let value = after.split("<string>").nth(1)?.split("</string>").next()?;
    Some(value.trim().to_owned())
}

fn install(env: &Environment, redactor: &Redactor) -> Value {
    let distribution = env.distribution;
    json!({
        "distributionChannel": distribution.distribution_channel().name(),
        "packageFormat": distribution.package_format().map(deckpilot_distribution::PackageFormat::name),
        "type": install_type(env),
        "executable": env.exe.as_deref().map(|exe| redactor.path(exe)),
    })
}

/// How this agent was installed, from the compiled record or its location.
fn install_type(env: &Environment) -> &'static str {
    let Some(exe) = env.exe.as_deref() else {
        return env
            .distribution
            .package_format()
            .map_or("unknown", deckpilot_distribution::PackageFormat::name);
    };
    // The path as seen from the root (`/` outside tests), without the
    // " (deleted)" Linux adds once an upgrade replaced the running file.
    let text = match exe.strip_prefix(&env.root) {
        Ok(relative) if env.root != Path::new("/") => format!("/{}", relative.display()),
        _ => exe.display().to_string(),
    }
    .replace('\\', "/");
    let text = text.strip_suffix(" (deleted)").unwrap_or(&text).to_owned();
    if text.contains("/target/") {
        return "development";
    }
    if let Some(format) = env.distribution.package_format() {
        return format.name();
    }
    match env.platform {
        "linux" => {
            if env.root.join(".flatpak-info").is_file() {
                // In the sandbox: the os and udev values are the runtime's.
                "flatpak"
            } else if env.appimage.is_some() {
                "appimage"
            } else if read_trimmed(&env.root.join(DPKG_LIST))
                .is_some_and(|list| list.lines().any(|line| line == text))
            {
                "deb"
            } else if text.starts_with("/usr/bin/")
                && ["var/lib/rpm", "usr/lib/sysimage/rpm"]
                    .iter()
                    .any(|db| env.root.join(db).is_dir())
            {
                "rpm"
            } else {
                "unknown"
            }
        }
        "macos" if text.contains(".app/Contents/MacOS/") => "app",
        "windows" => {
            let lower = text.to_lowercase();
            if lower.contains("/appdata/local/") {
                "nsis"
            } else if lower.contains("/program files") {
                "msi"
            } else {
                "unknown"
            }
        }
        _ => "unknown",
    }
}

fn agent(env: &Environment, redactor: &Redactor) -> Value {
    let lock = probe_lock(&env.paths.lock_file);
    let endpoint = env.paths.endpoint();
    let socket = probe_socket(&endpoint);
    let running = match lock {
        "held" => Some(true),
        "free" | "absent" => Some(false),
        _ => None,
    };
    json!({
        "dataDirectory": redactor.path(&env.paths.data_dir),
        "dataDirectoryOverridden": env.data_dir_overridden,
        "dataDirectoryExists": env.paths.data_dir.is_dir(),
        "lock": lock,
        "endpoint": redactor.path(Path::new(&endpoint.display())),
        "endpointFile": socket,
        "running": running,
    })
}

/// Whether an agent holds the instance lock: `held`, `free`, `absent` (no
/// lock file: no agent ever ran with this data directory) or `unknown`.
/// The file is opened read-only, never created, and locked shared for a
/// moment only.
fn probe_lock(path: &Path) -> &'static str {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return "absent",
        Err(_) => return "unknown",
    };
    match file.try_lock_shared() {
        Ok(()) => {
            drop(file);
            "free"
        }
        Err(TryLockError::WouldBlock) => "held",
        Err(TryLockError::Error(_)) => "unknown",
    }
}

/// Whether the agent's socket file exists (Unix). A Windows pipe is not
/// probed: looking it up would connect to it.
fn probe_socket(endpoint: &deckpilot_ipc::Endpoint) -> &'static str {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        match fs::symlink_metadata(endpoint.display()) {
            Ok(metadata) if metadata.file_type().is_socket() => "present",
            Ok(_) => "not-a-socket",
            Err(error) if error.kind() == io::ErrorKind::NotFound => "absent",
            Err(_) => "unknown",
        }
    }
    #[cfg(not(unix))]
    {
        let _ = endpoint;
        "not-probed"
    }
}

fn udev_rules(env: &Environment) -> Value {
    // In a Flatpak the rule directories are the runtime's, not the host's.
    if env.platform != "linux" || env.root.join(".flatpak-info").is_file() {
        return Value::Null;
    }
    Value::Array(
        UDEV_RULE_DIRS
            .iter()
            .map(|dir| {
                let path = Path::new("/").join(dir).join(UDEV_RULE);
                json!({
                    "path": path.display().to_string(),
                    "present": env.root.join(dir).join(UDEV_RULE).is_file(),
                })
            })
            .collect(),
    )
}

/// A USB device of Elgato's vendor ID, from sysfs.
#[derive(Debug, Clone)]
struct UsbDevice {
    /// The sysfs name, the USB port path (`1-2.3`).
    port: String,
    product_id: u16,
    serial: Option<String>,
    /// The `hidraw` nodes below it.
    hidraw: Vec<String>,
}

fn devices(
    env: &Environment,
    catalog: &ModelCatalog,
    driven: &[(u16, u16)],
    redactor: &Redactor,
) -> Value {
    let model_of = |product_id: u16| -> Option<&DeviceModel> {
        catalog.iter().find(|model| {
            model
                .usb
                .is_some_and(|usb| usb.vendor_id == ELGATO_VENDOR && usb.product_id == product_id)
        })
    };
    let known = |product_id: u16| model_of(product_id).is_some();
    let describe = |product_id: u16| {
        let model = model_of(product_id);
        json!({
            "vendorId": hex4(ELGATO_VENDOR),
            "productId": hex4(product_id),
            "model": model.map(|model| model.id.as_str()),
            "name": model.map(|model| model.name.clone()),
            "driver": driven.contains(&(ELGATO_VENDOR, product_id)),
        })
    };
    let mut findings = Vec::new();
    let (source, usb, hid) = match &env.hid {
        HidSource::Sysfs => {
            let (usb, hid) = sysfs_devices(env, &describe, &known, &mut findings);
            ("sysfs", Value::Array(usb), Ok(hid))
        }
        HidSource::Listed(listed) => {
            let hid = listed
                .as_ref()
                .map(|interfaces| {
                    listed_devices(
                        interfaces,
                        env.machine_id.as_deref(),
                        &describe,
                        &known,
                        &mut findings,
                    )
                })
                .map_err(|error| redactor.text(error));
            ("hidapi", Value::Null, hid)
        }
    };
    let (hid, hid_error) = match hid {
        Ok(hid) => (Value::Array(hid), Value::Null),
        Err(error) => (Value::Null, Value::String(error)),
    };
    json!({
        "source": source,
        "usb": usb,
        "hid": hid,
        "hidError": hid_error,
        "findings": findings,
    })
}

/// Linux: the Elgato USB devices with their hidraw nodes, and those nodes
/// as HID interfaces.
fn sysfs_devices(
    env: &Environment,
    describe: &dyn Fn(u16) -> Value,
    known: &dyn Fn(u16) -> bool,
    findings: &mut Vec<Value>,
) -> (Vec<Value>, Vec<Value>) {
    let root = env.root.as_path();
    let mut usb = Vec::new();
    let mut hid = Vec::new();
    for device in usb_devices(root) {
        let serial = device
            .serial
            .as_deref()
            .map(|serial| serial_pseudonym(serial, env.machine_id.as_deref()));
        let nodes: Vec<Value> = device
            .hidraw
            .iter()
            .map(|node| {
                json!({
                    "node": format!("/dev/{node}"),
                    "access": node_access(&root.join("dev").join(node)),
                })
            })
            .collect();
        let state = device_state(&device, known(device.product_id), &nodes);
        if let Some(code) = finding_for(state) {
            findings.push(finding(
                code,
                Some(&device.port),
                device.product_id,
                serial.as_deref(),
            ));
        }
        for node in &device.hidraw {
            let mut interface = describe(device.product_id);
            extend(
                &mut interface,
                json!({ "serial": serial, "node": format!("/dev/{node}") }),
            );
            hid.push(interface);
        }
        let mut entry = describe(device.product_id);
        extend(
            &mut entry,
            json!({
                "port": device.port,
                "serial": serial,
                "hidraw": nodes,
                "state": state,
            }),
        );
        usb.push(entry);
    }
    (usb, hid)
}

/// macOS and Windows: the interfaces hidapi lists.
fn listed_devices(
    interfaces: &[deckpilot_hid::HidInterface],
    machine_id: Option<&str>,
    describe: &dyn Fn(u16) -> Value,
    known: &dyn Fn(u16) -> bool,
    findings: &mut Vec<Value>,
) -> Vec<Value> {
    interfaces
        .iter()
        .filter(|interface| interface.vendor_id == ELGATO_VENDOR)
        .map(|interface| {
            let serial = interface
                .serial
                .as_deref()
                .map(|serial| serial_pseudonym(serial, machine_id));
            if interface.serial.is_none() && known(interface.product_id) {
                findings.push(finding(
                    "hid-without-serial",
                    None,
                    interface.product_id,
                    None,
                ));
            }
            let mut entry = describe(interface.product_id);
            extend(&mut entry, json!({ "serial": serial, "node": Value::Null }));
            entry
        })
        .collect()
}

fn finding(code: &str, port: Option<&str>, product_id: u16, serial: Option<&str>) -> Value {
    json!({
        "code": code,
        "port": port,
        "productId": hex4(product_id),
        "serial": serial,
        "hint": hint(code),
    })
}

fn extend(target: &mut Value, more: Value) {
    if let (Value::Object(target), Value::Object(more)) = (target, more) {
        target.extend(more);
    }
}

fn hex4(id: u16) -> String {
    format!("{id:04x}")
}

/// `ok`; `other-elgato-product` (not in the model catalog: a Cam Link, a
/// microphone, or a Stream Deck model not known yet; not a finding); `usb-present-hid-missing` (no hidraw
/// node: the kernel did not bind the HID interface); `hid-without-serial`
/// (DeckPilot identifies a device by its serial and skips it);
/// `node-missing` (sysfs has the node, `/dev` does not: a container or a
/// sandbox without the device); `no-permission` (a node this user may not
/// open: the udev rule did not apply).
fn device_state(device: &UsbDevice, stream_deck: bool, nodes: &[Value]) -> &'static str {
    if !stream_deck {
        "other-elgato-product"
    } else if device.hidraw.is_empty() {
        "usb-present-hid-missing"
    } else if device.serial.is_none() {
        "hid-without-serial"
    } else if nodes.iter().any(|node| node["access"] == "missing") {
        "node-missing"
    } else if nodes.iter().any(|node| node["access"] == "denied") {
        "no-permission"
    } else {
        "ok"
    }
}

fn finding_for(state: &str) -> Option<&'static str> {
    match state {
        "usb-present-hid-missing" => Some("usb-present-hid-missing"),
        "hid-without-serial" => Some("hid-without-serial"),
        "no-permission" => Some("no-permission"),
        "node-missing" => Some("node-missing"),
        _ => None,
    }
}

fn hint(code: &str) -> &'static str {
    match code {
        "usb-present-hid-missing" => {
            "The USB device is present but has no HID interface, so DeckPilot cannot see it. \
             This happens in the kernel, before any program can use the device: check \
             `dmesg` for errors such as `-28` (not enough bandwidth, often behind a monitor's \
             or dock's hub), then unplug and replug the Stream Deck, or connect it directly to \
             the computer. DeckPilot never resets USB devices \
             (docs/troubleshooting/usb-hub-resets.md)."
        }
        "hid-without-serial" => {
            "The HID interface reports no serial number; DeckPilot identifies devices by \
             their serial and skips this one. Replug the device."
        }
        "node-missing" => {
            "The kernel has a hidraw node for the device, but /dev does not: DeckPilot runs in \
             a container or sandbox without access to the device (a Flatpak needs \
             --device=all), or /dev is not managed by udev."
        }
        "no-permission" => {
            "This user may not open the device's hidraw node: the udev rule is missing or has \
             not been applied. Install the .deb or .rpm (or the rule, for the AppImage), \
             then replug the device."
        }
        _ => "",
    }
}

/// `read-write`, `denied` or `missing` (no node in /dev): from
/// `access(2)`, which opens nothing.
fn node_access(node: &Path) -> &'static str {
    #[cfg(unix)]
    {
        use rustix::fs::{Access, access};
        if !node.exists() {
            return "missing";
        }
        match access(node, Access::READ_OK | Access::WRITE_OK) {
            Ok(()) => "read-write",
            Err(_) => "denied",
        }
    }
    #[cfg(not(unix))]
    {
        let _ = node;
        "unknown"
    }
}

/// The USB devices of Elgato's vendor ID below `<root>/sys/bus/usb/devices`,
/// each with the `hidraw` nodes of `<root>/sys/class/hidraw` that belong to
/// it, sorted by port.
fn usb_devices(root: &Path) -> Vec<UsbDevice> {
    let hidraw = hidraw_by_usb_device(root);
    let mut devices = Vec::new();
    let Ok(entries) = fs::read_dir(root.join("sys/bus/usb/devices")) else {
        return devices;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let Some(vendor) = read_hex(&dir.join("idVendor")) else {
            continue;
        };
        if vendor != ELGATO_VENDOR {
            continue;
        }
        let Some(product_id) = read_hex(&dir.join("idProduct")) else {
            continue;
        };
        let canonical = fs::canonicalize(&dir).unwrap_or(dir.clone());
        devices.push(UsbDevice {
            port: entry.file_name().to_string_lossy().into_owned(),
            product_id,
            serial: read_trimmed(&dir.join("serial")).filter(|serial| !serial.is_empty()),
            hidraw: hidraw.get(&canonical).cloned().unwrap_or_default(),
        });
    }
    devices.sort_by(|a, b| a.port.cmp(&b.port));
    devices
}

/// `hidraw` node names, by the USB device directory (canonical) above them.
fn hidraw_by_usb_device(root: &Path) -> BTreeMap<PathBuf, Vec<String>> {
    let mut map: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    let Ok(entries) = fs::read_dir(root.join("sys/class/hidraw")) else {
        return map;
    };
    let sys = fs::canonicalize(root.join("sys")).unwrap_or_else(|_| root.join("sys"));
    for entry in entries.flatten() {
        let Ok(node) = fs::canonicalize(entry.path()) else {
            continue;
        };
        // The first directory above the node with an idVendor is its USB
        // device (the interface and the HID device in between have none).
        let usb = node
            .ancestors()
            .skip(1)
            .take_while(|dir| dir.starts_with(&sys) && *dir != sys)
            .find(|dir| dir.join("idVendor").is_file());
        if let Some(usb) = usb {
            map.entry(usb.to_path_buf())
                .or_default()
                .push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    for nodes in map.values_mut() {
        nodes.sort();
    }
    map
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_owned())
}

fn read_hex(path: &Path) -> Option<u16> {
    u16::from_str_radix(&read_trimmed(path)?, 16).ok()
}

/// A few lines for a person; the JSON form has everything.
pub fn summary(report: &Value) -> String {
    let mut lines = vec![
        format!(
            "DeckPilot agent {} ({} on {} {})",
            report["versions"]["agent"].as_str().unwrap_or("?"),
            report["install"]["type"].as_str().unwrap_or("?"),
            report["os"]["platform"].as_str().unwrap_or("?"),
            report["os"]["arch"].as_str().unwrap_or("?"),
        ),
        format!(
            "agent running: {} (lock {}, endpoint {})",
            match report["agent"]["running"].as_bool() {
                Some(true) => "yes",
                Some(false) => "no",
                None => "unknown",
            },
            report["agent"]["lock"].as_str().unwrap_or("?"),
            report["agent"]["endpointFile"].as_str().unwrap_or("?"),
        ),
    ];
    if let Some(rules) = report["udevRules"].as_array() {
        let present: Vec<&str> = rules
            .iter()
            .filter(|rule| rule["present"] == true)
            .filter_map(|rule| rule["path"].as_str())
            .collect();
        lines.push(if present.is_empty() {
            "udev rule: not installed".to_owned()
        } else {
            format!("udev rule: {}", present.join(", "))
        });
    }
    let devices = &report["devices"];
    let count = |key: &str| {
        devices[key]
            .as_array()
            .map_or_else(|| "n/a".to_owned(), |list| list.len().to_string())
    };
    lines.push(format!(
        "Elgato devices: {} USB, {} HID ({})",
        count("usb"),
        count("hid"),
        devices["source"].as_str().unwrap_or("?"),
    ));
    for finding in devices["findings"].as_array().into_iter().flatten() {
        lines.push(format!(
            "! {} (product {}, port {}): {}",
            finding["code"].as_str().unwrap_or("?"),
            finding["productId"].as_str().unwrap_or("?"),
            finding["port"].as_str().unwrap_or("-"),
            finding["hint"].as_str().unwrap_or(""),
        ));
    }
    lines.push("Run with --json for the full report.".to_owned());
    lines.join("\n")
}

#[cfg(test)]
#[path = "diagnose/tests.rs"]
mod tests;
