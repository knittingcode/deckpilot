//! The signed storefront: editorial placements and listing content beside
//! the registry index (storefront design §3, §5 and §6).
//!
//! ```text
//! timestamp.json ─pins→ index/<seq>.json
//! storefront/<seq>.json (+ .minisig: index role, comment role:storefront)
//!     ─binds→ the timestamp's index {path, sha256, length}
//!     ─pins→ storefront/listings/<id>/<sha256>.json ─pins→ media/<sha256>.<ext>
//!     ─pins→ storefront/editorial/<sha256>.json    ─pins→ media/<sha256>.<ext>
//! ```
//!
//! Nothing here changes an existing registry document, role or check: the
//! storefront lives at new paths that older clients never request, and it
//! is signed by the existing index role with its own exact trusted comment
//! ([`crate::sig::storefront_comment`]). A storefront that fails any check
//! is simply absent (catalog-only Discover); it never changes the result of
//! [`RegistryClient::refresh`] or of an install.
//!
//! Parser strictness: the envelope and asset references refuse unknown
//! fields; listing and editorial content ignore unknown fields, and an
//! editorial section of an unknown `kind` is skipped (and reported). An
//! unknown `storefront_version`, `listing_version` or `editorial_version`
//! refuses that one document.

pub mod rules;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use chrono::{DateTime, Utc};
use deckpilot_plugin_manifest::{FIRST_PARTY_PUBLISHER, PluginId, Sha256Digest};
use deckpilot_plugin_package::sha256;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::check_ref;
use crate::media::{self, MediaError, MediaFormat, RasterInfo};
use crate::model::{FileRef, MAX_INDEX_BYTES};
use crate::sig::{self, SigError, storefront_comment};
use crate::state::{StoreError, StorefrontRecord, TrustStore};
use crate::{MAX_SIGNATURE_BYTES, RegistryClient};

pub use rules::{
    AssetKind, Localized, Strictness, check_localized, check_markdown, check_text, check_url,
    localize,
};

/// The only `storefront_version` this crate reads.
pub const STOREFRONT_VERSION: u64 = 1;
/// The only `listing_version` this crate reads.
pub const LISTING_VERSION: u64 = 1;
/// The only `editorial_version` this crate reads.
pub const EDITORIAL_VERSION: u64 = 1;

/// Largest `storefront/<seq>.json`.
pub const MAX_STOREFRONT_BYTES: usize = 64 * 1024;
/// Largest `storefront/listings/<id>/<sha256>.json`.
pub const MAX_LISTING_BYTES: usize = 64 * 1024;
/// Largest `storefront/editorial/<sha256>.json`.
pub const MAX_EDITORIAL_BYTES: usize = 128 * 1024;
pub use crate::media::{MAX_MEDIA_BYTES, MAX_MEDIA_PIXELS};

/// Most media bytes one plugin's listing may reference.
pub const MAX_PLUGIN_MEDIA_BYTES: u64 = 8 * 1024 * 1024;
/// Most media bytes the editorial document may reference.
pub const MAX_EDITORIAL_MEDIA_BYTES: u64 = 16 * 1024 * 1024;

/// Most screenshots per listing.
pub const MAX_SCREENSHOTS: usize = 8;
/// Most highlights per listing.
pub const MAX_HIGHLIGHTS: usize = 6;
/// Most editorial sections.
pub const MAX_SECTIONS: usize = 12;
/// Most editorial collections.
pub const MAX_COLLECTIONS: usize = 16;

/// The section kinds this crate understands; others are skipped.
pub const SECTION_KINDS: [&str; 6] = [
    "banner_carousel",
    "collection_row",
    "spotlight",
    "recently_added",
    "category_highlights",
    "catalog",
];

macro_rules! doc_type {
    ($(#[$meta:meta])* $name:ident = $value:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name {
            #[default]
            #[serde(rename = $value)]
            Value,
        }
    };
}

doc_type!(
    /// `_type` of `storefront/<seq>.json`.
    StorefrontType = "deckpilot-registry-storefront"
);
doc_type!(
    /// `_type` of `storefront/listings/<id>/<sha256>.json`.
    ListingType = "deckpilot-registry-listing"
);
doc_type!(
    /// `_type` of `storefront/editorial/<sha256>.json`.
    EditorialType = "deckpilot-registry-editorial"
);

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's signature
fn is_false(value: &bool) -> bool {
    !*value
}

// ------------------------------------------------------------- envelope

/// A listing pinned by the envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListingEntry {
    pub id: PluginId,
    #[serde(rename = "ref")]
    pub reference: FileRef,
}

/// `storefront/<seq>.json`: the signed envelope (strict).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Storefront {
    #[serde(rename = "_type")]
    pub kind: StorefrontType,
    pub spec_version: u64,
    pub storefront_version: u64,
    pub seq: u64,
    pub keys_version: u64,
    #[serde(with = "crate::datetime")]
    pub generated_at: DateTime<Utc>,
    pub source_commit: String,
    /// Exactly the index the timestamp of `seq` pins.
    pub index: FileRef,
    pub listings: Vec<ListingEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editorial: Option<FileRef>,
}

