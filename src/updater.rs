//! Auto-update (#427): check GitHub for a newer release in the background and, when one
//! exists, surface an unobtrusive badge in the status bar. Clicking it downloads and
//! stages the new version in place on every desktop OS — Windows/Linux swap the bare
//! binary; macOS mounts the release dmg and rename-swaps the `.app` bundle, the same
//! trick Electron's Squirrel.Mac uses — then the badge becomes a **Restart** button that
//! relaunches into the new version. Falls back to a browser auto-download (dev builds,
//! failures), then the releases page.
//!
//! Network access uses the system `curl` (present on stock macOS, Windows 10+, and
//! virtually all Linux desktops) so the app gains no TLS dependency; if `curl` is missing
//! the check silently does nothing. Native builds only — the web app is always current.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::release_artifacts::{GITHUB_REPO, LINUX_ARTIFACT, MACOS_ARTIFACT, WINDOWS_ARTIFACT};

/// Result of a completed update attempt, surfaced in the status bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// The new version is staged in place — `launch` is what a restart should run (the
    /// `.app` bundle on macOS, the executable elsewhere).
    StagedRestartToFinish { launch: PathBuf },
    /// The platform artifact was handed to the browser (auto-download); install manually.
    OpenedInBrowser,
}

/// Shared updater state, written by background threads and read each frame.
#[derive(Clone, Debug, Default)]
pub struct UpdateState {
    /// A newer release's version (e.g. "0.4.2"), once the background check finds one.
    pub available: Option<String>,
    /// True while an update download/stage runs.
    pub in_progress: bool,
    /// The finished attempt's outcome or error.
    pub outcome: Option<Result<UpdateOutcome, String>>,
}

pub type SharedUpdateState = Arc<Mutex<UpdateState>>;

/// Kick off the background release check. Returns immediately; the shared state fills in
/// when (and if) the check finds a newer version. Disabled via `BEARCAD_NO_UPDATE_CHECK`.
pub fn spawn_check(state: SharedUpdateState) {
    if std::env::var_os("BEARCAD_NO_UPDATE_CHECK").is_some() {
        return;
    }
    std::thread::spawn(move || {
        cleanup_leftovers();
        if let Some(latest) = fetch_latest_version() {
            if is_newer(&latest, &update_check_version()) {
                if let Ok(mut s) = state.lock() {
                    s.available = Some(latest);
                }
            }
        }
    });
}

/// Start the platform-appropriate update in a background thread.
pub fn spawn_update(state: SharedUpdateState, ctx: egui::Context) {
    {
        let Ok(mut s) = state.lock() else { return };
        if s.in_progress {
            return;
        }
        s.in_progress = true;
        s.outcome = None;
    }
    std::thread::spawn(move || {
        let result = perform_update();
        if let Ok(mut s) = state.lock() {
            s.in_progress = false;
            s.outcome = Some(result);
        }
        ctx.request_repaint();
    });
}

/// Remove what a previous staged update left behind (#427): the renamed-aside old
/// binary/bundle. Best-effort; runs on the background check thread at startup.
fn cleanup_leftovers() {
    let Ok(exe) = std::env::current_exe() else { return };
    let _ = std::fs::remove_file(exe.with_extension("old"));
    if let Some(bundle) = app_bundle_of(&exe) {
        if let Some(parent) = bundle.parent() {
            let old = parent.join("BearCAD-old.app");
            if old.is_dir() {
                let _ = std::fs::remove_dir_all(&old);
            }
        }
    }
}

/// The latest release's version from the GitHub API, via system curl. `None` on any
/// failure (offline, no curl, rate limit) — the check is best-effort.
fn fetch_latest_version() -> Option<String> {
    let out = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "-m",
            "10",
            "-H",
            "User-Agent: bearcad-update-check",
            "https://api.github.com/repos/iffy/BearCAD/releases/latest",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    json.get("tag_name")
        .and_then(|t| t.as_str())
        .map(|t| t.trim_start_matches('v').to_string())
}

