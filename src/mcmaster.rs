//! Browse the McMaster-Carr catalog in a window and import a part straight into the
//! document (#1022).
//!
//! McMaster's own site is the catalog: it has the search, the drawings, the sizes and the
//! CAD downloads, and it is what anyone specifying a screw already uses. So this shows that
//! site — a real web view, in a window of its own — and catches the CAD download on its way
//! out. Pick the part the way you always do, choose STEP, and the body lands in the document
//! instead of in your Downloads folder.
//!
//! Their Product Information API would be the tidier route, but it is gated behind a signed
//! agreement and a client certificate McMaster issues per account, so it works for almost
//! nobody. Scraping the site isn't an option either — against their terms, and bot-blocked.
//! Showing the site and catching what the user themselves downloaded needs no account, no
//! credentials, and asks nothing of McMaster that a browser doesn't.
//!
//! # Why a second process
//!
//! A web view needs an event loop, and so does the app. Hosting one *inside* the app's own
//! window (wry's `build_as_child`) works on macOS and Windows but leaves the native view
//! floating over every egui window regardless of z-order, and on Linux it panics outright:
//! wry's WebKitGTK backend requires `gtk::init` on the calling thread and a GTK loop pumped
//! alongside it — which eframe/winit never does — and supports X11 only.
//!
//! So the window is its own process: **`bearcad mcmaster [part]`**, this same executable
//! under a subcommand ([`run_catalog_process`]). It owns a `tao` event loop, so it is a real
//! OS window with real z-order, its own taskbar entry, and — because `tao` initializes GTK
//! itself — Linux support for free. It reports what it caught on **stdout**, one line per
//! file ([`CaughtDownload::to_line`]), and the app reads those lines and imports them. No
//! packaging cost either: the binary already ships.

use std::path::{Path, PathBuf};

/// Where the window starts: McMaster's front page, which is their search.
pub const CATALOG_URL: &str = "https://www.mcmaster.com/";

/// The subcommand that runs the catalog window.
pub const SUBCOMMAND: &str = "mcmaster";

/// How the catalog process reports a caught file: `part<TAB>path<TAB>url`. A prefix rather
/// than a bare path, so the app can tell a report from anything else the platform's web view
/// decides to print on stdout — which it does.
pub const CAUGHT_PREFIX: &str = "part";

/// Where a text search lands on their site.
pub const SEARCH_URL: &str = "https://www.mcmaster.com/products/?q=";

/// The product page for a part number — what the window opens at when one was given, so a
/// known number skips the search.
pub fn part_url(part_number: &str) -> String {
    let part = normalize_part_number(part_number);
    if part.is_empty() {
        CATALOG_URL.to_string()
    } else {
        format!("https://www.mcmaster.com/{part}/")
    }
}

/// Where the window should open for whatever the user typed (#1022): their **search results**
/// for a phrase, the **product page** for something already shaped like a part number, and
/// the catalog's front page for nothing at all.
///
/// One function because the box takes one thing — what you're after — and a part number is
/// just a very specific way of saying it. Typing `91290A115` and typing `socket head screw`
/// both mean "show me this".
pub fn catalog_url_for(query: &str) -> String {
    let query = query.trim();
    if query.is_empty() {
        return CATALOG_URL.to_string();
    }
    // A pasted product link, or a bare part number, goes straight to the part.
    let compact = normalize_part_number(query);
    if !query.contains(char::is_whitespace) && looks_like_part_number(&compact) {
        return part_url(&compact);
    }
    format!("{SEARCH_URL}{}", url_query_encode(query))
}

