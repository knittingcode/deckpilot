//! `storefront check|build|sign` and the storefront part of `verify-site`
//! (storefront design §3, §4, §5 and §9).
//!
//! * `check` validates the sources: `registry/plugins/<id>/listing.json`
//!   (authors) and `editorial/editorial.json` (maintainers) against the
//!   bundled input schemas and every rule of §3 and §4, the ids and
//!   versions they name against `registry/`, and every image: the header
//!   walk the client does ([`crate::media::inspect`]) **and** a full decode
//!   with resource limits (the client never decodes).
//! * `build` writes the content-addressed listing and editorial documents,
//!   copies the images to `media/<sha256>.<ext>`, and writes the unsigned
//!   envelope `storefront/<seq>.json`, bound to the signed
//!   `index/<seq>.json`.
//! * `sign` signs the envelope with an **index** key and the exact comment
//!   `deckpilot-registry v1 role:storefront seq:<seq> keys:<kv>`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use deckpilot_plugin_manifest::{FIRST_PARTY_PUBLISHER, PluginId, Sha256Digest};
use deckpilot_plugin_package::sha256;
use serde_json::{Map, Value};

use super::catalog;
use super::keys::SigningKey;
use super::keyset;
use super::layout::{
    SiteSource, Workspace, read, read_optional, remove, sig_path, write, write_immutable,
};
use super::{PublishError, Result, fail, to_json};
use crate::RegistrySource as _;
use crate::media::{self, MediaFormat, RasterInfo};
use crate::model::{FileRef, Index, SPEC_VERSION};
use crate::sig::{index_comment, storefront_comment, verify_role};
use crate::storefront::{
    AssetKind, AssetRef, EditorialDoc, ListingDoc, ListingEntry, MAX_EDITORIAL_BYTES,
    MAX_LISTING_BYTES, MAX_STOREFRONT_BYTES, STOREFRONT_VERSION, SectionBody, Storefront,
    StorefrontType, Strictness, VerifiedStorefront, editorial_path, listing_path, media_path,
    storefront_path, verify_media,
};
use crate::{RegistryClient, TrustStore};

/// Most bytes a full decode may allocate (the largest RGBA image allowed is
/// 32 MiB).
const DECODE_ALLOC_LIMIT: u64 = 64 * 1024 * 1024;

/// Words a listing outside the first-party namespace should not use about
/// itself; flagged for review, not refused (§5 "Authors cannot place").
const REVIEW_WORDS: [&str; 3] = ["official", "verified", "deckpilot team"];

/// Decodes an image in full, within limits, and checks the decoded size
/// equals the header's (CI only; never the client).
pub(crate) fn decode_fully(bytes: &[u8], info: &RasterInfo) -> std::result::Result<(), String> {
    let format = match info.format {
        MediaFormat::Png => image::ImageFormat::Png,
        MediaFormat::Webp => image::ImageFormat::WebP,
        MediaFormat::Jpeg => image::ImageFormat::Jpeg,
    };
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(info.width);
    limits.max_image_height = Some(info.height);
    limits.max_alloc = Some(DECODE_ALLOC_LIMIT);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|e| format!("does not decode: {e}"))?;
    if (decoded.width(), decoded.height()) != (info.width, info.height) {
        return Err(format!(
            "decodes to {}×{}, its header says {}×{}",
            decoded.width(),
            decoded.height(),
            info.width,
            info.height
        ));
    }
    Ok(())
}

/// Header walk plus full decode: everything `check` requires of an image.
fn check_image(bytes: &[u8]) -> std::result::Result<RasterInfo, String> {
    let info = media::inspect(bytes).map_err(|e| e.to_string())?;
    decode_fully(bytes, &info)?;
    Ok(info)
}

// ---------------------------------------------------------------- schemas

fn validator(schema: &str) -> Result<jsonschema::Validator> {
    let schema: Value =
        serde_json::from_str(schema).map_err(|e| PublishError(format!("schema: {e}")))?;
    jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .map_err(|e| PublishError(format!("schema: {e}")))
}

