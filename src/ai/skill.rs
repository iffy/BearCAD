//! The BearCAD agent skill (#1602): one markdown file that teaches an AI agent to drive
//! the app through its Lua API.
//!
//! **One source, two consumers.** The file lives at `docs-site/static/bearcad-skill.md`, so
//! the website serves it verbatim at `/bearcad-skill.md`, and it is compiled into the
//! binary here so `bearcad skill install` works offline. There is no second copy to keep in
//! step.
//!
//! The test at the bottom is what keeps it honest: every `bearcad.*` call the skill shows
//! must exist in the registered Lua API, so the API cannot move without the skill failing
//! to build.

/// The skill, exactly as the website serves it.
pub const SKILL: &str = include_str!("../../docs-site/static/bearcad-skill.md");

/// Where the published copy lives, for agents that fetch rather than read a file.
pub const SKILL_URL: &str = "https://iffy.github.io/BearCAD/bearcad-skill.md";

/// The skill's YAML front matter (`name:` / `description:`), which Anthropic-style skills
/// require and every other tool ignores.
pub fn front_matter() -> Option<&'static str> {
    parts().map(|(front, _)| front)
}

/// The skill with its front matter removed — the form for tools that take a plain
/// instructions file (`AGENTS.md`, Copilot instructions) rather than a skill bundle.
pub fn body() -> &'static str {
    match parts() {
        Some((_, body)) => body.trim_start(),
        None => SKILL,
    }
}

/// `(front matter, body)` with the skill's enclosing `---` lines removed. Tolerates LF or
/// CRLF line endings so the embedded copy parses identically whatever platform checked the
/// source out (a Windows checkout writes `\r\n`; the parser must not care).
fn parts() -> Option<(&'static str, &'static str)> {
    let Some(rest) = SKILL.strip_prefix("---") else {
        return None;
    };
    let Some(after_open) = strip_eol(rest) else {
        return None;
    };
    let Some(close) = after_open.find("\n---") else {
        return None;
    };
    let front_end = if after_open[..close].ends_with('\r') { close - 1 } else { close };
    let tail = &after_open[close + "\n---".len()..];
    let Some(after_close) = strip_eol(tail) else {
        return None;
    };
    Some((&after_open[..front_end], after_close))
}

