//! Device retries and the flap guard (DP-USB-02, DP-USB-03) with simulated
//! Stream Deck MK.2s and injected faults: a device that cannot be opened
//! is retried with growing waits, one that keeps dropping right after it
//! opens is not redrawn on every cycle, and a reconnect restores the page,
//! images and brightness without running any action. These check the
//! agent's handling with the mock backend only, not the USB driver or a
//! physical device.

mod support;

use std::time::Duration;

use deckpilot_core::ProjectCatalog;
use deckpilot_core::{
    ActionParameters, BindingTarget, ControlBinding, DeviceLayout, LayoutPage, Profile, Project,
    ProjectKey, Trigger,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, DeviceError, DeviceId, Frame, models};
use deckpilot_ipc::{AgentClient, AgentEvent, AgentStatus, Request};
use deckpilot_modules::general;
use support::{FakeInstant, TIMEOUT, TempDir, TestAgent};

/// How long a test watches for something that must not happen: many
/// rescans (every 50 ms in tests).
const QUIET: Duration = Duration::from_millis(400);

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

/// "Next" on key 0, "Announce" on key 1 and a "Folder" page opened by
/// key 5 (log actions).
fn profile() -> Profile {
    let registry = support::registry();
    let mut profile = Profile::new("Work");
    profile
        .add_project(Project::new(ProjectKey::new("SHOP").unwrap(), "SHOP"))
        .unwrap();
    profile.default_project = Some(profile.projects[0].id);
    let log = |name: &str| {
        registry
            .create_instance(
                name,
                general::log_id(),
                &ActionParameters::new()
                    .with("message", deckpilot_core::ParameterValue::Text(name.into())),
            )
            .unwrap()
    };
    profile.actions.push(log("Next"));
    profile.actions.push(log("Announce"));
    let mut layout = DeviceLayout::generate("Main", &models::stream_deck_mk2(), &profile);
    let in_folder = log("In folder");
    let in_folder_id = in_folder.id;
    profile.actions.push(in_folder);
    let mut folder = LayoutPage::new("Folder");
    folder.bindings.push(ControlBinding::new(
        key(2),
        Trigger::Press,
        BindingTarget::action(in_folder_id),
    ));
    layout.pages[0].bindings.push(ControlBinding::new(
        key(5),
        Trigger::Press,
        BindingTarget::OpenPage { page: folder.id },
    ));
    layout.pages.push(folder);
    profile.layouts.push(layout);
    profile
}

async fn status(agent: &TestAgent) -> AgentStatus {
    match agent.request(Request::GetStatus).await {
        Ok(deckpilot_ipc::Response::Status { status }) => *status,
        other => panic!("status: {other:?}"),
    }
}

