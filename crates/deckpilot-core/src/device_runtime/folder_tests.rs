//! Folders in the device runtime: the reserved Back key and Auto Exit
//! countdowns, with a clock the tests move.

use std::sync::Mutex;
use std::time::Duration;

use deckpilot_device::{ControlInput, models};

use super::*;
use crate::testing::{Echo, registry};
use crate::{
    ActionHandler, ActionParameters, ActionType, ActionTypeId, AutoExit, ControlNavigation,
    DeckRequest, ExecutionContext, FolderBack, FolderCountdown, FolderHold, Glyph, HistoryMode,
    HoldEnd, LayoutPage, NavigationOperation, Os, ParameterValue, control_navigation,
};

/// `test.switch`: switches to the layout in `message`.
struct Switch(Echo);

impl ActionHandler for Switch {
    fn definition(&self) -> &ActionType {
        self.0.definition()
    }

    fn execute(&self, parameters: &ActionParameters, _: &ExecutionContext<'_>) -> ActionResult {
        let layout = parameters.text("message").unwrap().parse().unwrap();
        ActionResult::success("asked").with_request(DeckRequest::SwitchLayout {
            layout,
            page: None,
            history: HistoryMode::Push,
        })
    }
}

/// A clock the test moves.
#[derive(Clone)]
struct TestClock(Arc<Mutex<Instant>>);

impl TestClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }

    fn advance(&self, seconds: u64) -> Instant {
        let mut now = self.0.lock().unwrap();
        *now += Duration::from_secs(seconds);
        *now
    }
}

fn mk2() -> DeviceModelId {
    DeviceModelId::new(models::STREAM_DECK_MK2_ID)
}

fn device(serial: &str) -> DeviceId {
    DeviceId {
        model: mk2(),
        serial: serial.into(),
    }
}

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn bind(index: u16, target: BindingTarget) -> ControlBinding {
    ControlBinding::new(key(index), Trigger::Press, target)
}

/// Home: key 0 runs an action, key 1 opens Tools, key 2 switches to the
/// second layout, key 3 opens the top-level page Other. Tools (Auto Exit
/// after 10 s): key 1 opens Docker, key 2 runs the action, key 3 switches
/// layouts. Docker (Auto
/// Exit off): key 2 runs the action. Both folders have the reserved Back.
struct Fixture {
    profile: Profile,
    layout: LayoutId,
    second: LayoutId,
    home: PageId,
    tools: PageId,
    docker: PageId,
    other: PageId,
}

fn fixture() -> Fixture {
    let mut profile = Profile::new("Work");
    let run = crate::ActionInstance {
        id: ActionInstanceId::new(),
        name: "Run".into(),
        action_type: crate::testing::echo_type(),
        parameters: ActionParameters::new(),
        description: String::new(),
        icon: None,
        label: None,
    };
    let run_id = run.id;
    let second = DeviceLayout::new("Second", mk2());
    let switch = crate::ActionInstance {
        id: ActionInstanceId::new(),
        name: "Switch".into(),
        action_type: ActionTypeId::new("test.switch").unwrap(),
        parameters: ActionParameters::new()
            .with("message", ParameterValue::Text(second.id.to_string())),
        description: String::new(),
        icon: None,
        label: None,
    };
    let switch_id = switch.id;
    profile.actions.extend([run, switch]);
    let mut layout = DeviceLayout::new("Main", mk2());
    let home = layout.home_page;
    let mut tools = LayoutPage::folder("Tools", home);
    tools.auto_exit = Some(AutoExit {
        enabled: true,
        seconds: 10,
    });
    let docker = LayoutPage::folder("Docker", tools.id);
    let other = LayoutPage::new("Other");
    let (tools_id, docker_id, other_id) = (tools.id, docker.id, other.id);
    tools.bindings.extend([
        bind(1, BindingTarget::OpenPage { page: docker_id }),
        bind(2, BindingTarget::action(run_id)),
        bind(3, BindingTarget::action(switch_id)),
    ]);
    let mut docker = docker;
    docker.bindings.push(bind(2, BindingTarget::action(run_id)));
    layout.pages.extend([tools, docker, other]);
    layout.page_mut(home).unwrap().bindings.extend([
        bind(0, BindingTarget::action(run_id)),
        bind(1, BindingTarget::OpenPage { page: tools_id }),
        bind(2, BindingTarget::action(switch_id)),
        bind(3, BindingTarget::OpenPage { page: other_id }),
    ]);
    layout.explicit_tree = true;
    let layout_id = layout.id;
    let second_id = second.id;
    profile.layouts.extend([layout, second]);
    Fixture {
        profile,
        layout: layout_id,
        second: second_id,
        home,
        tools: tools_id,
        docker: docker_id,
        other: other_id,
    }
}

fn runtime(clock: &TestClock) -> Runtime {
    let mut registry = registry();
    registry
        .register(Box::new(Switch(Echo::with_id("test.switch"))))
        .unwrap();
    let mut runtime = Runtime::new(
        Engine::new(registry),
        ModelCatalog::builtin(),
        MachineConfig::new(Os::current()),
    );
    let clock = clock.clone();
    runtime.set_clock(move || clock.now());
    runtime
}

