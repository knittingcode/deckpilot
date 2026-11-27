use super::*;
use deckpilot_distribution::{Architecture, Distribution, DistributionChannel, Platform};
use std::collections::BTreeSet;

/// A GitHub build for Linux: its package format comes from the bundler,
/// so `install.type` is found from the files.
static GITHUB_LINUX: Distribution = match Distribution::new(
    DistributionChannel::Github,
    None,
    "0.15.0",
    Platform::Linux,
    Architecture::X86_64,
) {
    Ok(distribution) => distribution,
    Err(_) => panic!("valid record"),
};

const SERIAL_OK: &str = "CL12K1A00042";
#[cfg(unix)]
const MACHINE_ID: &str = "0123456789abcdef0123456789abcdef";
#[cfg(unix)]
const SERIAL_NO_HID: &str = "CL12K1A00043";
#[cfg(unix)]
const TOKEN: &str = "fixture-agent-token-3f9a";

/// Whether `value` has the type `schema` gives (the format of
/// `schema-v1.json`: type names with `|` alternatives, `[x]` arrays,
/// `{"$nullable": x}`, and objects with exactly the listed keys).
fn conforms(schema: &Value, value: &Value, at: &str) -> Result<(), String> {
    let type_name = |value: &Value| match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    match schema {
        Value::String(types) => {
            if types.split('|').any(|t| t == type_name(value)) {
                Ok(())
            } else {
                Err(format!("{at}: {} is not {types}", type_name(value)))
            }
        }
        Value::Array(element) => {
            let items = value
                .as_array()
                .ok_or_else(|| format!("{at}: {} is not an array", type_name(value)))?;
            items.iter().enumerate().try_for_each(|(i, item)| {
                let element = element
                    .first()
                    .ok_or_else(|| format!("{at}: empty array schema"))?;
                conforms(element, item, &format!("{at}[{i}]"))
            })
        }
        Value::Object(fields) if fields.contains_key("$nullable") => {
            if value.is_null() {
                Ok(())
            } else {
                conforms(&fields["$nullable"], value, at)
            }
        }
        Value::Object(fields) => {
            let object = value
                .as_object()
                .ok_or_else(|| format!("{at}: {} is not an object", type_name(value)))?;
            let expected: BTreeSet<&String> =
                fields.keys().filter(|key| !key.starts_with('$')).collect();
            let actual: BTreeSet<&String> = object.keys().collect();
            if expected != actual {
                return Err(format!(
                    "{at}: keys {actual:?}, the schema has {expected:?}"
                ));
            }
            expected
                .iter()
                .try_for_each(|key| conforms(&fields[*key], &object[*key], &format!("{at}.{key}")))
        }
        _ => Err(format!("{at}: bad schema {schema}")),
    }
}

fn schema_v1() -> Value {
    serde_json::from_str(include_str!("schema-v1.json")).unwrap()
}

#[cfg(unix)]
fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[cfg(unix)]
fn link(target: &str, link: &Path) {
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(target, link).unwrap();
}

/// A USB device directory under `sys/devices/usb1` with its attributes.
#[cfg(unix)]
fn usb_device(root: &Path, port: &str, vendor: &str, product: &str, serial: Option<&str>) {
    let dir = root.join("sys/devices/pci0000:00/usb1").join(port);
    write(&dir.join("idVendor"), &format!("{vendor}\n"));
    write(&dir.join("idProduct"), &format!("{product}\n"));
    if let Some(serial) = serial {
        write(&dir.join("serial"), &format!("{serial}\n"));
    }
    link(
        &format!("../../../devices/pci0000:00/usb1/{port}"),
        &root.join("sys/bus/usb/devices").join(port),
    );
}

/// A hidraw node below a USB device's first interface, as the kernel lays
/// it out, with its class link and its /dev node.
#[cfg(unix)]
fn hidraw(root: &Path, port: &str, hid_id: &str, node: &str) {
    let relative = format!("pci0000:00/usb1/{port}/{port}:1.0/{hid_id}/hidraw/{node}");
    let dir = root.join("sys/devices").join(&relative);
    write(&dir.join("dev"), "247:0\n");
    link(
        &format!("../../devices/{relative}"),
        &root.join("sys/class/hidraw").join(node),
    );
    write(&root.join("dev").join(node), "");
}

