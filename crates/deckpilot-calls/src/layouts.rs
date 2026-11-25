//! The managed Calls layouts (`docs/calls.md`, "Managed layouts"): one
//! layout per device model, generated from a template for the model's
//! capability class, with stable ids.
//!
//! - **Capability classes** ([`Shape::of`]): keys and dials, keys without
//!   displays, and grids of at most 6, 7 to 15, or more keys. Nothing
//!   assumes a Stream Deck MK.2: a template control the model lacks is left
//!   out.
//! - **Stable ids.** The layout is UUID v5 of the model and the template's
//!   major version; pages and action instances are UUID v5 of the layout
//!   and their key. Generating again gives the same ids on every machine,
//!   so seeding is idempotent and exports name the same layout.
//! - **Placed relative to the grid.** A template control at a position the
//!   model has stays there; one the model lacks (a key beyond a small
//!   grid, the dismiss key) moves to the first free key. Nothing is
//!   dropped: a model too small for a page is refused.
//! - **The protected dismiss key** is always the model's last key: it
//!   always takes the overlay away. A model without keys is refused.
//!
//! The templates are data of the Calls plugin's manifest
//! (`contributes.layoutTemplates`); [`builtin_templates`] are the reference
//! set it ships.

use std::collections::BTreeMap;

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, BindingTarget,
    ControlBinding, DeviceLayout, LayoutId, LayoutPage, PageId, Trigger,
};
use deckpilot_device::{Capability as Family, ControlId, DeviceCapabilities, DeviceModelId};
use uuid::Uuid;

#[cfg(test)]
#[path = "layouts/tests.rs"]
mod tests;

/// The owner of the managed layouts and the prefix of their action types.
pub const OWNER: &str = "deckpilot.calls";

/// The namespace of every Calls id.
fn namespace() -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, OWNER.as_bytes())
}

/// The capability class of a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Shape {
    GridSmall,
    Grid,
    GridLarge,
    KeysDials,
    NoDisplay,
}

impl Shape {
    /// The class of a model with `capabilities`.
    pub fn of(capabilities: &DeviceCapabilities) -> Self {
        let keys = capabilities.keys.map_or(0, |grid| grid.key_count());
        if capabilities.supports(Family::Dials) {
            Self::KeysDials
        } else if !capabilities.supports(Family::KeyDisplays) {
            Self::NoDisplay
        } else if keys <= 6 {
            Self::GridSmall
        } else if keys <= 15 {
            Self::Grid
        } else {
            Self::GridLarge
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::GridSmall => "grid-small",
            Self::Grid => "grid",
            Self::GridLarge => "grid-large",
            Self::KeysDials => "keys-dials",
            Self::NoDisplay => "no-display",
        }
    }
}

/// A page of the overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Page {
    Incoming,
    Outgoing,
    Active,
}

impl Page {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
            Self::Active => "active",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Incoming => "Incoming call",
            Self::Outgoing => "Calling",
            Self::Active => "In a call",
        }
    }
}

/// One control of a template page: a Calls action (its id without the
/// plugin prefix, e.g. `mic-toggle`) with its parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateControl {
    pub control: ControlId,
    pub action: String,
    pub parameters: ActionParameters,
}

/// A template for one capability class.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub shape: Shape,
    /// The major version: a new one is a new layout id.
    pub version: u32,
    pub pages: Vec<(Page, Vec<TemplateControl>)>,
}

/// A generated layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Generated {
    pub layout: DeviceLayout,
    pub actions: Vec<ActionInstance>,
    pub pages: BTreeMap<Page, PageId>,
    /// The protected dismiss key: always there.
    pub dismiss: Option<ControlId>,
    /// Template controls placed elsewhere: `(template position, used)`.
    pub moved: Vec<(ControlId, ControlId)>,
}

