//! Key labels on faces (`docs/key-text.md`, "Labels"): the shown binding's
//! labels are resolved like its title, each in its own text slot, filtered
//! by their switch and visibility, and never put in the look.

use deckpilot_device::models;

use super::*;
use crate::variables::{DisplayDependency, RecordingTemplates, TextSlot};
use crate::{KeyAppearance, KeyLabel, LabelId, LabelVisibility, Project, ProjectKey};

fn id(text: &str) -> LabelId {
    LabelId::new(text).unwrap()
}

fn label(name: &str, text: &str) -> KeyLabel {
    KeyLabel::new(id(name), text)
}

struct Setup {
    profile: Profile,
    layout: DeviceLayout,
    project: Project,
}

fn setup(bindings: Vec<ControlBinding>) -> Setup {
    let mut profile = Profile::new("Work");
    let project = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop front");
    profile.projects.push(project.clone());
    let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
    let home = layout.home_page;
    layout.page_mut(home).unwrap().name = "Main".into();
    layout.page_mut(home).unwrap().bindings = bindings;
    Setup {
        profile,
        layout,
        project,
    }
}

fn labelled(trigger: Trigger, labels: Vec<KeyLabel>) -> ControlBinding {
    let mut binding =
        ControlBinding::new(ControlId::Key { index: 0 }, trigger, BindingTarget::Back);
    binding.appearance = Some(KeyAppearance {
        labels,
        ..KeyAppearance::default()
    });
    binding
}

fn face(setup: &Setup, project: bool) -> KeyFace {
    page_faces(
        &ActionRegistry::default(),
        &setup.layout,
        setup.layout.home_page,
        &setup.profile,
        project.then_some(&setup.project),
        &[ControlId::Key { index: 0 }],
        None,
    )
    .remove(0)
    .1
}

fn texts(face: &KeyFace) -> Vec<(&str, &str, bool)> {
    face.labels
        .iter()
        .map(|label| (label.id.as_str(), label.text.as_str(), label.unresolved))
        .collect()
}

#[test]
fn static_and_dynamic_texts_are_resolved_like_titles() {
    let setup = setup(vec![labelled(
        Trigger::Press,
        vec![
            label("plain", "Deploy"),
            label("name", "{project.name}"),
            label("key", "{{ system.project.current.key }}"),
            label("profile", "{profile}"),
            label("page", "{page.number}/{page.count} {page.name}"),
        ],
    )]);
    let shown = face(&setup, true);
    assert_eq!(
        texts(&shown),
        [
            ("plain", "Deploy", false),
            ("name", "Shop front", false),
            ("key", "SHOP", false),
            ("profile", "Work", false),
            ("page", "1/1 Main", false),
        ]
    );
    // The templates stay in the layout: the face's look has none, and a
    // look of labels only is no look at all.
    assert_eq!(shown.appearance, None);
    // The spec is the label's look, without its text.
    assert_eq!(shown.labels[0].spec, label("x", "x").look);
}

#[test]
fn unresolved_references_show_the_marker_and_when_resolved_hides() {
    let mut hidden = label("hidden", "{project.key}");
    hidden.visible = Some(LabelVisibility::WhenResolved);
    let mut project_only = label("project", "Shop");
    project_only.visible = Some(LabelVisibility::WhenProject);
    let mut off = label("off", "Off");
    off.enabled = Some(false);
    let setup = setup(vec![labelled(
        Trigger::Press,
        vec![label("shown", "{project.key}"), hidden, project_only, off],
    )]);
    assert_eq!(texts(&face(&setup, false)), [("shown", "-", true)]);
    assert_eq!(
        texts(&face(&setup, true)),
        [
            ("shown", "SHOP", false),
            ("hidden", "SHOP", false),
            ("project", "Shop", false)
        ]
    );
}

#[test]
fn values_a_key_never_shows_are_not_resolved() {
    let setup = setup(vec![labelled(
        Trigger::Press,
        vec![
            label("run", "{{ steps.fetch.output.token }}"),
            label("execution", "id {{ workflow.execution.id }}"),
        ],
    )]);
    assert_eq!(
        texts(&face(&setup, true)),
        [("run", "-", true), ("execution", "id -", true)]
    );
}

#[test]
fn long_texts_are_cut_before_layout() {
    let setup = setup(vec![labelled(
        Trigger::Press,
        vec![label("long", &"ż".repeat(150))],
    )]);
    let shown = face(&setup, true);
    let text = &shown.labels[0].text;
    assert_eq!(
        text.chars().count(),
        crate::appearance::labels::MAX_LABEL_CLUSTERS
    );
    assert!(text.ends_with('…'));
}

