//! Register BearCAD as the app that opens `.bearcad` files on double-click (#1285).
//!
//! - **macOS:** document types live in the `.app` `Info.plist` (packaging). This module
//!   installs an Apple Event handler so Finder-open paths actually reach the app.
//! - **Linux:** FreeDesktop `.desktop` + MIME XML under `~/.local/share/…`.
//! - **Windows:** per-user `HKCU\Software\Classes` ProgID + open command.
//!
//! `bearcad install-cli` also registers associations; the GUI calls
//! [`ensure_registered`] once at launch so a portable Windows/Linux binary still works
//! without a separate install step. macOS registration is the bundled plist.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Document extension (no leading dot).
pub const EXTENSION: &str = "bearcad";
/// FreeDesktop / Windows MIME type.
pub const MIME_TYPE: &str = "application/x-bearcad";
/// macOS UTI exported by the app bundle (`Info.plist` UTExportedTypeDeclarations).
pub const UTI: &str = "com.bearcad.document";
/// Windows ProgID under `HKCU\Software\Classes`.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub const PROGID: &str = "BearCAD.document";
/// FreeDesktop desktop-file id (`*.desktop` basename without extension).
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
pub const DESKTOP_ID: &str = "com.bearcad.app";

/// Paths the OS asked us to open (Finder / file manager double-click, including while
/// the app is already running). Drained each frame by the app.
static PENDING_OPEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Queue a path the OS wants opened. Callers may pass non-`.bearcad` paths; the app
/// filters when draining.
pub fn queue_open_path(path: impl Into<String>) {
    let path = path.into();
    if path.is_empty() {
        return;
    }
    if let Ok(mut q) = PENDING_OPEN.lock() {
        if !q.iter().any(|p| p == &path) {
            q.push(path);
        }
    }
}

/// Take every path queued since the last drain.
pub fn drain_pending_open_paths() -> Vec<String> {
    PENDING_OPEN
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}

/// Register file associations for the running binary. Idempotent.
pub fn register() -> Result<String, String> {
    let exe = current_binary()?;
    register_with_exe(&exe)
}

/// Like [`register`] with an explicit executable path (tests inject a temp path).
pub fn register_with_exe(exe: &Path) -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        return register_linux(exe);
    }
    #[cfg(target_os = "windows")]
    {
        return register_windows(exe);
    }
    #[cfg(target_os = "macos")]
    {
        let _ = exe;
        // Association is declared in Info.plist for the .app bundle; no per-user write.
        Ok(format!(
            "macOS: .{EXTENSION} opens with BearCAD via the app bundle Info.plist \
             (UTI {UTI}; drag BearCAD.app into Applications)"
        ))
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = exe;
        Err("file association registration is not supported on this platform".into())
    }
}

/// Remove associations previously written by [`register`]. Idempotent.
pub fn unregister() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        return unregister_linux();
    }
    #[cfg(target_os = "windows")]
    {
        return unregister_windows();
    }
    #[cfg(target_os = "macos")]
    {
        Ok("macOS: nothing to unregister (association is the app bundle's Info.plist)".into())
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        Err("file association unregistration is not supported on this platform".into())
    }
}

/// Quiet, best-effort registration at GUI launch so double-click works after a portable
/// install (Windows/Linux). Logs failures; never aborts startup.
pub fn ensure_registered() {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        match register() {
            Ok(msg) => crate::diag::info(format!("file association: {msg}")),
            Err(err) => crate::diag::warn(format!("file association: {err}")),
        }
    }
}

fn current_binary() -> Result<PathBuf, String> {
    // Same resolve path as install-cli: canonical so re-runs stay stable.
    crate::cli_install::current_binary()
}

// ── Content generators (unit-tested on every OS; used by the matching platform) ─

/// FreeDesktop desktop entry body for `Exec={exe} %F`.
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
pub fn linux_desktop_entry(exe: &Path) -> String {
    let exec = shell_quote_path(exe);
    format!(
        "\
[Desktop Entry]
Type=Application
Name=BearCAD
Comment=Parametric CAD
Exec={exec} %F
Icon=bearcad
Terminal=false
Categories=Graphics;Engineering;Science;
MimeType={MIME_TYPE};
StartupWMClass=bearcad
"
    )
}