fn clock() -> TestClock {
    TestClock(Arc::new(Mutex::new(Instant::now())))
}

fn connect(runtime: &mut Runtime, device: &DeviceId) {
    runtime
        .handle_event(&DeviceEvent::Connected {
            device: device.clone(),
        })
        .unwrap();
}

fn input(
    runtime: &mut Runtime,
    device: &DeviceId,
    index: u16,
    input: ControlInput,
) -> RuntimeOutcome {
    runtime
        .handle_event(&DeviceEvent::Input {
            device: device.clone(),
            control: key(index),
            input,
        })
        .unwrap()
}

fn press(runtime: &mut Runtime, device: &DeviceId, index: u16) -> RuntimeOutcome {
    input(runtime, device, index, ControlInput::Press)
}

/// A started runtime with one device on the home page.
fn started(f: &Fixture, clock: &TestClock) -> (Runtime, DeviceId) {
    let mut runtime = runtime(clock);
    runtime.set_profile(Some(f.profile.clone()), None);
    let deck = device("A");
    connect(&mut runtime, &deck);
    (runtime, deck)
}

/// Applies every exit due now; returns how many navigated.
fn expire(runtime: &mut Runtime, now: Instant) -> usize {
    runtime
        .due_folder_exits(now)
        .iter()
        .filter(|exit| runtime.exit_folder(exit).is_some())
        .count()
}

#[test]
fn the_reserved_back_shows_on_every_folder_page_and_goes_up_one_level() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    let back_face = |runtime: &Runtime| {
        runtime
            .faces(&deck)
            .into_iter()
            .find(|(control, _)| *control == key(0))
            .unwrap()
            .1
    };
    // Not on the root page: key 0 runs the action there.
    assert_eq!(back_face(&runtime).style, FaceStyle::Action);
    assert_eq!(runtime.system_back(&deck), None);
    press(&mut runtime, &deck, 1);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    let face = back_face(&runtime);
    assert_eq!(face.style, FaceStyle::Back);
    // Its own icon; Tools exits by itself after 10 s, so the seconds left
    // are its title.
    assert_eq!(face.symbol.as_deref(), Some(symbols::FOLDER_BACK));
    assert_eq!(face.title, "10");
    assert_eq!(runtime.system_back(&deck), Some(key(0)));
    press(&mut runtime, &deck, 1);
    assert_eq!(runtime.current_page(&deck), Some(f.docker));
    // Docker has no Auto Exit: the plain Back, without a title.
    assert_eq!(back_face(&runtime), system_back_face());
    // Docker -> Tools -> Home, one folder level per press; the release
    // does nothing.
    assert!(matches!(
        input(&mut runtime, &deck, 0, ControlInput::Release),
        RuntimeOutcome::Ignored { .. }
    ));
    assert!(matches!(
        press(&mut runtime, &deck, 0),
        RuntimeOutcome::Navigated { page, .. } if page == f.tools
    ));
    assert!(matches!(
        press(&mut runtime, &deck, 0),
        RuntimeOutcome::Navigated { page, .. } if page == f.home
    ));
    // A top-level page other than home has no reserved Back.
    press(&mut runtime, &deck, 3);
    assert_eq!(runtime.current_page(&deck), Some(f.other));
    assert_eq!(runtime.system_back(&deck), None);
    assert_eq!(back_face(&runtime).style, FaceStyle::Empty);
}

#[test]
fn the_reserved_back_wins_over_a_binding_and_takes_no_gestures() {
    let mut f = fixture();
    let layout = f
        .profile
        .layouts
        .iter_mut()
        .find(|l| l.id == f.layout)
        .unwrap();
    // A stored layout that binds the reserved key anyway (refused when
    // saved): the key still goes back and shows Back.
    let run = f.profile.actions[0].id;
    let tools = layout.page_mut(f.tools).unwrap();
    tools.bindings.push(ControlBinding::new(
        key(0),
        Trigger::Click,
        BindingTarget::action(run),
    ));
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    assert_eq!(runtime.key_gestures(&deck, key(0)), None);
    assert!(matches!(
        runtime
            .dispatch_gesture(&deck, key(0), Gesture::Click)
            .unwrap(),
        RuntimeOutcome::Ignored { .. }
    ));
    assert!(
        !runtime
            .shown_targets(&deck)
            .iter()
            .any(|(control, _)| *control == key(0))
    );
    assert!(matches!(
        press(&mut runtime, &deck, 0),
        RuntimeOutcome::Navigated { page, .. } if page == f.home
    ));
}

#[test]
fn previews_name_the_reserved_back_as_a_system_control() {
    let f = fixture();
    let layout = f.profile.layout(f.layout).unwrap();
    let model = models::stream_deck_mk2();
    let controls = crate::editor_navigation::display_controls(&model.capabilities);
    let registry = registry();
    let at = |page, index| {
        control_navigation(&registry, layout, page, &f.profile, key(index), &controls)
    };
    assert_eq!(
        at(f.docker, 0),
        Some(ControlNavigation {
            operation: NavigationOperation::FolderBack,
            page: None,
            layout: None,
            automatic: true,
            system: true,
        })
    );
    assert_eq!(
        serde_json::to_value(at(f.tools, 0)).unwrap(),
        serde_json::json!({"operation": "folder.back", "automatic": true, "system": true})
    );
    assert_eq!(at(f.home, 0), None);
    let faces = page_faces(
        &registry,
        layout,
        f.docker,
        &f.profile,
        None,
        &model.capabilities.controls(),
        Some(&model.capabilities),
    );
    assert_eq!(faces[0].1.style, FaceStyle::Back);
}

