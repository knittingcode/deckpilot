//! The agent's storefront (storefront design §6 and §10, R2 and R3) over
//! HTTPS: a registry with a storefront made with the publishing tool and
//! throwaway keys, served by the local TLS server whose throwaway CA only
//! these tests trust.
#![allow(
    clippy::trivially_copy_pass_by_ref,
    clippy::needless_pass_by_value,
    clippy::manual_assert
)]

use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeDelta, Utc};
use deckpilot_plugin_package::{PackageBuilder, sha256};
use deckpilot_plugin_registry::publish::{Io, run};
use deckpilot_plugin_registry::storefront::{EditorialDoc, ListingDoc, Storefront};
use deckpilot_plugin_registry::{FileTrustStore, Policy, Refresh, RegistryClient, Role};
use serde_json::{Value, json};

use super::*;
use crate::plugins::fetch::{HttpsConfig, HttpsSource, TlsRoots};

// The same test server as `fetch/tests.rs`.
#[allow(clippy::duplicate_mod)]
#[path = "../../../tests/support/https.rs"]
mod https;

use https::{HttpsServer, Route};

const T0: &str = "2026-11-01T00:00:00Z";
const EXAMPLE: &str = include_str!(
    "../../../../../crates/deckpilot-plugin-registry/tests/fixtures/example-manifest.json"
);

fn at(hours: i64) -> DateTime<Utc> {
    T0.parse::<DateTime<Utc>>().unwrap() + TimeDelta::hours(hours)
}

