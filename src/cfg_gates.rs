//! A source lint: a function defined behind `#[cfg(target_os = "…")]` must not be
//! *named* by code that also compiles on other platforms.
//!
//! `cfg!(target_os = "macos")` is a runtime boolean, not a compile-time gate, so a
//! call guarded only by it still has to resolve everywhere — which is how #1835
//! broke the Linux build from a machine where `cargo build` was green.

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    /// A region of a file introduced by one or more `#[cfg(…)]` attributes.
    struct Region {
        cfg: String,
        start: usize,
        /// The line of the item the attributes govern.
        item: usize,
        end: usize,
    }

    struct SourceFile {
        path: PathBuf,
        lines: Vec<String>,
        regions: Vec<Region>,
    }

    fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                rust_sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// The `#[cfg(...)]` payload of an attribute line, if it is one.
    fn cfg_payload(line: &str) -> Option<String> {
        let t = line.trim();
        let rest = t.strip_prefix("#[cfg(")?;
        let inner = rest.strip_suffix(")]")?;
        Some(inner.to_string())
    }

    fn is_attribute_or_doc(line: &str) -> bool {
        let t = line.trim();
        t.starts_with("#[") || t.starts_with("#![") || t.starts_with("///") || t.starts_with("//!")
    }

    /// Map every `#[cfg(...)]` attribute to the span of lines it governs.
    fn parse(path: &Path) -> SourceFile {
        let text = std::fs::read_to_string(path).expect("read source");
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let mut regions = Vec::new();
        let mut pending: Vec<String> = Vec::new();
        let mut pending_start = 0usize;
        // (depth the item opened at, cfg text, attribute line, item line)
        let mut open: Vec<(i32, String, usize, usize)> = Vec::new();
        let mut depth: i32 = 0;

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("//") {
                continue;
            }
            if let Some(cfg) = cfg_payload(line) {
                if pending.is_empty() {
                    pending_start = i;
                }
                pending.push(cfg);
                continue;
            }
            if !pending.is_empty() && is_attribute_or_doc(line) {
                continue;
            }

            let opens = line.matches('{').count() as i32;
            let closes = line.matches('}').count() as i32;

            if !pending.is_empty() {
                let cfg = pending.join(" && ");
                if opens > 0 {
                    open.push((depth, cfg, pending_start, i));
                } else if trimmed.ends_with(';') {
                    regions.push(Region { cfg, start: pending_start, item: i, end: i });
                } else {
                    // Multi-line signature: keep waiting for the opening brace.
                    pending = vec![cfg];
                    depth += opens - closes;
                    continue;
                }
                pending.clear();
            }

            depth += opens - closes;
            while let Some(&(d, _, _, _)) = open.last() {
                if depth <= d {
                    let (_, cfg, start, item) = open.pop().expect("open region");
                    regions.push(Region { cfg, start, item, end: i });
                } else {
                    break;
                }
            }
        }
        for (_, cfg, start, item) in open {
            regions.push(Region { cfg, start, item, end: lines.len() });
        }
        SourceFile { path: path.to_path_buf(), lines, regions }
    }

    fn fn_name(line: &str) -> Option<&str> {
        let t = line.trim();
        let rest = t
            .strip_prefix("pub ")
            .map(|r| r.trim_start_matches("(crate) ").trim_start())
            .unwrap_or(t);
        let rest = rest.strip_prefix("unsafe ").unwrap_or(rest);
        let rest = rest.strip_prefix("async ").unwrap_or(rest);
        let rest = rest.strip_prefix("extern \"C\" ").unwrap_or(rest);
        let rest = rest.strip_prefix("fn ")?;
        let end = rest.find(|c: char| !c.is_alphanumeric() && c != '_')?;
        Some(&rest[..end])
    }

    /// The single OS a cfg expression pins to, if it pins to exactly one positively.
    /// `any(...)` and `not(...)` are treated as "not pinned" — `any(test, target_os =
    /// "macos")` really does compile off macOS.
    fn sole_target_os(cfg: &str) -> Option<String> {
        if cfg.contains("not(") || cfg.contains("any(") {
            return None;
        }
        let mut found = None;
        for (idx, _) in cfg.match_indices("target_os = \"") {
            let rest = &cfg[idx + "target_os = \"".len()..];
            let os = rest.split('"').next()?.to_string();
            if found.is_some_and(|f: String| f != os) {
                return None;
            }
            found = Some(os);
        }
        found
    }

    /// Does `cfg` positively require `os`?
    fn covers(cfg: &str, os: &str) -> bool {
        let needle = format!("target_os = \"{os}\"");
        cfg.contains(&needle) && !cfg.contains(&format!("not({needle}"))
    }

    #[test]
    fn os_gated_functions_are_only_called_from_matching_gates() {
        let src = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
        let mut paths = Vec::new();
        rust_sources(&src, &mut paths);
        let files: Vec<SourceFile> = paths.iter().map(|p| parse(p)).collect();

        // name -> the OS it is gated to, or None once any other definition is seen.
        let mut gated: BTreeMap<String, Option<String>> = BTreeMap::new();
        for file in &files {
            for (i, line) in file.lines.iter().enumerate() {
                let Some(name) = fn_name(line) else { continue };
                // Only the attributes on the function itself gate it; a `fn` nested in
                // some other gated item is a local helper, not a platform variant.
                let os = file
                    .regions
                    .iter()
                    .filter(|r| r.item == i)
                    .find_map(|r| sole_target_os(&r.cfg));
                match gated.get(name) {
                    Some(Some(prev)) if Some(prev.as_str()) == os.as_deref() => {}
                    Some(_) => {
                        gated.insert(name.to_string(), None);
                    }
                    None => {
                        gated.insert(name.to_string(), os);
                    }
                }
            }
        }
        let gated: BTreeMap<String, String> = gated
            .into_iter()
            .filter_map(|(k, v)| v.map(|os| (k, os)))
            .collect();

        let mut problems = BTreeSet::new();
        for file in &files {
            for (i, line) in file.lines.iter().enumerate() {
                let trimmed = line.trim();
                if trimmed.starts_with("//") || fn_name(line).is_some() {
                    continue;
                }
                for (name, os) in &gated {
                    let call = format!("{name}(");
                    let Some(at) = line.find(&call) else { continue };
                    // Skip `foo.name(` method calls and `something_name(`.
                    if line[..at]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
                    {
                        continue;
                    }
                    let guarded = file
                        .regions
                        .iter()
                        .any(|r| r.start <= i && i <= r.end && covers(&r.cfg, os));
                    if !guarded {
                        problems.insert(format!(
                            "{}:{}: `{name}` is defined only for target_os = \"{os}\" but is \
                             named here without a matching #[cfg] gate",
                            file.path
                                .strip_prefix(env!("CARGO_MANIFEST_DIR"))
                                .unwrap_or(&file.path)
                                .display(),
                            i + 1
                        ));
                    }
                }
            }
        }

        assert!(
            problems.is_empty(),
            "platform-gated functions called from portable code (this breaks other \
             platforms' builds; `cfg!(...)` is a runtime bool, not a gate):\n{}",
            problems.into_iter().collect::<Vec<_>>().join("\n")
        );
    }
}
