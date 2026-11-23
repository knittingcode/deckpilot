//! An application's icon on macOS, as `NSWorkspace` draws it in the Finder
//! (`iconForFile`), made a PNG at [`crate::icons::ICON_SIZE`] points. Two
//! `AppKit` calls take raw arguments and are `unsafe` in Rust: drawing the
//! icon into a `CGImage` (a pointer to the proposed rectangle) and encoding
//! it (a properties dictionary). Both get values built here.
#![allow(unsafe_code)]

use std::path::Path;

use objc2::AnyThread as _;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

/// The icon of the application (or any file) at `path`, as PNG bytes,
/// drawn into a new bitmap at [`crate::icons::ICON_SIZE`] points without
/// changing the shared `NSImage` `NSWorkspace` hands out, inside an
/// autorelease pool so nothing outlives the call.
pub fn icon_png(path: &Path) -> Option<Vec<u8>> {
    let path = path.to_str()?;
    objc2::rc::autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let image = workspace.iconForFile(&NSString::from_str(path));
        let side = f64::from(crate::icons::ICON_SIZE);
        let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(side, side));
        // SAFETY: `rect` is a valid, live rectangle; no context or hints.
        let cg = unsafe { image.CGImageForProposedRect_context_hints(&raw mut rect, None, None) }?;
        let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
        let properties = NSDictionary::new();
        // SAFETY: an empty properties dictionary is valid for every file type.
        let data = unsafe {
            bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
        }?;
        Some(data.to_vec())
    })
}
