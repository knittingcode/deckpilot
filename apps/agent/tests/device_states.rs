//! Device states (DP-USB-01) with simulated Stream Deck MK.2s and injected
//! faults: why a device is not usable is reported per device, a device
//! that is present is never reported as not connected, and a client of
//! protocol 20 receives the device list it always did. These check the
//! agent with the mock backend only, not the USB driver or a physical
//! device.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use deckpilot_core::Profile;
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{
    ConnectionState, DeviceAvailability, DeviceBackend, DeviceError, DeviceHandle, DeviceId,
    DiscoveredDevice, UnavailableDevice, models,
};
use deckpilot_ipc::{AgentClient, AgentStatus, ClientKind, DeviceInfo, Request, Response};
use support::{FakeInstant, TIMEOUT, TempDir, TestAgent};

fn info<'a>(status: &'a AgentStatus, device: &DeviceId) -> Option<&'a DeviceInfo> {
    status.devices.iter().find(|d| d.id == *device)
}

fn availability(status: &AgentStatus, device: &DeviceId) -> Option<DeviceAvailability> {
    info(status, device).and_then(|d| d.availability)
}

async fn status_of(client: &mut AgentClient) -> AgentStatus {
    match client.request(Request::GetStatus).await {
        Ok(Response::Status { status }) => *status,
        other => panic!("status: {other:?}"),
    }
}

