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
    let rest = SKILL.strip_prefix("---\n")?;
    let end = rest.find("\n---\n")?;
    Some(&rest[..end])
}

/// The skill with its front matter removed — the form for tools that take a plain
/// instructions file (`AGENTS.md`, Copilot instructions) rather than a skill bundle.
pub fn body() -> &'static str {
    match SKILL.strip_prefix("---\n").and_then(|rest| {
        rest.find("\n---\n")
            .map(|end| &rest[end + "\n---\n".len()..])
    }) {
        Some(body) => body.trim_start(),
        None => SKILL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
