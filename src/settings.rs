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

/// How a selected or hovered body is highlighted in the 3D viewport (#1110).
///
/// `Outlining` (the default) draws a screen-space outline around the body's flattened
/// silhouette — blue for selected, yellow for hovered — leaving the body itself in its
/// material colour. `Shading` is the older look: the body's fill is recoloured instead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyHighlightMethod {
    Shading,
    #[default]
    Outlining,
}

impl BodyHighlightMethod {
    /// Stable name used in instruction scripts.
    pub fn script_name(self) -> &'static str {
        match self {
            BodyHighlightMethod::Shading => "shading",
            BodyHighlightMethod::Outlining => "outlining",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "shading" | "shade" | "fill" | "solid" => Some(BodyHighlightMethod::Shading),
            "outlining" | "outline" => Some(BodyHighlightMethod::Outlining),
            _ => None,
        }
    }
}

/// Every persisted setting. Keep each field `#[serde(default)]` so older settings files
/// keep loading as more land here.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSettings {
    /// The library directory (#720): where `Library(...)` import sources resolve.
    #[serde(default)]
    pub library_directory: Option<PathBuf>,
    /// How selected/hovered bodies highlight (#1110): recolour the fill, or draw an
    /// outline around the silhouette.
    #[serde(default)]
    pub body_highlight_method: BodyHighlightMethod,
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
    pub fn load() -> Self {
        settings_path().map(|p| Self::load_from(&p)).unwrap_or_default()
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
            body_highlight_method: BodyHighlightMethod::Outlining,
        };
        settings.save_to(&path).unwrap();
        assert_eq!(AppSettings::load_from(&path), settings);
        std::fs::remove_file(&path).unwrap();
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
        // file missing fields, both load.
        let path = temp_file("bearcad_settings_forward.json");
        std::fs::write(&path, br#"{"library_directory": null, "future_thing": 3}"#).unwrap();
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

    #[test]
    fn body_highlight_method_round_trips_through_serde() {
        for method in [BodyHighlightMethod::Shading, BodyHighlightMethod::Outlining] {
            let json = serde_json::to_string(&method).unwrap();
            assert_eq!(serde_json::from_str::<BodyHighlightMethod>(&json).unwrap(), method);
        }
    }

    #[test]
    fn body_highlight_method_script_name_round_trips() {
        for method in [BodyHighlightMethod::Shading, BodyHighlightMethod::Outlining] {
            assert_eq!(
                BodyHighlightMethod::from_name(method.script_name()),
                Some(method),
                "{:?} should round-trip through its script name",
                method,
            );
        }
        assert_eq!(BodyHighlightMethod::from_name("Shading"), Some(BodyHighlightMethod::Shading));
        assert_eq!(BodyHighlightMethod::from_name("OUTLINE"), Some(BodyHighlightMethod::Outlining));
        assert_eq!(BodyHighlightMethod::from_name("nope"), None);
    }

    #[test]
    fn body_highlight_method_defaults_to_outlining() {
        // Older settings files predate the field; `#[serde(default)]` means they load as
        // the default (Outlining), and a brand-new AppSettings is Outlining too.
        let path = temp_file("bearcad_settings_old_no_highlight.json");
        std::fs::write(&path, b"{\"library_directory\": null}").unwrap();
        assert_eq!(
            AppSettings::load_from(&path).body_highlight_method,
            BodyHighlightMethod::Outlining,
        );
        assert_eq!(
            AppSettings::default().body_highlight_method,
            BodyHighlightMethod::Outlining,
        );
        std::fs::remove_file(&path).unwrap();
    }
}