/// A Linux system: an MK.2 with its HID interface (1-2), an MK.2 whose HID
/// interface is missing (1-3, the DP-USB-03 signature), a Stream Deck XL
/// (descriptor only, no driver) with HID (1-5), another vendor's device
/// with HID (1-4), the udev rule from the package, and a data directory
/// with a token and a database that must stay untouched.
#[cfg(unix)]
fn linux_fixture(root: &Path) -> Environment {
    usb_device(root, "1-2", "0fd9", "0080", Some(SERIAL_OK));
    hidraw(root, "1-2", "0003:0FD9:0080.0001", "hidraw3");
    usb_device(root, "1-3", "0fd9", "0080", Some(SERIAL_NO_HID));
    // The interface exists, the HID device does not (configuration failed).
    fs::create_dir_all(root.join("sys/devices/pci0000:00/usb1/1-3/1-3:1.0")).unwrap();
    usb_device(root, "1-4", "046d", "c52b", Some("LOGITECH1"));
    hidraw(root, "1-4", "0003:046D:C52B.0002", "hidraw0");
    usb_device(root, "1-5", "0fd9", "006c", Some("XL0001"));
    hidraw(root, "1-5", "0003:0FD9:006C.0003", "hidraw5");
    // An Elgato product that is not a Stream Deck (Cam Link 4K): no HID.
    usb_device(root, "1-6", "0fd9", "0066", Some("CAMLINK1"));
    // A root hub: no idVendor of Elgato.
    usb_device(root, "usb1", "1d6b", "0002", None);
    write(
        &root.join("usr/lib/udev/rules.d").join(UDEV_RULE),
        "# rule\n",
    );
    write(
        &root.join("etc/os-release"),
        "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\n",
    );
    write(
        &root.join("proc/sys/kernel/osrelease"),
        "6.8.0-49-generic\n",
    );
    write(
        &root.join(DPKG_LIST),
        "/.\n/usr\n/usr/bin\n/usr/bin/deckpilot-agent\n",
    );
    write(&root.join("usr/bin/deckpilot-agent"), "");
    write(&root.join("usr/bin/deckpilot-desktop"), "");
    let home = root.join("home/alice");
    let data = home.join(".local/share/io.github.knittingcode.deckpilot");
    write(&data.join("agent.token"), TOKEN);
    write(&data.join("deckpilot.db"), "SQLite format 3\0encrypted");
    Environment {
        root: root.to_path_buf(),
        platform: "linux",
        arch: "x86_64",
        agent_version: "0.15.0",
        distribution: &GITHUB_LINUX,
        paths: AgentPaths::in_dir(data),
        data_dir_overridden: false,
        exe: Some(root.join("usr/bin/deckpilot-agent")),
        appimage: None,
        home: Some(home),
        user: Some("alice".into()),
        machine_id: Some(MACHINE_ID.into()),
        hostname: Some("alice-laptop".into()),
        hid: HidSource::Sysfs,
    }
}

fn root() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// The report has the schema of its version, `diagnose/schema-v1.json`.
/// A new field is added there (and keeps the version); a field that
/// changes type or goes away needs a new `SCHEMA_VERSION`.
#[cfg(unix)]
#[test]
fn the_report_keeps_the_schema_of_its_version() {
    let root = root();
    let mut env = linux_fixture(root.path());
    let report = collect(&env);
    assert_eq!(report["schema"], SCHEMA);
    assert_eq!(report["schemaVersion"], 1);
    assert_eq!(
        SCHEMA_VERSION, 1,
        "a new schema version needs a new schema file"
    );
    conforms(&schema_v1(), &report, "report").unwrap();
    // With nothing found, the optional values are null or empty.
    let empty = tempfile::tempdir().unwrap();
    env.root = empty.path().to_path_buf();
    env.exe = None;
    conforms(&schema_v1(), &collect(&env), "empty report").unwrap();
}

#[test]
fn the_schema_check_refuses_changes() {
    let schema =
        json!({ "a": "string|null", "b": ["number"], "c": { "$nullable": { "d": "boolean" } } });
    conforms(&schema, &json!({ "a": null, "b": [1, 2], "c": null }), "v").unwrap();
    conforms(
        &schema,
        &json!({ "a": "x", "b": [], "c": { "d": true } }),
        "v",
    )
    .unwrap();
    for wrong in [
        json!({ "a": 1, "b": [], "c": null }),
        json!({ "a": "x", "b": ["1"], "c": null }),
        json!({ "a": "x", "b": [], "c": { "d": true, "e": 1 } }),
        json!({ "a": "x", "b": [] }),
    ] {
        assert!(conforms(&schema, &wrong, "v").is_err(), "{wrong}");
    }
}