/// `storefront/listings/<id>/<sha256>.json`.
pub fn listing_path(id: &PluginId, digest: &Sha256Digest) -> String {
    format!("storefront/listings/{id}/{digest}.json")
}

/// `storefront/editorial/<sha256>.json`.
pub fn editorial_path(digest: &Sha256Digest) -> String {
    format!("storefront/editorial/{digest}.json")
}

/// `storefront/<seq>.json`.
pub fn storefront_path(seq: u64) -> String {
    format!("storefront/{seq}.json")
}

/// `media/<sha256>.<ext>`: derived from the reference, never read from a
/// document.
pub fn media_path(asset: &AssetRef) -> String {
    format!("media/{}.{}", asset.sha256, asset.format.extension())
}

impl Storefront {
    pub fn validate(&self) -> Result<(), String> {
        if self.spec_version != crate::SPEC_VERSION {
            return Err(format!("spec_version {} is not 1", self.spec_version));
        }
        if self.storefront_version != STOREFRONT_VERSION {
            return Err(format!(
                "storefront_version {} is not {STOREFRONT_VERSION}",
                self.storefront_version
            ));
        }
        if self.seq == 0 || self.keys_version == 0 {
            return Err("seq and keys_version must be at least 1".into());
        }
        if self.source_commit.len() != 40
            || !self
                .source_commit
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err("source_commit is not a 40-digit commit hash".into());
        }
        if self.index.path != format!("index/{}.json", self.seq) {
            return Err(format!("index.path is not index/{}.json", self.seq));
        }
        if self.index.length == 0 || self.index.length > MAX_INDEX_BYTES {
            return Err("index.length is out of range".into());
        }
        let mut ids = BTreeSet::new();
        for (i, entry) in self.listings.iter().enumerate() {
            if !ids.insert(&entry.id) {
                return Err(format!("listings[{i}].id {} appears twice", entry.id));
            }
            let path = listing_path(&entry.id, &entry.reference.sha256);
            if entry.reference.path != path {
                return Err(format!("listings[{i}].ref.path is not {path}"));
            }
            if entry.reference.length == 0 || entry.reference.length > MAX_LISTING_BYTES as u64 {
                return Err(format!("listings[{i}].ref.length is out of range"));
            }
        }
        if let Some(editorial) = &self.editorial {
            let path = editorial_path(&editorial.sha256);
            if editorial.path != path {
                return Err(format!("editorial.path is not {path}"));
            }
            if editorial.length == 0 || editorial.length > MAX_EDITORIAL_BYTES as u64 {
                return Err("editorial.length is out of range".into());
            }
        }
        Ok(())
    }

    pub fn listing(&self, id: &PluginId) -> Option<&ListingEntry> {
        self.listings.iter().find(|l| l.id == *id)
    }
}

// ---------------------------------------------------------------- assets

/// A pinned image (strict): what the bytes at [`media_path`] must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetRef {
    pub sha256: Sha256Digest,
    pub length: u64,
    pub format: MediaFormat,
    pub width: u32,
    pub height: u32,
}

impl AssetRef {
    pub fn check(&self, kind: AssetKind) -> Result<(), String> {
        kind.check(self.format, self.width, self.height, self.length)
    }
}

/// Why a media file does not match its reference.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MediaMismatch {
    #[error("is {found} bytes, pinned {pinned}")]
    Length { found: u64, pinned: u64 },
    #[error("has sha256 {found}, pinned {pinned}")]
    Digest {
        found: Sha256Digest,
        pinned: Sha256Digest,
    },
    #[error("{0}")]
    Media(#[from] MediaError),
    #[error(
        "is a {found:?} {width}×{height} image, pinned {pinned_format:?} {pinned_width}×{pinned_height}"
    )]
    Header {
        found: MediaFormat,
        width: u32,
        height: u32,
        pinned_format: MediaFormat,
        pinned_width: u32,
        pinned_height: u32,
    },
}

/// Checks media bytes against their reference (§6): length exactly, then
/// sha256, then the header walk ([`media::inspect`]), then format and
/// dimensions equal to the reference. No pixel is decoded.
pub fn verify_media(asset: &AssetRef, bytes: &[u8]) -> Result<RasterInfo, MediaMismatch> {
    let found = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if found != asset.length {
        return Err(MediaMismatch::Length {
            found,
            pinned: asset.length,
        });
    }
    let digest = sha256(bytes);
    if digest != asset.sha256 {
        return Err(MediaMismatch::Digest {
            found: digest,
            pinned: asset.sha256,
        });
    }
    let info = media::inspect(bytes)?;
    if info.format != asset.format || info.width != asset.width || info.height != asset.height {
        return Err(MediaMismatch::Header {
            found: info.format,
            width: info.width,
            height: info.height,
            pinned_format: asset.format,
            pinned_width: asset.width,
            pinned_height: asset.height,
        });
    }
    Ok(info)
}

