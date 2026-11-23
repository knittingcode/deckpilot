//! Variables in key titles end to end (`docs/variables.md`, "Variables in
//! key titles"; protocol 17): the device shows the titles the editor's
//! preview resolves (`resolveDisplayTemplates`), a variable change redraws
//! only the keys that use it, an unrelated change redraws nothing, a
//! disabled module's variable shows the marker, secrets never leave the
//! agent, and saved layouts are not touched.

mod support;

use std::collections::BTreeMap;
use std::time::Duration;

use deckpilot_core::variables::{
    CatalogFilter, DisplayText, ResolutionContext, ResolvedTemplates, ScopeKey, UnresolvedReason,
    VariableDefinition, VariableKey, VariableScope, VariableType, VariableValue,
};
use deckpilot_core::{
    ActionParameters, BindingTarget, ControlBinding, DeviceLayout, ModuleId, ModuleSettings,
    ParameterValue, Profile, Project, ProjectKey, Trigger,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ControlId, DeviceId, Frame, models};
use deckpilot_ipc::{Request, Response};
use deckpilot_modules::general;
use deckpilot_storage::{ConfigStore, SqliteStore};
use support::{TIMEOUT, TempDir, TestAgent, data_dir, registry};

const TITLES: [&str; 6] = [
    "Build",
    "Projekt: {project.key}",
    "{{ shared.team.name }}",
    "{{ system.profile.current.name }} — {{ shared.team.size }}",
    "PIN {{ shared.vault.pin }}",
    "HTTP {{ shared.http.flag }}",
];

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn variable(text: &str) -> VariableKey {
    VariableKey::new(text).unwrap()
}

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
    let home = layout.home_page;
    layout
        .page_mut(home)
        .unwrap()
        .bindings
        .extend(TITLES.iter().enumerate().map(|(index, title)| {
            let mut binding = ControlBinding::new(
                key(u16::try_from(index).unwrap()),
                Trigger::Press,
                BindingTarget::action(log_id),
            );
            binding.label = Some((*title).to_owned());
            binding
        }));
    let id = layout.id;
    profile.layouts.push(layout);
    profile.activate_layout(id).unwrap();
    profile
}

fn store_variables(store: &SqliteStore, profile: &Profile) {
    let define = |definition: VariableDefinition| store.save_shared_variable(&definition).unwrap();
    let set = |name: &str, scope: ScopeKey, value: VariableValue| {
        store
            .set_shared_variable_value(&variable(name), &scope, &value)
            .unwrap();
    };
    define(VariableDefinition::new(
        variable("team.name"),
        VariableType::String,
        &[VariableScope::Global, VariableScope::Project],
    ));
    set(
        "team.name",
        ScopeKey::Project {
            profile: profile.id,
            project: profile.projects[0].id,
        },
        VariableValue::String("Shop team".into()),
    );
    define(VariableDefinition::new(
        variable("team.size"),
        VariableType::Integer,
        &[VariableScope::Global],
    ));
    set("team.size", ScopeKey::Global, VariableValue::Integer(4));
    define(VariableDefinition::new(
        variable("team.other"),
        VariableType::String,
        &[VariableScope::Global],
    ));
    define(VariableDefinition {
        sensitive: true,
        ..VariableDefinition::new(
            variable("vault.pin"),
            VariableType::String,
            &[VariableScope::Global],
        )
    });
    set(
        "vault.pin",
        ScopeKey::Global,
        VariableValue::String("s3cr3t-4711".into()),
    );
    let conflicts = store
        .declare_module_variables(
            &ModuleId::new("http").unwrap(),
            &[VariableDefinition {
                default: Some(VariableValue::Boolean(true)),
                ..VariableDefinition::new(
                    variable("http.flag"),
                    VariableType::Boolean,
                    &[VariableScope::Global],
                )
            }],
        )
        .unwrap();
    assert!(conflicts.is_empty(), "{conflicts:?}");
}

fn set_http(store: &SqliteStore, enabled: bool) {
    store
        .set_module_state(
            &ModuleId::new("http").unwrap(),
            &ModuleSettings {
                enabled,
                config: ActionParameters::new(),
            },
        )
        .unwrap();
}

