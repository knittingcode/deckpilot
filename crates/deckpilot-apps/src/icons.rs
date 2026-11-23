//! Application icons for the pickers, from the operating system: the
//! Finder's icon on macOS (`NSWorkspace`), the shell's icon on Windows
//! (`SHGetFileInfoW`), the icon theme's PNG on Linux (the freedesktop.org
//! Icon Theme Specification's `hicolor` theme and `/usr/share/pixmaps`).
//!
//! Every icon is decoded, scaled to at most [`ICON_SIZE`] pixels and
//! encoded again ([`normalize`]): the app only receives a small PNG this
//! crate made, never a file it names, and an icon is read only for an
//! application [`crate::discover`] listed.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use crate::desktop_entry::DesktopEntry;
use crate::xdg::XdgDirs;
use crate::{IdKind, InstalledApp, Platform};

/// Largest icon side, in pixels.
pub const ICON_SIZE: u32 = 64;
/// Largest icon file read or decoded, in bytes.
pub const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024;
/// Largest picture decoded, per side.
const MAX_SOURCE_SIDE: u32 = 2048;
/// Largest icon handed to the app, in bytes.
pub const MAX_ICON_BYTES: usize = 64 * 1024;

/// `png` decoded, scaled down to fit [`ICON_SIZE`] and encoded again;
/// `None` for anything that is not a sane PNG.
pub fn normalize(png: &[u8]) -> Option<Vec<u8>> {
    if png.len() as u64 > MAX_SOURCE_BYTES {
        return None;
    }
    let reader = image::ImageReader::with_format(Cursor::new(png), image::ImageFormat::Png);
    let (width, height) = reader.into_dimensions().ok()?;
    if width == 0 || height == 0 || width > MAX_SOURCE_SIDE || height > MAX_SOURCE_SIDE {
        return None;
    }
    let decoded = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?;
    let scaled = if width > ICON_SIZE || height > ICON_SIZE {
        decoded.resize(ICON_SIZE, ICON_SIZE, image::imageops::FilterType::Lanczos3)
    } else {
        decoded
    };
    let mut out = Vec::new();
    scaled
        .to_rgba8()
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    (out.len() <= MAX_ICON_BYTES).then_some(out)
}

/// The icon of `app`, as a normalized PNG, when the system has one.
pub fn icon(app: &InstalledApp, xdg: &XdgDirs) -> Option<Vec<u8>> {
    icon_in(Platform::current(), app, xdg)
}

/// As [`icon`], for `platform` (tests).
pub fn icon_in(platform: Platform, app: &InstalledApp, xdg: &XdgDirs) -> Option<Vec<u8>> {
    let path = PathBuf::from(app.path.as_deref()?);
    let raw = match platform {
        Platform::Linux if app.kind == IdKind::DesktopFile => {
            let text = read_bounded(&path)?;
            let entry = DesktopEntry::parse(&String::from_utf8(text).ok()?).ok()?;
            let file = theme_icon(entry.icon.as_deref()?, xdg)?;
            read_bounded(&file)?
        }
        // A network path (UNC, `\\?\UNC`) could reach another computer
        // and load its icon handlers: never drawn.
        Platform::Windows if is_network_path(&path) => return None,
        Platform::MacOs | Platform::Windows => native(&path)?,
        _ => return None,
    };
    normalize(&raw)
}

#[cfg(target_os = "macos")]
fn native(path: &Path) -> Option<Vec<u8>> {
    crate::native_macos_icon::icon_png(path)
}

#[cfg(windows)]
fn native(path: &Path) -> Option<Vec<u8>> {
    crate::native_windows_icon::icon_png(path)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn native(_path: &Path) -> Option<Vec<u8>> {
    None
}

/// `\\server\share\…`, `\\?\…` or `//server/…`.
fn is_network_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.starts_with("\\\\") || text.starts_with("//")
}

/// `path`, with its links resolved, when it is inside one of `roots`
/// (also with their links resolved).
fn contained(path: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    let real = std::fs::canonicalize(path).ok()?;
    roots
        .iter()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .any(|root| real.starts_with(root))
        .then_some(real)
}

fn read_bounded(path: &Path) -> Option<Vec<u8>> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_SOURCE_BYTES {
        return None;
    }
    std::fs::read(path).ok()
}

/// Sizes looked for, best first.
const SIZES: [&str; 8] = [
    "64x64", "48x48", "96x96", "128x128", "256x256", "32x32", "512x512", "scalable",
];

