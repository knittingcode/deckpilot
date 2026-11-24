//! Variables in key titles on the agent (`docs/variables.md`, "Variables in
//! key titles"; protocol 17, `deckpilot_ipc::display_templates`).
//!
//! The device's faces resolve their titles through the engine
//! ([`deckpilot_core::Runtime::display_faces`]); the editor asks for the same
//! resolution (`resolveDisplayTemplates`, [`Engine::display_templates`]), so
//! both show the same text. Nothing here resolves on its own.
//!
//! Re-rendering is event-driven. Every redraw records, per device, what each
//! text of each binding depends on ([`TitleDependencies`], keyed by
//! [`TextKey`]: page, control, trigger and text slot, so a key with several
//! texts follows all of them). When `sharedVariablesChanged` names a
//! dependency of a shown text (a shared variable's value, a plugin's
//! published value, a module or plugin that was enabled or disabled), only
//! the devices showing such a text are redrawn, and of those only the keys
//! whose face changed are sent (the frame diff of `render`). A change no
//! shown text depends on redraws nothing. Profile and project changes
//! already redraw every device (`after_change`), with the same diff.
//!
//! Resolved values are never logged: only counts are.

use std::collections::{BTreeSet, HashMap};

use deckpilot_core::Engine;
use deckpilot_core::variables::{
    CatalogFilter, DisplayDependency, DisplayText, ResolutionContext, TextKey, display_listed,
    resolve_display_template, with_members,
};
use deckpilot_device::DeviceId;
use deckpilot_ipc::display_templates::{ResolvedDisplayTemplate, check_templates};
use deckpilot_ipc::{ErrorCode, IpcError, Response};
use tracing::debug;

use super::Agent;
use super::variables::catalog;

/// What the texts each device shows depend on, from its last redraw.
#[derive(Debug, Default)]
pub struct TitleDependencies {
    devices: HashMap<DeviceId, HashMap<TextKey, BTreeSet<DisplayDependency>>>,
}

impl TitleDependencies {
    /// Replaces what `device` shows with `texts` (from its redraw). Static
    /// texts are not kept.
    pub fn record(&mut self, device: &DeviceId, texts: Vec<(TextKey, DisplayText)>) {
        let dynamic: HashMap<TextKey, BTreeSet<DisplayDependency>> = texts
            .into_iter()
            .filter(|(_, text)| text.is_dynamic())
            .map(|(key, text)| (key, text.dependencies))
            .collect();
        if dynamic.is_empty() {
            self.devices.remove(device);
        } else {
            self.devices.insert(device.clone(), dynamic);
        }
    }

    pub fn forget(&mut self, device: &DeviceId) {
        self.devices.remove(device);
    }

    /// The texts of `device` that a change of any of `keys` (as
    /// `sharedVariablesChanged` names them) changes.
    pub fn affected(&self, device: &DeviceId, keys: &[String]) -> Vec<TextKey> {
        self.devices
            .get(device)
            .into_iter()
            .flatten()
            .filter(|(_, dependencies)| {
                dependencies
                    .iter()
                    .any(|dependency| keys.iter().any(|key| dependency.changed_by(key)))
            })
            .map(|(key, _)| *key)
            .collect()
    }

    /// The devices showing a text that a change of any of `keys` changes.
    pub fn devices_for(&self, keys: &[String]) -> Vec<DeviceId> {
        let mut devices: Vec<DeviceId> = self
            .devices
            .keys()
            .filter(|device| !self.affected(device, keys).is_empty())
            .cloned()
            .collect();
        devices.sort();
        devices
    }

    /// Every device showing a text that depends on anything.
    pub fn dynamic_devices(&self) -> Vec<DeviceId> {
        let mut devices: Vec<DeviceId> = self.devices.keys().cloned().collect();
        devices.sort();
        devices
    }
}

/// The display context a client asks for: always a preview of a profile
/// with a project (its own, else the profile's active one), never a run, a
/// workflow or a device.
fn client_display_context(context: &ResolutionContext) -> ResolutionContext {
    deckpilot_core::variables::display_context(context.profile, context.project)
}

