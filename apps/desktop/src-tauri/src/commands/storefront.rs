//! The registry storefront for Discover (storefront design §6): editorial
//! sections, listings and their images, from an agent that lists the
//! plugin feature `storefront`.
//!
//! Every command answers `null` rather than failing when there is nothing
//! to show (no agent, plugins unavailable, an older agent, no storefront,
//! a refused one, a missing or modified image): Discover then shows the
//! catalog only, the monogram or a placeholder.
//!
//! Images arrive in chunks of verified bytes. This side checks them again
//! before the webview sees them: the same sha256 in every chunk and equal
//! to the one asked for, contiguous offsets, the declared total size at
//! most 1 MiB, a PNG, WebP or JPEG media type, and the sha256 of the
//! assembled bytes. Only then is a `data:image/<type>;base64,` URL returned,
//! which the existing CSP (`img-src 'self' data:`) already allows.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use deckpilot_ipc::plugins::{
    self, ICON_CHUNK_BYTES, ListingView, MAX_MEDIA_BYTES, MediaChunk, STOREFRONT_MEDIA_TYPES,
    StorefrontView, is_media_sha256,
};
use deckpilot_ipc::{AgentClient, Request, Response};
use sha2::{Digest as _, Sha256};
use tauri::State;

use super::plugins::{PluginResult, feature_connection};
use crate::state::AppState;

/// Most chunks one image may take.
const MAX_CHUNKS: usize = (1024 * 1024usize).div_ceil(ICON_CHUNK_BYTES) + 1;

async fn connection(state: &AppState) -> Option<AgentClient> {
    feature_connection(state, plugins::feature::STOREFRONT)
        .await
        .ok()
        .flatten()
}

/// `getStorefront`; `null`: catalog only.
#[tauri::command]
pub async fn get_storefront(state: State<'_, AppState>) -> PluginResult<Option<StorefrontView>> {
    let Some(mut client) = connection(&state).await else {
        return Ok(None);
    };
    Ok(match client.request(Request::GetStorefront).await {
        Ok(Response::Storefront { storefront }) => storefront.map(|s| *s),
        _ => None,
    })
}

/// `getRegistryListing`; `null`: no listing.
#[tauri::command]
pub async fn get_registry_listing(
    state: State<'_, AppState>,
    plugin: String,
) -> PluginResult<Option<ListingView>> {
    let Some(mut client) = connection(&state).await else {
        return Ok(None);
    };
    Ok(
        match client
            .request(Request::GetRegistryListing {
                plugin: plugin.clone(),
            })
            .await
        {
            Ok(Response::RegistryListing {
                plugin: answered,
                listing,
            }) if answered == plugin => listing.map(|l| *l),
            _ => None,
        },
    )
}

/// `getRegistryMedia`: a verified image as a `data:` URL; `null` on any
/// failure.
#[tauri::command]
pub async fn get_registry_media(
    state: State<'_, AppState>,
    sha256: String,
) -> PluginResult<Option<String>> {
    if !is_media_sha256(&sha256) {
        return Ok(None);
    }
    let Some(mut client) = connection(&state).await else {
        return Ok(None);
    };
    let mut chunks = Vec::new();
    let mut offset = 0;
    while chunks.len() < MAX_CHUNKS {
        let request = Request::GetRegistryMedia {
            sha256: sha256.clone(),
            offset,
        };
        let Ok(Response::RegistryMediaChunk { chunk }) = client.request(request).await else {
            return Ok(None);
        };
        let next = chunk.next;
        chunks.push(chunk);
        match next {
            Some(next) if next > offset => offset = next,
            Some(_) => return Ok(None),
            None => break,
        }
    }
    Ok(media_data_url(&sha256, &chunks))
}

/// The chunks of image `sha256` as one `data:` URL, or `None` unless they
/// are consistent and the assembled bytes are exactly that image.
pub(crate) fn media_data_url(sha256: &str, chunks: &[MediaChunk]) -> Option<String> {
    let first = chunks.first()?;
    if !is_media_sha256(sha256)
        || first.sha256 != sha256
        || !STOREFRONT_MEDIA_TYPES.contains(&first.media_type.as_str())
        || first.size == 0
        || first.size > MAX_MEDIA_BYTES
        || chunks.len() > MAX_CHUNKS
    {
        return None;
    }
    let mut bytes = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let last = i + 1 == chunks.len();
        if chunk.sha256 != first.sha256
            || chunk.media_type != first.media_type
            || chunk.size != first.size
            || chunk.offset != bytes.len() as u64
            || chunk.next.is_none() != last
        {
            return None;
        }
        let data = STANDARD.decode(&chunk.data).ok()?;
        if data.is_empty() || data.len() > ICON_CHUNK_BYTES {
            return None;
        }
        bytes.extend_from_slice(&data);
        if bytes.len() as u64 > first.size || chunk.next.is_some_and(|n| n != bytes.len() as u64) {
            return None;
        }
    }
    if bytes.len() as u64 != first.size {
        return None;
    }
    let digest = hex(&Sha256::digest(&bytes));
    if digest != sha256 {
        return None;
    }
    Some(format!(
        "data:{};base64,{}",
        first.media_type,
        STANDARD.encode(&bytes)
    ))
    .filter(|url| is_media_data_url(url))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// Whether `url` is a `data:` URL of a storefront image type.
