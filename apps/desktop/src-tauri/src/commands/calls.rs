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

// ------------------------------------------------- the editor's preview

/// The Device Editor's preview of a call page (`previewCallsPage`, protocol
/// 19): it lives on a connection of its own, as the live preview does,
/// because the agent ends a preview when the connection that asked closes.
/// One at a time: a new one replaces the old. It ends when the editor ends
/// it, when the main window hides, reloads or closes, and when this app
/// quits (the connection closes with the process); the agent also ends it
/// on lock, on a real call and on the device's disconnect.
#[derive(Debug, Default)]
pub struct CallsPreview {
    /// Ends the running preview: its task sends `endCallsPreview` and then
    /// closes the connection.
    running: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl CallsPreview {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<tokio::sync::oneshot::Sender<()>>> {
        self.running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Ends the running preview, if any: `endCallsPreview`, then its
    /// connection closes (which ends it on the agent too, should the
    /// request not get through, e.g. as the app quits).
    pub fn stop(&self) {
        if let Some(end) = self.lock().take() {
            let _ = end.send(());
        }
    }
}

/// Shows `page` of `device`'s Calls layout on the device as the editor
/// previews it (fixed content, no call). Replaces a running preview.
#[tauri::command]
pub async fn calls_preview_begin(
    state: State<'_, AppState>,
    preview: State<'_, CallsPreview>,
    device: DeviceId,
    page: deckpilot_ipc::calls::CallsPageKind,
) -> CommandResult<()> {
    preview.stop();
    let mut client =
        deckpilot_ipc::AgentClient::connect(&state.paths, deckpilot_ipc::ClientKind::Desktop)
            .await
            .map_err(agent_error)?;
    match client
        .request(Request::PreviewCallsPage {
            device: device.clone(),
            page,
        })
        .await
        .map_err(agent_error)?
    {
        Response::Ok => {}
        other => return Err(unexpected(&other)),
    }
    // The connection holds the preview until it is ended.
    let (end, ended) = tokio::sync::oneshot::channel::<()>();
    tauri::async_runtime::spawn(async move {
        let _ = ended.await;
        let _ = client.request(Request::EndCallsPreview { device }).await;
    });
    *preview.lock() = Some(end);
    Ok(())
}

/// Ends the editor's preview, if one runs.
#[tauri::command]
pub fn calls_preview_end(preview: State<'_, CallsPreview>) {
    preview.stop();
}
