//! Installing the `bearcad` command-line tool onto the user's PATH.
//!
//! On macOS the app is installed by dragging `BearCAD.app` into `/Applications`, which
//! runs no install code. To make the bundled `bearcad` executable usable from a terminal
//! we expose an explicit action — the `bearcad install-cli` subcommand and a matching
//! Help-menu item — that symlinks the running executable into a directory on PATH
//! (`/usr/local/bin` by default). `uninstall-cli` removes it again. The same mechanism
//! works on Linux; on platforms without POSIX symlinks it reports an error. (#49)

use std::path::{Path, PathBuf};

/// Default PATH location for the CLI symlink.
pub const DEFAULT_INSTALL_DIR: &str = "/usr/local/bin";
/// Name of the installed command.
pub const CLI_NAME: &str = "bearcad";

/// The default symlink path (`/usr/local/bin/bearcad`).
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
pub fn install_link(_source: &Path, _target: &Path) -> Result<(), String> {
    Err("install-cli is only supported on macOS and Linux".to_string())
}

/// Remove a CLI symlink previously created by [`install_link`]. Refuses to remove a real
/// (non-symlink) file at `target`. Succeeds quietly if nothing is there.
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

/// Install the CLI to the default location, returning a human-readable status line.
pub fn run_install() -> Result<String, String> {
    let source = current_binary()?;
    let target = default_target();
    install_link(&source, &target)?;
    Ok(format!(
        "Installed `{CLI_NAME}` -> {} (links to {})",
        target.display(),
        source.display()
    ))
}

/// Remove the CLI from the default location, returning a human-readable status line.
pub fn run_uninstall() -> Result<String, String> {
    let target = default_target();
    uninstall_link(&target)?;
    Ok(format!("Removed `{CLI_NAME}` ({})", target.display()))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bearcad_cli_install_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
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
}