struct Setup {
    agent: TestAgent,
    mock: MockBackend,
    device: DeviceId,
    profile: Profile,
    _dir: TempDir,
}

async fn setup() -> Setup {
    let dir = TempDir::new("deckpilot-title-variables");
    let profile = profile();
    let (paths, db_key) = data_dir(&dir, &profile, &[]);
    let store = support::open_store(&paths, &db_key);
    store
        .set_active_project(profile.id, Some(profile.projects[0].id))
        .unwrap();
    set_http(&store, false);
    store_variables(&store, &profile);
    drop(store);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "TITLES-1");
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
    fn context(&self) -> ResolutionContext {
        ResolutionContext::preview(self.profile.id)
    }

    /// What the agent answers for the layout's titles, by template.
    async fn resolve(&self) -> BTreeMap<String, DisplayText> {
        let response = self
            .agent
            .request(Request::ResolveDisplayTemplates {
                context: Box::new(self.context()),
                templates: TITLES.map(str::to_owned).to_vec(),
            })
            .await
            .unwrap();
        let json = serde_json::to_string(&response).unwrap();
        assert!(!json.contains("s3cr3t"), "a secret left the agent: {json}");
        let Response::DisplayTemplates { texts } = response else {
            panic!("display templates expected, got {response:?}");
        };
        texts
            .into_iter()
            .map(|answer| (answer.template, answer.display))
            .collect()
    }

    /// The frames the editor's preview draws from the agent's answers.
    async fn previewed(&self) -> BTreeMap<ControlId, Frame> {
        let answers = self.resolve().await;
        let profile = self
            .agent
            .store()
            .load_profile(self.profile.id)
            .unwrap()
            .unwrap();
        let project = profile.project(profile.projects[0].id);
        let model = models::stream_deck_mk2();
        let spec = model.capabilities.keys.and_then(|keys| keys.image).unwrap();
        let controls: Vec<ControlId> = (0..6).map(key).collect();
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
                (
                    control,
                    deckpilot_render::render_for(
                        &face,
                        spec.width,
                        spec.height,
                        &icons,
                        &model.effective_appearance(),
                    ),
                )
            })
            .collect()
    }

    /// Waits until the device shows what the preview draws.
    async fn device_shows_the_preview(&self) {
        let expected = self.previewed().await;
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

    fn writes(&self) -> BTreeMap<ControlId, usize> {
        self.mock.control_writes(&self.device)
    }

    async fn reload(&self) {
        let response = self.agent.request(Request::ReloadConfiguration).await;
        assert!(response.is_ok(), "{response:?}");
    }

    /// The writes after a change settled.
    async fn settled_writes(&self) -> BTreeMap<ControlId, usize> {
        tokio::time::sleep(Duration::from_millis(400)).await;
        self.writes()
    }
}

