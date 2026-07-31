//! Browse the McMaster-Carr catalog in a window and import a part straight into the
//! document (#1022).
//!
//! McMaster's own site is the catalog: it has the search, the drawings, the sizes and the
//! CAD downloads, and it is what anyone specifying a screw already uses. So this shows that
//! site — a real webview, in a window — and catches the CAD download on its way out. Pick
//! the part the way you always do, choose STEP, and the body lands in the document instead
//! of in your Downloads folder.
//!
//! Their Product Information API would be the tidier route, but it is gated behind a signed
//! agreement and a client certificate McMaster issues per account, so it works for almost
//! nobody. Scraping the site isn't an option either — against their terms, and bot-blocked.
//! Showing the site and catching what the user themselves downloaded needs no account, no
//! credentials, and asks nothing of McMaster that a browser doesn't.
//!
//! The webview is a native child of the app's own window ([`wry`]: WKWebView on macOS,
//! WebView2 on Windows, WebKitGTK on Linux), positioned into an egui window's rect each
//! frame. Native views composite above the wgpu canvas, so the egui window is the frame and
//! the webview is what fills it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Where the window starts: McMaster's front page, which is their search.
pub const CATALOG_URL: &str = "https://www.mcmaster.com/";

/// The product page for a part number — what the window opens at when a part number was
/// typed, so a known number skips the search.
pub fn part_url(part_number: &str) -> String {
    let part = normalize_part_number(part_number);
    if part.is_empty() {
        CATALOG_URL.to_string()
    } else {
        format!("https://www.mcmaster.com/{part}/")
    }
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
    /// The URL it came from — the only clue to the part number when the filename has none.
    pub url: String,
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

/// Whether a string reads as a McMaster part number: their catalog numbers are five or
/// more characters of digits and letters, and always carry a digit.
fn looks_like_part_number(s: &str) -> bool {
    s.len() >= 5
        && s.chars().all(|c| c.is_ascii_alphanumeric())
        && s.chars().any(|c| c.is_ascii_digit())
}

/// The part number a URL carries, wherever along it that sits — a CAD link is
/// `…/91290A115/cad`, not `…/cad/91290A115`, so the last segment is the wrong thing to
/// take.
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

/// A filename for a download, from its URL — what the started-handler names the file when
/// the platform hands us a bare URL.
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

/// Everything the webview's callbacks post back to the frame loop. The handlers run on the
/// platform's own threads, so this is the seam between them and the document.
#[derive(Debug, Default)]
pub struct CatalogInbox {
    /// Downloads that finished and are waiting to be imported.
    pub caught: Vec<CaughtDownload>,
    /// A download started, so the window can say it's working.
    pub in_flight: usize,
    /// Links that led off McMaster's site, to hand to the real browser.
    pub external: Vec<String>,
    /// The last thing worth telling the user.
    pub message: String,
}

pub type SharedInbox = Arc<Mutex<CatalogInbox>>;

/// The catalog window: the webview plus the scratch directory its downloads land in.
///
/// Native-only, and only where a webview is actually available. Dropping it takes the
/// webview down with it, which is how closing the window works.
#[cfg(not(target_arch = "wasm32"))]
pub struct CatalogWindow {
    webview: wry::WebView,
    inbox: SharedInbox,
    /// Where caught downloads are written: our own directory, so a catch never lands in the
    /// user's Downloads folder and never collides with a file already there.
    #[allow(dead_code)]
    download_dir: PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl CatalogWindow {
    /// Build the webview as a child of the app's own window, showing `url`.
    pub fn open(
        parent: &impl raw_window_handle::HasWindowHandle,
        url: &str,
        bounds: wry::Rect,
        repaint: egui::Context,
    ) -> Result<Self, String> {
        let inbox: SharedInbox = Arc::default();
        let download_dir = std::env::temp_dir().join("bearcad-mcmaster");
        std::fs::create_dir_all(&download_dir).map_err(|e| e.to_string())?;

        let started = {
            let (inbox, dir, ctx) = (inbox.clone(), download_dir.clone(), repaint.clone());
            move |url: String, path: &mut PathBuf| {
                *path = dir.join(download_file_name(&url));
                if let Ok(mut inbox) = inbox.lock() {
                    inbox.in_flight += 1;
                    inbox.message = "Downloading…".to_string();
                }
                ctx.request_repaint();
                true
            }
        };
        let completed = {
            let (inbox, ctx) = (inbox.clone(), repaint.clone());
            move |url: String, path: Option<PathBuf>, success: bool| {
                if let Ok(mut inbox) = inbox.lock() {
                    inbox.in_flight = inbox.in_flight.saturating_sub(1);
                    match (success, path) {
                        (true, Some(path)) => inbox.caught.push(CaughtDownload { path, url }),
                        _ => inbox.message = "That download didn't finish".to_string(),
                    }
                }
                ctx.request_repaint();
            }
        };
        let navigation = {
            let (inbox, ctx) = (inbox.clone(), repaint.clone());
            move |url: String| {
                if is_mcmaster_url(&url) {
                    return true;
                }
                // This window is McMaster's catalog; anything else is the browser's job.
                if let Ok(mut inbox) = inbox.lock() {
                    inbox.external.push(url);
                }
                ctx.request_repaint();
                false
            }
        };

        let webview = wry::WebViewBuilder::new()
            .with_url(url)
            .with_bounds(bounds)
            .with_download_started_handler(started)
            .with_download_completed_handler(completed)
            .with_navigation_handler(navigation)
            .build_as_child(parent)
            .map_err(|e| format!("could not open a web view: {e}"))?;
        Ok(Self { webview, inbox, download_dir })
    }

    /// Keep the webview sitting in the egui window's rect as it moves and resizes.
    pub fn set_bounds(&self, bounds: wry::Rect) {
        let _ = self.webview.set_bounds(bounds);
    }

    pub fn load(&self, url: &str) {
        let _ = self.webview.load_url(url);
    }

    /// Take whatever the webview's handlers have posted since the last frame.
    pub fn drain(&self) -> CatalogInbox {
        let Ok(mut inbox) = self.inbox.lock() else {
            return CatalogInbox::default();
        };
        CatalogInbox {
            caught: std::mem::take(&mut inbox.caught),
            in_flight: inbox.in_flight,
            external: std::mem::take(&mut inbox.external),
            message: inbox.message.clone(),
        }
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

    /// #1022: a typed part number opens straight at its product page; nothing typed opens
    /// the catalog's own front page, which is their search.
    #[test]
    fn a_typed_part_number_opens_its_product_page() {
        assert_eq!(part_url("91290A115"), "https://www.mcmaster.com/91290A115/");
        assert_eq!(part_url("  91290-a115"), "https://www.mcmaster.com/91290A115/");
        assert_eq!(part_url(""), CATALOG_URL);
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

    /// #1022: a caught file is imported by what it is — the two CAD formats the app reads
    /// — and anything else is refused rather than guessed at.
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
            body_name_for(Path::new("/tmp/cad.step"), "https://www.mcmaster.com/91290A115"),
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

    /// #1022: a download lands under a name of our choosing, in our own directory — never
    /// in the user's Downloads folder, and never without an extension to import it by.
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
        // Path separators can't escape the directory we chose.
        assert!(!download_file_name("https://x/../../etc/passwd").contains('/'));
    }
}
