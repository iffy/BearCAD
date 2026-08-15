//! Register BearCAD as the app that opens `.bearcad` files on double-click (#1285, #1326).
//!
//! - **macOS:** document types live in the `.app` `Info.plist` (packaging). Launch
//!   Services starts the bundle; Finder then delivers the path as `application:openURLs:`
//!   (or an `odoc` Apple Event), **not** argv. The handler is attached at
//!   `applicationWillFinishLaunching:` so the launch-time event is not lost after
//!   winit creates `NSApplication`.
//! - **Linux:** FreeDesktop `.desktop` + MIME XML under `~/.local/share/…`.
//! - **Windows:** per-user `HKCU\Software\Classes` ProgID + open command.
//!
//! `bearcad install-cli` also registers associations; the GUI calls
//! [`ensure_registered`] once at launch so a portable Windows/Linux binary still works
//! without a separate install step. macOS association is the bundled plist.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Document extension (no leading dot).
pub const EXTENSION: &str = "bearcad";
/// FreeDesktop / Windows MIME type.
pub const MIME_TYPE: &str = "application/x-bearcad";
/// macOS UTI exported by the app bundle (`Info.plist` UTExportedTypeDeclarations).
#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
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
/// filters when draining. `file://` URLs are decoded to filesystem paths.
///
/// Filled by the macOS open-documents handler today; other platforms open via argv.
/// Kept unconditional so tests can exercise the queue on every OS.
#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
pub fn queue_open_path(path: impl Into<String>) {
    let raw = path.into();
    let path = path_from_os_open_spec(&raw).unwrap_or_else(|| raw.trim().to_string());
    if path.is_empty() {
        return;
    }
    if let Ok(mut q) = PENDING_OPEN.lock() {
        if !q.iter().any(|p| p == &path) {
            q.push(path);
        }
    }
    wake_ui();
}

/// Take every path queued since the last drain.
pub fn drain_pending_open_paths() -> Vec<String> {
    PENDING_OPEN
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}

/// True if `path` is a BearCAD document (`.bearcad`, any case).
pub fn is_document_path(path: &str) -> bool {
    let path = path.trim();
    if path.is_empty() {
        return false;
    }
    Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

/// Decode a Finder / argv / `file://` spec to a filesystem path.
///
/// Empty input is `None`. `file://localhost/…` and percent-escapes are unfolded so
/// Launch Services URLs and a typed path share one opener.
pub fn path_from_os_open_spec(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(rest) = raw.strip_prefix("file://") {
        let decoded = percent_decode(rest);
        let path = decoded
            .strip_prefix("localhost")
            .unwrap_or(decoded.as_str());
        if path.is_empty() {
            return None;
        }
        Some(path.to_string())
    } else {
        Some(raw.to_string())
    }
}

/// Pending OS-open paths that are BearCAD documents, already decoded.
pub fn take_os_open_documents() -> Vec<String> {
    drain_pending_open_paths()
        .into_iter()
        .filter_map(|p| path_from_os_open_spec(&p))
        .filter(|p| is_document_path(p))
        .collect()
}

/// Wake the egui loop so a Finder-open arriving while the app is idle is drained.
pub fn install_repaint_context(ctx: egui::Context) {
    if let Ok(mut slot) = EGUI_CTX.lock() {
        *slot = Some(ctx);
    }
}

static EGUI_CTX: Mutex<Option<egui::Context>> = Mutex::new(None);

fn wake_ui() {
    if let Ok(slot) = EGUI_CTX.lock() {
        if let Some(ctx) = slot.as_ref() {
            ctx.request_repaint();
        }
    }
}

/// Minimal percent-decode for `file://` paths (space and common escapes).
fn percent_decode(s: &str) -> String {
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
/// install (Windows/Linux). On macOS, re-registers the running `.app` with Launch
/// Services so a replaced bundle still owns `.bearcad`. Logs failures; never aborts startup.
pub fn ensure_registered() {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        match register() {
            Ok(msg) => crate::diag::info(format!("file association: {msg}")),
            Err(err) => crate::diag::warn(format!("file association: {err}")),
        }
    }
    #[cfg(target_os = "macos")]
    {
        register_bundle_with_launch_services();
    }
}

fn current_binary() -> Result<PathBuf, String> {
    // Canonical so re-runs stay stable. Kept local so wasm (no installer module) compiles.
    let exe = std::env::current_exe().map_err(|e| format!("cannot find current executable: {e}"))?;
    std::fs::canonicalize(&exe).map_err(|e| format!("cannot resolve {}: {e}", exe.display()))
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

// ── macOS open-documents (Finder double-click) ───────────────────────────────

/// Install Finder open-file hooks. Safe to call more than once.
///
/// Must be registered **before** `eframe::run_native` so we observe
/// `NSApplicationWillFinishLaunchingNotification`. winit's `EventLoop::new` creates
/// `NSApplication` and claims `odoc`; the launch document is dispatched *after*
/// willFinishLaunching, which is when we add `application:openURLs:` to winit's
/// delegate (it does not implement that method).
#[cfg(target_os = "macos")]
pub fn install_open_documents_handler() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if let Err(e) = register_will_finish_launching_observer() {
            crate::diag::warn(format!("open-documents handler: {e}"));
        }
    });
    attach_open_file_hooks();
}