/// Percent-encode a search phrase for a query string. Spaces become `+`, every byte outside
/// the unreserved set is escaped — so a search for `1/4"-20` reaches them intact.
fn url_query_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        match byte {
            b' ' => out.push('+'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Normalize a typed part number: McMaster prints them with spaces and dashes their URLs
/// don't want, and people paste whole product links.
pub fn normalize_part_number(input: &str) -> String {
    let input = input.trim();
    // A pasted product URL: the part number is its last meaningful path segment.
    let tail = input
        .rsplit(['/', '=', '?'])
        .find(|s| !s.is_empty())
        .unwrap_or(input);
    tail.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
}

/// Whether a URL belongs to McMaster's own site. The window is McMaster's catalog and says
/// so, so a link that leads off it opens in the user's real browser instead of quietly
/// turning this window into a general-purpose one.
pub fn is_mcmaster_url(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        // In-page schemes (about:blank, blob:, data:) are the page's own business.
        return true;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    host.eq_ignore_ascii_case("mcmaster.com") || host.to_ascii_lowercase().ends_with(".mcmaster.com")
}

/// A CAD file the catalog window caught on its way to disk.
#[derive(Clone, Debug, PartialEq)]
pub struct CaughtDownload {
    pub path: PathBuf,
    /// The URL it came from — the clue to the part number when the filename hasn't got one.
    pub url: String,
}

impl CaughtDownload {
    /// The stdout line the catalog process reports this on.
    pub fn to_line(&self) -> String {
        format!(
            "{CAUGHT_PREFIX}\t{}\t{}",
            self.path.to_string_lossy(),
            self.url
        )
    }

    /// Parse one line of the catalog process's stdout. `None` for anything that isn't a
    /// report — the platform's web view prints plenty of its own noise on that stream.
    pub fn from_line(line: &str) -> Option<Self> {
        let mut parts = line.trim_end().split('\t');
        if parts.next()? != CAUGHT_PREFIX {
            return None;
        }
        let path = parts.next().filter(|p| !p.is_empty())?;
        Some(Self {
            path: PathBuf::from(path),
            url: parts.next().unwrap_or_default().to_string(),
        })
    }
}

/// What the import should do with a caught file, by its extension. McMaster offers a part's
/// model in several formats; these are the two the app reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CadFormat {
    Step,
    Stl,
}

impl CadFormat {
    pub fn of(path: &Path) -> Option<CadFormat> {
        let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
        match ext.as_str() {
            "step" | "stp" => Some(CadFormat::Step),
            "stl" => Some(CadFormat::Stl),
            _ => None,
        }
    }
}

/// Whether a string reads as a McMaster part number: their catalog numbers are five or more
/// characters of digits and letters, and always carry a digit.
fn looks_like_part_number(s: &str) -> bool {
    s.len() >= 5
        && s.chars().all(|c| c.is_ascii_alphanumeric())
        && s.chars().any(|c| c.is_ascii_digit())
}

/// The part number a URL carries, wherever along it that sits — a CAD link is
/// `…/91290A115/cad`, not `…/cad/91290A115`, so the last segment is the wrong thing to take.
pub fn part_number_in(url: &str) -> Option<String> {
    url.split(['/', '?', '&', '=', '#'])
        .map(|s| s.trim().to_ascii_uppercase())
        .find(|s| looks_like_part_number(s))
}

/// The body name a caught download should land under: the part number where the filename
/// carries one, else the one in the URL it came from, else the filename itself.
pub fn body_name_for(path: &Path, url: &str) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    // McMaster names CAD downloads after the part, so the stem usually *is* the number.
    let from_stem = normalize_part_number(&stem);
    if looks_like_part_number(&from_stem) {
        return from_stem;
    }
    if let Some(from_url) = part_number_in(url) {
        return from_url;
    }
    if stem.is_empty() { "McMaster-Carr part".to_string() } else { stem }
}

/// A filename for a download, from its URL — what names the file when the platform hands the
/// catalog process a bare URL. Sanitized: the URL never gets to choose a path.
pub fn download_file_name(url: &str) -> String {
    let tail = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("");
    let cleaned: String = tail
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
        .collect();
    if cleaned.is_empty() || !cleaned.contains('.') {
        "mcmaster-part.step".to_string()
    } else {
        cleaned
    }
}

/// Where caught downloads are written: the app's own scratch directory, so a catch never
/// lands in the user's Downloads folder and never collides with a file already there.
pub fn download_dir() -> PathBuf {
    std::env::temp_dir().join("bearcad-mcmaster")
}

// ---------------------------------------------------------------------------------------
// The catalog process — `bearcad mcmaster [part]`
// ---------------------------------------------------------------------------------------