/// An image with its text alternative. `decorative` (banner art only)
/// allows an empty `alt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Art {
    pub asset: AssetRef,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub alt: Localized,
    #[serde(default, skip_serializing_if = "is_false")]
    pub decorative: bool,
}

// --------------------------------------------------------------- listing

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Highlight {
    pub title: Localized,
    pub text: Localized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Screenshot {
    pub asset: AssetRef,
    pub alt: Localized,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<Localized>,
    /// `macos`, `windows`, `linux`; absent: every platform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Links {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub support: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelog: Option<String>,
}

/// `storefront/listings/<id>/<sha256>.json` (lenient: unknown fields are
/// ignored). An author's listing; it never carries placements or badges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListingDoc {
    #[serde(rename = "_type")]
    pub kind: ListingType,
    pub listing_version: u64,
    pub id: PluginId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promo: Option<Localized>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_md: Option<Localized>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub highlights: Vec<Highlight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<AssetRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screenshots: Vec<Screenshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promo_art: Option<Art>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Links>,
    /// Version → notes.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub release_notes: BTreeMap<String, Localized>,
}

/// The platforms a document may target.
pub const PLATFORMS: [&str; 3] = ["macos", "windows", "linux"];

fn check_platforms(
    field: &str,
    platforms: Option<&Vec<String>>,
    strictness: Strictness,
) -> Result<(), String> {
    let Some(platforms) = platforms else {
        return Ok(());
    };
    let unique: BTreeSet<_> = platforms.iter().collect();
    if platforms.is_empty() || unique.len() != platforms.len() {
        return Err(format!("{field}: at least one, all different"));
    }
    if strictness == Strictness::Publish
        && let Some(bad) = platforms.iter().find(|p| !PLATFORMS.contains(&p.as_str()))
    {
        return Err(format!("{field} names the unknown platform `{bad}`"));
    }
    Ok(())
}

impl ListingDoc {
    /// Every asset with its kind.
    pub fn assets(&self) -> Vec<(AssetKind, &AssetRef)> {
        let mut out = Vec::new();
        if let Some(icon) = &self.icon {
            out.push((AssetKind::Icon, icon));
        }
        for shot in &self.screenshots {
            out.push((AssetKind::Screenshot, &shot.asset));
        }
        if let Some(art) = &self.promo_art {
            out.push((AssetKind::PromoArt, &art.asset));
        }
        out
    }

    /// The §3.1 and §4 rules (text, links, markdown, counts, assets).
    #[allow(clippy::too_many_lines)] // one rule after the other
    pub fn validate(&self, strictness: Strictness) -> Result<(), String> {
        if self.listing_version != LISTING_VERSION {
            return Err(format!(
                "listing_version {} is not {LISTING_VERSION}",
                self.listing_version
            ));
        }
        let first_party = self.id.publisher() == FIRST_PARTY_PUBLISHER;
        if let Some(promo) = &self.promo {
            check_localized("promo", promo, 1, 80, false, strictness)?;
        }
        if let Some(description) = &self.description_md {
            check_localized("description_md", description, 1, 6000, true, strictness)?;
            for (tag, text) in description {
                check_markdown(&format!("description_md.{tag}"), text, first_party)?;
            }
        }
        if self.highlights.len() > MAX_HIGHLIGHTS {
            return Err(format!("at most {MAX_HIGHLIGHTS} highlights"));
        }
        for (i, highlight) in self.highlights.iter().enumerate() {
            check_localized(
                &format!("highlights[{i}].title"),
                &highlight.title,
                1,
                40,
                false,
                strictness,
            )?;
            check_localized(
                &format!("highlights[{i}].text"),
                &highlight.text,
                1,
                160,
                false,
                strictness,
            )?;
        }
        if self.screenshots.len() > MAX_SCREENSHOTS {
            return Err(format!("at most {MAX_SCREENSHOTS} screenshots"));
        }
        for (i, shot) in self.screenshots.iter().enumerate() {
            let at = format!("screenshots[{i}]");
            check_localized(&format!("{at}.alt"), &shot.alt, 1, 250, false, strictness)?;
            if let Some(caption) = &shot.caption {
                check_localized(&format!("{at}.caption"), caption, 1, 120, false, strictness)?;
            }
            check_platforms(
                &format!("{at}.platforms"),
                shot.platforms.as_ref(),
                strictness,
            )?;
        }
        if let Some(art) = &self.promo_art {
            if art.decorative {
                return Err("promo_art cannot be decorative; give it alt text".into());
            }
            check_localized("promo_art.alt", &art.alt, 1, 250, false, strictness)?;
        }
        if let Some(links) = &self.links {
            for (name, url) in [
                ("documentation", &links.documentation),
                ("support", &links.support),
                ("changelog", &links.changelog),
            ] {
                if let Some(url) = url {
                    check_url(&format!("links.{name}"), url, first_party)?;
                }
            }
        }
        for (version, notes) in &self.release_notes {
            let parsed = Version::parse(version)
                .map_err(|e| format!("release_notes key `{version}`: {e}"))?;
            if parsed.to_string() != *version {
                return Err(format!("release_notes key `{version}` is not canonical"));
            }
            let at = format!("release_notes.{version}");
            check_localized(&at, notes, 1, 2000, true, strictness)?;
            for (tag, text) in notes {
                check_markdown(&format!("{at}.{tag}"), text, first_party)?;
            }
        }
        let mut total = 0u64;
        for (kind, asset) in self.assets() {
            asset.check(kind)?;
            total += asset.length;
        }
        if total > MAX_PLUGIN_MEDIA_BYTES {
            return Err(format!(
                "the listing references {total} media bytes; at most {MAX_PLUGIN_MEDIA_BYTES}"
            ));
        }
        Ok(())
    }
}

