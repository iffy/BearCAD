//! Installing the `bearcad` command-line tool onto the user's PATH, and registering
//! `.bearcad` file associations (#49, #1285).
//!
//! On macOS the app is installed by dragging `BearCAD.app` into `/Applications`, which
//! runs no install code. To make the bundled `bearcad` executable usable from a terminal
//! we expose an explicit action — the `bearcad install-cli` subcommand and a matching
//! Help-menu item — that symlinks the running executable into a directory on PATH
//! (`/usr/local/bin` by default). `uninstall-cli` removes it again. The same mechanism
//! works on Linux. On Windows there is no PATH symlink; `install-cli` still registers
//! the `.bearcad` file association so double-click works.
//!
//! When the menu action cannot write the link (e.g. `/usr/local/bin` needs root), it
//! re-runs the link step through the standard macOS authorization prompt —
//! `osascript … with administrator privileges` — instead of telling the user to sudo
//! (#1788). The terminal subcommand keeps plain permissions: the user can sudo.

use std::path::{Path, PathBuf};

/// Default PATH location for the CLI symlink.
#[cfg_attr(windows, allow(dead_code))]
pub const DEFAULT_INSTALL_DIR: &str = "/usr/local/bin";
/// Name of the installed command.
#[cfg_attr(windows, allow(dead_code))]
pub const CLI_NAME: &str = "bearcad";

/// The default symlink path (`/usr/local/bin/bearcad`).
#[cfg_attr(not(unix), allow(dead_code))]
pub fn default_target() -> PathBuf {
    Path::new(DEFAULT_INSTALL_DIR).join(CLI_NAME)
}

/// The executable to link to: the currently running binary (inside the .app bundle on
/// macOS). Resolved through any symlinks so re-running `install-cli` from an already
/// installed link still points at the real binary.
pub fn current_binary() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find current executable: {e}"))?;
    // Canonicalize so a link never points at another link (or at a temp path).
    std::fs::canonicalize(&exe).map_err(|e| format!("cannot resolve {}: {e}", exe.display()))
}

/// Create (or replace) a symlink at `target` pointing to `source`.
///
/// Replaces an existing symlink at `target` unconditionally, but refuses to clobber a
/// real file/directory there (something other than our managed link) so we never delete
/// a user's unrelated binary.
#[cfg(unix)]
pub fn install_link(source: &Path, target: &Path) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
    }
    match std::fs::symlink_metadata(target) {
        Ok(meta) => {
            if !meta.file_type().is_symlink() {
                return Err(format!(
                    "{} already exists and is not a symlink; remove it first",
                    target.display()
                ));
            }
            std::fs::remove_file(target)
                .map_err(|e| format!("cannot replace {}: {e}", target.display()))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("cannot inspect {}: {e}", target.display())),
    }
    std::os::unix::fs::symlink(source, target).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!(
                "permission denied writing {}; re-run with elevated permissions \
                 (e.g. `sudo bearcad install-cli`)",
                target.display()
            )
        } else {
            format!("cannot link {} -> {}: {e}", target.display(), source.display())
        }
    })
}

#[cfg(not(unix))]
#[allow(dead_code)] // PATH symlink install is unix-only; Windows uses file association only.
pub fn install_link(_source: &Path, _target: &Path) -> Result<(), String> {
    Err("install-cli is only supported on macOS and Linux".to_string())
}

