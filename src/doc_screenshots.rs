//! Guards for the documentation screenshots the docs site links to.
//!
//! A page can reference `/img/screenshots/foo.png` that nothing ever captures, and both
//! Docusaurus and the deploy stay quiet about it (#1837): `onBrokenMarkdownImages` only
//! sees markdown `![]()`, and most doc images are JSX `<img src={useBaseUrl(...)}>`.
//! `docs-site/scripts/check-doc-screenshots.mjs` is the check; these tests keep it wired
//! in and keep the repo passing it.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    fn repo() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn have_node() -> bool {
        Command::new("node")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn run_check(args: &[&str]) -> Output {
        Command::new("node")
            .arg(repo().join("docs-site/scripts/check-doc-screenshots.mjs"))
            .args(args)
            .current_dir(repo())
            .output()
            .expect("run the screenshot check")
    }

    /// docs/ page + empty static/ + empty screenshots/ under one temp root.
    fn fixture(name: &str, page: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("bearcad-doc-shots-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs")).expect("docs dir");
        std::fs::create_dir_all(root.join("static/img/screenshots")).expect("static dir");
        std::fs::create_dir_all(root.join("screenshots")).expect("scripts dir");
        std::fs::write(root.join("docs/page.md"), page).expect("write page");
        root
    }

    fn fixture_args(root: &Path) -> Vec<String> {
        vec![
            format!("--docs={}", root.join("docs").display()),
            format!("--static={}", root.join("static/img/screenshots").display()),
            format!("--scripts={}", root.join("screenshots").display()),
        ]
    }

    /// A reference no capture script could ever satisfy is a typo — always a failure.
    #[test]
    fn a_reference_with_no_capture_script_fails_the_check() {
        if !have_node() {
            return;
        }
        let root = fixture(
            "ghost",
            "<img src={useBaseUrl(\"/img/screenshots/ghost.png\")} />\n",
        );
        let args = fixture_args(&root);
        let out = run_check(&args.iter().map(String::as_str).collect::<Vec<_>>());
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!out.status.success(), "expected a failure, got:\n{text}");
        assert!(text.contains("ghost.png"), "{text}");
    }

    /// A shot whose script exists but has not run yet only warns on the push path —
    /// screenshots legitimately lag master by up to a nightly — and fails under --strict,
    /// which is the nightly's own path, where every shot was just captured.
    #[test]
    fn an_uncaptured_but_generatable_shot_warns_and_fails_only_under_strict() {
        if !have_node() {
            return;
        }
        let root = fixture(
            "pending",
            "<img src={useBaseUrl(\"/img/screenshots/thing-blue.png\")} />\n",
        );
        std::fs::write(root.join("screenshots/thing.lua"), "-- captures thing-*\n")
            .expect("write script");
        let args = fixture_args(&root);
        let mut argv: Vec<&str> = args.iter().map(String::as_str).collect();

        let out = run_check(&argv);
        let text = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(
            out.status.success(),
            "a not-yet-captured shot must not block a docs deploy:\n{text}"
        );
        assert!(text.contains("thing-blue.png"), "{text}");

        argv.push("--strict");
        let out = run_check(&argv);
        assert!(
            !out.status.success(),
            "--strict must fail on a missing shot:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// The real site: every `/img/screenshots/...` a page links to is something a capture
    /// script produces.
    #[test]
    fn every_screenshot_the_docs_reference_has_a_capture_script() {
        if !have_node() {
            return;
        }
        let out = run_check(&[]);
        assert!(
            out.status.success(),
            "docs reference screenshots nothing captures:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// #1836: a red nightly used to sit in the Actions tab unread while the published
    /// screenshots froze 40 commits behind master. The workflow has to say so out loud.
    #[test]
    fn a_failing_or_stale_nightly_raises_an_alarm() {
        let wf = std::fs::read_to_string(repo().join(".github/workflows/docs.yml")).expect("wf");
        assert!(
            wf.contains("issues: write"),
            "the Website workflow needs issue-write permission to report a failed nightly"
        );
        assert!(
            wf.contains("gh issue"),
            "a failed or stale nightly must file/update a GitHub issue"
        );
        assert!(
            wf.contains("stale") && wf.contains("behind"),
            "the plan job must measure how far the published screenshots lag master"
        );
        let alert = wf
            .split("\n  alert:")
            .nth(1)
            .expect("the workflow needs an `alert` job");
        assert!(
            alert.contains("always()") && alert.contains("failure"),
            "the alert job must run even when an earlier job failed:\n{alert}"
        );
    }

    /// The staleness threshold is shared with the script that measures it.
    #[test]
    fn screenshot_staleness_is_measured_by_a_script_with_a_test() {
        let script = repo().join("scripts/screenshot-drift.sh");
        assert!(
            script.exists(),
            "scripts/screenshot-drift.sh should own the drift maths so it can be tested"
        );
        if cfg!(not(unix)) {
            return;
        }
        // A marker older than the threshold is stale; today's is not.
        let day = 24 * 60 * 60;
        let now = crate::time::SystemTime::now()
            .duration_since(crate::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        for (age_days, behind, want_stale) in [(0u64, 0u64, false), (0, 3, false), (3, 9, true)] {
            let out = Command::new("bash")
                .arg(&script)
                .current_dir(repo())
                .env("DRIFT_MARKER_TIME", (now - age_days * day).to_string())
                .env("DRIFT_BEHIND", behind.to_string())
                .output()
                .expect("run drift script");
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            assert!(out.status.success(), "{text}");
            assert!(
                text.contains(&format!("stale={want_stale}")),
                "{age_days} day(s) / {behind} commit(s) behind should be stale={want_stale}:\n{text}"
            );
        }
    }

    /// The check has to run as part of the docs build, and strictly on the nightly.
    #[test]
    fn the_docs_build_runs_the_check() {
        let pkg = std::fs::read_to_string(repo().join("docs-site/package.json")).expect("pkg");
        assert!(
            pkg.contains("check-doc-screenshots.mjs"),
            "docs-site/package.json must run the screenshot check as part of `npm run build`"
        );
        assert!(
            pkg.contains("\"prebuild\""),
            "wire it as `prebuild` so `npm run build` cannot skip it"
        );
        let wf = std::fs::read_to_string(repo().join(".github/workflows/docs.yml")).expect("wf");
        assert!(
            wf.contains("BEARCAD_DOCS_REQUIRE_SCREENSHOTS"),
            "the Website workflow must make the check strict when it generates screenshots"
        );
    }
}