#[cfg(unix)]
#[test]
fn usb_present_hid_missing_is_detected_for_supported_models() {
    let root = root();
    let report = collect(&linux_fixture(root.path()));
    let devices = &report["devices"];
    assert_eq!(devices["source"], "sysfs");
    let usb = devices["usb"].as_array().unwrap();
    // Elgato only, sorted by port: not the root hub, not the other vendor.
    let ports: Vec<&str> = usb.iter().map(|d| d["port"].as_str().unwrap()).collect();
    assert_eq!(ports, ["1-2", "1-3", "1-5", "1-6"]);

    assert_eq!(usb[0]["state"], "ok");
    assert_eq!(usb[0]["model"], "elgato.streamdeck.mk2");
    assert_eq!(usb[0]["driver"], true);
    assert_eq!(usb[0]["hidraw"][0]["node"], "/dev/hidraw3");
    assert_eq!(usb[0]["hidraw"][0]["access"], "read-write");

    assert_eq!(usb[1]["state"], "usb-present-hid-missing");
    assert_eq!(usb[1]["hidraw"], json!([]));
    assert!(
        usb[1]["serial"] == serial_pseudonym(SERIAL_NO_HID, Some(MACHINE_ID)),
        "port 1-3: the serial is not its pseudonym"
    );

    // XL: seen, no driver.
    assert_eq!(usb[2]["model"], "elgato.streamdeck.xl");
    assert_eq!(usb[2]["driver"], false);
    assert_eq!(usb[2]["state"], "ok");

    // Cam Link: not a Stream Deck, so no HID is expected and nothing is
    // reported about it.
    assert_eq!(usb[3]["model"], Value::Null);
    assert_eq!(usb[3]["state"], "other-elgato-product");

    let findings = devices["findings"].as_array().unwrap();
    let codes: Vec<&str> = findings.iter().filter_map(|f| f["code"].as_str()).collect();
    assert_eq!(codes, ["usb-present-hid-missing"]);
    assert_eq!(findings[0]["code"], "usb-present-hid-missing");
    assert_eq!(findings[0]["port"], "1-3");
    assert_eq!(findings[0]["productId"], "0080");
    assert!(findings[0]["hint"].as_str().unwrap().contains("dmesg"));

    // HID interfaces: those of Elgato's devices only.
    let hid: Vec<&str> = devices["hid"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["node"].as_str().unwrap())
        .collect();
    assert_eq!(hid, ["/dev/hidraw3", "/dev/hidraw5"]);
}

#[cfg(unix)]
#[test]
fn a_hid_node_without_serial_or_permission_is_reported() {
    let root = root();
    let env = linux_fixture(root.path());
    fs::remove_file(root.path().join("sys/devices/pci0000:00/usb1/1-2/serial")).unwrap();
    fs::remove_file(root.path().join("dev/hidraw5")).unwrap();
    let report = collect(&env);
    let usb = report["devices"]["usb"].as_array().unwrap();
    assert_eq!(usb[0]["state"], "hid-without-serial");
    assert_eq!(usb[0]["serial"], Value::Null);
    assert_eq!(usb[2]["hidraw"][0]["access"], "missing");
    assert_eq!(usb[2]["state"], "node-missing");
    let codes: Vec<&str> = report["devices"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        [
            "hid-without-serial",
            "usb-present-hid-missing",
            "node-missing"
        ]
    );
}

/// A node this user may not open (no ACL: the udev rule did not apply).
/// Root may open anything, so the check needs an ordinary user.
#[cfg(unix)]
#[test]
fn a_node_without_access_is_no_permission() {
    use std::os::unix::fs::PermissionsExt;
    let root = root();
    let env = linux_fixture(root.path());
    let node = root.path().join("dev/hidraw3");
    fs::set_permissions(&node, fs::Permissions::from_mode(0o000)).unwrap();
    if rustix::fs::access(&node, rustix::fs::Access::READ_OK).is_ok() {
        eprintln!("skipped: this user (root?) may open any file");
        return;
    }
    let report = collect(&env);
    assert_eq!(report["devices"]["usb"][0]["hidraw"][0]["access"], "denied");
    assert_eq!(report["devices"]["usb"][0]["state"], "no-permission");
    assert!(
        report["devices"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["code"] == "no-permission" && f["port"] == "1-2")
    );
}