impl Generated {
    /// The generated layout with the stored one in its place (the user's
    /// edits, a layout of an earlier template version): the overlay's pages
    /// are found by the stored layout's own ids, and only those it still
    /// has are used.
    #[must_use]
    pub fn with_stored(mut self, layout: DeviceLayout, actions: Vec<ActionInstance>) -> Self {
        self.pages = [Page::Incoming, Page::Outgoing, Page::Active]
            .into_iter()
            .map(|page| (page, page_id(layout.id, page)))
            .filter(|(_, id)| layout.page(*id).is_some())
            .collect();
        self.layout = layout;
        self.actions = actions;
        self
    }

    /// The page the overlay shows for `page`, falling back to the active
    /// page (a template without an incoming page shows the call there).
    pub fn page(&self, page: Page) -> Option<PageId> {
        self.pages
            .get(&page)
            .or_else(|| self.pages.get(&Page::Active))
            .copied()
    }

    /// The Calls action (id without prefix) behind an action instance.
    pub fn action_of(&self, id: ActionInstanceId) -> Option<&str> {
        self.actions
            .iter()
            .find(|a| a.id == id)
            .and_then(|a| a.action_type.as_str().strip_prefix(OWNER))
            .and_then(|rest| rest.strip_prefix('.'))
    }
}

/// Why nothing was generated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GenerateError {
    #[error("the template is for `{template}`, the model is `{model}`")]
    WrongShape {
        template: &'static str,
        model: &'static str,
    },
    #[error("the template has no active page")]
    NoActivePage,
    #[error("`{0}` is not a Calls action id")]
    BadAction(String),
    #[error("the model has no keys: the overlay would have no dismiss key")]
    NoDismiss,
    #[error("the model has too few keys for the `{0}` page")]
    NoRoom(&'static str),
}

fn has(capabilities: &DeviceCapabilities, control: ControlId) -> bool {
    capabilities.controls().contains(&control)
}

/// The layout of `template` for the model `model` with `capabilities`.
///
/// # Panics
///
/// Never: a UUID is always a valid layout, page and action id.
pub fn generate(
    template: &Template,
    model: &DeviceModelId,
    capabilities: &DeviceCapabilities,
) -> Result<Generated, GenerateError> {
    let shape = Shape::of(capabilities);
    if template.shape != shape {
        return Err(GenerateError::WrongShape {
            template: template.shape.as_str(),
            model: shape.as_str(),
        });
    }
    let active_controls = template
        .pages
        .iter()
        .find(|(page, _)| *page == Page::Active)
        .ok_or(GenerateError::NoActivePage)?;
    let layout_uuid = Uuid::new_v5(
        &namespace(),
        format!("layout:{}:{}", model.as_str(), template.version).as_bytes(),
    );
    let layout_id: LayoutId = layout_uuid
        .to_string()
        .parse()
        .expect("a UUID is a layout id");
    let mut layout = DeviceLayout::new("Calls", model.clone());
    layout.id = layout_id;
    layout.explicit_tree = true;
    layout.pages.clear();
    let mut pages = BTreeMap::new();
    let mut actions = Vec::new();
    let mut moved = Vec::new();
    let keys = capabilities.keys.map_or(0, |grid| grid.key_count());
    let dismiss = keys
        .checked_sub(1)
        .map(|index| ControlId::Key { index })
        .ok_or(GenerateError::NoDismiss)?;
    // The active page first: it is the layout's home page.
    let ordered = std::iter::once(active_controls).chain(
        template
            .pages
            .iter()
            .filter(|(page, _)| *page != Page::Active),
    );
    for (page, controls) in ordered {
        let mut taken = vec![dismiss];
        let mut layout_page = LayoutPage::new(page.name());
        layout_page.id = page_id(layout_id, *page);
        for control in controls {
            if !is_action_id(&control.action) {
                return Err(GenerateError::BadAction(control.action.clone()));
            }
            let placed = if has(capabilities, control.control) && !taken.contains(&control.control)
            {
                control.control
            } else {
                let free = (0..keys)
                    .map(|index| ControlId::Key { index })
                    .find(|key| !taken.contains(key))
                    .ok_or(GenerateError::NoRoom(page.as_str()))?;
                moved.push((control.control, free));
                free
            };
            taken.push(placed);
            let action_uuid = Uuid::new_v5(
                &layout_uuid,
                format!("action:{}:{}:{}", page.as_str(), placed, control.action).as_bytes(),
            );
            let instance = ActionInstance {
                id: action_uuid
                    .to_string()
                    .parse()
                    .expect("a UUID is an action id"),
                name: control.action.clone(),
                action_type: ActionTypeId::new(format!("{OWNER}.{}", control.action))
                    .map_err(|_| GenerateError::BadAction(control.action.clone()))?,
                parameters: control.parameters.clone(),
                description: String::new(),
                icon: None,
                label: None,
            };
            layout_page.bindings.push(ControlBinding::new(
                placed,
                Trigger::Press,
                BindingTarget::action(instance.id),
            ));
            actions.push(instance);
        }
        pages.insert(*page, layout_page.id);
        layout.pages.push(layout_page);
    }
    layout.home_page = pages[&Page::Active];
    Ok(Generated {
        layout,
        actions,
        pages,
        dismiss: Some(dismiss),
        moved,
    })
}

