//! Release artifact names and README download links.

pub const GITHUB_REPO: &str = "https://github.com/iffy/BearCAD";

pub const LINUX_ARTIFACT: &str = "bearcad-linux-x86_64.tar.gz";
pub const MACOS_ARTIFACT: &str = "bearcad.dmg";
pub const WINDOWS_ARTIFACT: &str = "bearcad.exe";

pub const RELEASES_BASE: &str = "https://github.com/iffy/BearCAD/releases/latest/download";

/// Hosted web app (wasm). Chromebooks install this as a PWA; desktop browsers run it in-tab.
#[cfg_attr(not(test), allow(dead_code))]
pub const WEB_APP_URL: &str = "https://bearcad.com/app/";

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

    /// #1328: release identity and notes come from `changer bump`, not Cargo.toml +
    /// GitHub-generated notes. The existing YYMMDD-### build number is still appended.
    #[test]
    fn ci_uses_changer_bump_for_release_version_and_notes() {
        let workflow = include_str!("../.github/workflows/ci.yml");
        assert!(
            workflow.contains("release-changelog.sh"),
            "CI should compute the release version and notes via scripts/release-changelog.sh"
        );
        assert!(
            !workflow.contains("generate_release_notes: true"),
            "CI should not use GitHub-generated release notes"
        );
        assert!(
            workflow.contains("body_path:"),
            "CI should publish changer bump output as the release body"
        );
    }

    /// #1328: the full changelog.md from changer bump is baked into the release binary.
    #[test]
    fn ci_embeds_changer_changelog_in_release_binaries() {
        let workflow = include_str!("../.github/workflows/ci.yml");
        assert!(
            workflow.contains("BEARCAD_CHANGELOG_PATH"),
            "release builds should point cargo at the changer-produced changelog"
        );
        let build = include_str!("../build.rs");
        assert!(
            build.contains("BEARCAD_CHANGELOG_PATH"),
            "build.rs should bake BEARCAD_CHANGELOG_PATH (or CHANGELOG.md) into the binary"
        );
    }

    /// `release-changelog.sh full` is the file `changer bump` would write (new section + old).
    #[test]
    #[cfg(unix)]
    fn release_changelog_full_matches_changer_concatenation() {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/release-changelog.sh");
        if std::process::Command::new("changer")
            .arg("--help")
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            return;
        }
        let out = std::process::Command::new("bash")
            .arg(script)
            .arg("full")
            .output()
            .expect("run release-changelog.sh full");
        assert!(
            out.status.success(),
            "script failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let got = String::from_utf8(out.stdout).unwrap();
        assert!(
            got.contains("# v") && got.contains("# v0.1.0"),
            "full changelog should include the new section and the existing one: {got:?}"
        );
        // The two version headings must not be glued onto one line.
        assert!(
            !got.contains("# v0.1.0") || got.contains("\n# v0.1.0"),
            "changer sections must be separated by a newline: {got:?}"
        );
    }

    /// `changer bump` deletes consumed snippets, so the next bump only sees new entries.
    #[test]
    #[cfg(unix)]
    fn changer_bump_excludes_already_released_entries() {
        if std::process::Command::new("changer")
            .arg("--help")
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            return;
        }
        let dir = std::env::temp_dir().join("bearcad-changer-1328");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("changes")).unwrap();
        std::fs::write(dir.join("CHANGELOG.md"), "# v0.1.0 - 2026-01-01\n\n- Initial\n").unwrap();
        std::fs::write(
            dir.join("changes/config.toml"),
            "update_nimble = false\nupdate_package_json = false\n",
        )
        .unwrap();
        std::fs::write(dir.join("changes/fix-released.md"), "already released\n").unwrap();
        let bump = std::process::Command::new("changer")
            .args(["bump", "0.1.1"])
            .current_dir(&dir)
            .output()
            .expect("changer bump");
        assert!(
            bump.status.success(),
            "changer bump failed: {}",
            String::from_utf8_lossy(&bump.stderr)
        );
        assert!(
            !dir.join("changes/fix-released.md").exists(),
            "changer bump should consume released snippets"
        );
        std::fs::write(dir.join("changes/fix-later.md"), "not yet released\n").unwrap();
        let notes = std::process::Command::new("changer")
            .args(["bump", "-n"])
            .current_dir(&dir)
            .output()
            .expect("changer bump -n");
        let stdout = String::from_utf8_lossy(&notes.stdout);
        assert!(
            !stdout.contains("already released"),
            "next bump must not repeat released notes: {stdout}"
        );
        assert!(
            stdout.contains("not yet released"),
            "next bump should include leftover snippets: {stdout}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #1328: publishing a draft runs `changer bump`, commits CHANGELOG, and tags vX.Y.Z.
    #[test]
    fn publish_workflow_bumps_changelog_and_tags_version() {
        let workflow = include_str!("../.github/workflows/release-published.yml");
        assert!(
            workflow.contains("release:") && workflow.contains("published"),
            "workflow should run when a draft is published as a release or pre-release"
        );
        assert!(
            workflow.contains("changer bump") || workflow.contains("release-changelog.sh"),
            "publishing should update CHANGELOG via changer bump"
        );
        assert!(
            workflow.contains("git tag") || workflow.contains("tag v"),
            "publishing should tag the released commit with the changelog version"
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

    /// #1213: Chromebook installs the hosted web app as a PWA — list it with the
    /// other platform downloads on the README and landing page.
    #[test]
    fn readme_offers_chromebook_install_next_to_downloads() {
        let readme = include_str!("../README.md");
        assert!(
            readme.contains("Chromebook"),
            "README download table should name Chromebook"
        );
        assert!(
            readme.contains(WEB_APP_URL),
            "README should link Chromebook install to {WEB_APP_URL}"
        );
    }

    /// #1438: user-facing docs must not call BearCAD "free"; pricing is name-your-price.
    #[test]
    fn docs_say_name_your_price_not_that_bearcad_is_free() {
        let get = include_str!("../docs-site/src/components/GetBearCAD/index.js");
        assert!(
            get.contains("Name your price"),
            "download/pay block should lead with name-your-price"
        );
        assert!(
            !get.contains("BearCAD is free"),
            "download/pay block should not call BearCAD free"
        );

        let intro = include_str!("../docs-site/docs/intro.mdx");
        let intro_lc = intro.to_ascii_lowercase();
        assert!(
            intro_lc.contains("name your price"),
            "overview should say name your price"
        );
        assert!(
            !intro_lc.contains("bearcad is a free") && !intro_lc.contains("bearcad is free"),
            "overview should not call BearCAD free"
        );

        let why = include_str!("../docs-site/docs/why.mdx");
        assert!(
            why.contains("| **Cost** | Name your price |"),
            "why-page cost row should list BearCAD as name-your-price"
        );

        let home = include_str!("../docs-site/src/pages/index.js");
        assert!(
            home.to_ascii_lowercase().contains("name your price"),
            "landing page should mention name your price"
        );
    }

    /// #1450: the pay CTA is "Pay Whatever", and a matching blue button
    /// sits in the top nav next to Download.
    /// #1470: no dollar sign on the Pay Whatever buttons.
    #[test]
    fn pay_whatever_button_and_navbar() {
        let get = include_str!("../docs-site/src/components/GetBearCAD/index.js");
        assert!(
            get.contains("Pay Whatever"),
            "download/pay block button should say Pay Whatever"
        );
        assert!(
            !get.contains("Name a price"),
            "download/pay block should not use the old Name a price label"
        );
        assert!(
            !get.contains("$"),
            "Pay Whatever button should not have a $ prefix"
        );
        assert!(
            !get.contains("DollarIcon") && !get.contains("<svg"),
            "Pay Whatever button should not use a dollar SVG icon"
        );

        let icons = include_str!("../docs-site/src/components/GetBearCAD/icons.js");
        assert!(
            !icons.contains("DollarIcon") && !icons.contains("function Dollar"),
            "GetBearCAD icon set should not include a dollar SVG"
        );

        let config = include_str!("../docs-site/docusaurus.config.js");
        let download_at = config
            .find("label: 'Download'")
            .expect("navbar should include a Download item");
        let pay_at = config
            .find("Pay Whatever")
            .expect("navbar should include a Pay Whatever item");
        assert!(
            (download_at as i32 - pay_at as i32).abs() < 400,
            "Pay Whatever should sit next to Download in the navbar items list"
        );
        assert!(
            config.contains("className: 'navbar-pay'"),
            "navbar Pay Whatever should have a class so it can be styled as a blue pill"
        );

        let css = include_str!("../docs-site/src/css/custom.css");
        assert!(
            css.contains("navbar-pay") && (css.contains("#3b7dd8") || css.contains("var(--pay)")),
            "navbar Pay Whatever should be the same blue as the GetBearCAD pay button"
        );
        assert!(
            !css.contains("content: '$'"),
            "navbar Pay Whatever should not prefix a $ character"
        );
    }

    /// #1441: the copy | icon split sits on the page center, not left of it
    /// because the logo column was only as wide as the icon.
    /// #1444: copy+CTAs on the left (right-aligned), icon on the right
    /// (left-aligned), so both hug that centered gutter.
    #[test]
    fn homepage_hero_column_gutter_is_centered() {
        let css = include_str!("../docs-site/src/pages/index.module.css");
        let desktop = css
            .split("@media screen and (min-width: 800px)")
            .nth(1)
            .expect("desktop hero breakpoint");
        let hero_inner = desktop
            .split(".heroInner")
            .nth(1)
            .and_then(|s| s.split('}').next())
            .expect(".heroInner in desktop breakpoint");
        assert!(
            hero_inner.contains("grid-template-columns: 1fr 1fr")
                || hero_inner.contains("grid-template-columns: minmax(0, 1fr) minmax(0, 1fr)"),
            "desktop hero should use equal columns so the gutter is page-centered, got:{hero_inner}"
        );

        let hero_inner_base = css
            .split(".heroInner")
            .nth(1)
            .and_then(|s| s.split('}').next())
            .expect(".heroInner base rule");
        assert!(
            hero_inner_base.contains("margin: 0 auto"),
            "hero inner should be centered so equal columns put the gutter on the page midline"
        );

        let rule = |class: &str| {
            desktop
                .split(class)
                .nth(1)
                .and_then(|s| s.split('}').next())
                .unwrap_or_else(|| panic!("{class} in desktop breakpoint"))
                .to_string()
        };
        let hero_brand = rule(".heroBrand");
        assert!(
            hero_brand.contains("grid-column: 2") && hero_brand.contains("justify-self: start"),
            "desktop icon should sit in the right column, left-aligned to the gutter, got:{hero_brand}"
        );
        let hero_copy = rule(".heroCopy");
        assert!(
            hero_copy.contains("text-align: right"),
            "desktop copy should right-align against the centered gutter, got:{hero_copy}"
        );
        let hero_ctas = rule(".heroCtas");
        assert!(
            hero_ctas.contains("grid-column: 1") && hero_ctas.contains("justify-self: end"),
            "desktop CTAs should sit with the copy, right-aligned to the gutter, got:{hero_ctas}"
        );
        let desktop_title = rule(".title");
        assert!(
            desktop_title.contains("text-align: right"),
            "desktop title should stay right-aligned against the gutter, got:{desktop_title}"
        );
    }

    /// Mobile stacks the hero in one column; Small/Quick/Fun/Bear should
    /// sit centered, not inherit the desktop right-align.
    #[test]
    fn homepage_hero_title_is_centered_on_mobile() {
        let css = include_str!("../docs-site/src/pages/index.module.css");
        let mobile = css
            .split("@media screen and (min-width: 800px)")
            .next()
            .expect("mobile hero styles");
        let title = mobile
            .split(".title {")
            .nth(1)
            .and_then(|s| s.split('}').next())
            .expect(".title base rule");
        assert!(
            title.contains("text-align: center"),
            "mobile Small/Quick/Fun/Bear title should be centered, got:{title}"
        );
        assert!(
            !title.contains("text-align: right"),
            "mobile title must not be right-aligned, got:{title}"
        );
    }

    /// Landing hero uses the colorful 2×2×2 materials viewport shot, not a
    /// second full-window UI dump.
    #[test]
    fn homepage_hero_uses_materials_screenshot() {
        let home = include_str!("../docs-site/src/pages/index.js");
        assert!(
            home.contains("/img/screenshots/materials.png"),
            "landing hero should show the materials 2×2×2 cubes screenshot"
        );
        assert!(
            !home.contains("/img/screenshots/elements-pane.png"),
            "landing page should not reuse the elements-pane UI screenshot"
        );
    }

    /// The finished bracket shot belongs on the Quickstart page, not the
    /// marketing homepage.
    #[test]
    fn homepage_does_not_show_quickstart_bracket() {
        let home = include_str!("../docs-site/src/pages/index.js");
        assert!(
            !home.contains("/img/screenshots/quickstart.png"),
            "landing page should not include the quickstart bracket screenshot"
        );
    }

    /// #1213 / #1439 / #1440 / #1443: Chromebook install sits next to the other
    /// platform downloads. After the name-your-price landing, the hero
    /// Download CTA jumps to the GetBearCAD block (`#get`); the dedicated
    /// download page still names Chromebooks and points at the hosted web app.
    #[test]
    fn homepage_offers_chromebook_install_next_to_downloads() {
        let home = include_str!("../docs-site/src/pages/index.js");
        assert!(
            home.contains("Download") && home.contains("#get"),
            "landing page Download should jump to the GetBearCAD block"
        );
        assert!(
            !home.contains("/docs/downloads"),
            "landing Download should not point at /docs/downloads"
        );
        assert!(
            home.contains("GetBearCAD"),
            "landing page should embed the GetBearCAD download/pay block"
        );

        let downloads = include_str!("../docs-site/docs/downloads.mdx");
        assert!(
            downloads.contains("Chromebook"),
            "download page should name Chromebook"
        );
        // Same relative /app/ path the "Run in your browser" CTA already uses.
        assert!(
            downloads.contains("pathname:///app/") || downloads.contains(WEB_APP_URL),
            "download page Chromebook install should point at the hosted web app"
        );
    }

    /// #1213: ChromeOS installs the web app when it is a proper PWA (manifest + SW).
    #[test]
    fn web_app_is_installable_pwa() {
        let index = include_str!("../web/index.html");
        assert!(
            index.contains("manifest.webmanifest") || index.contains("manifest.json"),
            "web/index.html should link a web app manifest"
        );
        assert!(
            index.contains("serviceWorker") || index.contains("service-worker"),
            "web/index.html should register a service worker"
        );

        let manifest = include_str!("../web/manifest.webmanifest");
        for needle in [
            "\"name\"",
            "\"short_name\"",
            "\"start_url\"",
            "\"display\"",
            "\"icons\"",
            "icon-192",
            "icon-512",
        ] {
            assert!(
                manifest.contains(needle),
                "manifest.webmanifest should include {needle}"
            );
        }
        assert!(
            manifest.contains("standalone")
                || manifest.contains("fullscreen")
                || manifest.contains("minimal-ui"),
            "manifest display must be installable (standalone/fullscreen/minimal-ui)"
        );

        let sw = include_str!("../web/sw.js");
        assert!(
            sw.contains("fetch"),
            "service worker needs a fetch handler for Chrome installability"
        );

        let build = include_str!("../scripts/build-web.sh");
        for asset in [
            "manifest.webmanifest",
            "sw.js",
            "icon-192.png",
            "icon-512.png",
        ] {
            assert!(
                build.contains(asset),
                "build-web.sh should ship {asset} into web/dist/"
            );
        }
    }

    /// #1244: wasm-bindgen imports every `fn kernel_*` from web/kernel-bridge.js as a
    /// named ES export. A missing export is a hard module load SyntaxError (app won't
    /// start). Keep the Rust extern block and the JS bridge in lockstep; also require
    /// each `_bearcad_*` the bridge calls to be listed in the Emscripten export list.
    #[test]
    fn kernel_bridge_exports_match_web_rs_imports() {
        let web_rs = include_str!("kernel/web.rs");
        let bridge = include_str!("../web/kernel-bridge.js");
        let emcc = include_str!("../scripts/build-occt-wasm.sh");

        fn kernel_fns<'a>(src: &'a str, line_prefix: &str) -> Vec<&'a str> {
            let mut out = Vec::new();
            for line in src.lines() {
                let t = line.trim_start();
                if let Some(rest) = t.strip_prefix(line_prefix) {
                    if rest.starts_with("kernel_") {
                        let name = rest.split('(').next().unwrap_or("");
                        if !name.is_empty() {
                            out.push(name);
                        }
                    }
                }
            }
            out.sort_unstable();
            out.dedup();
            out
        }

        let imports = kernel_fns(web_rs, "fn ");
        let exports = kernel_fns(bridge, "export function ");
        assert_eq!(
            imports, exports,
            "src/kernel/web.rs imports must match export function names in web/kernel-bridge.js"
        );
        assert!(
            exports.iter().any(|n| *n == "kernel_shell"),
            "kernel_shell must be exported (Shell tool web path)"
        );

        // Every `_bearcad_*` the bridge names must appear in EXPORTED_FUNCTIONS.
        let mut rest = bridge;
        while let Some(i) = rest.find("_bearcad_") {
            let after = &rest[i..];
            let end = after
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(after.len());
            let sym = &after[..end];
            assert!(
                emcc.contains(&format!("\"{sym}\"")),
                "build-occt-wasm.sh EXPORTED_FUNCTIONS must include {sym} (referenced by kernel-bridge.js)"
            );
            rest = &after[end..];
        }
    }
}