async fn wait_status(agent: &TestAgent, predicate: impl Fn(&AgentStatus) -> bool) -> AgentStatus {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        if let Ok(Response::Status { status }) = agent.request(Request::GetStatus).await
            && predicate(&status)
        {
            return *status;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "condition not reached in time"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_availability(
    agent: &TestAgent,
    device: &DeviceId,
    expected: DeviceAvailability,
) -> AgentStatus {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        if let Ok(Response::Status { status }) = agent.request(Request::GetStatus).await {
            if availability(&status, device) == Some(expected) {
                return *status;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{device}: {:?}, expected {expected:?}",
                availability(&status, device)
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_until(predicate: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while !predicate() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "condition not reached in time"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn data_dir(dir: &TempDir) -> (deckpilot_ipc::AgentPaths, deckpilot_storage::DatabaseKey) {
    support::data_dir(dir, &Profile::new("Work"), &[])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_errors_are_reported_per_device_and_present_devices_are_never_not_connected() {
    let dir = TempDir::new("deckpilot-states");
    let (paths, key) = data_dir(&dir);
    let mock = MockBackend::new();
    let mk2 = models::stream_deck_mk2();
    let cases = [
        (
            "DENIED",
            DeviceError::PermissionDenied("Permission denied".into()),
            DeviceAvailability::PermissionDenied,
        ),
        (
            "BUSY",
            DeviceError::Busy("exclusive access".into()),
            DeviceAvailability::Busy,
        ),
        (
            "BROKEN",
            DeviceError::Io("Input/output error".into()),
            DeviceAvailability::CommunicationError,
        ),
        (
            "NODRIVER",
            DeviceError::Unsupported("model".into()),
            DeviceAvailability::Unsupported,
        ),
    ];
    let mut ids = Vec::new();
    for (serial, error, _) in &cases {
        let id = mock.plug(&mk2, serial);
        mock.fail_open(&id, error.clone());
        ids.push(id);
    }
    let no_hid = mock.plug_unavailable(&mk2, "NOHID", DeviceAvailability::UsbPresentHidUnavailable);
    let fine = mock.plug(&mk2, "FINE");
    let agent = TestAgent::start_unlocked(&paths, &key, &mock, None).await;

    let status = wait_status(&agent, |s| {
        ids.iter().all(|id| availability(s, id).is_some())
            && availability(s, &no_hid).is_some()
            && availability(s, &fine) == Some(DeviceAvailability::Connected)
    })
    .await;
    for ((_, _, expected), id) in cases.iter().zip(&ids) {
        let device = info(&status, id).unwrap();
        assert_eq!(device.availability, Some(*expected), "{id}");
        assert_eq!(device.state, ConnectionState::Disconnected);
        assert!(device.availability.unwrap().is_present());
        assert_eq!(device.model_name, "Stream Deck MK.2");
    }
    assert_eq!(
        availability(&status, &no_hid),
        Some(DeviceAvailability::UsbPresentHidUnavailable)
    );
    assert_eq!(
        info(&status, &fine).unwrap().state,
        ConnectionState::Connected
    );

    // Fixed (the udev rule now applies): it connects at its next attempt.
    mock.clear_faults(&ids[0]);
    agent.request(Request::RefreshDevices).await.unwrap();
    wait_availability(&agent, &ids[0], DeviceAvailability::Connected).await;
    // Unplugged: now it is not connected.
    mock.unplug(&ids[0]);
    wait_availability(&agent, &ids[0], DeviceAvailability::NotConnected).await;
    // A device that only was seen disappears from the list when it goes.
    mock.unplug(&no_hid);
    wait_status(&agent, |s| info(s, &no_hid).is_none()).await;
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_that_keeps_dropping_goes_from_reconnecting_to_communication_error() {
    let dir = TempDir::new("deckpilot-states");
    let (paths, key) = data_dir(&dir);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let clock = FakeInstant::default();
    let policy = deckpilot_agent::RetryPolicy::default();
    let agent = TestAgent::start_with_device_retry(&paths, &key, &mock, policy, &clock);
    agent.unlock().await;
    wait_availability(&agent, &device, DeviceAvailability::Connected).await;

    // Every connection drops at its first write.
    mock.fail_after_writes(&device, 0, DeviceError::Disconnected);
    let base = mock.open_attempts(&device);
    mock.reenumerate(&device);
    let status = wait_availability(&agent, &device, DeviceAvailability::Reconnecting).await;
    assert_eq!(
        info(&status, &device).unwrap().state,
        ConnectionState::Disconnected
    );
    // Two more short connections: the third makes it flapping. Each step
    // waits for its open and its close, never for a fixed time: with the
    // clock moved again before a connection closed, two moves would make
    // one connection (the next wait counts from its close).
    for step in 1..=2 {
        let opened = base + step;
        clock.advance(policy.max);
        wait_until(|| mock.open_attempts(&device) >= opened).await;
        assert_eq!(mock.open_attempts(&device), opened, "one open per move");
        wait_status(&agent, |s| {
            info(s, &device).is_some_and(|d| d.state == ConnectionState::Disconnected)
        })
        .await;
    }
    wait_availability(&agent, &device, DeviceAvailability::CommunicationError).await;
    // The next connection opens (and is held back while it settles);
    // its restore fails again.
    clock.advance(policy.max);
    wait_availability(&agent, &device, DeviceAvailability::Connected).await;
    clock.advance(policy.settle);
    wait_availability(&agent, &device, DeviceAvailability::CommunicationError).await;

    // The link recovers: connected again, and its keys are restored.
    mock.clear_faults(&device);
    clock.advance(policy.max);
    wait_availability(&agent, &device, DeviceAvailability::Connected).await;
    clock.advance(policy.settle);
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while mock.brightness(&device).is_none() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "brightness restored"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    agent.shutdown().await;
}

/// The mock behind a switch that makes discovery fail, as when hidapi
/// cannot enumerate.
struct FlakyDiscovery {
    mock: MockBackend,
    failing: Arc<AtomicBool>,
}

impl DeviceBackend for FlakyDiscovery {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>, DeviceError> {
        if self.failing.load(Ordering::SeqCst) {
            Err(DeviceError::Io("enumeration failed".into()))
        } else {
            self.mock.discover()
        }
    }

    fn open(&mut self, device: &DeviceId) -> Result<Box<dyn DeviceHandle>, DeviceError> {
        self.mock.open(device)
    }

    fn unavailable(&mut self) -> Vec<UnavailableDevice> {
        self.mock.unavailable()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn while_enumeration_fails_devices_are_unknown_not_disconnected() {
    let dir = TempDir::new("deckpilot-states");
    let (paths, key) = data_dir(&dir);
    let mock = MockBackend::new();
    let open = mock.plug(&models::stream_deck_mk2(), "OPEN");
    let failing = Arc::new(AtomicBool::new(false));
    let backend = FlakyDiscovery {
        mock: mock.clone(),
        failing: failing.clone(),
    };
    let agent = TestAgent::start_with_backend(&paths, &key, &mock, Box::new(backend), None, None);
    agent.unlock().await;
    wait_availability(&agent, &open, DeviceAvailability::Connected).await;
    // A device whose connection is lost while enumeration fails.
    let lost = mock.plug(&models::stream_deck_mk2(), "LOST");
    wait_availability(&agent, &lost, DeviceAvailability::Connected).await;
    failing.store(true, Ordering::SeqCst);
    mock.reenumerate(&lost);
    let status = wait_status(&agent, |s| {
        info(s, &lost).is_some_and(|d| d.state == ConnectionState::Disconnected)
    })
    .await;
    // The open device stays connected; the other is not called gone.
    assert_eq!(
        availability(&status, &open),
        Some(DeviceAvailability::Connected)
    );
    assert_ne!(
        availability(&status, &lost),
        Some(DeviceAvailability::NotConnected)
    );
    failing.store(false, Ordering::SeqCst);
    wait_availability(&agent, &lost, DeviceAvailability::Connected).await;
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protocol_20_client_gets_the_device_list_it_always_did() {
    let dir = TempDir::new("deckpilot-states");
    let (paths, key) = data_dir(&dir);
    let mock = MockBackend::new();
    let mk2 = models::stream_deck_mk2();
    let fine = mock.plug(&mk2, "FINE");
    let denied = mock.plug(&mk2, "DENIED");
    mock.fail_open(&denied, DeviceError::PermissionDenied("EACCES".into()));
    let no_hid = mock.plug_unavailable(&mk2, "NOHID", DeviceAvailability::UsbPresentHidUnavailable);
    let agent = TestAgent::start_unlocked(&paths, &key, &mock, None).await;
    wait_status(&agent, |s| {
        availability(s, &denied).is_some() && availability(s, &fine).is_some()
    })
    .await;

    let mut current = AgentClient::connect(&agent.paths, ClientKind::Desktop)
        .await
        .unwrap();
    assert!(current.supports(deckpilot_ipc::Capability::DeviceStates));
    let status = status_of(&mut current).await;
    assert_eq!(status.devices.len(), 3);

    let mut old = AgentClient::connect_with_protocol(&agent.paths, ClientKind::Desktop, 20)
        .await
        .unwrap();
    assert_eq!(old.protocol(), 20);
    assert!(!old.supports(deckpilot_ipc::Capability::DeviceStates));
    let status = status_of(&mut old).await;
    let serials: Vec<&str> = status
        .devices
        .iter()
        .map(|d| d.id.serial.as_str())
        .collect();
    assert_eq!(serials, ["FINE"], "never-opened devices are left out");
    assert!(status.devices.iter().all(|d| d.availability.is_none()));
    let Ok(Response::Devices { devices }) = old.request(Request::ListDevices).await else {
        panic!("devices");
    };
    assert_eq!(devices.len(), 1);
    assert!(devices[0].availability.is_none());
    assert!(info(&status, &no_hid).is_none());
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn when_one_driver_fails_its_devices_are_unknown_not_disconnected() {
    let dir = TempDir::new("deckpilot-states");
    let (paths, key) = data_dir(&dir);
    let usb = MockBackend::new();
    let simulated = MockBackend::new();
    let lost = usb.plug(&models::stream_deck_mk2(), "USB");
    let other = simulated.plug(&models::stream_deck_mk2(), "SIM");
    let failing = Arc::new(AtomicBool::new(false));
    let backend = deckpilot_device::CombinedBackend::default()
        .with(Box::new(FlakyDiscovery {
            mock: usb.clone(),
            failing: failing.clone(),
        }))
        .with(Box::new(simulated.clone()));
    let agent =
        TestAgent::start_with_backend(&paths, &key, &simulated, Box::new(backend), None, None);
    agent.unlock().await;
    wait_availability(&agent, &lost, DeviceAvailability::Connected).await;
    wait_availability(&agent, &other, DeviceAvailability::Connected).await;
    // The USB driver cannot enumerate; the simulated one still can.
    failing.store(true, Ordering::SeqCst);
    let status = wait_status(&agent, |s| {
        info(s, &lost).is_some_and(|d| d.state == ConnectionState::Disconnected)
    })
    .await;
    assert_eq!(
        availability(&status, &lost),
        Some(DeviceAvailability::Unknown)
    );
    assert_eq!(
        availability(&status, &other),
        Some(DeviceAvailability::Connected)
    );
    failing.store(false, Ordering::SeqCst);
    wait_availability(&agent, &lost, DeviceAvailability::Connected).await;
    agent.shutdown().await;
}
