//! Homebrew `libpng.dylib` on `DYLD_LIBRARY_PATH` makes ImageIO SIGBUS at 0xbad4007
//! when it decodes a PNG (window-edge resize cursors). #1900

#![cfg(target_os = "macos")]

use std::path::Path;
use std::process::Command;

#[test]
fn homebrew_dyld_library_path_does_not_sigbus_imageio() {
    let lib = Path::new("/opt/homebrew/lib");
    if !lib.join("libpng.dylib").exists() {
        eprintln!("skip: no Homebrew libpng at {}", lib.display());
        return;
    }
    let exe = env!("CARGO_BIN_EXE_bearcad");
    let out = Command::new(exe)
        .env("DYLD_LIBRARY_PATH", lib)
        .env("BEARCAD_IMAGEIO_SELFTEST", "1")
        .output()
        .expect("spawn bearcad");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "ImageIO PNG decode SIGBUS under Homebrew DYLD_LIBRARY_PATH: status={:?} stdout={stdout} stderr={stderr}",
        out.status
    );
    assert!(
        stdout.contains("imageio-ok"),
        "expected imageio-ok, stdout={stdout} stderr={stderr}"
    );
}