pub(crate) fn is_media_data_url(url: &str) -> bool {
    STOREFRONT_MEDIA_TYPES
        .iter()
        .any(|media| url.starts_with(&format!("data:{media};base64,")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    fn chunks(bytes: &[u8], media: &str) -> Vec<MediaChunk> {
        let digest = sha(bytes);
        bytes
            .chunks(ICON_CHUNK_BYTES)
            .enumerate()
            .map(|(i, part)| {
                let offset = (i * ICON_CHUNK_BYTES) as u64;
                let end = offset + part.len() as u64;
                MediaChunk {
                    media_type: media.to_owned(),
                    size: bytes.len() as u64,
                    sha256: digest.clone(),
                    offset,
                    data: STANDARD.encode(part),
                    next: (end < bytes.len() as u64).then_some(end),
                }
            })
            .collect()
    }

    fn image(len: usize) -> Vec<u8> {
        (0..len).map(|i| u8::try_from(i % 251).unwrap()).collect()
    }

    #[test]
    fn consistent_chunks_become_a_data_url() {
        let bytes = image(ICON_CHUNK_BYTES * 3 + 17);
        let parts = chunks(&bytes, "image/webp");
        assert_eq!(parts.len(), 4);
        let url = media_data_url(&sha(&bytes), &parts).unwrap();
        assert_eq!(
            url,
            format!("data:image/webp;base64,{}", STANDARD.encode(&bytes))
        );
        let one = image(100);
        assert!(media_data_url(&sha(&one), &chunks(&one, "image/png")).is_some());
        assert!(media_data_url(&sha(&one), &chunks(&one, "image/jpeg")).is_some());
    }

    #[test]
    fn a_sha256_that_changes_across_chunks_is_refused() {
        let bytes = image(ICON_CHUNK_BYTES * 2 + 5);
        let mut parts = chunks(&bytes, "image/png");
        parts[1].sha256 = sha(b"another file");
        assert_eq!(media_data_url(&sha(&bytes), &parts), None);
        // Another image than the one asked for.
        let parts = chunks(&bytes, "image/png");
        assert_eq!(media_data_url(&sha(b"asked"), &parts), None);
        // Bytes that do not hash to the claimed digest.
        let mut forged = chunks(&bytes, "image/png");
        forged[0].data = STANDARD.encode(
            image(ICON_CHUNK_BYTES)
                .iter()
                .map(|b| b ^ 1)
                .collect::<Vec<_>>(),
        );
        assert_eq!(media_data_url(&sha(&bytes), &forged), None);
    }

    #[test]
    fn oversize_and_inconsistent_sizes_are_refused() {
        let bytes = image(200);
        let mut parts = chunks(&bytes, "image/png");
        parts[0].size = MAX_MEDIA_BYTES + 1;
        assert_eq!(media_data_url(&sha(&bytes), &parts), None);
        let mut short = chunks(&bytes, "image/png");
        short[0].size = 300;
        assert_eq!(media_data_url(&sha(&bytes), &short), None);
        let big = image(ICON_CHUNK_BYTES * 2);
        let mut gap = chunks(&big, "image/png");
        gap[1].offset += 1;
        assert_eq!(media_data_url(&sha(&big), &gap), None);
        let mut missing = chunks(&big, "image/png");
        missing.pop();
        assert_eq!(media_data_url(&sha(&big), &missing), None);
        assert_eq!(media_data_url(&sha(&big), &[]), None);
    }

    #[test]
    fn only_png_webp_and_jpeg_are_returned() {
        let bytes = b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec();
        for media in [
            "image/svg+xml",
            "text/html",
            "image/gif",
            "image/png;charset=x",
            "",
        ] {
            assert_eq!(
                media_data_url(&sha(&bytes), &chunks(&bytes, media)),
                None,
                "{media}"
            );
        }
        assert!(is_media_data_url("data:image/jpeg;base64,AAAA"));
        assert!(!is_media_data_url("data:image/svg+xml;base64,AAAA"));
        assert!(!is_media_data_url("https://example.com/x.png"));
    }

    /// Storefront images need no CSP change: `img-src 'self' data:` as
    /// before, nothing else loosened.
    #[test]
    fn the_csp_is_unchanged() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(
            config["app"]["security"]["csp"],
            "default-src 'self' ipc: http://ipc.localhost; img-src 'self' data:; \
             style-src 'self' 'unsafe-inline'"
        );
    }

    #[test]
    fn malformed_digests_are_refused_before_anything_is_sent() {
        let bytes = image(10);
        let parts = chunks(&bytes, "image/png");
        let upper = sha(&bytes).to_uppercase();
        assert_eq!(media_data_url(&upper, &parts), None);
        for bad in ["", "../x", &"a".repeat(63), &"a".repeat(65)] {
            assert!(!is_media_sha256(bad));
        }
    }
}
