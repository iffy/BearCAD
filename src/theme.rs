//! Application-wide dark theme, tuned to match the viewport drawing area.

use eframe::egui::{self, style::WidgetVisuals, Color32, CornerRadius, Stroke, Theme, ThemePreference};

/// Viewport background (`main::col::BG`).
pub const VIEWPORT_BG: Color32 = Color32::from_gray(28);

/// Command palette console background.
const PALETTE_CONSOLE_BG: Color32 = Color32::from_rgb(22, 24, 30);

/// Slightly lifted surface for chrome panels (toolbar, hierarchy, status).
const PANEL_BG: Color32 = Color32::from_gray(32);

/// Interactive widget fill.
const WIDGET_BG: Color32 = Color32::from_gray(40);
const WIDGET_HOVER: Color32 = Color32::from_gray(48);
const WIDGET_ACTIVE: Color32 = Color32::from_gray(56);

/// Subtle separators and widget outlines.
const BORDER: Color32 = Color32::from_gray(55);

/// Selected toolbar tools: faint blue tint behind bright label text.
const SELECTION_BG: Color32 = Color32::from_rgba_premultiplied(55, 95, 170, 48);
const SELECTION_TEXT: Color32 = Color32::WHITE;

/// Accent for a focused input's ring and, by default, an element picker's selected-element
/// highlight (#213). A picker may override the selected-highlight color (e.g. Slice cutters red).
pub const FOCUS_ACCENT: Color32 = Color32::from_rgb(120, 170, 240);
/// Selected-highlight color for a picker whose elements are consumed destructively (cut away),
/// e.g. the Slice cutters or the Revolve cut bodies (#213) — the red override that a picker can
/// set in place of the default [`FOCUS_ACCENT`].
pub const CUT_ACCENT: Color32 = Color32::from_rgb(220, 90, 90);
/// Marks the one value in a set of interlinked inputs that the app computes rather than the
/// user setting it — the green lock in the Repeat tool's Count/Offset/Distance trio (#642).
/// The two locks that aren't green use [`UNLOCKED_GRAY`].
pub const LOCKED_ACCENT: Color32 = Color32::from_rgb(90, 220, 130);
/// The resting tint of a lock that *isn't* the computed one (#642) — click it to move the
/// green lock here.
pub const UNLOCKED_GRAY: Color32 = Color32::from_gray(130);
/// The Move tool's picked-point marks (#660): the source point reads **go** and the target
/// **stop**, so the direction of the snap is obvious at a glance.
pub const MOVE_START_POINT: Color32 = Color32::from_rgb(90, 220, 130);
pub const MOVE_END_POINT: Color32 = Color32::from_rgb(230, 80, 80);
/// The reachable spots offered for end point B (#670): blue for a candidate, and the gold the
/// rest of the app hovers with for the one under the cursor.
/// The start-A → end-A connector line (#740): yellow, so the vector reads apart from both
/// endpoint marks (green start, red end).
pub const MOVE_CONNECTOR: Color32 = Color32::from_rgb(245, 220, 80);
pub const MOVE_CANDIDATE: Color32 = Color32::from_rgb(110, 165, 245);
pub const MOVE_CANDIDATE_HOVER: Color32 = Color32::from_rgb(255, 210, 90);
/// Face Snap's moving-face highlight (#1361): cyan, so the face the user is moving reads apart
/// from the blue of the one it lands on.
pub const MOVE_MOVING_FACE: Color32 = Color32::from_rgb(80, 215, 215);
/// Face Snap's fixed (landing) face highlight (#1361): blue.
pub const MOVE_FIXED_FACE: Color32 = Color32::from_rgb(96, 150, 226);
/// Input-like control background (matches text inputs' `extreme_bg_color`).
pub const INPUT_BG: Color32 = Color32::from_gray(18);
/// Resting outline for an input-like control.
pub const INPUT_BORDER: Color32 = BORDER;

fn set_widget_visuals(
    w: &mut WidgetVisuals,
    bg: Color32,
    border: Color32,
    fg: Color32,
    rounding: f32,
    expansion: f32,
) {
    // Buttons paint `weak_bg_fill`, not `bg_fill`.
    w.weak_bg_fill = bg;
    w.bg_fill = bg;
    w.bg_stroke = Stroke::new(1.0, border);
    w.fg_stroke = Stroke::new(1.0, fg);
    w.corner_radius = CornerRadius::same(rounding as u8);
    w.expansion = expansion;
}

/// Build the dark [`egui::Visuals`] used across the app.
pub fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL_BG;
    v.window_fill = VIEWPORT_BG;
    v.extreme_bg_color = Color32::from_gray(18);
    v.faint_bg_color = Color32::from_gray(24);
    v.code_bg_color = Color32::from_rgb(22, 24, 30);

    set_widget_visuals(
        &mut v.widgets.noninteractive,
        PANEL_BG,
        BORDER,
        Color32::from_gray(200),
        2.0,
        0.0,
    );
    set_widget_visuals(
        &mut v.widgets.inactive,
        WIDGET_BG,
        BORDER,
        Color32::from_gray(210),
        2.0,
        0.0,
    );
    set_widget_visuals(
        &mut v.widgets.hovered,
        WIDGET_HOVER,
        Color32::from_gray(75),
        Color32::from_gray(235),
        3.0,
        1.0,
    );
    set_widget_visuals(
        &mut v.widgets.active,
        WIDGET_ACTIVE,
        Color32::from_gray(90),
        Color32::WHITE,
        2.0,
        1.0,
    );
    v.widgets.active.fg_stroke = Stroke::new(2.0, Color32::WHITE);
    set_widget_visuals(
        &mut v.widgets.open,
        WIDGET_ACTIVE,
        BORDER,
        Color32::from_gray(220),
        2.0,
        0.0,
    );

    v.selection.bg_fill = SELECTION_BG;
    v.selection.stroke = Stroke::new(1.0, SELECTION_TEXT);

    v.hyperlink_color = Color32::from_rgb(120, 170, 240);
    v.warn_fg_color = Color32::from_rgb(240, 200, 120);
    v.error_fg_color = Color32::from_rgb(220, 90, 90);

    v
}

