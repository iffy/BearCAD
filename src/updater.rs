//! Auto-update (#427): check GitHub for a newer release in the background and, when one
//! exists, surface an unobtrusive badge in the status bar. Clicking it downloads and
//! stages the new version in place on every desktop OS — Windows/Linux swap the bare
//! binary; macOS mounts the release dmg and rename-swaps the `.app` bundle, the same
//! trick Electron's Squirrel.Mac uses — then the badge becomes a **Restart** button that
//! relaunches into the new version. Falls back to a browser auto-download (dev builds,
//! failures), then the releases page.
//!
//! Network access is ureq with rustls (#1596/#1610). Failures
//! are silent for the version check and reported in the badge for a download. Native builds
//! only — the web app is always current.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::release_artifacts::{GITHUB_REPO, LINUX_ARTIFACT, MACOS_ARTIFACT, WINDOWS_ARTIFACT};
use crate::settings::UpdateChannel;

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
///
/// `channel` (#1288) selects whether only stable releases or also pre-releases are
/// considered.
pub fn spawn_check(state: SharedUpdateState, channel: UpdateChannel) {
    if std::env::var_os("BEARCAD_NO_UPDATE_CHECK").is_some() {
        return;
    }
    std::thread::spawn(move || {
        cleanup_leftovers();
        if let Some(latest) = fetch_latest_version(channel) {
            if !is_dev_build() && is_newer(&latest, &update_check_version()) {
                if let Ok(mut s) = state.lock() {
                    s.available = Some(latest);
                }
            }
        }
    });
}

