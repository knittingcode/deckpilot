//! The storefront (storefront design §3 to §6 and §10, rows R1, R2 and R4
//! for this crate): media header checks, listing and editorial rules, the
//! client's verification of the signed storefront, and the publishing
//! commands `storefront check|build|sign` with `verify-site
//! --require-storefront`, all with throwaway keys.
#![allow(
    clippy::cast_possible_truncation,
    clippy::trivially_copy_pass_by_ref,
    clippy::needless_pass_by_value,
    clippy::unused_self,
    clippy::single_match_else,
    clippy::too_many_lines,
    clippy::type_complexity,
    clippy::map_unwrap_or,
    clippy::redundant_closure_for_method_calls
)]

mod common;

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use common::*;
use deckpilot_plugin_package::sha256;
use deckpilot_plugin_registry::media::{self, MediaError, MediaFormat};
use deckpilot_plugin_registry::publish::{Io, Workspace, run};
use deckpilot_plugin_registry::sig::{index_comment, storefront_comment, timestamp_comment};
use deckpilot_plugin_registry::storefront::*;
use deckpilot_plugin_registry::*;
use serde_json::{Value, json};
use tempfile::TempDir;

// ================================================================ images

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

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc(&[kind, data]).to_be_bytes());
    out
}

fn ihdr(width: u32, height: u32, depth: u8, color: u8) -> Vec<u8> {
    let mut data = width.to_be_bytes().to_vec();
    data.extend_from_slice(&height.to_be_bytes());
    data.extend_from_slice(&[depth, color, 0, 0, 0]);
    chunk(b"IHDR", &data)
}

/// A PNG whose headers are valid; its IDAT is not real image data (the
/// header walk does not decode, a full decode refuses it).
fn png_headers(width: u32, height: u32, extra: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    out.extend(ihdr(width, height, 8, 6));
    for c in extra {
        out.extend_from_slice(c);
    }
    out.extend(chunk(b"IDAT", b"not really deflate"));
    out.extend(chunk(b"IEND", b""));
    out
}

fn rgb(width: u32, height: u32) -> image::RgbImage {
    image::RgbImage::from_fn(width, height, |x, y| {
        image::Rgb([(x % 251) as u8, (y % 241) as u8, 90])
    })
}

fn encode(width: u32, height: u32, format: image::ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    match format {
        image::ImageFormat::WebP => {
            image::DynamicImage::ImageRgba8(
                image::DynamicImage::ImageRgb8(rgb(width, height)).to_rgba8(),
            )
            .write_to(&mut out, format)
            .unwrap();
        }
        _ => rgb(width, height).write_to(&mut out, format).unwrap(),
    }
    out.into_inner()
}

fn real_png(width: u32, height: u32) -> Vec<u8> {
    encode(width, height, image::ImageFormat::Png)
}

fn real_jpeg(width: u32, height: u32) -> Vec<u8> {
    encode(width, height, image::ImageFormat::Jpeg)
}

fn real_webp(width: u32, height: u32) -> Vec<u8> {
    encode(width, height, image::ImageFormat::WebP)
}

