//! Release artifact names and README download links.

pub const GITHUB_REPO: &str = "https://github.com/iffy/BearCAD";

pub const LINUX_ARTIFACT: &str = "bearcad-linux-x86_64.tar.gz";
pub const MACOS_ARTIFACT: &str = "bearcad.dmg";
pub const WINDOWS_ARTIFACT: &str = "bearcad.exe";

pub const RELEASES_BASE: &str = "https://github.com/iffy/BearCAD/releases/latest/download";

pub fn download_url(artifact: &str) -> String {
    format!("{RELEASES_BASE}/{artifact}")
}

#[cfg_attr(not(test), allow(dead_code))]
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

    /// #1129: release build numbers are YYMMDD-### (per UTC day), not GITHUB_RUN_NUMBER.
    /// Runs only on Unix: Windows CI has no reliable bash for this script, and release-id
    /// only invokes it on ubuntu-latest.
    #[test]
    #[cfg(unix)]
    fn next_build_number_is_yymmdd_sequence_per_day() {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/next-build-number.sh");
        let run = |date: &str, tags: &str| {
            let out = std::process::Command::new("bash")
                .args([script, "--date", date])
                // Don't inherit GITHUB_REPOSITORY — a TTY-ish stdin would otherwise make
                // the script call `gh` instead of reading the piped tags.
                .env_remove("GITHUB_REPOSITORY")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut child| {
                    use std::io::Write;
                    if let Some(mut stdin) = child.stdin.take() {
                        stdin.write_all(tags.as_bytes())?;
                    }
                    child.wait_with_output()
                })
                .expect("run next-build-number.sh");
            assert!(
                out.status.success(),
                "script failed (status {:?}): stderr={} stdout={}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr),
                String::from_utf8_lossy(&out.stdout),
            );
            String::from_utf8(out.stdout)
                .expect("utf8")
                .trim()
                .to_string()
        };

        assert_eq!(run("260812", ""), "260812-001");
        assert_eq!(
            run(
                "260812",
                "v0.1.0-build.260812-001\nv0.1.0-build.260812-002\nv0.1.0-build.260811-009\n"
            ),
            "260812-003"
        );
        // Legacy GITHUB_RUN_NUMBER tags must not be mistaken for today's sequence.
        assert_eq!(run("260812", "v0.1.0-build.628\nv0.1.0-build.591\n"), "260812-001");
        // Zero-padding: sequence 10 → next is 011.
        assert_eq!(
            run("260808", "v0.1.0-build.260808-009\nv0.1.0-build.260808-010\n"),
            "260808-011"
        );
    }

    #[test]
    fn ci_uses_date_style_build_numbers() {
        let workflow = include_str!("../.github/workflows/ci.yml");
        assert!(
            workflow.contains("next-build-number.sh"),
            "CI should compute build numbers via next-build-number.sh"
        );
        assert!(
            !workflow.contains("github.run_number"),
            "CI should not use GITHUB_RUN_NUMBER as the release build number"
        );
        assert!(
            !workflow.contains("GITHUB_RUN_NUMBER"),
            "CI should not use GITHUB_RUN_NUMBER as the release build number"
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