/// `s` with one leading line ending removed, LF or CRLF.
fn strip_eol(s: &'static str) -> Option<&'static str> {
    if s.starts_with("\r\n") {
        Some(&s[2..])
    } else if s.starts_with("\n") {
        Some(&s[1..])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway home + project pair for install tests.
    fn temp_dirs(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("bearcad-skill-test-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        (home, project)
    }

    #[test]
    fn installing_a_file_of_our_own_writes_the_whole_skill_and_uninstall_removes_it() {
        let (home, project) = temp_dirs("own");
        let target = target("claude").expect("the Claude Code target");
        assert!(!target.installed(Some(&home), Some(&project)));

        let path = install(target, Some(&home), Some(&project)).expect("install");
        assert!(path.ends_with(".claude/skills/bearcad/SKILL.md"), "got {}", path.display());
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, SKILL, "an owned file gets the skill verbatim, front matter and all");
        assert!(target.installed(Some(&home), Some(&project)));

        uninstall(target, Some(&home), Some(&project)).expect("uninstall");
        assert!(!path.exists());
        assert!(!target.installed(Some(&home), Some(&project)));
    }

    #[test]
    fn installing_into_a_shared_file_leaves_the_rest_of_it_alone() {
        let (home, project) = temp_dirs("region");
        let target = target("agents").expect("the AGENTS.md target");
        let path = project.join("AGENTS.md");
        std::fs::write(&path, "# House rules

Always run the tests.
").unwrap();

        install(target, Some(&home), Some(&project)).expect("install");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# House rules"), "the user's content comes first");
        assert!(text.contains("Always run the tests."), "and survives");
        assert!(text.contains(BEGIN_MARKER) && text.contains(END_MARKER));
        // A shared instructions file gets the body, not the skill's front matter.
        assert!(!text.contains("name: bearcad"));
        assert!(text.contains("bearcad.extrude"));

        // Installing again replaces the region rather than stacking another copy.
        install(target, Some(&home), Some(&project)).expect("re-install");
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches(BEGIN_MARKER).count(), 1, "one region, not two");

        uninstall(target, Some(&home), Some(&project)).expect("uninstall");
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "# House rules

Always run the tests.
", "back to exactly what it was");
    }

    #[test]
    fn a_shared_file_that_was_only_ours_is_removed_rather_than_left_empty() {
        let (home, project) = temp_dirs("empty-region");
        let target = target("copilot").expect("the Copilot target");
        install(target, Some(&home), Some(&project)).expect("install");
        let path = target.path(Some(&home), Some(&project)).unwrap();
        assert!(path.exists());

        uninstall(target, Some(&home), Some(&project)).expect("uninstall");
        assert!(!path.exists(), "no empty stub left behind");
    }

    #[test]
    fn uninstalling_something_that_was_never_installed_is_quiet() {
        let (home, project) = temp_dirs("absent");
        for target in TARGETS {
            assert_eq!(
                uninstall(target, Some(&home), Some(&project)),
                Ok(None),
                "{} should be a no-op",
                target.id
            );
        }
    }

    #[test]
    fn a_project_target_without_a_directory_says_so_rather_than_guessing() {
        let (home, _) = temp_dirs("no-dir");
        let target = target("agents").unwrap();
        let error = install(target, Some(&home), None).expect_err("no project directory");
        assert!(error.contains("--dir"), "got: {error}");
    }

    #[test]
    fn detection_follows_the_tool_directory() {
        let (home, project) = temp_dirs("detect");
        let claude = target("claude").unwrap();
        assert!(!claude.detected(Some(&home), Some(&project)), "no ~/.claude yet");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        assert!(claude.detected(Some(&home), Some(&project)), "~/.claude exists");
        // A project target is always available — the file is ours to create.
        assert!(target("agents").unwrap().detected(Some(&home), Some(&project)));
    }

    #[test]
    fn every_target_is_uniquely_named_and_points_somewhere_sensible() {
        for target in TARGETS {
            assert!(!target.id.is_empty() && !target.label.is_empty());
            assert!(!target.note.is_empty(), "{} needs a note saying who reads it", target.id);
            assert!(
                !target.relative_path.starts_with('/'),
                "{} must be relative to home or project",
                target.id
            );
            assert_eq!(
                TARGETS.iter().filter(|t| t.id == target.id).count(),
                1,
                "duplicate target id {}",
                target.id
            );
        }
    }

    #[test]
    fn the_skill_has_the_front_matter_a_skill_bundle_needs() {
        let front = front_matter().expect("front matter");
        assert!(front.contains("name: bearcad"), "got: {front}");
        assert!(front.contains("description:"), "got: {front}");
        // The description is what a tool matches on, so it has to say when to use this.
        assert!(front.to_lowercase().contains("bearcad"));
        assert!(front.len() > 80, "a one-word description helps nobody");
    }

    #[test]
    fn the_body_drops_the_front_matter_and_keeps_the_content() {
        let body = body();
        assert!(!body.starts_with("---"), "front matter is stripped");
        assert!(body.starts_with("# BearCAD"), "got: {}", &body[..40.min(body.len())]);
        assert!(body.contains("bearcad.extrude"));
    }

    #[test]
    fn the_skill_covers_what_an_agent_needs_to_get_started() {
        for topic in [
            "--script",     // how to run one at all
            "--exit",       // and how not to leave a window open
            "--repl",
            "bearcad.new",
            "bearcad.rect",
            "bearcad.extrude",
            "bearcad.parameter",
            "bearcad.export_step",
            "bearcad.ui.screenshot",
            "millimetres",  // units, the classic silent mistake
            "mcp",          // how to reach a document that is already open
        ] {
            assert!(SKILL.contains(topic), "the skill should mention {topic}");
        }
    }

    /// Every `bearcad.<name>` and `bearcad.ui.<name>` the skill shows must exist.
    ///
    /// This is the anti-drift check: rename a scripting function and this test fails,
    /// rather than an agent discovering it at runtime.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn every_api_call_the_skill_shows_actually_exists() {
        let lua = mlua::Lua::new();
        crate::lua_script::register_api(&lua).expect("register the API");
        let bearcad: mlua::Table = lua.globals().get("bearcad").expect("bearcad table");

        let exists = |path: &str| -> bool {
            let mut table = bearcad.clone();
            let parts: Vec<&str> = path.split('.').skip(1).collect();
            for (index, part) in parts.iter().enumerate() {
                let value: mlua::Value = match table.get(*part) {
                    Ok(value) => value,
                    Err(_) => return false,
                };
                let last = index + 1 == parts.len();
                match value {
                    mlua::Value::Nil => return false,
                    mlua::Value::Table(next) if !last => table = next,
                    _ => return last,
                }
            }
            false
        };

        // Only calls, not prose: `bearcad.<path>(` or `bearcad.<path>{`.
        let mut checked = 0;
        for (index, _) in SKILL.match_indices("bearcad.") {
            let rest = &SKILL[index..];
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
                .unwrap_or(rest.len());
            let path = rest[..end].trim_end_matches('.');
            let next = rest[end..].chars().next();
            if !matches!(next, Some('(') | Some('{')) {
                continue; // Prose, or a table being indexed rather than called.
            }
            assert!(
                exists(path),
                "the skill calls {path}, which the Lua API does not have"
            );
            checked += 1;
        }
        assert!(checked > 30, "expected the skill to show real calls, saw {checked}");
    }
}

/// Where a skill install writes, and in what form.
///
/// Each target is a path plus a format. Paths are verified against each tool's own
/// documentation; a tool whose location cannot be confirmed does not get a guess — it gets
/// [`Format::Instructions`], which prints what to do by hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// A file this skill owns: written whole, front matter included, removed on uninstall.
    Own,
    /// A file shared with the user's own content: the skill lives between markers and
    /// nothing else in the file is touched.
    Region,
    /// No confirmed location — print instructions instead of writing somewhere wrong.
    Instructions,
}

