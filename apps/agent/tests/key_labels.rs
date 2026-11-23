//! Key labels on the device (`docs/key-text.md`, "Labels"; capability
//! `keyLabels`, protocol 17): the device shows exactly the frames the
//! editor's preview draws (the parity test), variable changes redraw
//! coalesced (at most one redraw per device per 50 ms), a key press during
//! a storm of changes is handled at once, and a profile switch redraws the
//! labels with the new profile's texts.

mod support;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use deckpilot_core::variables::{
    DisplayText, ResolutionContext, ResolvedTemplates, ScopeKey, VariableDefinition, VariableKey,
    VariableScope, VariableType, VariableValue,
};
use deckpilot_core::{
    ActionParameters, BindingTarget, ControlBinding, DeviceLayout, KeyAppearance, KeyLabel,
    KeyLabelLook, LabelId, LabelLayer, LabelPosition, LayoutPage, ParameterValue, Profile, Project,
    ProjectKey, Spot, Trigger,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ControlId, DeviceId, Frame, models};
use deckpilot_ipc::{Capability, Request, Response};
use deckpilot_modules::general;
use deckpilot_storage::ConfigStore;
use support::{TIMEOUT, TempDir, TestAgent, data_dir, registry};

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn label(id: &str, text: &str, to: Spot) -> KeyLabel {
    KeyLabel {
        look: KeyLabelLook {
            position: LabelPosition::Anchored { to, inset: None },
            layer: Some(LabelLayer::OverTitle),
            ..KeyLabelLook::default()
        },
        ..KeyLabel::new(LabelId::new(id).unwrap(), text)
    }
}

/// The label templates of the home page, for the agent's answers.
const TEMPLATES: [&str; 5] = [
    "{{ shared.team.name }}",
    "{project.key}",
    "{profile}",
    "v2",
    "{page.number}/{page.count}",
];

/// Key 0: a title and two labels (a shared variable, the project), key 1:
/// a static label, key 2: opens the second page (whose key 0 is plain).
fn profile() -> Profile {
    let mut profile = Profile::new("Work");
    for project in ["SHOP", "CMR"] {
        profile
            .projects
            .push(Project::new(ProjectKey::new(project).unwrap(), project));
    }
    let log = registry()
        .create_instance(
            "Log",
            general::log_id(),
            &ActionParameters::new().with("message", ParameterValue::Text("hi".into())),
        )
        .unwrap();
    let log_id = log.id;
    profile.actions.push(log);
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let second = LayoutPage::new("Second");
    let second_id = second.id;
    let home = layout.home_page;
    let mut labelled = ControlBinding::new(key(0), Trigger::Press, BindingTarget::action(log_id));
    labelled.label = Some("Build".into());
    labelled.appearance = Some(KeyAppearance {
        labels: vec![
            label("team", TEMPLATES[0], Spot::Bottom),
            label("project", TEMPLATES[1], Spot::TopRight),
            label("profile", TEMPLATES[2], Spot::TopLeft),
        ],
        ..KeyAppearance::default()
    });
    let mut tagged = ControlBinding::new(key(1), Trigger::Press, BindingTarget::action(log_id));
    tagged.appearance = Some(KeyAppearance {
        labels: vec![
            label("tag", TEMPLATES[3], Spot::TopLeft),
            label("page", TEMPLATES[4], Spot::Bottom),
        ],
        ..KeyAppearance::default()
    });
    let mut open = ControlBinding::new(
        key(2),
        Trigger::Press,
        BindingTarget::OpenPage { page: second_id },
    );
    open.label = Some("Next".into());
    let page = layout.page_mut(home).unwrap();
    page.bindings.extend([labelled, tagged, open]);
    let mut back = ControlBinding::new(key(0), Trigger::Press, BindingTarget::action(log_id));
    back.label = Some("Second".into());
    let mut second = second;
    second.bindings.push(back);
    second.bindings.push(ControlBinding::new(
        key(2),
        Trigger::Press,
        BindingTarget::Back,
    ));
    layout.pages.push(second);
    let id = layout.id;
    profile.layouts.push(layout);
    profile.activate_layout(id).unwrap();
    profile
}

struct Setup {
    agent: TestAgent,
    mock: MockBackend,
    device: DeviceId,
    profile: Profile,
    _dir: TempDir,
}

async fn setup() -> Setup {
    let dir = TempDir::new("deckpilot-key-labels");
    let profile = profile();
    let (paths, db_key) = data_dir(&dir, &profile, &[]);
    let store = support::open_store(&paths, &db_key);
    store
        .set_active_project(profile.id, Some(profile.projects[0].id))
        .unwrap();
    let name = VariableKey::new("team.name").unwrap();
    store
        .save_shared_variable(&VariableDefinition::new(
            name.clone(),
            VariableType::String,
            &[VariableScope::Global],
        ))
        .unwrap();
    store
        .set_shared_variable_value(
            &name,
            &ScopeKey::Global,
            &VariableValue::String("Shop team".into()),
        )
        .unwrap();
    drop(store);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "LABELS-1");
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    Setup {
        agent,
        mock,
        device,
        profile,
        _dir: dir,
    }
}

