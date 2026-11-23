//! The registry storefront (storefront design §6): fetch, verify, cache
//! and serve the signed storefront of the verified index, its listings,
//! editorial document and images, and resolve them for the app.
//!
//! - **Fetch** after a refresh that verified a new index (or the same one
//!   with no storefront cached for it): `storefront/<seq>.json` and its
//!   signature through the registry's own [`RegistrySource`] (same origin,
//!   no redirect across origins), then every listing and the editorial
//!   document it pins. A 404 means none is published: catalog-only.
//! - **Verify** with the crate's client (`verify_storefront`,
//!   `verify_listing`, `verify_editorial`): index-role signature with the
//!   storefront comment, bound to the verified timestamp's index,
//!   equivocation, pinned documents by length then sha256 before parsing.
//!   A refused storefront is absent and its reason is reported; it never
//!   changes the refresh result, the registry state or an install.
//! - **Cache** under `<state>/storefront/` (`<seq>.json`, `.minisig`,
//!   `listings/<sha256>.json`, `editorial/<sha256>.json`), re-verified on
//!   every read against the stored state, so offline browsing shows the
//!   storefront of the stored seq.
//! - **Media** under `<state>/media/<sha256>`, named by a validated 64-hex
//!   digest only. Fetched lazily, only for digests the current verified
//!   storefront references; each fetch is capped at the pinned length and
//!   checked for length, sha256, the header walk and the pinned format and
//!   dimensions before it is written; re-hashed and re-checked on every
//!   read (a corrupt file is deleted and fetched again). At most
//!   [`MEDIA_BUDGET`] bytes: unreferenced files go first, then the least
//!   recently used. No pixel is decoded in the agent.
//! - **Resolve** for the app ([`view`], [`listing_view`]): the app's
//!   language (exact tag, then language, then `en`), sections inside their
//!   window (against the clock floor) and for this platform, placements of
//!   plugins that cannot be installed here dropped, sorted by `order` then
//!   `id`, at most [`MAX_STOREFRONT_ANSWER_BYTES`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Utc};
use deckpilot_ipc::plugins::{
    self as ipc, ICON_CHUNK_BYTES, MAX_STOREFRONT_ANSWER_BYTES, MediaChunk, is_media_sha256, reason,
};
use deckpilot_ipc::{ErrorCode, IpcError};
use deckpilot_plugin_manifest::{PluginId, Sha256Digest};
use deckpilot_plugin_registry::storefront::{
    AssetRef, EditorialDoc, ListingDoc, MAX_EDITORIAL_BYTES, MAX_LISTING_BYTES,
    MAX_STOREFRONT_BYTES, SectionBody, StorefrontError, VerifiedStorefront, localize, media_path,
    storefront_path, verify_media,
};
use deckpilot_plugin_registry::{
    FetchError, FileTrustStore, MAX_SIGNATURE_BYTES, Refresh, RegistryClient, RegistrySource,
    write_atomic,
};
use semver::Version;
use tracing::{debug, error, info, warn};

/// Most bytes of cached storefront images.
pub(super) const MEDIA_BUDGET: u64 = 64 * 1024 * 1024;
/// Most icons prefetched after a new storefront (placed plugins only);
/// the others are fetched when the app asks for them.
pub(super) const MAX_PREFETCH: usize = 8;
/// Longest a prefetch may start new fetches for. It runs after the answer
/// that triggered the refresh was sent, never on its path.
pub(super) const PREFETCH_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);
/// Longest one prefetch fetch may take, whole (the app's own requests use
/// the registry's document timeout).
pub(super) const PREFETCH_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

const STOREFRONT_DIR: &str = "storefront";
const MEDIA_DIR: &str = "media";

/// The verified storefront of the current seq and its verified documents.
#[derive(Debug, Clone)]
pub(super) struct Current {
    pub verified: VerifiedStorefront,
    pub listings: BTreeMap<PluginId, ListingDoc>,
    pub editorial: Option<EditorialDoc>,
    /// Every image the documents reference, by digest.
    pub assets: BTreeMap<Sha256Digest, AssetRef>,
}