fn rfc(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "deckpilot-storefront-{name}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ------------------------------------------------------------------ images

fn crc(parts: &[&[u8]]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for part in parts {
        for &byte in *part {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
    }
    !crc
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc(&[kind, data]).to_be_bytes());
}

/// A real, decodable 8-bit grayscale PNG (zlib with stored blocks).
fn png(width: u32, height: u32, shade: u8) -> Vec<u8> {
    let mut raw = Vec::new();
    for y in 0..height {
        raw.push(0);
        raw.extend((0..width).map(|x| shade ^ ((x ^ y) & 0x0F) as u8));
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(i + 1 == blocks.len()));
        let len = u16::try_from(block.len()).unwrap();
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", b"");
    out
}

// -------------------------------------------------------------------- site

/// A registry with plugins `acme.clock` and `acme.beta`, a listing for
/// `acme.clock` and an editorial document, published at hour 0.
struct Site {
    scratch: Scratch,
    /// Hour 0 of the site's publications.
    base: DateTime<Utc>,
}

fn package(plugin: &str, version: &str) -> Vec<u8> {
    let mut manifest: Value = serde_json::from_str(EXAMPLE).unwrap();
    manifest["id"] = json!(plugin);
    manifest["version"] = json!(version);
    manifest["publisher"]["id"] = json!(plugin.split('.').next().unwrap());
    let mut builder = PackageBuilder::new();
    builder
        .add(
            "manifest.json",
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap()
        .add(
            "plugin.wasm",
            deckpilot_plugin_package::wasm::minimal_abi_module_with_note(&format!(
                "{plugin} {version}"
            )),
        )
        .unwrap()
        .add("LICENSE", "MIT License\n")
        .unwrap()
        .add("README.md", "# Test plugin\n")
        .unwrap()
        .add("assets/icon-128.png", vec![1; 200])
        .unwrap()
        .add("assets/icon-32.png", vec![2; 50])
        .unwrap()
        .add("assets/keys/pr.png", vec![3; 50])
        .unwrap();
    builder.build().unwrap().bytes
}

impl Site {
    fn work(&self) -> PathBuf {
        self.scratch.0.join("work")
    }

    fn site(&self) -> PathBuf {
        self.work().join("site/v1")
    }

    fn key(&self, name: &str) -> String {
        self.scratch.0.join("keys").join(name).display().to_string()
    }

    fn run(&self, args: &[&str]) -> String {
        let mut all: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        if args.first().is_none_or(|c| *c != "keys") {
            all.push("--dir".into());
            all.push(self.work().display().to_string());
        }
        let env = |_: &str| None;
        let mut input: &[u8] = b"";
        let mut out = Vec::new();
        let mut io = Io {
            env: &env,
            stdin: &mut input,
            out: &mut out,
        };
        run(&all, &mut io).unwrap_or_else(|e| panic!("deckpilot-registry {args:?}: {e}"));
        String::from_utf8(out).unwrap()
    }

    fn write(&self, path: &str, bytes: impl AsRef<[u8]>) {
        let path = self.work().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn add(&self, plugin: &str, version: &str) {
        let file = self.scratch.0.join(format!("{plugin}-{version}.dpplugin"));
        std::fs::write(&file, package(plugin, version)).unwrap();
        let detail = self.scratch.0.join(format!("{plugin}.json"));
        std::fs::write(
            &detail,
            serde_json::to_vec(&json!({
                "id": plugin, "name": format!("Plugin {plugin}"), "summary": "A test plugin",
                "description": "A test plugin.", "license": "MIT", "categories": ["developer-tools"]
            }))
            .unwrap(),
        )
        .unwrap();
        self.run(&[
            "add-version",
            "--package",
            &file.display().to_string(),
            "--plugin-detail",
            &detail.display().to_string(),
            "--source-repository",
            "someone/test-plugin",
            "--tag",
            &format!("v{version}"),
            "--commit",
            &"1".repeat(40),
            "--builder",
            "1.0.0",
            "--now",
            &rfc(self.at(-24)),
        ]);
    }

    fn at(&self, hours: i64) -> DateTime<Utc> {
        self.base + TimeDelta::hours(hours)
    }

    fn new(name: &str) -> Self {
        Self::new_at(name, at(0))
    }

    fn new_at(name: &str, base: DateTime<Utc>) -> Self {
        let site = Self {
            scratch: Scratch::new(name),
            base,
        };
        std::fs::create_dir_all(site.scratch.0.join("keys")).unwrap();
        for role in ["root", "index", "timestamp"] {
            site.run(&[
                "keys",
                "generate",
                "--role",
                role,
                "--out",
                &site.key(&format!("{role}.key")),
                "--no-password",
            ]);
        }
        site.run(&[
            "keyset",
            "init",
            "--root",
            &site.key("root.key.pub"),
            "--index",
            &site.key("index.key.pub"),
            "--timestamp",
            &site.key("timestamp.key.pub"),
        ]);
        site.run(&["keyset", "sign", "--key", &site.key("root.key")]);
        site.run(&[
            "publisher",
            "--namespace",
            "acme",
            "--github-id",
            "1001",
            "--login",
            "acme",
            "--verified",
        ]);
        site.add("acme.clock", "1.0.0");
        site.add("acme.clock", "1.1.0");
        site.add("acme.beta", "0.1.0");
        site.write(
            "registry/plugins/acme.clock/listing.json",
            serde_json::to_vec_pretty(&json!({
                "listing_version": 1, "id": "acme.clock",
                "promo": { "en": "A clock on a key", "pl": "Zegar na klawiszu" },
                "icon": { "file": "media/icon.png" },
                "release_notes": { "1.1.0": { "en": "Faster." }, "1.0.0": { "en": "First." } }
            }))
            .unwrap(),
        );
        site.write(
            "registry/plugins/acme.clock/media/icon.png",
            png(256, 256, 0x40),
        );
        site.write(
            "editorial/editorial.json",
            serde_json::to_vec_pretty(&json!({
                "editorial_version": 1,
                "sections": [
                    { "id": "launch", "status": "published", "kind": "banner_carousel", "order": 10,
                      "items": [ { "id": "clock", "title": { "en": "Time on a key", "pl": "Czas na klawiszu" },
                                   "artwork": { "file": "media/banner.png", "alt": { "en": "A clock banner" } },
                                   "target": { "type": "plugin", "id": "acme.clock" } } ] },
                    { "id": "row", "status": "published", "kind": "collection_row", "order": 20,
                      "title": { "en": "Tools" }, "plugins": ["acme.beta", "acme.clock"] },
                    { "id": "spot", "status": "published", "kind": "spotlight", "order": 5,
                      "plugin": "acme.clock", "version": "1.1.0", "headline": { "en": "New: Clock" } },
                    { "id": "draft", "status": "draft", "kind": "catalog", "order": 1 }
                ]
            }))
            .unwrap(),
        );
        site.write("editorial/media/banner.png", png(1200, 400, 0x80));
        site.publish(0);
        site
    }

    fn publish(&self, hour: i64) {
        let now = rfc(self.at(hour));
        self.run(&[
            "build-index",
            "--source-commit",
            &"a".repeat(40),
            "--now",
            &now,
        ]);
        self.run(&["sign-index", "--key", &self.key("index.key")]);
        self.run(&["storefront", "build", "--now", &now]);
        self.run(&["storefront", "sign", "--key", &self.key("index.key")]);
        self.run(&[
            "timestamp",
            "--key",
            &self.key("timestamp.key"),
            "--now",
            &now,
        ]);
    }

    fn roots(&self) -> Role {
        serde_json::from_slice(&std::fs::read(self.work().join("roots.json")).unwrap()).unwrap()
    }

    fn seq(&self) -> u64 {
        let timestamp: Value =
            serde_json::from_slice(&std::fs::read(self.site().join("timestamp.json")).unwrap())
                .unwrap();
        timestamp["seq"].as_u64().unwrap()
    }

    fn storefront(&self) -> Storefront {
        serde_json::from_slice(
            &std::fs::read(self.site().join(format!("storefront/{}.json", self.seq()))).unwrap(),
        )
        .unwrap()
    }

    fn listing(&self) -> ListingDoc {
        let storefront = self.storefront();
        serde_json::from_slice(
            &std::fs::read(self.site().join(&storefront.listings[0].reference.path)).unwrap(),
        )
        .unwrap()
    }

    fn icon(&self) -> AssetRef {
        self.listing().icon.unwrap()
    }

    fn banner(&self) -> AssetRef {
        let storefront = self.storefront();
        let doc = EditorialDoc::parse(
            &std::fs::read(self.site().join(&storefront.editorial.unwrap().path)).unwrap(),
        )
        .unwrap();
        *doc.assets()[0]
    }
}

/// A client with its own state directory, and the storefront beside it.
struct Agent {
    state: PathBuf,
    client: RegistryClient<FileTrustStore>,
    storefronts: Storefronts,
}

impl Agent {
    fn new(site: &Site) -> Self {
        let state = site.scratch.0.join("agent-state");
        Self::open(site, state)
    }

    fn open(site: &Site, state: PathBuf) -> Self {
        let mut client =
            RegistryClient::open(site.roots(), FileTrustStore::new(&state), Policy::default())
                .unwrap();
        let mut storefronts = Storefronts::new(&state);
        storefronts.load_cached(&mut client);
        Self {
            state,
            client,
            storefronts,
        }
    }

    /// A refresh, then (as the worker does once the answer is sent) the
    /// bounded icon prefetch.
    fn refresh(&mut self, source: &HttpsSource, hour: i64) -> Refresh {
        let refresh = self.client.refresh(source, at(hour)).unwrap();
        self.storefronts.sync(&mut self.client, source, &refresh);
        self.storefronts
            .prefetch(source, MAX_PREFETCH, PREFETCH_BUDGET);
        refresh
    }

    fn media(&self, source: &HttpsSource, sha: &str) -> Result<Vec<u8>, IpcError> {
        let mut out = Vec::new();
        let mut offset = 0;
        loop {
            let chunk = self
                .storefronts
                .media_chunk(source, self.seq(), sha, offset)?;
            assert_eq!(chunk.sha256, sha);
            assert_eq!(chunk.offset, offset);
            out.extend(STANDARD.decode(&chunk.data).unwrap());
            match chunk.next {
                Some(next) => offset = next,
                None => return Ok(out),
            }
        }
    }

    fn seq(&self) -> Option<u64> {
        self.client.state().timestamp.as_ref().map(|t| t.seq)
    }

    fn cached_media(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.state.join("media"))
            .map(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

fn source(server: &HttpsServer) -> HttpsSource {
    source_trusting(server, vec![server.ca_pem.clone()])
}

fn source_trusting(server: &HttpsServer, roots: Vec<String>) -> HttpsSource {
    let config = HttpsConfig::new(
        &server.url("site/"),
        Some(&server.url("releases/")),
        TlsRoots::Only(roots),
    )
    .unwrap();
    HttpsSource::new(config).unwrap()
}

fn media_route(server: &HttpsServer, asset: &AssetRef) -> String {
    let _ = server;
    format!("/site/v1/{}", media_path(asset))
}

fn fetches(server: &HttpsServer, path: &str) -> usize {
    server
        .log()
        .iter()
        .filter(|line| line.contains(path))
        .count()
}

// ================================================================== tests

#[test]
fn the_storefront_is_fetched_verified_cached_and_served() {
    let site = Site::new("served");
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    let mut agent = Agent::new(&site);
    assert_eq!(agent.refresh(&source, 1), Refresh::Updated { seq: 1 });
    let current = agent.storefronts.current().expect("a storefront");
    assert_eq!(current.seq(), 1);
    assert_eq!(current.listings.len(), 1);
    assert_eq!(current.editorial.as_ref().unwrap().sections.len(), 3);
    assert_eq!(agent.storefronts.problem(), None);
    // The icon was prefetched? It is placed (row, spotlight).
    let icon = site.icon();
    assert_eq!(agent.cached_media(), vec![icon.sha256.to_string()]);
    // Served in chunks that add up to the verified bytes.
    let bytes = agent.media(&source, &icon.sha256.to_string()).unwrap();
    assert_eq!(sha256(&bytes), icon.sha256);
    let banner = site.banner();
    let bytes = agent.media(&source, &banner.sha256.to_string()).unwrap();
    assert!(bytes.len() > ICON_CHUNK_BYTES, "a multi-chunk image");
    assert_eq!(sha256(&bytes), banner.sha256);
    // Cached documents are what was verified.
    assert!(agent.state.join("storefront/1.json").is_file());
    assert!(agent.state.join("storefront/1.json.minisig").is_file());
    // An unchanged refresh fetches nothing again.
    let before = fetches(&server, "storefront/");
    assert_eq!(agent.refresh(&source, 2), Refresh::Unchanged { seq: 1 });
    assert_eq!(fetches(&server, "storefront/"), before);
}

#[test]
fn media_must_be_exactly_what_the_storefront_pins() {
    let site = Site::new("pins");
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    let mut agent = Agent::new(&site);
    agent.refresh(&source, 1);
    let banner = site.banner();
    let sha = banner.sha256.to_string();
    let route = media_route(&server, &banner);
    let original = std::fs::read(site.site().join(media_path(&banner))).unwrap();
    // Same length, other bytes: hash mismatch.
    let mut other = original.clone();
    let last = other.len() - 1;
    other[last] ^= 1;
    for (name, body) in [
        ("hash", other),
        ("short", original[..original.len() - 1].to_vec()),
        ("long", [original.clone(), vec![0]].concat()),
    ] {
        server.route(&route, Route::Body(body));
        let error = agent.media(&source, &sha).expect_err(name);
        assert_eq!(error.reason.as_deref(), Some(reason::REGISTRY), "{name}");
        assert!(
            agent.cached_media().iter().all(|n| *n != sha),
            "{name}: nothing cached"
        );
    }
    // Larger than pinned, with or without a content length: refused while
    // streaming.
    for declared in [true, false] {
        server.route(
            &route,
            Route::Oversized {
                len: original.len() + 4096,
                declared,
            },
        );
        assert!(agent.media(&source, &sha).is_err(), "{declared}");
    }
    // Cut off midway.
    server.route(&route, Route::CutOff { sent: 100 });
    assert!(agent.media(&source, &sha).is_err());
    server.clear_routes();
    assert_eq!(sha256(&agent.media(&source, &sha).unwrap()), banner.sha256);
    assert!(agent.cached_media().contains(&sha));
}

#[test]
fn only_referenced_well_formed_digests_are_served() {
    let site = Site::new("digests");
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    let mut agent = Agent::new(&site);
    agent.refresh(&source, 1);
    let icon = site.icon().sha256.to_string();
    let unknown = "0".repeat(64);
    let error = agent
        .storefronts
        .media_chunk(&source, agent.seq(), &unknown, 0)
        .unwrap_err();
    assert_eq!(error.reason.as_deref(), Some(reason::UNKNOWN_MEDIA));
    for bad in [
        format!("../{}", &icon[3..]),
        icon.to_uppercase(),
        icon[..63].to_owned(),
        format!("{icon}0"),
        format!("{}/", &icon[..63]),
        String::new(),
    ] {
        let error = agent
            .storefronts
            .media_chunk(&source, agent.seq(), &bad, 0)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{bad}");
    }
    let offset = agent
        .storefronts
        .media_chunk(&source, agent.seq(), &icon, 1 << 30)
        .unwrap_err();
    assert_eq!(offset.code, ErrorCode::InvalidRequest);
    // Nothing outside the media directory was touched or fetched.
    assert!(server.log().iter().all(|l| !l.contains("..")));
}

#[test]
fn a_cached_image_is_checked_on_every_read() {
    let site = Site::new("rehash");
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    let mut agent = Agent::new(&site);
    agent.refresh(&source, 1);
    let icon = site.icon();
    let sha = icon.sha256.to_string();
    let file = agent.state.join("media").join(&sha);
    assert!(file.is_file());
    let path = media_path(&icon);
    let before = fetches(&server, &path);
    agent.media(&source, &sha).unwrap();
    assert_eq!(fetches(&server, &path), before, "served from the cache");
    // Corrupted on disk: deleted, fetched again, and right.
    let mut bytes = std::fs::read(&file).unwrap();
    bytes[40] ^= 0xFF;
    std::fs::write(&file, &bytes).unwrap();
    assert_eq!(sha256(&agent.media(&source, &sha).unwrap()), icon.sha256);
    assert_eq!(fetches(&server, &path), before + 1);
    assert_eq!(sha256(&std::fs::read(&file).unwrap()), icon.sha256);
}

#[test]
fn a_header_that_differs_from_the_reference_is_refused() {
    let site = Site::new("header");
    // Re-sign a storefront whose listing claims another icon size.
    let storefront = site.storefront();
    let listing_path = site.site().join(&storefront.listings[0].reference.path);
    let mut listing: Value =
        serde_json::from_slice(&std::fs::read(&listing_path).unwrap()).unwrap();
    listing["icon"]["width"] = json!(512);
    listing["icon"]["height"] = json!(512);
    let bytes = serde_json::to_vec_pretty(&listing).unwrap();
    let digest = sha256(&bytes);
    let new_path = format!("storefront/listings/acme.clock/{digest}.json");
    std::fs::write(site.site().join(&new_path), &bytes).unwrap();
    let mut envelope: Value =
        serde_json::from_slice(&std::fs::read(site.site().join("storefront/1.json")).unwrap())
            .unwrap();
    envelope["listings"][0]["ref"] =
        json!({ "path": new_path, "sha256": digest.to_string(), "length": bytes.len() });
    std::fs::write(
        site.site().join("storefront/1.json"),
        serde_json::to_vec_pretty(&envelope).unwrap(),
    )
    .unwrap();
    std::fs::remove_file(site.site().join("storefront/1.json.minisig")).unwrap();
    // `storefront sign` refuses a listing that lies about its image, so
    // the envelope is signed by hand, as a compromised pipeline could.
    let key = deckpilot_plugin_registry::publish::SigningKey::load(
        Path::new(&site.key("index.key")),
        None,
    )
    .unwrap();
    let envelope_bytes = std::fs::read(site.site().join("storefront/1.json")).unwrap();
    let signature = key
        .sign(
            &envelope_bytes,
            &deckpilot_plugin_registry::sig::storefront_comment(1, 1),
        )
        .unwrap();
    std::fs::write(site.site().join("storefront/1.json.minisig"), signature).unwrap();
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    let mut agent = Agent::new(&site);
    agent.refresh(&source, 1);
    let sha = site.icon().sha256.to_string();
    let error = agent.media(&source, &sha).unwrap_err();
    assert!(error.message.contains("pinned"), "{}", error.message);
    assert!(!agent.cached_media().contains(&sha));
}

#[test]
fn a_cross_origin_redirect_is_not_followed() {
    let site = Site::new("redirect");
    let server = HttpsServer::start(&site.work());
    let other = HttpsServer::start(&site.work());
    let source = source_trusting(&server, vec![server.ca_pem.clone(), other.ca_pem.clone()]);
    let mut agent = Agent::new(&site);
    agent.refresh(&source, 1);
    let banner = site.banner();
    let path = media_path(&banner);
    server.route(
        &format!("/site/v1/{path}"),
        Route::Redirect {
            status: 302,
            location: other.url(&format!("site/v1/{path}")),
        },
    );
    assert!(agent.media(&source, &banner.sha256.to_string()).is_err());
    assert!(other.log().is_empty(), "{:?}", other.log());
    // The envelope too: refused, the storefront is absent, the refresh fine.
    let site2 = Site::new("redirect-envelope");
    let server = HttpsServer::start(&site2.work());
    let other = HttpsServer::start(&site2.work());
    server.route(
        "/site/v1/storefront/1.json",
        Route::Redirect {
            status: 302,
            location: other.url("site/v1/storefront/1.json"),
        },
    );
    let source = source_trusting(&server, vec![server.ca_pem.clone(), other.ca_pem.clone()]);
    let mut agent = Agent::new(&site2);
    assert_eq!(agent.refresh(&source, 1), Refresh::Updated { seq: 1 });
    assert!(agent.storefronts.current().is_none());
    assert!(agent.storefronts.problem().unwrap().contains("refused"));
    assert_eq!(other.log(), Vec::<String>::new());
}

#[test]
fn offline_the_cached_storefront_and_images_are_served() {
    let site = Site::new("offline");
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    let mut agent = Agent::new(&site);
    agent.refresh(&source, 1);
    let icon = site.icon().sha256.to_string();
    let banner = site.banner().sha256.to_string();
    // A restart with the registry unreachable.
    server.set_offline(true);
    let mut restarted = Agent::open(&site, agent.state.clone());
    assert_eq!(restarted.storefronts.current().map(Current::seq), Some(1));
    assert!(matches!(restarted.refresh(&source, 2), Refresh::Offline(_)));
    assert_eq!(restarted.storefronts.current().map(Current::seq), Some(1));
    assert_eq!(restarted.storefronts.current().unwrap().listings.len(), 1);
    // The prefetched icon is there; the banner was never fetched.
    assert_eq!(
        sha256(&restarted.media(&source, &icon).unwrap()).to_string(),
        icon
    );
    assert!(restarted.media(&source, &banner).is_err());
    // A tampered cache is not trusted offline either.
    let cached = restarted.state.join("storefront/1.json");
    let mut bytes = std::fs::read(&cached).unwrap();
    bytes[20] ^= 1;
    std::fs::write(&cached, bytes).unwrap();
    let tampered = Agent::open(&site, agent.state.clone());
    assert!(tampered.storefronts.current().is_none());
    assert!(tampered.storefronts.problem().is_some());
}

#[test]
fn the_media_cache_stays_within_its_budget() {
    let site = Site::new("budget");
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    let mut agent = Agent::new(&site);
    agent.refresh(&source, 1);
    let icon = site.icon();
    let banner = site.banner();
    // Stray files and an unreferenced image of an older storefront.
    let media = agent.state.join("media");
    std::fs::write(media.join(".tmpXYZ"), b"partial").unwrap();
    std::fs::write(media.join("f".repeat(64)), vec![0; 50_000]).unwrap();
    let budget = banner.length + icon.length;
    agent.storefronts = Storefronts::new(&agent.state).with_budget(budget);
    agent.storefronts.load_cached(&mut agent.client);
    agent.media(&source, &banner.sha256.to_string()).unwrap();
    let names = agent.cached_media();
    assert!(
        !names.contains(&"f".repeat(64)),
        "unreferenced goes first: {names:?}"
    );
    assert!(!names.contains(&".tmpXYZ".to_owned()));
    assert!(names.contains(&banner.sha256.to_string()));
    let total: u64 = names
        .iter()
        .map(|n| std::fs::metadata(media.join(n)).unwrap().len())
        .sum();
    assert!(total <= budget, "{total} > {budget}");
    // A budget below one image keeps just the image in use.
    agent.storefronts = Storefronts::new(&agent.state).with_budget(10);
    agent.storefronts.load_cached(&mut agent.client);
    std::fs::remove_file(media.join(banner.sha256.to_string())).unwrap();
    agent.media(&source, &banner.sha256.to_string()).unwrap();
    assert_eq!(agent.cached_media(), vec![banner.sha256.to_string()]);
}

#[test]
fn no_or_a_refused_storefront_leaves_the_registry_untouched() {
    // None published: catalog only, no problem.
    let site = Site::new("none");
    for file in ["storefront/1.json", "storefront/1.json.minisig"] {
        std::fs::remove_file(site.site().join(file)).unwrap();
    }
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    let mut agent = Agent::new(&site);
    assert_eq!(agent.refresh(&source, 1), Refresh::Updated { seq: 1 });
    assert!(agent.storefronts.current().is_none());
    assert_eq!(agent.storefronts.problem(), None);
    // A forged signature: refused, reported, the registry verified as usual.
    let site = Site::new("forged");
    let signature = site.site().join("storefront/1.json.minisig");
    let index_signature = std::fs::read(site.site().join("index/1.json.minisig")).unwrap();
    std::fs::write(&signature, index_signature).unwrap();
    let server = HttpsServer::start(&site.work());
    let forged = self::source(&server);
    let mut agent = Agent::new(&site);
    assert_eq!(agent.refresh(&forged, 1), Refresh::Updated { seq: 1 });
    assert!(agent.client.index().is_some());
    assert!(agent.storefronts.current().is_none());
    let problem = agent.storefronts.problem().unwrap();
    assert!(problem.contains("not a storefront signature"), "{problem}");
    // A refused storefront is never served from a stale cache either.
    assert!(!agent.state.join("storefront/1.json").exists());
}

#[test]
fn a_new_index_replaces_the_storefront_and_its_cache() {
    let site = Site::new("replace");
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    let mut agent = Agent::new(&site);
    agent.refresh(&source, 1);
    site.add("acme.beta", "0.2.0");
    site.publish(2);
    assert_eq!(agent.refresh(&source, 3), Refresh::Updated { seq: 2 });
    assert_eq!(agent.storefronts.current().map(Current::seq), Some(2));
    assert!(!agent.state.join("storefront/1.json").exists());
    assert!(agent.state.join("storefront/2.json").exists());
}

// ---------------------------------------------------------------- resolver

fn editorial(value: Value) -> EditorialDoc {
    let doc = EditorialDoc::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    doc.validate(deckpilot_plugin_registry::storefront::Strictness::Client)
        .unwrap();
    doc
}

fn art() -> Value {
    json!({ "asset": { "sha256": "ab".repeat(32), "length": 1000, "format": "png", "width": 1200, "height": 400 },
            "alt": { "en": "Art", "pl": "Grafika" } })
}

fn placeables(list: &[(&str, &[&str], &[&str])]) -> Placeables {
    list.iter()
        .map(|(id, categories, versions)| {
            (
                (*id).to_owned(),
                Placeable {
                    categories: categories.iter().map(|c| (*c).to_owned()).collect(),
                    versions: versions
                        .iter()
                        .map(|v| Version::parse(v).unwrap())
                        .collect(),
                },
            )
        })
        .collect()
}

fn sample() -> EditorialDoc {
    editorial(json!({
        "_type": "deckpilot-registry-editorial", "editorial_version": 1,
        "sections": [
            { "id": "zz", "kind": "catalog", "order": 50 },
            { "id": "aa", "kind": "catalog", "order": 50, "title": { "en": "All", "pl": "Wszystkie" } },
            { "id": "early", "kind": "collection_row", "order": 1, "title": { "en": "Soon" }, "plugins": ["acme.alpha"],
              "window": { "starts_at": "2026-12-01T00:00:00Z" } },
            { "id": "late", "kind": "collection_row", "order": 2, "title": { "en": "Gone" }, "plugins": ["acme.alpha"],
              "window": { "ends_at": "2026-11-01T00:00:00Z" } },
            { "id": "now", "kind": "collection_row", "order": 3, "title": { "en": "Now" },
              "plugins": ["acme.alpha", "acme.revoked", "acme.unknown", "acme.bravo"],
              "window": { "starts_at": "2026-11-01T00:00:00Z", "ends_at": "2026-12-01T00:00:00Z" } },
            { "id": "mac", "kind": "collection_row", "order": 4, "layout": "hero", "title": { "en": "Mac" },
              "plugins": ["acme.alpha"], "platforms": ["macos"] },
            { "id": "spot", "kind": "spotlight", "order": 6, "plugin": "acme.alpha", "version": "9.9.9", "headline": { "en": "A" } },
            { "id": "spot-ok", "kind": "spotlight", "order": 7, "plugin": "acme.alpha", "version": "1.0.0", "headline": { "en": "A" } },
            { "id": "banners", "kind": "banner_carousel", "order": 9, "items": [
                { "id": "p", "title": { "en": "P" }, "artwork": art(), "target": { "type": "plugin", "id": "acme.alpha" } },
                { "id": "x", "title": { "en": "X" }, "artwork": art(), "target": { "type": "plugin", "id": "acme.unknown" } },
                { "id": "c", "title": { "en": "C" }, "artwork": art(), "target": { "type": "collection", "id": "col" } },
                { "id": "e", "title": { "en": "E" }, "artwork": art(), "target": { "type": "collection", "id": "empty" } },
                { "id": "k", "title": { "en": "K" }, "artwork": art(), "target": { "type": "category", "id": "weather" } },
                { "id": "u", "title": { "en": "U" }, "artwork": art(), "target": { "type": "url", "id": "https://evil.example" } } ] },
            { "id": "cats", "kind": "category_highlights", "order": 10,
              "categories": [ { "id": "weather", "blurb": { "en": "Rain", "pl": "Deszcz" } }, { "id": "nothing" } ] },
            { "id": "future", "kind": "hologram", "order": 0 }
        ],
        "collections": [
            { "id": "col", "order": 2, "title": { "en": "Col" }, "plugins": ["acme.bravo", "acme.unknown"] },
            { "id": "empty", "order": 1, "title": { "en": "Empty" }, "plugins": ["acme.unknown"] }
        ]
    }))
}

fn sample_placeables() -> Placeables {
    placeables(&[
        ("acme.alpha", &["weather"], &["1.0.0"]),
        ("acme.bravo", &[], &["0.1.0"]),
    ])
}

#[test]
fn the_resolver_filters_orders_and_localizes() {
    let doc = sample();
    let now = "2026-11-15T00:00:00Z".parse().unwrap();
    let view = view(7, Some(&doc), &sample_placeables(), "pl-PL", "linux", now);
    assert_eq!(view.seq, 7);
    let ids: Vec<&str> = view.sections.iter().map(|s| s.id.as_str()).collect();
    // Window, platform, empty placements and unknown kinds dropped; sorted
    // by order, then id.
    assert_eq!(
        ids,
        ["now", "spot", "spot-ok", "banners", "cats", "aa", "zz"]
    );
    let now_row = &view.sections[0];
    assert_eq!(now_row.kind, "collectionRow");
    assert_eq!(now_row.plugins, ["acme.alpha", "acme.bravo"]);
    assert_eq!(
        view.sections[1].version, None,
        "an unpublished version is not shown"
    );
    assert_eq!(view.sections[2].version.as_deref(), Some("1.0.0"));
    let banners = &view.sections[3];
    let items: Vec<&str> = banners.items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(items, ["p", "c", "k"]);
    assert_eq!(
        banners.items[0].alt, "Grafika",
        "locale fallback pl-PL → pl"
    );
    assert_eq!(banners.items[0].title, "P", "→ en");
    assert_eq!(banners.items[0].artwork.media_type, "image/png");
    let cats = &view.sections[4];
    assert_eq!(cats.categories.len(), 1);
    assert_eq!(cats.categories[0].blurb.as_deref(), Some("Deszcz"));
    assert_eq!(view.sections[5].title.as_deref(), Some("Wszystkie"));
    // Collections: unknown ids and duplicates dropped, empty ones gone.
    assert_eq!(view.collections.len(), 1);
    assert_eq!(view.collections[0].plugins, ["acme.bravo"]);
    // On macOS the platform section appears, with a known layout.
    let mac = view_on(&doc, "macos", now);
    let section = mac.sections.iter().find(|s| s.id == "mac").unwrap();
    assert_eq!(section.layout.as_deref(), Some("cards"));
}

fn view_on(doc: &EditorialDoc, platform: &str, now: DateTime<Utc>) -> ipc::StorefrontView {
    view(1, Some(doc), &sample_placeables(), "en", platform, now)
}

#[test]
fn windows_follow_the_effective_clock() {
    let doc = sample();
    let ids = |now: &str| -> Vec<String> {
        view_on(&doc, "linux", now.parse().unwrap())
            .sections
            .iter()
            .map(|s| s.id.clone())
            .collect()
    };
    assert!(!ids("2026-10-31T23:59:59Z").contains(&"now".to_owned()));
    assert!(ids("2026-10-31T23:59:59Z").contains(&"late".to_owned()));
    assert!(ids("2026-11-01T00:00:00Z").contains(&"now".to_owned()));
    assert!(!ids("2026-11-01T00:00:00Z").contains(&"late".to_owned()));
    assert!(!ids("2026-12-01T00:00:00Z").contains(&"now".to_owned()));
    assert!(ids("2026-12-01T00:00:00Z").contains(&"early".to_owned()));
}

#[test]
fn nothing_installable_means_nothing_placed() {
    let doc = sample();
    let view = view(
        1,
        Some(&doc),
        &Placeables::new(),
        "en",
        "linux",
        "2026-11-15T00:00:00Z".parse().unwrap(),
    );
    let ids: Vec<&str> = view.sections.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["aa", "zz"], "only the catalog sections remain");
    assert_eq!(view.collections.len(), 0);
    // No editorial document: an empty view.
    let none = super::view(3, None, &sample_placeables(), "en", "linux", Utc::now());
    assert_eq!(none.sections.len(), 0);
}

#[test]
fn the_view_only_names_placeable_plugins() {
    // Whatever the editorial says, it can only name ids the agent offers:
    // names, publishers, badges and permissions come from the index.
    let doc = sample();
    let placeable = sample_placeables();
    let view = view_on(&doc, "macos", "2026-11-15T00:00:00Z".parse().unwrap());
    let mut named: Vec<&str> = view
        .sections
        .iter()
        .flat_map(|s| s.plugins.iter().map(String::as_str))
        .collect();
    named.extend(
        view.collections
            .iter()
            .flat_map(|c| c.plugins.iter().map(String::as_str)),
    );
    named.extend(
        view.sections
            .iter()
            .flat_map(|s| s.items.iter())
            .filter(|i| i.target.kind == "plugin")
            .map(|i| i.target.id.as_str()),
    );
    assert_ne!(named.len(), 0);
    assert!(
        named.iter().all(|id| placeable.contains_key(*id)),
        "{named:?}"
    );
    let json = serde_json::to_string(&view).unwrap();
    for field in [
        "\"name\"",
        "\"publisher\"",
        "\"verified\"",
        "\"official\"",
        "\"permissions\"",
    ] {
        assert!(!json.contains(field), "{field}");
    }
}

#[test]
fn the_answer_is_capped() {
    let id = |j: usize| {
        format!(
            "acme.{}{}",
            "q".repeat(31),
            char::from(b'a' + u8::try_from(j).unwrap())
        )
    };
    let item = |i: usize| {
        json!({ "id": format!("b{i}"), "title": { "en": "ż".repeat(60) }, "body": { "en": "ż".repeat(160) },
                "cta": { "en": "ż".repeat(24) },
                "artwork": { "asset": { "sha256": "ab".repeat(32), "length": 1000, "format": "png", "width": 1200, "height": 400 },
                             "alt": { "en": "ż".repeat(250) } },
                "target": { "type": "plugin", "id": id(i) } })
    };
    let sections: Vec<Value> = (0..12)
        .map(|i| {
            json!({ "id": format!("s{i:02}"), "kind": "banner_carousel", "order": i,
                         "items": (0..6).map(item).collect::<Vec<_>>() })
        })
        .collect();
    let collections: Vec<Value> = (0..16)
        .map(|i| {
            json!({ "id": format!("c{i:02}"), "order": i, "title": { "en": "x".repeat(40) },
                         "plugins": (0..24).map(id).collect::<Vec<_>>() })
        })
        .collect();
    let doc = editorial(
        json!({ "_type": "deckpilot-registry-editorial", "editorial_version": 1,
                                "sections": sections, "collections": collections }),
    );
    let placeable: Placeables = (0..24)
        .map(|j| {
            let mut p = Placeable::default();
            p.versions.insert(Version::new(1, 0, 0));
            (id(j), p)
        })
        .collect();
    let uncapped = serde_json::to_vec(&doc).unwrap().len();
    assert!(uncapped > MAX_STOREFRONT_ANSWER_BYTES, "{uncapped}");
    let view = view(1, Some(&doc), &placeable, "en", "linux", Utc::now());
    let size = serde_json::to_vec(&view).unwrap().len();
    assert!(size <= MAX_STOREFRONT_ANSWER_BYTES, "{size}");
    assert!(
        !view.sections.is_empty() && view.sections.len() < 12,
        "{}",
        view.sections.len()
    );
    // Trailing sections went first; collections are kept.
    assert_eq!(view.sections[0].id, "s00");
    assert_eq!(view.collections.len(), 16);
}

#[test]
fn listings_are_localized_filtered_and_capped() {
    let listing: ListingDoc = serde_json::from_value(json!({
        "_type": "deckpilot-registry-listing", "listing_version": 1, "id": "acme.alpha",
        "promo": { "en": "Hi", "pl": "Cześć" },
        "description_md": { "en": "d".repeat(6000) },
        "screenshots": [
            { "asset": { "sha256": "ab".repeat(32), "length": 9, "format": "webp", "width": 1280, "height": 800 },
              "alt": { "en": "All" } },
            { "asset": { "sha256": "cd".repeat(32), "length": 9, "format": "png", "width": 1280, "height": 800 },
              "alt": { "en": "Mac" }, "platforms": ["macos"] }
        ],
        "links": { "support": "https://acme.example/help" },
        "release_notes": {
            "1.0.0": { "en": "a".repeat(2000) }, "1.2.0": { "en": "b".repeat(2000) },
            "1.10.0": { "en": "c".repeat(2000) }, "9.0.0": { "en": "never published" }
        }
    }))
    .unwrap();
    let published: BTreeSet<Version> = ["1.0.0", "1.2.0", "1.10.0"]
        .iter()
        .map(|v| Version::parse(v).unwrap())
        .collect();
    let view = listing_view(&listing, &published, "pl", "linux");
    assert_eq!(view.promo.as_deref(), Some("Cześć"));
    assert_eq!(view.screenshots.len(), 1);
    assert_eq!(view.screenshots[0].media.media_type, "image/webp");
    let versions: Vec<&str> = view
        .release_notes
        .iter()
        .map(|n| n.version.as_str())
        .collect();
    assert_eq!(
        versions,
        ["1.10.0", "1.2.0", "1.0.0"],
        "newest first, semver order, published only"
    );
    assert_eq!(
        view.links.support.as_deref(),
        Some("https://acme.example/help")
    );
    assert_eq!(
        listing_view(&listing, &published, "en", "macos")
            .screenshots
            .len(),
        2
    );
    assert!(serde_json::to_vec(&view).unwrap().len() <= MAX_STOREFRONT_ANSWER_BYTES);
}

/// The site of the R8 compatibility matrix (storefront design §10), run by
/// hand: `DECKPILOT_R8_SITE=<dir> cargo test -p deckpilot-agent --features
/// plugins --lib export_compatibility_site -- --ignored`. Writes
/// `<dir>/with/` (the registry with a storefront, media and an editorial
/// section of a kind no client knows), `<dir>/bare/` (the same registry
/// without `storefront/` and `media/`) and `<dir>/roots.json`, signed with
/// throwaway keys for the current time.
#[test]
#[ignore = "writes the compatibility matrix's site; run by hand"]
fn export_compatibility_site() {
    let out = PathBuf::from(std::env::var("DECKPILOT_R8_SITE").expect("DECKPILOT_R8_SITE"));
    let site = Site::new_at("r8", Utc::now() - TimeDelta::hours(1));
    // A future section kind in the signed editorial document, re-signed.
    let storefront = site.storefront();
    let editorial_ref = storefront.editorial.clone().unwrap();
    let mut editorial: Value =
        serde_json::from_slice(&std::fs::read(site.site().join(&editorial_ref.path)).unwrap())
            .unwrap();
    editorial["sections"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "id": "future", "kind": "hologram", "order": 1, "beam": { "en": "?" } }));
    let bytes = serde_json::to_vec_pretty(&editorial).unwrap();
    let digest = sha256(&bytes);
    let path = format!("storefront/editorial/{digest}.json");
    std::fs::write(site.site().join(&path), &bytes).unwrap();
    std::fs::remove_file(site.site().join(&editorial_ref.path)).unwrap();
    let mut envelope: Value =
        serde_json::from_slice(&std::fs::read(site.site().join("storefront/1.json")).unwrap())
            .unwrap();
    envelope["editorial"] =
        json!({ "path": path, "sha256": digest.to_string(), "length": bytes.len() });
    std::fs::write(
        site.site().join("storefront/1.json"),
        serde_json::to_vec_pretty(&envelope).unwrap(),
    )
    .unwrap();
    std::fs::remove_file(site.site().join("storefront/1.json.minisig")).unwrap();
    site.run(&["storefront", "sign", "--key", &site.key("index.key")]);
    site.run(&["verify-site", "--require-storefront", "--require-packages"]);
    let copy = |from: &Path, to: &Path| {
        fn walk(from: &Path, to: &Path) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap().flatten() {
                let target = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    walk(&entry.path(), &target);
                } else {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        walk(from, to);
    };
    let _ = std::fs::remove_dir_all(&out);
    for variant in ["with", "bare"] {
        for part in ["site", "releases"] {
            copy(&site.work().join(part), &out.join(variant).join(part));
        }
    }
    for gone in ["storefront", "media"] {
        std::fs::remove_dir_all(out.join("bare/site/v1").join(gone)).unwrap();
    }
    std::fs::copy(site.work().join("roots.json"), out.join("roots.json")).unwrap();
}

/// A source whose image fetches are slow.
struct SlowMedia<'a> {
    inner: &'a HttpsSource,
    delay: std::time::Duration,
}

impl RegistrySource for SlowMedia<'_> {
    fn fetch(&self, path: &str, max_bytes: usize) -> Result<Vec<u8>, FetchError> {
        if path.starts_with("media/") {
            std::thread::sleep(self.delay);
        }
        self.inner.fetch(path, max_bytes)
    }
}

#[test]
fn a_refresh_never_waits_for_images() {
    let site = Site::new("slow");
    let server = HttpsServer::start(&site.work());
    let https = source(&server);
    let slow = SlowMedia {
        inner: &https,
        delay: std::time::Duration::from_secs(2),
    };
    let mut agent = Agent::new(&site);
    let started = std::time::Instant::now();
    let refresh = agent.client.refresh(&slow, at(1)).unwrap();
    agent.storefronts.sync(&mut agent.client, &slow, &refresh);
    let elapsed = started.elapsed();
    assert_eq!(refresh, Refresh::Updated { seq: 1 });
    assert!(agent.storefronts.current().is_some());
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "the refresh took {elapsed:?}"
    );
    assert_eq!(
        agent.cached_media(),
        Vec::<String>::new(),
        "nothing fetched yet"
    );
    assert_eq!(agent.storefronts.queued(), 1);
    // The deferred prefetch is bounded by time (no fetch starts after the
    // budget) and by count; the queue is gone either way.
    let started = std::time::Instant::now();
    agent
        .storefronts
        .prefetch(&slow, MAX_PREFETCH, std::time::Duration::ZERO);
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(agent.storefronts.queued(), 0);
    assert_eq!(agent.cached_media(), Vec::<String>::new());
    let mut other = Agent::open(&site, site.scratch.0.join("other-state"));
    let refresh = other.client.refresh(&slow, at(1)).unwrap();
    other.storefronts.sync(&mut other.client, &slow, &refresh);
    assert_eq!(other.storefronts.queued(), 1);
    other.storefronts.prefetch(&slow, 0, PREFETCH_BUDGET);
    assert_eq!(other.storefronts.queued(), 0);
    assert_eq!(other.cached_media(), Vec::<String>::new());
    // Within both bounds it is fetched ahead.
    let mut third = Agent::open(&site, site.scratch.0.join("third-state"));
    let refresh = third.client.refresh(&slow, at(1)).unwrap();
    third.storefronts.sync(&mut third.client, &slow, &refresh);
    third
        .storefronts
        .prefetch(&slow, MAX_PREFETCH, PREFETCH_BUDGET);
    assert_eq!(third.cached_media(), vec![site.icon().sha256.to_string()]);
    // On demand it is fetched all the same.
    let icon = site.icon().sha256.to_string();
    assert_eq!(
        sha256(&agent.media(&https, &icon).unwrap()).to_string(),
        icon
    );
}

#[test]
fn only_the_storefront_of_the_verified_seq_is_served() {
    let site = Site::new("served-seq");
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    let mut agent = Agent::new(&site);
    agent.refresh(&source, 1);
    let icon = site.icon().sha256.to_string();
    assert!(agent.storefronts.current_for(agent.seq()).is_some());
    // A newer index verified without the storefront following it (as if a
    // sync were skipped): nothing of the old one is served.
    site.add("acme.beta", "0.2.0");
    site.publish(2);
    agent.client.refresh(&source, at(3)).unwrap();
    assert_eq!(agent.seq(), Some(2));
    assert!(agent.storefronts.current().is_some());
    assert!(agent.storefronts.current_for(agent.seq()).is_none());
    let error = agent
        .storefronts
        .media_chunk(&source, agent.seq(), &icon, 0)
        .unwrap_err();
    assert_eq!(error.reason.as_deref(), Some(reason::UNKNOWN_MEDIA));
}