#[test]
fn a_legacy_folder_keeps_the_automatic_back_on_its_first_free_key() {
    let mut f = fixture();
    let run = f.profile.actions[0].id;
    let layout = f
        .profile
        .layouts
        .iter_mut()
        .find(|l| l.id == f.layout)
        .unwrap();
    let docker = layout.page_mut(f.docker).unwrap();
    docker.back = Some(FolderBack::LEGACY);
    docker.bindings.push(bind(0, BindingTarget::action(run)));
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    press(&mut runtime, &deck, 1);
    assert_eq!(runtime.system_back(&deck), None);
    // Key 0 runs the user's action; key 1 is the automatic Back.
    assert!(matches!(
        press(&mut runtime, &deck, 0),
        RuntimeOutcome::Action { .. }
    ));
    assert!(matches!(
        press(&mut runtime, &deck, 1),
        RuntimeOutcome::Navigated { page, .. } if page == f.tools
    ));
}

#[test]
fn auto_exit_leaves_the_folder_one_level_up_when_its_time_is_up() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    assert_eq!(runtime.next_folder_exit(), None);
    let entered = clock.now();
    press(&mut runtime, &deck, 1);
    assert_eq!(
        runtime.next_folder_exit(),
        Some(entered + Duration::from_secs(10))
    );
    // Using a key of the folder does not restart it.
    clock.advance(4);
    press(&mut runtime, &deck, 2);
    let now = clock.advance(5);
    assert_eq!(expire(&mut runtime, now), 0);
    let now = clock.advance(1);
    let exits = runtime.due_folder_exits(now);
    assert_eq!(exits.len(), 1);
    assert_eq!(exits[0].page, f.tools);
    assert_eq!(
        runtime.exit_folder(&exits[0]),
        Some(RuntimeOutcome::Navigated {
            device: deck.clone(),
            page: f.home
        })
    );
    assert_eq!(runtime.current_page(&deck), Some(f.home));
    assert_eq!(runtime.next_folder_exit(), None);
    // An exit applies once.
    assert_eq!(runtime.exit_folder(&exits[0]), None);
}

#[test]
fn a_folder_with_auto_exit_off_never_leaves_by_itself() {
    let mut f = fixture();
    let layout = f
        .profile
        .layouts
        .iter_mut()
        .find(|l| l.id == f.layout)
        .unwrap();
    layout.page_mut(f.tools).unwrap().auto_exit = Some(AutoExit::disabled());
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    press(&mut runtime, &deck, 1);
    assert_eq!(runtime.next_folder_exit(), None);
    let now = clock.advance(3600);
    assert_eq!(expire(&mut runtime, now), 0);
    assert_eq!(runtime.current_page(&deck), Some(f.docker));
}

#[test]
fn a_folder_around_the_one_shown_keeps_its_time_and_goes_on_when_shown_again() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    let entered = clock.now();
    press(&mut runtime, &deck, 1);
    clock.advance(3);
    // Into Docker (inside Tools): Tools' countdown stops with 7 s left and
    // never ends while Docker is shown.
    press(&mut runtime, &deck, 1);
    assert_eq!(runtime.folder_timers().due_of(&deck, f.tools), None);
    let now = clock.advance(60);
    assert_eq!(expire(&mut runtime, now), 0);
    assert_eq!(runtime.next_folder_exit(), None);
    assert_eq!(runtime.current_page(&deck), Some(f.docker));
    assert_eq!(
        runtime.folder_timers().left_of(&deck, f.tools, now),
        Some(Duration::from_secs(7))
    );
    // Back in Tools: it goes on from the 7 s it had, not from its start.
    press(&mut runtime, &deck, 0);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    let due = now + Duration::from_secs(7);
    assert_ne!(due, entered + Duration::from_secs(10));
    assert_eq!(runtime.folder_timers().due_of(&deck, f.tools), Some(due));
    assert_eq!(
        runtime.folder_countdown(&deck),
        Some(FolderCountdown {
            seconds: 7,
            paused: false
        })
    );
    let now = clock.advance(7);
    assert_eq!(expire(&mut runtime, now), 1);
    assert_eq!(runtime.current_page(&deck), Some(f.home));
}

#[test]
fn nested_folders_count_down_on_their_own() {
    let mut f = fixture();
    let layout = f
        .profile
        .layouts
        .iter_mut()
        .find(|l| l.id == f.layout)
        .unwrap();
    layout.page_mut(f.docker).unwrap().auto_exit = Some(AutoExit {
        enabled: true,
        seconds: 3,
    });
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    clock.advance(2);
    press(&mut runtime, &deck, 1);
    // Docker's own 3 s, from entering it; Tools waits with its 8 s.
    assert_eq!(runtime.folder_countdown(&deck).map(|c| c.seconds), Some(3));
    let now = clock.advance(3);
    assert_eq!(expire(&mut runtime, now), 1);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    // One level up, to Tools, whose own time goes on from 8 s.
    assert_eq!(runtime.folder_countdown(&deck).map(|c| c.seconds), Some(8));
    let now = clock.advance(7);
    assert_eq!(expire(&mut runtime, now), 0);
    let now = clock.advance(1);
    assert_eq!(expire(&mut runtime, now), 1);
    assert_eq!(runtime.current_page(&deck), Some(f.home));
}