#[cfg(not(target_os = "macos"))]
pub fn install_open_documents_handler() {}

#[cfg(target_os = "macos")]
fn register_bundle_with_launch_services() {
    use objc2::runtime::AnyObject;
    use objc2::msg_send;

    let bundle: *mut AnyObject = unsafe { msg_send![objc2::class!(NSBundle), mainBundle] };
    if bundle.is_null() {
        return;
    }
    let url: *mut AnyObject = unsafe { msg_send![bundle, bundleURL] };
    if url.is_null() {
        return;
    }
    let path_ns: *mut AnyObject = unsafe { msg_send![url, path] };
    let Some(path) = nsstring_to_rust(path_ns) else {
        return;
    };
    if !path.contains(".app") {
        return;
    }
    // NSURL is toll-free bridged to CFURLRef. `inUpdate = true` refreshes bindings
    // after the user replaces BearCAD.app.
    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        fn LSRegisterURL(url: *const std::ffi::c_void, update: u8) -> i32;
    }
    let status = unsafe { LSRegisterURL(url.cast(), 1) };
    if status != 0 {
        crate::diag::warn(format!("LSRegisterURL failed: {status}"));
    } else {
        crate::diag::info(format!("file association: registered bundle {path}"));
    }
}

#[cfg(target_os = "macos")]
fn attach_open_file_hooks() {
    if add_open_urls_to_app_delegate() {
        return;
    }
    // Fallback if the delegate class is not up yet, or method add failed: steal `odoc`.
    if objc2::runtime::AnyClass::get(c"WinitApplicationDelegate").is_some() {
        if let Err(e) = install_odoc_apple_event_handler() {
            crate::diag::warn(format!("open-documents Apple Event handler: {e}"));
        }
    }
}

