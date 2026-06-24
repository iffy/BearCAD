//! Animated GIF capture (SPEC §11.6).
//!
//! While recording is active the app requests a window screenshot on a fixed
//! cadence (~10 fps); the App crops each delivered frame to the 3D viewport (the
//! default) before handing it here, where it is downscaled to [`GIF_MAX_EDGE`] and
//! accumulated. Cropping and downscaling on capture keep the synchronous encode on
//! stop cheap. On stop the frames are encoded to `paramcad_<TIMESTAMP>.gif` using
//! the `gif` codec bundled with the `image` crate; the encoder's default palette
//! quantization yields medium-quality output.

use std::time::{Duration, Instant};

/// Target capture rate. 10 fps -> one frame every 100 ms.
pub const GIF_FPS: u32 = 10;
const FRAME_INTERVAL: Duration = Duration::from_millis(1000 / GIF_FPS as u64);

/// Longest-edge cap (px) for encoded GIF frames. Window framebuffers can be
/// several megapixels (e.g. retina 3024x1832); encoding those verbatim is slow
/// (it can block the UI thread on stop) and produces huge files. Downscaling to
/// this bound keeps "medium quality" output fast and reasonably sized (#5).
pub const GIF_MAX_EDGE: u32 = 720;

/// A single captured frame: tightly packed RGBA8 plus its dimensions.
#[derive(Clone)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// State machine for an in-progress GIF recording.
#[derive(Default)]
pub struct GifRecorder {
    active: bool,
    frames: Vec<CapturedFrame>,
    /// When the next frame should be captured. `None` means "capture now".
    next_capture: Option<Instant>,
    /// `true` captures the whole window; `false` (default) just the 3D viewport (#5).
    whole_window: bool,
}

impl GifRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_recording(&self) -> bool {
        self.active
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Whether this recording captures the whole window (vs. just the 3D viewport).
    pub fn whole_window(&self) -> bool {
        self.whole_window
    }

    /// Begin a new recording. No-op (returns `false`) if one is already running.
    /// `whole_window` selects full-window vs. 3D-viewport-only capture (#5).
    pub fn start(&mut self, whole_window: bool) -> bool {
        if self.active {
            return false;
        }
        self.active = true;
        self.whole_window = whole_window;
        self.frames.clear();
        self.next_capture = None;
        true
    }

    /// Whether a screenshot should be requested this frame, given `now`. Advances
    /// the internal schedule so callers get at most one request per interval.
    pub fn should_capture(&mut self, now: Instant) -> bool {
        if !self.active {
            return false;
        }
        match self.next_capture {
            None => {
                self.next_capture = Some(now + FRAME_INTERVAL);
                true
            }
            Some(due) if now >= due => {
                // Schedule from `due` (not `now`) so cadence doesn't drift slow.
                self.next_capture = Some(due + FRAME_INTERVAL);
                true
            }
            Some(_) => false,
        }
    }

    /// Time to wait before the next capture is due, given `now`. Used to schedule a
    /// just-in-time repaint instead of spinning the UI at full framerate while
    /// recording — that wasted-render loop was the source of in-use lag (#5).
    pub fn time_until_next_capture(&self, now: Instant) -> Duration {
        match self.next_capture {
            None => Duration::ZERO,
            Some(due) => due.saturating_duration_since(now),
        }
    }

    /// Store a delivered screenshot frame while recording. The frame is downscaled
    /// to [`GIF_MAX_EDGE`] immediately so the synchronous encode on stop stays cheap
    /// (encoding raw multi-megapixel frames was the source of save-time lag, #5).
    pub fn push_frame(&mut self, frame: CapturedFrame) {
        if self.active {
            self.frames.push(downscale_frame(frame));
        }
    }

    /// Stop recording and return the captured frames for encoding. Returns `None`
    /// if not recording or no frames were captured.
    pub fn stop(&mut self) -> Option<Vec<CapturedFrame>> {
        if !self.active {
            return None;
        }
        self.active = false;
        self.next_capture = None;
        let frames = std::mem::take(&mut self.frames);
        if frames.is_empty() {
            None
        } else {
            Some(frames)
        }
    }
}