#[test]
fn leaving_by_hand_cancels_and_entering_again_starts_anew() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    clock.advance(6);
    press(&mut runtime, &deck, 0);
    assert_eq!(runtime.current_page(&deck), Some(f.home));
    assert_eq!(runtime.next_folder_exit(), None);
    // Re-entering: 10 s from now, not from the first time.
    let entered = clock.advance(1);
    press(&mut runtime, &deck, 1);
    assert_eq!(
        runtime.next_folder_exit(),
        Some(entered + Duration::from_secs(10))
    );
    let now = clock.advance(9);
    assert_eq!(expire(&mut runtime, now), 0);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
}

#[test]
fn a_stale_exit_cannot_act_after_the_folder_was_entered_again() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    let now = clock.advance(10);
    let stale = runtime.due_folder_exits(now);
    assert_eq!(stale.len(), 1);
    // Out and in again before the exit is applied.
    press(&mut runtime, &deck, 0);
    press(&mut runtime, &deck, 1);
    assert_eq!(runtime.exit_folder(&stale[0]), None);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    let now = clock.advance(10);
    assert_eq!(expire(&mut runtime, now), 1);
    assert_eq!(runtime.current_page(&deck), Some(f.home));
}

#[test]
fn switching_layouts_or_profiles_and_locking_cancel_the_countdown() {
    let f = fixture();
    let clock = clock();
    // A layout switch from a key of the folder.
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    assert!(runtime.next_folder_exit().is_some());
    press(&mut runtime, &deck, 3);
    assert_eq!(runtime.layout_of(&deck).unwrap().id, f.second);
    assert_eq!(runtime.next_folder_exit(), None);
    // The layout a device shows, restored from the database.
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    let profile = f.profile.id;
    runtime.restore_device_layouts([(profile, deck.clone(), f.second)]);
    assert_eq!(runtime.next_folder_exit(), None);
    // Another profile.
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    runtime.set_profile(Some(Profile::new("Home")), None);
    assert_eq!(runtime.next_folder_exit(), None);
    // Locking (no profile).
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    runtime.set_profile(None, None);
    assert_eq!(runtime.next_folder_exit(), None);
    let now = clock.advance(60);
    assert_eq!(runtime.due_folder_exits(now), []);
}

#[test]
fn switching_auto_exit_off_while_inside_cancels_and_a_save_enters_nothing() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    let entered = clock.now();
    // A save that changes something else keeps the countdown.
    clock.advance(2);
    let mut edited = f.profile.clone();
    edited.name = "Renamed".into();
    runtime.reload_profile(edited.clone());
    assert_eq!(
        runtime.next_folder_exit(),
        Some(entered + Duration::from_secs(10))
    );
    let layout = edited
        .layouts
        .iter_mut()
        .find(|l| l.id == f.layout)
        .unwrap();
    layout.page_mut(f.tools).unwrap().auto_exit = Some(AutoExit::disabled());
    runtime.reload_profile(edited.clone());
    assert_eq!(runtime.next_folder_exit(), None);
    // Switched on again: from the next entry on.
    let layout = edited
        .layouts
        .iter_mut()
        .find(|l| l.id == f.layout)
        .unwrap();
    layout.page_mut(f.tools).unwrap().auto_exit = Some(AutoExit {
        enabled: true,
        seconds: 10,
    });
    runtime.reload_profile(edited);
    assert_eq!(runtime.next_folder_exit(), None);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
}

#[test]
fn disconnecting_cancels_and_reconnecting_starts_nothing() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    runtime
        .handle_event(&DeviceEvent::Disconnected {
            device: deck.clone(),
        })
        .unwrap();
    assert_eq!(runtime.next_folder_exit(), None);
    clock.advance(30);
    connect(&mut runtime, &deck);
    // Back on the folder it showed, without a countdown.
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    assert_eq!(runtime.next_folder_exit(), None);
    // Moving inside the folder does not enter it either.
    press(&mut runtime, &deck, 1);
    press(&mut runtime, &deck, 0);
    assert_eq!(runtime.next_folder_exit(), None);
    // Only entering again does.
    press(&mut runtime, &deck, 0);
    press(&mut runtime, &deck, 1);
    assert!(runtime.next_folder_exit().is_some());
}

#[test]
fn devices_count_down_on_their_own_also_on_a_shared_layout() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, a) = started(&f, &clock);
    let b = device("B");
    connect(&mut runtime, &b);
    press(&mut runtime, &a, 1);
    clock.advance(5);
    press(&mut runtime, &b, 1);
    let now = clock.advance(5);
    let due = runtime.due_folder_exits(now);
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].device, a);
    assert_eq!(expire(&mut runtime, now), 1);
    assert_eq!(runtime.current_page(&a), Some(f.home));
    assert_eq!(runtime.current_page(&b), Some(f.tools));
    // B leaving does not touch A, and the other way round.
    press(&mut runtime, &a, 1);
    let now = clock.advance(5);
    assert_eq!(expire(&mut runtime, now), 1);
    assert_eq!(runtime.current_page(&b), Some(f.home));
    assert_eq!(runtime.current_page(&a), Some(f.tools));
}

