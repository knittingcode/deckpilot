// Calls (protocol 19, docs/calls.md): the backend event and helpers of the
// Calls settings. Rust: deckpilot_ipc::calls, src-tauri commands/calls.rs.

/** Backend event with the latest {@link CallsStatus}. */
export const CALLS_EVENT = "agent-calls"

/** The call states the agent names, for their translations. */
export const CALL_STATES = [
  "idle",
  "incoming",
  "outgoing",
  "connecting",
  "active",
  "ending",
  "ended",
  "failed",
  "unknown",
] as const

export type CallStateName = (typeof CALL_STATES)[number]

export function callStateName(state: string): CallStateName {
  return (CALL_STATES as readonly string[]).includes(state) ? (state as CallStateName) : "unknown"
}
