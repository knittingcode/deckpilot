//! "USB present, HID unavailable" (DP-USB-03): a Stream Deck the USB bus
//! lists without a `hidraw` interface, e.g. when the kernel could not set
//! its configuration behind a hub (`-28`, not enough bandwidth). hidapi
//! sees nothing then, so the device would look unplugged.
//!
//! Linux only, read-only: reads the sysfs attributes `idVendor`,
//! `idProduct` and `serial` of each USB device (world-readable) and looks
//! for a `hidraw` directory below it. Nothing is opened or written, and no
//! USB request is sent.

use std::path::Path;

/// Where Linux lists USB devices.
#[cfg(target_os = "linux")]
pub const SYSFS_USB_DEVICES: &str = "/sys/bus/usb/devices";

/// A USB device of `vendor` with a product of `products` that has no
/// `hidraw` interface: its product id and serial number. Devices without
/// a serial number are left out (no identity to report them under).
pub fn without_hid(root: &Path, vendor: u16, products: &[u16]) -> Vec<(u16, String)> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        // Interfaces (`1-2:1.0`) have no idVendor; only devices do.
        let Some(id_vendor) = read_hex(&dir.join("idVendor")) else {
            continue;
        };
        let Some(product) = read_hex(&dir.join("idProduct")) else {
            continue;
        };
        if id_vendor != vendor || !products.contains(&product) {
            continue;
        }
        let Some(serial) = std::fs::read_to_string(dir.join("serial"))
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if !has_hidraw(&dir, 4) {
            found.push((product, serial));
        }
    }
    found.sort();
    found.dedup();
    found
}

fn read_hex(path: &Path) -> Option<u16> {
    let text = std::fs::read_to_string(path).ok()?;
    u16::from_str_radix(text.trim(), 16).ok()
}

/// Whether a directory named `hidraw` is below `dir`, at most `depth`
/// levels down (`<dev>/<dev>:1.0/0003:0FD9:0080.0001/hidraw/hidraw3`).
/// Symbolic links are not followed, so the walk stays inside the device.
fn has_hidraw(dir: &Path, depth: u32) -> bool {
    if depth == 0 {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let Ok(kind) = entry.file_type() else {
            return false;
        };
        kind.is_dir() && (entry.file_name() == "hidraw" || has_hidraw(&entry.path(), depth - 1))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tree(std::path::PathBuf);

    impl Tree {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "deckpilot-sysfs-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn device(
            &self,
            name: &str,
            vendor: &str,
            product: &str,
            serial: Option<&str>,
        ) -> std::path::PathBuf {
            let dir = self.0.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("idVendor"), format!("{vendor}\n")).unwrap();
            std::fs::write(dir.join("idProduct"), format!("{product}\n")).unwrap();
            if let Some(serial) = serial {
                std::fs::write(dir.join("serial"), format!("{serial}\n")).unwrap();
            }
            dir
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn finds_a_stream_deck_without_hidraw_only() {
        let tree = Tree::new();
        // With its HID interface: not reported.
        let ok = tree.device("1-2", "0fd9", "0080", Some("AL1"));
        std::fs::create_dir_all(ok.join("1-2:1.0/0003:0FD9:0080.0001/hidraw/hidraw3")).unwrap();
        // The DP-USB-03 signature: listed on USB, no interface bound.
        tree.device("3-1.4", "0fd9", "0080", Some("AL2"));
        // An interface directory of it has no idVendor.
        std::fs::create_dir_all(tree.0.join("3-1.4:1.0")).unwrap();
        // Another vendor, another product, no serial: ignored.
        tree.device("1-3", "046d", "0080", Some("X"));
        tree.device("1-4", "0fd9", "006c", Some("XL1"));
        tree.device("1-5", "0fd9", "0080", None);

        assert_eq!(
            without_hid(&tree.0, 0x0fd9, &[0x0080]),
            vec![(0x0080, "AL2".to_owned())]
        );
        assert_eq!(
            without_hid(&tree.0.join("missing"), 0x0fd9, &[0x0080]),
            vec![]
        );
    }
}