#[cfg(unix)]
#[test]
fn no_elgato_device_and_no_rule() {
    let root = root();
    let mut env = linux_fixture(root.path());
    let empty = tempfile::tempdir().unwrap();
    env.root = empty.path().to_path_buf();
    let report = collect(&env);
    assert_eq!(report["devices"]["usb"], json!([]));
    assert_eq!(report["devices"]["hid"], json!([]));
    assert_eq!(report["devices"]["findings"], json!([]));
    assert!(
        report["udevRules"]
            .as_array()
            .unwrap()
            .iter()
            .all(|rule| rule["present"] == false)
    );
    assert_eq!(report["os"]["name"], Value::Null);
}

#[cfg(unix)]
#[test]
fn serials_home_user_and_token_are_redacted() {
    let root = root();
    let mut env = linux_fixture(root.path());
    // A path with the user's name outside the home directory.
    env.exe = Some(root.path().join("opt/alice/deckpilot-agent"));
    let report = collect(&env);
    let text = serde_json::to_string(&report).unwrap();
    // The failure message names the case, never the value or the report.
    for (label, secret) in [
        ("serial of the MK.2 on 1-2", SERIAL_OK),
        ("serial of the MK.2 on 1-3", SERIAL_NO_HID),
        ("serial of the XL", "XL0001"),
        ("serial of the Cam Link", "CAMLINK1"),
        ("IPC token", TOKEN),
        ("machine ID", MACHINE_ID),
        ("home directory", "/home/alice"),
        ("user name", "alice"),
    ] {
        assert!(!text.contains(secret), "the report contains the {label}");
    }
    assert!(
        report["agent"]["dataDirectory"] == "~/.local/share/io.github.knittingcode.deckpilot",
        "the data directory is not shown below ~"
    );
    assert!(
        report["install"]["executable"]
            .as_str()
            .unwrap()
            .ends_with("/opt/<user>/deckpilot-agent")
    );
    // The pseudonym is stable and not the plain hash of the serial.
    let pseudonym = serial_pseudonym(SERIAL_OK, Some(MACHINE_ID));
    assert!(
        report["devices"]["usb"][0]["serial"] == pseudonym,
        "port 1-2: the serial is not its pseudonym"
    );
    assert!(
        pseudonym == serial_pseudonym(SERIAL_OK, Some(MACHINE_ID)),
        "the pseudonym is not stable"
    );
    assert!(
        pseudonym != serial_pseudonym(SERIAL_NO_HID, Some(MACHINE_ID)),
        "two serials share a pseudonym"
    );
    // Another machine, or none: another pseudonym.
    assert!(
        pseudonym != serial_pseudonym(SERIAL_OK, None),
        "the machine ID does not change the pseudonym"
    );
    assert!(
        pseudonym != serial_pseudonym(SERIAL_OK, Some("fedcba98")),
        "another machine ID gives the same pseudonym"
    );
    assert!(
        pseudonym.starts_with("sha256:") && pseudonym.len() == "sha256:".len() + 16,
        "the pseudonym is not sha256: and 16 hex digits"
    );
    let plain =
        Sha256::digest(SERIAL_OK.as_bytes())[..8]
            .iter()
            .fold(String::new(), |mut text, byte| {
                let _ = write!(text, "{byte:02x}");
                text
            });
    assert!(
        !pseudonym.contains(&plain),
        "the pseudonym is the plain hash of the serial"
    );
}

#[test]
fn redaction_keeps_other_paths_and_partial_names() {
    let redactor = Redactor {
        home: Some("/home/al/".into()),
        user: Some("al".into()),
        fold_case: false,
    };
    // A trailing separator of the home directory does not matter.
    assert_eq!(redactor.path(Path::new("/home/al")), "~");
    assert_eq!(
        redactor.path(Path::new("/home/al/x")),
        format!("~{}x", std::path::MAIN_SEPARATOR)
    );
    // Not the home directory, only a prefix of its name.
    assert_eq!(redactor.path(Path::new("/home/also/x")), "/home/also/x");
    assert_eq!(redactor.path(Path::new("/srv/al/x")), "/srv/<user>/x");
    // The name as a word of its own, also inside a segment.
    assert_eq!(
        redactor.path(Path::new("/srv/al-deckpilot/x")),
        "/srv/<user>-deckpilot/x"
    );
    assert_eq!(redactor.path(Path::new("/srv/also/x")), "/srv/also/x");
    assert_eq!(redactor.path(Path::new("/usr/local/bin")), "/usr/local/bin");
    assert_eq!(
        redactor.text("pipe deckpilot-al-1"),
        "pipe deckpilot-<user>-1"
    );
    // Windows: paths compare without case.
    let windows = Redactor {
        home: Some(PathBuf::from("/Users/Al")),
        user: Some("Al".into()),
        fold_case: true,
    };
    assert_eq!(windows.path(Path::new("/users/al")), "~");
    assert_eq!(windows.text("C:/USERS/AL/x"), "C:/USERS/<user>/x");
}

