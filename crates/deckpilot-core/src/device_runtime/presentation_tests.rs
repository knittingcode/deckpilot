//! The presentation stack (`presentation.rs`): an overlay never changes
//! what a device showed (T-O1), keeps its folder and page and pauses Auto
//! Exit under it (T-O2), stays on its device and follows its priority
//! (T-O3), falls back like a reload when what it covered is gone (T-O4),
//! can always be dismissed (T-O5), gives way to the editor's live preview
//! (T-O6), and fences held keys so nothing runs across a switch (T-R10).

use std::sync::Mutex;
use std::time::Duration;

use deckpilot_device::{ControlInput, models};

use super::*;
use crate::testing::registry;
use crate::{AutoExit, FaceStyle, LayoutPage, Os, ParameterValue};

#[derive(Clone)]
struct TestClock(Arc<Mutex<Instant>>);

impl TestClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }

    fn advance(&self, seconds: u64) {
        *self.0.lock().unwrap() += Duration::from_secs(seconds);
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

fn bind(index: u16, trigger: Trigger, target: BindingTarget) -> ControlBinding {
    ControlBinding::new(key(index), trigger, target)
}

fn instance(name: &str) -> crate::ActionInstance {
    crate::ActionInstance {
        id: ActionInstanceId::new(),
        name: name.into(),
        action_type: crate::testing::echo_type(),
        parameters: ActionParameters::new(),
        description: String::new(),
        icon: None,
        label: None,
    }
}

/// The user's profile: home with key 0 running `run`, key 1 opening the
/// folder Tools (Auto Exit 10 s), key 5 running `run` on its release. Tools
/// has the reserved Back and key 2 running `run`.
struct Fixture {
    profile: Profile,
    home: PageId,
    tools: PageId,
}

fn fixture() -> Fixture {
    let mut profile = Profile::new("Work");
    let run = instance("Run");
    let run_id = run.id;
    profile.actions.push(run);
    let mut layout = DeviceLayout::new("Main", mk2());
    let home = layout.home_page;
    let mut tools = LayoutPage::folder("Tools", home);
    tools.auto_exit = Some(AutoExit {
        enabled: true,
        seconds: 10,
    });
    let tools_id = tools.id;
    tools
        .bindings
        .push(bind(2, Trigger::Press, BindingTarget::action(run_id)));
    layout.pages.push(tools);
    layout.page_mut(home).unwrap().bindings.extend([
        bind(0, Trigger::Press, BindingTarget::action(run_id)),
        bind(
            1,
            Trigger::Press,
            BindingTarget::OpenPage { page: tools_id },
        ),
        bind(5, Trigger::Release, BindingTarget::action(run_id)),
    ]);
    layout.explicit_tree = true;
    profile.layouts.push(layout);
    Fixture {
        profile,
        home,
        tools: tools_id,
    }
}

/// The overlay: Active (key 0 the microphone action, key 1 opens More, key
/// 4 Back, key 14 the protected dismiss) and Incoming.
struct Overlay {
    spec: OverlaySpec,
    active: PageId,
    incoming: PageId,
    more: PageId,
    mic: ActionInstanceId,
}

fn overlay() -> Overlay {
    let mut layout = DeviceLayout::new("Calls", mk2());
    let active = layout.home_page;
    let incoming = LayoutPage::new("Incoming");
    let more = LayoutPage::new("More");
    let (incoming_id, more_id) = (incoming.id, more.id);
    layout.pages.extend([incoming, more]);
    let mut mic = instance("Microphone");
    mic.parameters = ActionParameters::new().with("mode", ParameterValue::Text("toggle".into()));
    let mic_id = mic.id;
    let mut overrides = ActionParameters::new();
    overrides = overrides.with("label", ParameterValue::Text("Mic".into()));
    layout.page_mut(active).unwrap().bindings.extend([
        bind(
            0,
            Trigger::Press,
            BindingTarget::Action {
                action: mic_id,
                overrides,
            },
        ),
        bind(1, Trigger::Press, BindingTarget::OpenPage { page: more_id }),
        bind(4, Trigger::Press, BindingTarget::Back),
    ]);
    layout
        .page_mut(more_id)
        .unwrap()
        .bindings
        .push(bind(4, Trigger::Press, BindingTarget::Back));
    Overlay {
        spec: OverlaySpec {
            owner: OverlayOwner::Calls,
            layout,
            actions: vec![mic],
            page: active,
            dismiss: Some(key(14)),
        },
        active,
        incoming: incoming_id,
        more: more_id,
        mic: mic_id,
    }
}

fn started(f: &Fixture) -> (Runtime, TestClock, DeviceId) {
    let clock = TestClock(Arc::new(Mutex::new(Instant::now())));
    let mut runtime = Runtime::new(
        Engine::new(registry()),
        ModelCatalog::builtin(),
        MachineConfig::new(Os::current()),
    );
    let moved = clock.clone();
    runtime.set_clock(move || moved.now());
    runtime.set_profile(Some(f.profile.clone()), None);
    let deck = device("A");
    connect(&mut runtime, &deck);
    (runtime, clock, deck)
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

fn release(runtime: &mut Runtime, device: &DeviceId, index: u16) -> RuntimeOutcome {
    input(runtime, device, index, ControlInput::Release)
}

fn tap(runtime: &mut Runtime, device: &DeviceId, index: u16) -> RuntimeOutcome {
    let outcome = press(runtime, device, index);
    release(runtime, device, index);
    outcome
}

/// Everything of the device's own presentation the overlay must leave
/// alone.
#[derive(Debug, PartialEq)]
struct Own {
    profile: Option<ProfileId>,
    page: Option<PageId>,
    navigation: NavigationState,
    remembered: Vec<(ProfileId, DeviceId, PageId)>,
    device_layouts: Vec<(ProfileId, DeviceId, LayoutId)>,
    history: Vec<LayoutId>,
    working: bool,
}

fn own(runtime: &Runtime, device: &DeviceId) -> Own {
    Own {
        profile: runtime.profile().map(|p| p.id),
        page: runtime.current_page(device),
        navigation: runtime.navigation_state(device),
        remembered: runtime.remembered_pages(),
        device_layouts: runtime.device_layouts(),
        history: runtime.layout_history(device),
        working: runtime.working_layout(device).is_some(),
    }
}

fn is_run(outcome: &RuntimeOutcome) -> bool {
    matches!(outcome, RuntimeOutcome::Action { .. })
}

// ------------------------------------------------------------------ T-O1

#[test]
fn an_overlay_never_changes_the_devices_own_presentation() {
    let f = fixture();
    let (mut runtime, _, deck) = started(&f);
    tap(&mut runtime, &deck, 1);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    let before = own(&runtime, &deck);
    let o = overlay();
    let session = runtime.push_overlay(&deck, o.spec).unwrap();
    assert!(runtime.shows_overlay(&deck));
    // Working the overlay: its pages, its back, its actions.
    assert!(
        matches!(tap(&mut runtime, &deck, 1), RuntimeOutcome::Navigated { page, .. } if page == o.more)
    );
    assert_eq!(runtime.overlay_page(&deck).unwrap().1, o.more);
    assert!(
        matches!(tap(&mut runtime, &deck, 4), RuntimeOutcome::Navigated { page, .. } if page == o.active)
    );
    assert!(runtime.show_overlay_page(&deck, session, o.incoming));
    assert!(runtime.show_overlay_page(&deck, session, o.active));
    runtime.suspend_overlay(&deck, session);
    runtime.resume_overlay(&deck, session);
    assert_eq!(own(&runtime, &deck), before, "while shown");
    assert!(runtime.pop_overlay(&deck, session));
    assert!(!runtime.shows_overlay(&deck));
    assert_eq!(own(&runtime, &deck), before, "after it went");
    assert!(!runtime.pop_overlay(&deck, session), "only once");
}

#[test]
fn an_overlay_key_hands_its_action_to_the_owner_and_runs_nothing() {
    let f = fixture();
    let (mut runtime, _, deck) = started(&f);
    let o = overlay();
    let session = runtime.push_overlay(&deck, o.spec).unwrap();
    let outcome = tap(&mut runtime, &deck, 0);
    let RuntimeOutcome::OverlayAction {
        session: got,
        action,
        parameters,
        page,
        ..
    } = outcome
    else {
        panic!("{outcome:?}")
    };
    assert_eq!((got, action, page), (session, o.mic, o.active));
    // The instance's parameters, with the binding's overrides over them.
    assert_eq!(parameters.text("mode"), Some("toggle"));
    assert_eq!(parameters.text("label"), Some("Mic"));
    // Unbound keys of the overlay do nothing; never the key under it.
    assert!(matches!(
        tap(&mut runtime, &deck, 5),
        RuntimeOutcome::Ignored { .. }
    ));
    assert!(matches!(
        tap(&mut runtime, &deck, 2),
        RuntimeOutcome::Ignored { .. }
    ));
    // Gestures never run on the device's own page under an overlay.
    assert!(runtime.key_gestures(&deck, key(0)).is_none());
    assert!(matches!(
        runtime
            .dispatch_gesture_as(
                &deck,
                key(0),
                crate::gesture::Gesture::Click,
                crate::RunOrigin::Gesture
            )
            .unwrap(),
        RuntimeOutcome::Ignored { .. }
    ));
}

#[test]
fn faces_are_the_overlays_with_the_owners_over_them() {
    let f = fixture();
    let (mut runtime, _, deck) = started(&f);
    let o = overlay();
    let session = runtime.push_overlay(&deck, o.spec).unwrap();
    runtime.set_overlay_faces(
        &deck,
        session,
        [(key(0), KeyFace::new(FaceStyle::Notice, "Muted"))],
    );
    let faces = runtime.faces(&deck);
    let face = |index: u16| {
        faces
            .iter()
            .find(|(c, _)| *c == key(index))
            .unwrap()
            .1
            .clone()
    };
    assert_eq!(face(0).title, "Muted");
    // The protected dismiss key shows Back.
    assert_eq!(face(14), system_back_face());
    assert_eq!(runtime.shown_targets(&deck).len(), 0);
    runtime.pop_overlay(&deck, session);
    assert_ne!(runtime.faces(&deck), faces);
}

// ------------------------------------------------------------------ T-O2

#[test]
fn the_folder_and_page_stay_and_auto_exit_pauses_under_an_overlay() {
    let f = fixture();
    let (mut runtime, clock, deck) = started(&f);
    tap(&mut runtime, &deck, 1);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    clock.advance(4);
    let session = runtime.push_overlay(&deck, overlay().spec).unwrap();
    // A long call: the folder under it never exits.
    clock.advance(60);
    assert_eq!(runtime.due_folder_exits(clock.now()).len(), 0);
    runtime.pop_overlay(&deck, session);
    assert_eq!(runtime.current_page(&deck), Some(f.tools));
    // The countdown goes on with what it had: 6 s.
    let left = runtime
        .folder_timers()
        .left_of(&deck, f.tools, clock.now())
        .unwrap();
    assert_eq!(left, Duration::from_secs(6));
    clock.advance(6);
    let exits = runtime.due_folder_exits(clock.now());
    assert_eq!(exits.len(), 1);
    assert!(runtime.exit_folder(&exits[0]).is_some());
    assert_eq!(runtime.current_page(&deck), Some(f.home));
}

#[test]
fn a_suspended_overlay_shows_the_device_and_its_countdown_runs() {
    let f = fixture();
    let (mut runtime, clock, deck) = started(&f);
    tap(&mut runtime, &deck, 1);
    let session = runtime.push_overlay(&deck, overlay().spec).unwrap();
    clock.advance(30);
    runtime.suspend_overlay(&deck, session);
    assert!(!runtime.shows_overlay(&deck));
    // The device's own keys work while suspended.
    assert!(is_run(&tap(&mut runtime, &deck, 2)));
    assert!(runtime.overlays(&deck)[0].suspended);
    assert!(runtime.resume_overlay(&deck, session));
    assert!(runtime.shows_overlay(&deck));
}

// ------------------------------------------------------------------ T-O3

#[test]
fn overlays_belong_to_their_device() {
    let f = fixture();
    let (mut runtime, _, deck) = started(&f);
    let other = device("B");
    connect(&mut runtime, &other);
    runtime.push_overlay(&deck, overlay().spec).unwrap();
    assert!(runtime.shows_overlay(&deck));
    assert!(!runtime.shows_overlay(&other));
    assert!(is_run(&tap(&mut runtime, &other, 0)));
    // Only a layout for the device's model, with the page asked for.
    let mut plus = overlay().spec;
    plus.layout.model = DeviceModelId::new(models::STREAM_DECK_PLUS_ID);
    assert_eq!(
        runtime.push_overlay(&other, plus),
        Err(OverlayError::WrongModel)
    );
    let mut page = overlay().spec;
    page.page = PageId::new();
    assert_eq!(
        runtime.push_overlay(&other, page),
        Err(OverlayError::UnknownPage)
    );
}

#[test]
fn the_newest_frame_of_a_priority_is_on_top_and_frames_are_bounded() {
    let f = fixture();
    let (mut runtime, _, deck) = started(&f);
    let first = runtime.push_overlay(&deck, overlay().spec).unwrap();
    let second = runtime.push_overlay(&deck, overlay().spec).unwrap();
    let views = runtime.overlays(&deck);
    assert_eq!(
        views.iter().map(|v| v.session).collect::<Vec<_>>(),
        [first, second]
    );
    assert!(views[1].visible && !views[0].visible);
    assert!(views.iter().all(|v| v.priority == OverlayPriority::Calls));
    assert!(OverlayPriority::System > OverlayPriority::Calls);
    // A suspended top frame shows what is under it, never the lower frame
    // as if it were the call.
    runtime.suspend_overlay(&deck, second);
    assert!(runtime.overlays(&deck)[0].visible);
    runtime.pop_overlay(&deck, first);
    runtime.pop_overlay(&deck, second);
    for _ in 0..MAX_FRAMES {
        runtime.push_overlay(&deck, overlay().spec).unwrap();
    }
    assert_eq!(
        runtime.push_overlay(&deck, overlay().spec),
        Err(OverlayError::TooMany)
    );
    assert_eq!(
        runtime.pop_overlays_of(OverlayOwner::Calls).len(),
        MAX_FRAMES
    );
    assert_eq!(runtime.all_overlays().len(), 0);
}

// ------------------------------------------------------------------ T-O4

#[test]
fn what_was_covered_and_is_gone_falls_back_like_a_reload() {
    let f = fixture();
    let (mut runtime, _, deck) = started(&f);
    tap(&mut runtime, &deck, 1);
    let session = runtime.push_overlay(&deck, overlay().spec).unwrap();
    // The folder is deleted from the layout while the call goes on.
    let mut profile = f.profile.clone();
    profile.layouts[0].pages.retain(|page| page.id != f.tools);
    runtime.reload_profile(profile);
    runtime.pop_overlay(&deck, session);
    assert_eq!(runtime.current_page(&deck), Some(f.home));
    // The profile switched to another one meanwhile: the device shows it.
    let session = runtime.push_overlay(&deck, overlay().spec).unwrap();
    let mut other = Profile::new("Other");
    other.layouts.push(DeviceLayout::new("Other", mk2()));
    let other_home = other.layouts[0].home_page;
    runtime.set_profile(Some(other), None);
    assert!(runtime.shows_overlay(&deck), "frames are not profile data");
    runtime.pop_overlay(&deck, session);
    assert_eq!(runtime.current_page(&deck), Some(other_home));
}

// ------------------------------------------------------------------ T-O5

#[test]
fn the_dismiss_key_and_back_on_the_first_page_take_the_overlay_away() {
    let f = fixture();
    let (mut runtime, _, deck) = started(&f);
    let session = runtime.push_overlay(&deck, overlay().spec).unwrap();
    // On a page further in, Back goes back; the dismiss key always pops.
    tap(&mut runtime, &deck, 1);
    let outcome = tap(&mut runtime, &deck, 14);
    assert_eq!(
        outcome,
        RuntimeOutcome::OverlayDismissed {
            device: deck.clone(),
            session
        }
    );
    assert!(!runtime.shows_overlay(&deck));
    let session = runtime.push_overlay(&deck, overlay().spec).unwrap();
    assert!(matches!(
        tap(&mut runtime, &deck, 4),
        RuntimeOutcome::OverlayDismissed { session: s, .. } if s == session
    ));
    assert_eq!(runtime.all_overlays().len(), 0);
}

// ------------------------------------------------------------------ T-O6

#[test]
fn the_live_preview_wins_over_an_overlay() {
    let f = fixture();
    let (mut runtime, _, deck) = started(&f);
    let session = runtime.push_overlay(&deck, overlay().spec).unwrap();
    let mut draft = f.profile.layouts[0].clone();
    draft.name = "Draft".into();
    runtime.set_working_layout(&deck, f.profile.id, draft);
    assert!(!runtime.shows_overlay(&deck));
    assert!(is_run(&tap(&mut runtime, &deck, 0)), "the preview runs");
    assert!(runtime.overlays(&deck).iter().any(|v| v.session == session));
    runtime.clear_working_layout(&deck);
    assert!(runtime.shows_overlay(&deck));
}

#[test]
fn frames_stay_with_a_disconnected_device() {
    let f = fixture();
    let (mut runtime, _, deck) = started(&f);
    let session = runtime.push_overlay(&deck, overlay().spec).unwrap();
    runtime
        .handle_event(&DeviceEvent::Disconnected {
            device: deck.clone(),
        })
        .unwrap();
    assert_eq!(runtime.overlays(&deck)[0].session, session);
    connect(&mut runtime, &deck);
    assert!(runtime.shows_overlay(&deck));
}

// ----------------------------------------------------------------- T-R10

#[test]
fn a_press_on_one_presentation_never_releases_on_the_other() {
    let f = fixture();
    let (mut runtime, _, deck) = started(&f);
    let o = overlay();
    let session = runtime.push_overlay(&deck, o.spec).unwrap();
    // "Back to work" is pressed on the overlay; the owner suspends it at
    // once, while the key is still down.
    assert!(matches!(
        press(&mut runtime, &deck, 0),
        RuntimeOutcome::OverlayAction { .. }
    ));
    let generation = runtime.presentation_generation();
    runtime.suspend_overlay(&deck, session);
    assert!(runtime.presentation_generation() > generation);
    // Its release lands on the restored page: dropped, nothing runs.
    assert!(matches!(
        release(&mut runtime, &deck, 0),
        RuntimeOutcome::Ignored { .. }
    ));
    // The next press is the device's own again.
    assert!(is_run(&tap(&mut runtime, &deck, 0)));
}

#[test]
fn a_key_held_when_the_overlay_comes_finishes_on_nothing() {
    let f = fixture();
    let (mut runtime, _, deck) = started(&f);
    // Key 5 runs on its release; it is down when a call comes in.
    assert!(matches!(
        press(&mut runtime, &deck, 5),
        RuntimeOutcome::Ignored { .. }
    ));
    let session = runtime.push_overlay(&deck, overlay().spec).unwrap();
    // Its release neither runs the device's binding nor the overlay's.
    assert!(matches!(
        release(&mut runtime, &deck, 5),
        RuntimeOutcome::Ignored { .. }
    ));
    runtime.pop_overlay(&deck, session);
    // Not fenced any more: a full press and release runs it.
    press(&mut runtime, &deck, 5);
    assert!(is_run(&release(&mut runtime, &deck, 5)));
    // Held across the pop: the overlay's key does not finish underneath.
    let session = runtime.push_overlay(&deck, overlay().spec).unwrap();
    press(&mut runtime, &deck, 5);
    runtime.pop_overlay(&deck, session);
    assert!(matches!(
        release(&mut runtime, &deck, 5),
        RuntimeOutcome::Ignored { .. }
    ));
}
