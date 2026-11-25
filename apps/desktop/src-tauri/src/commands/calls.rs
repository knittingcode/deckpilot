//! Calls (protocol 19, `docs/calls.md`): its state, the per-device overlay
//! setting and *Restore defaults* of a managed layout. The agent owns
//! Calls; these commands only ask it. With an agent below protocol 19, or
//! none, every command answers a reason the settings page explains, and the
//! `agent-calls` event brings every change.

use tauri::State;

use deckpilot_device::{DeviceId, DeviceModelId};
use deckpilot_ipc::calls::{BackToWork, CallsStatus};
use deckpilot_ipc::{ClientError, Request, Response};

use super::agent::unexpected;
use crate::agent;
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

async fn ask(state: &AppState, request: Request) -> CommandResult<Response> {
    agent::request(&state.paths, request)
        .await
        .map_err(agent_error)
}

fn agent_error(error: ClientError) -> CommandError {
    match error {
        ClientError::NotRunning => CommandError::invalid(
            "callsAgentNotRunning",
            "start the DeckPilot agent to see Calls",
        ),
        ClientError::Unsupported { agent, .. } => CommandError::invalid_with(
            "callsAgentTooOld",
            format!("Calls is not supported by the running agent ({agent})"),
            [("agent", agent.into())],
        ),
        other => other.into(),
    }
}

/// Calls as it is now.
#[tauri::command]
pub async fn get_calls_status(state: State<'_, AppState>) -> CommandResult<CallsStatus> {
    match ask(&state, Request::GetCallsStatus).await? {
        Response::CallsStatus { status } => Ok(*status),
        other => Err(unexpected(&other)),
    }
}

/// Turns the overlay on or off for `device`.
#[tauri::command]
pub async fn set_calls_overlay(
    state: State<'_, AppState>,
    device: DeviceId,
    on: bool,
) -> CommandResult<CallsStatus> {
    match ask(&state, Request::SetCallsOverlay { device, on }).await? {
        Response::CallsStatus { status } => Ok(*status),
        other => Err(unexpected(&other)),
    }
}

/// Sets the *Back to work* settings of `model`'s Calls layout (the agent
/// checks the times are in range).
#[tauri::command]
pub async fn set_calls_back_to_work(
    state: State<'_, AppState>,
    model: DeviceModelId,
    back: BackToWork,
) -> CommandResult<CallsStatus> {
    match ask(&state, Request::SetCallsBackToWork { model, back }).await? {
        Response::CallsStatus { status } => Ok(*status),
        other => Err(unexpected(&other)),
    }
}

/// *Restore defaults* of `model`'s Calls layout; the user's version is kept
/// first as `copy_name`. Answers the kept copy's id.
#[tauri::command]
pub async fn restore_calls_layout(
    state: State<'_, AppState>,
    model: DeviceModelId,
    copy_name: String,
) -> CommandResult<Option<String>> {
    match ask(&state, Request::RestoreCallsLayout { model, copy_name }).await? {
        Response::CallsLayoutRestored { copy } => Ok(copy),
        other => Err(unexpected(&other)),
    }
}