impl Current {
    pub fn seq(&self) -> u64 {
        self.verified.seq()
    }
}

/// The storefront part of a registry's state.
#[derive(Debug)]
pub(super) struct Storefronts {
    dir: PathBuf,
    media: PathBuf,
    budget: u64,
    current: Option<Current>,
    /// Why there is no storefront for the current seq (a refusal or an
    /// unreachable registry with nothing cached); `None` when one is
    /// shown or none is published.
    problem: Option<String>,
    /// Icons of the current storefront to fetch ahead once the current
    /// request is answered; emptied whenever the current storefront
    /// changes or goes.
    prefetch: Vec<AssetRef>,
}

fn sig_name(path: &str) -> String {
    format!("{path}.minisig")
}

fn read(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

fn store(path: &Path, bytes: &[u8]) {
    if let Err(e) = write_atomic(path, bytes) {
        warn!(path = %path.display(), error = %e, "a storefront document could not be cached");
    }
}

/// What fetching one storefront document gave.
enum Fetched {
    Bytes(Vec<u8>),
    NotFound,
    Unavailable(String),
    Refused(String),
}

fn fetch(source: &dyn RegistrySource, path: &str, limit: usize) -> Fetched {
    match source.fetch(path, limit) {
        Ok(bytes) => Fetched::Bytes(bytes),
        Err(FetchError::NotFound) => Fetched::NotFound,
        Err(FetchError::Unavailable(why)) => Fetched::Unavailable(why),
        Err(FetchError::Refused(why)) => Fetched::Refused(why),
    }
}

fn asset_map(
    listings: &BTreeMap<PluginId, ListingDoc>,
    editorial: Option<&EditorialDoc>,
) -> BTreeMap<Sha256Digest, AssetRef> {
    let mut assets = BTreeMap::new();
    for listing in listings.values() {
        for (_, asset) in listing.assets() {
            assets.insert(asset.sha256, *asset);
        }
    }
    for asset in editorial.map(EditorialDoc::assets).unwrap_or_default() {
        assets.insert(asset.sha256, *asset);
    }
    assets
}

impl Storefronts {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            dir: state_dir.join(STOREFRONT_DIR),
            media: state_dir.join(MEDIA_DIR),
            budget: MEDIA_BUDGET,
            current: None,
            problem: None,
            prefetch: Vec::new(),
        }
    }

    #[cfg(test)]
    pub fn with_budget(mut self, budget: u64) -> Self {
        self.budget = budget;
        self
    }

    #[cfg(test)]
    pub fn current(&self) -> Option<&Current> {
        self.current.as_ref()
    }

    /// Replaces the current storefront; the prefetch queue, which belongs
    /// to the old one, is dropped.
    fn set_current(&mut self, current: Option<Current>) {
        self.prefetch.clear();
        self.current = current;
    }

    /// The current storefront only if it is for `seq` (the verified
    /// timestamp's): what is served.
    pub fn current_for(&self, seq: Option<u64>) -> Option<&Current> {
        self.current.as_ref().filter(|c| Some(c.seq()) == seq)
    }

    /// "editorial unavailable: <reason>" for Diagnostics.
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    fn cached(&self, path: &str) -> PathBuf {
        self.dir
            .join(path.strip_prefix("storefront/").unwrap_or(path))
    }

    /// The storefront of the stored seq, from the cache only (agent start,
    /// offline). Every document is verified again.
    pub fn load_cached(&mut self, client: &mut RegistryClient<FileTrustStore>) {
        let Some(seq) = client.state().timestamp.as_ref().map(|t| t.seq) else {
            self.set_current(None);
            return;
        };
        if self.current.as_ref().is_some_and(|c| c.seq() == seq) {
            return;
        }
        let path = storefront_path(seq);
        let (Some(bytes), Some(signature)) = (
            read(&self.cached(&path)),
            read(&self.cached(&sig_name(&path))),
        ) else {
            self.set_current(None);
            return;
        };
        match client.verify_storefront(&bytes, &signature) {
            Ok(verified) => {
                note(&verified);
                let current = self.documents(client, verified, None);
                self.set_current(Some(current));
                self.problem = None;
            }
            Err(e) => self.refuse(&e),
        }
    }

    fn refuse(&mut self, e: &StorefrontError) {
        if matches!(e, StorefrontError::Equivocation { .. }) {
            error!(error = %e, "the registry storefront was refused: equivocation");
        } else {
            warn!(error = %e, "the registry storefront was refused");
        }
        self.set_current(None);
        self.problem = Some(e.to_string());
    }

    /// After a refresh: the storefront of the verified seq. Never fails;
    /// a problem is kept for Diagnostics.
    pub fn sync(
        &mut self,
        client: &mut RegistryClient<FileTrustStore>,
        source: &dyn RegistrySource,
        refresh: &Refresh,
    ) {
        let seq = match refresh {
            Refresh::Updated { seq } => *seq,
            Refresh::Unchanged { seq }
                if self.current.as_ref().is_some_and(|c| c.seq() == *seq) =>
            {
                return;
            }
            Refresh::Unchanged { seq } => {
                self.load_cached(client);
                if self.current.as_ref().is_some_and(|c| c.seq() == *seq) {
                    return;
                }
                *seq
            }
            Refresh::Offline(_) => {
                self.load_cached(client);
                return;
            }
        };
        let path = storefront_path(seq);
        let envelope = fetch(source, &path, MAX_STOREFRONT_BYTES);
        let signature = match &envelope {
            Fetched::Bytes(_) => fetch(source, &sig_name(&path), MAX_SIGNATURE_BYTES),
            _ => Fetched::NotFound,
        };
        let (bytes, signature) = match (envelope, signature) {
            (Fetched::Bytes(bytes), Fetched::Bytes(signature)) => (bytes, signature),
            (Fetched::NotFound, _) => {
                debug!(
                    seq,
                    "no storefront is published for this index: catalog only"
                );
                self.set_current(None);
                self.problem = None;
                self.prune(None);
                return;
            }
            (Fetched::Unavailable(why), _) | (_, Fetched::Unavailable(why)) => {
                warn!(seq, %why, "the registry storefront could not be fetched");
                self.load_cached(client);
                if self.current.is_none() {
                    self.problem = Some(format!("the storefront could not be fetched: {why}"));
                }
                return;
            }
            (Fetched::Refused(why), _) | (_, Fetched::Refused(why)) => {
                warn!(seq, %why, "the registry storefront fetch was refused");
                self.set_current(None);
                self.problem = Some(format!("the storefront fetch was refused: {why}"));
                return;
            }
            (_, Fetched::NotFound) => {
                self.set_current(None);
                self.problem = Some("the storefront has no signature".into());
                return;
            }
        };
        let verified = match client.verify_storefront(&bytes, &signature) {
            Ok(verified) => {
                note(&verified);
                verified
            }
            Err(e) => {
                self.refuse(&e);
                return;
            }
        };
        store(&self.cached(&path), &bytes);
        store(&self.cached(&sig_name(&path)), &signature);
        let current = self.documents(client, verified, Some(source));
        info!(
            seq,
            listings = current.listings.len(),
            sections = current.editorial.as_ref().map_or(0, |e| e.sections.len()),
            "registry storefront verified"
        );
        self.problem = None;
        self.prune(Some(&current));
        let queue = placed_icons(&current);
        self.set_current(Some(current));
        self.prefetch = queue;
    }

    /// The listings and editorial document `verified` pins: from the cache
    /// or (with a source) fetched, each verified; a refused one is left out.
    fn documents(
        &self,
        client: &RegistryClient<FileTrustStore>,
        verified: VerifiedStorefront,
        source: Option<&dyn RegistrySource>,
    ) -> Current {
        let get = |path: &str, limit: usize| -> Option<(Vec<u8>, bool)> {
            if let Some(bytes) = read(&self.cached(path)) {
                return Some((bytes, false));
            }
            match source.map(|s| fetch(s, path, limit)) {
                Some(Fetched::Bytes(bytes)) => Some((bytes, true)),
                _ => None,
            }
        };
        let mut listings = BTreeMap::new();
        for entry in &verified.storefront().listings {
            let Some((bytes, fresh)) = get(&entry.reference.path, MAX_LISTING_BYTES) else {
                continue;
            };
            match client.verify_listing(&verified, &entry.id, &bytes) {
                Ok(listing) => {
                    if fresh {
                        store(&self.cached(&entry.reference.path), &bytes);
                    }
                    listings.insert(entry.id.clone(), listing);
                }
                Err(e) => {
                    warn!(plugin = %entry.id, error = %e, "a storefront listing was refused");
                    let _ = std::fs::remove_file(self.cached(&entry.reference.path));
                }
            }
        }
        let editorial = verified.storefront().editorial.as_ref().and_then(|reference| {
            let (bytes, fresh) = get(&reference.path, MAX_EDITORIAL_BYTES)?;
            match client.verify_editorial(&verified, &bytes) {
                Ok(editorial) => {
                    if fresh {
                        store(&self.cached(&reference.path), &bytes);
                    }
                    for (id, kind) in &editorial.skipped {
                        info!(section = %id, %kind, "a storefront section of an unknown kind is skipped");
                    }
                    Some(editorial)
                }
                Err(e) => {
                    warn!(error = %e, "the storefront editorial document was refused");
                    let _ = std::fs::remove_file(self.cached(&reference.path));
                    None
                }
            }
        });
        let assets = asset_map(&listings, editorial.as_ref());
        Current {
            verified,
            listings,
            editorial,
            assets,
        }
    }

    /// Removes cached documents `current` does not use (all of them when
    /// there is none).
    fn prune(&self, current: Option<&Current>) {
        let mut keep: BTreeSet<PathBuf> = BTreeSet::new();
        if let Some(current) = current {
            let path = storefront_path(current.seq());
            keep.insert(self.cached(&path));
            keep.insert(self.cached(&sig_name(&path)));
            for entry in &current.verified.storefront().listings {
                keep.insert(self.cached(&entry.reference.path));
            }
            if let Some(editorial) = &current.verified.storefront().editorial {
                keep.insert(self.cached(&editorial.path));
            }
        }
        let mut dirs = vec![self.dir.clone(), self.dir.join("editorial")];
        if let Ok(entries) = std::fs::read_dir(self.dir.join("listings")) {
            dirs.extend(entries.flatten().map(|e| e.path()));
        }
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && !keep.contains(&path) {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }

    /// Fetches queued icons ahead (best effort, one at a time): at most
    /// `max`, and none started once `budget` has passed. Whatever is left
    /// is fetched when the app asks for it. Called after an answer was
    /// sent, never while a refresh answers.
    /// Only for the storefront of `seq` (the verified timestamp's): a
    /// queue left over from another one fetches nothing. `source` bounds
    /// each fetch (the registry passes one with a short timeout).
    pub fn prefetch(
        &mut self,
        source: &dyn RegistrySource,
        seq: Option<u64>,
        max: usize,
        budget: std::time::Duration,
    ) {
        let queue = std::mem::take(&mut self.prefetch);
        let Some(current) = self.current_for(seq) else {
            return;
        };
        let started = std::time::Instant::now();
        for icon in queue.iter().take(max) {
            if started.elapsed() >= budget {
                debug!(
                    "the storefront icon prefetch ran out of time; the rest are fetched on demand"
                );
                break;
            }
            if let Err(e) = self.media_bytes(current, source, icon) {
                debug!(media = %icon.sha256, error = %e.message, "a storefront icon could not be prefetched");
            }
        }
    }

    /// Icons queued for [`Storefronts::prefetch`].
    #[cfg(test)]
    pub fn queued(&self) -> usize {
        self.prefetch.len()
    }

    fn media_file(&self, digest: &Sha256Digest) -> PathBuf {
        // A digest prints as 64 lowercase hex digits: never a path.
        self.media.join(digest.to_string())
    }

    /// The verified bytes of `asset`: from the cache (re-checked) or
    /// fetched, checked and cached.
    fn media_bytes(
        &self,
        current: &Current,
        source: &dyn RegistrySource,
        asset: &AssetRef,
    ) -> Result<Vec<u8>, IpcError> {
        let file = self.media_file(&asset.sha256);
        if let Some(bytes) = read(&file) {
            match verify_media(asset, &bytes) {
                Ok(_) => {
                    touch(&file);
                    return Ok(bytes);
                }
                Err(e) => {
                    warn!(media = %asset.sha256, error = %e, "a cached storefront image is corrupt; it is fetched again");
                    let _ = std::fs::remove_file(&file);
                }
            }
        }
        let path = media_path(asset);
        let limit = usize::try_from(asset.length).unwrap_or(usize::MAX);
        let bytes = source.fetch(&path, limit).map_err(|e| {
            IpcError::new(ErrorCode::Failed, format!("{path}: {e}")).with_reason(reason::REGISTRY)
        })?;
        verify_media(asset, &bytes).map_err(|e| {
            warn!(media = %asset.sha256, error = %e, "a storefront image was refused");
            IpcError::new(ErrorCode::Failed, format!("{path} {e}")).with_reason(reason::REGISTRY)
        })?;
        if let Err(e) = write_atomic(&file, &bytes) {
            warn!(media = %asset.sha256, error = %e, "a storefront image could not be cached");
        }
        self.evict(current, &asset.sha256);
        Ok(bytes)
    }

    /// Keeps the media cache within the budget: unreferenced files first,
    /// then the least recently used; never `keep`.
    fn evict(&self, current: &Current, keep: &Sha256Digest) {
        let Ok(entries) = std::fs::read_dir(&self.media) else {
            return;
        };
        let mut files: Vec<(bool, SystemTime, u64, PathBuf)> = Vec::new();
        let mut total = 0u64;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() || !is_media_sha256(&name) {
                // Not ours (a temporary file of a crashed write, …).
                let _ = std::fs::remove_file(&path);
                continue;
            }
            if name == keep.to_string() {
                total += meta.len();
                continue;
            }
            let referenced =
                Sha256Digest::parse(&name).is_ok_and(|d| current.assets.contains_key(&d));
            let used = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            total += meta.len();
            files.push((referenced, used, meta.len(), path));
        }
        if total <= self.budget {
            return;
        }
        files.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, _, size, path) in files {
            if total <= self.budget {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(size);
            }
        }
    }

    /// One chunk of the storefront image `sha256` (`getRegistryMedia`).
    /// `seq` is the verified timestamp's: a storefront of another seq
    /// serves nothing.
    pub fn media_chunk(
        &self,
        source: &dyn RegistrySource,
        seq: Option<u64>,
        sha256: &str,
        offset: u64,
    ) -> Result<MediaChunk, IpcError> {
        let invalid = |m: &str| IpcError::new(ErrorCode::InvalidRequest, m.to_owned());
        if !is_media_sha256(sha256) {
            return Err(invalid("sha256 must be 64 lowercase hex digits"));
        }
        let unknown = || {
            IpcError::new(
                ErrorCode::InvalidRequest,
                format!("{sha256} is not an image of the verified storefront"),
            )
            .with_reason(reason::UNKNOWN_MEDIA)
        };
        let digest = Sha256Digest::parse(sha256).map_err(|_| unknown())?;
        let current = self.current_for(seq).ok_or_else(unknown)?;
        let asset = current.assets.get(&digest).ok_or_else(unknown)?;
        let bytes = self.media_bytes(current, source, asset)?;
        let size = bytes.len() as u64;
        if offset >= size {
            return Err(invalid("offset is beyond the image"));
        }
        let start = usize::try_from(offset).map_err(|_| invalid("offset"))?;
        let end = (start + ICON_CHUNK_BYTES).min(bytes.len());
        Ok(MediaChunk {
            media_type: asset.format.media_type().to_owned(),
            size,
            sha256: digest.to_string(),
            offset,
            data: STANDARD.encode(&bytes[start..end]),
            next: (end < bytes.len()).then_some(end as u64),
        })
    }
}