// ------------------------------------------------------------- editorial

/// A time window (UTC); either end may be open.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::datetime::option"
    )]
    pub starts_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::datetime::option"
    )]
    pub ends_at: Option<DateTime<Utc>>,
}

impl Window {
    /// Whether `now` is inside: `starts_at <= now < ends_at`.
    pub fn contains(&self, now: DateTime<Utc>) -> bool {
        self.starts_at.is_none_or(|s| s <= now) && self.ends_at.is_none_or(|e| now < e)
    }
}

/// Where a banner leads: `plugin`, `collection` or `category` (kept as
/// text, so an older client drops a target type it does not know instead
/// of refusing the document). No external links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
}

pub const TARGET_TYPES: [&str; 3] = ["plugin", "collection", "category"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BannerItem {
    pub id: String,
    pub title: Localized,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Localized>,
    pub artwork: Art,
    pub target: Target,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cta: Option<Localized>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryBlurb {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blurb: Option<Localized>,
}

/// What a section shows, by `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SectionBody {
    BannerCarousel {
        items: Vec<BannerItem>,
    },
    CollectionRow {
        /// `cards` or `compact`.
        #[serde(default = "default_layout")]
        layout: String,
        title: Localized,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<Localized>,
        plugins: Vec<PluginId>,
    },
    Spotlight {
        plugin: PluginId,
        /// Only selects which release notes to show.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<Version>,
        headline: Localized,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<Localized>,
    },
    RecentlyAdded {
        limit: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<Localized>,
    },
    CategoryHighlights {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<Localized>,
        categories: Vec<CategoryBlurb>,
    },
    Catalog {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<Localized>,
    },
}

fn default_layout() -> String {
    "cards".to_owned()
}

impl SectionBody {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::BannerCarousel { .. } => "banner_carousel",
            Self::CollectionRow { .. } => "collection_row",
            Self::Spotlight { .. } => "spotlight",
            Self::RecentlyAdded { .. } => "recently_added",
            Self::CategoryHighlights { .. } => "category_highlights",
            Self::Catalog { .. } => "catalog",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub id: String,
    pub order: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<Window>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
    #[serde(flatten)]
    pub body: SectionBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collection {
    pub id: String,
    pub order: i64,
    pub title: Localized,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<Localized>,
    pub plugins: Vec<PluginId>,
}

/// `storefront/editorial/<sha256>.json` (lenient). Owner-written
/// placements; published entries only (drafts never reach the site).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EditorialDoc {
    #[serde(rename = "_type")]
    pub kind: EditorialType,
    pub editorial_version: u64,
    pub sections: Vec<Section>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collections: Vec<Collection>,
    /// Sections of a kind this crate does not know: `(id, kind)`, skipped.
    #[serde(skip)]
    pub skipped: Vec<(String, String)>,
}

#[derive(Deserialize)]
struct RawEditorial {
    #[serde(rename = "_type")]
    kind: EditorialType,
    editorial_version: u64,
    #[serde(default)]
    sections: Vec<Value>,
    #[serde(default)]
    collections: Vec<Collection>,
}

#[derive(Deserialize)]
struct SectionHead {
    #[serde(default)]
    id: String,
    kind: String,
}

impl EditorialDoc {
    /// Parses leniently: unknown fields are ignored and sections of an
    /// unknown kind are skipped (listed in [`EditorialDoc::skipped`]). A
    /// malformed section of a known kind refuses the document.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let raw: RawEditorial = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if raw.editorial_version != EDITORIAL_VERSION {
            return Err(format!(
                "editorial_version {} is not {EDITORIAL_VERSION}",
                raw.editorial_version
            ));
        }
        let mut sections = Vec::new();
        let mut skipped = Vec::new();
        for (i, value) in raw.sections.into_iter().enumerate() {
            let head: SectionHead =
                serde_json::from_value(value.clone()).map_err(|e| format!("sections[{i}]: {e}"))?;
            if !SECTION_KINDS.contains(&head.kind.as_str()) {
                skipped.push((head.id, head.kind));
                continue;
            }
            let section: Section =
                serde_json::from_value(value).map_err(|e| format!("sections[{i}]: {e}"))?;
            sections.push(section);
        }
        Ok(Self {
            kind: raw.kind,
            editorial_version: raw.editorial_version,
            sections,
            collections: raw.collections,
            skipped,
        })
    }

    /// Every asset with its kind.
    pub fn assets(&self) -> Vec<&AssetRef> {
        self.sections
            .iter()
            .filter_map(|s| match &s.body {
                SectionBody::BannerCarousel { items } => Some(items),
                _ => None,
            })
            .flatten()
            .map(|item| &item.artwork.asset)
            .collect()
    }

    /// The §3.2 rules.
    #[allow(clippy::too_many_lines)] // one rule after the other
    pub fn validate(&self, strictness: Strictness) -> Result<(), String> {
        if self.editorial_version != EDITORIAL_VERSION {
            return Err(format!(
                "editorial_version {} is not {EDITORIAL_VERSION}",
                self.editorial_version
            ));
        }
        if self.sections.len() + self.skipped.len() > MAX_SECTIONS {
            return Err(format!("at most {MAX_SECTIONS} sections"));
        }
        if self.collections.len() > MAX_COLLECTIONS {
            return Err(format!("at most {MAX_COLLECTIONS} collections"));
        }
        let mut collection_ids = BTreeSet::new();
        for (i, collection) in self.collections.iter().enumerate() {
            let at = format!("collections[{i}]");
            check_slug(&format!("{at}.id"), &collection.id)?;
            if !collection_ids.insert(collection.id.as_str()) {
                return Err(format!("{at}.id `{}` appears twice", collection.id));
            }
            check_localized(
                &format!("{at}.title"),
                &collection.title,
                1,
                40,
                false,
                strictness,
            )?;
            if let Some(description) = &collection.description {
                check_localized(
                    &format!("{at}.description"),
                    description,
                    1,
                    300,
                    false,
                    strictness,
                )?;
            }
            check_plugin_list(&format!("{at}.plugins"), &collection.plugins)?;
        }
        let mut section_ids = BTreeSet::new();
        for (id, _) in &self.skipped {
            section_ids.insert(id.as_str());
        }
        let mut total = 0u64;
        for (i, section) in self.sections.iter().enumerate() {
            let at = format!("sections[{i}]");
            check_slug(&format!("{at}.id"), &section.id)?;
            if !section_ids.insert(section.id.as_str()) {
                return Err(format!("{at}.id `{}` appears twice", section.id));
            }
            if let Some(window) = &section.window
                && let (Some(start), Some(end)) = (window.starts_at, window.ends_at)
                && end <= start
            {
                return Err(format!("{at}.window ends before it starts"));
            }
            check_platforms(
                &format!("{at}.platforms"),
                section.platforms.as_ref(),
                strictness,
            )?;
            let optional =
                |field: &str, text: Option<&Localized>, max: usize| -> Result<(), String> {
                    text.map_or(Ok(()), |t| {
                        check_localized(&format!("{at}.{field}"), t, 1, max, false, strictness)
                    })
                };
            match &section.body {
                SectionBody::BannerCarousel { items } => {
                    if !(1..=6).contains(&items.len()) {
                        return Err(format!("{at}.items: 1 to 6 banners"));
                    }
                    let mut item_ids = BTreeSet::new();
                    for (j, item) in items.iter().enumerate() {
                        let at = format!("{at}.items[{j}]");
                        check_slug(&format!("{at}.id"), &item.id)?;
                        if !item_ids.insert(item.id.as_str()) {
                            return Err(format!("{at}.id `{}` appears twice", item.id));
                        }
                        check_localized(
                            &format!("{at}.title"),
                            &item.title,
                            1,
                            60,
                            false,
                            strictness,
                        )?;
                        if let Some(body) = &item.body {
                            check_localized(
                                &format!("{at}.body"),
                                body,
                                1,
                                160,
                                false,
                                strictness,
                            )?;
                        }
                        if let Some(cta) = &item.cta {
                            check_localized(&format!("{at}.cta"), cta, 1, 24, false, strictness)?;
                        }
                        if item.artwork.decorative {
                            if item.artwork.alt.values().any(|t| !t.is_empty()) {
                                return Err(format!("{at}.artwork is decorative but has alt text"));
                            }
                        } else {
                            check_localized(
                                &format!("{at}.artwork.alt"),
                                &item.artwork.alt,
                                1,
                                250,
                                false,
                                strictness,
                            )?;
                        }
                        item.artwork
                            .asset
                            .check(AssetKind::Banner)
                            .map_err(|e| format!("{at}.artwork: {e}"))?;
                        total += item.artwork.asset.length;
                        if strictness == Strictness::Publish {
                            if !TARGET_TYPES.contains(&item.target.kind.as_str()) {
                                return Err(format!(
                                    "{at}.target.type `{}` is not plugin, collection or category",
                                    item.target.kind
                                ));
                            }
                            match item.target.kind.as_str() {
                                "plugin" => {
                                    PluginId::parse(&item.target.id)
                                        .map_err(|e| format!("{at}.target.id: {e}"))?;
                                }
                                "collection"
                                    if !collection_ids.contains(item.target.id.as_str()) =>
                                {
                                    return Err(format!(
                                        "{at}.target names the unknown collection `{}`",
                                        item.target.id
                                    ));
                                }
                                "category" => {
                                    check_category(&format!("{at}.target.id"), &item.target.id)?;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                SectionBody::CollectionRow {
                    layout,
                    title,
                    description,
                    plugins,
                } => {
                    if strictness == Strictness::Publish
                        && !matches!(layout.as_str(), "cards" | "compact")
                    {
                        return Err(format!("{at}.layout `{layout}` is not cards or compact"));
                    }
                    check_localized(&format!("{at}.title"), title, 1, 40, false, strictness)?;
                    optional("description", description.as_ref(), 300)?;
                    check_plugin_list(&format!("{at}.plugins"), plugins)?;
                }
                SectionBody::Spotlight { headline, body, .. } => {
                    check_localized(
                        &format!("{at}.headline"),
                        headline,
                        1,
                        60,
                        false,
                        strictness,
                    )?;
                    optional("body", body.as_ref(), 300)?;
                }
                SectionBody::RecentlyAdded { limit, title } => {
                    if !(4..=24).contains(limit) {
                        return Err(format!("{at}.limit {limit} is not 4 to 24"));
                    }
                    optional("title", title.as_ref(), 40)?;
                }
                SectionBody::CategoryHighlights { title, categories } => {
                    if !(1..=12).contains(&categories.len()) {
                        return Err(format!("{at}.categories: 1 to 12"));
                    }
                    let mut seen = BTreeSet::new();
                    for (j, category) in categories.iter().enumerate() {
                        check_category(&format!("{at}.categories[{j}].id"), &category.id)?;
                        if !seen.insert(category.id.as_str()) {
                            return Err(format!("{at}.categories[{j}] appears twice"));
                        }
                        optional(
                            &format!("categories[{j}].blurb"),
                            category.blurb.as_ref(),
                            120,
                        )?;
                    }
                    optional("title", title.as_ref(), 40)?;
                }
                SectionBody::Catalog { title } => optional("title", title.as_ref(), 40)?,
            }
        }
        if total > MAX_EDITORIAL_MEDIA_BYTES {
            return Err(format!(
                "the editorial document references {total} media bytes; at most {MAX_EDITORIAL_MEDIA_BYTES}"
            ));
        }
        Ok(())
    }
}

/// `[a-z0-9-]{1,48}`.
fn check_slug(field: &str, id: &str) -> Result<(), String> {
    if (1..=48).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        Ok(())
    } else {
        Err(format!("{field} `{id}` is not 1 to 48 of a-z, 0-9 and -"))
    }
}

/// An index category: 2-24 of `a-z` and `-`.
fn check_category(field: &str, id: &str) -> Result<(), String> {
    if (2..=24).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b == b'-') {
        Ok(())
    } else {
        Err(format!("{field} `{id}` is not a category"))
    }
}

fn check_plugin_list(field: &str, plugins: &[PluginId]) -> Result<(), String> {
    let unique: BTreeSet<_> = plugins.iter().collect();
    if !(1..=24).contains(&plugins.len()) || unique.len() != plugins.len() {
        return Err(format!("{field}: 1 to 24 plugins, all different"));
    }
    Ok(())
}

// --------------------------------------------------------- verification

/// Why a storefront document was refused. A refused storefront is simply
/// absent (catalog-only); nothing else changes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StorefrontError {
    #[error("{document} is larger than {limit} bytes")]
    TooLarge { document: String, limit: usize },
    #[error("{document} is malformed: {message}")]
    Malformed { document: String, message: String },
    #[error("{document}: version {found} is not supported (this client reads {supported})")]
    UnsupportedVersion {
        document: String,
        found: u64,
        supported: u64,
    },
    #[error("no timestamp has been verified")]
    NoTimestamp,
    #[error("no verified index is available")]
    NoIndex,
    #[error("the signature is not a storefront signature (trusted comment {0:?})")]
    WrongRole(String),
    #[error("storefront signature: {0}")]
    Signature(SigError),
    #[error("the storefront is for seq {storefront}, the verified timestamp is seq {timestamp}")]
    Seq { storefront: u64, timestamp: u64 },
    #[error("the storefront names key set {found}, the current key set is {current}")]
    KeysVersion { found: u64, current: u64 },
    #[error("the storefront is bound to another index than the timestamp's: {0}")]
    IndexMismatch(String),
    #[error("the storefront lists {0}, which is not in the verified index")]
    UnknownPlugin(PluginId),
    #[error("two different storefronts were signed for seq {seq}")]
    Equivocation { seq: u64 },
    #[error("{document} does not match the storefront: {message}")]
    Mismatch { document: String, message: String },
    #[error("the storefront has no listing for {0}")]
    NoListing(PluginId),
    #[error("the storefront has no editorial document")]
    NoEditorial,
    #[error(transparent)]
    Store(#[from] StoreError),
}

fn malformed(document: &str, message: impl fmt::Display) -> StorefrontError {
    StorefrontError::Malformed {
        document: document.to_owned(),
        message: message.to_string(),
    }
}

fn too_large(document: &str, limit: usize) -> StorefrontError {
    StorefrontError::TooLarge {
        document: document.to_owned(),
        limit,
    }
}

/// A storefront whose signature, binding and content passed every check
/// for the client's current seq.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedStorefront {
    storefront: Storefront,
    sha256: Sha256Digest,
    /// Something the caller should log (warn): an unreadable stored
    /// record that was replaced.
    notice: Option<String>,
}

impl VerifiedStorefront {
    pub fn storefront(&self) -> &Storefront {
        &self.storefront
    }

    pub fn seq(&self) -> u64 {
        self.storefront.seq
    }

    /// The sha256 of the envelope bytes.
    pub fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }

    /// A warning for the caller's log, if verification had to replace an
    /// unreadable stored storefront record.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }
}