/// Default output filename for a recording started at the current local time.
pub fn default_gif_filename() -> String {
    format!("paramcad_{}.gif", timestamp())
}

/// `YYYYmmdd_HHMMSS` timestamp from the system clock, used in the GIF filename.
fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Civil date from Unix seconds (UTC) — avoids pulling in a date dependency.
    let days = (secs / 86_400) as i64;
    let secs_of_day = (secs % 86_400) as u32;
    let (hour, minute, second) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}{month:02}{day:02}_{hour:02}{minute:02}{second:02}")
}

/// Convert days-since-Unix-epoch to a `(year, month, day)` civil date.
/// Algorithm from Howard Hinnant's `days_from_civil` inverse (public domain).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Scaled-down (w, h) so the longest edge is at most [`GIF_MAX_EDGE`], preserving
/// aspect ratio. Returns the original size if already within bounds.
pub fn scaled_dimensions(width: u32, height: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= GIF_MAX_EDGE || longest == 0 {
        return (width.max(1), height.max(1));
    }
    let scale = GIF_MAX_EDGE as f32 / longest as f32;
    let w = ((width as f32 * scale).round() as u32).max(1);
    let h = ((height as f32 * scale).round() as u32).max(1);
    (w, h)
}

/// Crop a frame to the given physical-pixel rectangle `(x, y, w, h)`, clamped to
/// the frame. Used to capture only the 3D viewport region by default (#5). An
/// empty or out-of-range rect returns the frame unchanged.
pub fn crop_frame(frame: CapturedFrame, x: u32, y: u32, w: u32, h: u32) -> CapturedFrame {
    let x1 = (x + w).min(frame.width);
    let y1 = (y + h).min(frame.height);
    if x >= frame.width || y >= frame.height || x1 <= x || y1 <= y {
        return frame;
    }
    let (cw, ch) = (x1 - x, y1 - y);
    let mut rgba = Vec::with_capacity((cw * ch * 4) as usize);
    for row in y..y1 {
        let start = ((row * frame.width + x) * 4) as usize;
        let end = ((row * frame.width + x1) * 4) as usize;
        rgba.extend_from_slice(&frame.rgba[start..end]);
    }
    CapturedFrame {
        width: cw,
        height: ch,
        rgba,
    }
}

/// Downscale a frame to [`GIF_MAX_EDGE`] long edge, preserving aspect ratio.
/// Returns the frame unchanged if already within bounds (the common case once
/// frames are cropped to the viewport), so the typical path does no work.
fn downscale_frame(frame: CapturedFrame) -> CapturedFrame {
    let (out_w, out_h) = scaled_dimensions(frame.width, frame.height);
    if out_w == frame.width && out_h == frame.height {
        return frame;
    }
    use image::RgbaImage;
    let Some(buffer) = RgbaImage::from_raw(frame.width, frame.height, frame.rgba) else {
        // Size mismatch: shouldn't happen, but never panic mid-recording.
        return CapturedFrame {
            width: out_w,
            height: out_h,
            rgba: vec![0; (out_w * out_h * 4) as usize],
        };
    };
    let scaled =
        image::imageops::resize(&buffer, out_w, out_h, image::imageops::FilterType::Triangle);
    CapturedFrame {
        width: out_w,
        height: out_h,
        rgba: scaled.into_raw(),
    }
}