#[cfg(unix)]
#[test]
fn install_type_and_versions_from_the_files() {
    let root = root();
    let mut env = linux_fixture(root.path());
    let report = collect(&env);
    assert_eq!(report["install"]["type"], "deb");
    assert_eq!(report["install"]["distributionChannel"], "github");
    assert_eq!(report["install"]["packageFormat"], Value::Null);
    assert_eq!(report["versions"]["agent"], "0.15.0");
    assert_eq!(
        report["versions"]["desktop"],
        json!({ "present": true, "version": "0.15.0", "versionSource": "same-package" })
    );
    assert_eq!(
        report["versions"]["pluginHost"],
        json!({ "present": false, "version": null, "versionSource": null })
    );
    assert_eq!(report["os"]["name"], "Ubuntu 24.04.1 LTS");
    assert_eq!(report["os"]["kernel"], "6.8.0-49-generic");
    assert_eq!(report["ipc"]["protocol"], deckpilot_ipc::PROTOCOL_VERSION);

    env.appimage = Some(PathBuf::from("/home/alice/DeckPilot.AppImage"));
    assert_eq!(install_type(&env), "appimage");
    env.appimage = None;
    fs::remove_file(root.path().join(DPKG_LIST)).unwrap();
    assert_eq!(install_type(&env), "unknown");
    fs::create_dir_all(root.path().join("var/lib/rpm")).unwrap();
    assert_eq!(install_type(&env), "rpm");
    // After an upgrade replaced the running file.
    env.exe = Some(root.path().join("usr/bin/deckpilot-agent (deleted)"));
    assert_eq!(install_type(&env), "rpm");
    write(&root.path().join(".flatpak-info"), "[Application]\n");
    assert_eq!(install_type(&env), "flatpak");
    env.exe = Some(PathBuf::from("/src/deckpilot/target/debug/deckpilot-agent"));
    assert_eq!(install_type(&env), "development");
    env.exe = None;
    assert_eq!(install_type(&env), "unknown");
}

/// The agent's state from its lock, which is probed read-only: a held lock
/// is reported as running, the lock file is never created, and the
/// database and token are not touched.
#[cfg(unix)]
#[test]
fn the_agent_state_comes_from_the_lock_without_touching_anything() {
    let root = root();
    let env = linux_fixture(root.path());
    let data = env.paths.data_dir.clone();
    let before = fs::read(data.join("deckpilot.db")).unwrap();

    let report = collect(&env);
    assert_eq!(report["agent"]["lock"], "absent");
    assert_eq!(report["agent"]["running"], false);
    assert!(
        !env.paths.lock_file.exists(),
        "the probe created the lock file"
    );

    let lock = crate::InstanceLock::acquire(&env.paths.lock_file).unwrap();
    let report = collect(&env);
    assert_eq!(report["agent"]["lock"], "held");
    assert_eq!(report["agent"]["running"], true);
    // The running agent still holds its lock afterwards.
    assert!(matches!(
        crate::InstanceLock::acquire(&env.paths.lock_file),
        Err(crate::LockError::AlreadyRunning)
    ));
    drop(lock);

    let report = collect(&env);
    assert_eq!(report["agent"]["lock"], "free");
    assert_eq!(report["agent"]["running"], false);
    assert!(
        fs::read(data.join("deckpilot.db")).unwrap() == before,
        "the database changed"
    );
    assert!(
        fs::read_to_string(data.join("agent.token")).unwrap() == TOKEN,
        "the token file changed"
    );
}