/// A version field read before the strict parse, so a future version is
/// reported as such rather than as unknown fields.
#[derive(Deserialize)]
struct VersionPeek {
    storefront_version: Option<u64>,
    listing_version: Option<u64>,
}

impl<S: TrustStore> RegistryClient<S> {
    /// Verifies `storefront/<seq>.json` and its signature (storefront
    /// design §5, steps 1 to 8):
    ///
    /// 1. at most [`MAX_STOREFRONT_BYTES`];
    /// 2. the trusted comment names the storefront role;
    /// 3. a threshold of the **current key set's index role** signed it
    ///    with exactly `role:storefront seq:<s> keys:<kv>` for the stored
    ///    timestamp's seq and the current key set;
    /// 4. it parses strictly and validates;
    /// 5. its seq is the stored timestamp's, its key set the current one;
    /// 6. its index reference is exactly the timestamp's;
    /// 7. listed ids are unique and in the verified index, and every
    ///    reference is content-addressed;
    /// 8. equivocation: a different storefront recorded for the same seq is
    ///    refused (and the error should be logged at error level). A
    ///    stored record that does not parse is treated as absent and
    ///    replaced ([`VerifiedStorefront::notice`]).
    ///
    /// Then the storefront is recorded (seq and sha256) beside the trust
    /// state. Nothing else in the state changes, so a refused storefront
    /// never affects refreshes or installs.
    pub fn verify_storefront(
        &mut self,
        bytes: &[u8],
        signature: &[u8],
    ) -> Result<VerifiedStorefront, StorefrontError> {
        let timestamp = self
            .state()
            .timestamp
            .clone()
            .ok_or(StorefrontError::NoTimestamp)?;
        let keyset = self
            .state()
            .keyset
            .clone()
            .ok_or(StorefrontError::NoTimestamp)?;
        let seq = timestamp.seq;
        let document = storefront_path(seq);
        // 1
        if bytes.len() > MAX_STOREFRONT_BYTES {
            return Err(too_large(&document, MAX_STOREFRONT_BYTES));
        }
        let signature_document = format!("{document}.minisig");
        if signature.len() > MAX_SIGNATURE_BYTES {
            return Err(too_large(&signature_document, MAX_SIGNATURE_BYTES));
        }
        let signature = std::str::from_utf8(signature)
            .map(|t| t.trim().to_owned())
            .map_err(|e| malformed(&signature_document, e))?;
        // 2
        let comment = sig::trusted_comment(&signature).map_err(StorefrontError::Signature)?;
        if !sig::is_storefront_comment(&comment) {
            return Err(StorefrontError::WrongRole(comment));
        }
        // 3
        let expected = storefront_comment(seq, keyset.version);
        sig::verify_role(&keyset.index, &[signature], bytes, &expected)
            .map_err(StorefrontError::Signature)?;
        // 4
        let peek: VersionPeek =
            serde_json::from_slice(bytes).map_err(|e| malformed(&document, e))?;
        if let Some(found) = peek.storefront_version
            && found != STOREFRONT_VERSION
        {
            return Err(StorefrontError::UnsupportedVersion {
                document,
                found,
                supported: STOREFRONT_VERSION,
            });
        }
        let storefront: Storefront =
            serde_json::from_slice(bytes).map_err(|e| malformed(&document, e))?;
        storefront.validate().map_err(|e| malformed(&document, e))?;
        // 5
        if storefront.seq != seq {
            return Err(StorefrontError::Seq {
                storefront: storefront.seq,
                timestamp: seq,
            });
        }
        if storefront.keys_version != keyset.version {
            return Err(StorefrontError::KeysVersion {
                found: storefront.keys_version,
                current: keyset.version,
            });
        }
        // 6
        if storefront.index != timestamp.index {
            return Err(StorefrontError::IndexMismatch(format!(
                "{} {} {}",
                storefront.index.path, storefront.index.sha256, storefront.index.length
            )));
        }
        // 7 (uniqueness and content addressing are in `validate`)
        let index = self.index().ok_or(StorefrontError::NoIndex)?;
        if index.seq != seq {
            return Err(StorefrontError::NoIndex);
        }
        if let Some(unknown) = storefront
            .listings
            .iter()
            .find(|l| index.plugin(&l.id).is_none())
        {
            return Err(StorefrontError::UnknownPlugin(unknown.id.clone()));
        }
        // 8
        let digest = sha256(bytes);
        let record = StorefrontRecord {
            seq,
            sha256: digest,
        };
        // A record that does not parse (corrupt, or written by a newer
        // build in another format) is treated as absent and replaced:
        // refusing on it would leave Discover catalog-only for good. A
        // record that parses is always checked.
        let (stored, notice) = match self.store().load_storefront_record() {
            Ok(stored) => (stored, None),
            Err(StoreError::Corrupt(why)) => (
                None,
                Some(format!(
                    "the stored storefront record could not be read ({why}); it is replaced"
                )),
            ),
            Err(e) => return Err(e.into()),
        };
        match stored {
            Some(stored) if stored.seq == seq && stored.sha256 != digest => {
                return Err(StorefrontError::Equivocation { seq });
            }
            Some(stored) if stored == record => {}
            _ => self.store_mut().save_storefront_record(&record)?,
        }
        Ok(VerifiedStorefront {
            storefront,
            sha256: digest,
            notice,
        })
    }