/// Add `application:openURLs:` / `openFile:` / `openFiles:` to winit's
/// `WinitApplicationDelegate`. NSApplication converts the launch `odoc` into those
/// methods; winit 0.30 does not implement them, so Finder-open was dropped.
#[cfg(target_os = "macos")]
fn add_open_urls_to_app_delegate() -> bool {
    use objc2::ffi::{self as objc_ffi};
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
    use objc2::sel;

    let Some(cls) = AnyClass::get(c"WinitApplicationDelegate") else {
        return false;
    };
    let cls = cls as *const AnyClass as *mut AnyClass;

    unsafe extern "C-unwind" fn application_open_urls(
        _this: *mut AnyObject,
        _cmd: Sel,
        _app: *mut AnyObject,
        urls: *mut AnyObject,
    ) {
        for path in nsarray_urls_to_paths(urls) {
            queue_open_path(path);
        }
    }

    unsafe extern "C-unwind" fn application_open_file(
        _this: *mut AnyObject,
        _cmd: Sel,
        _app: *mut AnyObject,
        filename: *mut AnyObject,
    ) -> Bool {
        if let Some(s) = nsstring_to_rust(filename) {
            queue_open_path(s);
        }
        Bool::YES
    }

    unsafe extern "C-unwind" fn application_open_files(
        _this: *mut AnyObject,
        _cmd: Sel,
        _app: *mut AnyObject,
        filenames: *mut AnyObject,
    ) {
        for path in nsarray_strings_to_paths(filenames) {
            queue_open_path(path);
        }
    }

    // `application:openURLs:` is the modern Finder path (macOS 10.13+).
    let added_urls = unsafe {
        objc_ffi::class_addMethod(
            cls,
            sel!(application:openURLs:),
            std::mem::transmute::<
                unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
                Imp,
            >(application_open_urls),
            c"v@:@@".as_ptr(),
        )
    };
    let _ = unsafe {
        objc_ffi::class_addMethod(
            cls,
            sel!(application:openFile:),
            std::mem::transmute::<
                unsafe extern "C-unwind" fn(
                    *mut AnyObject,
                    Sel,
                    *mut AnyObject,
                    *mut AnyObject,
                ) -> Bool,
                Imp,
            >(application_open_file),
            c"B@:@@".as_ptr(),
        )
    };
    let _ = unsafe {
        objc_ffi::class_addMethod(
            cls,
            sel!(application:openFiles:),
            std::mem::transmute::<
                unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
                Imp,
            >(application_open_files),
            c"v@:@@".as_ptr(),
        )
    };

    // Success if we just added the method, or it was already added on a prior attach.
    added_urls.as_bool() || class_has_sel(cls, sel!(application:openURLs:))
}

#[cfg(target_os = "macos")]
fn class_has_sel(cls: *const objc2::runtime::AnyClass, sel: objc2::runtime::Sel) -> bool {
    use objc2::msg_send;
    if cls.is_null() {
        return false;
    }
    let has: bool = unsafe { msg_send![cls, instancesRespondToSelector: sel] };
    has
}