/// A JPEG made of headers only: SOI, APP0, a frame header, one scan, EOI.
fn jpeg_headers(
    sof: u8,
    precision: u8,
    width: u16,
    height: u16,
    components: u8,
    extra: &[Vec<u8>],
) -> Vec<u8> {
    let segment = |marker: u8, data: &[u8]| {
        let mut out = vec![0xFF, marker];
        out.extend_from_slice(&((data.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(data);
        out
    };
    let mut out = vec![0xFF, 0xD8];
    out.extend(segment(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"));
    for e in extra {
        out.extend_from_slice(e);
    }
    let mut frame = vec![precision];
    frame.extend_from_slice(&height.to_be_bytes());
    frame.extend_from_slice(&width.to_be_bytes());
    frame.push(components);
    for c in 0..components {
        frame.extend_from_slice(&[c + 1, 0x11, 0]);
    }
    out.extend(segment(sof, &frame));
    let mut scan = vec![components];
    for c in 0..components {
        scan.extend_from_slice(&[c + 1, 0]);
    }
    scan.extend_from_slice(&[0, 63, 0]);
    out.extend(segment(0xDA, &scan));
    out.extend_from_slice(&[0x12, 0x34, 0xFF, 0x00, 0x56, 0xFF, 0xD0, 0x78]);
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

fn jpeg_segment(marker: u8, data: &[u8]) -> Vec<u8> {
    let mut out = vec![0xFF, marker];
    out.extend_from_slice(&((data.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(data);
    out
}

fn riff(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut body = b"WEBP".to_vec();
    for (kind, data) in chunks {
        body.extend_from_slice(*kind);
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(data);
        if data.len() % 2 == 1 {
            body.push(0);
        }
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

fn vp8l(width: u32, height: u32) -> Vec<u8> {
    let bits = (width - 1) | ((height - 1) << 14);
    let mut data = vec![0x2f];
    data.extend_from_slice(&bits.to_le_bytes());
    data.extend_from_slice(&[0, 0, 0]);
    data
}

fn vp8x(flags: u8, width: u32, height: u32) -> Vec<u8> {
    let mut data = vec![flags, 0, 0, 0];
    data.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
    data.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
    data
}

fn refused(bytes: &[u8]) -> MediaError {
    media::inspect(bytes).expect_err("refused")
}

// ================================================================ media (R1)

#[test]
fn real_images_pass_the_header_walk() {
    for (bytes, format) in [
        (real_png(1280, 800), MediaFormat::Png),
        (real_jpeg(1280, 720), MediaFormat::Jpeg),
        (real_webp(640, 320), MediaFormat::Webp),
    ] {
        let info = media::inspect(&bytes).unwrap();
        assert_eq!(info.format, format);
        assert_eq!(MediaFormat::sniff(&bytes), Some(format));
    }
    let info = media::inspect(&real_png(1280, 800)).unwrap();
    assert_eq!((info.width, info.height), (1280, 800));
    let info = media::inspect(&real_jpeg(1280, 720)).unwrap();
    assert_eq!((info.width, info.height), (1280, 720));
    let info = media::inspect(&real_webp(640, 320)).unwrap();
    assert_eq!((info.width, info.height), (640, 320));
}

#[test]
fn hand_made_headers_pass_without_decoding() {
    assert!(media::inspect(&png_headers(512, 512, &[])).is_ok());
    assert!(
        media::inspect(&png_headers(
            512,
            512,
            &[chunk(b"sRGB", &[0]), chunk(b"pHYs", &[0; 9])]
        ))
        .is_ok()
    );
    assert!(media::inspect(&png_headers(512, 512, &[chunk(b"iCCP", &[b'x'; 2000])])).is_ok());
    assert!(media::inspect(&jpeg_headers(0xC0, 8, 1280, 800, 3, &[])).is_ok());
    assert!(media::inspect(&jpeg_headers(0xC2, 8, 1280, 800, 1, &[])).is_ok());
    assert!(media::inspect(&riff(&[(b"VP8L", vp8l(300, 100))])).is_ok());
    assert!(
        media::inspect(&riff(&[
            (b"VP8X", vp8x(0x10, 300, 100)),
            (b"ALPH", vec![0; 3]),
            (b"VP8L", vp8l(300, 100))
        ]))
        .is_ok()
    );
}

#[test]
fn animation_is_refused() {
    let apng = png_headers(512, 512, &[chunk(b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0])]);
    assert_eq!(refused(&apng), MediaError::Animated("APNG"));
    let animated = riff(&[
        (b"VP8X", vp8x(0x02, 300, 100)),
        (b"ANIM", vec![0; 6]),
        (b"ANMF", vec![0; 16]),
    ]);
    assert!(matches!(refused(&animated), MediaError::Animated(_)));
    // The chunk alone, without the flag, too.
    let sneaky = riff(&[
        (b"VP8X", vp8x(0, 300, 100)),
        (b"ANMF", vec![0; 16]),
        (b"VP8L", vp8l(300, 100)),
    ]);
    assert!(matches!(refused(&sneaky), MediaError::Animated(_)));
}

#[test]
fn metadata_is_refused() {
    for kind in [b"tEXt", b"zTXt", b"iTXt", b"eXIf"] {
        let png = png_headers(512, 512, &[chunk(kind, b"Author\0someone")]);
        assert!(matches!(refused(&png), MediaError::Metadata(_)), "{kind:?}");
    }
    let exif = jpeg_segment(0xE1, b"Exif\0\0MM\0*");
    let xmp = jpeg_segment(0xE1, b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta/>");
    let iptc = jpeg_segment(0xED, b"Photoshop 3.0\0");
    let comment = jpeg_segment(0xFE, b"made on my laptop");
    for extra in [exif, xmp, iptc, comment] {
        let jpeg = jpeg_headers(0xC0, 8, 1280, 800, 3, &[extra]);
        assert!(matches!(refused(&jpeg), MediaError::Metadata(_)));
    }
    for (kind, flag) in [(b"EXIF", 0x08), (b"XMP ", 0x04)] {
        let with_chunk = riff(&[
            (b"VP8X", vp8x(0, 300, 100)),
            (b"VP8L", vp8l(300, 100)),
            (kind, vec![1, 2, 3, 4]),
        ]);
        assert!(
            matches!(refused(&with_chunk), MediaError::Metadata(_)),
            "{kind:?}"
        );
        let with_flag = riff(&[(b"VP8X", vp8x(flag, 300, 100)), (b"VP8L", vp8l(300, 100))]);
        assert!(
            matches!(refused(&with_flag), MediaError::Metadata(_)),
            "{kind:?}"
        );
    }
    // An ICC profile above 64 KiB.
    let big_icc = png_headers(512, 512, &[chunk(b"iCCP", &vec![b'x'; 65 * 1024])]);
    assert!(matches!(refused(&big_icc), MediaError::Metadata(_)));
}

#[test]
fn svg_and_unknown_formats_are_refused() {
    assert_eq!(
        refused(b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script/></svg>"),
        MediaError::Svg
    );
    assert_eq!(refused(b"<?xml version=\"1.0\"?>\n<svg/>"), MediaError::Svg);
    assert_eq!(refused(b"\xef\xbb\xbf  <svg/>"), MediaError::Svg);
    assert_eq!(
        refused(b"GIF89a\x01\x00\x01\x00"),
        MediaError::UnknownFormat
    );
    assert_eq!(refused(b""), MediaError::UnknownFormat);
}

#[test]
fn bit_depth_and_components_are_limited() {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(ihdr(512, 512, 16, 6));
    png.extend(chunk(b"IDAT", b"x"));
    png.extend(chunk(b"IEND", b""));
    assert!(matches!(refused(&png), MediaError::BitDepth(_)));
    assert!(matches!(
        refused(&jpeg_headers(0xC0, 12, 1280, 800, 3, &[])),
        MediaError::BitDepth(_)
    ));
    assert!(
        matches!(
            refused(&jpeg_headers(0xC0, 8, 1280, 800, 4, &[])),
            MediaError::Unsupported(_)
        ),
        "CMYK"
    );
    assert!(
        matches!(
            refused(&jpeg_headers(0xC1, 8, 1280, 800, 3, &[])),
            MediaError::Unsupported(_)
        ),
        "SOF1"
    );
    assert!(
        matches!(
            refused(&jpeg_headers(0xC3, 8, 1280, 800, 3, &[])),
            MediaError::Unsupported(_)
        ),
        "lossless"
    );
}

#[test]
fn malformed_containers_are_refused() {
    let good = png_headers(512, 512, &[]);
    // Bad CRC.
    let mut bad_crc = good.clone();
    let at = 8 + 8 + 13; // the IHDR CRC
    bad_crc[at] ^= 0xFF;
    assert!(matches!(refused(&bad_crc), MediaError::Malformed { .. }));
    // Truncated.
    for cut in [10, 20, 40, good.len() - 4] {
        assert!(
            matches!(refused(&good[..cut]), MediaError::Malformed { .. }),
            "cut at {cut}"
        );
    }
    // Data after IEND.
    let mut after = good.clone();
    after.extend_from_slice(b"trailing");
    assert!(matches!(refused(&after), MediaError::Malformed { .. }));
    // A chunk outside the allow-list.
    assert!(matches!(
        refused(&png_headers(512, 512, &[chunk(b"sPLT", b"x\0\x08")])),
        MediaError::Unsupported(_)
    ));
    // RIFF length mismatch, both ways.
    let webp = riff(&[(b"VP8L", vp8l(300, 100))]);
    let mut long = webp.clone();
    long.extend_from_slice(&[0, 0]);
    assert!(matches!(refused(&long), MediaError::Malformed { .. }));
    assert!(matches!(
        refused(&webp[..webp.len() - 2]),
        MediaError::Malformed { .. }
    ));
    // JPEG: data after EOI, no EOI.
    let jpeg = jpeg_headers(0xC0, 8, 1280, 800, 3, &[]);
    let mut trailing = jpeg.clone();
    trailing.extend_from_slice(b"zz");
    assert!(matches!(refused(&trailing), MediaError::Malformed { .. }));
    assert!(matches!(
        refused(&jpeg[..jpeg.len() - 2]),
        MediaError::Malformed { .. }
    ));
    // VP8X canvas and image sizes differ.
    let differ = riff(&[(b"VP8X", vp8x(0, 300, 100)), (b"VP8L", vp8l(301, 100))]);
    assert!(matches!(refused(&differ), MediaError::Malformed { .. }));
}

#[test]
fn size_and_pixel_limits() {
    let mut huge = real_png(64, 64);
    // Over 1 MiB, whatever the content.
    huge.resize(1024 * 1024 + 1, 0);
    assert!(matches!(refused(&huge), MediaError::TooLarge { .. }));
    // 4096 × 4096 is above 8 MiP; the header alone says so.
    assert!(matches!(
        refused(&png_headers(4096, 4096, &[])),
        MediaError::Dimensions { .. }
    ));
    assert!(media::inspect(&png_headers(4096, 2048, &[])).is_ok());
    assert!(matches!(
        refused(&jpeg_headers(0xC0, 8, 0, 800, 3, &[])),
        MediaError::Dimensions { .. }
    ));
}

// ======================================================= verify_media (R2)

fn asset_of(bytes: &[u8]) -> AssetRef {
    let info = media::inspect(bytes).unwrap();
    AssetRef {
        sha256: sha256(bytes),
        length: bytes.len() as u64,
        format: info.format,
        width: info.width,
        height: info.height,
    }
}

#[test]
fn media_must_match_its_reference() {
    let bytes = real_png(1280, 800);
    let asset = asset_of(&bytes);
    assert!(verify_media(&asset, &bytes).is_ok());
    // Short, long.
    assert!(matches!(
        verify_media(&asset, &bytes[..bytes.len() - 1]),
        Err(MediaMismatch::Length { .. })
    ));
    let mut long = bytes.clone();
    long.push(0);
    assert!(matches!(
        verify_media(&asset, &long),
        Err(MediaMismatch::Length { .. })
    ));
    // Same length, other bytes.
    let mut other = bytes.clone();
    *other.last_mut().unwrap() ^= 1;
    assert!(matches!(
        verify_media(&asset, &other),
        Err(MediaMismatch::Digest { .. })
    ));
    // The reference lies about the header.
    for lie in [
        AssetRef {
            width: 1281,
            ..asset
        },
        AssetRef {
            height: 801,
            ..asset
        },
        AssetRef {
            format: MediaFormat::Webp,
            ..asset
        },
    ] {
        assert!(matches!(
            verify_media(&lie, &bytes),
            Err(MediaMismatch::Header { .. })
        ));
    }
    // A hash-correct reference to a refused file.
    let apng = png_headers(512, 512, &[chunk(b"acTL", &[0; 8])]);
    let pinned = AssetRef {
        sha256: sha256(&apng),
        length: apng.len() as u64,
        format: MediaFormat::Png,
        width: 512,
        height: 512,
    };
    assert!(matches!(
        verify_media(&pinned, &apng),
        Err(MediaMismatch::Media(MediaError::Animated(_)))
    ));
}

// ============================================================ listing (R1)

fn asset(format: &str, width: u32, height: u32, length: u64) -> Value {
    json!({ "sha256": "ab".repeat(32), "length": length, "format": format, "width": width, "height": height })
}

fn listing_value(id: &str) -> Value {
    json!({
        "_type": "deckpilot-registry-listing",
        "listing_version": 1,
        "id": id,
        "promo": { "en": "Weather on your deck, at a glance", "pl": "Pogoda na Twoim decku" },
        "description_md": { "en": "Current conditions and a **7-day** forecast.\n\n- Per-key city\n- [Docs](https://github.com/knittingcode/deckpilot-plugins)" },
        "highlights": [ { "title": { "en": "7-day forecast" }, "text": { "en": "Daily highs, lows and rain chance." } } ],
        "icon": asset("png", 512, 512, 48211),
        "screenshots": [
            { "asset": asset("webp", 2560, 1600, 610_233), "alt": { "en": "Six weather keys" },
              "caption": { "en": "The forecast page" }, "platforms": ["macos", "windows"] }
        ],
        "promo_art": { "asset": asset("jpg", 1600, 800, 300_000), "alt": { "en": "Weather icons" } },
        "links": {
            "documentation": "https://github.com/knittingcode/deckpilot-plugins/tree/main/first-party/weather",
            "support": "https://github.com/knittingcode/deckpilot-plugins/issues"
        },
        "release_notes": { "0.2.1": { "en": "Fixes the wind unit in Polish." } }
    })
}

fn listing_check(value: &Value, strictness: Strictness) -> Result<(), String> {
    let doc: ListingDoc = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    doc.validate(strictness)
}

fn set(value: &mut Value, pointer: &str, new: Value) {
    let (parent, key) = pointer.rsplit_once('/').unwrap();
    let target = value.pointer_mut(parent).unwrap();
    match target {
        Value::Object(map) => {
            map.insert(key.to_owned(), new);
        }
        Value::Array(list) => list[key.parse::<usize>().unwrap()] = new,
        _ => panic!("{pointer}"),
    }
}

fn listing_refused(pointer: &str, new: Value) -> String {
    let mut value = listing_value("deckpilot.weather");
    set(&mut value, pointer, new);
    listing_check(&value, Strictness::Publish).expect_err(&format!("{pointer} is refused"))
}

#[test]
fn a_valid_listing_passes_both_ways() {
    let value = listing_value("deckpilot.weather");
    listing_check(&value, Strictness::Publish).unwrap();
    listing_check(&value, Strictness::Client).unwrap();
    // A minimal one too.
    let minimal =
        json!({ "_type": "deckpilot-registry-listing", "listing_version": 1, "id": "acme.clock" });
    listing_check(&minimal, Strictness::Publish).unwrap();
}

#[test]
fn listing_text_rules() {
    assert!(listing_refused("/promo", json!({ "pl": "Tylko po polsku" })).contains("no `en`"));
    assert!(listing_refused("/promo", json!({ "en": "" })).contains("characters"));
    assert!(listing_refused("/promo", json!({ "en": "x".repeat(81) })).contains("characters"));
    assert!(listing_refused("/promo", json!({ "en": "ok", "de": "nein" })).contains("language"));
    assert!(listing_refused("/promo", json!({ "en": "line\nbreak" })).contains("control"));
    assert!(listing_refused("/promo", json!({ "en": "bell\u{7}" })).contains("control"));
    for bidi in ['\u{202E}', '\u{2066}', '\u{200F}'] {
        assert!(
            listing_refused("/promo", json!({ "en": format!("evil{bidi}txt.exe") }))
                .contains("bidirectional")
        );
    }
    assert!(listing_refused("/promo", json!({ "en": "zero\u{200B}width" })).contains("zero-width"));
    // Decomposed é (e + U+0301) is not NFC.
    assert!(listing_refused("/promo", json!({ "en": "Cafe\u{301}" })).contains("NFC"));
    assert!(
        listing_refused(
            "/highlights",
            json!(vec![
                json!({ "title": { "en": "a" }, "text": { "en": "b" } });
                7
            ])
        )
        .contains("highlights")
    );
    assert!(
        listing_refused("/highlights/0/title", json!({ "en": "x".repeat(41) }))
            .contains("characters")
    );
    assert!(
        listing_refused("/release_notes", json!({ "latest": { "en": "x" } }))
            .contains("release_notes")
    );
    assert!(
        listing_refused(
            "/release_notes",
            json!({ "0.2.1": { "en": "x".repeat(2001) } })
        )
        .contains("characters")
    );
}

#[test]
fn the_client_accepts_new_languages_and_ignores_unknown_fields() {
    let mut value = listing_value("acme.weather");
    set(
        &mut value,
        "/links",
        json!({ "support": "https://acme.example/help" }),
    );
    set(
        &mut value,
        "/promo",
        json!({ "en": "ok", "de": "gut", "pt-BR": "bom" }),
    );
    set(
        &mut value,
        "/promo_video",
        json!({ "url": "https://acme.example/v.mp4" }),
    );
    listing_check(&value, Strictness::Client).unwrap();
    assert!(listing_check(&value, Strictness::Publish).is_err());
    // A malformed tag is refused by both.
    set(&mut value, "/promo", json!({ "en": "ok", "EN-us": "no" }));
    assert!(listing_check(&value, Strictness::Client).is_err());
    // NFC is a publishing rule only.
    set(&mut value, "/promo", json!({ "en": "Cafe\u{301}" }));
    listing_check(&value, Strictness::Client).unwrap();
}

#[test]
fn listing_asset_rules() {
    assert!(
        listing_refused(
            "/screenshots",
            json!(vec![
                json!({ "asset": asset("png", 1280, 800, 1000), "alt": { "en": "a" } });
                9
            ])
        )
        .contains("screenshots")
    );
    assert!(listing_refused("/screenshots/0/alt", json!({ "pl": "tylko" })).contains("no `en`"));
    assert!(listing_refused("/screenshots/0/alt", json!({ "en": "" })).contains("characters"));
    // Aspect, width, bytes, pixels.
    assert!(
        listing_refused("/screenshots/0/asset", asset("png", 1280, 1280, 1000)).contains("16:10")
    );
    assert!(
        listing_refused("/screenshots/0/asset", asset("png", 1000, 625, 1000)).contains("16:10")
    );
    assert!(
        listing_refused("/screenshots/0/asset", asset("png", 2561, 1600, 1000)).contains("16:10")
    );
    assert!(
        listing_refused(
            "/screenshots/0/asset",
            asset("png", 2560, 1600, 1024 * 1024 + 1)
        )
        .contains("bytes")
    );
    assert!(listing_refused("/icon", asset("png", 255, 255, 1000)).contains("square"));
    assert!(listing_refused("/icon", asset("png", 512, 511, 1000)).contains("square"));
    assert!(listing_refused("/icon", asset("jpg", 512, 512, 1000)).contains("JPEG"));
    assert!(listing_refused("/icon", asset("png", 512, 512, 256 * 1024 + 1)).contains("bytes"));
    assert!(listing_refused("/promo_art/asset", asset("png", 1600, 1600, 1000)).contains("2:1"));
    assert!(listing_refused("/promo_art/alt", json!({})).contains("no `en`"));
    assert!(listing_refused("/promo_art/decorative", json!(true)).contains("decorative"));
    assert!(listing_refused("/screenshots/0/platforms", json!(["amiga"])).contains("platform"));
    assert!(listing_refused("/screenshots/0/platforms", json!([])).contains("at least one"));
    // The SVG format does not even parse.
    assert!(listing_refused("/icon", asset("svg", 512, 512, 1000)).contains("unknown variant"));
    // Unknown fields in an asset reference are refused (strict).
    let mut strict = asset("png", 512, 512, 1000);
    strict["url"] = json!("https://evil.example/x.png");
    assert!(listing_refused("/icon", strict).contains("unknown field"));
    // 9 MiB of media in total.
    let shots: Vec<Value> = (0..8)
        .map(|_| json!({ "asset": asset("png", 2560, 1600, 1024 * 1024), "alt": { "en": "a" } }))
        .collect();
    let mut value = listing_value("deckpilot.weather");
    set(&mut value, "/screenshots", json!(shots));
    set(
        &mut value,
        "/promo_art/asset",
        asset("png", 3200, 1600, 1024 * 1024),
    );
    assert!(
        listing_check(&value, Strictness::Publish)
            .unwrap_err()
            .contains("media bytes")
    );
}

#[test]
fn url_rules() {
    let ok = |url: &str, fp: bool| check_url("link", url, fp);
    assert!(ok("https://acme.example/docs?x=1#top", false).is_ok());
    assert!(ok("https://github.com/knittingcode/deckpilot-plugins", true).is_ok());
    assert!(ok("https://knittingcode.github.io/deckpilot/", true).is_ok());
    for (url, why) in [
        ("http://acme.example/", "https"),
        ("https://user:pw@acme.example/", "user"),
        ("https://user@acme.example/", "user"),
        ("https://192.168.1.1/", "IP"),
        ("https://[::1]/", "IP"),
        ("https://acme.example:8443/", "port"),
        ("https://xn--80ak6aa92e.com/", "punycode"),
        ("https://ACME.example/", "lowercase"),
        ("https://acme/", "host"),
        ("https://acme.example/a b", "character"),
        ("javascript:alert(1)", "https"),
        ("https://acme.example/\u{202E}", "character"),
    ] {
        let error = ok(url, false).expect_err(url);
        assert!(error.contains(why), "{url}: {error}");
    }
    let long = format!("https://acme.example/{}", "a".repeat(512));
    assert!(ok(&long, false).unwrap_err().contains("512"));
    // First-party listings link to the project only.
    assert!(ok("https://acme.example/", true).is_err());
    assert!(ok("https://github.com/someone-else/repo", true).is_err());
    assert!(ok("https://github.com.evil.example/knittingcode/", true).is_err());
}

#[test]
fn markdown_rules() {
    let ok = |text: &str| check_markdown("description", text, false);
    assert!(
        ok("Plain **bold** _em_ `code`\n\n- one\n- two\n* three\n[docs](https://acme.example/d)")
            .is_ok()
    );
    // Literal angle brackets that are not HTML are fine.
    assert!(ok("a < b and 3<4").is_ok());
    for (text, why) in [
        ("<script>alert(1)</script>", "HTML"),
        ("hi <img src=x onerror=alert(1)>", "HTML"),
        ("close </p>", "HTML"),
        ("<!-- comment -->", "HTML"),
        ("# Heading", "heading"),
        ("### Heading", "heading"),
        ("![alt](https://acme.example/x.png)", "image"),
        ("| a | b |", "table"),
        ("> quote", "quote"),
        ("- one\n  - nested", "nested"),
        ("[x](javascript:alert(1))", "https"),
        ("[x](http://acme.example)", "https"),
        ("[x](https://acme.example", "unterminated"),
    ] {
        let error = ok(text).expect_err(text);
        assert!(error.contains(why), "{text}: {error}");
    }
    // In a first-party listing, links follow the first-party hosts.
    assert!(check_markdown("d", "[x](https://acme.example/)", true).is_err());
    assert!(listing_refused("/description_md", json!({ "en": "<b>bold</b>" })).contains("HTML"));
}

// ========================================================== editorial (R3)

fn banner(id: &str, target: Value) -> Value {
    json!({ "id": id, "title": { "en": "Know before you go" }, "body": { "en": "Weather on your keys." },
            "artwork": { "asset": asset("webp", 2400, 800, 200_000), "alt": { "en": "Rain clouds" } },
            "target": target, "cta": { "en": "See plugin" } })
}

fn editorial_value() -> Value {
    json!({
        "_type": "deckpilot-registry-editorial",
        "editorial_version": 1,
        "sections": [
            { "id": "launch", "kind": "banner_carousel", "order": 10,
              "window": { "starts_at": "2026-12-01T00:00:00Z", "ends_at": "2027-01-15T00:00:00Z" },
              "platforms": ["macos", "windows", "linux"],
              "items": [ banner("weather", json!({ "type": "plugin", "id": "acme.alpha" })),
                         banner("focus", json!({ "type": "collection", "id": "productivity" })) ] },
            { "id": "focus", "kind": "collection_row", "order": 20, "layout": "cards",
              "title": { "en": "Stay focused" }, "plugins": ["acme.alpha", "acme.beta"] },
            { "id": "spot", "kind": "spotlight", "order": 30, "plugin": "acme.alpha", "version": "1.0.0",
              "headline": { "en": "New: Alpha" } },
            { "id": "new", "kind": "recently_added", "order": 40, "limit": 8, "title": { "en": "Recently added" } },
            { "id": "cats", "kind": "category_highlights", "order": 50,
              "categories": [ { "id": "monitoring", "blurb": { "en": "Watch what matters." } } ] },
            { "id": "all", "kind": "catalog", "order": 100, "title": { "en": "All plugins" } }
        ],
        "collections": [
            { "id": "productivity", "order": 10, "title": { "en": "Productivity" }, "plugins": ["acme.alpha"] }
        ]
    })
}

fn editorial_check(value: &Value, strictness: Strictness) -> Result<EditorialDoc, String> {
    let doc = EditorialDoc::parse(&serde_json::to_vec(value).unwrap())?;
    doc.validate(strictness)?;
    Ok(doc)
}

#[test]
fn a_valid_editorial_document_passes() {
    let doc = editorial_check(&editorial_value(), Strictness::Publish).unwrap();
    assert_eq!(doc.sections.len(), 6);
    assert_eq!(doc.skipped.len(), 0);
    assert_eq!(doc.assets().len(), 2);
    // Round trip: what CI writes is what clients read.
    let again = EditorialDoc::parse(&serde_json::to_vec(&doc).unwrap()).unwrap();
    assert_eq!(again, doc);
}

#[test]
fn unknown_section_kinds_are_skipped_not_fatal() {
    let mut value = editorial_value();
    value["sections"].as_array_mut().unwrap().push(json!({
        "id": "quiz", "kind": "quiz_of_the_week", "order": 5, "question": { "en": "?" } }));
    let doc = editorial_check(&value, Strictness::Client).unwrap();
    assert_eq!(doc.sections.len(), 6);
    assert_eq!(
        doc.skipped,
        vec![("quiz".to_owned(), "quiz_of_the_week".to_owned())]
    );
    // A known kind that is malformed refuses the document.
    let mut bad = editorial_value();
    bad["sections"][1]["plugins"] = json!("acme.alpha");
    assert!(editorial_check(&bad, Strictness::Client).is_err());
}

#[test]
fn editorial_rules() {
    let refused = |pointer: &str, new: Value, strictness: Strictness| {
        let mut value = editorial_value();
        set(&mut value, pointer, new);
        editorial_check(&value, strictness)
            .map(|_| ())
            .expect_err(pointer)
    };
    assert!(refused("/sections/0/items", json!([]), Strictness::Client).contains("1 to 6"));
    assert!(
        refused(
            "/sections/0/window/ends_at",
            json!("2026-11-01T00:00:00Z"),
            Strictness::Client
        )
        .contains("window")
    );
    assert!(refused("/sections/1/plugins", json!([]), Strictness::Client).contains("1 to 24"));
    assert!(
        refused(
            "/sections/1/plugins",
            json!(["acme.alpha", "acme.alpha"]),
            Strictness::Client
        )
        .contains("all different")
    );
    assert!(refused("/sections/3/limit", json!(3), Strictness::Client).contains("limit"));
    assert!(refused("/sections/3/limit", json!(25), Strictness::Client).contains("limit"));
    assert!(refused("/sections/4/categories", json!([]), Strictness::Client).contains("1 to 12"));
    assert!(refused("/sections/2/id", json!("launch"), Strictness::Client).contains("twice"));
    assert!(refused("/sections/2/id", json!("Bad Id"), Strictness::Client).contains("a-z"));
    assert!(
        refused(
            "/sections/0/items/0/artwork/alt",
            json!({}),
            Strictness::Client
        )
        .contains("no `en`")
    );
    assert!(
        refused(
            "/sections/0/items/0/artwork/asset",
            asset("png", 1200, 1200, 1000),
            Strictness::Client
        )
        .contains("3:1")
    );
    // Publishing only: target types, layouts, platforms, collection targets.
    let target = json!({ "type": "url", "id": "https://evil.example/" });
    assert!(
        refused(
            "/sections/0/items/0/target",
            target.clone(),
            Strictness::Publish
        )
        .contains("target.type")
    );
    let mut lenient = editorial_value();
    set(&mut lenient, "/sections/0/items/0/target", target);
    editorial_check(&lenient, Strictness::Client).unwrap();
    assert!(
        refused(
            "/sections/0/items/1/target/id",
            json!("nope"),
            Strictness::Publish
        )
        .contains("collection")
    );
    assert!(refused("/sections/1/layout", json!("hero"), Strictness::Publish).contains("layout"));
    // Decorative art has no alt text; an empty alt is fine then.
    let mut decorative = editorial_value();
    set(
        &mut decorative,
        "/sections/0/items/0/artwork/decorative",
        json!(true),
    );
    set(
        &mut decorative,
        "/sections/0/items/0/artwork/alt",
        json!({}),
    );
    editorial_check(&decorative, Strictness::Publish).unwrap();
    // At most 12 sections.
    let mut many = editorial_value();
    let sections: Vec<Value> = (0..13)
        .map(|i| json!({ "id": format!("s{i}"), "kind": "catalog", "order": i }))
        .collect();
    set(&mut many, "/sections", json!(sections));
    assert!(
        editorial_check(&many, Strictness::Client)
            .unwrap_err()
            .contains("12")
    );
}

#[test]
fn windows() {
    let window = Window {
        starts_at: Some(t0()),
        ends_at: Some(t0() + days(2)),
    };
    assert!(!window.contains(t0() - days(1)));
    assert!(window.contains(t0()));
    assert!(window.contains(t0() + days(1)));
    assert!(!window.contains(t0() + days(2)));
    assert!(Window::default().contains(t0()));
}

// ================================================ client verification (R4)

/// A storefront published beside `registry`'s current index.
struct Published {
    envelope: Vec<u8>,
    signature: String,
    listing: Vec<u8>,
    editorial: Vec<u8>,
}

fn listing_doc_bytes(id: &str) -> (Vec<u8>, Vec<u8>) {
    let icon = real_png(256, 256);
    let doc = json!({
        "_type": "deckpilot-registry-listing", "listing_version": 1, "id": id,
        "promo": { "en": "A clock on a key" },
        "icon": serde_json::to_value(asset_of(&icon)).unwrap(),
    });
    (serde_json::to_vec_pretty(&doc).unwrap(), icon)
}

fn editorial_doc_bytes() -> Vec<u8> {
    serde_json::to_vec_pretty(&json!({
        "_type": "deckpilot-registry-editorial", "editorial_version": 1,
        "sections": [ { "id": "row", "kind": "collection_row", "order": 1, "title": { "en": "Clocks" }, "plugins": ["acme.clock"] },
                      { "id": "future", "kind": "hologram", "order": 2 } ]
    }))
    .unwrap()
}

fn envelope_value(registry: &Registry, seq: u64, listing: &[u8], editorial: &[u8]) -> Value {
    let index = registry.site.get(&format!("index/{seq}.json"));
    let listing_sha = sha256(listing);
    let editorial_sha = sha256(editorial);
    json!({
        "_type": "deckpilot-registry-storefront",
        "spec_version": 1,
        "storefront_version": 1,
        "seq": seq,
        "keys_version": registry.keys_version.get(),
        "generated_at": "2026-11-14T12:00:00Z",
        "source_commit": "a".repeat(40),
        "index": { "path": format!("index/{seq}.json"), "sha256": sha256(&index).to_string(), "length": index.len() },
        "listings": [ { "id": "acme.clock", "ref": {
            "path": format!("storefront/listings/acme.clock/{listing_sha}.json"),
            "sha256": listing_sha.to_string(), "length": listing.len() } } ],
        "editorial": { "path": format!("storefront/editorial/{editorial_sha}.json"),
                       "sha256": editorial_sha.to_string(), "length": editorial.len() }
    })
}

fn publish_storefront(registry: &Registry, seq: u64) -> Published {
    let (listing, icon) = listing_doc_bytes("acme.clock");
    let editorial = editorial_doc_bytes();
    let envelope =
        serde_json::to_vec_pretty(&envelope_value(registry, seq, &listing, &editorial)).unwrap();
    let signature = registry.index_key.sign(
        &envelope,
        &storefront_comment(seq, registry.keys_version.get()),
    );
    registry
        .site
        .put(&format!("storefront/{seq}.json"), envelope.clone());
    registry
        .site
        .put(&format!("storefront/{seq}.json.minisig"), signature.clone());
    registry.site.put(
        &format!("storefront/listings/acme.clock/{}.json", sha256(&listing)),
        listing.clone(),
    );
    registry.site.put(
        &format!("storefront/editorial/{}.json", sha256(&editorial)),
        editorial.clone(),
    );
    registry
        .site
        .put(&format!("media/{}.png", sha256(&icon)), icon);
    Published {
        envelope,
        signature,
        listing,
        editorial,
    }
}

fn sign_envelope(registry: &Registry, value: &Value, comment: &str) -> (Vec<u8>, String) {
    let bytes = serde_json::to_vec_pretty(value).unwrap();
    let signature = registry.index_key.sign(&bytes, comment);
    (bytes, signature)
}

fn registry_with_clock() -> (Registry, Client) {
    let mut registry = Registry::new();
    registry.publish(t0(), &[plugin("acme.clock", &["1.0.0"])], &[]);
    let mut client = client(&registry);
    assert_eq!(
        client.refresh(&registry.site, t0()),
        Ok(Refresh::Updated { seq: 1 })
    );
    (registry, client)
}

#[test]
fn a_signed_storefront_is_accepted_with_its_documents() {
    let (registry, mut client) = registry_with_clock();
    let published = publish_storefront(&registry, 1);
    let verified = client
        .verify_storefront(&published.envelope, published.signature.as_bytes())
        .unwrap();
    assert_eq!(verified.seq(), 1);
    assert_eq!(*verified.sha256(), sha256(&published.envelope));
    let listing = client
        .verify_listing(&verified, &id("acme.clock"), &published.listing)
        .unwrap();
    assert_eq!(
        localize(listing.promo.as_ref().unwrap(), "pl-PL"),
        Some("A clock on a key")
    );
    let icon = listing.icon.unwrap();
    let icon_bytes = registry.site.get(&media_path(&icon));
    assert!(verify_media(&icon, &icon_bytes).is_ok());
    let editorial = client
        .verify_editorial(&verified, &published.editorial)
        .unwrap();
    assert_eq!(editorial.sections.len(), 1);
    assert_eq!(
        editorial.skipped,
        vec![("future".to_owned(), "hologram".to_owned())]
    );
    // Recorded for the equivocation check, beside the trust state.
    let record = client.store().load_storefront_record().unwrap().unwrap();
    assert_eq!(
        record,
        StorefrontRecord {
            seq: 1,
            sha256: sha256(&published.envelope)
        }
    );
    // Verifying the same bytes again (a cache re-read) is fine.
    client
        .verify_storefront(&published.envelope, published.signature.as_bytes())
        .unwrap();
}

#[test]
fn an_index_signature_is_not_a_storefront_signature_and_the_reverse() {
    let (mut registry, mut client) = registry_with_clock();
    let published = publish_storefront(&registry, 1);
    // The index key, the index comment: refused as a storefront.
    let as_index = registry
        .index_key
        .sign(&published.envelope, &index_comment(1, 1));
    assert!(matches!(
        client.verify_storefront(&published.envelope, as_index.as_bytes()),
        Err(StorefrontError::WrongRole(_))
    ));
    // The index's own signature over the index, presented with a storefront.
    let index_signature = registry.site.get("index/1.json.minisig");
    assert!(matches!(
        client.verify_storefront(&published.envelope, &index_signature),
        Err(StorefrontError::WrongRole(_))
    ));
    // And a storefront signature never passes as an index signature.
    let seq = 2;
    let index = registry.index_doc(
        seq,
        t0() + days(1),
        &[plugin("acme.clock", &["1.0.0"])],
        &[],
    );
    let bytes = serde_json::to_vec_pretty(&index).unwrap();
    registry.site.put("index/2.json", bytes.clone());
    registry.site.put(
        "index/2.json.minisig",
        registry.index_key.sign(&bytes, &storefront_comment(2, 1)),
    );
    registry.seq = 2;
    registry.sign_timestamp(t0() + days(1));
    let error = client.refresh(&registry.site, t0() + days(1)).unwrap_err();
    assert!(matches!(error, RegistryError::IndexSignature(_)), "{error}");
    assert_eq!(client.state().seq, 1);
}

#[test]
fn only_the_index_role_signs_storefronts() {
    let (registry, mut client) = registry_with_clock();
    let published = publish_storefront(&registry, 1);
    let comment = storefront_comment(1, 1);
    for signer in [
        &registry.timestamp_key,
        &registry.roots[0],
        &ThrowawayKey::generate(),
    ] {
        let signature = signer.sign(&published.envelope, &comment);
        assert!(matches!(
            client.verify_storefront(&published.envelope, signature.as_bytes()),
            Err(StorefrontError::Signature(_))
        ));
    }
    // A timestamp-role comment is not a storefront comment either.
    let signature = registry
        .timestamp_key
        .sign(&published.envelope, &timestamp_comment(1, 1));
    assert!(matches!(
        client.verify_storefront(&published.envelope, signature.as_bytes()),
        Err(StorefrontError::WrongRole(_))
    ));
}

#[test]
fn the_storefront_is_bound_to_the_verified_seq_key_set_and_index() {
    let (registry, mut client) = registry_with_clock();
    let (listing, _) = listing_doc_bytes("acme.clock");
    let editorial = editorial_doc_bytes();
    let good = envelope_value(&registry, 1, &listing, &editorial);
    // Signed for another seq: the comment does not match.
    let (bytes, signature) = sign_envelope(&registry, &good, &storefront_comment(2, 1));
    assert!(matches!(
        client.verify_storefront(&bytes, signature.as_bytes()),
        Err(StorefrontError::Signature(_))
    ));
    // Right comment, content claims another seq.
    let mut other_seq = good.clone();
    other_seq["seq"] = json!(2);
    other_seq["index"]["path"] = json!("index/2.json");
    let (bytes, signature) = sign_envelope(&registry, &other_seq, &storefront_comment(1, 1));
    assert!(matches!(
        client.verify_storefront(&bytes, signature.as_bytes()),
        Err(StorefrontError::Seq { .. })
    ));
    // Stale key set.
    let mut stale = good.clone();
    stale["keys_version"] = json!(7);
    let (bytes, signature) = sign_envelope(&registry, &stale, &storefront_comment(1, 1));
    assert!(matches!(
        client.verify_storefront(&bytes, signature.as_bytes()),
        Err(StorefrontError::KeysVersion { .. })
    ));
    // Another index (sha or length).
    let mut other_index = good.clone();
    other_index["index"]["sha256"] = json!("0".repeat(64));
    let (bytes, signature) = sign_envelope(&registry, &other_index, &storefront_comment(1, 1));
    assert!(matches!(
        client.verify_storefront(&bytes, signature.as_bytes()),
        Err(StorefrontError::IndexMismatch(_))
    ));
    let mut other_length = good.clone();
    other_length["index"]["length"] = json!(1);
    let (bytes, signature) = sign_envelope(&registry, &other_length, &storefront_comment(1, 1));
    assert!(matches!(
        client.verify_storefront(&bytes, signature.as_bytes()),
        Err(StorefrontError::IndexMismatch(_))
    ));
    // A listing for a plugin the index does not have.
    let mut unknown = good.clone();
    let path = format!("storefront/listings/acme.ghost/{}.json", sha256(&listing));
    unknown["listings"][0]["id"] = json!("acme.ghost");
    unknown["listings"][0]["ref"]["path"] = json!(path);
    let (bytes, signature) = sign_envelope(&registry, &unknown, &storefront_comment(1, 1));
    assert!(matches!(
        client.verify_storefront(&bytes, signature.as_bytes()),
        Err(StorefrontError::UnknownPlugin(_))
    ));
    // A reference that is not content-addressed.
    let mut moved = good.clone();
    moved["listings"][0]["ref"]["path"] = json!("storefront/listings/acme.clock/latest.json");
    let (bytes, signature) = sign_envelope(&registry, &moved, &storefront_comment(1, 1));
    assert!(matches!(
        client.verify_storefront(&bytes, signature.as_bytes()),
        Err(StorefrontError::Malformed { .. })
    ));
    let mut twice = good.clone();
    let entry = twice["listings"][0].clone();
    twice["listings"].as_array_mut().unwrap().push(entry);
    let (bytes, signature) = sign_envelope(&registry, &twice, &storefront_comment(1, 1));
    assert!(matches!(
        client.verify_storefront(&bytes, signature.as_bytes()),
        Err(StorefrontError::Malformed { .. })
    ));
    // The envelope refuses unknown fields; a future version is reported.
    let mut extra = good.clone();
    extra["featured"] = json!(["acme.clock"]);
    let (bytes, signature) = sign_envelope(&registry, &extra, &storefront_comment(1, 1));
    let error = client
        .verify_storefront(&bytes, signature.as_bytes())
        .unwrap_err();
    assert!(
        matches!(&error, StorefrontError::Malformed { message, .. } if message.contains("unknown field")),
        "{error}"
    );
    let mut future = good.clone();
    future["storefront_version"] = json!(2);
    let (bytes, signature) = sign_envelope(&registry, &future, &storefront_comment(1, 1));
    assert!(matches!(
        client.verify_storefront(&bytes, signature.as_bytes()),
        Err(StorefrontError::UnsupportedVersion { .. })
    ));
    // Too large.
    let big = vec![b' '; MAX_STOREFRONT_BYTES + 1];
    let signature = registry.index_key.sign(&big, &storefront_comment(1, 1));
    assert!(matches!(
        client.verify_storefront(&big, signature.as_bytes()),
        Err(StorefrontError::TooLarge { .. })
    ));
    // And the good one still passes after all those refusals.
    let (bytes, signature) = sign_envelope(&registry, &good, &storefront_comment(1, 1));
    client
        .verify_storefront(&bytes, signature.as_bytes())
        .unwrap();
}

#[test]
fn two_storefronts_for_one_seq_are_equivocation() {
    let (registry, mut client) = registry_with_clock();
    let published = publish_storefront(&registry, 1);
    client
        .verify_storefront(&published.envelope, published.signature.as_bytes())
        .unwrap();
    let mut other = envelope_value(&registry, 1, &published.listing, &published.editorial);
    other["generated_at"] = json!("2026-11-14T13:00:00Z");
    let (bytes, signature) = sign_envelope(&registry, &other, &storefront_comment(1, 1));
    assert_eq!(
        client.verify_storefront(&bytes, signature.as_bytes()),
        Err(StorefrontError::Equivocation { seq: 1 })
    );
    // Also after a restart (the record is persisted).
    let mut restarted = restart(&registry, &client);
    assert_eq!(
        restarted.verify_storefront(&bytes, signature.as_bytes()),
        Err(StorefrontError::Equivocation { seq: 1 })
    );
    restarted
        .verify_storefront(&published.envelope, published.signature.as_bytes())
        .unwrap();
}

#[test]
fn equivocation_survives_a_restart_with_the_file_store() {
    let dir = TempDir::new().unwrap();
    let mut registry = Registry::new();
    registry.publish(t0(), &[plugin("acme.clock", &["1.0.0"])], &[]);
    let published = publish_storefront(&registry, 1);
    {
        let mut client = file_client(&registry, dir.path());
        client.refresh(&registry.site, t0()).unwrap();
        client
            .verify_storefront(&published.envelope, published.signature.as_bytes())
            .unwrap();
    }
    assert!(dir.path().join(FileTrustStore::STOREFRONT_FILE).is_file());
    let mut other = envelope_value(&registry, 1, &published.listing, &published.editorial);
    other["generated_at"] = json!("2026-11-15T00:00:00Z");
    let (bytes, signature) = sign_envelope(&registry, &other, &storefront_comment(1, 1));
    let mut client = file_client(&registry, dir.path());
    assert_eq!(
        client.verify_storefront(&bytes, signature.as_bytes()),
        Err(StorefrontError::Equivocation { seq: 1 })
    );
}

#[test]
fn an_older_seqs_storefront_is_not_shown_after_a_newer_index() {
    let (mut registry, mut client) = registry_with_clock();
    let first = publish_storefront(&registry, 1);
    let verified = client
        .verify_storefront(&first.envelope, first.signature.as_bytes())
        .unwrap();
    registry.publish(
        t0() + days(1),
        &[plugin("acme.clock", &["1.0.0", "1.1.0"])],
        &[],
    );
    assert_eq!(
        client.refresh(&registry.site, t0() + days(1)),
        Ok(Refresh::Updated { seq: 2 })
    );
    // The old envelope with its valid (seq 1) signature.
    assert!(matches!(
        client.verify_storefront(&first.envelope, first.signature.as_bytes()),
        Err(StorefrontError::Signature(_))
    ));
    // A storefront verified before is no longer usable.
    assert!(matches!(
        client.verify_listing(&verified, &id("acme.clock"), &first.listing),
        Err(StorefrontError::Seq { .. })
    ));
    assert!(matches!(
        client.verify_editorial(&verified, &first.editorial),
        Err(StorefrontError::Seq { .. })
    ));
    // The new seq's storefront is accepted (and recorded over the old one).
    let second = publish_storefront(&registry, 2);
    client
        .verify_storefront(&second.envelope, second.signature.as_bytes())
        .unwrap();
    assert_eq!(
        client
            .store()
            .load_storefront_record()
            .unwrap()
            .unwrap()
            .seq,
        2
    );
}

#[test]
fn a_refused_storefront_never_changes_the_refresh_or_the_trust_state() {
    let mut registry = Registry::new();
    registry.publish(t0(), &[plugin("acme.clock", &["1.0.0"])], &[]);
    // Garbage where the storefront would be.
    registry
        .site
        .put("storefront/1.json", b"{ not json".to_vec());
    registry
        .site
        .put("storefront/1.json.minisig", b"nope".to_vec());
    let mut client = client(&registry);
    assert_eq!(
        client.refresh(&registry.site, t0()),
        Ok(Refresh::Updated { seq: 1 })
    );
    assert_eq!(client.freshness(t0()), Freshness::Fresh);
    let before = client.state().clone();
    let index_before = client.index().cloned();
    assert!(client.verify_storefront(b"{ not json", b"nope").is_err());
    let published = publish_storefront(&registry, 1);
    let tampered = {
        let mut bytes = published.envelope.clone();
        bytes[10] ^= 1;
        bytes
    };
    assert!(
        client
            .verify_storefront(&tampered, published.signature.as_bytes())
            .is_err()
    );
    assert_eq!(client.state(), &before);
    assert_eq!(client.index().cloned(), index_before);
    assert_eq!(client.store().load_storefront_record().unwrap(), None);
    // Installs still resolve.
    let detail = registry
        .site
        .get(&client.detail_ref(&id("acme.clock")).unwrap().path);
    assert!(
        client
            .resolve_install(&id("acme.clock"), &v("1.0.0"), &detail, None, t0())
            .is_ok()
    );
    // The next refresh is unaffected too.
    assert_eq!(
        client.refresh(&registry.site, t0() + days(1)),
        Ok(Refresh::Unchanged { seq: 1 })
    );
}

#[test]
fn pinned_documents_are_checked_before_parsing() {
    let (registry, mut client) = registry_with_clock();
    let published = publish_storefront(&registry, 1);
    let verified = client
        .verify_storefront(&published.envelope, published.signature.as_bytes())
        .unwrap();
    let clock = id("acme.clock");
    let mut tampered = published.listing.clone();
    tampered[5] ^= 1;
    assert!(matches!(
        client.verify_listing(&verified, &clock, &tampered),
        Err(StorefrontError::Mismatch { .. })
    ));
    assert!(matches!(
        client.verify_listing(&verified, &clock, &published.listing[1..]),
        Err(StorefrontError::Mismatch { .. })
    ));
    assert!(matches!(
        client.verify_listing(&verified, &id("acme.other"), &published.listing),
        Err(StorefrontError::NoListing(_))
    ));
    let mut editorial = published.editorial.clone();
    editorial.push(b'\n');
    assert!(matches!(
        client.verify_editorial(&verified, &editorial),
        Err(StorefrontError::Mismatch { .. })
    ));
}

#[test]
fn listing_and_editorial_versions_refuse_only_that_document() {
    let (registry, mut client) = registry_with_clock();
    let listing = serde_json::to_vec(
        &json!({ "_type": "deckpilot-registry-listing", "listing_version": 2, "id": "acme.clock" }),
    )
    .unwrap();
    let editorial = serde_json::to_vec(
        &json!({ "_type": "deckpilot-registry-editorial", "editorial_version": 2, "sections": [] }),
    )
    .unwrap();
    let value = envelope_value(&registry, 1, &listing, &editorial);
    let (bytes, signature) = sign_envelope(&registry, &value, &storefront_comment(1, 1));
    let verified = client
        .verify_storefront(&bytes, signature.as_bytes())
        .unwrap();
    assert!(matches!(
        client.verify_listing(&verified, &id("acme.clock"), &listing),
        Err(StorefrontError::UnsupportedVersion { .. })
    ));
    assert!(matches!(
        client.verify_editorial(&verified, &editorial),
        Err(StorefrontError::UnsupportedVersion { .. })
    ));
}

#[test]
fn a_listing_for_another_plugin_is_refused() {
    let (registry, mut client) = registry_with_clock();
    let (listing, _) = listing_doc_bytes("acme.other");
    let editorial = editorial_doc_bytes();
    let value = envelope_value(&registry, 1, &listing, &editorial);
    let (bytes, signature) = sign_envelope(&registry, &value, &storefront_comment(1, 1));
    let verified = client
        .verify_storefront(&bytes, signature.as_bytes())
        .unwrap();
    assert!(matches!(
        client.verify_listing(&verified, &id("acme.clock"), &listing),
        Err(StorefrontError::Mismatch { .. })
    ));
}

#[test]
fn the_trust_state_is_unchanged_and_tolerates_no_storefront_record() {
    let dir = TempDir::new().unwrap();
    let mut registry = Registry::new();
    registry.publish(t0(), &[plugin("acme.clock", &["1.0.0"])], &[]);
    let published = publish_storefront(&registry, 1);
    let mut client = file_client(&registry, dir.path());
    client.refresh(&registry.site, t0()).unwrap();
    client
        .verify_storefront(&published.envelope, published.signature.as_bytes())
        .unwrap();
    // registry-trust.json has exactly the fields DeckPilot 0.10 to 0.12
    // wrote (they refuse unknown fields after a downgrade).
    let state: Value = serde_json::from_slice(
        &std::fs::read(dir.path().join(FileTrustStore::STATE_FILE)).unwrap(),
    )
    .unwrap();
    let mut keys: Vec<&str> = state
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "clock_floor",
            "format",
            "keyset",
            "revocations",
            "revocations_resettable",
            "roots",
            "seq",
            "timestamp"
        ]
    );
    // A state directory from before storefronts (no record file) opens and
    // verifies; the record is simply created.
    std::fs::remove_file(dir.path().join(FileTrustStore::STOREFRONT_FILE)).unwrap();
    let mut client = file_client(&registry, dir.path());
    assert_eq!(client.store().load_storefront_record().unwrap(), None);
    client
        .verify_storefront(&published.envelope, published.signature.as_bytes())
        .unwrap();
    // A corrupt record refuses the storefront only.
    std::fs::write(dir.path().join(FileTrustStore::STOREFRONT_FILE), b"{").unwrap();
    let mut client = file_client(&registry, dir.path());
    assert!(matches!(
        client.verify_storefront(&published.envelope, published.signature.as_bytes()),
        Err(StorefrontError::Store(StoreError::Corrupt(_)))
    ));
    assert_eq!(
        client.refresh(&registry.site, t0()),
        Ok(Refresh::Unchanged { seq: 1 })
    );
}

// ============================================== the publishing tool (R1, R4)

fn time(at: DateTime<Utc>) -> String {
    at.to_rfc3339()
}

struct Fixture {
    work: TempDir,
    keys: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            work: TempDir::new().unwrap(),
            keys: TempDir::new().unwrap(),
        }
    }

    fn dir(&self) -> &Path {
        self.work.path()
    }

    fn ws(&self) -> Workspace {
        Workspace::new(self.dir())
    }

    fn site(&self) -> PathBuf {
        self.ws().site()
    }

    fn key(&self, name: &str) -> String {
        self.keys.path().join(name).display().to_string()
    }

    fn try_run_in(&self, dir: &Path, args: &[&str]) -> Result<String, String> {
        let mut all: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        if args.first().is_none_or(|c| *c != "keys") {
            all.push("--dir".into());
            all.push(dir.display().to_string());
        }
        let env = |_: &str| None;
        let mut input: &[u8] = b"";
        let mut out = Vec::new();
        let mut io = Io {
            env: &env,
            stdin: &mut input,
            out: &mut out,
        };
        run(&all, &mut io).map_err(|e| e.to_string())?;
        Ok(String::from_utf8(out).unwrap())
    }

    fn try_run(&self, args: &[&str]) -> Result<String, String> {
        self.try_run_in(self.dir(), args)
    }

    fn run(&self, args: &[&str]) -> String {
        self.try_run(args)
            .unwrap_or_else(|e| panic!("{args:?} failed: {e}"))
    }

    fn fails(&self, args: &[&str]) -> String {
        match self.try_run(args) {
            Ok(out) => panic!("{args:?} succeeded: {out}"),
            Err(e) => e,
        }
    }

    fn write(&self, path: &str, bytes: impl AsRef<[u8]>) {
        let path = self.dir().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn write_json(&self, path: &str, value: &Value) {
        self.write(path, serde_json::to_vec_pretty(value).unwrap());
    }

    fn add_version(&self, plugin: &str, version: &str) {
        let package = self
            .keys
            .path()
            .join(format!("{plugin}-{version}.dpplugin"));
        std::fs::write(&package, common::package(plugin, version).bytes).unwrap();
        let detail = self.keys.path().join(format!("{plugin}.plugin.json"));
        std::fs::write(
            &detail,
            serde_json::to_vec_pretty(&json!({
                "id": plugin, "name": format!("Plugin {plugin}"), "summary": "A test plugin",
                "description": "A test plugin.", "license": "MIT", "categories": ["developer-tools"]
            }))
            .unwrap(),
        )
        .unwrap();
        let commit = "1".repeat(40);
        let now = time(t0() - days(1));
        self.run(&[
            "add-version",
            "--package",
            &package.display().to_string(),
            "--plugin-detail",
            &detail.display().to_string(),
            "--source-repository",
            "someone/test-plugin",
            "--tag",
            &format!("v{version}"),
            "--commit",
            &commit,
            "--builder",
            "1.0.0",
            "--now",
            &now,
        ]);
    }

    fn build_and_sign_index(&self, at: DateTime<Utc>) {
        self.run(&[
            "build-index",
            "--source-commit",
            &"a".repeat(40),
            "--now",
            &time(at),
        ]);
        self.run(&["sign-index", "--key", &self.key("index.key")]);
    }

    fn storefront(&self, at: DateTime<Utc>) -> String {
        let built = self.run(&["storefront", "build", "--now", &time(at)]);
        self.run(&["storefront", "sign", "--key", &self.key("index.key")]);
        built
    }

    fn timestamp(&self, at: DateTime<Utc>) {
        self.run(&[
            "timestamp",
            "--key",
            &self.key("timestamp.key"),
            "--now",
            &time(at),
        ]);
    }

    fn verify(&self, extra: &[&str], at: DateTime<Utc>) -> Result<String, String> {
        let now = time(at);
        let mut args = vec!["verify-site", "--now", &now];
        args.extend_from_slice(extra);
        self.try_run(&args)
    }

    fn read_site(&self, path: &str) -> Vec<u8> {
        std::fs::read(self.site().join(path)).unwrap()
    }
}

fn base_fixture() -> Fixture {
    let fx = Fixture::new();
    for (role, name) in [
        ("root", "root.key"),
        ("index", "index.key"),
        ("timestamp", "timestamp.key"),
    ] {
        fx.run(&[
            "keys",
            "generate",
            "--role",
            role,
            "--out",
            &fx.key(name),
            "--no-password",
        ]);
    }
    fx.run(&[
        "keyset",
        "init",
        "--root",
        &fx.key("root.key.pub"),
        "--index",
        &fx.key("index.key.pub"),
        "--timestamp",
        &fx.key("timestamp.key.pub"),
    ]);
    fx.run(&["keyset", "sign", "--key", &fx.key("root.key")]);
    fx.run(&[
        "publisher",
        "--namespace",
        "acme",
        "--github-id",
        "1001",
        "--login",
        "acme",
        "--verified",
    ]);
    fx.add_version("acme.alpha", "1.0.0");
    fx.add_version("acme.alpha", "1.1.0");
    fx.add_version("acme.beta", "0.1.0");
    fx
}

fn source_listing() -> Value {
    json!({
        "$schema": "../../../schemas/inputs/listing.schema.json",
        "listing_version": 1,
        "id": "acme.alpha",
        "promo": { "en": "Alpha on your deck", "pl": "Alfa na Twoim decku" },
        "description_md": { "en": "Shows **alpha**.\n\n- Fast\n- [Docs](https://acme.example/docs)" },
        "highlights": [ { "title": { "en": "Fast" }, "text": { "en": "Very fast." } } ],
        "icon": { "file": "media/icon.png" },
        "screenshots": [
            { "file": "media/main.png", "alt": { "en": "The main page" }, "caption": { "en": "Main" } },
            { "file": "media/mac.webp", "alt": { "en": "On macOS" }, "platforms": ["macos"] }
        ],
        "promo_art": { "file": "media/promo.jpg", "alt": { "en": "Alpha art" } },
        "links": { "documentation": "https://acme.example/docs", "support": "https://acme.example/help" },
        "release_notes": { "1.1.0": { "en": "Faster." } }
    })
}

fn source_editorial() -> Value {
    json!({
        "$schema": "../schemas/inputs/editorial.schema.json",
        "editorial_version": 1,
        "sections": [
            { "id": "launch", "status": "published", "kind": "banner_carousel", "order": 10,
              "items": [ { "id": "alpha", "title": { "en": "Meet Alpha" },
                           "artwork": { "file": "media/banner.png", "alt": { "en": "Alpha banner" } },
                           "target": { "type": "plugin", "id": "acme.alpha" }, "cta": { "en": "See plugin" } } ] },
            { "id": "focus", "status": "published", "kind": "collection_row", "order": 20, "layout": "cards",
              "title": { "en": "Stay focused" }, "plugins": ["acme.alpha", "acme.beta"] },
            { "id": "secret", "status": "draft", "kind": "spotlight", "order": 30, "plugin": "acme.beta",
              "headline": { "en": "Coming soon: Beta 2" } },
            { "id": "old", "status": "archived", "kind": "catalog", "order": 40 },
            { "id": "all", "status": "published", "kind": "catalog", "order": 100, "title": { "en": "All plugins" } }
        ],
        "collections": [
            { "id": "productivity", "status": "published", "order": 10, "title": { "en": "Productivity" }, "plugins": ["acme.alpha"] },
            { "id": "draft-col", "status": "draft", "order": 20, "title": { "en": "Hidden" }, "plugins": ["acme.beta"] }
        ]
    })
}

fn with_sources(fx: &Fixture) {
    fx.write_json(
        "registry/plugins/acme.alpha/listing.json",
        &source_listing(),
    );
    fx.write(
        "registry/plugins/acme.alpha/media/icon.png",
        real_png(256, 256),
    );
    fx.write(
        "registry/plugins/acme.alpha/media/main.png",
        real_png(1280, 800),
    );
    fx.write(
        "registry/plugins/acme.alpha/media/mac.webp",
        real_webp(1280, 720),
    );
    fx.write(
        "registry/plugins/acme.alpha/media/promo.jpg",
        real_jpeg(1600, 800),
    );
    fx.write_json("editorial/editorial.json", &source_editorial());
    fx.write("editorial/media/banner.png", real_png(1200, 400));
}

/// Sources, index, storefront, timestamp: a full publish at t0.
fn published_fixture() -> Fixture {
    let fx = base_fixture();
    with_sources(&fx);
    fx.build_and_sign_index(t0());
    fx.storefront(t0());
    fx.timestamp(t0());
    fx
}

#[test]
fn check_build_sign_and_verify_site_with_a_storefront() {
    let fx = base_fixture();
    with_sources(&fx);
    let checked = fx.run(&["storefront", "check"]);
    assert!(
        checked.contains("1 listing(s), 3 published section(s), 1 collection(s), 5 image(s)"),
        "{checked}"
    );
    fx.build_and_sign_index(t0());
    let built = fx.storefront(t0());
    assert!(
        built.contains("1 listing(s), editorial yes, 5 image(s)"),
        "{built}"
    );
    fx.timestamp(t0());
    let out = fx
        .verify(&["--require-storefront", "--require-packages"], t0())
        .unwrap();
    assert!(
        out.contains("storefront accepted by the client: seq 1"),
        "{out}"
    );
    assert!(
        out.contains(
            "1 listing(s), 3 section(s) (0 of an unknown kind skipped), 5 image(s) verified"
        ),
        "{out}"
    );
    // Drafts and archived entries never reach the site; nor does `status`.
    let storefront: Storefront =
        serde_json::from_slice(&fx.read_site("storefront/1.json")).unwrap();
    let editorial = fx.read_site(&storefront.editorial.as_ref().unwrap().path);
    let text = String::from_utf8(editorial.clone()).unwrap();
    for absent in [
        "secret",
        "Coming soon",
        "draft-col",
        "Hidden",
        "\"old\"",
        "status",
        "media/banner.png",
    ] {
        assert!(!text.contains(absent), "{absent} in {text}");
    }
    // Media are content-addressed copies of the sources.
    let listing: ListingDoc =
        serde_json::from_slice(&fx.read_site(&storefront.listings[0].reference.path)).unwrap();
    let icon = listing.icon.unwrap();
    assert_eq!(fx.read_site(&media_path(&icon)), real_png(256, 256));
    assert_eq!(icon.format, MediaFormat::Png);
    assert_eq!((icon.width, icon.height), (256, 256));
    assert_eq!(listing.screenshots[1].asset.format, MediaFormat::Webp);
    assert_eq!(
        listing.promo_art.as_ref().unwrap().asset.format,
        MediaFormat::Jpeg
    );
    // The signature is the index key's, with the storefront comment.
    let keyset: Keyset = serde_json::from_slice(&fx.read_site("keys/1.json")).unwrap();
    let signature = String::from_utf8(fx.read_site("storefront/1.json.minisig")).unwrap();
    sig::verify_role(
        &keyset.index,
        &[signature.trim().to_owned()],
        &fx.read_site("storefront/1.json"),
        &storefront_comment(1, 1),
    )
    .unwrap();
    assert!(
        sig::verify_role(
            &keyset.index,
            &[signature.trim().to_owned()],
            &fx.read_site("storefront/1.json"),
            &index_comment(1, 1)
        )
        .is_err()
    );
}

#[test]
fn every_built_document_matches_its_schema() {
    let fx = published_fixture();
    let storefront_bytes = fx.read_site("storefront/1.json");
    assert_eq!(
        schema_errors(&validator(schemas::STOREFRONT), &storefront_bytes),
        Vec::<String>::new()
    );
    let storefront: Storefront = serde_json::from_slice(&storefront_bytes).unwrap();
    for entry in &storefront.listings {
        let bytes = fx.read_site(&entry.reference.path);
        assert_eq!(
            schema_errors(&validator(schemas::LISTING), &bytes),
            Vec::<String>::new()
        );
    }
    let editorial = fx.read_site(&storefront.editorial.unwrap().path);
    assert_eq!(
        schema_errors(&validator(schemas::EDITORIAL), &editorial),
        Vec::<String>::new()
    );
    // The sources match theirs.
    let listing = serde_json::to_vec(&source_listing()).unwrap();
    assert_eq!(
        schema_errors(&validator(schemas::LISTING_SOURCE), &listing),
        Vec::<String>::new()
    );
    let editorial = serde_json::to_vec(&source_editorial()).unwrap();
    assert_eq!(
        schema_errors(&validator(schemas::EDITORIAL_SOURCE), &editorial),
        Vec::<String>::new()
    );
    // And the hand-written fixtures of the rule tests.
    for value in [
        listing_value("deckpilot.weather"),
        listing_value("acme.weather"),
    ] {
        assert_eq!(
            schema_errors(
                &validator(schemas::LISTING),
                &serde_json::to_vec(&value).unwrap()
            ),
            Vec::<String>::new()
        );
    }
    assert_eq!(
        schema_errors(
            &validator(schemas::EDITORIAL),
            &serde_json::to_vec(&editorial_value()).unwrap()
        ),
        Vec::<String>::new()
    );
}

/// The envelope: the schema and the strict Rust model refuse the same
/// mutations of a built document.
#[test]
fn the_envelope_schema_and_the_model_agree() {
    let fx = published_fixture();
    let bytes = fx.read_site("storefront/1.json");
    let schema = validator(schemas::STOREFRONT);
    let ours = |bytes: &[u8]| {
        serde_json::from_slice::<Storefront>(bytes)
            .map_err(|e| e.to_string())
            .and_then(|doc| doc.validate())
            .is_ok()
    };
    assert!(ours(&bytes));
    let original: Value = serde_json::from_slice(&bytes).unwrap();
    let mutations: Vec<(&str, Option<Value>)> = vec![
        ("/featured", Some(json!(["acme.alpha"]))),
        ("/_type", Some(json!("deckpilot-registry-index"))),
        ("/spec_version", Some(json!(2))),
        ("/storefront_version", Some(json!(2))),
        ("/seq", Some(json!(0))),
        ("/source_commit", Some(json!("abc"))),
        ("/index/path", Some(json!("index/01.json"))),
        ("/index/sha256", Some(json!("A".repeat(64)))),
        ("/index/extra", Some(json!(1))),
        ("/listings/0/ref/length", Some(json!(0))),
        ("/listings/0/ref/length", Some(json!(70000))),
        ("/listings/0/ref/url", Some(json!("https://x.example/"))),
        ("/listings/0/id", Some(json!("Not An Id"))),
        (
            "/editorial/path",
            Some(json!("storefront/editorial/latest.json")),
        ),
        ("/editorial/length", Some(json!(200_000))),
        ("/listings", None),
        ("/index", None),
    ];
    for (pointer, value) in mutations {
        let mut doc = original.clone();
        match value {
            Some(value) => set(&mut doc, pointer, value),
            None => {
                let (parent, key) = pointer.rsplit_once('/').unwrap();
                doc.pointer_mut(if parent.is_empty() { "" } else { parent })
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove(key);
            }
        }
        let bytes = serde_json::to_vec(&doc).unwrap();
        assert!(
            !schema_errors(&schema, &bytes).is_empty(),
            "the schema accepts {pointer}"
        );
        assert!(!ours(&bytes), "the model accepts {pointer}");
    }
}

/// The sources: what the input schemas refuse, `storefront check` refuses.
#[test]
fn hostile_sources_are_refused_by_check() {
    let cases: Vec<(&str, Box<dyn Fn(&Fixture)>, &str)> = vec![
        (
            "placement in a listing",
            Box::new(|fx: &Fixture| {
                let mut listing = source_listing();
                listing["featured"] = json!(true);
                fx.write_json("registry/plugins/acme.alpha/listing.json", &listing);
            }),
            "featured",
        ),
        (
            "order in a listing",
            Box::new(|fx: &Fixture| {
                let mut listing = source_listing();
                listing["order"] = json!(1);
                fx.write_json("registry/plugins/acme.alpha/listing.json", &listing);
            }),
            "order",
        ),
        (
            "an SVG icon",
            Box::new(|fx: &Fixture| {
                let mut listing = source_listing();
                listing["icon"] = json!({ "file": "media/icon.svg" });
                fx.write_json("registry/plugins/acme.alpha/listing.json", &listing);
                fx.write("registry/plugins/acme.alpha/media/icon.svg", "<svg/>");
            }),
            "icon",
        ),
        (
            "an SVG under a PNG name",
            Box::new(|fx: &Fixture| {
                fx.write(
                    "registry/plugins/acme.alpha/media/icon.png",
                    "<svg xmlns='http://www.w3.org/2000/svg'/>",
                );
            }),
            "SVG",
        ),
        (
            "a PNG under a WebP name",
            Box::new(|fx: &Fixture| {
                fx.write(
                    "registry/plugins/acme.alpha/media/mac.webp",
                    real_png(1280, 720),
                );
            }),
            "not what its extension says",
        ),
        (
            "headers that do not decode",
            Box::new(|fx: &Fixture| {
                fx.write(
                    "registry/plugins/acme.alpha/media/main.png",
                    png_headers(1280, 800, &[]),
                );
            }),
            "does not decode",
        ),
        (
            "an animated PNG",
            Box::new(|fx: &Fixture| {
                fx.write(
                    "registry/plugins/acme.alpha/media/main.png",
                    png_headers(1280, 800, &[chunk(b"acTL", &[0; 8])]),
                );
            }),
            "animated",
        ),
        (
            "a screenshot of the wrong shape",
            Box::new(|fx: &Fixture| {
                fx.write(
                    "registry/plugins/acme.alpha/media/main.png",
                    real_png(1280, 1280),
                );
            }),
            "16:10",
        ),
        (
            "a path outside media/",
            Box::new(|fx: &Fixture| {
                let mut listing = source_listing();
                listing["icon"] = json!({ "file": "../plugin.json" });
                fx.write_json("registry/plugins/acme.alpha/listing.json", &listing);
            }),
            "icon",
        ),
        (
            "a missing file",
            Box::new(|fx: &Fixture| {
                std::fs::remove_file(fx.dir().join("registry/plugins/acme.alpha/media/main.png"))
                    .unwrap();
            }),
            "main.png",
        ),
        (
            "notes for an unrecorded version",
            Box::new(|fx: &Fixture| {
                let mut listing = source_listing();
                listing["release_notes"] = json!({ "9.9.9": { "en": "Not yet." } });
                fx.write_json("registry/plugins/acme.alpha/listing.json", &listing);
            }),
            "9.9.9",
        ),
        (
            "a listing in another plugin's directory",
            Box::new(|fx: &Fixture| {
                let mut listing = source_listing();
                listing["id"] = json!("acme.beta");
                fx.write_json("registry/plugins/acme.alpha/listing.json", &listing);
            }),
            "directory name",
        ),
        (
            "raw HTML in a description",
            Box::new(|fx: &Fixture| {
                let mut listing = source_listing();
                listing["description_md"] = json!({ "en": "<img src=x onerror=alert(1)>" });
                fx.write_json("registry/plugins/acme.alpha/listing.json", &listing);
            }),
            "HTML",
        ),
        (
            "an http link",
            Box::new(|fx: &Fixture| {
                let mut listing = source_listing();
                listing["links"] = json!({ "support": "http://acme.example/" });
                fx.write_json("registry/plugins/acme.alpha/listing.json", &listing);
            }),
            "support",
        ),
        (
            "an external link in editorial",
            Box::new(|fx: &Fixture| {
                let mut editorial = source_editorial();
                editorial["sections"][0]["items"][0]["target"] =
                    json!({ "type": "url", "id": "https://evil.example" });
                fx.write_json("editorial/editorial.json", &editorial);
            }),
            "target",
        ),
        (
            "an unknown plugin in editorial",
            Box::new(|fx: &Fixture| {
                let mut editorial = source_editorial();
                editorial["sections"][1]["plugins"] = json!(["acme.ghost"]);
                fx.write_json("editorial/editorial.json", &editorial);
            }),
            "acme.ghost",
        ),
        (
            "an editorial section without status",
            Box::new(|fx: &Fixture| {
                let mut editorial = source_editorial();
                editorial["sections"][1]
                    .as_object_mut()
                    .unwrap()
                    .remove("status");
                fx.write_json("editorial/editorial.json", &editorial);
            }),
            "status",
        ),
        (
            "an unknown section kind in the source",
            Box::new(|fx: &Fixture| {
                let mut editorial = source_editorial();
                editorial["sections"][1]["kind"] = json!("hero");
                fx.write_json("editorial/editorial.json", &editorial);
            }),
            "kind",
        ),
        (
            "a link inside editorial media",
            Box::new(|fx: &Fixture| {
                #[cfg(unix)]
                {
                    let target = fx.dir().join("registry/plugins/acme.alpha/media/main.png");
                    let link = fx.dir().join("editorial/media/banner.png");
                    std::fs::remove_file(&link).unwrap();
                    std::os::unix::fs::symlink(target, link).unwrap();
                }
                #[cfg(not(unix))]
                fx.write("editorial/media/banner.png", "not an image");
            }),
            "banner.png",
        ),
    ];
    for (name, mutate, expected) in cases {
        let fx = base_fixture();
        with_sources(&fx);
        fx.run(&["storefront", "check"]);
        mutate(&fx);
        let error = fx.fails(&["storefront", "check"]);
        assert!(error.contains(expected), "{name}: {error}");
        // Build refuses the same sources.
        fx.build_and_sign_index(t0());
        assert!(
            fx.try_run(&["storefront", "build", "--now", &time(t0())])
                .is_err(),
            "{name}"
        );
    }
}

#[test]
fn review_words_are_flagged_outside_the_first_party_namespace() {
    let fx = base_fixture();
    with_sources(&fx);
    let mut listing = source_listing();
    listing["promo"] = json!({ "en": "The official Alpha, verified" });
    fx.write_json("registry/plugins/acme.alpha/listing.json", &listing);
    let out = fx.run(&["storefront", "check"]);
    assert!(
        out.contains("review: the listing says \"official\""),
        "{out}"
    );
    assert!(
        out.contains("review: the listing says \"verified\""),
        "{out}"
    );
}

#[test]
fn verify_site_without_the_flag_behaves_as_before() {
    let fx = published_fixture();
    let with = fx.verify(&["--require-packages"], t0()).unwrap();
    // The same site without any storefront file.
    let bare = base_fixture();
    copy_tree(&fx.dir().join("site"), &bare.dir().join("site"));
    std::fs::copy(fx.dir().join("roots.json"), bare.dir().join("roots.json")).unwrap();
    std::fs::remove_dir_all(bare.site().join("storefront")).unwrap();
    std::fs::remove_dir_all(bare.site().join("media")).unwrap();
    let without = bare.verify(&["--require-packages"], t0()).unwrap();
    assert_eq!(with, without);
    assert!(!with.contains("storefront"), "{with}");
    // A broken storefront does not matter without the flag either.
    std::fs::write(fx.site().join("storefront/1.json.minisig"), "garbage").unwrap();
    assert_eq!(fx.verify(&["--require-packages"], t0()).unwrap(), with);
    // With the flag it does, and a missing storefront fails.
    assert!(
        fx.verify(&["--require-storefront"], t0())
            .unwrap_err()
            .contains("refuses storefront/1.json")
    );
    let error = bare.verify(&["--require-storefront"], t0()).unwrap_err();
    assert!(
        error.contains("storefront/1.json") && error.contains("--require-storefront"),
        "{error}"
    );
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn verify_site_refuses_tampered_storefront_files() {
    let fx = published_fixture();
    fx.verify(&["--require-storefront"], t0()).unwrap();
    let storefront: Storefront =
        serde_json::from_slice(&fx.read_site("storefront/1.json")).unwrap();
    let listing_path = storefront.listings[0].reference.path.clone();
    let listing: ListingDoc = serde_json::from_slice(&fx.read_site(&listing_path)).unwrap();
    let icon = media_path(listing.icon.as_ref().unwrap());
    let tamper = |path: &str| {
        let original = fx.read_site(path);
        let mut bytes = original.clone();
        let at = bytes.len() / 2;
        bytes[at] ^= 1;
        std::fs::write(fx.site().join(path), bytes).unwrap();
        let error = fx.verify(&["--require-storefront"], t0()).unwrap_err();
        std::fs::write(fx.site().join(path), original).unwrap();
        error
    };
    assert!(tamper(&icon).contains("sha256"));
    assert!(tamper(&listing_path).contains("sha256"));
    assert!(tamper("storefront/1.json").contains("refuses storefront/1.json"));
    // A missing image.
    std::fs::remove_file(fx.site().join(&icon)).unwrap();
    assert!(
        fx.verify(&["--require-storefront"], t0())
            .unwrap_err()
            .contains(&icon)
    );
}

#[test]
fn signing_needs_the_index_key_and_a_signed_index() {
    let fx = base_fixture();
    with_sources(&fx);
    fx.run(&[
        "build-index",
        "--source-commit",
        &"a".repeat(40),
        "--now",
        &time(t0()),
    ]);
    assert!(
        fx.fails(&["storefront", "build", "--now", &time(t0())])
            .contains("not signed")
    );
    fx.run(&["sign-index", "--key", &fx.key("index.key")]);
    fx.run(&["storefront", "build", "--now", &time(t0())]);
    assert!(
        fx.fails(&["storefront", "build", "--now", &time(t0())])
            .contains("already exists")
    );
    for key in ["timestamp.key", "root.key"] {
        assert!(
            fx.fails(&["storefront", "sign", "--key", &fx.key(key)])
                .contains("not an index key"),
            "{key}"
        );
    }
    fx.run(&["storefront", "sign", "--key", &fx.key("index.key")]);
    assert!(
        fx.fails(&["storefront", "sign", "--key", &fx.key("index.key")])
            .contains("already exists")
    );
}

#[test]
fn a_storefront_is_bound_to_the_index_it_was_built_for() {
    let fx = base_fixture();
    with_sources(&fx);
    fx.build_and_sign_index(t0());
    fx.run(&["storefront", "build", "--now", &time(t0())]);
    // The index changes under the unsigned storefront: sign refuses.
    let index = fx.site().join("index/1.json");
    let mut bytes = std::fs::read(&index).unwrap();
    bytes.push(b'\n');
    std::fs::write(&index, bytes).unwrap();
    assert!(
        fx.fails(&["storefront", "sign", "--key", &fx.key("index.key")])
            .contains("not bound")
    );
}

#[test]
fn a_new_publish_keeps_storefronts_of_kept_indexes_and_prunes_the_rest() {
    let fx = published_fixture();
    fx.add_version("acme.beta", "0.2.0");
    // A changed listing: new content, new media.
    let mut listing = source_listing();
    listing["promo"] = json!({ "en": "Alpha, now better" });
    fx.write_json("registry/plugins/acme.alpha/listing.json", &listing);
    fx.write(
        "registry/plugins/acme.alpha/media/main.png",
        real_png(1440, 900),
    );
    fx.run(&[
        "build-index",
        "--source-commit",
        &"b".repeat(40),
        "--now",
        &time(t0() + days(1)),
        "--keep",
        "0",
    ]);
    fx.run(&["sign-index", "--key", &fx.key("index.key")]);
    fx.storefront(t0() + days(1));
    fx.timestamp(t0() + days(1));
    assert!(!fx.site().join("storefront/1.json").exists());
    assert!(!fx.site().join("storefront/1.json.minisig").exists());
    assert!(fx.site().join("storefront/2.json").exists());
    // Only what seq 2 references is left.
    let count = |dir: &str| {
        std::fs::read_dir(fx.site().join(dir))
            .map(|d| d.count())
            .unwrap_or(0)
    };
    assert_eq!(count("media"), 5);
    assert_eq!(count("storefront/listings/acme.alpha"), 1);
    assert_eq!(count("storefront/editorial"), 1);
    fx.verify(&["--require-storefront"], t0() + days(1))
        .unwrap();
}

#[test]
fn a_listing_of_an_unlisted_plugin_is_skipped() {
    let fx = base_fixture();
    with_sources(&fx);
    fx.run(&[
        "yank",
        "--id",
        "acme.beta",
        "--version",
        "0.1.0",
        "--reason",
        "broken",
        "--now",
        &time(t0()),
    ]);
    let mut beta = source_listing();
    beta["id"] = json!("acme.beta");
    beta.as_object_mut().unwrap().remove("release_notes");
    beta.as_object_mut().unwrap().remove("screenshots");
    beta.as_object_mut().unwrap().remove("promo_art");
    beta.as_object_mut().unwrap().remove("icon");
    fx.write_json("registry/plugins/acme.beta/listing.json", &beta);
    fx.build_and_sign_index(t0());
    let built = fx.run(&["storefront", "build", "--now", &time(t0())]);
    assert!(
        built.contains("skipped the listing of acme.beta"),
        "{built}"
    );
    let storefront: Storefront =
        serde_json::from_slice(&fx.read_site("storefront/1.json")).unwrap();
    assert_eq!(storefront.listings.len(), 1);
}

#[test]
fn a_site_storefront_with_an_unknown_section_kind_verifies() {
    // What an older client sees when a later CI adds a section kind: the
    // client skips it and accepts the rest.
    let fx = published_fixture();
    let storefront_bytes = fx.read_site("storefront/1.json");
    let mut storefront: Value = serde_json::from_slice(&storefront_bytes).unwrap();
    let path = storefront["editorial"]["path"].as_str().unwrap().to_owned();
    let mut editorial: Value = serde_json::from_slice(&fx.read_site(&path)).unwrap();
    editorial["sections"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "id": "future", "kind": "hologram", "order": 1 }));
    let bytes = serde_json::to_vec_pretty(&editorial).unwrap();
    let digest = sha256(&bytes);
    let new_path = format!("storefront/editorial/{digest}.json");
    std::fs::write(fx.site().join(&new_path), &bytes).unwrap();
    storefront["editorial"] =
        json!({ "path": new_path, "sha256": digest.to_string(), "length": bytes.len() });
    let envelope = serde_json::to_vec_pretty(&storefront).unwrap();
    std::fs::write(fx.site().join("storefront/1.json"), &envelope).unwrap();
    std::fs::remove_file(fx.site().join("storefront/1.json.minisig")).unwrap();
    fx.run(&["storefront", "sign", "--key", &fx.key("index.key")]);
    let out = fx.verify(&["--require-storefront"], t0()).unwrap();
    assert!(
        out.contains("3 section(s) (1 of an unknown kind skipped)"),
        "{out}"
    );
}

#[test]
fn the_storefront_never_changes_the_signed_registry_documents() {
    // Same inputs with and without storefront sources: identical keys,
    // index, details and timestamp bytes (timestamps are re-signed, so the
    // content is compared, not the signature).
    let with = base_fixture();
    with_sources(&with);
    let without = base_fixture();
    for fx in [&with, &without] {
        fx.build_and_sign_index(t0());
    }
    with.storefront(t0());
    for fx in [&with, &without] {
        fx.timestamp(t0());
    }
    let mut files = BTreeMap::new();
    collect(&without.site(), &without.site(), &mut files);
    for (path, bytes) in &files {
        if path.ends_with(".minisig") || path.starts_with("keys/") {
            continue;
        }
        if path == "timestamp.json" || path.starts_with("index/") || path.starts_with("plugins/") {
            // Different keys per fixture, but same documents apart from
            // `keys_version` (equal) and hashes of identical bytes.
            assert_eq!(*bytes, with.read_site(path), "{path}");
        }
    }
    assert!(
        files
            .keys()
            .all(|p| !p.starts_with("storefront/") && !p.starts_with("media/"))
    );
}

fn collect(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        if entry.path().is_dir() {
            collect(base, &entry.path(), out);
        } else {
            let rel = entry
                .path()
                .strip_prefix(base)
                .unwrap()
                .display()
                .to_string()
                .replace('\\', "/");
            out.insert(rel, std::fs::read(entry.path()).unwrap());
        }
    }
}
