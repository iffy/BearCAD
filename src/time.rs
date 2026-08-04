//! The clock, once, for the whole crate (#1048).
//!
//! `std::time::Instant::now()` and `SystemTime::now()` **compile** for wasm32 and panic at
//! runtime — "time not implemented on this platform". A single startup timestamp took the web
//! app down to a black screen before its first frame, and nothing caught it: the wasm CI job
//! builds, and a build cannot see a runtime panic.
//!
//! Three modules had already grown their own `target_arch` switch for exactly this. Rather
//! than a fourth, everything in the crate reads its clock from here — a re-export of
//! [`web_time`] on wasm and of [`std::time`] everywhere else, identical in API. The raw paths
//! are then banned outright, which `no_raw_std_time_outside_this_module` enforces, so the next
//! one is caught by `cargo test` rather than by a blank browser tab.

#[cfg(target_arch = "wasm32")]
pub use web_time::{Instant, SystemTime, UNIX_EPOCH};

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::{Instant, SystemTime, UNIX_EPOCH};

// Two things are deliberately *not* routed through here, because neither can panic:
//
// - `std::time::Duration` — arithmetic on a count of nanoseconds, identical on every target.
// - `std::time::UNIX_EPOCH` where it is compared against a **filesystem** timestamp, which
//   `std::fs::metadata` hands back as a `std::time::SystemTime` whatever the target. Pairing
//   that with this module's `UNIX_EPOCH` would be a type error, not a fix.

#[cfg(test)]
mod tests {
    /// #1048: nothing outside this module may read the clock through `std::time`, because on
    /// wasm those calls panic at runtime and the wasm CI job only builds. `Duration` is fine
    /// anywhere — it holds no clock.
    #[test]
    fn no_raw_std_time_outside_this_module() {
        let mut offenders = Vec::new();
        let mut stack = vec![std::path::PathBuf::from("src")];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs")
                    || path.file_name().is_some_and(|n| n == "time.rs")
                {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                for (i, line) in text.lines().enumerate() {
                    let code = line.split("//").next().unwrap_or("");
                    for banned in ["std::time::Instant", "std::time::SystemTime"] {
                        if code.contains(banned) {
                            offenders.push(format!("{}:{}: {banned}", path.display(), i + 1));
                        }
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "read the clock through `crate::time` instead — `std::time` panics at runtime on \
             wasm32, and the wasm CI job only builds (#1048):\n  {}",
            offenders.join("\n  ")
        );
    }
}
