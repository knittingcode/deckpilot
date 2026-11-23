//! Variables in key titles on a device (`docs/variables.md`, "Variables in
//! key titles"): the faces resolve titles through the engine, follow the
//! active project and profile, record what each text depends on, and agree
//! with the editor's preview, which resolves the same templates through
//! the same function.

use std::collections::BTreeMap;
use std::sync::Arc;

use deckpilot_device::{DeviceEvent, models};

use super::*;
use crate::testing::{echo_type, registry, with_projects};
use crate::variables::doubles::{MemoryVariables, builtin_registry};
use crate::variables::{
    DisplayDependency, DisplayText, ResolvedTemplates, ScopeKey, TextSlot, VariableDefinition,
    VariableKey, VariableScope, VariableType, VariableValue, resolve_display_template,
};
use crate::{ActionParameters, Os};

const TITLES: [&str; 6] = [
    "Build",
    "Projekt: {project.key}",
    "{{ shared.team.name }}",
    "{{ shared.team.size }}",
    "{profile} — {{ projects.current.name }}",
    "PIN {{ shared.vault.pin }}",
];

fn key(text: &str) -> VariableKey {
    VariableKey::new(text).unwrap()
}

fn device() -> DeviceId {
    DeviceId {
        model: DeviceModelId::new(models::STREAM_DECK_MK2_ID),
        serial: "TITLES".into(),
    }
}

fn profile() -> Profile {
    let mut profile = with_projects("Work", &["SHOP", "CMR"]);
    let action = registry()
        .create_instance(
            "Echo",
            echo_type(),
            &ActionParameters::new().with("message", crate::ParameterValue::Text("hi".into())),
        )
        .unwrap();
    let action_id = action.id;
    profile.actions.push(action);
    let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
    let home = layout.home_page;
    layout
        .page_mut(home)
        .unwrap()
        .bindings
        .extend(
            TITLES
                .iter()
                .enumerate()
                .map(|(index, title)| ControlBinding {
                    control: ControlId::Key {
                        index: u16::try_from(index).unwrap(),
                    },
                    trigger: Trigger::Press,
                    target: BindingTarget::action(action_id),
                    label: Some((*title).to_owned()),
                    appearance: None,
                }),
        );
    profile.layouts.push(layout);
    profile
}

fn variables(profile: &Profile) -> MemoryVariables {
    let shop = profile.projects[0].id;
    MemoryVariables::new(profile.clone())
        .define(VariableDefinition::new(
            key("team.name"),
            VariableType::String,
            &[VariableScope::Global, VariableScope::Project],
        ))
        .set(
            &key("team.name"),
            ScopeKey::Global,
            VariableValue::String("Core".into()),
        )
        .set(
            &key("team.name"),
            ScopeKey::Project {
                profile: profile.id,
                project: shop,
            },
            VariableValue::String("Shop team".into()),
        )
        .define(VariableDefinition::new(
            key("team.size"),
            VariableType::Integer,
            &[VariableScope::Global],
        ))
        .set(
            &key("team.size"),
            ScopeKey::Global,
            VariableValue::Integer(4),
        )
        .define(VariableDefinition {
            sensitive: true,
            ..VariableDefinition::new(
                key("vault.pin"),
                VariableType::String,
                &[VariableScope::Global],
            )
        })
        .set(
            &key("vault.pin"),
            ScopeKey::Global,
            VariableValue::String("s3cr3t".into()),
        )
}

fn runtime(profile: &Profile) -> Runtime {
    let engine = Engine::new(registry())
        .with_variables(Arc::new(variables(profile)))
        .with_variable_registry(Arc::new(builtin_registry()));
    let mut runtime = Runtime::new(
        engine,
        deckpilot_device::ModelCatalog::builtin(),
        MachineConfig::new(Os::Linux),
    );
    runtime.set_profile(Some(profile.clone()), Some(profile.projects[0].id));
    let _ = runtime.handle_event(&DeviceEvent::Connected { device: device() });
    runtime
}

fn titles(faces: &[(ControlId, KeyFace)]) -> Vec<String> {
    faces
        .iter()
        .take(TITLES.len())
        .map(|(_, face)| face.title.clone())
        .collect()
}

