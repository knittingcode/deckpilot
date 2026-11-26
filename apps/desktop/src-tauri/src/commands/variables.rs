//! Variable catalog and reference previews for the variable picker.
//!
//! Both are forwarded to the agent, which owns resolution
//! (docs/action-architecture.md §10). Values only ever come from the
//! agent's previews, with sensitive values redacted there; nothing here
//! reads variable values itself.
//!
//! Without a running agent, or with one older than protocol 7, the
//! commands fail with a message starting with [`NOT_SUPPORTED`]; the picker
//! then shows metadata without values.

use tauri::State;

use deckpilot_core::variables::{
    CatalogEntry, CatalogFilter, PreviewItem, ReferencePreview, ResolutionContext, expand_members,
};
use deckpilot_ipc::display_templates::ResolvedDisplayTemplate;
use deckpilot_ipc::{ClientError, MAX_PREVIEW_ITEMS, Request, Response};

use super::agent::unexpected;
use crate::agent;
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// Start of the error message when the agent cannot answer variable requests.
pub const NOT_SUPPORTED: &str = "not supported";

fn not_supported(error: ClientError) -> CommandError {
    match error {
        ClientError::NotRunning | ClientError::Closed => CommandError::invalid(
            "variablesAgentNotRunning",
            format!("{NOT_SUPPORTED}: start the DeckPilot agent to see variable values"),
        ),
        ClientError::Unsupported { .. } => CommandError::invalid(
            "variablesAgentTooOld",
            format!(
                "{NOT_SUPPORTED}: the running agent is too old for variables; restart the agent"
            ),
        ),
        other => other.into(),
    }
}

/// The references available in `context` that match `filter`: metadata,
/// never values. The agent lists a structured variable once, with its
/// shape's members and its keys; its member entries
/// (`….repositories.cms.name`) are composed here ([`expand_members`]), so
/// the agent's answer stays within one message however many repositories
/// a project has (protocol 18).
#[tauri::command]
pub async fn list_variables(
    state: State<'_, AppState>,
    context: ResolutionContext,
    filter: Option<CatalogFilter>,
) -> CommandResult<Vec<CatalogEntry>> {
    let filter = filter.unwrap_or_default();
    let request = Request::ListVariables {
        context: Box::new(context),
        filter: Box::new(filter.clone()),
    };
    match agent::request(&state.paths, request)
        .await
        .map_err(not_supported)?
    {
        Response::Variables { entries } => Ok(expand_members(entries, &filter)),
        other => Err(unexpected(&other)),
    }
}

/// What each reference resolves to in `context`, in the order asked.
#[tauri::command]
pub async fn preview_references(
    state: State<'_, AppState>,
    context: ResolutionContext,
    items: Vec<PreviewItem>,
) -> CommandResult<Vec<ReferencePreview>> {
    if items.is_empty() {
        return Ok(Vec::new());
    }
    if items.len() > MAX_PREVIEW_ITEMS {
        return Err(CommandError::invalid_with(
            "tooManyPreviews",
            format!("at most {MAX_PREVIEW_ITEMS} references can be previewed at once"),
            [("max", MAX_PREVIEW_ITEMS.into())],
        ));
    }
    let request = Request::PreviewReferences {
        context: Box::new(context),
        items,
    };
    match agent::request(&state.paths, request)
        .await
        .map_err(not_supported)?
    {
        Response::ReferencePreviews { previews } => Ok(previews),
        other => Err(unexpected(&other)),
    }
}

/// The references a key title may use in `context` that match `filter`:
/// display-safe sources only, metadata only (protocol 17,
/// `deckpilot_ipc::display_templates`). A structured variable comes for
/// its display-safe members only; they are composed here and the variable
/// itself is never offered ([`expand_members`], protocol 18).
#[tauri::command]
pub async fn list_display_variables(
    state: State<'_, AppState>,
    context: ResolutionContext,
    filter: Option<CatalogFilter>,
) -> CommandResult<Vec<CatalogEntry>> {
    let filter = filter.unwrap_or_default();
    let request = Request::ListDisplayVariables {
        context: Box::new(context),
        filter: Box::new(filter.clone()),
    };
    match agent::request(&state.paths, request)
        .await
        .map_err(not_supported)?
    {
        Response::Variables { entries } => Ok(expand_members(entries, &filter)),
        other => Err(unexpected(&other)),
    }
}

/// What each title template shows on a key in `context`, in the order
/// asked, resolved by the agent as the device shows it.
#[tauri::command]
pub async fn resolve_display_templates(
    state: State<'_, AppState>,
    context: ResolutionContext,
    templates: Vec<String>,
) -> CommandResult<Vec<ResolvedDisplayTemplate>> {
    if templates.is_empty() {
        return Ok(Vec::new());
    }
    if let Err(message) = deckpilot_ipc::display_templates::check_templates(&templates) {
        return Err(CommandError::invalid_with(
            "tooManyTemplates",
            message,
            [(
                "max",
                deckpilot_ipc::display_templates::MAX_DISPLAY_TEMPLATES.into(),
            )],
        ));
    }
    resolve_templates(&state.paths, context, templates)
        .await
        .map_err(not_supported)
}

/// `resolveDisplayTemplates` on the agent.
pub async fn resolve_templates(
    paths: &deckpilot_ipc::AgentPaths,
    context: ResolutionContext,
    templates: Vec<String>,
) -> Result<Vec<ResolvedDisplayTemplate>, ClientError> {
    let request = Request::ResolveDisplayTemplates {
        context: Box::new(context),
        templates,
    };
    match agent::request(paths, request).await? {
        Response::DisplayTemplates { texts } => Ok(texts),
        _ => Err(ClientError::Closed),
    }
}