impl Agent {
    /// The engine's display templates for `profile`'s context, as the
    /// device resolves them.
    fn with_display<R>(
        &self,
        context: &ResolutionContext,
        then: impl FnOnce(&Engine, &deckpilot_core::variables::DisplayEnvironment<'_>) -> R,
    ) -> Result<R, IpcError> {
        let profile = self.context_profile(context.profile)?;
        let context = client_display_context(&self.preview_context(context));
        let engine = self.runtime.engine();
        Ok(engine.display_templates(
            &profile,
            self.runtime.machine(),
            context.project,
            |environment| then(engine, environment),
        ))
    }

    /// `listDisplayVariables`: the catalog of `listVariables`, display-safe
    /// entries only.
    pub(super) fn list_display_variables(
        &self,
        context: &ResolutionContext,
        filter: &CatalogFilter,
    ) -> Result<Response, IpcError> {
        let profile = self.context_profile(context.profile)?;
        let context = client_display_context(&self.preview_context(context));
        let registry = self.runtime.engine().variable_registry();
        let entries = catalog(
            registry,
            &self.current_variables(context.profile)?,
            &context,
            false,
        );
        // The display-safe members of structured variables (the current
        // project's repositories' names and keys, never a path or an id);
        // never the structured variables themselves.
        let project = context.project.and_then(|id| profile.project(id));
        let entries = with_members(entries, project)
            .into_iter()
            .filter(|entry| display_listed(entry, registry) && filter.matches(entry))
            .collect();
        Ok(Response::Variables { entries })
    }

    /// `resolveDisplayTemplates`: each template as a key shows it.
    pub(super) fn resolve_display_templates(
        &self,
        context: &ResolutionContext,
        templates: Vec<String>,
    ) -> Result<Response, IpcError> {
        check_templates(&templates)
            .map_err(|message| IpcError::new(ErrorCode::InvalidRequest, message))?;
        let texts = self.with_display(context, |_, environment| {
            templates
                .into_iter()
                .map(|template| {
                    let display = resolve_display_template(environment, &template);
                    ResolvedDisplayTemplate { template, display }
                })
                .collect::<Vec<_>>()
        })?;
        Ok(Response::DisplayTemplates { texts })
    }

    /// Variables changed (`sharedVariablesChanged`): schedules a redraw of
    /// the devices whose shown texts (titles, labels) depend on one of
    /// them, coalesced per device (`display_redraws.rs`), and nothing else.
    pub(super) fn display_variables_changed(&mut self, keys: &[String]) {
        let devices = self.titles.devices_for(keys);
        debug!(
            changed = keys.len(),
            devices = devices.len(),
            "variables changed; redrawing the texts that use them"
        );
        let now = std::time::Instant::now();
        for device in devices {
            self.display_redraws.schedule(&device, now);
        }
    }

    /// Events were missed: every device with a dynamic text is redrawn
    /// (the frame diff sends only what changed), coalesced as well.
    pub(super) fn display_events_lagged(&mut self) {
        let now = std::time::Instant::now();
        for device in self.titles.dynamic_devices() {
            self.display_redraws.schedule(&device, now);
        }
    }

    /// The coalesced variable redraws that are due.
    pub(super) fn on_display_redraws_due(&mut self) {
        for device in self.display_redraws.take_due(std::time::Instant::now()) {
            self.handle
                .variable_redraws
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.render(&device, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_core::variables::TextSlot;
    use deckpilot_core::{PageId, Trigger};
    use deckpilot_device::{ControlId, DeviceModelId, models};

    fn device(serial: &str) -> DeviceId {
        DeviceId {
            model: DeviceModelId::new(models::STREAM_DECK_MK2_ID),
            serial: serial.into(),
        }
    }

    fn text(key: u16, reference: Option<&str>) -> (TextKey, DisplayText) {
        let mut text = DisplayText::plain("x");
        if let Some(reference) = reference {
            text.dependencies.insert(DisplayDependency::Variable {
                reference: reference.into(),
            });
        }
        (
            TextKey {
                page: PageId::new(),
                control: ControlId::Key { index: key },
                trigger: Trigger::Press,
                slot: TextSlot::Title,
            },
            text,
        )
    }

    #[test]
    fn only_devices_whose_texts_depend_on_a_change_are_named() {
        let mut titles = TitleDependencies::default();
        let (a, b) = (device("A"), device("B"));
        titles.record(&a, vec![text(0, Some("shared.team.name")), text(1, None)]);
        titles.record(&b, vec![text(0, Some("projects.current.key"))]);
        assert_eq!(titles.devices_for(&["team.name".into()]), vec![a.clone()]);
        assert_eq!(titles.affected(&a, &["team.name".into()]).len(), 1);
        assert_eq!(
            titles.devices_for(&["plugins.deckpilot.projects.current.key".into()]),
            vec![b.clone()]
        );
        assert_eq!(
            titles.devices_for(&["team.size".into()]),
            Vec::<DeviceId>::new()
        );
        // A device that shows only static texts is not kept.
        titles.record(&b, vec![text(0, None)]);
        assert_eq!(titles.dynamic_devices(), vec![a.clone()]);
        titles.forget(&a);
        assert_eq!(titles.dynamic_devices(), Vec::<DeviceId>::new());
    }
}