/// Encode captured frames to an animated GIF at [`GIF_FPS`]. Frames are already
/// downscaled to [`GIF_MAX_EDGE`] on capture; here they are only normalized to the
/// first frame's size so the GIF has a consistent canvas if the window was resized
/// mid-capture.
pub fn encode_gif(path: &str, frames: &[CapturedFrame]) -> Result<(), String> {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame, RgbaImage};

    let Some(first) = frames.first() else {
        return Err("no frames captured".to_string());
    };
    let (out_w, out_h) = scaled_dimensions(first.width, first.height);

    let file =
        std::fs::File::create(path).map_err(|e| format!("failed to create {path}: {e}"))?;
    let writer = std::io::BufWriter::new(file);
    let mut encoder = GifEncoder::new(writer);
    encoder
        .set_repeat(Repeat::Infinite)
        .map_err(|e| format!("gif encode error: {e}"))?;
    let delay = Delay::from_numer_denom_ms(1000, GIF_FPS);
    for f in frames {
        let buffer = RgbaImage::from_raw(f.width, f.height, f.rgba.clone())
            .ok_or_else(|| "frame buffer size mismatch".to_string())?;
        // Normalize every frame to the GIF canvas size.
        let buffer = if buffer.width() == out_w && buffer.height() == out_h {
            buffer
        } else {
            image::imageops::resize(
                &buffer,
                out_w,
                out_h,
                image::imageops::FilterType::Triangle,
            )
        };
        encoder
            .encode_frame(Frame::from_parts(buffer, 0, 0, delay))
            .map_err(|e| format!("gif encode error: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_frame(w: u32, h: u32, rgb: [u8; 3]) -> CapturedFrame {
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
        CapturedFrame {
            width: w,
            height: h,
            rgba,
        }
    }

    #[test]
    fn start_clears_and_activates() {
        let mut r = GifRecorder::new();
        assert!(!r.is_recording());
        assert!(r.start(false));
        assert!(r.is_recording());
        // Starting again while active is a no-op.
        assert!(!r.start(false));
    }

    #[test]
    fn first_capture_is_immediate_then_throttled() {
        let mut r = GifRecorder::new();
        r.start(false);
        let t0 = Instant::now();
        assert!(r.should_capture(t0), "first frame should capture immediately");
        // A request just after t0 (well under the interval) must be throttled.
        assert!(!r.should_capture(t0 + Duration::from_millis(10)));
        // After a full interval, capture again.
        assert!(r.should_capture(t0 + FRAME_INTERVAL + Duration::from_millis(1)));
    }

    #[test]
    fn not_recording_never_captures() {
        let mut r = GifRecorder::new();
        assert!(!r.should_capture(Instant::now()));
    }

    #[test]
    fn stop_returns_frames_and_deactivates() {
        let mut r = GifRecorder::new();
        r.start(false);
        r.push_frame(solid_frame(2, 2, [255, 0, 0]));
        r.push_frame(solid_frame(2, 2, [0, 255, 0]));
        assert_eq!(r.frame_count(), 2);
        let frames = r.stop().expect("frames returned");
        assert_eq!(frames.len(), 2);
        assert!(!r.is_recording());
        // Stopping again yields nothing.
        assert!(r.stop().is_none());
    }

    #[test]
    fn stop_with_no_frames_is_none() {
        let mut r = GifRecorder::new();
        r.start(false);
        assert!(r.stop().is_none());
    }

    #[test]
    fn push_frame_ignored_when_inactive() {
        let mut r = GifRecorder::new();
        r.push_frame(solid_frame(2, 2, [1, 2, 3]));
        assert_eq!(r.frame_count(), 0);
    }

    #[test]
    fn default_filename_shape() {
        let name = default_gif_filename();
        assert!(name.starts_with("paramcad_"));
        assert!(name.ends_with(".gif"));
        // paramcad_YYYYmmdd_HHMMSS.gif
        assert_eq!(name.len(), "paramcad_".len() + 15 + ".gif".len());
    }

    #[test]
    fn civil_from_days_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2000-01-01 is 10957 days after the epoch.
        assert_eq!(civil_from_days(10957), (2000, 1, 1));
    }

    #[test]
    fn encode_gif_writes_a_file() {
        let frames = vec![
            solid_frame(4, 4, [200, 30, 30]),
            solid_frame(4, 4, [30, 200, 30]),
        ];
        let path = std::env::temp_dir().join("le3_gif_recorder_test.gif");
        let path_str = path.to_str().unwrap();
        encode_gif(path_str, &frames).expect("encode succeeds");
        let bytes = std::fs::read(path_str).expect("file exists");
        // GIF magic header.
        assert_eq!(&bytes[0..3], b"GIF");
        let _ = std::fs::remove_file(path_str);
    }

    #[test]
    fn encode_gif_rejects_empty() {
        assert!(encode_gif("/tmp/unused.gif", &[]).is_err());
    }

    #[test]
    fn scaled_dimensions_caps_long_edge() {
        // Retina-ish framebuffer scales down with aspect preserved.
        let (w, h) = scaled_dimensions(3024, 1832);
        assert_eq!(w.max(h), GIF_MAX_EDGE);
        assert_eq!(w, 720);
        assert_eq!(h, 436); // 1832 * 720/3024, rounded
        // Already-small frames are untouched.
        assert_eq!(scaled_dimensions(640, 480), (640, 480));
        // Degenerate sizes never produce a zero dimension.
        assert_eq!(scaled_dimensions(0, 0), (1, 1));
    }

    #[test]
    fn encode_gif_downscales_large_frames() {
        let big = solid_frame(2000, 1000, [10, 20, 30]);
        let path = std::env::temp_dir().join("le3_gif_downscale_test.gif");
        let path_str = path.to_str().unwrap();
        encode_gif(path_str, std::slice::from_ref(&big)).expect("encode succeeds");
        let bytes = std::fs::read(path_str).expect("file exists");
        assert_eq!(&bytes[0..3], b"GIF");
        // Logical screen width is bytes 6-7 (little-endian) of the GIF header.
        let width = u16::from_le_bytes([bytes[6], bytes[7]]);
        assert_eq!(width, GIF_MAX_EDGE as u16);
        let _ = std::fs::remove_file(path_str);
    }

    #[test]
    fn start_records_whole_window_flag() {
        let mut r = GifRecorder::new();
        r.start(true);
        assert!(r.whole_window());
        r.stop();
        r.start(false);
        assert!(!r.whole_window());
    }

    #[test]
    fn time_until_next_capture_counts_down() {
        let mut r = GifRecorder::new();
        r.start(false);
        let t0 = Instant::now();
        // Before the first capture, a frame is due immediately.
        assert_eq!(r.time_until_next_capture(t0), Duration::ZERO);
        // After capturing, the next is one interval out, then counts down.
        assert!(r.should_capture(t0));
        let remaining = r.time_until_next_capture(t0);
        assert!(remaining <= FRAME_INTERVAL && remaining > Duration::ZERO);
        // Once the interval has elapsed, it saturates to zero (never negative).
        assert_eq!(
            r.time_until_next_capture(t0 + FRAME_INTERVAL + Duration::from_millis(5)),
            Duration::ZERO
        );
    }

    #[test]
    fn push_frame_downscales_large_frames() {
        let mut r = GifRecorder::new();
        r.start(false);
        r.push_frame(solid_frame(2000, 1000, [1, 2, 3]));
        let frames = r.stop().expect("frame stored");
        // Stored frame is already capped to GIF_MAX_EDGE so the encode is cheap.
        assert_eq!(frames[0].width.max(frames[0].height), GIF_MAX_EDGE);
        // Buffer length stays consistent with the reported dimensions.
        assert_eq!(
            frames[0].rgba.len(),
            (frames[0].width * frames[0].height * 4) as usize
        );
    }

    #[test]
    fn crop_frame_extracts_subregion() {
        // 4x4 frame; crop the bottom-right 2x2.
        let mut f = solid_frame(4, 4, [0, 0, 0]);
        // Mark pixel (3,3) red so we can confirm it survives the crop.
        let idx = ((3 * 4 + 3) * 4) as usize;
        f.rgba[idx] = 255;
        let cropped = crop_frame(f, 2, 2, 2, 2);
        assert_eq!((cropped.width, cropped.height), (2, 2));
        assert_eq!(cropped.rgba.len(), 2 * 2 * 4);
        // Bottom-right pixel of the cropped region is the red one.
        assert_eq!(cropped.rgba[(3 * 4) as usize], 255);
    }

    #[test]
    fn crop_frame_clamps_and_passes_through_bad_rects() {
        // Rect overruns the frame: clamps to available pixels.
        let f = solid_frame(4, 4, [9, 9, 9]);
        let cropped = crop_frame(f, 2, 2, 100, 100);
        assert_eq!((cropped.width, cropped.height), (2, 2));
        // Out-of-range origin returns the frame unchanged (no panic).
        let f = solid_frame(4, 4, [9, 9, 9]);
        let same = crop_frame(f, 10, 10, 2, 2);
        assert_eq!((same.width, same.height), (4, 4));
    }
}