/// macOS and Windows: HID interfaces from hidapi's list, no USB view.
#[test]
fn listed_hid_interfaces_without_usb() {
    let root = root();
    let data = root.path().join("data");
    let env = Environment {
        root: root.path().to_path_buf(),
        platform: "macos",
        arch: "aarch64",
        agent_version: "0.15.0",
        distribution: &GITHUB_LINUX,
        paths: AgentPaths::in_dir(data),
        data_dir_overridden: true,
        exe: Some(PathBuf::from(
            "/Applications/DeckPilot.app/Contents/MacOS/deckpilot-agent",
        )),
        appimage: None,
        home: None,
        user: None,
        machine_id: None,
        hostname: None,
        hid: HidSource::Listed(Ok(vec![
            deckpilot_hid::HidInterface {
                vendor_id: 0x0fd9,
                product_id: 0x0080,
                serial: Some(SERIAL_OK.into()),
            },
            deckpilot_hid::HidInterface {
                vendor_id: 0x0fd9,
                product_id: 0x0080,
                serial: None,
            },
            // Not a Stream Deck: listed, never a finding.
            deckpilot_hid::HidInterface {
                vendor_id: 0x0fd9,
                product_id: 0x0070,
                serial: None,
            },
        ])),
    };
    let report = collect(&env);
    conforms(&schema_v1(), &report, "hidapi report").unwrap();
    assert_eq!(report["devices"]["source"], "hidapi");
    assert_eq!(report["devices"]["usb"], Value::Null);
    assert_eq!(report["udevRules"], Value::Null);
    let hid = report["devices"]["hid"].as_array().unwrap();
    assert_eq!(hid.len(), 3);
    assert!(
        hid[0]["serial"] == serial_pseudonym(SERIAL_OK, None),
        "the first interface's serial is not its pseudonym"
    );
    assert_eq!(hid[2]["model"], Value::Null);
    assert_eq!(report["devices"]["findings"].as_array().unwrap().len(), 1);
    assert_eq!(
        report["devices"]["findings"][0]["code"],
        "hid-without-serial"
    );
    assert_eq!(report["agent"]["dataDirectoryOverridden"], true);
    assert_eq!(report["agent"]["dataDirectoryExists"], false);
    assert!(
        !root.path().join("data").exists(),
        "the report created the data directory"
    );

    let failed = Environment {
        hid: HidSource::Listed(Err("cannot open /Users/bob/x".into())),
        home: Some(PathBuf::from("/Users/bob")),
        user: Some("bob".into()),
        ..env
    };
    let report = collect(&failed);
    conforms(&schema_v1(), &report, "failed hidapi report").unwrap();
    assert_eq!(report["devices"]["hid"], Value::Null);
    assert!(
        report["devices"]["hidError"] == "cannot open /Users/<user>/x",
        "the hidapi error is not redacted"
    );
}

#[cfg(unix)]
#[test]
fn the_summary_names_the_finding() {
    let root = root();
    let report = collect(&linux_fixture(root.path()));
    let text = summary(&report);
    // The failure message names the line, never the summary itself.
    for (label, line) in [
        (
            "version and install type",
            "DeckPilot agent 0.15.0 (deb on linux x86_64)",
        ),
        ("agent state", "agent running: no"),
        ("device counts", "Elgato devices: 4 USB, 2 HID (sysfs)"),
        (
            "finding",
            "! usb-present-hid-missing (product 0080, port 1-3)",
        ),
    ] {
        assert!(text.contains(line), "the summary lacks the {label} line");
    }
}