/// Run the catalog window. This **is** the process: it owns the event loop and returns when
/// the window closes, so it runs before any of the app's own startup.
///
/// Reports each caught file on stdout as [`CaughtDownload::to_line`]; the app reads them.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_catalog_process(query: Option<&str>) -> Result<(), String> {
    use std::io::Write as _;
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::window::WindowBuilder;

    let dir = download_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("scratch directory: {e}"))?;

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("McMaster-Carr — BearCAD")
        .with_inner_size(tao::dpi::LogicalSize::new(1100.0, 800.0))
        .build(&event_loop)
        .map_err(|e| format!("could not open a window: {e}"))?;

    // A file is reported the moment it lands, so a part imports while the window stays open
    // for the next one.
    let report = move |path: &Path, url: &str| {
        let line = CaughtDownload { path: path.to_path_buf(), url: url.to_string() }.to_line();
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    };

    let started_dir = dir.clone();
    let webview = wry::WebViewBuilder::new()
        .with_url(catalog_url_for(query.unwrap_or_default()))
        .with_download_started_handler(move |url, path| {
            *path = started_dir.join(download_file_name(&url));
            true
        })
        .with_download_completed_handler(move |url, path, success| {
            if let (true, Some(path)) = (success, path) {
                report(&path, &url);
            }
        })
        .with_navigation_handler(|url| {
            if is_mcmaster_url(&url) {
                return true;
            }
            // This window is McMaster's catalog; anything else is the browser's job.
            let _ = crate::open_in_browser(&url);
            false
        })
        .build(&window)
        .map_err(|e| format!("could not open a web view: {e}"))?;

    event_loop.run(move |event, _, control_flow| {
        // The web view has to outlive the loop, and moving it in is what does that.
        let _ = &webview;
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
            *control_flow = ControlFlow::Exit;
        }
    });
}

// ---------------------------------------------------------------------------------------
// The app's side — run the catalog process and collect what it catches
// ---------------------------------------------------------------------------------------

/// A running catalog window, and what it has caught so far.
#[cfg(not(target_arch = "wasm32"))]
pub struct CatalogSession {
    child: std::process::Child,
    caught: std::sync::Arc<std::sync::Mutex<Vec<CaughtDownload>>>,
    /// Set by the reader thread when the process's stdout ends — i.e. the window closed.
    finished: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(not(target_arch = "wasm32"))]
