//! Homebrew image dylibs vs Apple ImageIO on macOS.
//!
//! Unsigned binaries honor `DYLD_LIBRARY_PATH`. ImageIO links
//! `…/Resources/libPng.dylib` (and JPEG/GIF/TIFF). On case-insensitive APFS a
//! Homebrew `libpng.dylib` in that path wins, and the first PNG decode SIGBUSes
//! at `0xbad4007` — typically an NSWindow edge-resize cursor (#1900, #533, #119).
//!
//! dyld applies the path at load, so unsetting it in `main` is too late. Re-exec
//! with colliding directories removed.

#[cfg(any(test, target_os = "macos"))]
use std::ffi::{OsStr, OsString};
#[cfg(any(test, target_os = "macos"))]
use std::path::{Path, PathBuf};

#[cfg(any(test, target_os = "macos"))]
const COLLIDING_LEAVES: &[&str] = &[
    "libpng.dylib",
    "libjpeg.dylib",
    "libgif.dylib",
    "libtiff.dylib",
    "libwebp.dylib",
    "libjp2.dylib",
];

#[cfg(target_os = "macos")]
const MARKER: &str = "BEARCAD_IMAGEIO_DYLD_SANITIZED";

/// Drop `DYLD_LIBRARY_PATH` entries that would shadow ImageIO's private codecs.
#[cfg(any(test, target_os = "macos"))]
pub fn strip_colliding_dyld_dirs(value: impl AsRef<OsStr>) -> OsString {
    let kept: Vec<PathBuf> = std::env::split_paths(value.as_ref())
        .filter(|p| !p.as_os_str().is_empty() && !dir_shadows_imageio(p))
        .collect();
    std::env::join_paths(&kept).unwrap_or_default()
}

#[cfg(any(test, target_os = "macos"))]
pub fn dyld_library_path_needs_strip(value: impl AsRef<OsStr>) -> bool {
    std::env::split_paths(value.as_ref()).any(|p| dir_shadows_imageio(&p))
}

#[cfg(any(test, target_os = "macos"))]
fn dir_shadows_imageio(dir: &Path) -> bool {
    COLLIDING_LEAVES.iter().any(|leaf| dir.join(leaf).exists())
}

/// Re-exec without ImageIO-colliding `DYLD_LIBRARY_PATH` entries. No-op elsewhere.
pub fn protect() {
    #[cfg(target_os = "macos")]
    protect_macos();
}

#[cfg(target_os = "macos")]
fn protect_macos() {
    let current = std::env::var_os("DYLD_LIBRARY_PATH");
    let needs = current
        .as_ref()
        .is_some_and(|v| dyld_library_path_needs_strip(v));
    if std::env::var_os(MARKER).is_some() {
        if needs {
            eprintln!(
                "bearcad: DYLD_LIBRARY_PATH still shadows ImageIO codecs; \
                 hovering a window edge may SIGBUS"
            );
        }
        return;
    }
    if !needs {
        return;
    }
    let cleaned = strip_colliding_dyld_dirs(current.as_ref().unwrap());
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("bearcad: cannot re-exec to protect ImageIO: {e}");
            return;
        }
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.args(std::env::args_os().skip(1));
    cmd.env(MARKER, "1");
    if cleaned.is_empty() {
        cmd.env_remove("DYLD_LIBRARY_PATH");
    } else {
        cmd.env("DYLD_LIBRARY_PATH", cleaned);
    }
    use std::os::unix::process::CommandExt;
    let err = cmd.exec();
    eprintln!("bearcad: re-exec to protect ImageIO failed: {err}");
}

/// Decode a 1×1 PNG through ImageIO (the crash path). Used by the self-test.
#[cfg(target_os = "macos")]
pub fn selftest_png_decode() -> Result<(), String> {
    // 1×1 RGB PNG.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8,
        0xcf, 0xc0, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x05, 0xfe, 0xd4, 0xef, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    unsafe {
        let data = CFDataCreate(std::ptr::null(), PNG.as_ptr(), PNG.len() as isize);
        if data.is_null() {
            return Err("CFDataCreate failed".into());
        }
        let src = CGImageSourceCreateWithData(data, std::ptr::null());
        CFRelease(data);
        if src.is_null() {
            return Err("CGImageSourceCreateWithData failed".into());
        }
        let img = CGImageSourceCreateImageAtIndex(src, 0, std::ptr::null());
        CFRelease(src);
        if img.is_null() {
            return Err("CGImageSourceCreateImageAtIndex failed".into());
        }
        let (w, h) = (CGImageGetWidth(img), CGImageGetHeight(img));
        CGImageRelease(img);
        if w != 1 || h != 1 {
            return Err(format!("unexpected PNG size {w}×{h}"));
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[link(name = "ImageIO", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CFDataCreate(
        allocator: *const std::ffi::c_void,
        bytes: *const u8,
        length: isize,
    ) -> *mut std::ffi::c_void;
    fn CFRelease(cf: *const std::ffi::c_void);
    fn CGImageSourceCreateWithData(
        data: *const std::ffi::c_void,
        options: *const std::ffi::c_void,
    ) -> *mut std::ffi::c_void;
    fn CGImageSourceCreateImageAtIndex(
        src: *const std::ffi::c_void,
        index: usize,
        options: *const std::ffi::c_void,
    ) -> *mut std::ffi::c_void;
    fn CGImageGetWidth(image: *const std::ffi::c_void) -> usize;
    fn CGImageGetHeight(image: *const std::ffi::c_void) -> usize;
    fn CGImageRelease(image: *const std::ffi::c_void);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_pair() -> (PathBuf, PathBuf, PathBuf) {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "bearcad-imageio-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let keep = root.join("keep");
        let drop = root.join("drop");
        fs::create_dir_all(&keep).unwrap();
        fs::create_dir_all(&drop).unwrap();
        (root, keep, drop)
    }

    #[test]
    fn strip_drops_dir_that_contains_libpng() {
        let (root, keep, drop) = temp_pair();
        fs::write(drop.join("libpng.dylib"), b"").unwrap();
        let value = std::env::join_paths([&drop, &keep]).unwrap();
        let stripped = strip_colliding_dyld_dirs(&value);
        let parts: Vec<_> = std::env::split_paths(&stripped).collect();
        let _ = fs::remove_dir_all(&root);
        assert_eq!(parts, vec![keep]);
    }

    #[test]
    fn strip_keeps_dirs_without_image_dylibs() {
        let (root, keep, other) = temp_pair();
        let value = std::env::join_paths([&keep, &other]).unwrap();
        let stripped = strip_colliding_dyld_dirs(&value);
        let parts: Vec<_> = std::env::split_paths(&stripped).collect();
        let _ = fs::remove_dir_all(&root);
        assert_eq!(parts, vec![keep, other]);
    }

    #[test]
    fn strip_empties_when_every_dir_collides() {
        let (root, _, drop) = temp_pair();
        fs::write(drop.join("libjpeg.dylib"), b"").unwrap();
        assert!(dyld_library_path_needs_strip(&drop));
        let stripped = strip_colliding_dyld_dirs(&drop);
        let _ = fs::remove_dir_all(&root);
        assert!(stripped.is_empty());
    }

    #[test]
    fn innocent_path_does_not_need_strip() {
        let (root, keep, _) = temp_pair();
        assert!(!dyld_library_path_needs_strip(&keep));
        let _ = fs::remove_dir_all(&root);
    }
}