fn note(verified: &VerifiedStorefront) {
    if let Some(notice) = verified.notice() {
        warn!(%notice, "registry storefront record");
    }
}

/// The icons of plugins the editorial places (rows, spotlights,
/// collections), each once, in placement order.
fn placed_icons(current: &Current) -> Vec<AssetRef> {
    let Some(editorial) = &current.editorial else {
        return Vec::new();
    };
    let mut placed: Vec<&PluginId> = Vec::new();
    for section in &editorial.sections {
        match &section.body {
            SectionBody::CollectionRow { plugins, .. } => placed.extend(plugins),
            SectionBody::Spotlight { plugin, .. } => placed.push(plugin),
            _ => {}
        }
    }
    for collection in &editorial.collections {
        placed.extend(&collection.plugins);
    }
    let mut seen = BTreeSet::new();
    placed
        .into_iter()
        .filter_map(|id| current.listings.get(id).and_then(|l| l.icon))
        .filter(|icon| seen.insert(icon.sha256))
        .collect()
}

fn touch(path: &Path) {
    if let Ok(file) = std::fs::File::options().append(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

// ------------------------------------------------------------- resolving

/// What the resolver knows of one plugin the app could install here: in
/// the verified index, a published, non-withdrawn version compatible with
/// this host.
#[derive(Debug, Clone, Default)]
pub(super) struct Placeable {
    pub categories: BTreeSet<String>,
    /// Published, non-withdrawn versions.
    pub versions: BTreeSet<Version>,
}

pub(super) type Placeables = BTreeMap<String, Placeable>;

fn media_ref(asset: &AssetRef) -> ipc::MediaRef {
    ipc::MediaRef {
        sha256: asset.sha256.to_string(),
        media_type: asset.format.media_type().to_owned(),
        width: asset.width,
        height: asset.height,
    }
}

fn text(map: &deckpilot_plugin_registry::storefront::Localized, locale: &str) -> String {
    localize(map, locale).unwrap_or_default().to_owned()
}

fn opt_text(
    map: Option<&deckpilot_plugin_registry::storefront::Localized>,
    locale: &str,
) -> Option<String> {
    map.and_then(|m| localize(m, locale)).map(str::to_owned)
}

fn placeable_ids<'a>(
    ids: impl IntoIterator<Item = &'a PluginId>,
    placeable: &Placeables,
) -> Vec<String> {
    let mut seen = BTreeSet::new();
    ids.into_iter()
        .filter(|id| placeable.contains_key(id.as_str()))
        .filter(|id| seen.insert(id.as_str().to_owned()))
        .map(ToString::to_string)
        .collect()
}

fn platform_ok(platforms: Option<&Vec<String>>, platform: &str) -> bool {
    platforms.is_none_or(|p| p.iter().any(|x| x == platform))
}

/// The storefront `current` for this app: see the module documentation.
/// `now` is the effective time (the registry's clock floor or later).
#[allow(clippy::too_many_lines)] // one arm per section kind
pub(super) fn view(
    seq: u64,
    editorial: Option<&EditorialDoc>,
    placeable: &Placeables,
    locale: &str,
    platform: &str,
    now: DateTime<Utc>,
) -> ipc::StorefrontView {
    let Some(editorial) = editorial else {
        return ipc::StorefrontView {
            seq,
            sections: Vec::new(),
            collections: Vec::new(),
        };
    };
    let mut collections: Vec<(i64, &str, ipc::CollectionView)> = editorial
        .collections
        .iter()
        .filter_map(|c| {
            let plugins = placeable_ids(&c.plugins, placeable);
            (!plugins.is_empty()).then(|| {
                (
                    c.order,
                    c.id.as_str(),
                    ipc::CollectionView {
                        id: c.id.clone(),
                        title: text(&c.title, locale),
                        description: opt_text(c.description.as_ref(), locale),
                        plugins,
                    },
                )
            })
        })
        .collect();
    collections.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(b.1)));
    let collection_ids: BTreeSet<&str> = collections.iter().map(|c| c.1).collect();
    let categories: BTreeSet<&str> = placeable
        .values()
        .flat_map(|p| p.categories.iter().map(String::as_str))
        .collect();
    let mut sections: Vec<(i64, &str, ipc::SectionView)> = Vec::new();
    for section in &editorial.sections {
        if !section.window.as_ref().is_none_or(|w| w.contains(now))
            || !platform_ok(section.platforms.as_ref(), platform)
        {
            continue;
        }
        let mut out = ipc::SectionView {
            id: section.id.clone(),
            kind: String::new(),
            layout: None,
            title: None,
            description: None,
            items: Vec::new(),
            plugins: Vec::new(),
            categories: Vec::new(),
            limit: None,
            version: None,
        };
        match &section.body {
            SectionBody::BannerCarousel { items } => {
                out.kind = "bannerCarousel".into();
                out.items = items
                    .iter()
                    .filter(|item| match item.target.kind.as_str() {
                        "plugin" => placeable.contains_key(item.target.id.as_str()),
                        "collection" => collection_ids.contains(item.target.id.as_str()),
                        "category" => categories.contains(item.target.id.as_str()),
                        _ => false,
                    })
                    .map(|item| ipc::BannerView {
                        id: item.id.clone(),
                        title: text(&item.title, locale),
                        body: opt_text(item.body.as_ref(), locale),
                        cta: opt_text(item.cta.as_ref(), locale),
                        artwork: media_ref(&item.artwork.asset),
                        alt: if item.artwork.decorative {
                            String::new()
                        } else {
                            text(&item.artwork.alt, locale)
                        },
                        decorative: item.artwork.decorative,
                        target: ipc::TargetView {
                            kind: item.target.kind.clone(),
                            id: item.target.id.clone(),
                        },
                    })
                    .collect();
                if out.items.is_empty() {
                    continue;
                }
            }
            SectionBody::CollectionRow {
                layout,
                title,
                description,
                plugins,
            } => {
                out.kind = "collectionRow".into();
                out.layout = Some(
                    if layout == "compact" {
                        "compact"
                    } else {
                        "cards"
                    }
                    .into(),
                );
                out.title = Some(text(title, locale));
                out.description = opt_text(description.as_ref(), locale);
                out.plugins = placeable_ids(plugins, placeable);
                if out.plugins.is_empty() {
                    continue;
                }
            }
            SectionBody::Spotlight {
                plugin,
                version,
                headline,
                body,
            } => {
                let Some(info) = placeable.get(plugin.as_str()) else {
                    continue;
                };
                out.kind = "spotlight".into();
                out.plugins = vec![plugin.to_string()];
                out.title = Some(text(headline, locale));
                out.description = opt_text(body.as_ref(), locale);
                out.version = version
                    .as_ref()
                    .filter(|v| info.versions.contains(v))
                    .map(ToString::to_string);
            }
            SectionBody::RecentlyAdded { limit, title } => {
                out.kind = "recentlyAdded".into();
                out.limit = Some(*limit);
                out.title = opt_text(title.as_ref(), locale);
            }
            SectionBody::CategoryHighlights {
                title,
                categories: list,
            } => {
                out.kind = "categoryHighlights".into();
                out.title = opt_text(title.as_ref(), locale);
                out.categories = list
                    .iter()
                    .filter(|c| categories.contains(c.id.as_str()))
                    .map(|c| ipc::CategoryView {
                        id: c.id.clone(),
                        blurb: opt_text(c.blurb.as_ref(), locale),
                    })
                    .collect();
                if out.categories.is_empty() {
                    continue;
                }
            }
            SectionBody::Catalog { title } => {
                out.kind = "catalog".into();
                out.title = opt_text(title.as_ref(), locale);
            }
        }
        sections.push((section.order, section.id.as_str(), out));
    }
    sections.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(b.1)));
    let mut view = ipc::StorefrontView {
        seq,
        sections: sections.into_iter().map(|s| s.2).collect(),
        collections: collections.into_iter().map(|c| c.2).collect(),
    };
    while encoded_len(&view) > MAX_STOREFRONT_ANSWER_BYTES {
        if let Some(dropped) = view.sections.pop() {
            warn!(section = %dropped.id, "the storefront answer is too large; a trailing section is dropped");
        } else if let Some(dropped) = view.collections.pop() {
            warn!(collection = %dropped.id, "the storefront answer is too large; a collection is dropped");
        } else {
            break;
        }
    }
    view
}

