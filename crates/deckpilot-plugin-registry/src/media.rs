//! Storefront media: a pure header and chunk walk of PNG, JPEG and WebP
//! files (storefront design §4 and §6).
//!
//! [`inspect`] reads no pixel data: it walks the container (PNG chunks with
//! their CRCs, JPEG marker segments up to the end of image, WebP RIFF
//! chunks) and enforces every §4 rule that needs nothing else:
//!
//! * the format is sniffed from the magic bytes (the file name and any
//!   declared format are not consulted here);
//! * at most [`MAX_MEDIA_BYTES`], at most [`MAX_MEDIA_PIXELS`], no zero side;
//! * 8-bit channels only (no 16-bit PNG, no 12-bit JPEG);
//! * no animation (APNG `acTL`, WebP `ANIM`/`ANMF` or the VP8X animation
//!   flag);
//! * no metadata (PNG `tEXt`/`zTXt`/`iTXt`/`eXIf`, JPEG APP1 (Exif, XMP),
//!   APP13 (IPTC) and comments, WebP `EXIF`/`XMP ` or their VP8X flags);
//!   an ICC profile of at most [`MAX_ICC_BYTES`] is allowed;
//! * PNG: an allow-list of chunks, every CRC checked, nothing after `IEND`;
//! * JPEG: baseline or progressive, 8-bit, 1 or 3 components, exactly one
//!   frame header whose dimensions are authoritative, nothing after `EOI`;
//! * WebP: the RIFF size equals the file length; `VP8 `, `VP8L`, or `VP8X`
//!   with a still image.
//!
//! SVG (or anything else) is refused: there is one decoder family, raster.
//!
//! Full pixel decoding is deliberately not here: the agent (a privileged
//! process) never runs a decompressor over network bytes. The publishing
//! tool (feature `publish`) decodes every asset in full in CI
//! (`publish::storefront`). The dimensions checked here bound what the
//! webview may decode to [`MAX_MEDIA_PIXELS`] × 4 bytes (32 MiB).

use std::fmt;

use serde::{Deserialize, Serialize};

/// Largest storefront media file (§4: screenshots, promo and banner art;
/// icons have a lower limit of their own).
pub const MAX_MEDIA_BYTES: usize = 1024 * 1024;
/// Most pixels (`width × height`) of a storefront image (8 × 2^20), so that a
/// decoded RGBA image is at most 32 MiB.
pub const MAX_MEDIA_PIXELS: u64 = 8_388_608;
/// Largest embedded ICC profile.
pub const MAX_ICC_BYTES: usize = 64 * 1024;

/// A storefront image format. Serialized as the file extension the site
/// uses (`v1/media/<sha256>.<format>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MediaFormat {
    #[serde(rename = "png")]
    Png,
    #[serde(rename = "webp")]
    Webp,
    #[serde(rename = "jpg")]
    Jpeg,
}

impl MediaFormat {
    pub const ALL: [Self; 3] = [Self::Png, Self::Webp, Self::Jpeg];

    /// The file extension on the site (`png`, `webp`, `jpg`).
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Jpeg => "jpg",
        }
    }

    /// The media type of a `data:` URL.
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Webp => "image/webp",
            Self::Jpeg => "image/jpeg",
        }
    }

    pub fn from_extension(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.extension() == text)
    }

    /// The format whose magic bytes start `bytes`.
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(PNG_SIGNATURE) {
            Some(Self::Png)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Self::Jpeg)
        } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            Some(Self::Webp)
        } else {
            None
        }
    }
}

impl fmt::Display for MediaFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Png => "PNG",
            Self::Webp => "WebP",
            Self::Jpeg => "JPEG",
        })
    }
}

/// What [`inspect`] learned from the headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RasterInfo {
    pub format: MediaFormat,
    pub width: u32,
    pub height: u32,
}