#[test]
fn device_faces_resolve_titles_through_the_engine() {
    let profile = profile();
    let runtime = runtime(&profile);
    let (faces, texts) = runtime.display_faces(&device());
    assert_eq!(
        titles(&faces),
        [
            "Build",
            "Projekt: SHOP",
            "Shop team",
            "4",
            "Work — SHOP",
            "PIN -"
        ]
    );
    // Every title is recorded by its binding and slot; the static one
    // depends on nothing.
    assert_eq!(texts.len(), TITLES.len());
    let at = |index: u16| {
        texts
            .iter()
            .find(|(key, _)| key.control == ControlId::Key { index })
            .map(|(key, text)| {
                assert_eq!(key.slot, TextSlot::Title);
                assert_eq!(key.trigger, Trigger::Press);
                text
            })
            .unwrap()
    };
    assert!(!at(0).is_dynamic());
    assert!(at(2).dependencies.contains(&DisplayDependency::Variable {
        reference: "shared.team.name".into()
    }));
    assert_eq!(at(5).unresolved[0].reference, "shared.vault.pin");
    // The secret is in no face and no recorded text.
    let payload = serde_json::to_string(&faces).unwrap()
        + &serde_json::to_string(&texts.iter().map(|(_, t)| t).collect::<Vec<_>>()).unwrap();
    assert!(!payload.contains("s3cr3t"), "{payload}");
    // The stored templates are untouched.
    let stored: Vec<_> = profile.layouts[0].pages[0]
        .bindings
        .iter()
        .map(|b| b.label.clone().unwrap())
        .collect();
    assert_eq!(stored, TITLES);
}

#[test]
fn switching_the_project_changes_only_the_keys_that_use_it() {
    let profile = profile();
    let mut runtime = runtime(&profile);
    let before = runtime.faces(&device());
    runtime.select_project(profile.projects[1].id).unwrap();
    let after = runtime.faces(&device());
    // The agent sends a key only when its face differs (`render`).
    let changed: Vec<ControlId> = before
        .iter()
        .zip(&after)
        .filter(|(a, b)| a != b)
        .map(|((control, _), _)| *control)
        .collect();
    assert_eq!(
        changed,
        [1, 2, 4].map(|index| ControlId::Key { index }),
        "{:?}",
        titles(&after)
    );
    assert_eq!(
        titles(&after),
        ["Build", "Projekt: CMR", "Core", "4", "Work — CMR", "PIN -"]
    );
}

#[test]
fn switching_the_profile_changes_only_the_keys_that_use_it() {
    let work = profile();
    let mut runtime = runtime(&work);
    let before = runtime.faces(&device());
    // The same layout and actions in a profile of another name.
    let mut renamed = work.clone();
    renamed.name = "Home".into();
    runtime.reload_profile(renamed);
    let after = runtime.faces(&device());
    let changed: Vec<ControlId> = before
        .iter()
        .zip(&after)
        .filter(|(a, b)| a != b)
        .map(|((control, _), _)| *control)
        .collect();
    assert_eq!(changed, [ControlId::Key { index: 4 }]);
    assert_eq!(after[4].1.title, "Home — SHOP");
}

#[test]
fn the_preview_shows_what_the_device_shows() {
    let profile = profile();
    let runtime = runtime(&profile);
    let device_faces = runtime.faces(&device());
    // The editor's path: the agent resolves the templates
    // (`resolveDisplayTemplates`) with the engine's display environment,
    // and the preview draws the page with those answers.
    let answers: BTreeMap<String, DisplayText> = runtime.engine().display_templates(
        &profile,
        runtime.machine(),
        runtime.active_project_id(),
        |environment| {
            TITLES
                .iter()
                .map(|t| ((*t).to_owned(), resolve_display_template(environment, t)))
                .collect()
        },
    );
    let layout = &profile.layouts[0];
    let model = models::stream_deck_mk2();
    let controls: Vec<ControlId> = device_faces.iter().map(|(c, _)| *c).collect();
    let preview = core_display(&profile, runtime.active_project(), |core| {
        page_faces_with(
            runtime.engine().registry(),
            layout,
            layout.home_page,
            &profile,
            runtime.active_project(),
            &controls,
            Some(&model.capabilities),
            &ResolvedTemplates {
                resolved: &answers,
                fallback: core,
            },
        )
    });
    assert_eq!(preview, device_faces);
    // Without the agent's answers the preview falls back to the core alone:
    // the older placeholders resolve, the rest shows the marker.
    let alone = page_faces(
        runtime.engine().registry(),
        layout,
        layout.home_page,
        &profile,
        runtime.active_project(),
        &controls,
        Some(&model.capabilities),
    );
    assert_eq!(
        titles(&alone),
        ["Build", "Projekt: SHOP", "-", "-", "Work — -", "PIN -"]
    );
}
