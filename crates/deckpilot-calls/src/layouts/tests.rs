//! T-L1 (generator): one layout per model of its capability class, with
//! stable UUID v5 ids, never assuming an MK.2.

use deckpilot_device::{ModelCatalog, models};

use super::*;

fn template_for(capabilities: &DeviceCapabilities) -> Template {
    let shape = Shape::of(capabilities);
    builtin_templates()
        .into_iter()
        .find(|t| t.shape == shape)
        .unwrap()
}

#[test]
fn every_built_in_model_gets_a_valid_layout_of_its_class() {
    let catalog = ModelCatalog::builtin();
    let mut shapes = Vec::new();
    for model in catalog.iter() {
        let caps = &model.capabilities;
        let generated = generate(&template_for(caps), &model.id, caps).unwrap();
        shapes.push(Shape::of(caps));
        assert_eq!(generated.layout.model, model.id);
        assert_eq!(generated.layout.home_page, generated.pages[&Page::Active]);
        // Every binding names one of its own actions and a control the
        // model has.
        for page in &generated.layout.pages {
            for binding in &page.bindings {
                let action = binding.target.action_id().unwrap();
                assert!(generated.action_of(action).is_some());
                assert!(caps.controls().contains(&binding.control), "{}", model.id);
            }
        }
        if let Some(dismiss) = generated.dismiss {
            assert!(caps.controls().contains(&dismiss));
        }
    }
    shapes.sort();
    shapes.dedup();
    assert!(
        shapes.len() >= 3,
        "the built-in models span several classes: {shapes:?}"
    );
}

#[test]
fn ids_are_stable_and_follow_the_model_and_major_version() {
    let catalog = ModelCatalog::builtin();
    let mk2 = catalog
        .get(&DeviceModelId::new(models::STREAM_DECK_MK2_ID))
        .unwrap();
    let template = template_for(&mk2.capabilities);
    let a = generate(&template, &mk2.id, &mk2.capabilities).unwrap();
    let b = generate(&template, &mk2.id, &mk2.capabilities).unwrap();
    assert_eq!(a, b, "generating again gives the same layout");
    let mut next = template.clone();
    next.version = 2;
    let c = generate(&next, &mk2.id, &mk2.capabilities).unwrap();
    assert_ne!(c.layout.id, a.layout.id);
    assert_ne!(c.actions[0].id, a.actions[0].id);
    let xl = catalog
        .get(&DeviceModelId::new(models::STREAM_DECK_XL_ID))
        .unwrap();
    let x = generate(&template_for(&xl.capabilities), &xl.id, &xl.capabilities).unwrap();
    assert_ne!(x.layout.id, a.layout.id);
}

#[test]
fn a_model_of_another_class_is_refused_and_missing_controls_are_left_out() {
    let catalog = ModelCatalog::builtin();
    let mk2 = catalog
        .get(&DeviceModelId::new(models::STREAM_DECK_MK2_ID))
        .unwrap();
    let small = builtin_templates()
        .into_iter()
        .find(|t| t.shape == Shape::GridSmall)
        .unwrap();
    assert!(matches!(
        generate(&small, &mk2.id, &mk2.capabilities),
        Err(GenerateError::WrongShape { .. })
    ));
    // A grid template with a key the model lacks: that key is left out.
    let mut grid = template_for(&mk2.capabilities);
    grid.pages[2].1.push(TemplateControl {
        control: ControlId::Key { index: 40 },
        action: "end".into(),
        parameters: ActionParameters::new(),
    });
    let generated = generate(&grid, &mk2.id, &mk2.capabilities).unwrap();
    assert_eq!(generated.skipped, [ControlId::Key { index: 40 }]);
    // The action types are the Calls plugin's.
    assert!(
        generated
            .actions
            .iter()
            .all(|a| a.action_type.as_str().starts_with("deckpilot.calls."))
    );
    assert_eq!(generated.dismiss, Some(ControlId::Key { index: 14 }));
}

#[test]
fn bad_templates_are_refused() {
    let catalog = ModelCatalog::builtin();
    let mk2 = catalog
        .get(&DeviceModelId::new(models::STREAM_DECK_MK2_ID))
        .unwrap();
    let mut no_active = template_for(&mk2.capabilities);
    no_active.pages.retain(|(page, _)| *page != Page::Active);
    assert_eq!(
        generate(&no_active, &mk2.id, &mk2.capabilities),
        Err(GenerateError::NoActivePage)
    );
    let mut bad = template_for(&mk2.capabilities);
    bad.pages[0].1[0].action = "../Answer".into();
    assert!(matches!(
        generate(&bad, &mk2.id, &mk2.capabilities),
        Err(GenerateError::BadAction(_))
    ));
}
