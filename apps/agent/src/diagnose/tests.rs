use super::*;
use deckpilot_distribution::{Architecture, Distribution, DistributionChannel, Platform};

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
            items
                .iter()
                .enumerate()
                .try_for_each(|(i, item)| conforms(&element[0], item, &format!("{at}[{i}]")))
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
            let expected: Vec<&String> =
                fields.keys().filter(|key| !key.starts_with('$')).collect();
            let actual: Vec<&String> = object.keys().collect();
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
    assert_eq!(ports, ["1-2", "1-3", "1-5"]);

    assert_eq!(usb[0]["state"], "ok");
    assert_eq!(usb[0]["model"], "elgato.streamdeck.mk2");
    assert_eq!(usb[0]["driver"], true);
    assert_eq!(usb[0]["hidraw"][0]["node"], "/dev/hidraw3");
    assert_eq!(usb[0]["hidraw"][0]["access"], "read-write");

    assert_eq!(usb[1]["state"], "usb-present-hid-missing");
    assert_eq!(usb[1]["hidraw"], json!([]));
    assert_eq!(usb[1]["serial"], serial_pseudonym(SERIAL_NO_HID));

    // XL: seen, no driver.
    assert_eq!(usb[2]["model"], "elgato.streamdeck.xl");
    assert_eq!(usb[2]["driver"], false);
    assert_eq!(usb[2]["state"], "ok");

    let findings = devices["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{findings:?}");
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
    assert_eq!(usb[2]["state"], "no-permission");
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
            "no-permission"
        ]
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
    for secret in [
        SERIAL_OK,
        SERIAL_NO_HID,
        "XL0001",
        TOKEN,
        "/home/alice",
        "alice",
    ] {
        assert!(!text.contains(secret), "{secret} in the report: {text}");
    }
    assert_eq!(
        report["agent"]["dataDirectory"],
        "~/.local/share/io.github.knittingcode.deckpilot"
    );
    assert!(
        report["install"]["executable"]
            .as_str()
            .unwrap()
            .ends_with("/opt/<user>/deckpilot-agent")
    );
    // The pseudonym is stable and not the plain hash of the serial.
    let pseudonym = serial_pseudonym(SERIAL_OK);
    assert_eq!(report["devices"]["usb"][0]["serial"], pseudonym);
    assert_eq!(pseudonym, serial_pseudonym(SERIAL_OK));
    assert_ne!(pseudonym, serial_pseudonym(SERIAL_NO_HID));
    assert!(pseudonym.starts_with("sha256:") && pseudonym.len() == "sha256:".len() + 16);
    let plain =
        Sha256::digest(SERIAL_OK.as_bytes())[..8]
            .iter()
            .fold(String::new(), |mut text, byte| {
                let _ = write!(text, "{byte:02x}");
                text
            });
    assert!(!pseudonym.contains(&plain));
}

#[test]
fn redaction_keeps_other_paths_and_partial_names() {
    let redactor = Redactor {
        home: Some("/home/al".into()),
        user: Some("al".into()),
    };
    assert_eq!(redactor.path(Path::new("/home/al")), "~");
    assert_eq!(redactor.path(Path::new("/home/al/x")), "~/x");
    // Not the home directory, only a prefix of it.
    assert_eq!(redactor.path(Path::new("/home/alice/x")), "/home/alice/x");
    assert_eq!(redactor.path(Path::new("/srv/al/x")), "/srv/<user>/x");
    assert_eq!(redactor.path(Path::new("/srv/also/x")), "/srv/also/x");
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
        json!({ "present": true, "version": "0.15.0" })
    );
    assert_eq!(
        report["versions"]["pluginHost"],
        json!({ "present": false, "version": null })
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
    assert_eq!(fs::read(data.join("deckpilot.db")).unwrap(), before);
    assert_eq!(fs::read_to_string(data.join("agent.token")).unwrap(), TOKEN);
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
        ])),
    };
    let report = collect(&env);
    conforms(&schema_v1(), &report, "hidapi report").unwrap();
    assert_eq!(report["devices"]["source"], "hidapi");
    assert_eq!(report["devices"]["usb"], Value::Null);
    assert_eq!(report["udevRules"], Value::Null);
    let hid = report["devices"]["hid"].as_array().unwrap();
    assert_eq!(hid.len(), 2);
    assert_eq!(hid[0]["serial"], serial_pseudonym(SERIAL_OK));
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
        hid: HidSource::Listed(Err("no HID API".into())),
        ..env
    };
    let report = collect(&failed);
    conforms(&schema_v1(), &report, "failed hidapi report").unwrap();
    assert_eq!(report["devices"]["hid"], Value::Null);
    assert_eq!(report["devices"]["hidError"], "no HID API");
}

#[cfg(unix)]
#[test]
fn the_summary_names_the_finding() {
    let root = root();
    let report = collect(&linux_fixture(root.path()));
    let text = summary(&report);
    assert!(
        text.contains("DeckPilot agent 0.15.0 (deb on linux x86_64)"),
        "{text}"
    );
    assert!(text.contains("agent running: no"), "{text}");
    assert!(
        text.contains("Elgato devices: 3 USB, 2 HID (sysfs)"),
        "{text}"
    );
    assert!(
        text.contains("! usb-present-hid-missing (product 0080)"),
        "{text}"
    );
}