/// Start the platform-appropriate update in a background thread. Downloads the artifact
/// for `version` (the tag the badge reported) so a pre-release channel install does not
/// silently pull `/latest` (#1288).
pub fn spawn_update(state: SharedUpdateState, ctx: egui::Context, version: String) {
    {
        let Ok(mut s) = state.lock() else { return };
        if s.in_progress {
            return;
        }
        s.in_progress = true;
        s.outcome = None;
    }
    std::thread::spawn(move || {
        let result = perform_update(&version);
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
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
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

/// GitHub rejects API calls without a User-Agent.
const USER_AGENT: &str = "bearcad-update-check";
/// Whole-request deadline for the version check.
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
/// Whole-request deadline for an artifact download.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// The latest version string for `channel` from the GitHub API. `None` on any failure
/// (offline, rate limit, malformed JSON) — the check is best-effort.
fn fetch_latest_version(channel: UpdateChannel) -> Option<String> {
    let url = match channel {
        // Non-prerelease, non-draft only.
        UpdateChannel::Release => "https://api.github.com/repos/iffy/BearCAD/releases/latest",
        // Full list so prereleases are included; we pick the first non-draft.
        UpdateChannel::PreRelease => {
            "https://api.github.com/repos/iffy/BearCAD/releases?per_page=20"
        }
    };
    fetch_latest_version_from(url, channel)
}

/// Like [`fetch_latest_version`], but against an arbitrary URL so tests can drive a loopback
/// server instead of GitHub.
fn fetch_latest_version_from(url: &str, channel: UpdateChannel) -> Option<String> {
    parse_latest_version_json(&http_get_bytes(url).ok()?, channel)
}

/// Pick a version string out of a GitHub releases API body for `channel`.
///
/// - **Release**: a single release object (`/releases/latest`).
/// - **Pre-release**: an array; first non-draft entry wins (GitHub returns newest first),
///   so stable and prerelease releases both compete.
///
/// Public for unit tests (#1288).
pub fn parse_latest_version_json(bytes: &[u8], channel: UpdateChannel) -> Option<String> {
    let json: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    match channel {
        UpdateChannel::Release => tag_name_from_release(&json),
        UpdateChannel::PreRelease => {
            let arr = json.as_array()?;
            arr.iter()
                .find(|r| r.get("draft").and_then(|d| d.as_bool()) != Some(true))
                .and_then(tag_name_from_release)
        }
    }
}

fn tag_name_from_release(release: &serde_json::Value) -> Option<String> {
    release
        .get("tag_name")
        .and_then(|t| t.as_str())
        .map(|t| t.trim_start_matches('v').to_string())
}

/// Whether `candidate` is a strictly newer version than `current` (dotted numeric
/// compare; non-numeric segments compare as 0).
/// A build that isn't a published artifact: a debug build, or a `git describe` carrying
/// commits past the latest tag (`-N-g<sha>`). Whatever a dev build contains is by
/// definition ahead of what's released, so the update check treats it as newer than any
/// tag and never offers to "update" it backwards (#764).
pub fn is_dev_build() -> bool {
    cfg!(debug_assertions) || env!("BEARCAD_GIT_DESCRIBE").contains("-g")
}

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
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

/// This platform's artifact filename.
fn platform_artifact() -> &'static str {
    if cfg!(target_os = "windows") {
        WINDOWS_ARTIFACT
    } else if cfg!(target_os = "macos") {
        MACOS_ARTIFACT
    } else {
        LINUX_ARTIFACT
    }
}

/// The direct download URL for this platform's release artifact.
///
/// When `version` is set (the tag the update check found, without or with a leading `v`),
/// the URL targets that exact release so a pre-release install does not pull `/latest`
/// (#1288). Without a version, falls back to the `/releases/latest/download` shortcut.
pub fn platform_artifact_url() -> String {
    platform_artifact_url_for(None)
}

/// Like [`platform_artifact_url`], optionally pinned to a release version/tag.
pub fn platform_artifact_url_for(version: Option<&str>) -> String {
    let artifact = platform_artifact();
    match version.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => {
            let tag = if v.starts_with('v') {
                v.to_string()
            } else {
                format!("v{v}")
            };
            format!("{GITHUB_REPO}/releases/download/{tag}/{artifact}")
        }
        None => crate::release_artifacts::download_url(artifact),
    }
}

/// The releases page, the universal fallback.
pub fn releases_page_url() -> String {
    format!("{GITHUB_REPO}/releases/latest")
}

/// Releases page for a specific version (browser fallback after a failed stage).
pub fn release_page_url_for(version: &str) -> String {
    let tag = if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{version}")
    };
    format!("{GITHUB_REPO}/releases/tag/{tag}")
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
fn perform_update(version: &str) -> Result<UpdateOutcome, String> {
    if cfg!(target_os = "macos") {
        return perform_macos_update(version);
    }
    let exe = std::env::current_exe().map_err(|e| format!("current exe: {e}"))?;
    let dir = std::env::temp_dir().join("bearcad-update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("temp dir: {e}"))?;

    let url = platform_artifact_url_for(Some(version));
    let staged: std::path::PathBuf = if cfg!(target_os = "windows") {
        let path = dir.join("bearcad-new.exe");
        http_download(&url, &path)?;
        path
    } else {
        let archive = dir.join("bearcad.tar.gz");
        http_download(&url, &archive)?;
        let status = std::process::Command::new("tar")
            .args([
                "xzf",
                &archive.to_string_lossy(),
                "-C",
                &dir.to_string_lossy(),
            ])
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
fn perform_macos_update(version: &str) -> Result<UpdateOutcome, String> {
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
    http_download(&platform_artifact_url_for(Some(version)), &dmg)?;

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
        let _ = std::process::Command::new("rm")
            .args(["-rf", &staged.to_string_lossy()])
            .status();
        let status = std::process::Command::new("ditto")
            .args([
                &new_app.to_string_lossy()[..],
                &staged.to_string_lossy()[..],
            ])
            .status()
            .map_err(|e| format!("ditto: {e}"))?;
        if !status.success() {
            return Err("copying the new app failed".to_string());
        }
        // Rename-swap: the running bundle moves aside (macOS keeps executing it), the new
        // bundle takes its name.
        let old = parent.join("BearCAD-old.app");
        let _ = std::process::Command::new("rm")
            .args(["-rf", &old.to_string_lossy()])
            .status();
        std::fs::rename(&bundle, &old).map_err(|e| format!("stage old app: {e}"))?;
        if let Err(e) = std::fs::rename(&staged, &bundle) {
            let _ = std::fs::rename(&old, &bundle); // roll back
            return Err(format!("install new app: {e}"));
        }
        Ok(UpdateOutcome::StagedRestartToFinish {
            launch: bundle.clone(),
        })
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

fn http_agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent(USER_AGENT)
        .build()
        .into()
}

/// GET `url` and return the body. Used for the GitHub releases JSON (small).
fn http_get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let mut response = http_agent(CHECK_TIMEOUT)
        .get(url)
        .call()
        .map_err(|e| format!("{url}: {e}"))?;
    response
        .body_mut()
        .read_to_vec()
        .map_err(|e| format!("{url}: {e}"))
}

/// GET `url` into `to`, following redirects. Streams so a dmg/tar is not held in RAM.
fn http_download(url: &str, to: &std::path::Path) -> Result<(), String> {
    let mut response = http_agent(DOWNLOAD_TIMEOUT)
        .get(url)
        .call()
        .map_err(|e| format!("download failed: {url}: {e}"))?;
    let mut file = std::fs::File::create(to).map_err(|e| format!("download: {e}"))?;
    let mut reader = response.body_mut().with_config().limit(u64::MAX).reader();
    std::io::copy(&mut reader, &mut file).map_err(|e| format!("download: {e}"))?;
    file.flush().map_err(|e| format!("download: {e}"))?;
    drop(file);
    if std::fs::metadata(to).map(|m| m.len()).unwrap_or(0) == 0 {
        let _ = std::fs::remove_file(to);
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
    /// Build numbers were YYMMDD-### per day (#1129); they are the abbreviated commit SHA
    /// now (#1788), which carries no ordering — within one version, a different SHA is
    /// just a different build, never an update.
    #[test]
    fn own_build_number_is_not_an_update() {
        assert!(!is_newer(
            "0.1.0-build.260812-001",
            "0.1.0-build.260812-001"
        ));
        assert!(is_newer("0.1.0-build.260812-002", "0.1.0-build.260812-001"));
        assert!(!is_newer(
            "0.1.0-build.260812-001",
            "0.1.0-build.260812-002"
        ));
        // Later calendar day is newer regardless of sequence.
        assert!(is_newer("0.1.0-build.260813-001", "0.1.0-build.260812-999"));
        // Legacy run_number tags still order under the date-style numbers.
        assert!(is_newer("0.1.0-build.260812-001", "0.1.0-build.628"));

        // SHA build numbers: identical is identical, and SHAs can't order — the semver
        // alone decides, so a re-release within one version never prompts (#1788).
        assert!(!is_newer("0.1.0-build.abc1234", "0.1.0-build.abc1234"));
        assert!(!is_newer("0.1.0-build.def5678", "0.1.0-build.abc1234"));
        assert!(!is_newer("0.1.0-build.abc1234", "0.1.0-build.def5678"));
        // A bumped semver is newer whatever the build identifiers look like.
        assert!(is_newer("0.2.0-build.abc1234", "0.1.0-build.def5678"));
        assert!(is_newer("0.1.0-build.abc1234", "0.1.0-build.260812-001-3-gabc1234") == false);
    }

    /// #466: a dev build sitting on commits past the latest tag (`git describe` appends
    /// `-N-g<sha>`) is ahead of that release, not behind it.
    #[test]
    fn dev_build_past_the_latest_tag_is_not_an_update() {
        assert!(!is_newer(
            "0.1.0-build.260812-001",
            "0.1.0-build.260812-001-3-gabc1234"
        ));
        assert!(is_newer(
            "0.1.0-build.260812-002",
            "0.1.0-build.260812-001-3-gabc1234"
        ));
    }

    /// #764: and once releases march past that tag, the dev build is *still* ahead — it
    /// carries unreleased work, so no update badge, whatever the numbers say.
    #[test]
    fn dev_builds_never_see_an_update() {
        assert!(
            is_dev_build(),
            "the test binary is a dev build (debug assertions, untagged describe)"
        );
        // The comparison that the badge is gated on: a much newer release still can't
        // reach a dev build, because the gate is `!is_dev_build()`.
        assert!(is_newer(
            "0.1.0-build.260899-999",
            "0.1.0-build.260812-001-3-gabc1234"
        ));
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

    /// #1288: when a version is known, the download URL pins that release tag.
    #[test]
    fn platform_artifact_url_for_version_uses_tag() {
        let url = platform_artifact_url_for(Some("0.1.0-build.260812-001"));
        assert!(
            url.contains("/releases/download/v0.1.0-build.260812-001/"),
            "{url}"
        );
        assert!(url.ends_with(platform_artifact()), "{url}");
        // Leading `v` is not doubled.
        let url2 = platform_artifact_url_for(Some("v0.1.0-build.260812-001"));
        assert_eq!(url, url2);
        assert_eq!(
            release_page_url_for("0.1.0-build.1"),
            format!("{GITHUB_REPO}/releases/tag/v0.1.0-build.1")
        );
    }

    /// #1288: release channel reads a single release object; pre-release skips drafts
    /// and accepts prereleases from a list (newest first).
    #[test]
    fn parse_latest_version_respects_channel() {
        let latest = br#"{
            "tag_name": "v0.1.0-build.100",
            "draft": false,
            "prerelease": false
        }"#;
        assert_eq!(
            parse_latest_version_json(latest, UpdateChannel::Release).as_deref(),
            Some("0.1.0-build.100")
        );

        let list = br#"[
            {"tag_name": "v0.1.0-build.102", "draft": true, "prerelease": false},
            {"tag_name": "v0.1.0-build.101", "draft": false, "prerelease": true},
            {"tag_name": "v0.1.0-build.100", "draft": false, "prerelease": false}
        ]"#;
        // Pre-release channel: skip the draft, take the prerelease.
        assert_eq!(
            parse_latest_version_json(list, UpdateChannel::PreRelease).as_deref(),
            Some("0.1.0-build.101")
        );
        // Release channel is not used with a list body, but a missing tag_name yields None.
        assert_eq!(
            parse_latest_version_json(list, UpdateChannel::Release),
            None
        );

        // Only drafts → nothing to offer.
        let drafts = br#"[
            {"tag_name": "v0.1.0-build.99", "draft": true, "prerelease": false}
        ]"#;
        assert_eq!(
            parse_latest_version_json(drafts, UpdateChannel::PreRelease),
            None
        );
    }

    #[test]
    fn app_bundle_of_detects_bundles_and_bare_binaries() {
        assert_eq!(
            app_bundle_of(Path::new(
                "/Applications/BearCAD.app/Contents/MacOS/bearcad"
            )),
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

/// Loopback HTTP against the updater's ureq helper (#1610). Real sockets, no network.
#[cfg(test)]
mod tests_http {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::PathBuf;

    struct FakeHttp {
        port: u16,
        handle: std::thread::JoinHandle<Vec<String>>,
    }

    impl FakeHttp {
        fn serving(responses: Vec<Vec<u8>>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
            let port = listener.local_addr().unwrap().port();
            let handle = std::thread::spawn(move || {
                let mut requests = Vec::new();
                for response in responses {
                    let (mut socket, _) = listener.accept().expect("accept");
                    requests.push(read_request(&socket));
                    let _ = socket.write_all(&response);
                    let _ = socket.flush();
                    // Drop the socket so the client does not wait for keep-alive.
                    drop(socket);
                }
                requests
            });
            Self { port, handle }
        }

        fn url(&self, path: &str) -> String {
            format!("http://127.0.0.1:{}{path}", self.port)
        }

        fn received(self) -> Vec<String> {
            self.handle.join().expect("server thread")
        }
    }

    fn read_request(socket: &TcpStream) -> String {
        let mut reader = BufReader::new(socket);
        let mut head = String::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            let blank = line.trim().is_empty();
            head.push_str(&line);
            if blank {
                break;
            }
        }
        head
    }

    fn http_ok(body: &[u8], content_type: &str) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    fn http_empty(status: &str) -> Vec<u8> {
        format!("HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").into_bytes()
    }

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "bearcad_{name}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn fetch_latest_version_reads_github_json_over_http() {
        let body = br#"{"tag_name":"v0.1.0-build.100","draft":false,"prerelease":false}"#;
        let server = FakeHttp::serving(vec![http_ok(body, "application/json")]);
        let version =
            fetch_latest_version_from(&server.url("/releases/latest"), UpdateChannel::Release);
        assert_eq!(version.as_deref(), Some("0.1.0-build.100"));
        let requests = server.received();
        assert!(
            requests[0].starts_with("GET /releases/latest"),
            "got: {}",
            requests[0]
        );
        assert!(
            requests[0]
                .to_ascii_lowercase()
                .contains("user-agent: bearcad-update-check"),
            "GitHub requires a User-Agent; got: {}",
            requests[0]
        );
    }

    #[test]
    fn fetch_latest_version_is_none_when_the_server_errors() {
        let server = FakeHttp::serving(vec![http_empty("403 Forbidden")]);
        assert_eq!(
            fetch_latest_version_from(&server.url("/releases/latest"), UpdateChannel::Release),
            None
        );
        let _ = server.received();
    }

    #[test]
    fn http_get_bytes_returns_the_body() {
        let body = b"{\"ok\":true}";
        let server = FakeHttp::serving(vec![http_ok(body, "application/json")]);
        let got = http_get_bytes(&server.url("/")).expect("GET");
        assert_eq!(got, body);
        let _ = server.received();
    }

    #[test]
    fn http_get_bytes_fails_when_nothing_listens() {
        let err = http_get_bytes("http://127.0.0.1:1/").expect_err("connection refused");
        assert!(err.contains("127.0.0.1:1"), "got: {err}");
    }

    #[test]
    fn http_download_writes_the_file_including_binary_bytes() {
        let payload: Vec<u8> = (0u8..=255).cycle().take(512).collect();
        let server = FakeHttp::serving(vec![http_ok(&payload, "application/octet-stream")]);
        let path = scratch("http_download_test");
        http_download(&server.url("/artifact"), &path).expect("download");
        assert_eq!(std::fs::read(&path).unwrap(), payload);
        let _ = std::fs::remove_file(&path);
        let _ = server.received();
    }

    #[test]
    fn http_download_follows_redirects() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = b"staged-bytes";
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let first = read_request(&socket);
            let loc = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/file\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
            );
            let _ = socket.write_all(loc.as_bytes());
            let _ = socket.flush();
            drop(socket);
            let (mut socket, _) = listener.accept().unwrap();
            let second = read_request(&socket);
            let mut ok = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .into_bytes();
            ok.extend_from_slice(body);
            let _ = socket.write_all(&ok);
            let _ = socket.flush();
            (first, second)
        });

        let path = scratch("http_redirect_test");
        http_download(&format!("http://127.0.0.1:{port}/latest"), &path).expect("redirect");
        assert_eq!(std::fs::read(&path).unwrap(), body);
        let _ = std::fs::remove_file(&path);
        let (first, second) = handle.join().unwrap();
        assert!(first.starts_with("GET /latest"), "got: {first}");
        assert!(second.starts_with("GET /file"), "got: {second}");
    }

    #[test]
    fn http_download_rejects_an_empty_file() {
        let server = FakeHttp::serving(vec![http_empty("200 OK")]);
        let path = scratch("http_empty_test");
        let err = http_download(&server.url("/empty"), &path).expect_err("empty");
        assert!(err.contains("empty"), "got: {err}");
        let _ = server.received();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn http_download_fails_on_http_error() {
        let server = FakeHttp::serving(vec![http_empty("404 Not Found")]);
        let path = scratch("http_404_test");
        let err = http_download(&server.url("/missing"), &path).expect_err("404");
        assert!(
            err.contains("404") || err.contains("download failed"),
            "got: {err}"
        );
        let _ = server.received();
        let _ = std::fs::remove_file(&path);
    }
}