/// FreeDesktop shared-MIME-info package for `.bearcad`.
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
pub fn linux_mime_xml() -> String {
    format!(
        "\
<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">
  <mime-type type=\"{MIME_TYPE}\">
    <comment>BearCAD document</comment>
    <glob pattern=\"*.{EXTENSION}\"/>
  </mime-type>
</mime-info>
"
    )
}

/// Windows open-command registry value: `"C:\path\bearcad.exe" "%1"`.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub fn windows_open_command(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

/// Windows DefaultIcon value: `C:\path\bearcad.exe,0`.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub fn windows_default_icon(exe: &Path) -> String {
    format!("{},0", exe.display())
}

/// Quote a path for a FreeDesktop `Exec=` key (spaces → double quotes).
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
fn shell_quote_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.chars().any(|c| c.is_whitespace()) {
        format!("\"{s}\"")
    } else {
        s.into_owned()
    }
}

// ── Linux ────────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn xdg_data_home() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("XDG_DATA_HOME").filter(|s| !s.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| "HOME is not set".to_string())?;
    Ok(PathBuf::from(home).join(".local/share"))
}

#[cfg(target_os = "linux")]
fn linux_paths() -> Result<(PathBuf, PathBuf), String> {
    let base = xdg_data_home()?;
    let desktop = base
        .join("applications")
        .join(format!("{DESKTOP_ID}.desktop"));
    let mime = base
        .join("mime/packages")
        .join(format!("{DESKTOP_ID}.xml"));
    Ok((desktop, mime))
}

#[cfg(target_os = "linux")]
fn register_linux(exe: &Path) -> Result<String, String> {
    let (desktop_path, mime_path) = linux_paths()?;
    if let Some(parent) = desktop_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    if let Some(parent) = mime_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(&desktop_path, linux_desktop_entry(exe))
        .map_err(|e| format!("write {}: {e}", desktop_path.display()))?;
    std::fs::write(&mime_path, linux_mime_xml())
        .map_err(|e| format!("write {}: {e}", mime_path.display()))?;

    // Refresh caches when the tools are present (no error if missing — the files alone
    // are enough for many file managers after the next login).
    let data_home = xdg_data_home()?;
    let _ = std::process::Command::new("update-desktop-database")
        .arg(data_home.join("applications"))
        .status();
    let _ = std::process::Command::new("update-mime-database")
        .arg(data_home.join("mime"))
        .status();
    // Prefer BearCAD for this MIME type in the user default apps list.
    let _ = std::process::Command::new("xdg-mime")
        .args(["default", &format!("{DESKTOP_ID}.desktop"), MIME_TYPE])
        .status();

    Ok(format!(
        "registered .{EXTENSION} → {} (desktop {}, mime {})",
        exe.display(),
        desktop_path.display(),
        mime_path.display()
    ))
}

#[cfg(target_os = "linux")]
fn unregister_linux() -> Result<String, String> {
    let (desktop_path, mime_path) = linux_paths()?;
    for p in [&desktop_path, &mime_path] {
        match std::fs::remove_file(p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot remove {}: {e}", p.display())),
        }
    }
    if let Ok(data_home) = xdg_data_home() {
        let _ = std::process::Command::new("update-desktop-database")
            .arg(data_home.join("applications"))
            .status();
        let _ = std::process::Command::new("update-mime-database")
            .arg(data_home.join("mime"))
            .status();
    }
    Ok(format!("unregistered .{EXTENSION} file association"))
}

// ── Windows ──────────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn register_windows(exe: &Path) -> Result<String, String> {
    let exe_str = exe
        .to_str()
        .ok_or_else(|| "executable path is not valid UTF-8".to_string())?;
    // Per-user classes — no admin required.
    reg_add(r"HKCU\Software\Classes\.bearcad", None, PROGID)?;
    reg_add(
        r"HKCU\Software\Classes\BearCAD.document",
        None,
        "BearCAD Document",
    )?;
    reg_add(
        r"HKCU\Software\Classes\BearCAD.document\DefaultIcon",
        None,
        &windows_default_icon(exe),
    )?;
    reg_add(
        r"HKCU\Software\Classes\BearCAD.document\shell\open\command",
        None,
        &windows_open_command(exe),
    )?;
    // Content type helps Explorer / browsers identify the file.
    reg_add(
        r"HKCU\Software\Classes\.bearcad",
        Some("Content Type"),
        MIME_TYPE,
    )?;
    let _ = exe_str;
    Ok(format!(
        "registered .{EXTENSION} → {exe_str} (HKCU ProgID {PROGID})"
    ))
}