fn schema_problems(
    validator: &jsonschema::Validator,
    value: &Value,
    file: &str,
    out: &mut Vec<String>,
) {
    for error in validator.iter_errors(value) {
        out.push(format!("{file}{}: {error}", error.instance_path()));
    }
}

// ----------------------------------------------------------------- media

/// An image read from a source directory, checked.
#[derive(Debug, Clone)]
struct Image {
    asset: AssetRef,
    bytes: Vec<u8>,
}

/// `media/<name>.<png|webp|jpg>`, a regular file (not a link) inside
/// `base/media/`.
fn source_image(base: &Path, file: &str) -> std::result::Result<Image, String> {
    let name = file
        .strip_prefix("media/")
        .ok_or_else(|| format!("`{file}` is not under media/"))?;
    let (stem, ext) = name
        .rsplit_once('.')
        .ok_or_else(|| format!("`{file}` has no extension"))?;
    let declared = MediaFormat::from_extension(ext)
        .ok_or_else(|| format!("`{file}`: only .png, .webp and .jpg (no SVG)"))?;
    if !(1..=48).contains(&stem.len())
        || !stem
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(format!(
            "`{file}`: the name must be 1 to 48 of a-z, 0-9 and -"
        ));
    }
    let media_dir = base.join("media");
    match std::fs::symlink_metadata(&media_dir) {
        Ok(meta) if meta.file_type().is_dir() => {}
        Ok(_) => {
            return Err(format!(
                "`{file}`: media/ is not a plain directory (a link?)"
            ));
        }
        Err(e) => return Err(format!("`{file}`: media/: {e}")),
    }
    let path = media_dir.join(name);
    let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("`{file}`: {e}"))?;
    if !meta.file_type().is_file() {
        return Err(format!("`{file}` is not a regular file"));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("`{file}`: {e}"))?;
    let info = check_image(&bytes).map_err(|e| format!("`{file}` {e}"))?;
    if info.format != declared {
        return Err(format!(
            "`{file}` is a {} file, not what its extension says",
            info.format
        ));
    }
    Ok(Image {
        asset: AssetRef {
            sha256: sha256(&bytes),
            length: bytes.len() as u64,
            format: info.format,
            width: info.width,
            height: info.height,
        },
        bytes,
    })
}