#[test]
fn each_label_has_its_own_dependencies() {
    let setup = setup(vec![labelled(
        Trigger::Press,
        vec![label("static", "Deploy"), label("dynamic", "{project.key}")],
    )]);
    let texts = core_display(&setup.profile, Some(&setup.project), |templates| {
        let recorder = RecordingTemplates::new(templates);
        page_faces_with(
            &ActionRegistry::default(),
            &setup.layout,
            setup.layout.home_page,
            &setup.profile,
            Some(&setup.project),
            &[ControlId::Key { index: 0 }],
            None,
            &recorder,
        );
        recorder.into_texts()
    });
    let labels: Vec<_> = texts
        .iter()
        .filter_map(|(key, text)| match key.slot {
            TextSlot::Label { id } => Some((id.to_string(), text.is_dynamic(), text)),
            TextSlot::Title => None,
        })
        .collect();
    assert_eq!(labels.len(), 2);
    assert_eq!((labels[0].0.as_str(), labels[0].1), ("static", false));
    assert_eq!((labels[1].0.as_str(), labels[1].1), ("dynamic", true));
    assert!(
        labels[1]
            .2
            .dependencies
            .contains(&DisplayDependency::Project)
    );
}

#[test]
fn only_the_shown_bindings_labels_are_drawn() {
    // A click and a long press: the click's look is shown, the long
    // press's labels are dormant.
    let setup = setup(vec![
        labelled(Trigger::Click, vec![label("click", "Click")]),
        labelled(Trigger::LongPress, vec![label("hold", "Hold")]),
    ]);
    assert_eq!(texts(&face(&setup, true)), [("click", "Click", false)]);
}

#[test]
fn faces_without_labels_serialize_as_before() {
    let setup = setup(vec![ControlBinding::new(
        ControlId::Key { index: 0 },
        Trigger::Press,
        BindingTarget::Back,
    )]);
    let shown = face(&setup, true);
    let json = serde_json::to_value(&shown).unwrap();
    assert!(json.get("labels").is_none(), "{json}");
    let labelled = setup_with_labels();
    let json = serde_json::to_value(face(&labelled, true)).unwrap();
    assert_eq!(
        json["labels"][0],
        serde_json::json!({ "id": "a", "text": "A", "spec": {} })
    );
}

fn setup_with_labels() -> Setup {
    setup(vec![labelled(Trigger::Press, vec![label("a", "A")])])
}

#[test]
fn the_folders_reserved_back_key_shows_no_labels() {
    assert_eq!(system_back_face().labels, []);
    assert_eq!(KeyFace::empty().labels, []);
}

#[test]
fn markers_follow_page_placeholders_and_the_cut() {
    let setup = setup(vec![labelled(
        Trigger::Press,
        vec![
            label(
                "mixed",
                "{page.name}: {{ shared.team.name }} / {project.key} ż",
            ),
            label("cut", &format!("{}{{project.key}}", "x".repeat(130))),
            // Two references back to back: two markers.
            label("adjacent", "{{ shared.team.a }}{{ shared.team.b }}"),
            // A page name that looks like a reference is text.
            label("paged", "{page.name}"),
        ],
    )]);
    let shown = face(&setup, false);
    let mixed = &shown.labels[0];
    assert_eq!(mixed.text, "Main: - / - ż");
    let at = |m: &crate::LabelMarker| (m.start, m.end, m.reference.clone());
    assert_eq!(
        mixed.markers.iter().map(at).collect::<Vec<_>>(),
        [
            (6, 7, "shared.team.name".to_owned()),
            (10, 11, "project.key".to_owned())
        ]
    );
    // The marker past the cut is dropped.
    let cut = &shown.labels[1];
    assert!(cut.unresolved);
    assert_eq!(cut.markers, []);
    assert!(cut.text.ends_with('…'));
    let adjacent = &shown.labels[2];
    assert_eq!(adjacent.text, "--");
    assert_eq!(
        adjacent.markers.iter().map(at).collect::<Vec<_>>(),
        [
            (0, 1, "shared.team.a".to_owned()),
            (1, 2, "shared.team.b".to_owned())
        ]
    );
    let json = serde_json::to_value(mixed).unwrap();
    assert_eq!(
        json["markers"][0],
        serde_json::json!({ "start": 6, "end": 7, "reference": "shared.team.name" })
    );
}

#[test]
fn texts_are_cut_between_grapheme_clusters() {
    // "e" with a combining acute accent: two characters, one cluster.
    let accented = "e\u{301}".repeat(130);
    let setup = setup(vec![labelled(Trigger::Press, vec![label("a", &accented)])]);
    let shown = face(&setup, true);
    let text = &shown.labels[0].text;
    assert!(text.ends_with("e\u{301}…"), "never inside a cluster");
    assert_eq!(text.chars().count(), 2 * 119 + 1);
}