#[cfg(target_os = "windows")]
fn unregister_windows() -> Result<String, String> {
    // Delete the extension and the ProgID tree. /f = force, ignore missing.
    reg_delete(r"HKCU\Software\Classes\.bearcad")?;
    reg_delete(r"HKCU\Software\Classes\BearCAD.document")?;
    Ok(format!("unregistered .{EXTENSION} file association"))
}

/// `reg add key [/v valueName] /d data /f` — Windows built-in, no extra crate.
#[cfg(target_os = "windows")]
fn reg_add(key: &str, value_name: Option<&str>, data: &str) -> Result<(), String> {
    let mut cmd = std::process::Command::new("reg");
    cmd.arg("add").arg(key);
    match value_name {
        Some(name) => {
            cmd.arg("/v").arg(name);
        }
        None => {
            cmd.arg("/ve");
        }
    }
    cmd.arg("/d").arg(data).arg("/f");
    let out = cmd
        .output()
        .map_err(|e| format!("failed to run reg.exe: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("reg add {key} failed: {stderr}"));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn reg_delete(key: &str) -> Result<(), String> {
    let out = std::process::Command::new("reg")
        .args(["delete", key, "/f"])
        .output()
        .map_err(|e| format!("failed to run reg.exe: {e}"))?;
    // Missing key is success for our purposes (reg returns 1).
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
        if stderr.contains("unable to find") || stderr.contains("cannot find") {
            return Ok(());
        }
        // Still treat as ok when the key is already gone.
        let _ = stderr;
    }
    Ok(())
}

// ── macOS open-documents Apple Event ─────────────────────────────────────────

/// Install a handler for Finder "open documents" events so double-clicked `.bearcad`
/// files reach [`queue_open_path`]. Safe to call more than once; keeps the handler alive
/// for the process lifetime.
#[cfg(target_os = "macos")]
pub fn install_open_documents_handler() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if let Err(e) = install_open_documents_handler_inner() {
            crate::diag::warn(format!("open-documents handler: {e}"));
        }
    });
}

#[cfg(not(target_os = "macos"))]
pub fn install_open_documents_handler() {}