impl Setup {
    /// What the agent answers for the templates of titles and labels.
    async fn resolve(&self) -> BTreeMap<String, DisplayText> {
        let templates: Vec<String> = TEMPLATES
            .iter()
            .chain(&["Build", "Next"])
            .map(|t| (*t).to_owned())
            .collect();
        let response = self
            .agent
            .request(Request::ResolveDisplayTemplates {
                context: Box::new(ResolutionContext::preview(self.profile.id)),
                templates,
            })
            .await
            .unwrap();
        let Response::DisplayTemplates { texts } = response else {
            panic!("display templates expected, got {response:?}");
        };
        texts
            .into_iter()
            .map(|answer| (answer.template, answer.display))
            .collect()
    }

    /// The home page's frames as the editor's preview draws them from the
    /// agent's answers (`preview_layout` at the native size).
    async fn previewed(&self) -> BTreeMap<ControlId, Frame> {
        let answers = self.resolve().await;
        let profile = self
            .agent
            .store()
            .load_profile(self.profile.id)
            .unwrap()
            .unwrap();
        let project = profile.project(
            self.agent
                .store()
                .active_project(profile.id)
                .unwrap()
                .unwrap(),
        );
        let model = models::stream_deck_mk2();
        let spec = model.capabilities.keys.and_then(|keys| keys.image).unwrap();
        let controls: Vec<ControlId> = (0..3).map(key).collect();
        let layout = &profile.layouts[0];
        let faces = deckpilot_core::core_display(&profile, project, |core| {
            deckpilot_core::page_faces_with(
                &registry(),
                layout,
                layout.home_page,
                &profile,
                project,
                &controls,
                Some(&model.capabilities),
                &ResolvedTemplates {
                    resolved: &answers,
                    fallback: core,
                },
            )
        });
        let icons = deckpilot_render::Icons::from_profile(&profile);
        faces
            .into_iter()
            .map(|(control, face)| {
                let frame = deckpilot_render::render_for(
                    &face,
                    spec.width,
                    spec.height,
                    &icons,
                    &model.effective_appearance(),
                );
                (control, frame)
            })
            .collect()
    }

    async fn device_shows_the_preview(&self) {
        let expected = self.previewed().await;
        assert!(
            expected.values().all(|frame| frame.width() == 72),
            "native size"
        );
        let mock = self.mock.clone();
        let device = self.device.clone();
        let shown = tokio::task::spawn_blocking(move || {
            mock.wait_for(&device, TIMEOUT, |frames, _| {
                expected
                    .iter()
                    .all(|(control, frame)| frames.get(control) == Some(frame))
            })
        })
        .await
        .unwrap();
        assert!(shown, "the device does not show what the preview shows");
    }

