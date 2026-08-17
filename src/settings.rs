//! Persisted app-level settings (#720).
//!
//! Documents carry their own state; this is the small remainder that belongs to the
//! *machine* — first the library directory, the folder imports can reference by a stable
//! library-relative path (`UnitSource::Library`) instead of a path relative to the
//! importing file. Stored as JSON in the platform config directory, loaded at startup,
//! saved on change. A missing or malformed file silently means defaults: settings are
//! never worth an error dialog at boot.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_true() -> bool {
    true
}

/// Which GitHub release stream the auto-updater watches (#1288).
///
/// - **Release** (default): only published non-prerelease releases (`/releases/latest`).
/// - **Pre-release**: the newest published release of either kind (includes prereleases).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    #[default]
    Release,
    PreRelease,
}

impl UpdateChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::PreRelease => "pre_release",
        }
    }

    /// Parse a script/UI string: `"release"` / `"stable"`, or `"pre_release"` /
    /// `"prerelease"` / `"pre-release"`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "release" | "stable" => Some(Self::Release),
            "pre_release" | "prerelease" | "pre-release" => Some(Self::PreRelease),
            _ => None,
        }
    }
}

/// Every persisted setting. Keep each field `#[serde(default)]` so older settings files
/// keep loading as more land here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSettings {
    /// The library directory (#720): where `Library(...)` import sources resolve.
    #[serde(default)]
    pub library_directory: Option<PathBuf>,
    /// Registry names of tutorials the user has finished (#1241). Confirm-SVG checks in
    /// the Tutorials pane (#1260); survives restarts.
    #[serde(default)]
    pub completed_tutorials: Vec<String>,
    /// User dismissed the unfinished-tutorials highlight and launch prompt (#1434).
    #[serde(default)]
    pub skip_all_tutorials: bool,
    /// Unix timestamp of first launch, stamped only when creating a new settings file
    /// (#1434). Missing on upgrades so existing installs are not treated as fresh.
    #[serde(default)]
    pub installed_at_unix: Option<i64>,
    /// When true, Zoom to Fit glides over [`crate::camera::ZOOM_TO_FIT_DURATION`]
    /// (half Home, #1276/#1303). When false, the camera snaps. On by default.
    #[serde(default = "default_true")]
    pub animate_zoom_to_fit: bool,
    /// Auto-update channel (#1288): release (default) or pre-release.
    #[serde(default)]
    pub update_channel: UpdateChannel,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            library_directory: None,
            completed_tutorials: Vec::new(),
            skip_all_tutorials: false,
            installed_at_unix: None,
            animate_zoom_to_fit: true,
            update_channel: UpdateChannel::Release,
        }
    }
}

/// Where the settings file lives: the platform config directory + `BearCAD/settings.json`.
/// `None` when the platform gives us no home (then settings just don't persist).
pub fn settings_path() -> Option<PathBuf> {
    let base = if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support")
    } else if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else {
        match std::env::var_os("XDG_CONFIG_HOME") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
        }
    };
    Some(base.join("BearCAD").join("settings.json"))
}

impl AppSettings {
    /// Load from the standard location; missing or malformed → defaults, no error.
    /// A missing file is a fresh install: stamp [`Self::installed_at_unix`] and write it.
    pub fn load() -> Self {
        settings_path()
            .map(|p| Self::load_or_init(&p))
            .unwrap_or_default()
    }