    /// Verifies a listing pinned by `storefront`: the storefront is still
    /// the current seq's, length and sha256 before parsing, a known
    /// `listing_version`, the id, and the §3/§4 rules.
    pub fn verify_listing(
        &self,
        storefront: &VerifiedStorefront,
        id: &PluginId,
        bytes: &[u8],
    ) -> Result<ListingDoc, StorefrontError> {
        self.check_current(storefront)?;
        let entry = storefront
            .storefront
            .listing(id)
            .ok_or_else(|| StorefrontError::NoListing(id.clone()))?;
        let document = entry.reference.path.as_str();
        if bytes.len() > MAX_LISTING_BYTES {
            return Err(too_large(document, MAX_LISTING_BYTES));
        }
        check_ref(bytes, &entry.reference).map_err(|message| StorefrontError::Mismatch {
            document: document.to_owned(),
            message,
        })?;
        let peek: VersionPeek =
            serde_json::from_slice(bytes).map_err(|e| malformed(document, e))?;
        if let Some(found) = peek.listing_version
            && found != LISTING_VERSION
        {
            return Err(StorefrontError::UnsupportedVersion {
                document: document.to_owned(),
                found,
                supported: LISTING_VERSION,
            });
        }
        let listing: ListingDoc =
            serde_json::from_slice(bytes).map_err(|e| malformed(document, e))?;
        if listing.id != *id {
            return Err(StorefrontError::Mismatch {
                document: document.to_owned(),
                message: format!("the listing is for {}", listing.id),
            });
        }
        listing
            .validate(Strictness::Client)
            .map_err(|e| malformed(document, e))?;
        Ok(listing)
    }

