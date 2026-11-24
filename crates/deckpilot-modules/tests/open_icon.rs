//! The automatic icon of Open keys (`docs/system-actions.md`, "The key's
//! icon"): a document for a file, the open folder for a folder, Open's own
//! document icon when nothing is chosen or the target cannot be told; the
//! user's icon or image always wins and is never touched; nothing
//! automatic is stored. The device ([`Runtime::display_faces`]) and the
//! app's previews ([`page_faces_scoped`] with a [`TargetCache`]) compute
//! it in the same function and draw the same frames.

use std::path::{Path, PathBuf};

use deckpilot_core::device_runtime::page_faces_scoped;
use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, AssetRef, BindingTarget,
    ControlBinding, DeviceLayout, Engine, IconFit, IconRef, KeyAppearance, KeyFace, MachineConfig,
    MachineResourceBinding, Os, ParameterValue, Profile, Project, ProjectKey, ResourceKey,
    ResourceKind, ResourceRef, Runtime, TargetCache, Trigger, WorkflowScope, core_display,
};
use deckpilot_device::{ControlId, DeviceEvent, DeviceId, DeviceModelId, ModelCatalog, models};
use deckpilot_modules::{builtin, system};

const FILE: &str = system::actions::FILE_ICON;
const FOLDER: &str = system::actions::FOLDER_ICON;

const KEY: ControlId = ControlId::Key { index: 0 };