/// The stable id of `page` in the layout `layout`: UUID v5 of the layout
/// id and the page's key.
///
/// # Panics
///
/// Never: layout and page ids are UUIDs.
pub fn page_id(layout: LayoutId, page: Page) -> PageId {
    let layout_uuid: Uuid = layout.to_string().parse().expect("a layout id is a UUID");
    Uuid::new_v5(&layout_uuid, format!("page:{}", page.as_str()).as_bytes())
        .to_string()
        .parse()
        .expect("a UUID is a page id")
}

fn is_action_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn control(control: ControlId, action: &str) -> TemplateControl {
    TemplateControl {
        control,
        action: action.into(),
        parameters: ActionParameters::new(),
    }
}

/// The reference templates, one per shape: what the Calls plugin's
/// manifest ships (`contributes.layoutTemplates`).
pub fn builtin_templates() -> Vec<Template> {
    let incoming = |answer: u16, reject: u16| {
        (
            Page::Incoming,
            vec![
                control(key(answer), "answer"),
                control(key(reject), "reject"),
            ],
        )
    };
    let outgoing = |cancel: u16| {
        (
            Page::Outgoing,
            vec![control(key(cancel), "cancel-outgoing")],
        )
    };
    let active = |mic: u16, camera: u16, end: u16, back: u16| {
        (
            Page::Active,
            vec![
                control(key(mic), "mic-toggle"),
                control(key(camera), "camera-toggle"),
                control(key(end), "end"),
                control(key(back), "return-to-work"),
            ],
        )
    };
    vec![
        Template {
            shape: Shape::GridSmall,
            version: 1,
            pages: vec![incoming(0, 2), outgoing(2), active(0, 1, 2, 3)],
        },
        Template {
            shape: Shape::Grid,
            version: 1,
            pages: vec![incoming(6, 8), outgoing(7), active(5, 6, 7, 10)],
        },
        Template {
            shape: Shape::GridLarge,
            version: 1,
            pages: vec![incoming(10, 13), outgoing(12), active(10, 11, 12, 24)],
        },
        Template {
            shape: Shape::KeysDials,
            version: 1,
            pages: vec![
                incoming(0, 3),
                outgoing(3),
                (
                    Page::Active,
                    vec![
                        control(ControlId::Encoder { index: 0 }, "mic-toggle"),
                        control(ControlId::Encoder { index: 1 }, "camera-toggle"),
                        control(key(3), "end"),
                        control(key(4), "return-to-work"),
                    ],
                ),
            ],
        },
        Template {
            shape: Shape::NoDisplay,
            version: 1,
            pages: vec![(Page::Active, vec![control(key(0), "mic-toggle")])],
        },
    ]
}