/// Command palette console fill.
pub fn palette_console_fill() -> Color32 {
    PALETTE_CONSOLE_BG
}

/// Panel chrome frame (toolbar, status bar, side panes).
pub fn panel_frame() -> egui::Frame {
    egui::Frame {
        fill: PANEL_BG,
        stroke: Stroke::new(1.0, BORDER),
        inner_margin: egui::Margin::symmetric(8, 6),
        ..Default::default()
    }
}

/// Apply dark theme to an egui context.
/// The egui family a pencil drawing's labels are laid out in (#1830).
pub fn hand_lettered_family() -> egui::FontFamily {
    egui::FontFamily::Name(crate::pencil::LABEL_FONT_FAMILY.into())
}

/// Register the bundled hand-lettered face (#1830) so a pencil view's captions and dimensions
/// can be set in it. Called once with the theme, before anything lays text out: asking egui to
/// lay out text in a family its atlas has not been told about panics (#392).
fn register_hand_lettered_font(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let key = crate::pencil::LABEL_FONT_FAMILY.to_string();
    fonts.font_data.insert(
        key.clone(),
        std::sync::Arc::new(egui::FontData::from_static(crate::pencil::LABEL_FONT)),
    );
    // The hand-lettered face first, then the default proportional stack behind it, so glyphs
    // it lacks still render rather than coming out blank.
    let mut stack = vec![key];
    if let Some(default) = fonts.families.get(&egui::FontFamily::Proportional) {
        stack.extend(default.iter().cloned());
    }
    fonts
        .families
        .insert(egui::FontFamily::Name(crate::pencil::LABEL_FONT_FAMILY.into()), stack);
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    register_hand_lettered_font(ctx);
    // Stay on dark visuals even when the OS prefers light mode.
    ctx.set_theme(ThemePreference::Dark);
    let v = visuals();
    ctx.set_visuals_of(Theme::Dark, v.clone());
    ctx.set_visuals_of(Theme::Light, v.clone());
    ctx.set_visuals(v);
    // Finger-sized widgets in touch mode: taller interact targets and roomier padding,
    // so buttons, rows, and checkboxes clear ~40 physical px on tablets and phones.
    if crate::touch::active() {
        ctx.all_styles_mut(|style| {
            style.spacing.interact_size.y = style.spacing.interact_size.y.max(28.0);
            style.spacing.button_padding = style.spacing.button_padding.max(egui::vec2(8.0, 6.0));
            style.spacing.item_spacing = style.spacing.item_spacing.max(egui::vec2(8.0, 6.0));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #1830: the bundled hand-lettered face has to be one egui's own parser accepts, and it
    /// has to actually carry the Latin letters a caption is made of. An unparseable face
    /// panics inside the glyph-atlas build (#392), and a face without the glyphs would letter
    /// every caption in the fallback sans while looking registered.
    #[test]
    fn the_bundled_hand_lettered_face_parses_and_has_latin() {
        use ab_glyph::Font as _;
        let face = ab_glyph::FontRef::try_from_slice(crate::pencil::LABEL_FONT)
            .expect("egui's parser accepts the bundled face");
        for ch in "BodyFront-Right0123456789 —".chars() {
            assert!(
                face.glyph_id(ch).0 != 0,
                "the face is missing {ch:?}, so captions would fall back to the sans"
            );
        }
    }

    /// #1830: …and registering it leaves the family egui lays a pencil caption out in.
    #[test]
    fn applying_the_theme_registers_the_hand_lettered_family() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let family = hand_lettered_family();
        assert_eq!(family, egui::FontFamily::Name(crate::pencil::LABEL_FONT_FAMILY.into()));
        // A pass has to run before the atlas knows the family; after that, laying text out in
        // it must work and must not silently come back as the default.
        let _ = ctx.run_ui(Default::default(), |_| {});
        let width = |f: egui::FontFamily| {
            let mut w = 0.0;
            let _ = ctx.run_ui(Default::default(), |ui| {
                w = ui
                    .painter()
                    .layout_no_wrap(
                        "Body 4 — Front".to_string(),
                        egui::FontId::new(11.0, f.clone()),
                        egui::Color32::WHITE,
                    )
                    .size()
                    .x;
            });
            w
        };
        assert!(width(family.clone()) > 0.0, "the hand-lettered family lays text out");
        assert_ne!(
            width(family),
            width(egui::FontFamily::Proportional),
            "and it is a different face from the sans, not a silent fallback"
        );
    }

    #[test]
    fn theme_is_dark_and_matches_viewport() {
        let v = visuals();
        assert!(v.dark_mode);
        assert_eq!(v.window_fill, VIEWPORT_BG);
        assert_eq!(v.panel_fill, PANEL_BG);
    }

    #[test]
    fn button_fill_uses_weak_bg_fill() {
        let v = visuals();
        assert_eq!(v.widgets.inactive.weak_bg_fill, WIDGET_BG);
        assert_eq!(v.widgets.inactive.weak_bg_fill, v.widgets.inactive.bg_fill);
    }

    #[test]
    fn selected_tool_has_high_contrast() {
        let v = visuals();
        assert_eq!(v.selection.stroke.color, SELECTION_TEXT);
        assert!(
            v.selection.bg_fill.a() < 70,
            "selection background should stay faint"
        );
    }
}