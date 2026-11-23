//! An application's icon on Windows, as the shell shows it
//! (`SHGetFileInfoW` with `SHGFI_ICON`, the large icon), read into 32-bit
//! pixels with `GetDIBits` and encoded as a PNG. Every handle is freed
//! before returning.
#![allow(unsafe_code)]

use std::io::Cursor;
use std::path::Path;

use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
    GetObjectW, HGDIOBJ, ReleaseDC,
};
use windows_sys::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows_sys::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW};
use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};

/// Largest icon side read.
const MAX_SIDE: i32 = 256;

/// The shell's large icon of the file at `path`, as PNG bytes.
pub fn icon_png(path: &Path) -> Option<Vec<u8>> {
    let wide: Vec<u16> = path
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut info = SHFILEINFOW::default();
    let size = u32::try_from(std::mem::size_of::<SHFILEINFOW>()).ok()?;
    // SAFETY: `wide` is NUL-terminated and `info` is a SHFILEINFOW of
    // `size` bytes; both outlive the call.
    let found = unsafe {
        SHGetFileInfoW(
            wide.as_ptr(),
            FILE_FLAGS_AND_ATTRIBUTES::default(),
            &raw mut info,
            size,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    if found == 0 || info.hIcon.is_null() {
        return None;
    }
    let pixels = pixels_of(info.hIcon);
    // SAFETY: the icon came from SHGetFileInfoW and is destroyed once.
    unsafe { DestroyIcon(info.hIcon) };
    let (width, height, rgba) = pixels?;
    let picture = image::RgbaImage::from_raw(width, height, rgba)?;
    let mut out = Vec::new();
    picture
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

/// The colour bitmap of `icon` as RGBA rows, top first.
fn pixels_of(
    icon: windows_sys::Win32::UI::WindowsAndMessaging::HICON,
) -> Option<(u32, u32, Vec<u8>)> {
    let mut parts = ICONINFO::default();
    // SAFETY: `icon` is a live icon; `parts` receives two bitmaps, which
    // are deleted below.
    if unsafe { GetIconInfo(icon, &raw mut parts) } == 0 {
        return None;
    }
    let result = read_bitmap(parts.hbmColor);
    for bitmap in [parts.hbmColor, parts.hbmMask] {
        if !bitmap.is_null() {
            // SAFETY: the bitmaps of GetIconInfo belong to the caller.
            unsafe { DeleteObject(bitmap as HGDIOBJ) };
        }
    }
    result
}

fn read_bitmap(bitmap: windows_sys::Win32::Graphics::Gdi::HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    if bitmap.is_null() {
        return None;
    }
    let mut header = BITMAP::default();
    let size = i32::try_from(std::mem::size_of::<BITMAP>()).ok()?;
    // SAFETY: `header` is a BITMAP of `size` bytes.
    if unsafe { GetObjectW(bitmap as HGDIOBJ, size, (&raw mut header).cast()) } == 0 {
        return None;
    }
    let (width, height) = (header.bmWidth, header.bmHeight.abs());
    if width <= 0 || height <= 0 || width > MAX_SIDE || height > MAX_SIDE {
        return None;
    }
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).ok()?,
            biWidth: width,
            // Negative: rows top first.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..BITMAPINFOHEADER::default()
        },
        ..BITMAPINFO::default()
    };
    let (w, h) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    let mut bgra = vec![0u8; (w * h * 4) as usize];
    // SAFETY: the screen's device context is released below; `bgra` holds
    // `height` rows of `width` 32-bit pixels, as `info` describes.
    let rows = unsafe {
        let dc = GetDC(std::ptr::null_mut());
        let rows = GetDIBits(
            dc,
            bitmap,
            0,
            h,
            bgra.as_mut_ptr().cast(),
            &raw mut info,
            DIB_RGB_COLORS,
        );
        ReleaseDC(std::ptr::null_mut(), dc);
        rows
    };
    if rows == 0 {
        return None;
    }
    // BGRA to RGBA; an icon without alpha (all zero) is opaque.
    let opaque = bgra.as_chunks::<4>().0.iter().all(|p| p[3] == 0);
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        if opaque {
            pixel[3] = 255;
        }
    }
    Some((w, h, bgra))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shell_draws_the_icon_of_an_executable() {
        let exe = std::env::current_exe().unwrap();
        let png = icon_png(&exe).expect("an executable has a shell icon");
        let icon = crate::icons::normalize(&png).unwrap();
        assert!(image::load_from_memory(&icon).unwrap().width() <= crate::icons::ICON_SIZE);
        assert_eq!(icon_png(Path::new(r"C:\deckpilot\no\such\file.exe")), None);
    }
}