/// Remove a CLI symlink previously created by [`install_link`]. Refuses to remove a real
/// (non-symlink) file at `target`. Succeeds quietly if nothing is there.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn uninstall_link(target: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(target) {
        Ok(meta) => {
            if !meta.file_type().is_symlink() {
                return Err(format!(
                    "{} is not a bearcad symlink; refusing to remove it",
                    target.display()
                ));
            }
            std::fs::remove_file(target)
                .map_err(|e| format!("cannot remove {}: {e}", target.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot inspect {}: {e}", target.display())),
    }
}

/// Install the CLI to the default location and register `.bearcad` file associations.
/// Returns a human-readable status line (may span PATH + association).
pub fn run_install() -> Result<String, String> {
    install_with(Elevation::Never)
}

/// [`run_install`] for the GUI menu action: when the link needs root, ask for it with
/// the standard macOS authorization prompt instead of failing (#1788).
pub fn run_install_gui() -> Result<String, String> {
    install_with(Elevation::WhenNeeded)
}

/// How the install may elevate when the link target is not user-writable.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Elevation {
    Never,
    /// macOS only: `osascript … with administrator privileges`.
    WhenNeeded,
}

fn install_with(elevation: Elevation) -> Result<String, String> {
    let source = current_binary()?;
    let mut parts = Vec::new();

    #[cfg(unix)]
    {
        let target = default_target();
        match install_link(&source, &target) {
            Ok(()) => parts.push(format!(
                "Installed `{CLI_NAME}` -> {} (links to {})",
                target.display(),
                source.display()
            )),
            Err(e) => {
                let can_elevate =
                    elevation == Elevation::WhenNeeded && e.contains("permission denied");
                if !can_elevate {
                    return Err(e);
                }
                // Only macOS has a standard authorization prompt; every other unix
                // resolves to the stub below. The gate has to be a `#[cfg]`, not
                // `cfg!(...)`, or the macOS-only helper still has to exist on Linux
                // (#1835).
                elevated_install_link(&source, &target)?;
                parts.push(format!(
                    "Installed `{CLI_NAME}` -> {} (with administrator privileges; links to {})",
                    target.display(),
                    source.display()
                ));
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (source, elevation);
        // Windows has no POSIX symlink install; file association is the useful half.
    }

    match crate::file_association::register() {
        Ok(msg) => parts.push(msg),
        Err(err) => {
            if parts.is_empty() {
                return Err(err);
            }
            parts.push(format!("file association: {err}"));
        }
    }

    if parts.is_empty() {
        return Err("install-cli: nothing to install on this platform".into());
    }
    Ok(parts.join("; "))
}

/// Non-macOS unix has no standard authorization prompt, so the install just reports
/// that the link target needs root (#1835).
#[cfg(all(unix, not(target_os = "macos")))]
fn elevated_install_link(_source: &Path, target: &Path) -> Result<(), String> {
    Err(format!(
        "cannot write {}: re-run the install as root",
        target.display()
    ))
}

/// Single-quote a path for POSIX shell use.
#[cfg(target_os = "macos")]
fn shell_quote(path: &Path) -> String {
    let s = path.display().to_string();
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Re-run the link step with the standard macOS authorization prompt (#1788). The
/// elevated script keeps [`install_link`]'s safety rule — it never replaces anything
/// that is not one of our own symlinks.
#[cfg(target_os = "macos")]
fn elevated_install_link(source: &Path, target: &Path) -> Result<(), String> {
    let t = shell_quote(target);
    let dir = shell_quote(
        target
            .parent()
            .unwrap_or(Path::new(DEFAULT_INSTALL_DIR)),
    );
    let s = shell_quote(source);
    // Refuse non-symlinks inside the privileged script (no TOCTOU-free way to do this
    // check unprivileged), then link like `install_link` does.
    let script = format!(
        "if [ -e {t} ] && [ ! -L {t} ]; then echo REFUSE_NOT_SYMLINK; exit 1; fi; \
         mkdir -p {dir} && ln -sfn {s} {t} && echo OK",
    );
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(format!(
            "do shell script \"{}\" with administrator privileges",
            script.replace('\\', "\\\\").replace('"', "\\\"")
        ))
        .output()
        .map_err(|e| format!("could not run the macOS authorization prompt: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() && stdout.contains("OK") {
        return Ok(());
    }
    if stdout.contains("REFUSE_NOT_SYMLINK") {
        return Err(format!(
            "{t} already exists and is not a symlink; remove it first"
        ));
    }
    let combined = format!("{}{}", stdout.trim(), stderr.trim());
    if combined.contains("User canceled") || combined.contains("user canceled") {
        return Err("canceled by user".to_string());
    }
    Err(format!("administrator install failed: {}", combined.trim()))
}

/// Remove the CLI symlink and unregister `.bearcad` file associations.
pub fn run_uninstall() -> Result<String, String> {
    let mut parts = Vec::new();

    #[cfg(unix)]
    {
        let target = default_target();
        uninstall_link(&target)?;
        parts.push(format!("Removed `{CLI_NAME}` ({})", target.display()));
    }

    match crate::file_association::unregister() {
        Ok(msg) => parts.push(msg),
        Err(err) => {
            if parts.is_empty() {
                return Err(err);
            }
            parts.push(format!("file association: {err}"));
        }
    }

    if parts.is_empty() {
        return Ok("uninstall-cli: nothing to remove".into());
    }
    Ok(parts.join("; "))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bearcad_cli_install_{tag}_{}_{}",
            std::process::id(),
            crate::time::SystemTime::now()
                .duration_since(crate::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn install_link_creates_symlink_to_source() {
        let dir = temp_dir("create");
        let source = dir.join("real_bearcad");
        std::fs::write(&source, b"binary").unwrap();
        let target = dir.join("bin").join("bearcad");
        install_link(&source, &target).unwrap();
        assert!(std::fs::symlink_metadata(&target).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_link(&target).unwrap(), source);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn install_link_replaces_existing_symlink() {
        let dir = temp_dir("replace");
        let old_source = dir.join("old");
        let new_source = dir.join("new");
        std::fs::write(&old_source, b"old").unwrap();
        std::fs::write(&new_source, b"new").unwrap();
        let target = dir.join("bearcad");
        install_link(&old_source, &target).unwrap();
        install_link(&new_source, &target).unwrap();
        assert_eq!(std::fs::read_link(&target).unwrap(), new_source);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn install_link_refuses_to_clobber_real_file() {
        let dir = temp_dir("clobber");
        let source = dir.join("src");
        std::fs::write(&source, b"src").unwrap();
        let target = dir.join("bearcad");
        std::fs::write(&target, b"i am a real file").unwrap();
        let err = install_link(&source, &target).unwrap_err();
        assert!(err.contains("not a symlink"), "got: {err}");
        // The real file is untouched.
        assert_eq!(std::fs::read(&target).unwrap(), b"i am a real file");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn uninstall_link_removes_symlink_and_is_idempotent() {
        let dir = temp_dir("uninstall");
        let source = dir.join("src");
        std::fs::write(&source, b"src").unwrap();
        let target = dir.join("bearcad");
        install_link(&source, &target).unwrap();
        uninstall_link(&target).unwrap();
        assert!(std::fs::symlink_metadata(&target).is_err());
        // Removing again is a no-op, not an error.
        uninstall_link(&target).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn uninstall_link_refuses_real_file() {
        let dir = temp_dir("uninstall_real");
        let target = dir.join("bearcad");
        std::fs::write(&target, b"real").unwrap();
        assert!(uninstall_link(&target).unwrap_err().contains("not a bearcad symlink"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// #1788: paths handed to the privileged shell script are single-quoted so spaces
    /// and quotes cannot break out of the command.
    #[cfg(target_os = "macos")]
    #[test]
    fn shell_quote_escapes_spaces_and_quotes() {
        assert_eq!(shell_quote(Path::new("/usr/local/bin/bearcad")), "'/usr/local/bin/bearcad'");
        assert_eq!(
            shell_quote(Path::new("/Applications/My Apps/BearCAD.app")),
            "'/Applications/My Apps/BearCAD.app'"
        );
        assert_eq!(
            shell_quote(Path::new("/o'dd/pla'ce")),
            "'/o'\\''dd/pla'\\''ce'"
        );
    }
}