#[test]
fn a_restarted_runtime_has_no_countdowns() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    let remembered = runtime.remembered_pages();
    // A new agent: pages restored, nothing counting down.
    let mut restarted = super::folder_tests::runtime(&clock);
    restarted.restore_pages(remembered);
    restarted.set_profile(Some(f.profile.clone()), None);
    connect(&mut restarted, &deck);
    assert_eq!(restarted.current_page(&deck), Some(f.home));
    assert_eq!(restarted.next_folder_exit(), None);
}

/// Folder keys (`crate::folder_control`): a press opens the folder at
/// once, whatever else a stored (not yet migrated) layout has on the key,
/// and its recovered bindings never run.
#[test]
fn a_folder_key_opens_on_the_press_and_nothing_else_on_it_runs() {
    let mut f = fixture();
    let run = f.profile.actions[0].id;
    let layout = f
        .profile
        .layouts
        .iter_mut()
        .find(|l| l.id == f.layout)
        .unwrap();
    // Home key 1 opens Tools; stale gestures and a release on it, and a
    // recovered binding, as an unmigrated or imported layout may have.
    let home = layout.page_mut(f.home).unwrap();
    home.bindings.extend([
        ControlBinding::new(key(1), Trigger::DoubleClick, BindingTarget::action(run)),
        ControlBinding::new(key(1), Trigger::Release, BindingTarget::action(run)),
    ]);
    layout.recovered.push(crate::RecoveredBindings {
        page: f.home,
        control: key(1),
        folder: f.tools,
        bindings: vec![ControlBinding::new(
            key(1),
            Trigger::LongPress,
            BindingTarget::action(run),
        )],
        gestures: None,
        reason: crate::RecoveryReason::FolderControl,
    });
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    assert_eq!(runtime.folder_control(&deck, key(1)), Some(f.tools));
    // Not a folder key: the action key and the top-level page key.
    assert_eq!(runtime.folder_control(&deck, key(0)), None);
    assert_eq!(runtime.folder_control(&deck, key(3)), None);
    // Raw: no gesture waits for a double click.
    assert_eq!(runtime.key_gestures(&deck, key(1)), None);
    assert_eq!(runtime.stateful_binding(&deck, key(1)), None);
    for gesture in [Gesture::Click, Gesture::DoubleClick, Gesture::LongPress] {
        assert!(matches!(
            runtime.dispatch_gesture(&deck, key(1), gesture).unwrap(),
            RuntimeOutcome::Ignored { .. }
        ));
    }
    assert_eq!(runtime.current_page(&deck), Some(f.home));
    assert!(matches!(
        press(&mut runtime, &deck, 1),
        RuntimeOutcome::Navigated { page, .. } if page == f.tools
    ));
    // Nested: Tools key 1 opens Docker the same way.
    assert_eq!(runtime.folder_control(&deck, key(1)), Some(f.docker));
    assert!(matches!(
        press(&mut runtime, &deck, 1),
        RuntimeOutcome::Navigated { page, .. } if page == f.docker
    ));
    // The protected Back still goes up one level at a time.
    press(&mut runtime, &deck, 0);
    press(&mut runtime, &deck, 0);
    assert_eq!(runtime.current_page(&deck), Some(f.home));
    // The release of the folder key runs nothing.
    assert!(matches!(
        input(&mut runtime, &deck, 1, ControlInput::Release),
        RuntimeOutcome::Ignored { .. }
    ));
}

#[test]
fn auto_exit_counts_from_a_folder_key_press() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    let entered = clock.now();
    assert!(matches!(
        press(&mut runtime, &deck, 1),
        RuntimeOutcome::Navigated { page, .. } if page == f.tools
    ));
    assert_eq!(
        runtime.next_folder_exit(),
        Some(entered + Duration::from_secs(10))
    );
    let now = clock.advance(10);
    assert_eq!(expire(&mut runtime, now), 1);
    assert_eq!(runtime.current_page(&deck), Some(f.home));
}

// Runs started from a folder's keys (`folder_activity.rs`): the agent
// holds the folder for each run and releases it at the run's end.

/// Changes Tools in the fixture.
fn tools_with(f: &mut Fixture, change: impl FnOnce(&mut LayoutPage)) {
    let layout = f
        .profile
        .layouts
        .iter_mut()
        .find(|l| l.id == f.layout)
        .unwrap();
    change(layout.page_mut(f.tools).unwrap());
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "what the Back key shows: `None` is the plain key"
)]
fn countdown(seconds: u32, paused: bool) -> Option<FolderCountdown> {
    Some(FolderCountdown { seconds, paused })
}

fn back_face(runtime: &Runtime, deck: &DeviceId) -> KeyFace {
    runtime
        .faces(deck)
        .into_iter()
        .find(|(control, _)| *control == key(0))
        .unwrap()
        .1
}