/// A source file must be a regular file, not a link (absent is fine).
fn plain_file(path: &Path) -> std::result::Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => Ok(()),
        Ok(_) => Err("is not a regular file (a link?)".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Replaces `{"file": …}` at `value[key]` (an object) with the asset
/// reference: the whole object for an icon, its `asset` member otherwise.
fn resolve_file(
    object: &mut Map<String, Value>,
    base: &Path,
    whole: bool,
    at: &str,
    images: &mut Vec<Image>,
    problems: &mut Vec<String>,
) {
    let Some(Value::String(file)) = object.remove("file") else {
        problems.push(format!("{at}: `file` is missing"));
        return;
    };
    match source_image(base, &file) {
        Ok(image) => {
            let asset = serde_json::to_value(image.asset).unwrap_or(Value::Null);
            if whole {
                if let Value::Object(asset) = asset {
                    object.extend(asset);
                }
            } else {
                object.insert("asset".into(), asset);
            }
            images.push(image);
        }
        Err(problem) => problems.push(format!("{at}: {problem}")),
    }
}

// --------------------------------------------------------------- sources

/// A source turned into its signed form.
struct Built<T> {
    doc: T,
    images: Vec<Image>,
}

/// Reads, checks and resolves `registry/plugins/<id>/listing.json`.
#[allow(clippy::too_many_lines)] // one rule after the other
fn listing_source(
    ws: &Workspace,
    id: &PluginId,
    known: &BTreeMap<PluginId, BTreeSet<String>>,
    problems: &mut Vec<String>,
    notes: &mut Vec<String>,
) -> Result<Option<Built<ListingDoc>>> {
    let path = ws.listing_source_path(id);
    let file = format!("registry/plugins/{id}/listing.json");
    if let Err(problem) = plain_file(&path) {
        problems.push(format!("{file}: {problem}"));
        return Ok(None);
    }
    let Some(bytes) = read_optional(&path)? else {
        return Ok(None);
    };
    let before = problems.len();
    let mut value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(e) => {
            problems.push(format!("{file}: {e}"));
            return Ok(None);
        }
    };
    schema_problems(
        &validator(crate::schemas::LISTING_SOURCE)?,
        &value,
        &file,
        problems,
    );
    if problems.len() > before {
        return Ok(None);
    }
    let Some(object) = value.as_object_mut() else {
        return Ok(None);
    };
    object.remove("$schema");
    if object.get("id").and_then(Value::as_str) != Some(id.as_str()) {
        problems.push(format!("{file}: `id` is not the directory name {id}"));
        return Ok(None);
    }
    let base = ws.plugins_dir().join(id.as_str());
    let mut images = Vec::new();
    if let Some(Value::Object(icon)) = object.get_mut("icon") {
        resolve_file(
            icon,
            &base,
            true,
            &format!("{file} icon"),
            &mut images,
            problems,
        );
    }
    if let Some(Value::Array(shots)) = object.get_mut("screenshots") {
        for (i, shot) in shots.iter_mut().enumerate() {
            if let Value::Object(shot) = shot {
                resolve_file(
                    shot,
                    &base,
                    false,
                    &format!("{file} screenshots[{i}]"),
                    &mut images,
                    problems,
                );
            }
        }
    }
    if let Some(Value::Object(art)) = object.get_mut("promo_art") {
        resolve_file(
            art,
            &base,
            false,
            &format!("{file} promo_art"),
            &mut images,
            problems,
        );
    }
    if problems.len() > before {
        return Ok(None);
    }
    let mut signed = Map::new();
    signed.insert("_type".into(), Value::from("deckpilot-registry-listing"));
    signed.extend(std::mem::take(object));
    let doc: ListingDoc = match serde_json::from_value(Value::Object(signed)) {
        Ok(doc) => doc,
        Err(e) => {
            problems.push(format!("{file}: {e}"));
            return Ok(None);
        }
    };
    if let Err(e) = doc.validate(Strictness::Publish) {
        problems.push(format!("{file}: {e}"));
        return Ok(None);
    }
    let versions = known.get(id);
    if versions.is_none() {
        problems.push(format!(
            "{file}: {id} has no registry/plugins/{id}/plugin.json"
        ));
    }
    for version in doc.release_notes.keys() {
        if versions.is_some_and(|v| !v.contains(version)) {
            problems.push(format!(
                "{file}: release_notes names {version}, which is not recorded in versions/"
            ));
        }
    }
    if id.publisher() != FIRST_PARTY_PUBLISHER {
        let text = serde_json::to_string(&doc)
            .unwrap_or_default()
            .to_lowercase();
        for word in REVIEW_WORDS {
            if text.contains(word) {
                notes.push(format!(
                    "{file}: review: the listing says \"{word}\" (only DeckPilot's own plugins are official)"
                ));
            }
        }
    }
    let listing_bytes = to_json(&doc)?;
    if listing_bytes.len() > MAX_LISTING_BYTES {
        problems.push(format!("{file}: the signed listing exceeds 64 KiB"));
    }
    Ok(Some(Built { doc, images }))
}