async fn wait_status(agent: &TestAgent, predicate: impl Fn(&AgentStatus) -> bool) -> AgentStatus {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let status = status(agent).await;
        if predicate(&status) {
            return status;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "condition not reached in time"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_until(what: &str, predicate: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while !predicate() {
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn page<'a>(status: &'a AgentStatus, device: &DeviceId) -> Option<&'a str> {
    status
        .devices
        .iter()
        .find(|d| d.id == *device)
        .and_then(|d| d.page.as_deref())
}

fn connected(status: &AgentStatus, device: &DeviceId) -> bool {
    status
        .devices
        .iter()
        .any(|d| d.id == *device && d.state == ConnectionState::Connected)
}

fn home_frames(
    mock: &MockBackend,
    device: &DeviceId,
) -> std::collections::BTreeMap<ControlId, Frame> {
    assert!(mock.wait_for(device, TIMEOUT, |frames, _| {
        frames.contains_key(&key(0)) && frames.contains_key(&key(1)) && frames.contains_key(&key(5))
    }));
    mock.frames(device)
}

/// Actions completed until `within` passes.
async fn actions_within(events: &mut AgentClient, within: Duration) -> Vec<String> {
    let deadline = tokio::time::Instant::now() + within;
    let mut names = Vec::new();
    while let Ok(event) = tokio::time::timeout_at(deadline, events.next_event()).await {
        if let AgentEvent::ActionCompleted { name, .. } = event.unwrap() {
            names.push(name);
        }
    }
    names
}

/// Waits until `device` has been opened `attempts` times and the agent
/// has recorded the failure of the last open: the mock counts an open
/// before the agent reads the clock for its failure, and a request is
/// answered only after the rescan that made the open finished, so the
/// clock is never moved in between.
async fn failed_open(agent: &TestAgent, mock: &MockBackend, device: &DeviceId, attempts: usize) {
    wait_until("the open", || mock.open_attempts(device) >= attempts).await;
    assert_eq!(mock.open_attempts(device), attempts, "one open per wait");
    status(agent).await;
}

/// Opens of `device` after the clock stood still for [`QUIET`]: the count
/// no longer changes.
async fn settled_attempts(mock: &MockBackend, device: &DeviceId) -> usize {
    let before = mock.open_attempts(device);
    tokio::time::sleep(QUIET).await;
    let after = mock.open_attempts(device);
    assert_eq!(before, after, "no opens while the clock stands still");
    after
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_that_cannot_be_opened_is_retried_with_growing_waits() {
    let dir = TempDir::new("deckpilot-retry");
    let (paths, key_material) = support::data_dir(&dir, &profile(), &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    mock.fail_open(&device, DeviceError::Io("Permission denied".into()));
    let clock = FakeInstant::default();
    let policy = deckpilot_agent::RetryPolicy::default();
    let agent = TestAgent::start_with_device_retry(&paths, &key_material, &mock, policy, &clock);
    agent.unlock().await;

    failed_open(&agent, &mock, &device, 1).await;
    // Rescans every 50 ms do not retry before the wait is over.
    assert_eq!(settled_attempts(&mock, &device).await, 1);
    // First wait: 2 s ±20%.
    clock.advance(Duration::from_millis(1500));
    assert_eq!(settled_attempts(&mock, &device).await, 1);
    clock.advance(Duration::from_millis(1000));
    failed_open(&agent, &mock, &device, 2).await;
    // Second wait: 4 s ±20%, so not yet after 3 s.
    clock.advance(Duration::from_secs(3));
    assert_eq!(settled_attempts(&mock, &device).await, 2);
    clock.advance(Duration::from_secs(2));
    failed_open(&agent, &mock, &device, 3).await;

    // Ten simulated minutes, one wait at a time: after the n-th failed
    // open the next comes no sooner than 0.8 and no later than 1.2 times
    // its base wait (2 s doubling, at most 60 s). Each step moves the
    // clock to just before the earliest time (no open), then to the
    // latest (exactly one open, waited for), so the count never depends
    // on how fast the runner is.
    let mut elapsed = Duration::ZERO;
    let mut attempts = 3;
    while elapsed < Duration::from_secs(600) {
        let base = policy.base_delay(u32::try_from(attempts).unwrap());
        let earliest = base.mul_f64(0.8).saturating_sub(Duration::from_millis(1));
        // Slack for float rounding: the factor at its top is
        // 1.2000000000000002, and the fake clock moves in whole ms.
        let latest = (base.mul_f64(1.2) + Duration::from_millis(2)).min(policy.max);
        clock.advance(earliest);
        assert_eq!(
            settled_attempts(&mock, &device).await,
            attempts,
            "no open before {earliest:?} after failure {attempts}"
        );
        clock.advance(latest.saturating_sub(earliest));
        elapsed += latest;
        attempts += 1;
        failed_open(&agent, &mock, &device, attempts).await;
    }
    // The waits grew to 60 s: far fewer opens than the 300 a fixed 2 s
    // retry would make (3 + 3 waits up to 38.4 s + 9 of 60 s).
    let attempts = settled_attempts(&mock, &device).await;
    assert_eq!(attempts, 15, "bounded opens over 10 minutes");
    assert!(!connected(&status(&agent).await, &device));

    // Fixed: the device connects at the next attempt and is drawn.
    mock.clear_faults(&device);
    clock.advance(Duration::from_secs(61));
    wait_status(&agent, |s| connected(s, &device)).await;
    home_frames(&mock, &device);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replugged_device_is_tried_at_once_after_long_waits() {
    let dir = TempDir::new("deckpilot-retry");
    let (paths, key_material) = support::data_dir(&dir, &profile(), &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    // Another program holds the device.
    mock.fail_open(&device, DeviceError::Io("Device or resource busy".into()));
    let clock = FakeInstant::default();
    let policy = deckpilot_agent::RetryPolicy::default();
    let agent = TestAgent::start_with_device_retry(&paths, &key_material, &mock, policy, &clock);
    agent.unlock().await;
    for attempts in 1..=5 {
        wait_until("open", || mock.open_attempts(&device) == attempts).await;
        clock.advance(policy.max);
    }
    wait_until("sixth open", || mock.open_attempts(&device) == 6).await;
    assert_eq!(settled_attempts(&mock, &device).await, 6, "waits 60 s now");

    // The user quits the other program and replugs: with the clock
    // standing still, the device opens at once.
    mock.unplug(&device);
    tokio::time::sleep(QUIET).await;
    mock.clear_faults(&device);
    mock.plug(&models::stream_deck_mk2(), "MK2-A");
    wait_status(&agent, |s| connected(s, &device)).await;
    home_frames(&mock, &device);
    assert_eq!(mock.open_attempts(&device), 7);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_devices_retries_a_waiting_device_at_once() {
    let dir = TempDir::new("deckpilot-retry");
    let (paths, key_material) = support::data_dir(&dir, &profile(), &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    mock.fail_open_times(
        &device,
        DeviceError::Io("Device or resource busy".into()),
        3,
    );
    let clock = FakeInstant::default();
    let agent = TestAgent::start_with_device_retry(
        &paths,
        &key_material,
        &mock,
        deckpilot_agent::RetryPolicy::default(),
        &clock,
    );
    agent.unlock().await;
    wait_until("first open", || mock.open_attempts(&device) == 1).await;
    assert_eq!(settled_attempts(&mock, &device).await, 1);

    agent.request(Request::RefreshDevices).await.unwrap();
    wait_until("open on refresh", || mock.open_attempts(&device) == 2).await;
    // Once: the failure after it waits again.
    assert_eq!(settled_attempts(&mock, &device).await, 2);
    agent.request(Request::RefreshDevices).await.unwrap();
    agent.request(Request::RefreshDevices).await.unwrap();
    wait_status(&agent, |s| connected(s, &device)).await;
    assert_eq!(mock.open_attempts(&device), 4);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_flapping_device_is_not_redrawn_on_every_cycle_and_gets_its_state_back() {
    let dir = TempDir::new("deckpilot-retry");
    let (paths, key_material) = support::data_dir(&dir, &profile(), &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let clock = FakeInstant::default();
    let policy = deckpilot_agent::RetryPolicy::default();
    let agent = TestAgent::start_with_device_retry(&paths, &key_material, &mock, policy, &clock);
    agent.unlock().await;
    home_frames(&mock, &device);
    agent
        .request(Request::SetBrightness { percent: 35 })
        .await
        .unwrap();
    wait_until("brightness", || mock.brightness(&device) == Some(35)).await;
    mock.tap(&device, key(5));
    wait_status(&agent, |s| page(s, &device) == Some("Folder")).await;
    assert!(
        mock.wait_for(&device, TIMEOUT, |frames, _| frames.contains_key(&key(2))
            && !frames.contains_key(&key(1)))
    );
    let folder = mock.frames(&device);
    // A stable session first: it ends no streak, and the next drop is the
    // first short one.
    clock.advance(policy.stable_after);
    tokio::time::sleep(Duration::from_millis(120)).await;

    // From now on every connection drops at its first write, and the
    // device re-enumerates (a hub reset; a new hidraw path, same serial).
    mock.fail_after_writes(&device, 0, DeviceError::Disconnected);
    let mut events = agent.events().await;
    let base = mock.open_attempts(&device);
    mock.reenumerate(&device);
    // Three short sessions: each opened and failed on its first write.
    for cycle in 1..=3 {
        wait_until("open", || mock.open_attempts(&device) == base + cycle).await;
        wait_status(&agent, |s| !connected(s, &device)).await;
        assert_eq!(settled_attempts(&mock, &device).await, base + cycle);
        clock.advance(policy.max);
    }

    // Flapping now: the fourth session opens but writes nothing until it
    // has lasted the settle time.
    wait_until("fourth open", || mock.open_attempts(&device) == base + 4).await;
    wait_status(&agent, |s| connected(s, &device)).await;
    let writes = mock.image_writes(&device);
    tokio::time::sleep(QUIET).await;
    assert_eq!(
        mock.image_writes(&device),
        writes,
        "no redraw while settling"
    );
    assert_eq!(mock.brightness(&device), None, "nor brightness");
    assert!(
        connected(&status(&agent).await, &device),
        "the session lives"
    );
    // Settled: restored, which fails again (still faulty).
    clock.advance(policy.settle);
    wait_status(&agent, |s| !connected(s, &device)).await;
    assert_eq!(settled_attempts(&mock, &device).await, base + 4);

    // The link recovers. The next session is still held back while it
    // settles, then gets the page, images and brightness it had.
    mock.clear_faults(&device);
    clock.advance(policy.max);
    wait_status(&agent, |s| connected(s, &device)).await;
    assert_eq!(mock.open_attempts(&device), base + 5);
    tokio::time::sleep(QUIET).await;
    assert!(
        mock.frames(&device).is_empty(),
        "nothing drawn before it settles"
    );
    // Presses on keys that show nothing yet are ignored.
    mock.tap(&device, key(2));
    tokio::time::sleep(QUIET).await;
    clock.advance(policy.settle);
    assert!(
        mock.wait_for(&device, TIMEOUT, |frames, _| *frames == folder),
        "images restored"
    );
    wait_until("brightness restored", || {
        mock.brightness(&device) == Some(35)
    })
    .await;
    let status = status(&agent).await;
    assert_eq!(page(&status, &device), Some("Folder"), "page restored");
    // Reconnecting ran no action.
    assert_eq!(
        actions_within(&mut events, QUIET).await,
        Vec::<String>::new()
    );

    // Navigation and input work on the restored session.
    mock.tap(&device, key(2));
    assert_eq!(
        actions_within(&mut events, Duration::from_secs(3)).await,
        vec!["In folder".to_owned()]
    );
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnecting_restores_presentation_and_runs_no_action() {
    let dir = TempDir::new("deckpilot-retry");
    let (paths, key_material) = support::data_dir(&dir, &profile(), &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    // The agent's own policy for its (test) rescan interval, on the real
    // clock.
    let agent = TestAgent::start_unlocked(&paths, &key_material, &mock, None).await;
    let home = home_frames(&mock, &device);
    let mut events = agent.events().await;
    mock.tap(&device, key(0));
    assert_eq!(
        actions_within(&mut events, Duration::from_secs(3)).await,
        vec!["Next".to_owned()]
    );

    // Unplug and replug, then re-enumerate several times in a row.
    mock.unplug(&device);
    wait_status(&agent, |s| !connected(s, &device)).await;
    mock.plug(&models::stream_deck_mk2(), "MK2-A");
    wait_status(&agent, |s| connected(s, &device)).await;
    for _ in 0..3 {
        mock.reenumerate(&device);
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    wait_status(&agent, |s| connected(s, &device)).await;
    assert!(
        mock.wait_for(&device, TIMEOUT, |frames, _| *frames == home),
        "images restored"
    );
    assert_eq!(
        actions_within(&mut events, QUIET).await,
        Vec::<String>::new(),
        "no action runs on reconnect"
    );
    mock.tap(&device, key(0));
    assert_eq!(
        actions_within(&mut events, Duration::from_secs(3)).await,
        vec!["Next".to_owned()]
    );
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_failing_device_does_not_delay_another_and_a_new_path_reconnects_at_once() {
    let dir = TempDir::new("deckpilot-retry");
    let (paths, key_material) = support::data_dir(&dir, &profile(), &[]);
    let mock = MockBackend::new();
    let a = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    mock.fail_open(&a, DeviceError::Io("Permission denied".into()));
    let clock = FakeInstant::default();
    let policy = deckpilot_agent::RetryPolicy::default();
    let agent = TestAgent::start_with_device_retry(&paths, &key_material, &mock, policy, &clock);
    agent.unlock().await;
    wait_until("A tried", || mock.open_attempts(&a) == 1).await;

    // B arrives while A waits, with the clock standing still: B connects
    // at once and works.
    let b = mock.plug(&models::stream_deck_mk2(), "MK2-B");
    wait_status(&agent, |s| connected(s, &b)).await;
    let home = home_frames(&mock, &b);
    mock.tap(&b, key(5));
    wait_status(&agent, |s| page(s, &b) == Some("Folder")).await;
    assert_eq!(settled_attempts(&mock, &a).await, 1, "A still waits");

    // B's session has been stable; it comes back under a new path (same
    // serial): it reconnects without waiting, on its page.
    clock.advance(policy.stable_after);
    tokio::time::sleep(Duration::from_millis(120)).await;
    let folder = mock.frames(&b);
    mock.reenumerate(&b);
    wait_until("B reopened", || mock.open_attempts(&b) == 2).await;
    let status = wait_status(&agent, |s| connected(s, &b)).await;
    assert_eq!(page(&status, &b), Some("Folder"));
    assert!(mock.wait_for(&b, TIMEOUT, |frames, _| *frames == folder));
    assert_ne!(folder, home);

    // A is fixed: it connects at its next attempt, independently of B.
    mock.clear_faults(&a);
    clock.advance(policy.max);
    wait_status(&agent, |s| connected(s, &a) && connected(s, &b)).await;
    assert!(mock.wait_for(&a, TIMEOUT, |frames, _| *frames == home));
    agent.shutdown().await;
}