    /// Load from `path`. If the file does not exist, this is a fresh install — stamp
    /// the install time and write defaults. An existing file without `installed_at_unix`
    /// is an upgrade, not a fresh install (#1434).
    pub fn load_or_init(path: &std::path::Path) -> Self {
        if path.exists() {
            return Self::load_from(path);
        }
        let mut settings = Self::default();
        settings.installed_at_unix = Some(
            crate::time::SystemTime::now()
                .duration_since(crate::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        );
        let _ = settings.save_to(path);
        settings
    }

    /// Load from an explicit path (the standard one, or a test's temp file).
    pub fn load_from(path: &std::path::Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Save to the standard location, creating its directory. Errors are returned (for
    /// the status bar), never fatal.
    pub fn save(&self) -> Result<(), String> {
        let path = settings_path().ok_or("no settings directory on this platform")?;
        self.save_to(&path)
    }

    /// Save to an explicit path (the standard one, or a test's temp file).
    pub fn save_to(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn settings_round_trip() {
        let path = temp_file("bearcad_settings_roundtrip.json");
        let _ = std::fs::remove_file(&path);
        let settings = AppSettings {
            library_directory: Some(PathBuf::from("/some/library")),
            completed_tutorials: vec!["navigate".into(), "cube".into()],
            skip_all_tutorials: true,
            installed_at_unix: Some(1_700_000_000),
            animate_zoom_to_fit: false,
            update_channel: UpdateChannel::PreRelease,
        };
        settings.save_to(&path).unwrap();
        assert_eq!(AppSettings::load_from(&path), settings);
        std::fs::remove_file(&path).unwrap();
    }

    /// #1241: older settings files without `completed_tutorials` still load.
    #[test]
    fn completed_tutorials_defaults_when_absent() {
        let path = temp_file("bearcad_settings_no_tutorials.json");
        std::fs::write(&path, br#"{"library_directory": null}"#).unwrap();
        let loaded = AppSettings::load_from(&path);
        assert!(loaded.completed_tutorials.is_empty());
        std::fs::remove_file(&path).unwrap();
    }

    /// #1276: older settings files without `animate_zoom_to_fit` still animate (default on).
    #[test]
    fn animate_zoom_to_fit_defaults_on_when_absent() {
        let path = temp_file("bearcad_settings_no_zoom_anim.json");
        std::fs::write(&path, br#"{"library_directory": null}"#).unwrap();
        let loaded = AppSettings::load_from(&path);
        assert!(loaded.animate_zoom_to_fit);
        std::fs::remove_file(&path).unwrap();
    }

    /// #1288: older settings files without `update_channel` stay on the release channel.
    #[test]
    fn update_channel_defaults_to_release_when_absent() {
        let path = temp_file("bearcad_settings_no_channel.json");
        std::fs::write(&path, br#"{"library_directory": null}"#).unwrap();
        let loaded = AppSettings::load_from(&path);
        assert_eq!(loaded.update_channel, UpdateChannel::Release);
        std::fs::remove_file(&path).unwrap();
    }

    /// #1434: a brand-new settings file is a fresh install and records the time.
    #[test]
    fn missing_settings_file_stamps_install_time() {
        let path = temp_file("bearcad_settings_fresh_install.json");
        let _ = std::fs::remove_file(&path);
        let stamped = AppSettings::load_or_init(&path);
        assert!(stamped.installed_at_unix.is_some());
        assert!(!stamped.skip_all_tutorials);
        let loaded = AppSettings::load_from(&path);
        assert_eq!(loaded.installed_at_unix, stamped.installed_at_unix);
        std::fs::remove_file(&path).unwrap();
    }

    /// #1434: an older settings file without `installed_at_unix` is an upgrade.
    #[test]
    fn existing_settings_without_install_time_are_an_upgrade() {
        let path = temp_file("bearcad_settings_upgrade_install.json");
        std::fs::write(&path, br#"{"library_directory": null}"#).unwrap();
        let loaded = AppSettings::load_or_init(&path);
        assert!(loaded.installed_at_unix.is_none());
        assert!(!loaded.skip_all_tutorials);
        std::fs::remove_file(&path).unwrap();
    }

    /// #1434: older settings files without `skip_all_tutorials` stay un-skipped.
    #[test]
    fn skip_all_tutorials_defaults_off_when_absent() {
        let path = temp_file("bearcad_settings_no_skip_all.json");
        std::fs::write(&path, br#"{"library_directory": null}"#).unwrap();
        let loaded = AppSettings::load_from(&path);
        assert!(!loaded.skip_all_tutorials);
        std::fs::remove_file(&path).unwrap();
    }

    /// #1288: channel strings scripts/UI accept.
    #[test]
    fn update_channel_parse() {
        assert_eq!(UpdateChannel::parse("release"), Some(UpdateChannel::Release));
        assert_eq!(UpdateChannel::parse("stable"), Some(UpdateChannel::Release));
        assert_eq!(
            UpdateChannel::parse("pre_release"),
            Some(UpdateChannel::PreRelease)
        );
        assert_eq!(
            UpdateChannel::parse("prerelease"),
            Some(UpdateChannel::PreRelease)
        );
        assert_eq!(
            UpdateChannel::parse("pre-release"),
            Some(UpdateChannel::PreRelease)
        );
        assert_eq!(UpdateChannel::parse("nightly"), None);
        assert_eq!(UpdateChannel::Release.as_str(), "release");
        assert_eq!(UpdateChannel::PreRelease.as_str(), "pre_release");
    }

    #[test]
    fn missing_or_malformed_file_means_defaults() {
        let missing = temp_file("bearcad_settings_missing_dir/nope/settings.json");
        assert_eq!(AppSettings::load_from(&missing), AppSettings::default());

        let path = temp_file("bearcad_settings_malformed.json");
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(AppSettings::load_from(&path), AppSettings::default());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn unknown_fields_and_absent_fields_load() {
        // Forward compatibility both ways: a newer file with extra fields, and an older
        // file missing fields, both load. Older files that still carry
        // `body_highlight_method` (#1110, removed in #1155) are just unknown fields.
        let path = temp_file("bearcad_settings_forward.json");
        std::fs::write(&path, br#"{"library_directory": null, "future_thing": 3}"#).unwrap();
        assert_eq!(AppSettings::load_from(&path), AppSettings::default());
        std::fs::write(
            &path,
            br#"{"library_directory": null, "body_highlight_method": "Outlining"}"#,
        )
        .unwrap();
        assert_eq!(AppSettings::load_from(&path), AppSettings::default());
        std::fs::write(&path, b"{}").unwrap();
        assert_eq!(AppSettings::load_from(&path), AppSettings::default());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn settings_path_is_under_a_bearcad_dir() {
        // Whatever the platform, the file is namespaced under a BearCAD directory.
        let path = settings_path().expect("a config path on dev machines");
        assert!(path.ends_with("BearCAD/settings.json"), "{path:?}");
    }
}