/// The PNG of icon `name` (a desktop entry's `Icon=`): in the `hicolor`
/// theme of each XDG data folder, then `/usr/share/pixmaps`. An absolute
/// `Icon=` counts only when it is a `.png` inside one of those folders.
/// Every candidate is checked where it really is: `..` is refused, and a
/// link that leads outside those folders is not followed. SVG icons are
/// not drawn (no renderer is bundled).
pub fn theme_icon(name: &str, xdg: &XdgDirs) -> Option<PathBuf> {
    let name = name.trim();
    if name.is_empty()
        || name.contains('\0')
        || Path::new(name)
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    let mut roots: Vec<PathBuf> = xdg
        .data_home
        .iter()
        .chain(&xdg.data_dirs)
        .map(|base| base.join("icons"))
        .collect();
    let pixmaps: Vec<PathBuf> = xdg
        .data_dirs
        .iter()
        .map(|base| base.join("pixmaps"))
        .collect();
    let all: Vec<PathBuf> = roots.iter().chain(&pixmaps).cloned().collect();
    let path = Path::new(name);
    if path.is_absolute() {
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("png"))
        {
            return None;
        }
        return contained(path, &all).filter(|real| real.is_file());
    }
    if name.contains(['/', '\\']) || name.starts_with('.') {
        return None;
    }
    let file = format!("{name}.png");
    for root in &roots {
        for size in SIZES {
            let candidate = root.join("hicolor").join(size).join("apps").join(&file);
            if candidate.is_file()
                && let Some(real) = contained(&candidate, &all)
            {
                return Some(real);
            }
        }
    }
    roots.clear();
    pixmaps
        .iter()
        .map(|dir| dir.join(&file))
        .filter(|candidate| candidate.is_file())
        .find_map(|candidate| contained(&candidate, &all))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(side: u32) -> Vec<u8> {
        let picture = image::RgbaImage::from_pixel(side, side, image::Rgba([200, 30, 30, 255]));
        let mut out = Vec::new();
        picture
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn icons_are_scaled_down_and_encoded_again() {
        let big = normalize(&png(512)).unwrap();
        let decoded = image::load_from_memory(&big).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (ICON_SIZE, ICON_SIZE));
        let small = normalize(&png(16)).unwrap();
        assert_eq!(image::load_from_memory(&small).unwrap().width(), 16);
        assert_eq!(normalize(b"not a png"), None);
        assert_eq!(normalize(&png(4096)), None);
    }

    #[test]
    fn theme_icons_come_from_the_icon_folders_only() {
        let data = tempfile::tempdir().unwrap();
        let apps = data.path().join("icons/hicolor/48x48/apps");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(apps.join("editor.png"), png(48)).unwrap();
        let pixmaps = data.path().join("pixmaps");
        std::fs::create_dir_all(&pixmaps).unwrap();
        std::fs::write(pixmaps.join("old.png"), png(32)).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.png"), png(8)).unwrap();
        let dirs = XdgDirs::only(&[data.path()]);
        let real = |p: PathBuf| std::fs::canonicalize(p).unwrap();
        assert_eq!(
            theme_icon("editor", &dirs),
            Some(real(apps.join("editor.png")))
        );
        assert_eq!(
            theme_icon("old", &dirs),
            Some(real(pixmaps.join("old.png")))
        );
        let inside = apps.join("editor.png");
        assert_eq!(
            theme_icon(&inside.to_string_lossy(), &dirs),
            Some(real(inside))
        );
        // `..` out of the icon folders, written as an absolute path.
        let escape = format!("{}/../../../../../secret.png", apps.display());
        assert_eq!(theme_icon(&escape, &dirs), None);
        assert_eq!(theme_icon("../secret", &dirs), None);
        // A link inside the theme that leads outside it.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path().join("secret.png"), apps.join("link.png"))
                .unwrap();
            assert_eq!(theme_icon("link", &dirs), None);
            let linked = apps.join("link.png");
            assert_eq!(theme_icon(&linked.to_string_lossy(), &dirs), None);
            std::os::unix::fs::symlink(
                outside.path().join("secret.png"),
                pixmaps.join("plink.png"),
            )
            .unwrap();
            assert_eq!(theme_icon("plink", &dirs), None);
        }
        for refused in [
            outside
                .path()
                .join("secret.png")
                .to_string_lossy()
                .into_owned(),
            "../../etc/passwd".to_owned(),
            "missing".to_owned(),
        ] {
            assert_eq!(theme_icon(&refused, &dirs), None, "{refused}");
        }

        // A desktop entry's icon, through `icon_in`.
        let entry = data.path().join("applications/editor.desktop");
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::fs::write(
            &entry,
            "[Desktop Entry]\nType=Application\nName=Editor\nExec=editor\nIcon=editor\n",
        )
        .unwrap();
        let app = InstalledApp {
            name: "Editor".into(),
            id: "editor.desktop".into(),
            kind: IdKind::DesktopFile,
            path: Some(entry.to_string_lossy().into_owned()),
            browser: false,
        };
        let icon = icon_in(Platform::Linux, &app, &dirs).unwrap();
        assert_eq!(image::load_from_memory(&icon).unwrap().width(), 48);
    }
}