/// Reads, checks and resolves `editorial/editorial.json`: published
/// entries only, without `status`.
#[allow(clippy::too_many_lines)] // one rule after the other
fn editorial_source(
    ws: &Workspace,
    known: &BTreeMap<PluginId, BTreeSet<String>>,
    problems: &mut Vec<String>,
) -> Result<Option<Built<EditorialDoc>>> {
    let file = "editorial/editorial.json";
    if let Err(problem) = plain_file(&ws.editorial_source_path()) {
        problems.push(format!("{file}: {problem}"));
        return Ok(None);
    }
    let Some(bytes) = read_optional(&ws.editorial_source_path())? else {
        return Ok(None);
    };
    let before = problems.len();
    let mut value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(e) => {
            problems.push(format!("{file}: {e}"));
            return Ok(None);
        }
    };
    schema_problems(
        &validator(crate::schemas::EDITORIAL_SOURCE)?,
        &value,
        file,
        problems,
    );
    if problems.len() > before {
        return Ok(None);
    }
    let Some(object) = value.as_object_mut() else {
        return Ok(None);
    };
    object.remove("$schema");
    let published = |list: Option<Value>| -> Vec<Value> {
        let Some(Value::Array(list)) = list else {
            return Vec::new();
        };
        list.into_iter()
            .filter(|entry| entry.get("status").and_then(Value::as_str) == Some("published"))
            .map(|mut entry| {
                if let Some(entry) = entry.as_object_mut() {
                    entry.remove("status");
                }
                entry
            })
            .collect()
    };
    let mut sections = published(object.remove("sections"));
    let collections = published(object.remove("collections"));
    let base = ws.editorial_dir();
    let mut images = Vec::new();
    for section in &mut sections {
        let id = section
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if let Some(Value::Array(items)) = section.get_mut("items") {
            for (j, item) in items.iter_mut().enumerate() {
                if let Some(Value::Object(art)) = item.get_mut("artwork") {
                    resolve_file(
                        art,
                        &base,
                        false,
                        &format!("{file} {id}.items[{j}].artwork"),
                        &mut images,
                        problems,
                    );
                }
            }
        }
    }
    if problems.len() > before {
        return Ok(None);
    }
    let mut signed = Map::new();
    signed.insert("_type".into(), Value::from("deckpilot-registry-editorial"));
    signed.insert(
        "editorial_version".into(),
        object.remove("editorial_version").unwrap_or(Value::Null),
    );
    signed.insert("sections".into(), Value::Array(sections));
    if !collections.is_empty() {
        signed.insert("collections".into(), Value::Array(collections));
    }
    let bytes =
        serde_json::to_vec(&Value::Object(signed)).map_err(|e| PublishError(e.to_string()))?;
    let doc = match EditorialDoc::parse(&bytes) {
        Ok(doc) => doc,
        Err(e) => {
            problems.push(format!("{file}: {e}"));
            return Ok(None);
        }
    };
    if let Err(e) = doc.validate(Strictness::Publish) {
        problems.push(format!("{file}: {e}"));
        return Ok(None);
    }
    // Every placed plugin is in registry/, a spotlight version recorded.
    let mut placed: Vec<(String, PluginId)> = Vec::new();
    for section in &doc.sections {
        let at = section.id.clone();
        match &section.body {
            SectionBody::CollectionRow { plugins, .. } => {
                placed.extend(plugins.iter().map(|id| (at.clone(), id.clone())));
            }
            SectionBody::Spotlight {
                plugin, version, ..
            } => {
                placed.push((at.clone(), plugin.clone()));
                if let Some(version) = version
                    && known
                        .get(plugin)
                        .is_some_and(|v| !v.contains(&version.to_string()))
                {
                    problems.push(format!("{file} {at}: {plugin} {version} is not recorded"));
                }
            }
            SectionBody::BannerCarousel { items } => {
                for item in items {
                    if item.target.kind == "plugin"
                        && let Ok(id) = PluginId::parse(&item.target.id)
                    {
                        placed.push((at.clone(), id));
                    }
                }
            }
            _ => {}
        }
    }
    for collection in &doc.collections {
        placed.extend(
            collection
                .plugins
                .iter()
                .map(|id| (collection.id.clone(), id.clone())),
        );
    }
    for (at, id) in placed {
        if !known.contains_key(&id) {
            problems.push(format!("{file} {at}: {id} is not in registry/plugins/"));
        }
    }
    let signed_bytes = to_json(&doc)?;
    if signed_bytes.len() > MAX_EDITORIAL_BYTES {
        problems.push(format!(
            "{file}: the signed editorial document exceeds 128 KiB"
        ));
    }
    Ok(Some(Built { doc, images }))
}

/// Ids with a `listing.json`.
fn listing_ids(ws: &Workspace) -> Result<Vec<PluginId>> {
    let mut ids = Vec::new();
    let dir = ws.plugins_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ids),
        Err(e) => return fail(format!("{}: {e}", dir.display())),
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if let Ok(id) = PluginId::parse(&name)
            && ws.listing_source_path(&id).is_file()
        {
            ids.push(id);
        }
    }
    ids.sort();
    Ok(ids)
}