    /// Verifies the editorial document pinned by `storefront`: current
    /// seq, length and sha256 before parsing, a known `editorial_version`,
    /// the §3.2 rules. Sections of unknown kinds are skipped
    /// ([`EditorialDoc::skipped`]).
    pub fn verify_editorial(
        &self,
        storefront: &VerifiedStorefront,
        bytes: &[u8],
    ) -> Result<EditorialDoc, StorefrontError> {
        self.check_current(storefront)?;
        let reference = storefront
            .storefront
            .editorial
            .as_ref()
            .ok_or(StorefrontError::NoEditorial)?;
        let document = reference.path.as_str();
        if bytes.len() > MAX_EDITORIAL_BYTES {
            return Err(too_large(document, MAX_EDITORIAL_BYTES));
        }
        check_ref(bytes, reference).map_err(|message| StorefrontError::Mismatch {
            document: document.to_owned(),
            message,
        })?;
        let editorial = EditorialDoc::parse(bytes).map_err(|message| {
            if message.starts_with("editorial_version") {
                StorefrontError::UnsupportedVersion {
                    document: document.to_owned(),
                    found: serde_json::from_slice::<Value>(bytes)
                        .ok()
                        .and_then(|v| v["editorial_version"].as_u64())
                        .unwrap_or_default(),
                    supported: EDITORIAL_VERSION,
                }
            } else {
                malformed(document, message)
            }
        })?;
        editorial
            .validate(Strictness::Client)
            .map_err(|e| malformed(document, e))?;
        Ok(editorial)
    }

    /// A storefront verified earlier is usable only while its seq is the
    /// stored timestamp's (an older seq's storefront is never shown once a
    /// newer index is verified).
    fn check_current(&self, storefront: &VerifiedStorefront) -> Result<(), StorefrontError> {
        let timestamp = self
            .state()
            .timestamp
            .as_ref()
            .ok_or(StorefrontError::NoTimestamp)?;
        if storefront.seq() != timestamp.seq || storefront.storefront.index != timestamp.index {
            return Err(StorefrontError::Seq {
                storefront: storefront.seq(),
                timestamp: timestamp.seq,
            });
        }
        Ok(())
    }
}