/// Whether `candidate` is a strictly newer version than `current` (dotted numeric
/// compare; non-numeric segments compare as 0).
/// The version string the update check compares against release tags: the baked
/// `git describe` when available (so a release build knows its own build number and
/// the badge never claims its own version is an update), else the crate version.
fn update_check_version() -> String {
    let describe = env!("BEARCAD_GIT_DESCRIBE");
    if describe.starts_with('v') {
        describe.trim_start_matches('v').to_string()
    } else {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split(['.', '-'])
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parse(candidate), parse(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// The direct download URL for this platform's release artifact.
pub fn platform_artifact_url() -> String {
    let artifact = if cfg!(target_os = "windows") {
        WINDOWS_ARTIFACT
    } else if cfg!(target_os = "macos") {
        MACOS_ARTIFACT
    } else {
        LINUX_ARTIFACT
    };
    crate::release_artifacts::download_url(artifact)
}

/// The releases page, the universal fallback.
pub fn releases_page_url() -> String {
    format!("{GITHUB_REPO}/releases/latest")
}

/// Download and stage the update where the platform allows a clean swap.
///
/// - **Windows** (bare `bearcad.exe` artifact) and **Linux** (binary inside a tar.gz):
///   download to a temp dir, then swap the running executable via the rename trick (the
///   old binary moves aside to `bearcad-old…`; the OS keeps running it until restart).
/// - **macOS** (a `.dmg`): the same trick Electron's Squirrel.Mac uses — a running `.app`
///   bundle can be renamed, so mount the dmg (`hdiutil attach`), copy the new bundle next
///   to the installed one, and rename-swap. Falls back to a browser auto-download when the
///   app isn't running from a bundle (e.g. a dev build).
fn perform_update() -> Result<UpdateOutcome, String> {
    if cfg!(target_os = "macos") {
        return perform_macos_update();
    }
    let exe = std::env::current_exe().map_err(|e| format!("current exe: {e}"))?;
    let dir = std::env::temp_dir().join("bearcad-update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("temp dir: {e}"))?;

    let url = platform_artifact_url();
    let staged: std::path::PathBuf = if cfg!(target_os = "windows") {
        let path = dir.join("bearcad-new.exe");
        curl_download(&url, &path)?;
        path
    } else {
        let archive = dir.join("bearcad.tar.gz");
        curl_download(&url, &archive)?;
        let status = std::process::Command::new("tar")
            .args(["xzf", &archive.to_string_lossy(), "-C", &dir.to_string_lossy()])
            .status()
            .map_err(|e| format!("tar: {e}"))?;
        if !status.success() {
            return Err("tar extraction failed".to_string());
        }
        // The archive holds the single `bearcad` binary (possibly under a folder).
        find_binary(&dir, "bearcad").ok_or("no bearcad binary in the archive")?
    };

    // Rename trick: the running executable moves aside (the OS keeps executing it), the
    // new one takes its place; a restart runs the new version.
    let old = exe.with_extension("old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| format!("stage old binary: {e}"))?;
    match std::fs::rename(&staged, &exe).or_else(|_| {
        // Cross-device temp dir: fall back to copy.
        std::fs::copy(&staged, &exe).map(|_| ())
    }) {
        Ok(()) => {}
        Err(e) => {
            // Roll back so the install stays runnable.
            let _ = std::fs::rename(&old, &exe);
            return Err(format!("install new binary: {e}"));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755));
    }
    Ok(UpdateOutcome::StagedRestartToFinish { launch: exe })
}

/// The `.app` bundle a macOS executable runs from (`…/BearCAD.app/Contents/MacOS/bearcad`
/// → `…/BearCAD.app`), if it is inside one.
pub fn app_bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos_dir = exe.parent()?;
    let contents = macos_dir.parent()?;
    let bundle = contents.parent()?;
    (macos_dir.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
    .then(|| bundle.to_path_buf())
}

/// macOS staged update (#427, Squirrel.Mac-style): mount the release dmg, copy the new
/// `.app` beside the installed bundle, rename the old aside, rename the new into place.
/// The running app keeps executing from the renamed bundle until restart.
fn perform_macos_update() -> Result<UpdateOutcome, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current exe: {e}"))?;
    let Some(bundle) = app_bundle_of(&exe) else {
        // Not running from an .app bundle (dev build / bare binary): auto-download in the
        // browser instead of guessing at an install layout.
        return Ok(UpdateOutcome::OpenedInBrowser);
    };
    let parent = bundle.parent().ok_or("app bundle has no parent")?;

    let dir = std::env::temp_dir().join("bearcad-update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("temp dir: {e}"))?;
    let dmg = dir.join("bearcad.dmg");
    curl_download(&platform_artifact_url(), &dmg)?;

    let mount = dir.join("mnt");
    let status = std::process::Command::new("hdiutil")
        .args([
            "attach",
            "-nobrowse",
            "-quiet",
            "-mountpoint",
            &mount.to_string_lossy(),
            &dmg.to_string_lossy(),
        ])
        .status()
        .map_err(|e| format!("hdiutil: {e}"))?;
    if !status.success() {
        return Err("mounting the update dmg failed".to_string());
    }
    // Everything after the mount must detach it, success or not.
    let result = (|| -> Result<UpdateOutcome, String> {
        let new_app = find_app_bundle(&mount).ok_or("no .app in the update dmg")?;
        // Copy to the same directory as the installed bundle so the final rename is
        // same-volume (atomic). `ditto` preserves the bundle's signatures/permissions.
        let staged = parent.join(".bearcad-update.app");
        let _ = std::process::Command::new("rm").args(["-rf", &staged.to_string_lossy()]).status();
        let status = std::process::Command::new("ditto")
            .args([&new_app.to_string_lossy()[..], &staged.to_string_lossy()[..]])
            .status()
            .map_err(|e| format!("ditto: {e}"))?;
        if !status.success() {
            return Err("copying the new app failed".to_string());
        }
        // Rename-swap: the running bundle moves aside (macOS keeps executing it), the new
        // bundle takes its name.
        let old = parent.join("BearCAD-old.app");
        let _ = std::process::Command::new("rm").args(["-rf", &old.to_string_lossy()]).status();
        std::fs::rename(&bundle, &old).map_err(|e| format!("stage old app: {e}"))?;
        if let Err(e) = std::fs::rename(&staged, &bundle) {
            let _ = std::fs::rename(&old, &bundle); // roll back
            return Err(format!("install new app: {e}"));
        }
        Ok(UpdateOutcome::StagedRestartToFinish { launch: bundle.clone() })
    })();
    let _ = std::process::Command::new("hdiutil")
        .args(["detach", "-quiet", &mount.to_string_lossy()])
        .status();
    result
}

/// The first `.app` bundle directly inside `dir` (the dmg root).
fn find_app_bundle(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let path = entry.path();
        (path.is_dir() && path.extension().is_some_and(|e| e == "app")).then_some(path)
    })
}

/// Relaunch the staged version and quit this process (#427): `open -n` for a macOS `.app`
/// bundle, a plain spawn for a bare executable.
pub fn restart_into(launch: &Path) -> Result<(), String> {
    if launch.extension().is_some_and(|e| e == "app") {
        std::process::Command::new("open")
            .args(["-n", &launch.to_string_lossy()])
            .spawn()
            .map_err(|e| format!("open: {e}"))?;
    } else {
        std::process::Command::new(launch)
            .spawn()
            .map_err(|e| format!("spawn: {e}"))?;
    }
    // Give the spawn a moment to take, then exit; the new instance carries on.
    std::thread::sleep(std::time::Duration::from_millis(150));
    std::process::exit(0);
}

fn curl_download(url: &str, to: &std::path::Path) -> Result<(), String> {
    let status = std::process::Command::new("curl")
        .args(["-fsSL", "-m", "300", "-o", &to.to_string_lossy(), url])
        .status()
        .map_err(|e| format!("curl: {e}"))?;
    if !status.success() {
        return Err(format!("download failed: {url}"));
    }
    if std::fs::metadata(to).map(|m| m.len()).unwrap_or(0) == 0 {
        return Err("downloaded file is empty".to_string());
    }
    Ok(())
}

/// Find a file named `name` anywhere under `dir` (the tarball may nest it in a folder).
fn find_binary(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_binary(&path, name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|f| f == name) {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests_release_identity {
    use super::*;

    /// #460: a release build baked with its own tag must not see itself as an update.
    #[test]
    fn own_build_number_is_not_an_update() {
        assert!(!is_newer("0.1.0-build.261", "0.1.0-build.261"));
        assert!(is_newer("0.1.0-build.262", "0.1.0-build.261"));
        assert!(!is_newer("0.1.0-build.260", "0.1.0-build.261"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare_handles_tags_and_lengths() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("0.1.9.1", "0.1.9"));
        assert!(!is_newer("0.1.9", "0.1.9"));
        assert!(!is_newer("0.1.8", "0.1.9"));
        assert!(!is_newer("v0.1.9", "0.1.9"));
    }

    #[test]
    fn platform_artifact_url_points_at_latest_download() {
        let url = platform_artifact_url();
        assert!(url.starts_with(crate::release_artifacts::RELEASES_BASE));
        assert!(releases_page_url().starts_with(GITHUB_REPO));
    }

    #[test]
    fn app_bundle_of_detects_bundles_and_bare_binaries() {
        assert_eq!(
            app_bundle_of(Path::new("/Applications/BearCAD.app/Contents/MacOS/bearcad")),
            Some(PathBuf::from("/Applications/BearCAD.app"))
        );
        assert_eq!(app_bundle_of(Path::new("/usr/local/bin/bearcad")), None);
        assert_eq!(app_bundle_of(Path::new("/tmp/target/debug/bearcad")), None);
    }

    #[test]
    fn find_binary_searches_nested_folders() {
        let dir = std::env::temp_dir().join("bearcad_find_binary_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("nested").join("bearcad"), b"x").unwrap();
        assert_eq!(
            find_binary(&dir, "bearcad"),
            Some(dir.join("nested").join("bearcad"))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