/// Why a media file was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MediaError {
    #[error("is {size} bytes; at most {limit} are allowed")]
    TooLarge { size: usize, limit: usize },
    #[error("is an SVG; storefront art must be PNG, WebP or JPEG")]
    Svg,
    #[error("is not a PNG, WebP or JPEG file")]
    UnknownFormat,
    #[error("is a malformed {format} file: {reason}")]
    Malformed { format: MediaFormat, reason: String },
    #[error("is {width}×{height} pixels; at most {MAX_MEDIA_PIXELS} pixels and no empty side")]
    Dimensions { width: u32, height: u32 },
    #[error("uses {0}; only 8-bit channels are allowed")]
    BitDepth(String),
    #[error("is animated ({0}); storefront art is a still image")]
    Animated(&'static str),
    #[error("carries metadata ({0}); strip it before publishing")]
    Metadata(String),
    #[error("is a {0}, which is not allowed")]
    Unsupported(String),
}

fn malformed(format: MediaFormat, reason: impl Into<String>) -> MediaError {
    MediaError::Malformed {
        format,
        reason: reason.into(),
    }
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Checks `bytes` against every §4 rule a header walk can enforce and
/// returns the format and dimensions. See the module documentation.
pub fn inspect(bytes: &[u8]) -> Result<RasterInfo, MediaError> {
    if bytes.len() > MAX_MEDIA_BYTES {
        return Err(MediaError::TooLarge {
            size: bytes.len(),
            limit: MAX_MEDIA_BYTES,
        });
    }
    let info = match MediaFormat::sniff(bytes) {
        Some(MediaFormat::Png) => inspect_png(bytes)?,
        Some(MediaFormat::Jpeg) => inspect_jpeg(bytes)?,
        Some(MediaFormat::Webp) => inspect_webp(bytes)?,
        None if looks_like_svg(bytes) => return Err(MediaError::Svg),
        None => return Err(MediaError::UnknownFormat),
    };
    let pixels = u64::from(info.width) * u64::from(info.height);
    if info.width == 0 || info.height == 0 || pixels > MAX_MEDIA_PIXELS {
        return Err(MediaError::Dimensions {
            width: info.width,
            height: info.height,
        });
    }
    Ok(info)
}

/// An XML or SVG document (for a clearer refusal; anything that is not one
/// of the three raster formats is refused either way).
fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(1024)];
    let text = String::from_utf8_lossy(head).to_ascii_lowercase();
    let text = text.trim_start_matches('\u{feff}').trim_start();
    text.starts_with("<?xml") || text.starts_with("<svg") || text.contains("<svg")
}