/// Where the target's file lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Under the user's home directory: applies everywhere.
    User,
    /// Inside a project directory: applies to that project.
    Project,
}

/// One place the skill can be installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    /// Name used on the command line (`--target claude`).
    pub id: &'static str,
    /// What to call it in a list.
    pub label: &'static str,
    pub scope: Scope,
    pub format: Format,
    /// Path relative to the home (User) or project (Project) directory.
    pub relative_path: &'static str,
    /// One line on who reads this file.
    pub note: &'static str,
}

/// The markers a [`Format::Region`] install writes between. Anything outside them is the
/// user's, and is never touched.
pub const BEGIN_MARKER: &str = "<!-- BEGIN BearCAD skill -->";
pub const END_MARKER: &str = "<!-- END BearCAD skill -->";

/// Every install target, in the order `bearcad skill targets` lists them.
pub const TARGETS: &[Target] = &[
    Target {
        id: "claude",
        label: "Claude Code",
        scope: Scope::User,
        format: Format::Own,
        relative_path: ".claude/skills/bearcad/SKILL.md",
        // Grok Build reads `.claude/` natively, so this one file serves both.
        note: "~/.claude/skills — also read by Grok Build",
    },
    Target {
        id: "codex",
        label: "OpenAI Codex",
        scope: Scope::User,
        format: Format::Region,
        relative_path: ".codex/AGENTS.md",
        note: "~/.codex/AGENTS.md, Codex's global instructions",
    },
    Target {
        id: "claude-project",
        label: "Claude Code (this project)",
        scope: Scope::Project,
        format: Format::Own,
        relative_path: ".claude/skills/bearcad/SKILL.md",
        note: "project-local skill, checked in with the repo",
    },
    Target {
        id: "agents",
        label: "AGENTS.md",
        scope: Scope::Project,
        format: Format::Region,
        relative_path: "AGENTS.md",
        note: "the shared convention — Codex, Grok, Cursor and others read it",
    },
    Target {
        id: "copilot",
        label: "GitHub Copilot",
        scope: Scope::Project,
        format: Format::Region,
        relative_path: ".github/copilot-instructions.md",
        note: "VS Code and the Copilot CLI",
    },
    Target {
        id: "cursor",
        label: "Cursor",
        scope: Scope::Project,
        format: Format::Own,
        relative_path: ".cursor/rules/bearcad.mdc",
        note: "project rule file",
    },
];

pub fn target(id: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|t| t.id == id)
}

impl Target {
    /// The file this target writes, given a home directory and a project directory.
    /// `None` when the scope needs a directory that was not supplied.
    pub fn path(&self, home: Option<&std::path::Path>, project: Option<&std::path::Path>)
        -> Option<std::path::PathBuf>
    {
        let base = match self.scope {
            Scope::User => home?,
            Scope::Project => project?,
        };
        Some(base.join(self.relative_path))
    }

    /// Whether the tool this target belongs to looks present. A project target is
    /// "detected" whenever a project directory is given — the file is ours to create.
    pub fn detected(&self, home: Option<&std::path::Path>, project: Option<&std::path::Path>)
        -> bool
    {
        match self.scope {
            // The tool's own directory (~/.claude, ~/.codex) existing is the signal.
            Scope::User => self
                .path(home, project)
                .and_then(|p| p.parent().map(|d| top_tool_dir(d, home)))
                .is_some_and(|dir| dir.exists()),
            Scope::Project => project.is_some(),
        }
    }