    fn frame(&self, control: ControlId) -> Option<Frame> {
        self.mock.frames(&self.device).get(&control).cloned()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_device_shows_the_previews_labels() {
    let s = setup().await;
    let answers = s.resolve().await;
    assert_eq!(answers["{{ shared.team.name }}"].text, "Shop team");
    assert_eq!(answers["{project.key}"].text, "SHOP");
    s.device_shows_the_preview().await;
    // The labels are drawn: the key differs from the same key without them.
    let plain = {
        let face = deckpilot_core::KeyFace::new(deckpilot_core::FaceStyle::Action, "Build");
        deckpilot_render::render(&face, 72, 72)
    };
    assert_ne!(s.frame(key(0)).unwrap(), plain);
    // The capability is announced.
    assert!(
        Capability::of_protocol(deckpilot_ipc::PROTOCOL_VERSION).contains(&Capability::KeyLabels)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_profile_switch_redraws_the_labels_with_its_texts() {
    let mut s = setup().await;
    s.device_shows_the_preview().await;
    let before = s.frame(key(0)).unwrap();
    // Another profile with the same keys: `{profile}` reads differently.
    let mut other = profile();
    other.name = "Home".into();
    let store = s.agent.store();
    store.save_profile(&other).unwrap();
    store
        .set_active_project(other.id, Some(other.projects[0].id))
        .unwrap();
    drop(store);
    let response = s
        .agent
        .request(Request::SetActiveProfile { profile: other.id })
        .await;
    assert!(response.is_ok(), "{response:?}");
    s.profile = other;
    assert_eq!(s.resolve().await["{profile}"].text, "Home");
    s.device_shows_the_preview().await;
    assert_ne!(s.frame(key(0)).unwrap(), before, "{{profile}} changed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn variable_changes_are_coalesced_per_device() {
    let s = setup().await;
    s.device_shows_the_preview().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let before = s.agent.handle.variable_redraws();
    // A burst of 100 changes of a variable key 0 shows.
    for _ in 0..100 {
        s.agent
            .handle
            .publish_variables_changed(vec!["team.name".into()]);
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let burst = s.agent.handle.variable_redraws() - before;
    assert!((1..=2).contains(&burst), "{burst} redraws for one burst");
    // A variable no key shows: nothing.
    let before = s.agent.handle.variable_redraws();
    s.agent
        .handle
        .publish_variables_changed(vec!["team.other".into()]);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(s.agent.handle.variable_redraws(), before);
}

/// Waits until key 2 shows `next` (the home page) or not (the second).
async fn wait_key2(s: &Setup, next: &Frame, on_home: bool) {
    let mock = s.mock.clone();
    let device = s.device.clone();
    let next = next.clone();
    let shown = tokio::task::spawn_blocking(move || {
        mock.wait_for(&device, TIMEOUT, |frames, _| {
            frames
                .get(&key(2))
                .is_some_and(|frame| (*frame == next) == on_home)
        })
    })
    .await
    .unwrap();
    assert!(shown, "the press was not handled");
}

/// The median of five presses of "Next", from the tap until the second
/// page shows (each followed by Back).
async fn press_latency(s: &Setup, next: &Frame) -> Duration {
    let mut times = Vec::new();
    for _ in 0..5 {
        s.mock.tap(&s.device, key(2));
        let start = Instant::now();
        wait_key2(s, next, false).await;
        times.push(start.elapsed());
        s.mock.tap(&s.device, key(2));
        wait_key2(s, next, true).await;
        // The home page shows the storm again.
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    times.sort();
    times[2]
}

/// Input latency is not affected by label updates. During a storm of
/// variable changes that change what key 0 shows (a new value every 2 ms,
/// read and announced as a plugin's publish would be), a key press opens
/// its page about as fast as without the storm (no press waits for a
/// redraw), while the storm redraws the key at most 20 times a second and
/// the key does change, and afterwards shows the final value. The latency
/// bound has a generous margin, as for titles (`title_variables.rs`), so a
/// loaded runner does not flake: a press that waited behind a redraw per
/// change would take seconds. How many redraws the storm must cause follows
/// how many changes it made while they were counted: a loaded runner writes
/// far fewer than one every 2 ms (40 in all was seen).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_press_during_a_storm_of_changes_is_handled_at_once() {
    let s = setup().await;
    s.device_shows_the_preview().await;
    // Key 2 is "Next" on the home page and Back on the second: what it
    // shows tells the pages apart while key 0 changes with the storm.
    let next = s.frame(key(2)).unwrap();
    let quiet = press_latency(&s, &next).await;

    let storm = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let raging = storm.clone();
    // The changes made so far, for the redraws they must cause.
    let changes = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let made = changes.clone();
    let handle = s.agent.handle.clone();
    let database = s.agent.store();
    let storming = tokio::spawn(async move {
        let name = VariableKey::new("team.name").unwrap();
        let mut round = 0_u32;
        while raging.load(std::sync::atomic::Ordering::Relaxed) {
            round += 1;
            database
                .set_shared_variable_value(
                    &name,
                    &ScopeKey::Global,
                    &VariableValue::String(format!("Team {round}")),
                )
                .unwrap();
            handle.reload_variables();
            made.store(round, std::sync::atomic::Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        round
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    // The key really changes during the storm.
    let early = s.frame(key(0)).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_ne!(s.frame(key(0)).unwrap(), early, "the storm is drawn");
    let stormy = press_latency(&s, &next).await;
    // The redraw rate on the home page, during the storm.
    let before = s.agent.handle.variable_redraws();
    let changed_before = changes.load(std::sync::atomic::Ordering::Relaxed);
    let counted = Instant::now();
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let redraws = s.agent.handle.variable_redraws() - before;
    let in_window = changes.load(std::sync::atomic::Ordering::Relaxed) - changed_before;
    let elapsed = counted.elapsed().as_secs_f64();
    storm.store(false, std::sync::atomic::Ordering::Relaxed);
    let rounds = storming.await.unwrap();
    println!(
        "press latency (median of 5): {quiet:?} quiet, {stormy:?} during a storm of {rounds} changes; \
         {redraws} variable redraws for {in_window} changes in {elapsed:.2} s"
    );
    assert!(
        stormy < quiet * 4 + Duration::from_millis(500),
        "{stormy:?} during the storm vs {quiet:?}"
    );
    #[expect(clippy::cast_precision_loss, reason = "a few dozen redraws")]
    let rate = redraws as f64 / elapsed;
    assert!(rate <= 21.0, "{redraws} redraws in {elapsed:.2} s");
    assert!(redraws >= 1, "the storm was never drawn");
    // A full storm (hundreds of changes) must be redrawn at least 5 times;
    // a loaded runner makes far fewer changes and redraws later, so one
    // redraw for every 10 changes it did make, at least one.
    let expected = usize::try_from(in_window / 10).unwrap_or(5).clamp(1, 5);
    assert!(
        redraws >= expected,
        "the storm was redrawn {redraws} times for {in_window} changes (expected {expected})"
    );
    // After the storm the device shows the final value.
    s.device_shows_the_preview().await;
}