/// Everything the sources produce, or every problem found.
struct Sources {
    listings: Vec<Built<ListingDoc>>,
    editorial: Option<Built<EditorialDoc>>,
    notes: Vec<String>,
}

fn sources(ws: &Workspace) -> Result<Sources> {
    let known: BTreeMap<PluginId, BTreeSet<String>> = catalog::plugins(ws)?
        .into_iter()
        .map(|(id, p)| {
            (
                id,
                p.versions.iter().map(|v| v.version.to_string()).collect(),
            )
        })
        .collect();
    let mut problems = Vec::new();
    let mut notes = Vec::new();
    let mut listings = Vec::new();
    for id in listing_ids(ws)? {
        if let Some(built) = listing_source(ws, &id, &known, &mut problems, &mut notes)? {
            listings.push(built);
        }
    }
    let editorial = editorial_source(ws, &known, &mut problems)?;
    if !problems.is_empty() {
        return fail(format!(
            "the storefront sources have {} problem(s):\n  {}",
            problems.len(),
            problems.join("\n  ")
        ));
    }
    Ok(Sources {
        listings,
        editorial,
        notes,
    })
}

/// What `storefront check` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub listings: usize,
    pub sections: usize,
    pub collections: usize,
    pub images: usize,
    pub media_bytes: u64,
    /// Not errors: text flagged for review.
    pub notes: Vec<String>,
}

/// `storefront check`: every source rule; fails listing every problem.
pub fn check(ws: &Workspace) -> Result<CheckReport> {
    let sources = sources(ws)?;
    let mut digests = BTreeSet::new();
    let mut media_bytes = 0;
    for image in sources
        .listings
        .iter()
        .flat_map(|l| &l.images)
        .chain(sources.editorial.iter().flat_map(|e| &e.images))
    {
        if digests.insert(image.asset.sha256) {
            media_bytes += image.asset.length;
        }
    }
    Ok(CheckReport {
        listings: sources.listings.len(),
        sections: sources
            .editorial
            .as_ref()
            .map_or(0, |e| e.doc.sections.len()),
        collections: sources
            .editorial
            .as_ref()
            .map_or(0, |e| e.doc.collections.len()),
        images: digests.len(),
        media_bytes,
        notes: sources.notes,
    })
}

// ----------------------------------------------------------------- build

/// What `storefront build` wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildReport {
    pub seq: u64,
    pub listings: usize,
    /// Listings of plugins the index does not list (nothing installable).
    pub skipped: Vec<PluginId>,
    pub editorial: bool,
    pub media: usize,
    pub notes: Vec<String>,
}

fn signed_index_seq(ws: &Workspace, seq: Option<u64>) -> Result<u64> {
    match seq {
        Some(seq) => Ok(seq),
        None => ws.index_seqs()?.last().copied().ok_or_else(|| {
            PublishError("no index; run `build-index` and `sign-index` first".into())
        }),
    }
}

