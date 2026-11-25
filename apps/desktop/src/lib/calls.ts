// Calls (protocol 19, docs/calls.md): the backend event and helpers of the
// Calls settings. Rust: deckpilot_ipc::calls, src-tauri commands/calls.rs.

/** Backend event with the latest {@link CallsStatus}. */
export const CALLS_EVENT = "agent-calls"

/**
 * The provider of the detection-only Teams adapter (operating-system
 * signals): the only one whose missing controls are Teams'.
 */
export const OS_SIGNALS_PROVIDER = "os-signals"

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

/** The *Back to work* timer presets (seconds). */
export const BACK_TIMER_PRESETS = [20, 30, 60] as const
/** Shortest and longest *Back to work* timer (mode A), in seconds. */
export const BACK_TIMER_RANGE = [5, 600] as const
/** Shortest and longest maximum of mode C, in seconds. */
export const BACK_MAX_RANGE = [10, 1800] as const
/** Mode C's maximum when it is chosen anew. */
export const BACK_DEFAULT_MAX = 60

/** Why `text` is not a number of seconds in `[min, max]`; `null` when it is. */
export function checkSeconds(text: string, [min, max]: readonly [number, number]): string | null {
  const trimmed = text.trim()
  if (!/^\d+$/.test(trimmed)) return "whole"
  const seconds = Number(trimmed)
  return seconds < min || seconds > max ? "range" : null
}

/** The translation of a Calls action (`answer`, `mic-toggle`, ...), by key. */
export const CALL_ACTIONS = {
  answer: "devices:calls.actions.answer",
  reject: "devices:calls.actions.reject",
  "cancel-outgoing": "devices:calls.actions.cancelOutgoing",
  end: "devices:calls.actions.end",
  "mic-toggle": "devices:calls.actions.micToggle",
  "mic-mute": "devices:calls.actions.micMute",
  "mic-unmute": "devices:calls.actions.micUnmute",
  "camera-toggle": "devices:calls.actions.cameraToggle",
  "camera-on": "devices:calls.actions.cameraOn",
  "camera-off": "devices:calls.actions.cameraOff",
  "return-to-work": "devices:calls.actions.returnToWork",
  "show-calls": "devices:calls.actions.showCalls",
} as const

/** The translation of why a control is disabled, by its stable code. */
export const CALL_REASONS = {
  "provider-has-no-api": "devices:calls.reasons.providerHasNoApi",
  "platform-not-supported": "devices:calls.reasons.platformNotSupported",
  "not-in-meeting": "devices:calls.reasons.notInMeeting",
  "no-permission-in-meeting": "devices:calls.reasons.noPermissionInMeeting",
  "toggle-only": "devices:calls.reasons.toggleOnly",
  "needs-pairing": "devices:calls.reasons.needsPairing",
  "needs-os-permission": "devices:calls.reasons.needsOsPermission",
  "consent-not-given": "devices:calls.reasons.consentNotGiven",
} as const