fn encoded_len<T: serde::Serialize>(value: &T) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |v| v.len())
}

/// A plugin's listing for this app: localized, screenshots for
/// `platform`, release notes of `published` versions newest first, at most
/// [`MAX_STOREFRONT_ANSWER_BYTES`] (the oldest notes go first).
pub(super) fn listing_view(
    listing: &ListingDoc,
    published: &BTreeSet<Version>,
    locale: &str,
    platform: &str,
) -> ipc::ListingView {
    let mut notes: Vec<(Version, String)> = listing
        .release_notes
        .iter()
        .filter_map(|(version, text_map)| {
            let version = Version::parse(version).ok()?;
            published
                .contains(&version)
                .then(|| (version, text(text_map, locale)))
        })
        .collect();
    notes.sort_by(|a, b| b.0.cmp(&a.0));
    let links = listing.links.clone().unwrap_or_default();
    let mut view = ipc::ListingView {
        promo: opt_text(listing.promo.as_ref(), locale),
        description_md: opt_text(listing.description_md.as_ref(), locale),
        highlights: listing
            .highlights
            .iter()
            .map(|h| ipc::HighlightView {
                title: text(&h.title, locale),
                text: text(&h.text, locale),
            })
            .collect(),
        icon: listing.icon.as_ref().map(media_ref),
        screenshots: listing
            .screenshots
            .iter()
            .filter(|s| platform_ok(s.platforms.as_ref(), platform))
            .map(|s| ipc::ScreenshotView {
                media: media_ref(&s.asset),
                alt: text(&s.alt, locale),
                caption: opt_text(s.caption.as_ref(), locale),
            })
            .collect(),
        promo_art: listing.promo_art.as_ref().map(|art| ipc::ArtView {
            media: media_ref(&art.asset),
            alt: text(&art.alt, locale),
        }),
        links: ipc::ListingLinks {
            documentation: links.documentation,
            support: links.support,
            changelog: links.changelog,
        },
        release_notes: notes
            .into_iter()
            .map(|(version, text)| ipc::ReleaseNoteView {
                version: version.to_string(),
                text,
            })
            .collect(),
    };
    while encoded_len(&view) > MAX_STOREFRONT_ANSWER_BYTES && view.release_notes.pop().is_some() {}
    if encoded_len(&view) > MAX_STOREFRONT_ANSWER_BYTES {
        warn!("a storefront listing is too large to answer; its description is left out");
        view.description_md = None;
    }
    view
}

#[cfg(test)]
mod tests;