/// `storefront build`: the signed documents' content for `index/<seq>.json`
/// (default: the newest), which must be signed already.
#[allow(clippy::too_many_lines)] // index check, documents, envelope
pub fn build(ws: &Workspace, seq: Option<u64>, now: DateTime<Utc>) -> Result<BuildReport> {
    let keyset = keyset::current(ws)?;
    let seq = signed_index_seq(ws, seq)?;
    let index_path = ws.index_path(seq);
    let index_bytes = read(&index_path)?;
    let index_signature = read_optional(&sig_path(&index_path))?.ok_or_else(|| {
        PublishError(format!(
            "index/{seq}.json is not signed; run `sign-index` first"
        ))
    })?;
    let index_signature = String::from_utf8(index_signature)
        .map_err(|_| PublishError(format!("index/{seq}.json.minisig is not text")))?;
    verify_role(
        &keyset.index,
        &[index_signature.trim().to_owned()],
        &index_bytes,
        &index_comment(seq, keyset.version),
    )
    .map_err(|e| {
        PublishError(format!(
            "index/{seq}.json is not signed for key set {}: {e}",
            keyset.version
        ))
    })?;
    let index: Index = super::layout::parse(&index_path, &index_bytes)?;
    let storefront_file = ws.storefront_path(seq);
    if storefront_file.exists() {
        return fail(format!("{} already exists", storefront_file.display()));
    }
    let sources = sources(ws)?;
    let site = ws.site();
    let mut written_media = BTreeSet::new();
    let mut write_images = |images: &[Image]| -> Result<()> {
        for image in images {
            if written_media.insert(image.asset.sha256) {
                write_immutable(&site.join(media_path(&image.asset)), &image.bytes)?;
            }
        }
        Ok(())
    };
    let mut listings = Vec::new();
    let mut skipped = Vec::new();
    for built in &sources.listings {
        if index.plugin(&built.doc.id).is_none() {
            skipped.push(built.doc.id.clone());
            continue;
        }
        write_images(&built.images)?;
        let bytes = to_json(&built.doc)?;
        let digest = sha256(&bytes);
        let path = listing_path(&built.doc.id, &digest);
        write_immutable(&site.join(&path), &bytes)?;
        listings.push(ListingEntry {
            id: built.doc.id.clone(),
            reference: FileRef {
                path,
                sha256: digest,
                length: bytes.len() as u64,
            },
        });
    }
    let editorial = match &sources.editorial {
        Some(built) => {
            write_images(&built.images)?;
            let bytes = to_json(&built.doc)?;
            let digest = sha256(&bytes);
            let path = editorial_path(&digest);
            write_immutable(&site.join(&path), &bytes)?;
            Some(FileRef {
                path,
                sha256: digest,
                length: bytes.len() as u64,
            })
        }
        None => None,
    };
    let storefront = Storefront {
        kind: StorefrontType::Value,
        spec_version: SPEC_VERSION,
        storefront_version: STOREFRONT_VERSION,
        seq,
        keys_version: keyset.version,
        generated_at: now,
        source_commit: index.source_commit.clone(),
        index: FileRef {
            path: format!("index/{seq}.json"),
            sha256: sha256(&index_bytes),
            length: index_bytes.len() as u64,
        },
        listings,
        editorial,
    };
    storefront
        .validate()
        .map_err(|e| PublishError(format!("storefront/{seq}.json: {e}")))?;
    let bytes = to_json(&storefront)?;
    if bytes.len() > MAX_STOREFRONT_BYTES {
        return fail("the storefront envelope exceeds 64 KiB");
    }
    write(&storefront_file, &bytes)?;
    prune(ws)?;
    Ok(BuildReport {
        seq,
        listings: storefront.listings.len(),
        skipped,
        editorial: storefront.editorial.is_some(),
        media: written_media.len(),
        notes: sources.notes,
    })
}