/// A key of the folder starts a run that ends later (a workflow, a
/// plugin's action): the agent holds the key's folder entry for it.
fn start_run(runtime: &mut Runtime, deck: &DeviceId) -> FolderHold {
    let entry = runtime.folder_entry(deck).unwrap();
    runtime.hold_folder_entry(&entry).unwrap()
}

#[test]
fn the_back_key_counts_the_seconds_down_once_a_second() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    assert_eq!(runtime.next_folder_wake(), None);
    let entered = clock.now();
    press(&mut runtime, &deck, 1);
    // From entering: 10, then one less every whole second.
    assert_eq!(runtime.folder_countdown(&deck), countdown(10, false));
    assert_eq!(
        back_face(&runtime, &deck),
        folder_back_face(countdown(10, false))
    );
    assert_eq!(
        runtime.next_folder_wake(),
        Some(entered + Duration::from_secs(1))
    );
    let mut shown = vec![back_face(&runtime, &deck).title];
    for _ in 0..9 {
        let wake = runtime.next_folder_wake().unwrap();
        // Just before the wake, the face has not changed.
        *clock.0.lock().unwrap() = wake.checked_sub(Duration::from_millis(1)).unwrap();
        assert_eq!(back_face(&runtime, &deck).title, *shown.last().unwrap());
        *clock.0.lock().unwrap() = wake;
        shown.push(back_face(&runtime, &deck).title);
    }
    assert_eq!(shown, ["10", "9", "8", "7", "6", "5", "4", "3", "2", "1"]);
    // The last wake is the exit itself.
    assert_eq!(
        runtime.next_folder_wake(),
        Some(entered + Duration::from_secs(10))
    );
    assert_eq!(runtime.next_folder_wake(), runtime.next_folder_exit());
    let now = clock.advance(1);
    assert_eq!(expire(&mut runtime, now), 1);
    assert_eq!(runtime.next_folder_wake(), None);
}

#[test]
fn a_run_pauses_the_countdown_and_it_resumes_with_what_was_left() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    // 10 s; a run starts at 3 s: 7 s are kept.
    clock.advance(3);
    let run = start_run(&mut runtime, &deck);
    assert_eq!(runtime.folder_countdown(&deck), countdown(7, true));
    assert_eq!(
        back_face(&runtime, &deck),
        folder_back_face(countdown(7, true))
    );
    assert_eq!(back_face(&runtime, &deck).glyph, Some(Glyph::Pause));
    assert_eq!(runtime.next_folder_exit(), None);
    assert_eq!(runtime.next_folder_wake(), None);
    // The run takes 20 s: none of it is counted.
    let now = clock.advance(20);
    assert_eq!(expire(&mut runtime, now), 0);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    assert_eq!(runtime.folder_countdown(&deck), countdown(7, true));
    // Over (no Exit after action completes): on from 7 s.
    assert_eq!(runtime.release_folder_hold(&run, HoldEnd::Ran), None);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    assert_eq!(runtime.folder_countdown(&deck), countdown(7, false));
    assert_eq!(
        runtime.next_folder_exit(),
        Some(now + Duration::from_secs(7))
    );
    let now = clock.advance(6);
    assert_eq!(expire(&mut runtime, now), 0);
    let now = clock.advance(1);
    assert_eq!(expire(&mut runtime, now), 1);
    assert_eq!(runtime.current_page(&deck), Some(f.home));
}

#[test]
fn a_key_that_ran_at_once_pauses_for_no_time_and_leaves_the_countdown_alone() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    let entered = clock.now();
    press(&mut runtime, &deck, 1);
    clock.advance(4);
    let entry = runtime.folder_entry(&deck).unwrap();
    assert!(matches!(
        press(&mut runtime, &deck, 2),
        RuntimeOutcome::Action { .. }
    ));
    assert_eq!(runtime.folder_action_ran(&entry), None);
    assert_eq!(
        runtime.next_folder_exit(),
        Some(entered + Duration::from_secs(10))
    );
    // Never paused: the Back key showed no pause mark.
    assert_eq!(runtime.folder_countdown(&deck), countdown(6, false));
}

#[test]
fn an_action_that_ran_in_place_exits_at_once_or_with_the_runs_it_started() {
    let mut f = fixture();
    tools_with(&mut f, |tools| tools.auto_return = true);
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    // Alone: the folder closes as soon as it ran.
    let entry = runtime.folder_entry(&deck).unwrap();
    press(&mut runtime, &deck, 2);
    assert!(matches!(
        runtime.folder_action_ran(&entry),
        Some(RuntimeOutcome::Navigated { page, .. }) if page == f.home
    ));
    // With a run it started (a pick, a switch's workflow): with the run.
    press(&mut runtime, &deck, 1);
    let entry = runtime.folder_entry(&deck).unwrap();
    press(&mut runtime, &deck, 2);
    let run = runtime.hold_folder_entry(&entry).unwrap();
    assert_eq!(runtime.folder_action_ran(&entry), None);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    // Even if the run itself could not start, the action ran.
    assert!(matches!(
        runtime.release_folder_hold(&run, HoldEnd::NotRun),
        Some(RuntimeOutcome::Navigated { page, .. }) if page == f.home
    ));
}