#[cfg(target_os = "macos")]
fn nsarray_urls_to_paths(arr: *mut objc2::runtime::AnyObject) -> Vec<String> {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    if arr.is_null() {
        return Vec::new();
    }
    let count: usize = unsafe { msg_send![arr, count] };
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let item: *mut AnyObject = unsafe { msg_send![arr, objectAtIndex: i] };
        if item.is_null() {
            continue;
        }
        let path_ns: *mut AnyObject = unsafe { msg_send![item, path] };
        if let Some(s) = nsstring_to_rust(path_ns) {
            out.push(s);
            continue;
        }
        let abs: *mut AnyObject = unsafe { msg_send![item, absoluteString] };
        if let Some(s) = nsstring_to_rust(abs) {
            if let Some(p) = path_from_os_open_spec(&s) {
                out.push(p);
            }
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn nsarray_strings_to_paths(arr: *mut objc2::runtime::AnyObject) -> Vec<String> {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    if arr.is_null() {
        return Vec::new();
    }
    let count: usize = unsafe { msg_send![arr, count] };
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let item: *mut AnyObject = unsafe { msg_send![arr, objectAtIndex: i] };
        if let Some(s) = nsstring_to_rust(item) {
            if let Some(p) = path_from_os_open_spec(&s) {
                out.push(p);
            }
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn nsstring_to_rust(s: *mut objc2::runtime::AnyObject) -> Option<String> {
    use objc2::msg_send;
    if s.is_null() {
        return None;
    }
    let cstr: *const std::ffi::c_char = unsafe { msg_send![s, UTF8String] };
    if cstr.is_null() {
        return None;
    }
    let s = unsafe { std::ffi::CStr::from_ptr(cstr) };
    Some(s.to_string_lossy().into_owned())
}

#[cfg(target_os = "macos")]
fn register_will_finish_launching_observer() -> Result<(), String> {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{define_class, msg_send, sel, MainThreadOnly};
    use objc2_foundation::{NSObject, NSObjectProtocol};

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "BearCADOpenFileLaunchObserver"]
        struct OpenFileLaunchObserver;

        impl OpenFileLaunchObserver {
            #[unsafe(method(appWillFinishLaunching:))]
            fn app_will_finish_launching(&self, _notification: *mut AnyObject) {
                // NSApplicationWillFinishLaunchingNotification: NSApp exists, winit's
                // delegate is set, the queued launch `odoc` has not been dispatched yet.
                attach_open_file_hooks();
            }

            #[unsafe(method(handleOpenDocuments:withReplyEvent:))]
            fn handle_open_documents(&self, event: *mut AnyObject, _reply: *mut AnyObject) {
                if event.is_null() {
                    return;
                }
                let event: &AnyObject = unsafe { &*event };
                for path in paths_from_odoc_event(event) {
                    queue_open_path(path);
                }
            }
        }

        unsafe impl NSObjectProtocol for OpenFileLaunchObserver {}
    );

    let mtm = objc2::MainThreadMarker::new()
        .ok_or_else(|| "open-documents handler must be installed on the main thread".to_string())?;
    let observer = OpenFileLaunchObserver::alloc(mtm);
    let observer: Retained<OpenFileLaunchObserver> = unsafe { msg_send![observer, init] };

    let center: *mut AnyObject =
        unsafe { msg_send![objc2::class!(NSNotificationCenter), defaultCenter] };
    if center.is_null() {
        return Err("NSNotificationCenter.defaultCenter returned nil".into());
    }
    let name: *mut AnyObject = unsafe {
        msg_send![
            objc2::class!(NSString),
            stringWithUTF8String: c"NSApplicationWillFinishLaunchingNotification".as_ptr()
        ]
    };
    let observer_obj: &AnyObject = observer.as_ref();
    let _: () = unsafe {
        msg_send![
            &*center,
            addObserver: observer_obj,
            selector: sel!(appWillFinishLaunching:),
            name: name,
            object: std::ptr::null::<AnyObject>()
        ]
    };

    // Also keep this object as the `odoc` target if we have to steal the Apple Event.
    if let Ok(mut slot) = ODOC_TARGET.lock() {
        *slot = Some(observer_obj as *const AnyObject as usize);
    }
    std::mem::forget(observer);
    Ok(())
}

#[cfg(target_os = "macos")]
static ODOC_TARGET: Mutex<Option<usize>> = Mutex::new(None);

#[cfg(target_os = "macos")]
fn install_odoc_apple_event_handler() -> Result<(), String> {
    use objc2::runtime::AnyObject;
    use objc2::{msg_send, sel};

    const K_CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
    const K_AE_OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");

    let target = ODOC_TARGET
        .lock()
        .ok()
        .and_then(|g| *g)
        .ok_or_else(|| "open-documents observer was not created".to_string())?;
    let handler_obj = target as *const AnyObject;
    if handler_obj.is_null() {
        return Err("open-documents observer pointer is null".into());
    }

    let manager: *mut AnyObject =
        unsafe { msg_send![objc2::class!(NSAppleEventManager), sharedAppleEventManager] };
    if manager.is_null() {
        return Err("NSAppleEventManager.sharedAppleEventManager returned nil".into());
    }
    let sel = sel!(handleOpenDocuments:withReplyEvent:);
    let _: () = unsafe {
        msg_send![
            &*manager,
            setEventHandler: handler_obj,
            andSelector: sel,
            forEventClass: K_CORE_EVENT_CLASS,
            andEventID: K_AE_OPEN_DOCUMENTS
        ]
    };
    Ok(())
}

#[cfg(target_os = "macos")]
fn paths_from_odoc_event(event: &objc2::runtime::AnyObject) -> Vec<String> {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    const KEY_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
    let list: *mut AnyObject =
        unsafe { msg_send![event, paramDescriptorForKeyword: KEY_DIRECT_OBJECT] };
    if list.is_null() {
        return Vec::new();
    }
    let list: &AnyObject = unsafe { &*list };
    let count: isize = unsafe { msg_send![list, numberOfItems] };
    let mut out = Vec::new();
    for i in 1..=count {
        let item: *mut AnyObject = unsafe { msg_send![list, descriptorAtIndex: i] };
        if item.is_null() {
            continue;
        }
        let item: &AnyObject = unsafe { &*item };
        let url: *mut AnyObject = unsafe { msg_send![item, fileURLValue] };
        if !url.is_null() {
            let url: &AnyObject = unsafe { &*url };
            let path: *mut AnyObject = unsafe { msg_send![url, path] };
            if let Some(s) = nsstring_to_rust(path) {
                out.push(s);
                continue;
            }
        }
        let s: *mut AnyObject = unsafe { msg_send![item, stringValue] };
        if let Some(s) = nsstring_to_rust(s) {
            if let Some(path) = path_from_os_open_spec(&s) {
                out.push(path);
            } else {
                out.push(s);
            }
        }
    }
    out
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

    #[test]
    fn is_document_path_accepts_bearcad_only() {
        assert!(is_document_path("/tmp/part.bearcad"));
        assert!(is_document_path("/tmp/part.BEARCAD"));
        assert!(is_document_path("part.bearcad"));
        assert!(!is_document_path("/tmp/part.bearcad.json"));
        assert!(!is_document_path("/tmp/part.lua"));
        assert!(!is_document_path(""));
        assert!(!is_document_path("/tmp/part"));
    }

    #[test]
    fn path_from_os_open_spec_decodes_file_urls() {
        assert_eq!(
            path_from_os_open_spec("file:///tmp/part.bearcad").as_deref(),
            Some("/tmp/part.bearcad")
        );
        assert_eq!(
            path_from_os_open_spec("file:///tmp/My%20Part.bearcad").as_deref(),
            Some("/tmp/My Part.bearcad")
        );
        assert_eq!(
            path_from_os_open_spec("file://localhost/tmp/part.bearcad").as_deref(),
            Some("/tmp/part.bearcad")
        );
        assert_eq!(
            path_from_os_open_spec("/tmp/part.bearcad").as_deref(),
            Some("/tmp/part.bearcad")
        );
        assert_eq!(path_from_os_open_spec(""), None);
        assert_eq!(path_from_os_open_spec("   "), None);
    }

    /// Website wasm builds `file_association` but gates `cli_install` out. A call into
    /// that module fails `cargo build --target wasm32-unknown-unknown` (#1335).
    #[test]
    fn register_does_not_depend_on_cli_install() {
        let src = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/file_association.rs"),
        )
        .expect("file_association.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap_or(&src);
        assert!(
            !prod.contains("crate::cli_install"),
            "file_association is compiled for wasm; the installer module is not"
        );
    }

    /// Finder launch-open is delivered as `application:openURLs:` on the NSApplication
    /// delegate *after* NSApp exists — not as argv, and not to an Apple Event handler
    /// installed before `EventLoop::new` (winit's NSApp steals `odoc`). The hooks must
    /// attach at `applicationWillFinishLaunching:` (before the queued `odoc` is dispatched).
    #[test]
    fn macos_open_file_hooks_attach_at_will_finish_launching() {
        let src = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/file_association.rs"),
        )
        .expect("file_association.rs");
        assert!(
            src.contains("NSApplicationWillFinishLaunching"),
            "must observe willFinishLaunching so launch-time odoc is not lost:\n(handler installed only in main() is overwritten when NSApp starts)"
        );
        assert!(
            src.contains("application:openURLs:"),
            "must implement application:openURLs: on the app delegate; winit's does not"
        );
    }

    /// The packaged `.app` Info.plist is what Launch Services uses to *launch* BearCAD
    /// when a `.bearcad` is double-clicked. Association lives here, not in a per-user
    /// write (macOS `register()` is a no-op).
    #[test]
    fn macos_app_plist_declares_bearcad_document() {
        let pkg = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/package-release.sh"),
        )
        .expect("package-release.sh");
        for needle in [
            "CFBundleDocumentTypes",
            "UTExportedTypeDeclarations",
            "com.bearcad.document",
            "CFBundleTypeExtensions",
            "LSHandlerRank",
            "Owner",
            "CFBundleTypeRole",
            "Editor",
            "LSItemContentTypes",
            "public.filename-extension",
            "application/x-bearcad",
        ] {
            assert!(
                pkg.contains(needle),
                "macos Info.plist in package-release.sh must contain {needle}"
            );
        }
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

    /// Release `.app` / `.dmg` must be Developer ID signed, notarized, and stapled
    /// when packaging on CI. Local builds without a cert still ad-hoc sign.
    #[test]
    fn macos_release_is_developer_id_signed_notarized_and_stapled() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let sign = std::fs::read_to_string(root.join("scripts/sign-macos.sh"))
            .expect("scripts/sign-macos.sh must exist");
        for needle in [
            "Developer ID Application",
            "notarytool",
            "stapler staple",
            "--options runtime",
            "APPLE_CODESIGN_IDENTITY",
            "APPLE_API_KEY_ID",
            "APPLE_API_ISSUER_ID",
            "APPLE_API_KEY_P8",
            "APPLE_DEVELOPER_ID_APPLICATION_P12",
            "BEARCAD_REQUIRE_SIGN",
            "BEARCAD_REQUIRE_NOTARIZE",
        ] {
            assert!(
                sign.contains(needle),
                "sign-macos.sh must mention {needle}"
            );
        }

        let pkg = std::fs::read_to_string(root.join("scripts/package-release.sh"))
            .expect("package-release.sh");
        assert!(
            pkg.contains("sign-macos.sh"),
            "package-release.sh must invoke scripts/sign-macos.sh"
        );
        assert!(
            !pkg.contains("codesign --force --deep --sign -"),
            "package-release.sh must not ad-hoc-sign the assembled app itself; sign-macos.sh owns signing"
        );

        let entitlements = root.join("macos/BearCAD.entitlements");
        assert!(
            entitlements.is_file(),
            "macos/BearCAD.entitlements must exist for hardened-runtime signing"
        );

        let ci = std::fs::read_to_string(root.join(".github/workflows/ci.yml"))
            .expect("ci.yml");
        for needle in [
            "APPLE_DEVELOPER_ID_APPLICATION_P12",
            "APPLE_DEVELOPER_ID_APPLICATION_PASSWORD",
            "APPLE_DEVELOPER_ID_INSTALLER_P12",
            "APPLE_DEVELOPER_ID_INSTALLER_PASSWORD",
            "APPLE_CODESIGN_IDENTITY",
            "APPLE_API_KEY_ID",
            "APPLE_API_ISSUER_ID",
            "APPLE_API_KEY_P8",
            "BEARCAD_REQUIRE_SIGN",
            "BEARCAD_REQUIRE_NOTARIZE",
            "sign-macos.sh",
        ] {
            assert!(
                ci.contains(needle),
                "ci.yml must wire {needle} into the macOS release job"
            );
        }
    }

    /// Local/CI-without-cert path: `sign-macos.sh sign-app` ad-hoc signs a bundle.
    #[cfg(target_os = "macos")]
    #[test]
    fn sign_macos_sh_adhoc_signs_a_dummy_app() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = std::env::temp_dir().join(format!(
            "bearcad-sign-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        let app = tmp.join("BearCAD.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(
            app.join("Contents/MacOS/bearcad"),
            b"#!/bin/sh\necho ok\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let bin = app.join("Contents/MacOS/bearcad");
            let mut perms = std::fs::metadata(&bin).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&bin, perms).unwrap();
        }
        std::fs::write(
            app.join("Contents/Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>bearcad</string>
  <key>CFBundleIdentifier</key><string>com.bearcad.app.test</string>
  <key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
"#,
        )
        .unwrap();

        let status = std::process::Command::new(root.join("scripts/sign-macos.sh"))
            .args(["sign-app"])
            .arg(&app)
            .env("BEARCAD_FORCE_ADHOC", "1")
            .env_remove("APPLE_CODESIGN_IDENTITY")
            .env_remove("BEARCAD_REQUIRE_SIGN")
            .env_remove("BEARCAD_REQUIRE_NOTARIZE")
            .env_remove("APPLE_API_KEY_P8")
            .status()
            .expect("run sign-macos.sh");
        assert!(status.success(), "sign-macos.sh sign-app failed: {status}");
        let verify = std::process::Command::new("codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(&app)
            .status()
            .expect("codesign --verify");
        assert!(verify.success(), "ad-hoc signature did not verify");
        let _ = std::fs::remove_dir_all(&tmp);
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