fn u16_be(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_be(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u32_le(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u24_le(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 3)?;
    Some(u32::from(b[0]) | (u32::from(b[1]) << 8) | (u32::from(b[2]) << 16))
}

// ------------------------------------------------------------------ PNG

/// CRC-32 (ISO-HDLC, as PNG uses), bitwise: chunks are small and this runs
/// once per file.
pub(crate) fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for part in parts {
        for &byte in *part {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
    }
    !crc
}

/// PNG chunks a storefront image may carry. Everything else is refused,
/// critical or not.
const PNG_ALLOWED: [&[u8; 4]; 10] = [
    b"IHDR", b"PLTE", b"tRNS", b"IDAT", b"IEND", b"sRGB", b"gAMA", b"cHRM", b"iCCP", b"pHYs",
];

#[allow(clippy::too_many_lines)] // one pass over the chunks
fn inspect_png(bytes: &[u8]) -> Result<RasterInfo, MediaError> {
    let bad = |reason: &str| malformed(MediaFormat::Png, reason);
    let mut at = PNG_SIGNATURE.len();
    let mut header: Option<(u32, u32, u8)> = None;
    let mut seen_plte = false;
    let mut seen_idat = false;
    let mut idat_done = false;
    let mut ended = false;
    while at < bytes.len() {
        let length = u32_be(bytes, at).ok_or_else(|| bad("a chunk header is truncated"))?;
        let length = usize::try_from(length).map_err(|_| bad("a chunk is too long"))?;
        if length > 0x7FFF_FFFF {
            return Err(bad("a chunk length is out of range"));
        }
        let kind: [u8; 4] = bytes
            .get(at + 4..at + 8)
            .and_then(|k| k.try_into().ok())
            .ok_or_else(|| bad("a chunk header is truncated"))?;
        let data_start = at + 8;
        let data_end = data_start
            .checked_add(length)
            .ok_or_else(|| bad("a chunk is too long"))?;
        let data = bytes
            .get(data_start..data_end)
            .ok_or_else(|| bad("a chunk is truncated"))?;
        let crc = u32_be(bytes, data_end).ok_or_else(|| bad("a chunk CRC is truncated"))?;
        if crc != crc32(&[&kind, data]) {
            return Err(bad(&format!(
                "the CRC of chunk {} is wrong",
                String::from_utf8_lossy(&kind)
            )));
        }
        at = data_end + 4;
        let name = String::from_utf8_lossy(&kind).into_owned();
        if header.is_none() && &kind != b"IHDR" {
            return Err(bad("the first chunk is not IHDR"));
        }
        match &kind {
            b"acTL" | b"fcTL" | b"fdAT" => return Err(MediaError::Animated("APNG")),
            b"tEXt" | b"zTXt" | b"iTXt" | b"eXIf" => {
                return Err(MediaError::Metadata(format!("PNG {name} chunk")));
            }
            _ if !PNG_ALLOWED.contains(&&kind) => {
                return Err(MediaError::Unsupported(format!("PNG {name} chunk")));
            }
            _ => {}
        }
        if &kind != b"IDAT" && seen_idat {
            idat_done = true;
        }
        match &kind {
            b"IHDR" => {
                if header.is_some() {
                    return Err(bad("IHDR appears twice"));
                }
                if length != 13 {
                    return Err(bad("IHDR is not 13 bytes"));
                }
                let width = u32_be(data, 0).ok_or_else(|| bad("IHDR"))?;
                let height = u32_be(data, 4).ok_or_else(|| bad("IHDR"))?;
                let (depth, color, compression, filter, interlace) =
                    (data[8], data[9], data[10], data[11], data[12]);
                let depth_ok = match color {
                    // Palette and grayscale may pack pixels below 8 bits.
                    0 | 3 => matches!(depth, 1 | 2 | 4 | 8),
                    2 | 4 | 6 => depth == 8,
                    _ => return Err(bad(&format!("color type {color} is not defined"))),
                };
                if depth == 16 {
                    return Err(MediaError::BitDepth("16-bit PNG channels".into()));
                }
                if !depth_ok {
                    return Err(bad(&format!(
                        "bit depth {depth} is not valid for color type {color}"
                    )));
                }
                if compression != 0 || filter != 0 || interlace > 1 {
                    return Err(bad("IHDR names an undefined method"));
                }
                if width == 0 || height == 0 || width > 0x7FFF_FFFF || height > 0x7FFF_FFFF {
                    return Err(MediaError::Dimensions { width, height });
                }
                header = Some((width, height, color));
            }
            b"PLTE" => {
                if seen_plte || seen_idat {
                    return Err(bad("PLTE is misplaced"));
                }
                if length == 0 || length % 3 != 0 || length > 768 {
                    return Err(bad("PLTE has a bad length"));
                }
                seen_plte = true;
            }
            b"iCCP" => {
                if seen_idat || seen_plte {
                    return Err(bad("iCCP is misplaced"));
                }
                if length > MAX_ICC_BYTES {
                    return Err(MediaError::Metadata(format!(
                        "an ICC profile of {length} bytes, more than {MAX_ICC_BYTES}"
                    )));
                }
            }
            b"IDAT" => {
                if idat_done {
                    return Err(bad("IDAT chunks are not consecutive"));
                }
                seen_idat = true;
            }
            b"IEND" => {
                if length != 0 {
                    return Err(bad("IEND carries data"));
                }
                ended = true;
                break;
            }
            _ => {
                if seen_idat && matches!(&kind, b"sRGB" | b"gAMA" | b"cHRM" | b"pHYs") {
                    return Err(bad(&format!("{name} follows the image data")));
                }
            }
        }
    }
    if !ended {
        return Err(bad("IEND is missing"));
    }
    if at != bytes.len() {
        return Err(bad("data follows IEND"));
    }
    let (width, height, color) = header.ok_or_else(|| bad("IHDR is missing"))?;
    if !seen_idat {
        return Err(bad("there is no image data"));
    }
    if color == 3 && !seen_plte {
        return Err(bad("a palette image has no PLTE"));
    }
    Ok(RasterInfo {
        format: MediaFormat::Png,
        width,
        height,
    })
}

// ----------------------------------------------------------------- JPEG

#[allow(clippy::too_many_lines, clippy::match_same_arms)] // one pass over the markers
fn inspect_jpeg(bytes: &[u8]) -> Result<RasterInfo, MediaError> {
    let bad = |reason: &str| malformed(MediaFormat::Jpeg, reason);
    let mut at = 2; // after SOI
    let mut frame: Option<(u32, u32)> = None;
    let mut icc = 0usize;
    let mut scans = 0usize;
    loop {
        // A marker: 0xFF, optional 0xFF fill bytes, then the code.
        if bytes.get(at) != Some(&0xFF) {
            return Err(bad("a marker is expected"));
        }
        while bytes.get(at) == Some(&0xFF) {
            at += 1;
        }
        let code = *bytes
            .get(at)
            .ok_or_else(|| bad("the file ends inside a marker"))?;
        at += 1;
        match code {
            0xD9 => break, // EOI
            0xD8 => return Err(bad("a second start of image")),
            0x01 | 0xD0..=0xD7 => return Err(bad("a stand-alone marker out of place")),
            0x00 => return Err(bad("a stuffed byte outside the entropy data")),
            _ => {}
        }
        let length = usize::from(u16_be(bytes, at).ok_or_else(|| bad("a segment is truncated"))?);
        if length < 2 {
            return Err(bad("a segment length is below 2"));
        }
        let data = bytes
            .get(at + 2..at + length)
            .ok_or_else(|| bad("a segment is truncated"))?;
        at += length;
        match code {
            // SOF0 baseline, SOF2 progressive (Huffman).
            0xC0 | 0xC2 => {
                if frame.is_some() {
                    return Err(bad("more than one frame header"));
                }
                if scans > 0 {
                    return Err(bad("the frame header follows a scan"));
                }
                let precision = *data.first().ok_or_else(|| bad("SOF"))?;
                if precision != 8 {
                    return Err(MediaError::BitDepth(format!(
                        "{precision}-bit JPEG samples"
                    )));
                }
                let height = u16_be(data, 1).ok_or_else(|| bad("SOF"))?;
                let width = u16_be(data, 3).ok_or_else(|| bad("SOF"))?;
                let components = *data.get(5).ok_or_else(|| bad("SOF"))?;
                if !matches!(components, 1 | 3) {
                    return Err(MediaError::Unsupported(format!(
                        "JPEG with {components} components (CMYK or other)"
                    )));
                }
                if data.len() != 6 + 3 * usize::from(components) {
                    return Err(bad("the frame header has a bad length"));
                }
                if height == 0 {
                    return Err(MediaError::Unsupported(
                        "JPEG with its height in a DNL marker".into(),
                    ));
                }
                frame = Some((u32::from(width), u32::from(height)));
            }
            0xC1 | 0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                return Err(MediaError::Unsupported(format!(
                    "JPEG coding process SOF{} (only baseline and progressive)",
                    code - 0xC0
                )));
            }
            0xE1 => return Err(MediaError::Metadata("JPEG APP1 (Exif or XMP)".into())),
            0xED => return Err(MediaError::Metadata("JPEG APP13 (IPTC)".into())),
            0xFE => return Err(MediaError::Metadata("a JPEG comment".into())),
            0xE2 => {
                if !data.starts_with(b"ICC_PROFILE\0") {
                    return Err(MediaError::Metadata("a JPEG APP2 segment".into()));
                }
                icc += data.len();
                if icc > MAX_ICC_BYTES + 1024 {
                    return Err(MediaError::Metadata(format!(
                        "an ICC profile of more than {MAX_ICC_BYTES} bytes"
                    )));
                }
            }
            // APP0 (JFIF) and APP14 (Adobe colour transform).
            0xE0 | 0xEE => {}
            0xE3..=0xEF => {
                return Err(MediaError::Metadata(format!(
                    "a JPEG APP{} segment",
                    code - 0xE0
                )));
            }
            // DHT, DAC (refused by its SOF), DQT, DRI, DNL.
            0xC4 | 0xCC | 0xDB | 0xDD | 0xDC => {}
            0xDA => {
                if frame.is_none() {
                    return Err(bad("a scan before the frame header"));
                }
                scans += 1;
                // Entropy-coded data: up to the next marker that is not a
                // stuffed byte or a restart marker.
                loop {
                    let next = bytes[at..]
                        .iter()
                        .position(|&b| b == 0xFF)
                        .map(|p| at + p)
                        .ok_or_else(|| bad("the file ends inside a scan (no EOI)"))?;
                    match bytes.get(next + 1) {
                        Some(0x00 | 0xD0..=0xD7) => at = next + 2,
                        Some(0xFF) => at = next + 1,
                        Some(_) => {
                            at = next;
                            break;
                        }
                        None => return Err(bad("the file ends inside a scan (no EOI)")),
                    }
                }
            }
            0xF0..=0xFD | 0x02..=0xBF => {
                return Err(MediaError::Unsupported(format!("JPEG marker 0x{code:02X}")));
            }
            _ => {}
        }
    }
    if at != bytes.len() {
        return Err(bad("data follows the end of image"));
    }
    let (width, height) = frame.ok_or_else(|| bad("there is no frame header"))?;
    if scans == 0 {
        return Err(bad("there is no scan"));
    }
    Ok(RasterInfo {
        format: MediaFormat::Jpeg,
        width,
        height,
    })
}

// ----------------------------------------------------------------- WebP

/// Width and height of a `VP8 ` key frame.
fn vp8_size(data: &[u8]) -> Option<(u32, u32)> {
    // Frame tag: bit 0 clear for a key frame; then the start code.
    if data.first()? & 1 != 0 || data.get(3..6)? != [0x9d, 0x01, 0x2a] {
        return None;
    }
    let width = u32::from(u16::from_le_bytes([*data.get(6)?, *data.get(7)?]) & 0x3FFF);
    let height = u32::from(u16::from_le_bytes([*data.get(8)?, *data.get(9)?]) & 0x3FFF);
    Some((width, height))
}

/// Width and height of a `VP8L` image.
fn vp8l_size(data: &[u8]) -> Option<(u32, u32)> {
    if *data.first()? != 0x2f {
        return None;
    }
    let bits = u32_le(data, 1)?;
    // Version (3 bits after alpha) must be 0.
    if bits >> 29 != 0 {
        return None;
    }
    Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
}

#[allow(clippy::too_many_lines)] // one pass over the chunks
fn inspect_webp(bytes: &[u8]) -> Result<RasterInfo, MediaError> {
    let bad = |reason: &str| malformed(MediaFormat::Webp, reason);
    let riff = u32_le(bytes, 4).ok_or_else(|| bad("the RIFF header is truncated"))?;
    let riff = usize::try_from(riff).map_err(|_| bad("the RIFF size is out of range"))?;
    if riff.checked_add(8) != Some(bytes.len()) {
        return Err(bad("the RIFF size is not the file length"));
    }
    let mut chunks = Vec::new();
    let mut at = 12;
    while at < bytes.len() {
        let kind: [u8; 4] = bytes
            .get(at..at + 4)
            .and_then(|k| k.try_into().ok())
            .ok_or_else(|| bad("a chunk header is truncated"))?;
        let size = u32_le(bytes, at + 4).ok_or_else(|| bad("a chunk header is truncated"))?;
        let size = usize::try_from(size).map_err(|_| bad("a chunk is too long"))?;
        let start = at + 8;
        let end = start
            .checked_add(size)
            .ok_or_else(|| bad("a chunk is too long"))?;
        let data = bytes
            .get(start..end)
            .ok_or_else(|| bad("a chunk is truncated"))?;
        // Odd sizes are padded to even.
        at = end + (size & 1);
        if at > bytes.len() {
            return Err(bad("a chunk's padding is missing"));
        }
        chunks.push((kind, data));
    }
    let (first, first_data) = *chunks.first().ok_or_else(|| bad("there is no chunk"))?;
    let check_other = |kind: &[u8; 4]| -> Result<(), MediaError> {
        match kind {
            b"ANIM" | b"ANMF" => Err(MediaError::Animated("WebP animation")),
            b"EXIF" => Err(MediaError::Metadata("WebP EXIF chunk".into())),
            b"XMP " => Err(MediaError::Metadata("WebP XMP chunk".into())),
            other => Err(MediaError::Unsupported(format!(
                "WebP {} chunk",
                String::from_utf8_lossy(other)
            ))),
        }
    };
    let (width, height) = match &first {
        b"VP8 " | b"VP8L" => {
            if let Some((kind, _)) = chunks.get(1) {
                check_other(kind)?;
            }
            if &first == b"VP8 " {
                vp8_size(first_data).ok_or_else(|| bad("the VP8 frame header is invalid"))?
            } else {
                vp8l_size(first_data).ok_or_else(|| bad("the VP8L header is invalid"))?
            }
        }
        b"VP8X" => {
            if first_data.len() != 10 {
                return Err(bad("VP8X is not 10 bytes"));
            }
            let flags = first_data[0];
            if flags & 0x02 != 0 {
                return Err(MediaError::Animated("WebP animation flag"));
            }
            if flags & 0x08 != 0 {
                return Err(MediaError::Metadata("WebP EXIF flag".into()));
            }
            if flags & 0x04 != 0 {
                return Err(MediaError::Metadata("WebP XMP flag".into()));
            }
            if flags & 0xC1 != 0 || first_data[1..4] != [0, 0, 0] {
                return Err(bad("VP8X sets reserved bits"));
            }
            let width = u24_le(first_data, 4).ok_or_else(|| bad("VP8X"))? + 1;
            let height = u24_le(first_data, 7).ok_or_else(|| bad("VP8X"))? + 1;
            let mut image = None;
            let mut seen_alpha = false;
            for (index, (kind, data)) in chunks.iter().enumerate().skip(1) {
                match kind {
                    b"ICCP" => {
                        if flags & 0x20 == 0 || index != 1 {
                            return Err(bad("ICCP is misplaced or not flagged"));
                        }
                        if data.len() > MAX_ICC_BYTES {
                            return Err(MediaError::Metadata(format!(
                                "an ICC profile of {} bytes, more than {MAX_ICC_BYTES}",
                                data.len()
                            )));
                        }
                    }
                    b"ALPH" => {
                        if image.is_some() || seen_alpha {
                            return Err(bad("ALPH is misplaced"));
                        }
                        seen_alpha = true;
                    }
                    b"VP8 " | b"VP8L" => {
                        if image.is_some() {
                            return Err(bad("more than one image chunk"));
                        }
                        let size = if kind == b"VP8 " {
                            vp8_size(data)
                        } else {
                            vp8l_size(data)
                        };
                        image = Some(size.ok_or_else(|| bad("the image chunk header is invalid"))?);
                    }
                    other => check_other(other)?,
                }
            }
            let image = image.ok_or_else(|| bad("there is no image chunk"))?;
            if image != (width, height) {
                return Err(bad("the image size differs from the canvas size"));
            }
            (width, height)
        }
        other => {
            check_other(other)?;
            return Err(bad("there is no image chunk"));
        }
    };
    Ok(RasterInfo {
        format: MediaFormat::Webp,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_the_png_reference() {
        // The CRC of an empty IEND chunk, as every PNG ends.
        assert_eq!(crc32(&[b"IEND"]), 0xAE42_6082);
    }

    #[test]
    fn format_names() {
        for format in MediaFormat::ALL {
            assert_eq!(
                MediaFormat::from_extension(format.extension()),
                Some(format)
            );
        }
        assert_eq!(MediaFormat::from_extension("jpeg"), None);
        assert_eq!(MediaFormat::from_extension("svg"), None);
    }
}