/// Every field of schema version 1 as it was published: path and type. The
/// schema file may gain fields (they keep the version), but none of these
/// may go away or change type without a new `SCHEMA_VERSION`; this list is
/// never edited for version 1.
const SCHEMA_V1_BASELINE: &[&str] = &[
    "agent.dataDirectory: string",
    "agent.dataDirectoryExists: boolean",
    "agent.dataDirectoryOverridden: boolean",
    "agent.endpoint: string",
    "agent.endpointFile: string",
    "agent.lock: string",
    "agent.running: boolean|null",
    "devices.findings[].code: string",
    "devices.findings[].hint: string",
    "devices.findings[].port: string|null",
    "devices.findings[].productId: string",
    "devices.findings[].serial: string|null",
    "devices.hid?[].driver: boolean",
    "devices.hid?[].model: string|null",
    "devices.hid?[].name: string|null",
    "devices.hid?[].node: string|null",
    "devices.hid?[].productId: string",
    "devices.hid?[].serial: string|null",
    "devices.hid?[].vendorId: string",
    "devices.hidError: string|null",
    "devices.source: string",
    "devices.usb?[].driver: boolean",
    "devices.usb?[].hidraw[].access: string",
    "devices.usb?[].hidraw[].node: string",
    "devices.usb?[].model: string|null",
    "devices.usb?[].name: string|null",
    "devices.usb?[].port: string",
    "devices.usb?[].productId: string",
    "devices.usb?[].serial: string|null",
    "devices.usb?[].state: string",
    "devices.usb?[].vendorId: string",
    "install.distributionChannel: string",
    "install.executable: string|null",
    "install.packageFormat: string|null",
    "install.type: string",
    "ipc.minimumCompatibleAgentProtocol: number",
    "ipc.minimumProtocol: number",
    "ipc.protocol: number",
    "os.arch: string",
    "os.kernel: string|null",
    "os.name: string|null",
    "os.platform: string",
    "schema: string",
    "schemaVersion: number",
    "udevRules?[].path: string",
    "udevRules?[].present: boolean",
    "versions.agent: string",
    "versions.desktop.present: boolean",
    "versions.desktop.version: string|null",
    "versions.desktop.versionSource: string|null",
    "versions.pluginHost.present: boolean",
    "versions.pluginHost.version: string|null",
    "versions.pluginHost.versionSource: string|null",
];

/// The schema file's fields as `path: type` lines (`?` marks a nullable
/// array or object, `[]` an array's elements).
fn schema_fields(schema: &Value, at: &str, out: &mut BTreeSet<String>) {
    match schema {
        Value::String(types) => {
            out.insert(format!("{at}: {types}"));
        }
        Value::Array(element) => {
            if let Some(element) = element.first() {
                schema_fields(element, &format!("{at}[]"), out);
            }
        }
        Value::Object(fields) if fields.contains_key("$nullable") => {
            schema_fields(&fields["$nullable"], &format!("{at}?"), out);
        }
        Value::Object(fields) => {
            for (key, value) in fields.iter().filter(|(key, _)| !key.starts_with('$')) {
                let path = if at.is_empty() {
                    key.clone()
                } else {
                    format!("{at}.{key}")
                };
                schema_fields(value, &path, out);
            }
        }
        _ => {}
    }
}

#[test]
fn the_schema_file_keeps_every_field_of_version_1() {
    let mut fields = BTreeSet::new();
    schema_fields(&schema_v1(), "", &mut fields);
    let missing: Vec<&&str> = SCHEMA_V1_BASELINE
        .iter()
        .filter(|field| !fields.contains(**field))
        .collect();
    assert!(
        missing.is_empty(),
        "schema-v1.json lost or retyped fields of version 1: {missing:?}"
    );
}

/// What `probe_lock` says once copies of a released lock's descriptor are
/// gone: a program another test of this process is starting holds a copy of
/// every descriptor until it executes (close-on-exec, see `InstanceLock`),
/// so a lock just released can still read as held for a moment.
#[cfg(unix)]
fn probe_until(path: &Path, want: &str) -> &'static str {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let seen = probe_lock(path);
        if seen == want || std::time::Instant::now() >= deadline {
            return seen;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Probing the lock while an agent starts never makes it give up: a probe
/// leaves nothing held, does not block another probe, and a probe that
/// holds the lock while an agent starts is waited out (the agent retries
/// for `STARTUP_LOCK_WAIT`).
///
/// Shown by the order of events, not by racing a prober that probes
/// without pause against the agent: such a prober holds the lock most of
/// the time, and a loaded machine that preempts it while it holds the lock
/// made every retry of the agent collide with it.
#[cfg(unix)]
#[test]
fn probing_while_an_agent_starts_never_stops_it() {
    let root = root();
    let env = linux_fixture(root.path());
    let path = env.paths.lock_file.clone();
    // A previous agent left its lock file behind (the probe never creates it).
    fs::write(&path, "").unwrap();

    // A probe holds nothing once it returns: after any number of probes the
    // agent gets its lock (a probe that kept it would make the agent wait in
    // vain); while the agent holds it, a probe sees it held.
    for _ in 0..20 {
        for _ in 0..10 {
            assert_eq!(probe_until(&path, "free"), "free");
        }
        let lock = crate::InstanceLock::acquire_waiting(&path, crate::STARTUP_LOCK_WAIT)
            .expect("a probe that returned holds nothing");
        assert_eq!(probe_lock(&path), "held");
        drop(lock);
    }

    // A probe in progress (the shared lock `probe_lock` takes, on the same
    // file) does not block another probe, but does block an agent for that
    // moment; the agent waits and gets the lock once the probe ends.
    let probe = fs::File::open(&path).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match probe.try_lock_shared() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) => {
                assert!(std::time::Instant::now() < deadline, "the lock stayed held");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(std::fs::TryLockError::Error(error)) => panic!("cannot lock the file: {error}"),
        }
    }
    assert_eq!(
        probe_lock(&path),
        "free",
        "a probe does not block another probe"
    );
    assert!(
        matches!(
            crate::InstanceLock::acquire(&path),
            Err(crate::LockError::AlreadyRunning)
        ),
        "a probe in progress holds the lock"
    );
    // The probe ends while the agent is waiting. The pause only decides
    // whether the agent's first attempt meets the probe (and so has to
    // retry); a correct agent gets the lock either way.
    let end_probe = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        drop(probe);
    });
    let lock = crate::InstanceLock::acquire_waiting(&path, crate::STARTUP_LOCK_WAIT)
        .expect("a starting agent gets its lock despite the probe");
    end_probe.join().unwrap();
    assert_eq!(probe_lock(&path), "held");
    drop(lock);
    assert_eq!(probe_until(&path, "free"), "free");
}