    /// Whether the skill is installed at this target right now.
    pub fn installed(&self, home: Option<&std::path::Path>, project: Option<&std::path::Path>)
        -> bool
    {
        let Some(path) = self.path(home, project) else {
            return false;
        };
        match self.format {
            Format::Own => path.exists(),
            Format::Region => std::fs::read_to_string(&path)
                .is_ok_and(|text| text.contains(BEGIN_MARKER)),
            Format::Instructions => false,
        }
    }

    /// The text this target's file should carry: the whole skill for a file of our own, the
    /// body (front matter stripped) for a shared instructions file.
    fn payload(&self) -> &'static str {
        match self.format {
            Format::Own => SKILL,
            _ => body(),
        }
    }
}

/// The tool directory immediately under `home` — `~/.claude` for `~/.claude/skills/bearcad`.
fn top_tool_dir(dir: &std::path::Path, home: Option<&std::path::Path>) -> std::path::PathBuf {
    let Some(home) = home else {
        return dir.to_path_buf();
    };
    match dir.strip_prefix(home).ok().and_then(|rest| rest.iter().next()) {
        Some(first) => home.join(first),
        None => dir.to_path_buf(),
    }
}

/// Install the skill for `target`. Returns the path written.
pub fn install(
    target: &Target,
    home: Option<&std::path::Path>,
    project: Option<&std::path::Path>,
) -> Result<std::path::PathBuf, String> {
    if target.format == Format::Instructions {
        return Err(format!(
            "{} has no confirmed location — run `bearcad skill print` and add it by hand",
            target.label
        ));
    }
    let path = target
        .path(home, project)
        .ok_or_else(|| match target.scope {
            Scope::Project => format!("{} needs a project directory (--dir)", target.label),
            Scope::User => format!("{}: no home directory on this platform", target.label),
        })?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let contents = match target.format {
        Format::Own => target.payload().to_string(),
        Format::Region => {
            let existing = std::fs::read_to_string(&path).unwrap_or_default();
            replace_region(&existing, target.payload())
        }
        Format::Instructions => unreachable!("returned above"),
    };
    std::fs::write(&path, contents).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

/// Remove the skill from `target`, leaving anything else in the file alone. Succeeds
/// quietly when nothing is installed.
pub fn uninstall(
    target: &Target,
    home: Option<&std::path::Path>,
    project: Option<&std::path::Path>,
) -> Result<Option<std::path::PathBuf>, String> {
    let Some(path) = target.path(home, project) else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }
    match target.format {
        Format::Own => {
            std::fs::remove_file(&path)
                .map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
            // Tidy the directory we created, but only if it is now empty.
            if let Some(dir) = path.parent() {
                let _ = std::fs::remove_dir(dir);
            }
            Ok(Some(path))
        }
        Format::Region => {
            let existing = std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let Some(without) = remove_region(&existing) else {
                return Ok(None);
            };
            if without.trim().is_empty() {
                // The file existed only for us; do not leave an empty stub behind.
                std::fs::remove_file(&path)
                    .map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
            } else {
                std::fs::write(&path, without)
                    .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            }
            Ok(Some(path))
        }
        Format::Instructions => Ok(None),
    }
}

/// `existing` with the marked region replaced by `payload`, appending it when there is no
/// region yet. Everything outside the markers is preserved byte for byte.
fn replace_region(existing: &str, payload: &str) -> String {
    let block = format!("{BEGIN_MARKER}\n{}\n{END_MARKER}\n", payload.trim_end());
    match (existing.find(BEGIN_MARKER), existing.find(END_MARKER)) {
        (Some(start), Some(end)) if end > start => {
            let tail = end + END_MARKER.len();
            let mut out = String::with_capacity(existing.len() + block.len());
            out.push_str(&existing[..start]);
            out.push_str(block.trim_end());
            out.push_str(&existing[tail..]);
            out
        }
        _ => {
            let mut out = existing.to_string();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&block);
            out
        }
    }
}

/// `existing` without the marked region, or `None` when there is no region in it.
fn remove_region(existing: &str) -> Option<String> {
    let start = existing.find(BEGIN_MARKER)?;
    let end = existing.find(END_MARKER)? + END_MARKER.len();
    if end <= start {
        return None;
    }
    let mut out = String::with_capacity(existing.len());
    out.push_str(existing[..start].trim_end());
    let tail = existing[end..].trim_start_matches('\n');
    if !out.is_empty() && !tail.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(tail);
    // Leave the file as a text file: the user's content keeps its trailing newline.
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}