fn changed(before: &BTreeMap<ControlId, usize>, after: &BTreeMap<ControlId, usize>) -> Vec<u16> {
    after
        .iter()
        .filter(|(control, count)| before.get(*control) != Some(*count))
        .filter_map(|(control, _)| match control {
            ControlId::Key { index } => Some(*index),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_device_shows_resolved_titles_and_redraws_only_what_changed() {
    let s = setup().await;
    let answers = s.resolve().await;
    let text = |template: &str| answers[template].text.clone();
    assert_eq!(text("Build"), "Build");
    assert_eq!(text("Projekt: {project.key}"), "Projekt: SHOP");
    assert_eq!(text("{{ shared.team.name }}"), "Shop team");
    assert_eq!(
        text("{{ system.profile.current.name }} — {{ shared.team.size }}"),
        "Work — 4"
    );
    assert_eq!(text("PIN {{ shared.vault.pin }}"), "PIN -");
    assert_eq!(
        answers["PIN {{ shared.vault.pin }}"].unresolved[0].reason,
        UnresolvedReason::NotDisplayable
    );
    // The HTTP module is disabled: its variable shows the marker.
    assert_eq!(text("HTTP {{ shared.http.flag }}"), "HTTP -");
    assert_eq!(
        answers["HTTP {{ shared.http.flag }}"].unresolved[0].reason,
        UnresolvedReason::NotDisplayable
    );
    s.device_shows_the_preview().await;

    // A value one key uses: only that key is drawn again.
    let before = s.settled_writes().await;
    s.agent
        .store()
        .set_shared_variable_value(
            &variable("team.size"),
            &ScopeKey::Global,
            &VariableValue::Integer(9),
        )
        .unwrap();
    s.reload().await;
    s.device_shows_the_preview().await;
    assert_eq!(changed(&before, &s.settled_writes().await), [3]);

    // A variable no key uses: nothing is drawn.
    let before = s.settled_writes().await;
    s.agent
        .store()
        .set_shared_variable_value(
            &variable("team.other"),
            &ScopeKey::Global,
            &VariableValue::String("unrelated".into()),
        )
        .unwrap();
    s.reload().await;
    assert_eq!(
        changed(&before, &s.settled_writes().await),
        Vec::<u16>::new()
    );

    // The module comes back: its key shows the value, and only it changes.
    let before = s.settled_writes().await;
    set_http(&s.agent.store(), true);
    s.reload().await;
    assert_eq!(
        s.resolve().await["HTTP {{ shared.http.flag }}"].text,
        "HTTP true"
    );
    s.device_shows_the_preview().await;
    assert_eq!(changed(&before, &s.settled_writes().await), [5]);
    // And goes again: no stale value.
    set_http(&s.agent.store(), false);
    s.reload().await;
    assert_eq!(
        s.resolve().await["HTTP {{ shared.http.flag }}"].text,
        "HTTP -"
    );
    s.device_shows_the_preview().await;

    // The secret is in no log line.
    let log = support::agent_log::test_log();
    assert!(!log.contains("s3cr3t"), "a secret was logged");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_picker_lists_display_safe_variables_only() {
    let s = setup().await;
    let response = s
        .agent
        .request(Request::ListDisplayVariables {
            context: Box::new(s.context()),
            filter: Box::new(CatalogFilter {
                include_unavailable: true,
                ..CatalogFilter::default()
            }),
        })
        .await
        .unwrap();
    let Response::Variables { entries } = response else {
        panic!("variables expected");
    };
    let listed: Vec<String> = entries.iter().map(|e| e.reference.to_string()).collect();
    for wanted in [
        "shared.team.name",
        "shared.team.size",
        "system.profile.current.name",
        "system.project.current.key",
    ] {
        assert!(listed.iter().any(|r| r == wanted), "{wanted}: {listed:?}");
    }
    for refused in [
        "shared.vault.pin",
        "workflow.execution.id",
        "workflow.id",
        "system.device.current.id",
        "system.profile.current.id",
        "system.work.session.state",
    ] {
        assert!(
            !listed.iter().any(|r| r == refused),
            "{refused}: {listed:?}"
        );
    }
    let json = serde_json::to_string(&entries).unwrap();
    assert!(!json.contains("s3cr3t"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_layouts_are_not_touched() {
    let s = setup().await;
    let stored = |s: &Setup| {
        let profile = s.agent.store().load_profile(s.profile.id).unwrap().unwrap();
        serde_json::to_string(&profile.layouts).unwrap()
    };
    let before = stored(&s);
    assert_eq!(before, serde_json::to_string(&s.profile.layouts).unwrap());
    s.device_shows_the_preview().await;
    s.reload().await;
    s.device_shows_the_preview().await;
    assert_eq!(stored(&s), before, "the agent changed a saved layout");
    // A layout as an older version saved it reads and writes the same bytes.
    let layout = &s.profile.layouts[0];
    let json = serde_json::to_string(layout).unwrap();
    let again: DeviceLayout = serde_json::from_str(&json).unwrap();
    assert_eq!(serde_json::to_string(&again).unwrap(), json);
    let labels: Vec<_> = again.pages[0]
        .bindings
        .iter()
        .map(|b| b.label.clone().unwrap())
        .collect();
    assert_eq!(labels, TITLES);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn too_many_templates_are_refused() {
    let s = setup().await;
    let refused = s
        .agent
        .request(Request::ResolveDisplayTemplates {
            context: Box::new(s.context()),
            templates: vec![
                "x".into();
                deckpilot_ipc::display_templates::MAX_DISPLAY_TEMPLATES + 1
            ],
        })
        .await;
    assert!(refused.is_err(), "{refused:?}");
}
