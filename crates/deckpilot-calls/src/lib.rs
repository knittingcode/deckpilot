//! The Calls contracts of DeckPilot (`docs/calls.md`).
//!
//! - [`provider`]: the adapter contract every call provider implements,
//!   with capability discovery and an explicit unsupported state for every
//!   capability (a provider never leaves one out, and nothing assumes a
//!   capability works).
//! - [`snapshot`]: the sanitised call state the plugin, the key faces and
//!   the IPC see. It never holds participants, numbers, meeting links or
//!   meeting ids; a title only when the user opted in.
//! - [`ops`]: the host operations `call.state` and `call.control` of the
//!   bundled Calls plugin, and the gate an action passes before a command
//!   is sent.
//! - [`engine`]: the call state engine: tracks, timeouts, focus, and the
//!   overlay intent.
//! - [`ret`]: "Wróć do pracy" (Back to work): the per-device return
//!   sessions.
//! - [`clock`]: the engine's monotonic clock. Nothing here reads a clock:
//!   every call that needs the time is given `now`, so tests run on a
//!   virtual clock.
//!
//! This crate is pure: no I/O, no threads, no clock reads. Provider
//! adapters live in the agent.

pub mod clock;
pub mod driver;
pub mod engine;
pub mod layouts;
pub mod ops;
pub mod provider;
pub mod ret;
pub mod snapshot;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use clock::Instant;
pub use ops::{CallControlRequest, CallControlResult, GateError, OP_CALL_CONTROL, OP_CALL_STATE};
pub use provider::{
    AppVersion, Availability, CallAction, CallCommand, CallKey, CallKind, CallObservation,
    CallProvider, Capability, CapabilitySet, CommandOutcome, CommandRefusal, CommandToken,
    Condition, ConsentState, Direction, DisconnectReason, Discovery, Evidence, ObservedEnd,
    ObservedPhase, Private, ProviderCx, ProviderError, ProviderEvent, ProviderId, ProviderSession,
    Support, Tri, UnsupportedReason,
};
pub use snapshot::{CallSnapshot, CallState, EndCause, FailureCode};