#[cfg(target_os = "macos")]
fn install_open_documents_handler_inner() -> Result<(), String> {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{define_class, msg_send, sel, MainThreadOnly};
    use objc2_foundation::{NSObject, NSObjectProtocol};

    // 'aevt' / 'odoc' / '----' as big-endian FourCCs (AEEventClass / AEEventID / AEKeyword).
    const K_CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
    const K_AE_OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
    const KEY_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "BearCADOpenDocumentsHandler"]
        struct OpenDocumentsHandler;

        impl OpenDocumentsHandler {
            #[unsafe(method(handleOpenDocuments:withReplyEvent:))]
            fn handle_open_documents(&self, event: *mut AnyObject, _reply: *mut AnyObject) {
                if event.is_null() {
                    return;
                }
                // Safety: AppKit hands a live NSAppleEventDescriptor.
                let event: &AnyObject = unsafe { &*event };
                let paths = paths_from_open_event(event, KEY_DIRECT_OBJECT);
                for path in paths {
                    queue_open_path(path);
                }
            }
        }

        unsafe impl NSObjectProtocol for OpenDocumentsHandler {}
    );

    fn paths_from_open_event(event: &AnyObject, key: u32) -> Vec<String> {
        // paramDescriptorForKeyword: → list of file descriptors
        let list: *mut AnyObject =
            unsafe { msg_send![event, paramDescriptorForKeyword: key] };
        if list.is_null() {
            return Vec::new();
        }
        let list: &AnyObject = unsafe { &*list };
        let count: isize = unsafe { msg_send![list, numberOfItems] };
        let mut out = Vec::new();
        // Apple Event lists are 1-indexed.
        for i in 1..=count {
            let item: *mut AnyObject = unsafe { msg_send![list, descriptorAtIndex: i] };
            if item.is_null() {
                continue;
            }
            let item: &AnyObject = unsafe { &*item };
            // Prefer fileURLValue (NSURL), fall back to stringValue.
            let url: *mut AnyObject = unsafe { msg_send![item, fileURLValue] };
            if !url.is_null() {
                let url: &AnyObject = unsafe { &*url };
                let path: *mut AnyObject = unsafe { msg_send![url, path] };
                if !path.is_null() {
                    if let Some(s) = nsstring_to_rust(path) {
                        out.push(s);
                        continue;
                    }
                }
            }
            let s: *mut AnyObject = unsafe { msg_send![item, stringValue] };
            if !s.is_null() {
                if let Some(s) = nsstring_to_rust(s) {
                    // May be a file:// URL or a path.
                    if let Some(path) = s.strip_prefix("file://") {
                        out.push(urlencoding_lite_decode(path));
                    } else {
                        out.push(s);
                    }
                }
            }
        }
        out
    }

    fn nsstring_to_rust(s: *mut AnyObject) -> Option<String> {
        let cstr: *const std::ffi::c_char = unsafe { msg_send![s, UTF8String] };
        if cstr.is_null() {
            return None;
        }
        let s = unsafe { std::ffi::CStr::from_ptr(cstr) };
        Some(s.to_string_lossy().into_owned())
    }

    /// Minimal percent-decode for file:// paths (space and common escapes).
    fn urlencoding_lite_decode(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'%' if i + 2 < bytes.len() => {
                    let hex = |b: u8| (b as char).to_digit(16);
                    match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                        (Some(hi), Some(lo)) => {
                            out.push((hi * 16 + lo) as u8);
                            i += 3;
                            continue;
                        }
                        _ => out.push(bytes[i]),
                    }
                }
                b'+' => out.push(b' '),
                b => out.push(b),
            }
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    // Allocate the handler on the main thread and leak a retain so AppKit can call it.
    let mtm = objc2::MainThreadMarker::new()
        .ok_or_else(|| "open-documents handler must be installed on the main thread".to_string())?;
    let handler = OpenDocumentsHandler::alloc(mtm);
    let handler: Retained<OpenDocumentsHandler> = unsafe { msg_send![handler, init] };

    let manager: *mut AnyObject =
        unsafe { msg_send![objc2::class!(NSAppleEventManager), sharedAppleEventManager] };
    if manager.is_null() {
        return Err("NSAppleEventManager.sharedAppleEventManager returned nil".into());
    }
    let sel = sel!(handleOpenDocuments:withReplyEvent:);
    let handler_obj: &AnyObject = handler.as_ref();
    let _: () = unsafe {
        msg_send![
            &*manager,
            setEventHandler: handler_obj,
            andSelector: sel,
            forEventClass: K_CORE_EVENT_CLASS,
            andEventID: K_AE_OPEN_DOCUMENTS
        ]
    };

    // Keep the handler alive for the process lifetime.
    std::mem::forget(handler);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn linux_desktop_entry_names_mime_and_exec() {
        let body = linux_desktop_entry(Path::new("/opt/BearCAD/bearcad"));
        assert!(body.contains("MimeType=application/x-bearcad;"), "{body}");
        assert!(body.contains("Exec=/opt/BearCAD/bearcad %F"), "{body}");
        assert!(body.contains("Name=BearCAD"), "{body}");
        assert!(body.contains("Type=Application"), "{body}");
    }

    #[test]
    fn linux_desktop_entry_quotes_paths_with_spaces() {
        let body = linux_desktop_entry(Path::new("/opt/Bear CAD/bearcad"));
        assert!(
            body.contains("Exec=\"/opt/Bear CAD/bearcad\" %F"),
            "{body}"
        );
    }

    #[test]
    fn linux_mime_xml_maps_extension() {
        let xml = linux_mime_xml();
        assert!(xml.contains("application/x-bearcad"), "{xml}");
        assert!(xml.contains("*.bearcad"), "{xml}");
        assert!(xml.contains("BearCAD document"), "{xml}");
    }

    #[test]
    fn windows_open_command_quotes_exe_and_arg() {
        let cmd = windows_open_command(Path::new(r"C:\Program Files\BearCAD\bearcad.exe"));
        assert_eq!(cmd, r#""C:\Program Files\BearCAD\bearcad.exe" "%1""#);
    }

    #[test]
    fn windows_default_icon_uses_exe_index_zero() {
        let icon = windows_default_icon(Path::new(r"C:\BearCAD\bearcad.exe"));
        assert_eq!(icon, r"C:\BearCAD\bearcad.exe,0");
    }

    #[test]
    fn association_constants_are_stable() {
        assert_eq!(EXTENSION, "bearcad");
        assert_eq!(MIME_TYPE, "application/x-bearcad");
        assert_eq!(UTI, "com.bearcad.document");
        assert_eq!(PROGID, "BearCAD.document");
        assert_eq!(DESKTOP_ID, "com.bearcad.app");
    }

    /// #1290: the QuickLook Preview Extension Info.plist must claim the same UTI the
    /// app exports, or Space-bar preview silently never fires for `.bearcad` files.
    #[test]
    fn quicklook_extension_plist_claims_document_uti() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let plist = std::fs::read_to_string(root.join("macos/quicklook/Info.plist"))
            .expect("macos/quicklook/Info.plist must exist");
        assert!(
            plist.contains(UTI),
            "QLSupportedContentTypes must include {UTI}:\n{plist}"
        );
        assert!(
            plist.contains("com.apple.quicklook.preview"),
            "must be a QuickLook preview extension:\n{plist}"
        );
        assert!(
            plist.contains("com.bearcad.app.quicklook"),
            "extension bundle id must nest under com.bearcad.app:\n{plist}"
        );
        // Package script must inject the appex into the .app bundle.
        let pkg = std::fs::read_to_string(root.join("scripts/package-release.sh"))
            .expect("package-release.sh");
        assert!(
            pkg.contains("build-macos-quicklook.sh"),
            "package-release.sh must build the QuickLook appex"
        );
        assert!(
            pkg.contains("PlugIns/BearCADQuickLook.appex"),
            "package-release.sh must place the appex under Contents/PlugIns"
        );
    }

    #[test]
    fn queue_and_drain_pending_open_paths() {
        // Isolate from other tests: drain first.
        let _ = drain_pending_open_paths();
        queue_open_path("/tmp/a.bearcad");
        queue_open_path("/tmp/a.bearcad"); // dedupe
        queue_open_path("/tmp/b.bearcad");
        let got = drain_pending_open_paths();
        assert_eq!(got, vec!["/tmp/a.bearcad", "/tmp/b.bearcad"]);
        assert!(drain_pending_open_paths().is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn register_linux_writes_desktop_and_mime_under_xdg_data_home() {
        let root = std::env::temp_dir().join(format!(
            "bearcad_file_assoc_{}_{}",
            std::process::id(),
            // Clock via `crate::time` — raw `std::time::SystemTime` is banned (#1048).
            crate::time::SystemTime::now()
                .duration_since(crate::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        // Point XDG_DATA_HOME at the temp root so we never touch the real home.
        // SAFETY: test-only, single-threaded test process for this env key.
        std::env::set_var("XDG_DATA_HOME", &root);
        let exe = root.join("fake-bearcad");
        std::fs::write(&exe, b"#!/bin/sh\n").unwrap();

        let msg = register_with_exe(&exe).expect("register");
        assert!(msg.contains("registered"), "{msg}");

        let desktop = root.join("applications/com.bearcad.app.desktop");
        let mime = root.join("mime/packages/com.bearcad.app.xml");
        let desktop_body = std::fs::read_to_string(&desktop).unwrap();
        let mime_body = std::fs::read_to_string(&mime).unwrap();
        assert!(desktop_body.contains("application/x-bearcad"), "{desktop_body}");
        assert!(desktop_body.contains(&exe.display().to_string()), "{desktop_body}");
        assert!(mime_body.contains("*.bearcad"), "{mime_body}");

        unregister().expect("unregister");
        assert!(!desktop.exists());
        assert!(!mime.exists());

        std::env::remove_var("XDG_DATA_HOME");
        let _ = std::fs::remove_dir_all(&root);
    }
}
