//! **Experimental.** Client-side verification of the signed, static
//! DeckPilot plugin registry.
//!
//! Nothing in DeckPilot uses this crate yet: no app, agent or module crate
//! depends on it, and plugins stay disabled. It implements sections 9, 10,
//! 12.3 (steps 1 and 2), 13.5 and 14 of the plugin marketplace design
//! (`docs/marketplace/architecture.md`, proposed in pull request #191),
//! promoting the retired trust spike (`docs/marketplace/spike-findings.md`).
//!
//! There is no network code. The client takes document bytes (directly,
//! or through a [`RegistrySource`] the agent's fetcher implements) and a
//! [`TrustStore`] for its state.
//!
//! ```text
//! embedded roots ─signs→ keys/<n>.json ─names→ index + timestamp keys
//! timestamp.json ─pins (sha256, length)→ index/<seq>.json
//! index/<seq>.json ─pins→ plugins/<id>/<sha256>.json ─pins→ .dpplugin
//! ```
//!
//! Decisions on the spike's open findings are documented in the README of
//! this crate: freshness comes from the timestamp alone (the index expiry
//! is not enforced by default), revocations can be lifted only by a
//! root-signed key set with `seq_floor_reset`, and `seq_floor` is a floor
//! (`max`) unless reset.
//!
//! With the `publish` feature, the `publish` module and the `deckpilot-registry`
//! binary add the offline tooling that generates and signs the site.
//!
//! [`storefront`] verifies the signed storefront (editorial placements and
//! listing content, bound to the verified index; signed by the index role
//! with its own trusted comment) and [`media`] walks the headers of its
//! images. Older clients never fetch either.
//!
//! [`publisher`] verifies the first-party publisher signature of a
//! `.dpplugin` release artifact of DeckPilot's own plugins, before an
//! application build bundles it (authenticity only; it grants nothing).
//! [`privileged_policy`] is its privileged-kinds gate (closed) and the
//! reviewed policy an opened gate would admit, exactly.

mod client;
mod datetime;
mod error;
pub mod media;
mod model;
pub mod privileged_policy;
#[cfg(feature = "publish")]
pub mod publish;
pub mod publisher;
pub mod roots;
#[cfg(test)]
mod roots_check;
pub mod sig;
mod state;
pub mod storefront;

pub use client::{
    Freshness, InstallTarget, InstalledVerdict, MAX_KEYSET_BYTES, MAX_KEYSET_SIGNATURES,
    MAX_KEYSET_SIGNATURES_BYTES, MAX_SIGNATURE_BYTES, MAX_TIMESTAMP_BYTES, Policy, Refresh,
    RegistryClient, RegistrySource, StaleReason, VerifiedTimestamp,
};
pub use error::{FetchError, InstallError, RegistryError};
pub use model::*;
pub use state::{
    FileTrustStore, MemoryTrustStore, STATE_FORMAT, StoreError, StorefrontRecord, TrustState,
    TrustStore, write_atomic,
};

/// The #191 registry JSON Schemas (draft 2020-12), unchanged. They refer to
/// the manifest schema (`deckpilot_plugin_manifest::SCHEMA_JSON`) by its
/// `$id`.
pub mod schemas {
    pub const KEYSET: &str = include_str!("../schema/registry-keyset.schema.json");
    pub const TIMESTAMP: &str = include_str!("../schema/registry-timestamp.schema.json");
    pub const INDEX: &str = include_str!("../schema/registry-index.schema.json");
    pub const PLUGIN: &str = include_str!("../schema/registry-plugin.schema.json");
    /// `storefront/<seq>.json` (storefront design §3.3).
    pub const STOREFRONT: &str = include_str!("../schema/registry-storefront.schema.json");
    /// `storefront/listings/<id>/<sha256>.json`.
    pub const LISTING: &str = include_str!("../schema/registry-listing.schema.json");
    /// `storefront/editorial/<sha256>.json`.
    pub const EDITORIAL: &str = include_str!("../schema/registry-editorial.schema.json");
    /// The author source `registry/plugins/<id>/listing.json` (§3.1).
    pub const LISTING_SOURCE: &str = include_str!("../schema/inputs/listing.schema.json");
    /// The owner source `editorial/editorial.json` (§3.2).
    pub const EDITORIAL_SOURCE: &str = include_str!("../schema/inputs/editorial.schema.json");
}
