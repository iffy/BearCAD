//! Release artifact names and README download links.

pub const GITHUB_REPO: &str = "https://github.com/iffy/BearCAD";

pub const LINUX_ARTIFACT: &str = "bearcad-linux-x86_64.tar.gz";
pub const MACOS_ARTIFACT: &str = "bearcad.dmg";
pub const WINDOWS_ARTIFACT: &str = "bearcad.exe";

pub const RELEASES_BASE: &str = "https://github.com/iffy/BearCAD/releases/latest/download";

pub fn download_url(artifact: &str) -> String {
    format!("{RELEASES_BASE}/{artifact}")
}

pub const ALL_ARTIFACTS: &[&str] = &[LINUX_ARTIFACT, MACOS_ARTIFACT, WINDOWS_ARTIFACT];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_urls_use_github_repo() {
        assert!(RELEASES_BASE.starts_with(GITHUB_REPO));
    }

    #[test]
    fn prune_script_tolerates_missing_release_tags() {
        let script = include_str!("../scripts/prune-draft-releases.sh");
        assert!(
            !script.contains("--cleanup-tag"),
            "prune script should not use --cleanup-tag; draft releases may lack tag refs"
        );
        assert!(
            script.contains("git/refs/tags"),
            "prune script should best-effort delete orphaned tags"
        );
    }

    #[test]
    fn ci_publishes_draft_releases_and_prunes_old_drafts() {
        let workflow = include_str!("../.github/workflows/ci.yml");
        assert!(
            workflow.contains("draft: true"),
            "CI should publish draft releases"
        );
        assert!(
            workflow.contains("prerelease: false"),
            "CI releases should not be pre-releases"
        );
        assert!(
            workflow.contains("prune-draft-releases.sh 2"),
            "CI should keep only the two newest draft releases"
        );
    }

    #[test]
    fn readme_links_to_github_repo() {
        let readme = include_str!("../README.md");
        assert!(
            readme.contains(GITHUB_REPO),
            "README should link to {GITHUB_REPO}"
        );
    }

    #[test]
    fn readme_links_directly_to_each_platform_artifact() {
        let readme = include_str!("../README.md");
        for artifact in ALL_ARTIFACTS {
            let url = download_url(artifact);
            assert!(
                readme.contains(&url),
                "README should link directly to {url}"
            );
        }
    }
}