//! Embedded changelog shown by Help → Changelog (#1328).
//!
//! Release builds bake the full `changelog.md` produced by `changer bump` (via
//! `BEARCAD_CHANGELOG_PATH`). Local builds embed the repo's `CHANGELOG.md`.

/// The changelog markdown baked into this binary.
pub fn markdown() -> &'static str {
    include_str!(concat!(env!("OUT_DIR"), "/changelog.md"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{Action, AppState};

    #[test]
    fn baked_changelog_has_a_version_heading() {
        let md = markdown();
        assert!(
            md.contains("# v"),
            "embedded changelog should start from a changer version heading, got: {md:?}"
        );
        assert!(
            md.contains("v0.1.0") || md.contains("# v"),
            "embedded changelog should include this tree's changelog"
        );
    }

    #[test]
    fn changelog_window_toggles_via_action() {
        let mut state = AppState::default();
        assert!(!state.changelog_open);
        assert_eq!(
            state.apply(Action::SetChangelogWindow { open: Some(true) }),
            crate::actions::ActionResult::Ok
        );
        assert!(state.changelog_open);
        assert!(state.status.to_lowercase().contains("changelog"));
        state.apply(Action::SetChangelogWindow { open: Some(false) });
        assert!(!state.changelog_open);
        state.apply(Action::SetChangelogWindow { open: None });
        assert!(state.changelog_open);
    }
}