impl CatalogSession {
    /// Start the catalog window as a child of this process, showing `query` — a search
    /// phrase, a part number, or nothing for the catalog's front page.
    ///
    /// The executable is our own ([`std::env::current_exe`]) under the `mcmaster`
    /// subcommand, so there is no second binary to build, sign or package.
    pub fn open(query: Option<&str>, repaint: egui::Context) -> Result<Self, String> {
        use std::io::{BufRead as _, BufReader};
        let exe = std::env::current_exe().map_err(|e| format!("cannot find myself: {e}"))?;
        let mut command = std::process::Command::new(exe);
        command.arg(SUBCOMMAND);
        // The query goes through verbatim — a search phrase has to survive as one argument,
        // and it is `catalog_url_for` on the other side that decides what it means.
        if let Some(query) = query.map(str::trim).filter(|q| !q.is_empty()) {
            command.arg(query);
        }
        let mut child = command
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start the catalog window: {e}"))?;
        let stdout = child.stdout.take().ok_or("the catalog window has no stdout")?;
        let caught: std::sync::Arc<std::sync::Mutex<Vec<CaughtDownload>>> = Default::default();
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let (caught, finished) = (caught.clone(), finished.clone());
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if let Some(download) = CaughtDownload::from_line(&line) {
                        if let Ok(mut caught) = caught.lock() {
                            caught.push(download);
                        }
                        repaint.request_repaint();
                    }
                }
                // Stdout closed: the window is gone.
                finished.store(true, std::sync::atomic::Ordering::Relaxed);
                repaint.request_repaint();
            });
        }
        Ok(Self { child, caught, finished })
    }

    /// Everything caught since the last call.
    pub fn take_caught(&self) -> Vec<CaughtDownload> {
        self.caught
            .lock()
            .map(|mut c| std::mem::take(&mut *c))
            .unwrap_or_default()
    }

    /// Whether the window has closed.
    pub fn finished(&self) -> bool {
        self.finished.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Close the window, if it is still open.
    pub fn close(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Closing the app takes the catalog window with it — an orphaned window with no document to
/// import into is just a stray browser.
#[cfg(not(target_arch = "wasm32"))]
impl Drop for CatalogSession {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #1022: a part number survives however it was typed — McMaster prints them with
    /// spaces and dashes, and people paste whole product links.
    #[test]
    fn part_numbers_normalize_however_they_were_typed() {
        assert_eq!(normalize_part_number("91290A115"), "91290A115");
        assert_eq!(normalize_part_number("  91290a115 "), "91290A115");
        assert_eq!(normalize_part_number("91290-A115"), "91290A115");
        assert_eq!(
            normalize_part_number("https://www.mcmaster.com/91290A115/"),
            "91290A115"
        );
        assert_eq!(normalize_part_number(""), "");
    }

    /// #1022: a part number opens straight at its product page; nothing given opens the
    /// catalog's own front page, which is their search.
    #[test]
    fn a_part_number_opens_its_product_page() {
        assert_eq!(part_url("91290A115"), "https://www.mcmaster.com/91290A115/");
        assert_eq!(part_url("  91290-a115"), "https://www.mcmaster.com/91290A115/");
        assert_eq!(part_url(""), CATALOG_URL);
    }

    /// #1022: what you type decides where the window opens — their search results for a
    /// phrase, the product page for something already shaped like a part number, and their
    /// front page for nothing at all. One box, because a part number is just a very specific
    /// way of saying what you're after.
    #[test]
    fn a_query_opens_the_search_and_a_part_number_the_part() {
        assert_eq!(
            catalog_url_for("socket head screw"),
            "https://www.mcmaster.com/products/?q=socket+head+screw"
        );
        assert_eq!(catalog_url_for("91290A115"), "https://www.mcmaster.com/91290A115/");
        assert_eq!(
            catalog_url_for("https://www.mcmaster.com/91290A115/"),
            "https://www.mcmaster.com/91290A115/"
        );
        assert_eq!(catalog_url_for("   "), CATALOG_URL);
        assert_eq!(catalog_url_for(""), CATALOG_URL);
        // A single word that isn't a part number is still a search, not a bogus part page.
        assert_eq!(
            catalog_url_for("bearings"),
            "https://www.mcmaster.com/products/?q=bearings"
        );
        // A phrase that happens to contain a part number searches for the phrase.
        assert!(catalog_url_for("91290A115 washer").starts_with(SEARCH_URL));
    }

    /// #1022: a search phrase reaches them intact — the sizes people actually type are full
    /// of characters a query string can't carry raw.
    #[test]
    fn a_search_phrase_is_encoded_for_the_url() {
        assert_eq!(url_query_encode("socket head"), "socket+head");
        // The thread callout for a quarter-inch screw: slash, quote and hash all escape.
        assert_eq!(url_query_encode("1/4\"-20"), "1%2F4%22-20");
        assert_eq!(url_query_encode("m3×0.5"), "m3%C3%970.5");
        assert_eq!(url_query_encode("a&b=c"), "a%26b%3Dc");
        // Unreserved characters are left alone rather than needlessly escaped.
        assert_eq!(url_query_encode("a-b_c.d~e9"), "a-b_c.d~e9");
    }

    /// #1022: the window is McMaster's catalog and stays that way — a link that leads off
    /// their site goes to the real browser instead of turning this into a general one.
    #[test]
    fn only_mcmaster_pages_stay_in_the_window() {
        assert!(is_mcmaster_url("https://www.mcmaster.com/91290A115/"));
        assert!(is_mcmaster_url("https://mcmaster.com/"));
        assert!(is_mcmaster_url("https://images.mcmaster.com/thing.png"));
        assert!(!is_mcmaster_url("https://example.com/"));
        // A lookalike host must not pass for theirs.
        assert!(!is_mcmaster_url("https://mcmaster.com.example.com/"));
        assert!(!is_mcmaster_url("https://notmcmaster.com/"));
        // In-page schemes are the page's own business.
        assert!(is_mcmaster_url("about:blank"));
        assert!(is_mcmaster_url("blob:https://www.mcmaster.com/abc"));
    }

    /// #1022: the two processes agree on the wire — a caught file round-trips through its
    /// stdout line, and the platform's own chatter on that stream is ignored rather than
    /// mistaken for a report.
    #[test]
    fn a_caught_file_round_trips_through_stdout() {
        let caught = CaughtDownload {
            path: PathBuf::from("/tmp/bearcad-mcmaster/91290A115.STEP"),
            url: "https://www.mcmaster.com/91290A115/cad".to_string(),
        };
        assert_eq!(CaughtDownload::from_line(&caught.to_line()), Some(caught.clone()));
        // A trailing newline is what the reader actually hands us.
        assert_eq!(
            CaughtDownload::from_line(&format!("{}\n", caught.to_line())),
            Some(caught)
        );
        // Web-view noise on the same stream is not a report.
        assert_eq!(CaughtDownload::from_line(""), None);
        assert_eq!(CaughtDownload::from_line("[WebKit] some warning"), None);
        assert_eq!(CaughtDownload::from_line("part"), None);
        assert_eq!(CaughtDownload::from_line("part\t"), None);
        // A report with no URL still names its file.
        assert_eq!(
            CaughtDownload::from_line("part\t/tmp/x.step"),
            Some(CaughtDownload { path: PathBuf::from("/tmp/x.step"), url: String::new() })
        );
    }

    /// #1022: a caught file is imported by what it is — the two CAD formats the app reads —
    /// and anything else is refused rather than guessed at.
    #[test]
    fn only_readable_cad_formats_are_imported() {
        assert_eq!(CadFormat::of(Path::new("/tmp/91290A115.STEP")), Some(CadFormat::Step));
        assert_eq!(CadFormat::of(Path::new("/tmp/91290A115.stp")), Some(CadFormat::Step));
        assert_eq!(CadFormat::of(Path::new("/tmp/91290A115.stl")), Some(CadFormat::Stl));
        assert_eq!(CadFormat::of(Path::new("/tmp/91290A115.dxf")), None);
        assert_eq!(CadFormat::of(Path::new("/tmp/91290A115.zip")), None);
        assert_eq!(CadFormat::of(Path::new("/tmp/noextension")), None);
    }

    /// #1022: an imported part names itself by its part number, from the filename where
    /// McMaster put one there and from the URL when it didn't.
    #[test]
    fn a_caught_part_names_itself_by_its_number() {
        assert_eq!(
            body_name_for(Path::new("/tmp/91290A115.STEP"), "https://www.mcmaster.com/x"),
            "91290A115"
        );
        // A generic filename falls back to the part number in the URL it came from — and
        // the number isn't the URL's last segment, so it has to be looked for.
        assert_eq!(
            body_name_for(
                Path::new("/tmp/download.step"),
                "https://www.mcmaster.com/91290A115/cad"
            ),
            "91290A115"
        );
        assert_eq!(
            part_number_in("https://www.mcmaster.com/cad/download?part=91290A115&fmt=step")
                .as_deref(),
            Some("91290A115")
        );
        assert_eq!(part_number_in("https://www.mcmaster.com/cad/"), None);
        // Nothing to go on at all still names the body something.
        assert_eq!(body_name_for(Path::new(""), "https://example.com"), "McMaster-Carr part");
    }

    /// #1022: a download lands under a name of our choosing, in our own directory — never in
    /// the user's Downloads folder, and never without an extension to import it by.
    #[test]
    fn downloads_get_a_usable_file_name() {
        assert_eq!(
            download_file_name("https://www.mcmaster.com/cad/91290A115.STEP"),
            "91290A115.STEP"
        );
        assert_eq!(
            download_file_name("https://www.mcmaster.com/cad/91290A115.STEP?token=abc"),
            "91290A115.STEP"
        );
        // A URL with nothing usable at the end still gets a name the importer can read.
        assert_eq!(download_file_name("https://www.mcmaster.com/download"), "mcmaster-part.step");
        assert_eq!(download_file_name(""), "mcmaster-part.step");
        // A URL can't choose a path: no separator survives, so a catch can't escape the
        // scratch directory.
        let escaped = download_file_name("https://x/../../etc/passwd");
        assert!(!escaped.contains('/') && !escaped.contains('\\'), "got {escaped}");
    }
}