/// Keeps the storefronts of the indexes still on the site and every file
/// they reference; removes the rest.
fn prune(ws: &Workspace) -> Result<()> {
    let site = ws.site();
    let indexes: BTreeSet<u64> = ws.index_seqs()?.into_iter().collect();
    let mut referenced: BTreeSet<PathBuf> = BTreeSet::new();
    for seq in ws.storefront_seqs()? {
        let path = ws.storefront_path(seq);
        if !indexes.contains(&seq) {
            remove(&sig_path(&path))?;
            remove(&path)?;
            continue;
        }
        let storefront: Storefront = super::layout::read_json(&path)?;
        for entry in &storefront.listings {
            referenced.insert(site.join(&entry.reference.path));
            let listing: ListingDoc = super::layout::read_json(&site.join(&entry.reference.path))?;
            for (_, asset) in listing.assets() {
                referenced.insert(site.join(media_path(asset)));
            }
        }
        if let Some(editorial) = &storefront.editorial {
            referenced.insert(site.join(&editorial.path));
            let doc =
                EditorialDoc::parse(&read(&site.join(&editorial.path))?).map_err(PublishError)?;
            for asset in doc.assets() {
                referenced.insert(site.join(media_path(asset)));
            }
        }
    }
    let mut dirs = vec![
        site.join("storefront").join("editorial"),
        site.join("media"),
    ];
    if let Ok(entries) = std::fs::read_dir(site.join("storefront").join("listings")) {
        dirs.extend(entries.flatten().map(|e| e.path()));
    }
    for dir in dirs {
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        for file in files.flatten() {
            if file.path().is_file() && !referenced.contains(&file.path()) {
                remove(&file.path())?;
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ sign

/// `storefront sign`: signs `storefront/<seq>.json` (default: the newest)
/// with an index key and the storefront comment, after checking that it is
/// bound to the signed `index/<seq>.json` of the current key set.
pub fn sign(ws: &Workspace, seq: Option<u64>, key: &SigningKey) -> Result<u64> {
    let keyset = keyset::current(ws)?;
    let seq = match seq {
        Some(seq) => seq,
        None => *ws
            .storefront_seqs()?
            .last()
            .ok_or_else(|| PublishError("no storefront; run `storefront build` first".into()))?,
    };
    let path = ws.storefront_path(seq);
    let bytes = read(&path)?;
    let storefront: Storefront = super::layout::parse(&path, &bytes)?;
    storefront
        .validate()
        .map_err(|e| PublishError(format!("{}: {e}", path.display())))?;
    if storefront.seq != seq {
        return fail(format!("{} holds seq {}", path.display(), storefront.seq));
    }
    if storefront.keys_version != keyset.version {
        return fail(format!(
            "storefront/{seq}.json names key set {}, the current key set is {}; build it again",
            storefront.keys_version, keyset.version
        ));
    }
    let index_path = ws.index_path(seq);
    let index_bytes = read(&index_path)?;
    let index_ref = FileRef {
        path: format!("index/{seq}.json"),
        sha256: sha256(&index_bytes),
        length: index_bytes.len() as u64,
    };
    if storefront.index != index_ref {
        return fail(format!(
            "storefront/{seq}.json is not bound to the current index/{seq}.json"
        ));
    }
    let index_signature = read_optional(&sig_path(&index_path))?
        .ok_or_else(|| PublishError(format!("index/{seq}.json is not signed")))?;
    verify_role(
        &keyset.index,
        &[String::from_utf8_lossy(&index_signature).trim().to_owned()],
        &index_bytes,
        &index_comment(seq, keyset.version),
    )
    .map_err(|e| {
        PublishError(format!(
            "index/{seq}.json is not signed for this key set: {e}"
        ))
    })?;
    if !keyset.index.identities().contains(&key.identity()) {
        return fail(format!(
            "this key is not an index key of keys/{}.json (the storefront is signed by the index role)",
            keyset.version
        ));
    }
    let signature_path = sig_path(&path);
    if signature_path.exists() {
        return fail(format!("{} already exists", signature_path.display()));
    }
    // What the signature will vouch for is checked again on disk: every
    // pinned document by length, sha256 and its typed rules, and every
    // image it pins (header and full decode).
    check_pinned(ws, &storefront)?;
    let comment = storefront_comment(seq, keyset.version);
    let signature = key.sign(&bytes, &comment)?;
    verify_role(
        &keyset.index,
        std::slice::from_ref(&signature),
        &bytes,
        &comment,
    )
    .map_err(|e| PublishError(format!("self-check failed: {e}")))?;
    write(&signature_path, signature.as_bytes())?;
    Ok(seq)
}

/// The documents and images `storefront` pins, as they are on disk.
fn check_pinned(ws: &Workspace, storefront: &Storefront) -> Result<()> {
    let site = ws.site();
    let pinned = |reference: &FileRef, limit: usize| -> Result<Vec<u8>> {
        let bytes = read(&site.join(&reference.path))?;
        if bytes.len() > limit {
            return fail(format!("{} is larger than {limit} bytes", reference.path));
        }
        crate::client::check_ref(&bytes, reference).map_err(|e| {
            PublishError(format!(
                "{} does not match the storefront: {e}",
                reference.path
            ))
        })?;
        Ok(bytes)
    };
    let mut assets = Vec::new();
    for entry in &storefront.listings {
        let bytes = pinned(&entry.reference, MAX_LISTING_BYTES)?;
        let listing: ListingDoc = super::layout::parse(&site.join(&entry.reference.path), &bytes)?;
        if listing.id != entry.id {
            return fail(format!(
                "{} is the listing of {}",
                entry.reference.path, listing.id
            ));
        }
        listing
            .validate(Strictness::Publish)
            .map_err(|e| PublishError(format!("{}: {e}", entry.reference.path)))?;
        assets.extend(listing.assets().into_iter().map(|(_, a)| *a));
    }
    if let Some(reference) = &storefront.editorial {
        let bytes = pinned(reference, MAX_EDITORIAL_BYTES)?;
        let doc = EditorialDoc::parse(&bytes)
            .map_err(|e| PublishError(format!("{}: {e}", reference.path)))?;
        doc.validate(Strictness::Publish)
            .map_err(|e| PublishError(format!("{}: {e}", reference.path)))?;
        assets.extend(doc.assets().into_iter().copied());
    }
    let mut seen = BTreeSet::new();
    for asset in assets {
        if !seen.insert(asset.sha256) {
            continue;
        }
        let path = media_path(&asset);
        let bytes = read(&site.join(&path))?;
        let info = verify_media(&asset, &bytes).map_err(|e| PublishError(format!("{path} {e}")))?;
        decode_fully(&bytes, &info).map_err(|e| PublishError(format!("{path} {e}")))?;
    }
    Ok(())
}

// ------------------------------------------------------------- verify-site

/// What `verify-site --require-storefront` checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorefrontReport {
    pub seq: u64,
    pub sha256: Sha256Digest,
    pub listings: usize,
    pub sections: usize,
    pub skipped_sections: usize,
    pub media: usize,
}

/// The storefront of `client`'s current seq, as a client verifies it, plus
/// every pinned image (header walk as the client does, then a full decode).
pub(crate) fn verify_site_storefront<S: TrustStore>(
    client: &mut RegistryClient<S>,
    source: &SiteSource,
) -> Result<StorefrontReport> {
    let seq = client
        .state()
        .timestamp
        .as_ref()
        .map(|t| t.seq)
        .ok_or_else(|| PublishError("no timestamp stored".into()))?;
    let path = storefront_path(seq);
    let fetch = |path: &str, limit: usize| {
        source
            .fetch(path, limit)
            .map_err(|e| PublishError(format!("{path}: {e} (required by --require-storefront)")))
    };
    let bytes = fetch(&path, MAX_STOREFRONT_BYTES)?;
    let signature = fetch(&format!("{path}.minisig"), crate::MAX_SIGNATURE_BYTES)?;
    let verified: VerifiedStorefront = client
        .verify_storefront(&bytes, &signature)
        .map_err(|e| PublishError(format!("the client refuses {path}: {e}")))?;
    let mut assets: Vec<(Option<AssetKind>, AssetRef)> = Vec::new();
    for entry in &verified.storefront().listings {
        let bytes = fetch(&entry.reference.path, MAX_LISTING_BYTES)?;
        let listing = client
            .verify_listing(&verified, &entry.id, &bytes)
            .map_err(|e| {
                PublishError(format!("the client refuses {}: {e}", entry.reference.path))
            })?;
        assets.extend(listing.assets().into_iter().map(|(k, a)| (Some(k), *a)));
    }
    let (sections, skipped) = match &verified.storefront().editorial {
        Some(reference) => {
            let bytes = fetch(&reference.path, MAX_EDITORIAL_BYTES)?;
            let doc = client
                .verify_editorial(&verified, &bytes)
                .map_err(|e| PublishError(format!("the client refuses {}: {e}", reference.path)))?;
            assets.extend(
                doc.assets()
                    .into_iter()
                    .map(|a| (Some(AssetKind::Banner), *a)),
            );
            (doc.sections.len(), doc.skipped.len())
        }
        None => (0, 0),
    };
    let mut seen = BTreeSet::new();
    for (_, asset) in &assets {
        if !seen.insert(asset.sha256) {
            continue;
        }
        let path = media_path(asset);
        let bytes = fetch(&path, media::MAX_MEDIA_BYTES)?;
        let info = verify_media(asset, &bytes).map_err(|e| PublishError(format!("{path} {e}")))?;
        decode_fully(&bytes, &info).map_err(|e| PublishError(format!("{path} {e}")))?;
    }
    Ok(StorefrontReport {
        seq,
        sha256: *verified.sha256(),
        listings: verified.storefront().listings.len(),
        sections,
        skipped_sections: skipped,
        media: seen.len(),
    })
}