#[test]
fn exit_after_completion_leaves_when_the_run_is_over_not_when_it_starts() {
    let mut f = fixture();
    tools_with(&mut f, |tools| {
        tools.auto_return = true;
        tools.auto_exit = Some(AutoExit::disabled());
    });
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    // No countdown: the plain Back key.
    assert_eq!(runtime.folder_countdown(&deck), None);
    assert_eq!(back_face(&runtime, &deck), system_back_face());
    let run = start_run(&mut runtime, &deck);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    clock.advance(600);
    assert_eq!(back_face(&runtime, &deck), system_back_face());
    assert_eq!(
        runtime.release_folder_hold(&run, HoldEnd::Ran),
        Some(RuntimeOutcome::Navigated {
            device: deck.clone(),
            page: f.home
        })
    );
    assert_eq!(runtime.current_page(&deck), Some(f.home));
    // Released once only: a second release does not navigate again.
    assert_eq!(runtime.release_folder_hold(&run, HoldEnd::Ran), None);
    assert_eq!(runtime.current_page(&deck), Some(f.home));
}

#[test]
fn a_run_that_could_not_start_stays_in_the_folder_and_resumes() {
    let mut f = fixture();
    tools_with(&mut f, |tools| tools.auto_return = true);
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    clock.advance(2);
    let run = start_run(&mut runtime, &deck);
    let now = clock.advance(5);
    assert_eq!(runtime.release_folder_hold(&run, HoldEnd::NotRun), None);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    assert_eq!(
        runtime.next_folder_exit(),
        Some(now + Duration::from_secs(8))
    );
}

#[test]
fn with_both_on_a_run_exits_at_its_end_and_without_one_the_time_does() {
    let mut f = fixture();
    tools_with(&mut f, |tools| tools.auto_return = true);
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    clock.advance(3);
    let run = start_run(&mut runtime, &deck);
    clock.advance(20);
    assert!(matches!(
        runtime.release_folder_hold(&run, HoldEnd::Ran),
        Some(RuntimeOutcome::Navigated { page, .. }) if page == f.home
    ));
    // Right away, and nothing of the countdown is left to resume.
    assert_eq!(runtime.next_folder_exit(), None);
    assert_eq!(runtime.next_folder_wake(), None);
    // No run: the time alone still closes it.
    press(&mut runtime, &deck, 1);
    let now = clock.advance(10);
    assert_eq!(expire(&mut runtime, now), 1);
    assert_eq!(runtime.current_page(&deck), Some(f.home));
}

#[test]
fn with_both_off_runs_change_nothing() {
    let mut f = fixture();
    tools_with(&mut f, |tools| tools.auto_exit = Some(AutoExit::disabled()));
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    let run = start_run(&mut runtime, &deck);
    assert_eq!(back_face(&runtime, &deck), system_back_face());
    assert_eq!(runtime.release_folder_hold(&run, HoldEnd::Ran), None);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    assert_eq!(runtime.next_folder_wake(), None);
}

#[test]
fn concurrent_runs_hold_until_the_last_one_ends_and_exit_once() {
    let mut f = fixture();
    tools_with(&mut f, |tools| tools.auto_return = true);
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    clock.advance(1);
    let first = start_run(&mut runtime, &deck);
    let second = start_run(&mut runtime, &deck);
    assert_ne!(first.id, second.id);
    assert_eq!(runtime.folder_timers().holds_of(&deck, f.tools), 2);
    // One of two over: still paused, still inside.
    clock.advance(30);
    assert_eq!(runtime.release_folder_hold(&first, HoldEnd::Ran), None);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    assert_eq!(runtime.folder_countdown(&deck), countdown(9, true));
    assert_eq!(runtime.next_folder_exit(), None);
    // The first again changes nothing; the second ends it.
    assert_eq!(runtime.release_folder_hold(&first, HoldEnd::Ran), None);
    assert!(matches!(
        runtime.release_folder_hold(&second, HoldEnd::NotRun),
        Some(RuntimeOutcome::Navigated { page, .. }) if page == f.home
    ));
    assert_eq!(runtime.release_folder_hold(&second, HoldEnd::Ran), None);
    assert_eq!(runtime.current_page(&deck), Some(f.home));
}

#[test]
fn leaving_the_folder_by_hand_makes_its_runs_unable_to_navigate() {
    let mut f = fixture();
    tools_with(&mut f, |tools| tools.auto_return = true);
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    let run = start_run(&mut runtime, &deck);
    // The protected Back while the run goes on.
    press(&mut runtime, &deck, 0);
    assert_eq!(runtime.current_page(&deck), Some(f.home));
    assert_eq!(runtime.release_folder_hold(&run, HoldEnd::Ran), None);
    assert_eq!(runtime.current_page(&deck), Some(f.home));
    // Entered again: the old run's end does nothing to the new entry.
    press(&mut runtime, &deck, 1);
    let old = start_run(&mut runtime, &deck);
    press(&mut runtime, &deck, 0);
    press(&mut runtime, &deck, 1);
    assert_eq!(runtime.release_folder_hold(&old, HoldEnd::Ran), None);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    assert_eq!(runtime.folder_countdown(&deck), countdown(10, false));
    assert_eq!(runtime.fork_folder_hold(&old), None);
}