/// In a Flatpak the rule directories are the sandbox's: not reported.
#[cfg(unix)]
#[test]
fn a_flatpak_reports_no_udev_rules() {
    let root = root();
    let env = linux_fixture(root.path());
    write(&root.path().join(".flatpak-info"), "[Application]\n");
    let report = collect(&env);
    assert_eq!(report["install"]["type"], "flatpak");
    assert_eq!(report["udevRules"], Value::Null);
    conforms(&schema_v1(), &report, "flatpak report").unwrap();
}

/// Windows shapes of the report's paths and names (#515's redaction):
/// drive letters, case, separators and verbatim paths.
#[cfg(windows)]
#[test]
fn windows_paths_and_names_are_redacted() {
    let redactor = Redactor {
        home: Some(PathBuf::from(r"C:\Users\Alice")),
        user: Some("Alice".into()),
        fold_case: true,
    };
    for (label, path) in [
        ("drive-letter home", r"C:\Users\Alice\AppData\Roaming\x"),
        ("lower case", r"c:\users\alice\AppData\Roaming\x"),
        ("forward slashes", "C:/Users/Alice/AppData/Roaming/x"),
        ("mixed separators", r"C:\Users/Alice\AppData/Roaming\x"),
        ("verbatim", r"\\?\C:\Users\Alice\AppData\Roaming\x"),
    ] {
        let text = redactor.path(Path::new(path));
        assert!(text.starts_with('~'), "not below ~: {label}");
        assert!(
            !text.to_lowercase().contains("alice"),
            "the user name is left: {label}"
        );
    }
    // Outside the home directory the user name still goes.
    let other = redactor.path(Path::new(r"D:\Data\ALICE\x"));
    assert!(
        !other.to_lowercase().contains("alice"),
        "the user name is left: other drive"
    );
    assert!(
        !redactor.text("signed in as alice").contains("alice"),
        "the user name is left: message"
    );
}

/// The endpoint's id is a hash of the data directory's path (with the
/// user name in it): the report hides it.
#[test]
fn the_endpoint_id_is_hidden() {
    assert_eq!(
        hide_endpoint_id(r"\\.\pipe\deckpilot-agent-0123456789abcdef"),
        r"\\.\pipe\deckpilot-agent-<id>"
    );
    assert_eq!(
        hide_endpoint_id("/tmp/deckpilot-501/agent-a4ae57efd340dc75.sock"),
        "/tmp/deckpilot-501/agent-<id>.sock"
    );
    // Anything else stays.
    assert_eq!(
        hide_endpoint_id("~/agent-notes/agent-12"),
        "~/agent-notes/agent-12"
    );
}

#[cfg(unix)]
#[test]
fn the_report_hides_the_endpoint_id() {
    let root = root();
    let env = linux_fixture(root.path());
    let report = collect(&env);
    let endpoint = report["agent"]["endpoint"].as_str().unwrap();
    assert!(endpoint.contains("agent-<id>"), "the endpoint keeps its id");
}
