//! The built agent executable starts at all. Windows refuses to load it
//! when a DLL import cannot be resolved (e.g. `comctl32.dll` version 6
//! without an application manifest), long before `main` runs; other tests
//! use the agent as a library and would not notice.

use std::process::Command;

#[test]
fn the_executable_starts_and_prints_its_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"))
        .arg("--version")
        .output()
        .expect("the executable can be started");
    assert!(
        output.status.success(),
        "deckpilot-agent --version failed with {} (on Windows 0xc0000139 means a missing DLL entry point)\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with(&format!("deckpilot-agent {}", env!("CARGO_PKG_VERSION"))),
        "{stdout}"
    );
}

/// `diagnose --json` runs without an agent and without a data directory,
/// prints the versioned report and creates nothing.
#[test]
fn diagnose_prints_the_report_and_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("never-created");
    let output = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"))
        .args(["diagnose", "--json"])
        .env("DECKPILOT_DATA_DIR", &data)
        .output()
        .expect("the executable can be started");
    assert!(
        output.status.success(),
        "diagnose --json failed with {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema"], "deckpilot.diagnose");
    assert_eq!(report["schemaVersion"], 1);
    assert_eq!(report["versions"]["agent"], env!("CARGO_PKG_VERSION"));
    assert_eq!(report["agent"]["running"], false);
    assert_eq!(report["agent"]["lock"], "absent");
    assert_eq!(report["agent"]["dataDirectoryOverridden"], true);
    assert!(!data.exists(), "diagnose created the data directory");

    let summary = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"))
        .arg("diagnose")
        .env("DECKPILOT_DATA_DIR", &data)
        .output()
        .unwrap();
    assert!(summary.status.success());
    assert!(String::from_utf8_lossy(&summary.stdout).contains("agent running: no"));

    let wrong = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"))
        .args(["diagnose", "--fix"])
        .env("DECKPILOT_DATA_DIR", &data)
        .output()
        .unwrap();
    assert_eq!(wrong.status.code(), Some(2));
    let help = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"))
        .args(["diagnose", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("deckpilot-agent diagnose [--json]"));
    assert!(!data.exists());
}