/// A fresh folder with `report.pdf` and `docs/README.md` in it.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "deckpilot-open-icon-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("report.pdf"), b"%PDF").unwrap();
        std::fs::write(dir.join("docs/README.md"), b"# docs").unwrap();
        Self(dir)
    }

    fn path(&self, relative: &str) -> String {
        let path: &Path = &self.0;
        if relative.is_empty() {
            path.to_string_lossy().into_owned()
        } else {
            path.join(relative).to_string_lossy().into_owned()
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn key(value: &str) -> ResourceKey {
    ResourceKey::new(value).unwrap()
}

fn fixed(value: &str) -> ParameterValue {
    ParameterValue::Resource(ResourceRef::Fixed { key: key(value) })
}

fn machine(bindings: &[(&str, ResourceKind, String)]) -> MachineConfig {
    let mut machine = MachineConfig::new(Os::current());
    for (k, kind, value) in bindings {
        machine.set_binding(MachineResourceBinding {
            key: key(k),
            kind: *kind,
            value: value.clone(),
        });
    }
    machine
}

/// An action of `action_type` with `parameters`, with the icon new actions
/// copy from their type unless `icon` is given.
fn action(action_type: &str, parameters: ActionParameters, icon: Option<&str>) -> ActionInstance {
    let type_icon = if action_type == system::actions::OPEN_DIRECTORY {
        FOLDER
    } else {
        FILE
    };
    ActionInstance {
        id: ActionInstanceId::new(),
        name: "Open".into(),
        action_type: ActionTypeId::new(action_type).unwrap(),
        parameters,
        description: String::new(),
        icon: Some(icon.unwrap_or(type_icon).into()),
        label: None,
    }
}

fn open(parameters: ActionParameters) -> ActionInstance {
    action(system::actions::OPEN_FILE, parameters, None)
}

fn location(value: &str) -> ActionParameters {
    ActionParameters::new().with("location", fixed(value))
}

/// A profile whose layout runs `instance` on key 0's press, with `look`.
fn profile(instance: ActionInstance, look: Option<KeyAppearance>) -> Profile {
    let mut profile = Profile::new("Work");
    let id = instance.id;
    profile.actions.push(instance);
    let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
    let home = layout.home_page;
    layout
        .page_mut(home)
        .unwrap()
        .bindings
        .push(ControlBinding {
            appearance: look,
            ..ControlBinding::new(KEY, Trigger::Press, BindingTarget::action(id))
        });
    profile.layouts.push(layout);
    profile
}

fn engine() -> Engine {
    Engine::new(builtin(Os::current()).action_registry())
}

fn device() -> DeviceId {
    DeviceId {
        model: DeviceModelId::new(models::STREAM_DECK_MK2_ID),
        serial: "OPEN-1".into(),
    }
}

/// The device's runtime, showing `profile`.
fn runtime(profile: &Profile, machine: MachineConfig) -> Runtime {
    let mut runtime = Runtime::new(engine(), ModelCatalog::builtin(), machine);
    runtime.set_profile(Some(profile.clone()), None);
    runtime
        .handle_event(&DeviceEvent::Connected { device: device() })
        .unwrap();
    runtime
}

/// Key 0 as the device shows it.
fn device_face(runtime: &Runtime) -> KeyFace {
    runtime
        .faces(&device())
        .into_iter()
        .find(|(control, _)| *control == KEY)
        .map(|(_, face)| face)
        .unwrap()
}

/// Key 0 as the app's preview computes it (`preview_layout`): the same
/// function, with the app's own cache.
fn preview_face(
    profile: &Profile,
    machine: &MachineConfig,
    project: Option<&Project>,
    cache: &TargetCache,
) -> KeyFace {
    let model = models::stream_deck_mk2();
    let registry = builtin(Os::current()).action_registry();
    let layout = &profile.layouts[0];
    core_display(profile, project, |titles| {
        page_faces_scoped(
            &registry,
            layout,
            layout.home_page,
            profile,
            project,
            &model.capabilities.controls(),
            Some(&model.capabilities),
            titles,
            &WorkflowScope::default(),
            &cache.on(machine, project),
        )
    })
    .into_iter()
    .find(|(control, _)| *control == KEY)
    .map(|(_, face)| face)
    .unwrap()
}

/// The symbol of key 0 on the device and in a fresh preview, checked to
/// be the same.
fn symbol(profile: &Profile, machine: &MachineConfig) -> String {
    let on_device = device_face(&runtime(profile, machine.clone()));
    let previewed = preview_face(profile, machine, None, &TargetCache::new());
    assert_eq!(on_device, previewed, "the device and the preview agree");
    on_device.symbol.unwrap()
}

fn custom_look() -> KeyAppearance {
    KeyAppearance {
        icon: Some(IconRef {
            id: AssetRef::parse("deckpilot.house").unwrap(),
            fit: IconFit::Fit,
        }),
        ..KeyAppearance::default()
    }
}

#[test]
fn a_chosen_file_shows_the_document_and_a_folder_the_open_folder() {
    let dir = Scratch::new("choose");
    let machine = machine(&[
        (
            "file.report-abc123",
            ResourceKind::Other,
            dir.path("report.pdf"),
        ),
        ("folder.work-abc123", ResourceKind::Directory, dir.path("")),
    ]);
    assert_eq!(
        symbol(
            &profile(open(location("file.report-abc123")), None),
            &machine
        ),
        FILE
    );
    assert_eq!(
        symbol(
            &profile(open(location("folder.work-abc123")), None),
            &machine
        ),
        FOLDER
    );
    // A file or a folder inside a chosen folder (`path`).
    let inside =
        |path: &str| location("folder.work-abc123").with("path", ParameterValue::Text(path.into()));
    assert_eq!(
        symbol(&profile(open(inside("docs")), None), &machine),
        FOLDER
    );
    assert_eq!(
        symbol(&profile(open(inside("docs/README.md")), None), &machine),
        FILE
    );
    // An action without a stored icon (older or hand-made) gets it too.
    let mut bare = open(location("folder.work-abc123"));
    bare.icon = None;
    assert_eq!(symbol(&profile(bare, None), &machine), FOLDER);
}

#[test]
fn the_icon_follows_the_target_when_it_changes() {
    let dir = Scratch::new("switch");
    let file = (
        "file.target-abc123",
        ResourceKind::Other,
        dir.path("report.pdf"),
    );
    let folder = (
        "folder.target-abc123",
        ResourceKind::Directory,
        dir.path("docs"),
    );
    let profile_with = |value: &str| profile(open(location(value)), None);
    // File to folder by choosing again (another binding), and back.
    let bindings = machine(&[file.clone(), folder.clone()]);
    assert_eq!(symbol(&profile_with(file.0), &bindings), FILE);
    assert_eq!(symbol(&profile_with(folder.0), &bindings), FOLDER);
    assert_eq!(symbol(&profile_with(file.0), &bindings), FILE);

    // On a running device: the profile is reloaded after the action was
    // saved with another target, and the key follows.
    let mut runtime = runtime(&profile_with(file.0), bindings.clone());
    assert_eq!(device_face(&runtime).symbol.as_deref(), Some(FILE));
    let mut changed = runtime.profile().unwrap().clone();
    changed.actions[0].parameters = location(folder.0);
    runtime.reload_profile(changed.clone());
    assert_eq!(device_face(&runtime).symbol.as_deref(), Some(FOLDER));
    changed.actions[0].parameters = location(file.0);
    runtime.reload_profile(changed);
    assert_eq!(device_face(&runtime).symbol.as_deref(), Some(FILE));

    // The same key, its resource bound elsewhere (Settings › Resources).
    let rebound = |value: String| machine(&[("file.target-abc123", ResourceKind::Other, value)]);
    runtime.set_machine(rebound(dir.path("docs")));
    assert_eq!(device_face(&runtime).symbol.as_deref(), Some(FOLDER));
    runtime.set_machine(rebound(dir.path("report.pdf")));
    assert_eq!(device_face(&runtime).symbol.as_deref(), Some(FILE));
}

#[test]
fn a_file_replaced_by_a_folder_shows_at_the_next_event_without_polling() {
    let dir = Scratch::new("replaced");
    let target = dir.path("target");
    std::fs::write(&target, b"a file").unwrap();
    let bindings = machine(&[("file.target-abc123", ResourceKind::Other, target.clone())]);
    let profile = profile(open(location("file.target-abc123")), None);
    let mut runtime = runtime(&profile, bindings);
    assert_eq!(device_face(&runtime).symbol.as_deref(), Some(FILE));
    std::fs::remove_file(&target).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    // Redraws (live states, presses) reuse what was found.
    assert_eq!(device_face(&runtime).symbol.as_deref(), Some(FILE));
    // The profile loads again (an action or a binding was saved).
    runtime.reload_profile(profile.clone());
    assert_eq!(device_face(&runtime).symbol.as_deref(), Some(FOLDER));
    // Back to a file, picked up when the layout is shown again (a live
    // preview's working layout).
    std::fs::remove_dir_all(&target).unwrap();
    std::fs::write(&target, b"a file again").unwrap();
    runtime.set_working_layout(&device(), profile.id, profile.layouts[0].clone());
    assert_eq!(device_face(&runtime).symbol.as_deref(), Some(FILE));
}

#[test]
fn nothing_chosen_missing_or_unknown_targets_show_the_document() {
    let dir = Scratch::new("unknown");
    let machine = machine(&[
        (
            "file.gone-abc123",
            ResourceKind::Other,
            dir.path("moved.pdf"),
        ),
        (
            "folder.gone-abc123",
            ResourceKind::Directory,
            dir.path("moved"),
        ),
        ("folder.work-abc123", ResourceKind::Directory, dir.path("")),
        (
            "app.safari-abc123",
            ResourceKind::Application,
            "com.apple.Safari".into(),
        ),
        (
            "file.share-abc123",
            ResourceKind::Other,
            "\\\\server\\share\\a.pdf".into(),
        ),
    ]);
    let reference = ParameterValue::Reference(
        deckpilot_core::variables::VariableRef::parse("shared.paths.docs").unwrap(),
    );
    for parameters in [
        // Nothing chosen: a cleared choice is an empty project role.
        ActionParameters::new(),
        ActionParameters::new().with(
            "location",
            ParameterValue::Resource(ResourceRef::ProjectRole {
                role: String::new(),
            }),
        ),
        // Missing files and folders, and an unbound key.
        location("file.gone-abc123"),
        location("folder.gone-abc123"),
        location("folder.work-abc123").with("path", ParameterValue::Text("nope/x.md".into())),
        location("file.unbound-abc123"),
        // Not a file or folder on this computer.
        location("app.safari-abc123"),
        location("file.share-abc123"),
        // Unresolved: given by a variable, known only when it runs.
        ActionParameters::new().with("location", reference.clone()),
        location("folder.work-abc123").with("path", reference),
        location("folder.work-abc123").with(
            "path",
            ParameterValue::Template("{{ shared.paths.docs }}/README.md".into()),
        ),
        // A project role without an active project.
        ActionParameters::new().with(
            "location",
            ParameterValue::Resource(ResourceRef::ProjectRole {
                role: "repository".into(),
            }),
        ),
    ] {
        assert_eq!(
            symbol(&profile(open(parameters.clone()), None), &machine),
            FILE,
            "{parameters:?}"
        );
    }
}

#[test]
fn project_roles_follow_the_active_project() {
    let dir = Scratch::new("roles");
    let machine = machine(&[
        (
            "alpha.repository",
            ResourceKind::Directory,
            dir.path("docs"),
        ),
        (
            "beta.repository",
            ResourceKind::Other,
            dir.path("report.pdf"),
        ),
    ]);
    let instance = open(ActionParameters::new().with(
        "location",
        ParameterValue::Resource(ResourceRef::ProjectRole {
            role: "repository".into(),
        }),
    ));
    let mut profile = profile(instance, None);
    for (name, binding) in [("ALPHA", "alpha.repository"), ("BETA", "beta.repository")] {
        let mut project = Project::new(ProjectKey::new(name).unwrap(), name);
        project.resources.insert("repository".into(), key(binding));
        profile.projects.push(project);
    }
    let alpha = profile.projects[0].clone();
    let beta = profile.projects[1].clone();
    let cache = TargetCache::new();
    assert_eq!(
        preview_face(&profile, &machine, Some(&alpha), &cache)
            .symbol
            .as_deref(),
        Some(FOLDER)
    );
    assert_eq!(
        preview_face(&profile, &machine, Some(&beta), &cache)
            .symbol
            .as_deref(),
        Some(FILE)
    );
    // On the device, by the active project.
    let mut runtime = Runtime::new(engine(), ModelCatalog::builtin(), machine);
    runtime.set_profile(Some(profile.clone()), Some(alpha.id));
    runtime
        .handle_event(&DeviceEvent::Connected { device: device() })
        .unwrap();
    assert_eq!(device_face(&runtime).symbol.as_deref(), Some(FOLDER));
    runtime.select_project(beta.id).unwrap();
    assert_eq!(device_face(&runtime).symbol.as_deref(), Some(FILE));
}

#[test]
fn the_users_icon_wins_and_is_never_touched() {
    let dir = Scratch::new("custom");
    let machine = machine(&[
        (
            "file.report-abc123",
            ResourceKind::Other,
            dir.path("report.pdf"),
        ),
        ("folder.work-abc123", ResourceKind::Directory, dir.path("")),
    ]);
    // The action's own icon, chosen in the action editor.
    let chosen =
        |target: &str| action(system::actions::OPEN_FILE, location(target), Some("rocket"));
    assert_eq!(
        symbol(&profile(chosen("folder.work-abc123"), None), &machine),
        "rocket"
    );
    assert_eq!(
        symbol(&profile(chosen("file.report-abc123"), None), &machine),
        "rocket"
    );
    // The user chose the open folder for a file: kept.
    let folder_for_a_file = action(
        system::actions::OPEN_FILE,
        location("file.report-abc123"),
        Some(FOLDER),
    );
    assert_eq!(symbol(&profile(folder_for_a_file, None), &machine), FOLDER);

    // The key's own icon: drawn over the automatic one, whatever the
    // target, and kept as it is in the layout.
    let icons = deckpilot_render::Icons::default();
    let model = models::stream_deck_mk2();
    let defaults = model.effective_appearance();
    let (w, h) = (72, 72);
    let draw = |face: &KeyFace| deckpilot_render::render_for(face, w, h, &icons, &defaults);
    let file_key = profile(open(location("file.report-abc123")), Some(custom_look()));
    let folder_key = profile(open(location("folder.work-abc123")), Some(custom_look()));
    let file_face = device_face(&runtime(&file_key, machine.clone()));
    let folder_face = device_face(&runtime(&folder_key, machine.clone()));
    assert_eq!(file_face.appearance, Some(custom_look()));
    assert_eq!(folder_face.appearance, Some(custom_look()));
    assert_eq!(
        draw(&file_face),
        draw(&folder_face),
        "the user's icon, not the target's"
    );

    // Removed: the automatic icon is back, for each target.
    let mut removed = folder_key.clone();
    removed.layouts[0].pages[0].bindings[0].appearance = None;
    let mut runtime = runtime(&folder_key, machine.clone());
    runtime.reload_profile(removed);
    let back = device_face(&runtime);
    assert_eq!(back.symbol.as_deref(), Some(FOLDER));
    assert_ne!(draw(&back), draw(&folder_face));
    // Changing the target leaves the user's icon alone.
    let mut retargeted = folder_key.clone();
    retargeted.actions[0].parameters = location("file.report-abc123");
    runtime.reload_profile(retargeted.clone());
    assert_eq!(device_face(&runtime).appearance, Some(custom_look()));
    assert_eq!(
        retargeted.layouts[0].pages[0].bindings[0].appearance,
        Some(custom_look())
    );
}

#[test]
fn saved_open_folder_keys_keep_running_with_their_folder_icon() {
    let dir = Scratch::new("legacy");
    let machine = machine(&[
        ("folder.work-abc123", ResourceKind::Directory, dir.path("")),
        (
            "folder.gone-abc123",
            ResourceKind::Directory,
            dir.path("moved"),
        ),
    ]);
    let registry = builtin(Os::current()).action_registry();
    let legacy = registry
        .definition(&ActionTypeId::new(system::actions::OPEN_DIRECTORY).unwrap())
        .expect("Open folder is still registered");
    assert!(legacy.deprecated, "not offered for new assignments");
    for (target, path) in [
        ("folder.work-abc123", None),
        ("folder.work-abc123", Some("docs")),
        // Even when its folder is gone: its own icon, not a guess.
        ("folder.gone-abc123", None),
    ] {
        let mut parameters = location(target);
        if let Some(path) = path {
            parameters = parameters.with("path", ParameterValue::Text(path.into()));
        }
        let instance = action(system::actions::OPEN_DIRECTORY, parameters, None);
        let profile = profile(instance, None);
        assert_eq!(symbol(&profile, &machine), FOLDER, "{target} {path:?}");
        // Its key still runs it: no missing or unavailable face.
        let face = device_face(&runtime(&profile, machine.clone()));
        assert!(!face.unavailable);
        assert_eq!(face.style, deckpilot_core::FaceStyle::Action);
    }
}

#[test]
fn saved_and_reopened_layouts_store_no_automatic_icon() {
    let dir = Scratch::new("saved");
    let machine = machine(&[("folder.work-abc123", ResourceKind::Directory, dir.path(""))]);
    let saved = profile(open(location("folder.work-abc123")), None);
    let json = serde_json::to_string(&saved).unwrap();
    // The action keeps the icon it was made with (the type's), the key no
    // icon of its own: the open folder is computed, never written.
    assert!(!json.contains(FOLDER), "{json}");
    let reopened: Profile = serde_json::from_str(&json).unwrap();
    assert_eq!(reopened, saved);
    assert_eq!(symbol(&reopened, &machine), FOLDER);
    // The faces themselves say what they show; serializing one does not
    // change the profile.
    let again = serde_json::to_string(&reopened).unwrap();
    assert_eq!(again, json);
}

#[test]
fn the_preview_and_the_device_draw_the_same_key() {
    let dir = Scratch::new("consistency");
    let machine = machine(&[
        (
            "file.report-abc123",
            ResourceKind::Other,
            dir.path("report.pdf"),
        ),
        ("folder.work-abc123", ResourceKind::Directory, dir.path("")),
    ]);
    let icons = deckpilot_render::Icons::default();
    let model = models::stream_deck_mk2();
    let defaults = model.effective_appearance();
    let image = model.capabilities.keys.and_then(|keys| keys.image).unwrap();
    let size = (image.width, image.height);
    let mut frames = Vec::new();
    for target in ["file.report-abc123", "folder.work-abc123"] {
        let profile = profile(open(location(target)), None);
        let on_device = device_face(&runtime(&profile, machine.clone()));
        let previewed = preview_face(&profile, &machine, None, &TargetCache::new());
        assert_eq!(on_device, previewed);
        let device_frame =
            deckpilot_render::render_for(&on_device, size.0, size.1, &icons, &defaults);
        let preview_frame =
            deckpilot_render::render_for(&previewed, size.0, size.1, &icons, &defaults);
        assert_eq!(device_frame, preview_frame, "{target}");
        frames.push(device_frame);
    }
    // And the two targets look different.
    assert_ne!(frames[0], frames[1]);
}