#[test]
fn another_folder_page_layout_profile_or_a_disconnect_make_runs_stale() {
    let mut f = fixture();
    tools_with(&mut f, |tools| tools.auto_return = true);
    let clock = clock();
    // Another folder opened inside: the run's end does not leave it, and
    // Tools stays paused until it is shown and the run is over.
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    let run = start_run(&mut runtime, &deck);
    press(&mut runtime, &deck, 1);
    assert_eq!(runtime.current_page(&deck), Some(f.docker));
    assert_eq!(runtime.release_folder_hold(&run, HoldEnd::Ran), None);
    assert_eq!(runtime.current_page(&deck), Some(f.docker));
    press(&mut runtime, &deck, 0);
    assert_eq!(runtime.folder_countdown(&deck), countdown(10, false));
    // Another page.
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    let run = start_run(&mut runtime, &deck);
    runtime
        .navigate(
            &deck,
            &crate::EditorTarget::Show {
                layout: f.layout,
                stack: vec![f.other],
            },
        )
        .unwrap();
    assert_eq!(runtime.release_folder_hold(&run, HoldEnd::Ran), None);
    assert_eq!(runtime.current_page(&deck), Some(f.other));
    // Another layout (a key of the folder switches it).
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    let run = start_run(&mut runtime, &deck);
    press(&mut runtime, &deck, 3);
    assert_eq!(runtime.layout_of(&deck).unwrap().id, f.second);
    assert_eq!(runtime.release_folder_hold(&run, HoldEnd::Ran), None);
    assert_eq!(runtime.layout_of(&deck).unwrap().id, f.second);
    // Another profile, and back to this one.
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    let run = start_run(&mut runtime, &deck);
    runtime.set_profile(Some(Profile::new("Home")), None);
    runtime.set_profile(Some(f.profile.clone()), None);
    press(&mut runtime, &deck, 1);
    assert_eq!(runtime.release_folder_hold(&run, HoldEnd::Ran), None);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    // A disconnect: the device comes back on the folder, the run's end
    // leaves nothing.
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    let run = start_run(&mut runtime, &deck);
    runtime
        .handle_event(&DeviceEvent::Disconnected {
            device: deck.clone(),
        })
        .unwrap();
    assert_eq!(runtime.release_folder_hold(&run, HoldEnd::Ran), None);
    connect(&mut runtime, &deck);
    assert_eq!(runtime.release_folder_hold(&run, HoldEnd::Ran), None);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    // Locking.
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    let run = start_run(&mut runtime, &deck);
    runtime.set_profile(None, None);
    assert_eq!(runtime.release_folder_hold(&run, HoldEnd::Ran), None);
}

#[test]
fn a_run_in_a_nested_folder_holds_that_folder_only() {
    let mut f = fixture();
    let layout = f
        .profile
        .layouts
        .iter_mut()
        .find(|l| l.id == f.layout)
        .unwrap();
    let docker = layout.page_mut(f.docker).unwrap();
    docker.auto_exit = Some(AutoExit {
        enabled: true,
        seconds: 5,
    });
    docker.auto_return = true;
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    press(&mut runtime, &deck, 1);
    clock.advance(4);
    press(&mut runtime, &deck, 1);
    clock.advance(1);
    let run = start_run(&mut runtime, &deck);
    assert_eq!(runtime.folder_countdown(&deck), countdown(4, true));
    clock.advance(30);
    // Docker exits after its run, one level up: Tools goes on from 6 s.
    assert!(matches!(
        runtime.release_folder_hold(&run, HoldEnd::Ran),
        Some(RuntimeOutcome::Navigated { page, .. }) if page == f.tools
    ));
    assert_eq!(runtime.folder_countdown(&deck), countdown(6, false));
    // Tools has no Exit after action completes: its runs only pause it.
    let run = start_run(&mut runtime, &deck);
    assert_eq!(runtime.release_folder_hold(&run, HoldEnd::Ran), None);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    let now = clock.advance(6);
    assert_eq!(expire(&mut runtime, now), 1);
    assert_eq!(runtime.current_page(&deck), Some(f.home));
}

#[test]
fn a_page_outside_folders_has_nothing_to_hold() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    assert_eq!(runtime.hold_folder(&deck), None);
    press(&mut runtime, &deck, 3);
    assert_eq!(runtime.hold_folder(&deck), None);
}

#[test]
fn the_live_preview_shows_the_draft_countdown_and_exits_with_it() {
    let mut f = fixture();
    let clock = clock();
    let (mut runtime, deck) = started(&f, &clock);
    // The editor's draft switches Exit after action completes on and
    // shortens the time; the device runs the draft.
    tools_with(&mut f, |tools| {
        tools.auto_return = true;
    });
    let mut draft = f.profile.layout(f.layout).unwrap().clone();
    draft.page_mut(f.tools).unwrap().auto_exit = Some(AutoExit {
        enabled: true,
        seconds: 4,
    });
    runtime.set_working_layout(&deck, f.profile.id, draft);
    press(&mut runtime, &deck, 1);
    assert_eq!(
        back_face(&runtime, &deck),
        folder_back_face(countdown(4, false))
    );
    let run = start_run(&mut runtime, &deck);
    assert!(matches!(
        runtime.release_folder_hold(&run, HoldEnd::Ran),
        Some(RuntimeOutcome::Navigated { page, .. }) if page == f.home
    ));
}
