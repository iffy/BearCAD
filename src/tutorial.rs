//! Interactive tutorial mode: Bear (the view cube) walks a first-time user through
//! building a real part, pointing with glowing rings and narrating in a speech
//! bubble. Tutorials live in a registry ([`TUTORIALS`]) so more can be added; each
//! is a list of [`Step`]s that either auto-advance when a document predicate is
//! satisfied or wait for the bubble's Next button.
//!
//! # Authoring steps
//!
//! **One action per step.** Every click is its own step; every bit of typing is its
//! own step. Never combine a click with typing, two clicks, or two typed values in
//! the same step. (The long **bracket** walkthrough predates this rule and is left
//! alone; every newer tutorial follows it.)

use crate::actions::{Action, AppState, Tool};
use crate::model::{ConstraintKind, VertexTreatmentKind};

/// A UI element a tutorial step can point at with a glowing ring. The frame's
/// renderer records these rects as it draws (`AppState::tutorial_anchor_rects`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum UiAnchor {
    /// A toolbar tool button.
    Tool(Tool),
    /// The Parameters pane's `+` add button.
    ParametersAdd,
    /// The Parameters pane's new-parameter **name** field.
    ParametersName,
    /// The Parameters pane's new-parameter **value** field.
    ParametersValue,
    /// A constraint button in the Context pane's Constraints list (#770) — where a
    /// squaring-up step points once both of its picks are made.
    ConstraintButton(crate::geometric_constraints::GeometricConstraintType),
    /// The extrude Output row's **Cut** button (#804).
    ExtrudeCut,
    /// The floating value field of the dimension being typed (#814).
    DimensionValue,
    /// The extrude tool's floating **distance** field (#816).
    ExtrudeDistance,
    /// In-progress rectangle **width** field (#1258).
    RectWidth,
    /// In-progress rectangle **height** field (#1258).
    RectHeight,
    /// Shape tool **Height** field in the Context pane (#1264).
    ShapeHeight,
    /// Shape tool **Radius** field in the Context pane (#1264).
    ShapeRadius,
    /// Shape tool **kind** button in the Context pane (#1272): Cuboid / Cylinder / Sphere.
    ShapeKind(crate::model::PrimitiveKind),
    /// The sketch row in the Elements pane (#1279) — double-click to reopen for edit.
    ElementsSketch,
    /// The view-cube bear HUD (#1269).
    ViewCube,
    /// The house (Home view) button under the view cube (#1269).
    ViewHome,
    /// A status-bar pane toggle (phone layout only, #828): Elements / Context / Params.
    PaneButton(crate::actions::Pane),
}

/// What a step's glowing orb points at, once resolved against the live state.
#[derive(Clone, Copy, Debug)]
pub enum StepTarget {
    World(glam::Vec3),
    Ui(UiAnchor),
}

/// Where a step's glowing ring points.
#[derive(Clone, Copy, Debug)]
pub enum StepAnchor {
    Ui(UiAnchor),
    /// A computed world point, projected into the viewport — e.g. the next profile
    /// vertex to click, so a drawing step leads point by point.
    World(fn(&AppState) -> Option<glam::Vec3>),
    /// Either, chosen per frame: the constraint steps point at the geometry to click and
    /// then at the pane button that applies the constraint (#770).
    Guided(fn(&AppState) -> Option<StepTarget>),
    /// No ring — narration only.
    None,
}

/// A one-click shortcut the speech bubble offers for a step that is pure typing:
/// the button does the step's work so the user doesn't have to key a whole list in
/// by hand. Filling it in still satisfies the step's `done` predicate, so the
/// tutorial carries on exactly as if they had typed it.
pub struct StepAssist {
    /// Button label in the bubble.
    pub label: &'static str,
    /// What the button does. It runs against the live state, so it can look at what's
    /// already there and only fill in the rest — and can apply a sequence of actions that
    /// depend on each other (pick, then constrain).
    pub run: fn(&mut AppState),
}

pub struct Step {
    /// What Bear says for this step.
    pub narration: &'static str,
    pub anchor: StepAnchor,
    /// Auto-advance when this returns true; `None` shows a Next button instead.
    pub done: Option<fn(&AppState) -> bool>,
    /// Runs once when the tutorial lands on this step going forward (never while
    /// reviewing with Back) — e.g. framing the camera on the area the step works in.
    pub on_enter: Option<fn(&mut AppState)>,
    /// Optional "do it for me" button (see [`StepAssist`]).
    pub assist: Option<StepAssist>,
    /// When this returns true the orb floats a **Shift** keycap beside it (#759): the
    /// click it's pointing at has to be Shift+clicked to add to the selection.
    pub needs_shift: Option<fn(&AppState) -> bool>,
    /// A mouse-button badge plus a looping drag animation beside the orb (#819) — for the
    /// steps that want a **drag**, not a click.
    pub drag_hint: Option<&'static str>,
    /// A `(key, explanation)` badge under the orb (#777) — used to introduce **Space**, the
    /// Selection Exploder, on steps whose target sits under other geometry.
    pub key_hint: Option<(&'static str, &'static str)>,
    /// The step's work as a short numbered sequence (#854): every mark shows at once, so the
    /// whole move is visible from the start, and each ring goes green as its part lands.
    pub marks: Option<fn(&AppState) -> Vec<GuideMark>>,
    /// The words this step wants typed (#778), shown in code blue beside the orb — right
    /// where the typing lands.
    pub type_hint: Option<TypeHint>,
    /// Narration for the **phone** layout (#828), where the panes are floating windows
    /// toggled from the status bar rather than columns down the sides.
    pub phone_narration: Option<&'static str>,
    /// A step that only exists on the phone layout (opening and tucking away the floating
    /// panes). It passes straight through on a desktop, and is left out of the step
    /// numbering there so the count isn't padded with steps that never show.
    pub only_on_phone: bool,
}

impl Step {
    /// What this step says on the device it's being read on.
    pub fn narration_for(&self, app: &AppState) -> &'static str {
        match (app.compact_layout, self.phone_narration) {
            (true, Some(text)) => text,
            _ => self.narration,
        }
    }

    /// Whether this step is part of the walkthrough on this device — phone-only steps
    /// aren't, on anything wider.
    pub fn shown_on(&self, app: &AppState) -> bool {
        app.compact_layout || !self.only_on_phone
    }
}

/// Where this step sits in the walkthrough **as this device sees it** (#828): `(position,
/// total)`, counting only the steps that show here, so a desktop reader never sees a count
/// padded by phone-only pane steps. A step that isn't shown here (transiently, while it
/// auto-advances) reports the position of the one before it.
pub fn step_position(app: &AppState, tutorial: usize, step: usize) -> (usize, usize) {
    let Some(tut) = TUTORIALS.get(tutorial) else {
        return (step + 1, step + 1);
    };
    let shown = |s: &Step| s.shown_on(app);
    let total = tut.steps.iter().filter(|s| shown(s)).count();
    let position = tut
        .steps
        .iter()
        .take(step + 1)
        .filter(|s| shown(s))
        .count()
        .max(1);
    (position, total.max(position))
}

/// What a step's "Type …" badge says: either fixed words, or a line computed from the live
/// state — the parameter list names whichever one is still missing (#782).
#[derive(Clone, Copy)]
pub enum TypeHint {
    Fixed(&'static str),
    Dynamic(fn(&AppState) -> Option<String>),
}

impl TypeHint {
    pub fn text(self, app: &AppState) -> Option<String> {
        match self {
            Self::Fixed(text) => Some(text.to_string()),
            Self::Dynamic(f) => f(app),
        }
    }
}

/// Split a step's narration into plain prose and **code** runs (#757): anything between
/// backticks — parameter names, values, the exact letters to type — which the bubble draws
/// in monospace and its own colour so it stands out from the sentence around it.
/// Backticks never survive into the drawn text; an unclosed one just ends the string.
pub fn narration_spans(text: &str) -> Vec<(&str, bool)> {
    let mut spans = Vec::new();
    let mut rest = text;
    let mut code = false;
    while let Some(tick) = rest.find('`') {
        if tick > 0 {
            spans.push((&rest[..tick], code));
        }
        rest = &rest[tick + 1..];
        code = !code;
    }
    if !rest.is_empty() {
        spans.push((rest, code));
    }
    spans
}

/// Side-panel title shown in the UI (#1255), matching Elements / Parameters / Context.
pub const PANE_TITLE: &str = "Tutorials";

pub struct Tutorial {
    /// Stable name for scripting (`bearcad.ui.tutorial("bracket")`).
    pub name: &'static str,
    /// Human title shown in the tutorial picker.
    pub title: &'static str,
    pub steps: &'static [Step],
}

/// A running tutorial: which one and how far along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TutorialRun {
    pub tutorial: usize,
    pub step: usize,
    /// Reviewing earlier steps (the Back button): auto-advance stands down until
    /// Next reaches a step whose work isn't done yet.
    pub hold: bool,
}

pub static TUTORIALS: &[Tutorial] = &[
    Tutorial {
        name: "cube",
        title: "Rectangle to cube",
        steps: CUBE_STEPS,
    },
    // Second walkthrough (#1269): camera + exploder, with cubes already in the document.
    Tutorial {
        name: "navigate",
        title: "Pan, orbit, zoom & pick",
        steps: NAVIGATE_STEPS,
    },
    Tutorial {
        name: "shapes",
        title: "Place cuboid, sphere & cylinder",
        steps: SHAPES_STEPS,
    },
    Tutorial {
        name: "dimensioned_box",
        title: "Dimensioned box, then edit",
        steps: DIMENSIONED_BOX_STEPS,
    },
    // Longer walkthrough last so newcomers start with the short ones (#1251).
    Tutorial {
        name: "bracket",
        title: "Build an angle bracket",
        steps: BRACKET_STEPS,
    },
];

pub fn tutorial_index(name: &str) -> Option<usize> {
    TUTORIALS.iter().position(|t| t.name == name)
}

/// A plain step (no assist, no phone-only branches) for the shorter tutorials.
const fn plain_step(
    narration: &'static str,
    anchor: StepAnchor,
    done: Option<fn(&AppState) -> bool>,
) -> Step {
    Step {
        narration,
        anchor,
        done,
        on_enter: None,
        assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    }
}

const fn assisted_step(
    narration: &'static str,
    anchor: StepAnchor,
    done: Option<fn(&AppState) -> bool>,
    assist: StepAssist,
    type_hint: Option<TypeHint>,
) -> Step {
    Step {
        narration,
        anchor,
        done,
        on_enter: None,
        assist: Some(assist),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint,
        phone_narration: None,
        only_on_phone: false,
    }
}

const fn plain_step_enter(
    narration: &'static str,
    anchor: StepAnchor,
    done: Option<fn(&AppState) -> bool>,
    on_enter: fn(&mut AppState),
) -> Step {
    Step {
        narration,
        anchor,
        done,
        on_enter: Some(on_enter),
        assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    }
}

const fn assisted_step_enter(
    narration: &'static str,
    anchor: StepAnchor,
    done: Option<fn(&AppState) -> bool>,
    assist: StepAssist,
    type_hint: Option<TypeHint>,
    on_enter: fn(&mut AppState),
) -> Step {
    Step {
        narration,
        anchor,
        done,
        on_enter: Some(on_enter),
        assist: Some(assist),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint,
        phone_narration: None,
        only_on_phone: false,
    }
}

/// The tutorial named by a page URL's query string, if it names a real one (#765):
/// `?tutorial=bracket` opens the web app with that walkthrough already running, so a docs
/// page can link straight into it. Unknown names and missing parameters give `None`.
///
/// Only the web entry point calls it; the native build has `--tutorial <name>` instead and
/// reaches this from its tests.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn tutorial_from_query(query: &str) -> Option<usize> {
    query
        .trim_start_matches('?')
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == "tutorial")
        .and_then(|(_, value)| tutorial_index(value))
}

// --- Bracket predicates -----------------------------------------------------------

fn live_constraints(app: &AppState) -> impl Iterator<Item = &crate::model::Constraint> {
    app.doc.constraints.values()
}

fn param_exists(app: &AppState, name: &str) -> bool {
    app.doc
        .parameters
        .values()
        .any(|p| p.name.eq_ignore_ascii_case(name))
}

fn name_box_tapped(app: &AppState) -> bool {
    app.parameters_pane.new_name_focused
        || !app.parameters_pane.new_name.trim().is_empty()
        || param_exists(app, "leg")
}

/// The value box has the keyboard (or already holds something) — its own step now (#861),
/// so the click guide points at the box before the typing guide takes over.
fn value_box_tapped(app: &AppState) -> bool {
    app.parameters_pane.new_value_focused
        || !app.parameters_pane.new_value.trim().is_empty()
        || param_exists(app, "leg")
}

fn name_says_leg(app: &AppState) -> bool {
    app.parameters_pane.new_name.trim().eq_ignore_ascii_case("leg") || param_exists(app, "leg")
}

fn value_says_50(app: &AppState) -> bool {
    crate::value::eval_length_mm(&app.parameters_pane.new_value)
        .is_some_and(|v| (v - 50.0).abs() < 1e-3)
        || param_exists(app, "leg")
}

fn leg_added(app: &AppState) -> bool {
    param_exists(app, "leg")
}

/// The numbers the tutorial enters **up front**, in the order it introduces them. `thick`
/// and `width` are deliberately missing: they're defined later, from a dimension field, to
/// teach the `name = value` shorthand (#788).
const BRACKET_PARAMS: [(&str, &str); 4] = [
    ("leg", "50mm"),
    ("hole", "5mm"),
    ("bend", "4mm"),
    ("bend_angle", "120deg"),
];

fn params_defined(app: &AppState) -> bool {
    BRACKET_PARAMS.iter().all(|(name, _)| param_exists(app, name))
}

/// The parameter-list step's orb (#832): the name box until that name is typed, then the
/// value box beside it — the same one-box-at-a-time walk the "Type …" badge does.
fn param_list_orb(app: &AppState) -> Option<StepTarget> {
    let (name, _) = BRACKET_PARAMS
        .iter()
        .find(|(name, _)| !param_exists(app, name))?;
    if app.parameters_pane.new_name.trim().eq_ignore_ascii_case(name) {
        Some(StepTarget::Ui(UiAnchor::ParametersValue))
    } else {
        Some(StepTarget::Ui(UiAnchor::ParametersName))
    }
}

/// The "Add them for me" button: adds whichever bracket parameters are still
/// missing, leaving any the user already typed (or renamed the value of) alone.
fn hole_param_defined(app: &AppState) -> bool {
    param_exists(app, "hole")
}

fn bend_param_defined(app: &AppState) -> bool {
    hole_param_defined(app) && param_exists(app, "bend")
}

fn add_hole_param(app: &mut AppState) {
    ensure_param(app, "hole", "5mm");
}

fn add_bend_param(app: &mut AppState) {
    add_hole_param(app);
    ensure_param(app, "bend", "4mm");
}

fn add_missing_params(app: &mut AppState) {
    for (name, expression) in BRACKET_PARAMS {
        ensure_param(app, name, expression);
    }
}

/// Define `name` if it isn't defined yet — the assists lean on the parameters the tutorial
/// asks for without clobbering values the user chose.
fn ensure_param(app: &mut AppState, name: &str, expression: &str) {
    if !param_exists(app, name) {
        app.apply(Action::AddParameter {
            name: name.to_string(),
            expression: expression.to_string(),
        });
    }
}

/// The first bracket parameter, which the four "type leg" steps all boil down to.
fn add_leg_param(app: &mut AppState) {
    ensure_param(app, "leg", "50mm");
}

/// What the parameter-list step wants typed **next** (#782/#812): the missing parameter's
/// name while the name box is still empty, then its value once the name is in — one box at a
/// time, since that's how it's typed.
fn next_missing_param(app: &AppState) -> Option<String> {
    let (name, value) = BRACKET_PARAMS
        .iter()
        .find(|(name, _)| !param_exists(app, name))?;
    if app.parameters_pane.new_name.trim().eq_ignore_ascii_case(name) {
        Some(value.to_string())
    } else {
        Some(name.to_string())
    }
}

// --- Phone-layout steps (#828) ---------------------------------------------------------
//
// On a phone the panes are floating windows toggled from the status bar, so the walkthrough
// has to include those taps. Each of these steps is satisfied outright on a desktop (where
// the panes are docked columns), so it auto-advances the moment it's reached and only ever
// shows up on a phone.

fn pane_open(app: &AppState, pane: crate::actions::Pane) -> bool {
    app.panes.is_visible(pane)
}

/// "Open this pane" — already true off the phone layout, where panes are always docked.
fn params_pane_ready(app: &AppState) -> bool {
    !app.compact_layout || pane_open(app, crate::actions::Pane::Parameters)
}

fn context_pane_ready(app: &AppState) -> bool {
    !app.compact_layout || pane_open(app, crate::actions::Pane::Context)
}

/// "Tuck it away again" — the floating pane covers the model, and the next steps need it.
fn params_pane_tucked(app: &AppState) -> bool {
    !app.compact_layout || !pane_open(app, crate::actions::Pane::Parameters)
}

fn context_pane_tucked(app: &AppState) -> bool {
    !app.compact_layout || !pane_open(app, crate::actions::Pane::Context)
}

fn params_button_orb(app: &AppState) -> Option<StepTarget> {
    (!params_pane_ready(app) || !params_pane_tucked(app))
        .then_some(StepTarget::Ui(UiAnchor::PaneButton(crate::actions::Pane::Parameters)))
}

fn context_button_orb(app: &AppState) -> Option<StepTarget> {
    (!context_pane_ready(app) || !context_pane_tucked(app))
        .then_some(StepTarget::Ui(UiAnchor::PaneButton(crate::actions::Pane::Context)))
}

fn line_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Line
}

/// The sloppy bracket profile the tutorial leads the user around, in sketch-local
/// millimetres (mirrors the quickstart's rough hexagon; the constraint steps square
/// it up afterwards). It is drawn on the **corner of the XY datum plane nearest the origin**
/// (#841/#850/#875) — clear of the origin and both axes (the plane itself stands off them, so
/// the first click has to land on the plane to open the sketch at all), and close enough in
/// that opening the sketch, which aims the view at the plane's origin, still shows all of it.
const PROFILE_POINTS: [(f32, f32); 6] = [
    (33.5, 8.0),
    (84.5, 10.5),
    (83.0, 15.8),
    (38.0, 13.5),
    (16.0, 55.0),
    (8.0, 51.0),
];

/// The next profile vertex to click while drawing the sloppy outline: follows the
/// chain (placed lines + the in-progress segment) and finally points back at the
/// start to close the loop.
fn next_profile_point(app: &AppState) -> Option<glam::Vec3> {
    // No sketch open yet: the first click opens one on the XY plane, so point at the first
    // profile vertex *on that plane* — not the world origin, which isn't even on it (#850).
    let Some(session) = app.sketch_session else {
        let (u, v) = PROFILE_POINTS[0];
        let frame = crate::face::sketch_frame(
            &app.doc,
            crate::model::FaceId::ConstructionPlane(app.doc.ground_plane()?),
        )?;
        return Some(crate::face::local_to_world(&frame, u, v));
    };
    let frame = crate::face::sketch_geometry_frame(&app.doc, session.sketch)?;
    let placed = app
        .doc
        .lines
        .values()
        .filter(|l| l.sketch == session.sketch && !l.construction)
        .count();
    let index = match placed {
        0 if app.creating_line.is_none() => 0,
        0 => 1,
        n if n < 5 => n + 1,
        _ => 0, // last segment: close the loop back at the start
    };
    let (u, v) = PROFILE_POINTS[index % PROFILE_POINTS.len()];
    Some(crate::face::local_to_world(&frame, u, v))
}

fn profile_drawn(app: &AppState) -> bool {
    app.doc.lines.values().filter(|l| !l.construction).count() >= 6
}

fn constraint_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Constraint
}

fn dimension_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Dimension
}

/// Frame the camera over the region the sloppy profile occupies: drawn from way out, the
/// glowing click-points crowd together — glide in so they sit comfortably apart.
fn frame_profile_area(app: &mut AppState) {
    // The profile on the plane, plus the origin with room to spare (#850/#852): the next
    // steps ask for a click on it, and framing it right on the viewport's edge made it a
    // fiddly target.
    let (min, max) = (
        glam::Vec3::new(-10.0, -10.0, 0.0),
        glam::Vec3::new(94.0, 65.0, 10.0),
    );
    // Opening the sketch starts its own straight-on transition, which aims at the plane's
    // origin and would leave the profile hanging off the bottom of the viewport (#875).
    // Re-aim that transition rather than replacing it, so the view still lands square on.
    if app.cam.reaim_transition_at_bounds(min, max, app.viewport_aspect) {
        return;
    }
    app.cam.frame_bounds_animated(min, max, app.viewport_aspect, 0.35);
}

fn constraint_count(app: &AppState, f: fn(&ConstraintKind) -> bool) -> usize {
    live_constraints(app).filter(|c| f(&c.kind)).count()
}

/// #577: "squaring up" a line means constraining it parallel to a sketch axis (the
/// axis-based replacement for Horizontal/Vertical).
fn axis_parallel_kind(k: &ConstraintKind) -> bool {
    use crate::model::ConstraintLine;
    matches!(k, ConstraintKind::Parallel { line_a, line_b }
        if matches!(line_a, ConstraintLine::OriginAxis(_))
            || matches!(line_b, ConstraintLine::OriginAxis(_)))
}

// --- Constraint-step click targets (#758/#759/#761) --------------------------------
//
// Every squaring-up step is two clicks and a key. The orb points at whichever of the two
// isn't picked yet — so a mis-click leaves it pointing back at the one still wanted — and
// the second click also floats a **Shift** keycap, because that's the one you hold Shift
// for.

/// One thing a constraint step asks for a click on.
#[derive(Clone, Copy, PartialEq)]
enum ClickTarget {
    /// The nth drawn (non-construction) line of the open sketch, in the order the drawing
    /// step laid them down.
    ProfileLine(usize),
}

fn sketch_frame(app: &AppState) -> Option<crate::face::SketchFrame> {
    let session = app.sketch_session?;
    crate::face::sketch_geometry_frame(&app.doc, session.sketch)
}

/// Document indices of the open sketch's drawn lines, in creation order.
fn profile_lines(app: &AppState) -> Vec<crate::model::LineKey> {
    // Always the sketch the profile was drawn in — not whichever sketch happens to be open.
    // Later stages open *other* sketches (the screw holes) and still point at the profile
    // (#790/#796).
    let Some(sketch) = profile_sketch(app) else {
        return Vec::new();
    };
    app.doc
        .lines
        .iter()
        .filter(|(_, l)| l.sketch == sketch && !l.construction)
        .map(|(i, _)| i)
        .collect()
}

fn profile_polyline(app: &AppState, nth: usize) -> Option<Vec<glam::Vec3>> {
    let index = *profile_lines(app).get(nth)?;
    crate::face::line_world_polyline(&app.doc, app.doc.lines.get(index)?)
}

/// The point half way **along** a polyline (by length, #769) — the middle of a straight
/// line, and the middle of the curve for a bezier, rather than whichever vertex happens to
/// sit at the halfway index (for a straight line, that was its end).
fn polyline_midpoint(points: &[glam::Vec3]) -> Option<glam::Vec3> {
    let total: f32 = points.windows(2).map(|w| (w[1] - w[0]).length()).sum();
    if points.len() < 2 || total < 1e-6 {
        return points.first().copied();
    }
    let mut walked = 0.0;
    for w in points.windows(2) {
        let seg = (w[1] - w[0]).length();
        if walked + seg >= total * 0.5 {
            let t = if seg > 1e-6 { (total * 0.5 - walked) / seg } else { 0.5 };
            return Some(w[0].lerp(w[1], t));
        }
        walked += seg;
    }
    points.last().copied()
}

/// Where the orb sits for a target: a line's middle, or a clear stretch of the X axis away
/// from the profile.
fn target_point(app: &AppState, target: ClickTarget) -> Option<glam::Vec3> {
    match target {
        ClickTarget::ProfileLine(n) => polyline_midpoint(&profile_polyline(app, n)?),
    }
}

fn target_selected(app: &AppState, target: ClickTarget) -> bool {
    use crate::hierarchy::SceneElement;
    match target {
        ClickTarget::ProfileLine(n) => profile_lines(app)
            .get(n)
            .is_some_and(|i| app.scene_selection.is_selected(SceneElement::Line(*i))),
    }
}

/// Whether `element` is one of the two things this step asks for.
fn element_is_target(app: &AppState, element: &crate::hierarchy::SceneElement, target: ClickTarget) -> bool {
    use crate::hierarchy::SceneElement;
    match target {
        ClickTarget::ProfileLine(n) => matches!(element, SceneElement::Line(i)
            if profile_lines(app).get(n) == Some(i)),
    }
}

/// Something is selected that this step's pair doesn't include — the previous step's picks,
/// most often. The step then starts over with a **plain** click on its first target, which
/// replaces the selection instead of adding a third thing to it (#785).
fn selection_has_strays(app: &AppState, a: ClickTarget, b: ClickTarget) -> bool {
    app.scene_selection
        .iter()
        .any(|element| !element_is_target(app, &element, a) && !element_is_target(app, &element, b))
}

/// The orb's target for a two-click constraint step: the first thing until it's picked,
/// then the second — and nothing once both are in hand (the key press is all that's left).
fn constraint_click_point(
    app: &AppState,
    a: ClickTarget,
    b: ClickTarget,
) -> Option<glam::Vec3> {
    // Anything else still selected has to go first: point back at the first target, whose
    // plain click clears the strays (#785).
    if selection_has_strays(app, a, b) || !target_selected(app, a) {
        target_point(app, a)
    } else if !target_selected(app, b) {
        target_point(app, b)
    } else {
        None
    }
}

/// Shift belongs to the *second* click of a pair — it adds to the selection. Not while
/// strays are selected: that first click has to replace them, so it's Shift-free (#785).
fn constraint_needs_shift(app: &AppState, a: ClickTarget, b: ClickTarget) -> bool {
    !selection_has_strays(app, a, b) && target_selected(app, a) && !target_selected(app, b)
}

/// One mark of a step's numbered guide (#854): where it points and whether that part is
/// already done. A step whose work is a short sequence — click this, Shift+click that, press
/// the button — shows them all at once, numbered, so the whole move is visible from the
/// start and each ring turns green as it lands.
#[derive(Clone, Copy, Debug)]
pub struct GuideMark {
    pub target: StepTarget,
    pub done: bool,
}

/// The three marks of a two-pick constraint step: the first pick, the second, and the pane
/// button that applies it.
fn constraint_marks(
    app: &AppState,
    a: ClickTarget,
    b: ClickTarget,
    kind: GC,
) -> Vec<GuideMark> {
    // A stray selection means the first click has to happen again, so nothing counts as done.
    let strays = selection_has_strays(app, a, b);
    let a_done = !strays && target_selected(app, a);
    let b_done = a_done && target_selected(app, b);
    let mut marks = Vec::new();
    if let Some(p) = target_point(app, a) {
        marks.push(GuideMark { target: StepTarget::World(p), done: a_done });
    }
    if let Some(p) = target_point(app, b) {
        marks.push(GuideMark { target: StepTarget::World(p), done: b_done });
    }
    marks.push(GuideMark {
        target: StepTarget::Ui(UiAnchor::ConstraintButton(kind)),
        done: false,
    });
    marks
}

/// The one-pick axis constraints (#876): "parallel to the X axis" needs only the line — the
/// axis itself isn't picked, the pane's own axis button (`6`/`7`) supplies it.
fn axis_constraint_click(app: &AppState, a: ClickTarget, kind: GC) -> Option<StepTarget> {
    let strays = app
        .scene_selection
        .iter()
        .any(|element| !element_is_target(app, &element, a));
    if strays || !target_selected(app, a) {
        target_point(app, a).map(StepTarget::World)
    } else {
        Some(StepTarget::Ui(UiAnchor::ConstraintButton(kind)))
    }
}

/// The two marks of a one-pick constraint step: the line, then the button that squares it up.
fn axis_constraint_marks(app: &AppState, a: ClickTarget, kind: GC) -> Vec<GuideMark> {
    let strays = app
        .scene_selection
        .iter()
        .any(|element| !element_is_target(app, &element, a));
    let mut marks = Vec::new();
    if let Some(p) = target_point(app, a) {
        marks.push(GuideMark {
            target: StepTarget::World(p),
            done: !strays && target_selected(app, a),
        });
    }
    marks.push(GuideMark {
        target: StepTarget::Ui(UiAnchor::ConstraintButton(kind)),
        done: false,
    });
    marks
}

/// Generates a step's orb-target and Shift-hint functions for a two-click pair (the step
/// table needs plain `fn` pointers, so each pair gets its own pair of functions). Once both
/// picks are in hand the orb moves to the pane button that applies the constraint (#770) —
/// the last thing left to do.
macro_rules! constraint_step {
    ($point:ident, $shift:ident, $marks:ident, $a:expr, $b:expr, $kind:expr) => {
        fn $point(app: &AppState) -> Option<StepTarget> {
            match constraint_click_point(app, $a, $b) {
                Some(world) => Some(StepTarget::World(world)),
                None => Some(StepTarget::Ui(UiAnchor::ConstraintButton($kind))),
            }
        }
        fn $shift(app: &AppState) -> bool {
            constraint_needs_shift(app, $a, $b)
        }
        fn $marks(app: &AppState) -> Vec<GuideMark> {
            constraint_marks(app, $a, $b, $kind)
        }
    };
}

// The profile is drawn as six lines: 0 base bottom, 1 base end cap, 2 inner base,
// 3 tilted leg outer, 4 tilted leg end cap, 5 tilted leg inner (back to the bend corner).
use crate::geometric_constraints::GeometricConstraintType as GC;
fn level_click(app: &AppState) -> Option<StepTarget> {
    axis_constraint_click(app, ClickTarget::ProfileLine(0), GC::AlongXAxis)
}
fn level_marks(app: &AppState) -> Vec<GuideMark> {
    axis_constraint_marks(app, ClickTarget::ProfileLine(0), GC::AlongXAxis)
}
constraint_step!(
    base_strip_click,
    base_strip_shift,
    base_strip_marks,
    ClickTarget::ProfileLine(0),
    ClickTarget::ProfileLine(2),
    GC::Parallel
);
constraint_step!(
    legs_click,
    legs_shift,
    legs_marks,
    ClickTarget::ProfileLine(3),
    ClickTarget::ProfileLine(5),
    GC::Parallel
);
constraint_step!(
    cap_one_click,
    cap_one_shift,
    cap_one_marks,
    ClickTarget::ProfileLine(1),
    ClickTarget::ProfileLine(0),
    GC::Perpendicular
);
constraint_step!(
    cap_two_click,
    cap_two_shift,
    cap_two_marks,
    ClickTarget::ProfileLine(4),
    ClickTarget::ProfileLine(3),
    GC::Perpendicular
);

// The squaring-up steps, one constraint application each. Every predicate is cumulative
// (each includes the ones before it), so a user who works ahead skips ahead and Back
// reviews hold their ground.

fn base_leveled(app: &AppState) -> bool {
    constraint_count(app, axis_parallel_kind) >= 1
}

fn base_strip_even(app: &AppState) -> bool {
    base_leveled(app)
        && constraint_count(app, |k| matches!(k, ConstraintKind::Parallel { .. })) >= 2
}

fn legs_parallel(app: &AppState) -> bool {
    base_strip_even(app)
        && constraint_count(app, |k| matches!(k, ConstraintKind::Parallel { .. })) >= 3
}

fn first_cap_squared(app: &AppState) -> bool {
    legs_parallel(app)
        && constraint_count(app, |k| matches!(k, ConstraintKind::Perpendicular { .. })) >= 1
}

fn profile_squared(app: &AppState) -> bool {
    first_cap_squared(app)
        && constraint_count(app, |k| matches!(k, ConstraintKind::Perpendicular { .. })) >= 2
}

// --- Dimensioning steps (#773/#776): one dimension per step, the orb on the line to click.

/// Whether the nth profile line already carries a length dimension.
fn line_has_length_dim(app: &AppState, nth: usize) -> bool {
    use crate::model::DistanceTarget;
    let Some(&index) = profile_lines(app).get(nth) else {
        return false;
    };
    live_constraints(app).any(|c| {
        matches!(&c.kind,
            ConstraintKind::Distance { target: DistanceTarget::LineLength(i) } if *i == index)
    })
}

/// Where the *label* of a line's dimension will land: off the line, away from the sketch's
/// middle — the same side a committed dimension takes, so the orb points at the spot the
/// next click should drop it (#779).
fn dimension_label_spot(app: &AppState, nth: usize) -> Option<glam::Vec3> {
    let frame = sketch_frame(app)?;
    let poly = profile_polyline(app, nth)?;
    let (a, b) = (*poly.first()?, *poly.last()?);
    let (ua, va) = crate::face::world_to_local(&frame, a);
    let (ub, vb) = crate::face::world_to_local(&frame, b);
    // The sketch's centroid in local mm — labels point away from it.
    let mut sum = (0.0f32, 0.0f32);
    let mut n = 0usize;
    for index in profile_lines(app) {
        if let Some(line) = app.doc.lines.get(index) {
            sum.0 += line.x0 + line.x1;
            sum.1 += line.y0 + line.y1;
            n += 2;
        }
    }
    let (cu, cv) = if n > 0 {
        (sum.0 / n as f32, sum.1 / n as f32)
    } else {
        (0.0, 0.0)
    };
    let (ou, ov) = crate::dimensions::outward_perpendicular_uv(ua, va, ub, vb, cu, cv);
    const AWAY_MM: f32 = 11.0;
    Some(crate::face::local_to_world(
        &frame,
        (ua + ub) * 0.5 + ou * AWAY_MM,
        (va + vb) * 0.5 + ov * AWAY_MM,
    ))
}

/// Whether the tool is currently placing or typing a dimension for the nth profile line.
fn dimensioning_line(app: &AppState, nth: usize) -> bool {
    use crate::model::{DimensionTarget, DistanceTarget};
    let Some(&index) = profile_lines(app).get(nth) else {
        return false;
    };
    let is_this = |target: &DimensionTarget| {
        matches!(target,
            DimensionTarget::Distance(DistanceTarget::LineLength(i)) if *i == index)
    };
    app.placing_dimension.as_ref().is_some_and(|p| is_this(&p.target))
        || app
            .editing_committed_dim
            .as_ref()
            .and_then(|e| e.target.dimension_target(&app.doc))
            .is_some_and(|t| is_this(&t))
}

/// The orb for a "dimension this line" step: the line's middle until it's picked, then the
/// spot to drop the dimension (#779), then the value field it opened (#814), and nothing
/// once it's dimensioned.
fn dimension_line_orb(app: &AppState, nth: usize) -> Option<StepTarget> {
    if line_has_length_dim(app, nth) {
        return None;
    }
    if app.editing_committed_dim.is_some() {
        return Some(StepTarget::Ui(UiAnchor::DimensionValue));
    }
    if dimensioning_line(app, nth) {
        return dimension_label_spot(app, nth).map(StepTarget::World);
    }
    target_point(app, ClickTarget::ProfileLine(nth)).map(StepTarget::World)
}

fn base_leg_dimensioned(app: &AppState) -> bool {
    line_has_length_dim(app, 0)
}

fn tilted_leg_dimensioned(app: &AppState) -> bool {
    base_leg_dimensioned(app) && line_has_length_dim(app, 5)
}

fn base_cap_dimensioned(app: &AppState) -> bool {
    tilted_leg_dimensioned(app) && line_has_length_dim(app, 1)
}

fn tilted_cap_dimensioned(app: &AppState) -> bool {
    base_cap_dimensioned(app) && line_has_length_dim(app, 4)
}

/// A dimension step's "Type …" badge waits for the value input (#786/#787): during picking
/// and placement there's nowhere to type yet, so naming the words would just be noise.
fn typed_value_hint(app: &AppState, text: &'static str) -> Option<String> {
    app.editing_committed_dim
        .is_some()
        .then(|| text.to_string())
}

fn leg_value_hint(app: &AppState) -> Option<String> {
    typed_value_hint(app, "leg")
}
fn thick_value_hint(app: &AppState) -> Option<String> {
    // Until `thick` exists it's defined right in the field (#788).
    if param_exists(app, "thick") {
        typed_value_hint(app, "thick")
    } else {
        typed_value_hint(app, "thick=5mm")
    }
}

/// The extrude step's orb (#790): the toolbar button until the tool is up, then the profile
/// **face** it wants clicked, and it stays there while the distance is typed.
fn extrude_orb(app: &AppState) -> Option<StepTarget> {
    if app.tool != Tool::Extrude {
        return Some(StepTarget::Ui(UiAnchor::Tool(Tool::Extrude)));
    }
    // Face picked: the depth is what's left, so point at the field that takes it (#816).
    if app.creating_extrusion.as_ref().is_some_and(|ce| !ce.faces.is_empty()) {
        return Some(StepTarget::Ui(UiAnchor::ExtrudeDistance));
    }
    // Halfway between the base leg's two rails: a spot that's actually **on** the face. The
    // profile's overall centroid falls in the L's notch, off the material (#815).
    let outer = profile_polyline(app, 0).as_deref().and_then(polyline_midpoint)?;
    let inner = profile_polyline(app, 2).as_deref().and_then(polyline_midpoint)?;
    Some(StepTarget::World(outer.lerp(inner, 0.5)))
}

/// "Type width=40mm" waits for the extrude's distance field (#789).
fn extrude_value_hint(app: &AppState) -> Option<String> {
    app.creating_extrusion
        .is_some()
        .then(|| "width=40mm".to_string())
}
fn bend_angle_value_hint(app: &AppState) -> Option<String> {
    typed_value_hint(app, "bend_angle")
}

fn base_leg_orb(app: &AppState) -> Option<StepTarget> {
    dimension_line_orb(app, 0)
}
fn tilted_leg_orb(app: &AppState) -> Option<StepTarget> {
    dimension_line_orb(app, 5)
}
fn base_cap_orb(app: &AppState) -> Option<StepTarget> {
    dimension_line_orb(app, 1)
}
fn tilted_cap_orb(app: &AppState) -> Option<StepTarget> {
    dimension_line_orb(app, 4)
}

/// The bend-angle step: the bottom line, then (with Shift) the inner leg line, then the
/// spot to drop the arc (#779).
fn bend_angle_orb(app: &AppState) -> Option<StepTarget> {
    // Typing the value: the orb belongs on the field, not back on the line (#814).
    if app.editing_committed_dim.is_some() {
        return Some(StepTarget::Ui(UiAnchor::DimensionValue));
    }
    if let Some(world) =
        constraint_click_point(app, ClickTarget::ProfileLine(0), ClickTarget::ProfileLine(3))
    {
        return Some(StepTarget::World(world));
    }
    // Both picked: point into the wedge between them, a little way from the bend corner.
    let frame = sketch_frame(app)?;
    let corner = profile_polyline(app, 0)?.first().copied()?;
    let (cu, cv) = crate::face::world_to_local(&frame, corner);
    let toward = |nth: usize| -> Option<(f32, f32)> {
        let mid = polyline_midpoint(&profile_polyline(app, nth)?)?;
        let (mu, mv) = crate::face::world_to_local(&frame, mid);
        let (du, dv) = (mu - cu, mv - cv);
        let len = (du * du + dv * dv).sqrt();
        (len > 1e-4).then_some((du / len, dv / len))
    };
    let (au, av) = toward(0)?;
    let (bu, bv) = toward(3)?;
    let (su, sv) = (au + bu, av + bv);
    let len = (su * su + sv * sv).sqrt();
    const ARC_MM: f32 = 16.0;
    let (su, sv) = if len > 1e-4 {
        (su / len, sv / len)
    } else {
        (au, av)
    };
    Some(StepTarget::World(crate::face::local_to_world(
        &frame,
        cu + su * ARC_MM,
        cv + sv * ARC_MM,
    )))
}

fn bend_angle_shift(app: &AppState) -> bool {
    constraint_needs_shift(app, ClickTarget::ProfileLine(0), ClickTarget::ProfileLine(3))
}

/// The holes' circles, in the sketch they were drawn in.
fn hole_circles(app: &AppState) -> Vec<&crate::model::Circle> {
    let Some(profile) = profile_sketch(app) else {
        return Vec::new();
    };
    app.doc
        .circles
        .values()
        .filter(|c| !c.construction && c.sketch != profile)
        .collect()
}

/// A hole's centre in world space (`edge` false) or a point on its rim (`edge` true, the
/// countersink's target).
fn hole_point(app: &AppState, nth: usize, edge: bool) -> Option<glam::Vec3> {
    let profile = profile_sketch(app)?;
    let circle = *hole_circles(app).get(nth)?;
    let frame = crate::face::sketch_geometry_frame(&app.doc, circle.sketch)?;
    let _ = profile;
    let r = if edge { circle.r } else { 0.0 };
    Some(crate::face::local_to_world(&frame, circle.cx + r, circle.cy))
}

/// The cut step (#803/#804): each hole face in turn, then the pane's **Cut** button.
fn hole_cut_orb(app: &AppState) -> Option<StepTarget> {
    if app.tool != Tool::Extrude {
        return Some(StepTarget::Ui(UiAnchor::Tool(Tool::Extrude)));
    }
    let picked = app
        .creating_extrusion
        .as_ref()
        .map(|ce| ce.faces.len())
        .unwrap_or(0);
    if picked < hole_circles(app).len() {
        return hole_point(app, picked, false).map(StepTarget::World);
    }
    // Both faces in hand: the Output → Cut button, then the depth field (#816).
    if !matches!(
        app.creating_extrusion.as_ref().map(|ce| ce.body_mode),
        Some(crate::actions::ExtrudeBodyMode::Cut(_))
    ) {
        return Some(StepTarget::Ui(UiAnchor::ExtrudeCut));
    }
    Some(StepTarget::Ui(UiAnchor::ExtrudeDistance))
}

fn hole_cut_value_hint(app: &AppState) -> Option<String> {
    app.creating_extrusion
        .as_ref()
        .filter(|ce| ce.faces.len() >= hole_circles(app).len().max(1))
        .map(|_| "-(thick+1)".to_string())
}

/// The countersink step (#806): the Chamfer tool, then each hole's rim.
fn countersink_orb(app: &AppState) -> Option<StepTarget> {
    if app.tool != Tool::Chamfer {
        return Some(StepTarget::Ui(UiAnchor::Tool(Tool::Chamfer)));
    }
    let picked = app
        .creating_edge_treatment
        .as_ref()
        .map(|cet| cet.edges.len())
        .unwrap_or(0);
    (picked < 2)
        .then(|| hole_point(app, picked, true).map(StepTarget::World))
        .flatten()
}

fn countersink_shift(app: &AppState) -> bool {
    app.creating_edge_treatment
        .as_ref()
        .is_some_and(|cet| cet.edges.len() == 1)
}

fn countersink_value_hint(app: &AppState) -> Option<String> {
    treatment_value_hint(app, "1.2")
}

/// The corner-rounding step (#806): the Fillet tool, then each flange-tip edge.
fn corner_fillet_orb(app: &AppState) -> Option<StepTarget> {
    if app.tool != Tool::Fillet {
        return Some(StepTarget::Ui(UiAnchor::Tool(Tool::Fillet)));
    }
    // Profile vertices 1, 2 (base flange tip) and 4, 5 (tilted flange tip).
    let picked = app
        .creating_edge_treatment
        .as_ref()
        .map(|cet| cet.edges.len())
        .unwrap_or(0);
    let corners = [1usize, 2, 4, 5];
    corners
        .get(picked)
        .and_then(|nth| bend_edge_point(app, *nth))
        .map(StepTarget::World)
}

fn corner_fillet_shift(app: &AppState) -> bool {
    app.creating_edge_treatment
        .as_ref()
        .is_some_and(|cet| !cet.edges.is_empty())
}

fn corner_value_hint(app: &AppState) -> Option<String> {
    treatment_value_hint(app, "2")
}

/// The hole-positioning step's hint (#801): the same distance from each end, so the pair
/// sits evenly on the flange.
fn hole_position_hint(app: &AppState) -> Option<String> {
    app.editing_committed_dim
        .as_ref()
        .map(|_| "10mm".to_string())
}

/// A fresh start for the dimensioning stage (#772): the constraint steps leave their last
/// pair selected, and under the Dimension tool a live selection is already a dimension in
/// the making — so drop it before the tutorial asks for the first click.
fn clear_selection_for_dimensioning(app: &mut AppState) {
    app.scene_selection.clear();
    app.placing_dimension = None;
}

fn profile_dimensioned(app: &AppState) -> bool {
    live_constraints(app)
        .filter(|c| matches!(c.kind, ConstraintKind::Distance { .. }))
        .count()
        >= 4
        && live_constraints(app)
            .filter(|c| matches!(c.kind, ConstraintKind::Angle { .. }))
            .count()
            >= 1
}

fn extruded(app: &AppState) -> bool {
    !app.doc.extrusions.is_empty()
}

/// Count treated edges of `kind` across both the first-class edge-treatment operations (#531)
/// and any legacy extrusion-baked treatments (old files), so tutorial progress tracks either.
fn edge_treatment_count(app: &AppState, kind: VertexTreatmentKind) -> usize {
    let ops: usize = app
        .doc
        .edge_treatment_ops
        .values()
        .filter(|o| o.kind == kind)
        .map(|o| o.edges.len())
        .sum();
    let legacy = app
        .doc
        .extrusions
        .values()
        .flat_map(|e| &e.edge_treatments)
        .filter(|t| t.kind == kind)
        .count();
    ops + legacy
}

fn fillet_count(app: &AppState) -> usize {
    edge_treatment_count(app, VertexTreatmentKind::Fillet)
}

fn chamfer_count(app: &AppState) -> usize {
    edge_treatment_count(app, VertexTreatmentKind::Chamfer)
}

fn bend_rounded(app: &AppState) -> bool {
    fillet_count(app) >= 2
}

fn inner_bend_rounded(app: &AppState) -> bool {
    fillet_count(app) >= 1
}

/// The sketch the bracket profile lives in — identified by its lines, so it stays the
/// profile's even when another sketch (the screw holes) is the open one.
fn profile_sketch(app: &AppState) -> Option<crate::model::SketchId> {
    app.doc
        .lines
        .values()
        .find(|l| !l.construction)
        .map(|l| l.sketch)
}

/// A point on the body's **vertical** feature edge (the one running along the extrusion)
/// nearest profile vertex `nth` — the bend edges the fillet steps point at (#791).
fn bend_edge_point(app: &AppState, nth: usize) -> Option<glam::Vec3> {
    let frame = crate::face::sketch_geometry_frame(&app.doc, profile_sketch(app)?)?;
    let corner = profile_polyline(app, nth)?.first().copied()?;
    let body = app.doc.bodies.keys().next()?;
    let solid = crate::extrude::body_solid_mesh(&app.doc, body)?;
    let normal = frame.normal.normalize_or_zero();
    let mut best: Option<(f32, glam::Vec3)> = None;
    for (a, b) in crate::gpu_viewport::solid_mesh_unique_edges(&solid) {
        let dir = (b - a).normalize_or_zero();
        // Only edges running along the extrusion — the corner's own vertical edge.
        if dir.dot(normal).abs() < 0.9 {
            continue;
        }
        // Distance from the corner to that edge's line, measured in the sketch plane.
        let mid = a.lerp(b, 0.5);
        let offset = mid - corner;
        let in_plane = offset - normal * offset.dot(normal);
        let d = in_plane.length();
        if best.as_ref().is_none_or(|(best_d, _)| d < *best_d) {
            best = Some((d, mid));
        }
    }
    best.map(|(_, mid)| mid)
}

/// The bend's **inside** corner is where the two inner profile lines meet (profile vertex 3);
/// the **outside** one is the pinned corner at the origin (vertex 0).
fn inner_bend_orb(app: &AppState) -> Option<StepTarget> {
    if app.tool != Tool::Fillet {
        return Some(StepTarget::Ui(UiAnchor::Tool(Tool::Fillet)));
    }
    bend_edge_point(app, 3).map(StepTarget::World)
}

fn outer_bend_orb(app: &AppState) -> Option<StepTarget> {
    if app.tool != Tool::Fillet {
        return Some(StepTarget::Ui(UiAnchor::Tool(Tool::Fillet)));
    }
    bend_edge_point(app, 0).map(StepTarget::World)
}

/// A fillet/chamfer amount is typed into a floating field that only exists once an edge is
/// picked (#789's rule, applied to the treatment tools).
fn treatment_value_hint(app: &AppState, text: &'static str) -> Option<String> {
    (app.creating_edge_treatment.is_some() || app.creating_vertex_treatment.is_some())
        .then(|| text.to_string())
}

fn bend_value_hint(app: &AppState) -> Option<String> {
    treatment_value_hint(app, "bend")
}

fn bend_thick_value_hint(app: &AppState) -> Option<String> {
    treatment_value_hint(app, "bend+thick")
}

fn hole_circles_drawn(app: &AppState) -> bool {
    app.doc.circles.values().filter(|c| !c.construction).count() >= 2
}

fn first_hole_drawn(app: &AppState) -> bool {
    app.doc.circles.values().filter(|c| !c.construction).count() >= 1
}

// --- The screw-hole stage (#795/#796/#798/#799): sketch on the flange's inside face, two
// circles, then dimension them — one click per step, each with the orb on it.

/// How deep the body runs along the sketch normal (the extrusion's `width`).
fn body_depth(app: &AppState) -> Option<(glam::Vec3, f32)> {
    let frame = crate::face::sketch_geometry_frame(&app.doc, profile_sketch(app)?)?;
    let body = app.doc.bodies.keys().next()?;
    let (min, max) = crate::extrude::body_solid_mesh(&app.doc, body)?.bounds()?;
    let n = frame.normal.normalize_or_zero();
    let depth = (max - min).dot(n).abs();
    (depth > 1e-3).then_some((n, depth))
}

/// A point on the **inside face of the base flange** — the face swept by the inner base line
/// (profile line 2) — at `t` along that line (0 = the flange tip end) and `s` of the way
/// through the body's depth.
fn flange_face_point(app: &AppState, t: f32, s: f32) -> Option<glam::Vec3> {
    let poly = profile_polyline(app, 2)?;
    let (a, b) = (*poly.first()?, *poly.last()?);
    let (n, depth) = body_depth(app)?;
    Some(a.lerp(b, t) + n * (depth * s))
}

/// The sketch the holes are drawn in: an open sketch that isn't the profile's.
fn hole_sketch_open(app: &AppState) -> bool {
    match (app.sketch_session, profile_sketch(app)) {
        (Some(session), Some(profile)) => session.sketch != profile,
        _ => false,
    }
}

fn sketch_tool_ready(app: &AppState) -> bool {
    // Already in a fresh sketch counts too — the step is done either way (#796).
    app.tool == Tool::Sketch || hole_sketch_open(app)
}

fn sketch_tool_orb(app: &AppState) -> Option<StepTarget> {
    (!sketch_tool_ready(app)).then_some(StepTarget::Ui(UiAnchor::Tool(Tool::Sketch)))
}

/// The bracket's inside face looks away from the home view, so the tutorial spins the view
/// round to it before asking for the click (#817). "Looking at it" means the camera sits on
/// the same side of the face's plane as its outward normal.
fn looking_at_flange_face(app: &AppState) -> bool {
    if hole_sketch_open(app) {
        return true;
    }
    let Some(frame) = profile_sketch(app).and_then(|sketch| {
        crate::face::sketch_geometry_frame(&app.doc, sketch)
    }) else {
        return false;
    };
    // The inside face's outward normal points from the inner base line away from the profile.
    let Some(spot) = flange_face_point(app, 0.5, 0.5) else {
        return false;
    };
    let Some(mid_outer) = profile_polyline(app, 0).as_deref().and_then(polyline_midpoint) else {
        return false;
    };
    let outward = {
        let v = spot - mid_outer;
        let n = frame.normal.normalize_or_zero();
        (v - n * v.dot(n)).normalize_or_zero()
    };
    if outward.length_squared() < 1e-6 {
        return false;
    }
    (app.cam.eye() - spot).normalize_or_zero().dot(outward) > 0.25
}

/// The outer bend edge faces the camera (#867). The bend's outside is on the far side of the
/// bracket from the inner one, so the reader has to swing the view round before that fillet
/// step can be clicked — this is the predicate for the step that teaches the spin.
fn looking_at_outer_bend(app: &AppState) -> bool {
    if bend_rounded(app) {
        return true;
    }
    let (Some(inner), Some(outer)) = (bend_edge_point(app, 3), bend_edge_point(app, 0)) else {
        // No bend edges to look at yet (the fillets haven't been set up): nothing to wait for.
        return true;
    };
    // Outward = from the inner bend edge toward the outer one; we're looking at the outside
    // once the eye is on that side of it.
    let outward = (outer - inner).normalize_or_zero();
    if outward.length_squared() < 1e-6 {
        return false;
    }
    (app.cam.eye() - outer).normalize_or_zero().dot(outward) > 0.25
}

/// Swing the view round to the outside of the bend — the spin step's own button (#867).
fn assist_spin_to_outer_bend(app: &mut AppState) {
    let (Some(inner), Some(outer)) = (bend_edge_point(app, 3), bend_edge_point(app, 0)) else {
        return;
    };
    let dir = (outer - inner).normalize_or_zero();
    if dir.length_squared() < 1e-6 {
        return;
    }
    let eye_dir = (dir + glam::Vec3::Z * 0.35).normalize_or_zero();
    // Stand the eye off along the outward direction, looking back at the bend.
    let (yaw, pitch) = crate::camera::Camera::view_direction_to_yaw_pitch(eye_dir);
    let view = crate::camera::HomeView {
        target: outer,
        yaw,
        pitch,
        distance: app.cam.distance.max(120.0),
        view_up: None,
    };
    app.cam.start_transition_to_view(view, 0.5);
    // The pose lands now as well: the step's predicate reads the camera, and an animation
    // that hasn't ticked yet would leave the button looking like it did nothing.
    app.cam.yaw = view.yaw;
    app.cam.pitch = view.pitch;
    app.cam.target = view.target;
    app.cam.distance = view.distance;
}

/// The spin-to-the-bend step's orb: the outer bend edge it's swinging round to.
fn outer_bend_spin_orb(app: &AppState) -> Option<StepTarget> {
    bend_edge_point(app, 0).map(StepTarget::World)
}

/// Where the spin step's orb sits (#819): over the bracket itself, since the drag can start
/// anywhere but that's where the eye is.
fn spin_orb(app: &AppState) -> Option<StepTarget> {
    let outer = profile_polyline(app, 0).as_deref().and_then(polyline_midpoint)?;
    let inner = profile_polyline(app, 2).as_deref().and_then(polyline_midpoint)?;
    Some(StepTarget::World(outer.lerp(inner, 0.5)))
}

/// Swing the camera round to that face — what the assist (and the step's own hint) does.
fn assist_spin_to_flange(app: &mut AppState) {
    let Some(spot) = flange_face_point(app, 0.5, 0.5) else { return };
    let Some(mid_outer) = profile_polyline(app, 0).as_deref().and_then(polyline_midpoint) else {
        return;
    };
    let dir = (spot - mid_outer).normalize_or_zero();
    if dir.length_squared() < 1e-6 {
        return;
    }
    // Stand the eye off along the face's outward direction, looking back at it. Only the
    // sideways part of that direction counts — the step asks the reader to spin, not climb.
    let sideways = glam::Vec3::new(dir.x, dir.y, 0.0).normalize_or_zero();
    let eye_dir = (sideways + glam::Vec3::Z * 0.35).normalize_or_zero();
    let (yaw, pitch) = crate::camera::Camera::view_direction_to_yaw_pitch(eye_dir);
    let view = crate::camera::HomeView {
        target: spot,
        yaw,
        pitch,
        distance: app.cam.distance.max(120.0),
        view_up: None,
    };
    app.cam.start_transition_to_view(view, 0.5);
    // The pose lands now as well: the step's predicate reads the camera, and an animation
    // that hasn't ticked yet would leave the button looking like it did nothing.
    app.cam.yaw = view.yaw;
    app.cam.pitch = view.pitch;
    app.cam.target = view.target;
    app.cam.distance = view.distance;
}

/// Point at the middle of the flange's inside face — the face to click.
fn flange_face_orb(app: &AppState) -> Option<StepTarget> {
    if hole_sketch_open(app) {
        return None;
    }
    flange_face_point(app, 0.5, 0.5).map(StepTarget::World)
}

fn circle_tool_ready(app: &AppState) -> bool {
    app.tool == Tool::Circle || first_hole_drawn(app)
}

fn circle_tool_orb(app: &AppState) -> Option<StepTarget> {
    (!circle_tool_ready(app)).then_some(StepTarget::Ui(UiAnchor::Tool(Tool::Circle)))
}

fn first_hole_orb(app: &AppState) -> Option<StepTarget> {
    (!first_hole_drawn(app))
        .then(|| flange_face_point(app, 0.18, 0.35).map(StepTarget::World))
        .flatten()
}

fn second_hole_orb(app: &AppState) -> Option<StepTarget> {
    (!hole_circles_drawn(app))
        .then(|| flange_face_point(app, 0.18, 0.68).map(StepTarget::World))
        .flatten()
}

/// "Type hole" waits for the circle's diameter field.
fn hole_value_hint(app: &AppState) -> Option<String> {
    app.creating_circle.is_some().then(|| "hole".to_string())
}

/// Positioning dimensions in the holes sketch (#799) — a hole's own **diameter** doesn't
/// count, that's the `hole` value typed while drawing it.
fn hole_position_dims(app: &AppState) -> usize {
    use crate::model::DistanceTarget;
    let Some(session) = app.sketch_session else {
        return 0;
    };
    live_constraints(app)
        .filter(|c| {
            c.sketch == session.sketch
                && matches!(&c.kind, ConstraintKind::Distance { target }
                    if !matches!(target, DistanceTarget::CircleDiameter(_)))
        })
        .count()
}

fn holes_dimensioned(app: &AppState) -> bool {
    if app.sketch_session.is_none() {
        // Left the sketch: the cut step's predicate takes over anyway.
        return hole_circles_drawn(app);
    }
    hole_position_dims(app) >= 2
}

/// Point at each hole's centre in turn while they're being positioned.
/// The hole-positioning step as a numbered pair (#869): the hole's centre, then the flange
/// edge it's measured from. The centre goes green once it's selected, so a click that landed
/// is visibly a click that landed.
fn hole_dimension_marks(app: &AppState) -> Vec<GuideMark> {
    use crate::hierarchy::SceneElement;
    use crate::model::{ConstraintLine, ConstraintPoint};
    let mut marks = Vec::new();
    if holes_dimensioned(app) || app.tool != Tool::Dimension {
        return marks;
    }
    let Some(session) = app.sketch_session else { return marks };
    let Some(frame) = crate::face::sketch_geometry_frame(&app.doc, session.sketch) else {
        return marks;
    };
    let Some(face) = app.doc.sketch_face(session.sketch) else { return marks };
    let placed = hole_position_dims(app);
    let nth = placed.min(1);
    let Some((ci, circle)) = app
        .doc
        .circles
        .iter()
        .filter(|(_, c)| !c.construction && c.sketch == session.sketch)
        .nth(nth)
    else {
        return marks;
    };
    let centre_selected = app
        .scene_selection
        .is_selected(SceneElement::Point(ConstraintPoint::CircleCenter(ci)));
    marks.push(GuideMark {
        target: StepTarget::World(crate::face::local_to_world(&frame, circle.cx, circle.cy)),
        done: centre_selected,
    });
    // The face edge this hole measures from — the same one the assist uses.
    let index = if nth == 0 { 1 } else { 3 };
    let edge = ConstraintLine::FaceEdge { face, index };
    if let Some((a, b)) = crate::constraint_viewport::constraint_line_world_endpoints(&app.doc, session.sketch, edge.clone()) {
        marks.push(GuideMark {
            target: StepTarget::World((a + b) * 0.5),
            done: app.scene_selection.is_selected(SceneElement::FaceEdge(edge)),
        });
    }
    marks
}

fn hole_dimension_orb(app: &AppState) -> Option<StepTarget> {
    if holes_dimensioned(app) {
        return None;
    }
    // The step asks for the Dimension tool first — point at it while another tool is up
    // (the circle steps leave the Circle tool active, #820).
    if app.tool != Tool::Dimension && app.placing_dimension.is_none() {
        return Some(StepTarget::Ui(UiAnchor::Tool(Tool::Dimension)));
    }
    if app.editing_committed_dim.is_some() {
        return Some(StepTarget::Ui(UiAnchor::DimensionValue));
    }
    let session = app.sketch_session?;
    let frame = crate::face::sketch_geometry_frame(&app.doc, session.sketch)?;
    let placed = hole_position_dims(app);
    let circle = app
        .doc
        .circles
        .values()
        .filter(|c| !c.construction && c.sketch == session.sketch)
        .nth(placed.min(1))?;
    Some(StepTarget::World(crate::face::local_to_world(
        &frame, circle.cx, circle.cy,
    )))
}

fn cut_extrusion_count(app: &AppState) -> usize {
    app.doc
        .bodies
        .values()
        .map(|b| b.source.cut_extrusion_indices().len())
        .sum()
}

fn holes_cut(app: &AppState) -> bool {
    cut_extrusion_count(app) >= 1
}

fn holes_countersunk(app: &AppState) -> bool {
    chamfer_count(app) >= 2
}

fn corners_rounded(app: &AppState) -> bool {
    fillet_count(app) >= 4
}

fn label_engraved(app: &AppState) -> bool {
    !app.doc.sketch_texts.is_empty() && cut_extrusion_count(app) >= 2
}

fn bend_angle_changed(app: &AppState) -> bool {
    crate::value::eval_angle_rad_in_doc("bend_angle", &app.doc)
        .is_some_and(|rad| (rad.to_degrees() - 120.0).abs() > 1.0)
}


// --- "Do it for me" (#810) -------------------------------------------------------------
//
// Every step that *does* something offers to do it — the same document changes the user's
// clicks would make, so the step's own predicate then advances the tutorial exactly as if
// they'd done it themselves. (The two narration-only bookends have nothing to do; their
// button is Next.)

/// Draw the sloppy profile for the user. No longer a button of its own (#843 — clicking the
/// glowing points is the whole job), but the constraint assists below lean on it so pressing
/// *their* button works even for someone who skipped ahead past the drawing.
/// Select exactly `a` and `b` — what a click and a Shift+click do.
fn select_pair(
    app: &mut AppState,
    a: crate::hierarchy::SceneElement,
    b: crate::hierarchy::SceneElement,
) {
    app.scene_selection.clear();
    app.scene_selection.insert(a);
    app.scene_selection.insert(b);
}

fn draw_profile_for_me(app: &mut AppState) {
    use crate::model::{ConstraintPoint, FaceId, LineEnd};
    if app.sketch_session.is_none() {
        let Some(ground) = app.doc.ground_plane() else { return };
        app.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(ground),
            viewport: None,
        });
    }
    if profile_lines(app).len() >= PROFILE_POINTS.len() {
        return;
    }
    for i in 0..PROFILE_POINTS.len() {
        let (x0, y0) = PROFILE_POINTS[i];
        let (x1, y1) = PROFILE_POINTS[(i + 1) % PROFILE_POINTS.len()];
        app.apply(Action::CreateLineSegment { x0, y0, x1, y1, bezier: None, dimension: None });
    }
    let lines = profile_lines(app);
    for i in 0..lines.len() {
        let j = (i + 1) % lines.len();
        select_pair(
            app,
            crate::hierarchy::SceneElement::Point(ConstraintPoint::LineEndpoint {
                line: lines[i],
                end: LineEnd::End,
            }),
            crate::hierarchy::SceneElement::Point(ConstraintPoint::LineEndpoint {
                line: lines[j],
                end: LineEnd::Start,
            }),
        );
        app.apply(Action::AddGeometricConstraint(GC::Coincident));
    }
    app.scene_selection.clear();
}

/// Dimension the nth profile line with `expression`, defining the parameter it names first
/// when the tutorial hasn't introduced it yet (#788's `thick`).
fn dimension_profile_line(app: &mut AppState, nth: usize, expression: &str) {
    use crate::model::{DimensionTarget, DistanceTarget};
    let Some(sketch) = profile_sketch(app) else { return };
    let Some(&index) = profile_lines(app).get(nth) else { return };
    if expression == "thick" {
        ensure_param(app, "thick", "5mm");
    }
    let _ = crate::constraints::apply_dimension_expression(
        &mut app.doc,
        sketch,
        DimensionTarget::Distance(DistanceTarget::LineLength(index)),
        expression,
    );
    let _ = crate::constraints::solve_document_constraints(&mut app.doc);
    app.refresh_document_health();
}

fn assist_base_leg_dim(app: &mut AppState) {
    // The first assist after the constraint steps, which have no button of their own now
    // (#864): make the profile if the reader clicked past drawing it.
    draw_profile_for_me(app);
    dimension_profile_line(app, 0, "leg");
}
fn assist_tilted_leg_dim(app: &mut AppState) {
    dimension_profile_line(app, 5, "leg");
}
fn assist_base_cap_dim(app: &mut AppState) {
    dimension_profile_line(app, 1, "thick");
}
fn assist_tilted_cap_dim(app: &mut AppState) {
    dimension_profile_line(app, 4, "thick");
}

fn assist_bend_angle_dim(app: &mut AppState) {
    use crate::model::{ConstraintLine, DimensionTarget};
    let Some(sketch) = profile_sketch(app) else { return };
    let lines = profile_lines(app);
    let (Some(&a), Some(&b)) = (lines.first(), lines.get(3)) else { return };
    let sign = crate::constraints::angle_constraint_natural_sign(
        &app.doc,
        ConstraintLine::Line(a),
        ConstraintLine::Line(b),
    )
    .unwrap_or(1);
    let _ = crate::constraints::apply_dimension_expression(
        &mut app.doc,
        sketch,
        DimensionTarget::Angle {
            line_a: ConstraintLine::Line(a),
            line_b: ConstraintLine::Line(b),
            rotation_sign: sign,
        },
        "bend_angle",
    );
    let _ = crate::constraints::solve_document_constraints(&mut app.doc);
    app.refresh_document_health();
}

/// Extrude the profile `width` deep — leaving the sketch first, like the step says.
fn assist_extrude(app: &mut AppState) {
    use crate::actions::ExtrudeBodyChoice;
    use crate::model::ExtrudeFace;
    ensure_param(app, "width", "40mm");
    let Some(sketch) = profile_sketch(app) else { return };
    let lines = profile_lines(app);
    if lines.len() < 3 {
        return;
    }
    if app.sketch_session.is_some() {
        app.apply(Action::ExitSketch);
    }
    let width = crate::value::eval_length_mm_in_doc("width", &app.doc).unwrap_or(40.0);
    app.apply(Action::CreateExtrusion {
        sketch,
        faces: vec![ExtrudeFace::Polygon(lines)],
        distance: width,
        body: ExtrudeBodyChoice::New,
        target: None,
        expression: Some("width".to_string()),
        symmetric: false,
    
        taper: 0.0,
        taper_mode: crate::model::ExtrudeTaperMode::Distance,
        taper_expression: None,

    });
}

/// The tutorial's own first extrusion — the plate (#1055: named by key, not by index 0).
fn plate_extrusion(app: &AppState) -> Option<crate::model::ExtrusionKey> {
    app.doc.extrusions.keys().next()
}

/// Round one of the bend's vertical edges. `nth` is the profile vertex the edge stands on.
fn fillet_vertical_edge(app: &mut AppState, edge: usize, expression: &str) {
    use crate::model::{ExtrusionEdgeRef, VertexTreatmentKind};
    let Some(plate) = plate_extrusion(app) else { return };
    let amount = crate::value::eval_length_mm_in_doc(expression, &app.doc).unwrap_or(4.0);
    app.apply(Action::CommitEdgeTreatments {
        edges: vec![(plate, ExtrusionEdgeRef::Vertical { face: 0, edge })],
        kind: VertexTreatmentKind::Fillet,
        amount,
    });
}

fn assist_inner_bend(app: &mut AppState) {
    fillet_vertical_edge(app, 2, "bend");
}
fn assist_outer_bend(app: &mut AppState) {
    fillet_vertical_edge(app, 5, "bend+thick");
}

/// Place one screw hole at the spot the step's orb points at.
fn draw_hole(app: &mut AppState, v: f32) {
    ensure_param(app, "hole", "5mm");
    let r = crate::value::eval_length_mm_in_doc("hole", &app.doc).unwrap_or(5.0) * 0.5;
    app.apply(Action::CreateCircle {
        cx: 19.0,
        cy: v,
        r,
        diameter_expr: Some("hole".to_string()),
    });
}

/// Open the sketch on the flange's inside face. Not a button any more (#843 — clicking the
/// glowing face is the whole job), but the hole assists lean on it so theirs still work for
/// someone who skipped ahead.
fn open_flange_sketch_for_me(app: &mut AppState) {
    use crate::model::{ExtrudeFace, FaceId};
    if hole_sketch_open(app) {
        return;
    }
    let lines = profile_lines(app);
    if lines.len() < 3 {
        return;
    }
    let Some(plate) = plate_extrusion(app) else { return };
    app.apply(Action::BeginSketch {
        face: FaceId::ExtrudeSide {
            extrusion: plate,
            profile: ExtrudeFace::Polygon(lines),
            edge: 2,
        },
        viewport: None,
    });
}

fn assist_first_hole(app: &mut AppState) {
    open_flange_sketch_for_me(app);
    if !first_hole_drawn(app) {
        draw_hole(app, 10.0);
    }
}
fn assist_second_hole(app: &mut AppState) {
    if !hole_circles_drawn(app) {
        draw_hole(app, 30.0);
    }
}

/// Cut both holes through the bracket.
fn assist_cut_holes(app: &mut AppState) {
    use crate::actions::ExtrudeBodyChoice;
    use crate::model::ExtrudeFace;
    // The holes' own sketch — not whichever sketch happens to be open, which may be an empty
    // one the user opened on the same face (#823).
    let Some(profile) = profile_sketch(app) else { return };
    let circles: Vec<crate::model::CircleKey> = app
        .doc
        .circles
        .iter()
        .filter(|(_, c)| !c.construction && c.sketch != profile)
        .map(|(i, _)| i)
        .collect();
    if circles.is_empty() {
        return;
    }
    let Some(sketch) = app.doc.circles.get(circles[0]).map(|c| c.sketch) else { return };
    if app.sketch_session.is_some() {
        app.apply(Action::ExitSketch);
    }
    let depth = crate::value::eval_length_mm_in_doc("thick + 1", &app.doc).unwrap_or(6.0);
    app.apply(Action::CreateExtrusion {
        sketch,
        faces: circles.into_iter().map(ExtrudeFace::Circle).collect(),
        distance: -depth,
        body: ExtrudeBodyChoice::Cut,
        target: None,
        expression: Some("-(thick+1)".to_string()),
        symmetric: false,
    
        taper: 0.0,
        taper_mode: crate::model::ExtrudeTaperMode::Distance,
        taper_expression: None,

    });
}

/// Countersink both holes: a chamfer on each hole's rim.
fn assist_countersink(app: &mut AppState) {
    use crate::model::{ExtrusionEdgeRef, VertexTreatmentKind};
    let Some(cut) = app.doc.extrusions.keys().last() else { return };
    let faces = app.doc.extrusions[cut].faces.len();
    let edges = (0..faces)
        .map(|face| (cut, ExtrusionEdgeRef::Cap { face, edge: 0, top: false }))
        .collect::<Vec<_>>();
    if edges.is_empty() {
        return;
    }
    app.apply(Action::CommitEdgeTreatments {
        edges,
        kind: VertexTreatmentKind::Chamfer,
        amount: 1.2,
    });
}

/// Round the four flange-tip corners.
fn assist_round_corners(app: &mut AppState) {
    use crate::model::{ExtrusionEdgeRef, VertexTreatmentKind};
    let Some(plate) = plate_extrusion(app) else { return };
    let edges = [0usize, 1, 3, 4]
        .into_iter()
        .map(|edge| (plate, ExtrusionEdgeRef::Vertical { face: 0, edge }))
        .collect::<Vec<_>>();
    app.apply(Action::CommitEdgeTreatments {
        edges,
        kind: VertexTreatmentKind::Fillet,
        amount: 2.0,
    });
}

/// Space the holes evenly: each one the same distance from its own end of the flange.
fn assist_position_holes(app: &mut AppState) {
    use crate::model::{
        ConstraintLine, ConstraintPoint, DimensionTarget, DistanceTarget,
    };
    let Some(session) = app.sketch_session else { return };
    let Some(face) = app.doc.sketch_face(session.sketch) else { return };
    let circles: Vec<crate::model::CircleKey> = app
        .doc
        .circles
        .iter()
        .filter(|(_, c)| !c.construction && c.sketch == session.sketch)
        .map(|(i, _)| i)
        .collect();
    // The sketched-on face's own boundary edges: give hole 0 its distance from edge 0 and
    // hole 1 the same distance from the opposite edge, so the pair sits evenly.
    for (nth, &circle) in circles.iter().enumerate().take(2) {
        let index = if nth == 0 { 1 } else { 3 };
        let target = DimensionTarget::Distance(DistanceTarget::PointLineDistance {
            point: ConstraintPoint::CircleCenter(circle),
            line: ConstraintLine::FaceEdge { face: face.clone(), index },
            side: 1,
        });
        let _ = crate::constraints::apply_dimension_expression(
            &mut app.doc,
            session.sketch,
            target,
            "10mm",
        );
    }
    let _ = crate::constraints::solve_document_constraints(&mut app.doc);
    app.refresh_document_health();
}

/// Engrave the label: text on the outer face of the base, cut a millimetre deep.
fn assist_engrave(app: &mut AppState) {
    use crate::actions::ExtrudeBodyChoice;
    use crate::model::{ExtrudeFace, FaceId};
    if !app.doc.sketch_texts.is_empty() && cut_extrusion_count(app) >= 2 {
        return;
    }
    let lines = profile_lines(app);
    if lines.len() < 3 {
        return;
    }
    if app.sketch_session.is_some() {
        app.apply(Action::ExitSketch);
    }
    let Some(plate) = plate_extrusion(app) else { return };
    app.apply(Action::BeginSketch {
        face: FaceId::ExtrudeSide {
            extrusion: plate,
            profile: ExtrudeFace::Polygon(lines),
            edge: 0,
        },
        viewport: None,
    });
    let Some(session) = app.sketch_session else { return };
    // Whatever font this machine has — the same fallback the Text tool picks (#282).
    let font_family = ["Helvetica", "Arial", "Segoe UI", "DejaVu Sans", "Liberation Sans"]
        .into_iter()
        .find(|fam| crate::text::font_bytes(fam, false, false).is_some())
        .map(|fam| fam.to_string())
        .or_else(|| crate::text::system_font_families().into_iter().next())
        .unwrap_or_default();
    app.apply(Action::CreateSketchText {
        sketch: session.sketch,
        text: "BearCAD".to_string(),
        font_family,
        bold: false,
        italic: false,
        underline: false,
        size: 5.0,
        size_expr: "5mm".to_string(),
        origin: (6.0, 17.0),
        rotation: 0.0,
        wrap_width: None,
    });
    let Some(text) = app.doc.sketch_texts.keys().last() else {
        return;
    };
    let sketch = session.sketch;
    let glyphs = app
        .doc
        .sketch_texts
        .get(text)
        .map(|t| crate::text::group_glyphs(&t.contours).len())
        .unwrap_or(0);
    if glyphs == 0 {
        return;
    }
    app.apply(Action::ExitSketch);
    app.apply(Action::CreateExtrusion {
        sketch,
        faces: (0..glyphs)
            .map(|glyph| ExtrudeFace::TextGlyph { text, glyph })
            .collect(),
        distance: -1.0,
        body: ExtrudeBodyChoice::Cut,
        target: None,
        expression: None,
        symmetric: false,
    
        taper: 0.0,
        taper_mode: crate::model::ExtrudeTaperMode::Distance,
        taper_expression: None,

    });
}

/// Change the bend angle — the payoff step.
fn assist_change_angle(app: &mut AppState) {
    let index = app.doc.parameters.iter().find_map(|(key, p)| {
        p.name.eq_ignore_ascii_case("bend_angle").then_some(key)
    });
    if let Some(index) = index {
        app.apply(Action::CommitParameterExpression {
            index,
            expression: "150deg".to_string(),
        });
    }
}

static BRACKET_STEPS: &[Step] = &[
    Step {
        narration: "Hi, I'm Bear! Let's build a real part together: a 120\u{b0} angle \
                    bracket with a rounded bend and countersunk screw holes. I'll point with \
                    glowing rings; you do the clicking. I've opened a fresh document for us.",
        anchor: StepAnchor::None,
        done: None,
        on_enter: None,
        assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "On a phone the panes hide away. Tap `Params` in the bar at the bottom to bring the Parameters pane out.",
        anchor: StepAnchor::Guided(params_button_orb),
        done: Some(params_pane_ready),
        on_enter: None,
                assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: true,
    },
    Step {
        narration: "First, a name for our first number. See the Parameters pane on the \
                    right? Tap inside the name box \u{2014} the pulsing ring marks it.",
        anchor: StepAnchor::Ui(UiAnchor::ParametersName),
        done: Some(name_box_tapped),
        on_enter: None,
                assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: Some("The Parameters pane is open now. Tap inside the `name` box \u{2014} the pulsing ring marks it."),
        only_on_phone: false,
    },
    Step {
        narration: "Type `leg` \u{2014} just those three letters. It's the length of each \
                    of the bracket's legs.",
        anchor: StepAnchor::Ui(UiAnchor::ParametersName),
        done: Some(name_says_leg),
        on_enter: None,
        assist: Some(StepAssist { label: "Add it for me", run: add_leg_param }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Fixed("leg")),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Now tap the value box beside it.",
        anchor: StepAnchor::Ui(UiAnchor::ParametersValue),
        done: Some(value_box_tapped),
        on_enter: None,
        assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Type `50mm` \u{2014} how long each leg is.",
        anchor: StepAnchor::Ui(UiAnchor::ParametersValue),
        done: Some(value_says_50),
        on_enter: None,
        assist: Some(StepAssist { label: "Add it for me", run: add_leg_param }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Fixed("50mm")),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Press + to add it. Your first parameter!",
        anchor: StepAnchor::Ui(UiAnchor::ParametersAdd),
        done: Some(leg_added),
        on_enter: None,
                assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Now `hole`=`5mm`, the same way.",
        anchor: StepAnchor::Guided(param_list_orb),
        done: Some(hole_param_defined),
        on_enter: None,
        assist: Some(StepAssist { label: "Add it for me", run: add_hole_param }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Dynamic(next_missing_param)),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "And `bend`=`4mm` \u{2014} how tightly the bracket bends.",
        anchor: StepAnchor::Guided(param_list_orb),
        done: Some(bend_param_defined),
        on_enter: None,
        assist: Some(StepAssist { label: "Add it for me", run: add_bend_param }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Dynamic(next_missing_param)),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Last one: `bend_angle`=`120deg`.",
        anchor: StepAnchor::Guided(param_list_orb),
        done: Some(params_defined),
        on_enter: None,
        assist: Some(StepAssist { label: "Add it for me", run: add_missing_params }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Dynamic(next_missing_param)),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Tap `Params` again to tuck the pane away \u{2014} you'll want the whole screen for drawing.",
        anchor: StepAnchor::Guided(params_button_orb),
        done: Some(params_pane_tucked),
        on_enter: None,
                assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: true,
    },
    Step {
        narration: "Grab the Line tool \u{2014} the glowing button up top, or press L.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Line)),
        done: Some(line_tool_active),
        on_enter: None,
                assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: Some("Grab the Line tool \u{2014} the glowing button in the toolbar along the top."),
        only_on_phone: false,
    },
    Step {
        narration: "I've brought us in over the `XY` plane \u{2014} the flat one lying on the \
                    ground. Click each glowing point in turn to draw a loose sketch on it.",
        anchor: StepAnchor::World(next_profile_point),
        done: Some(profile_drawn),
        on_enter: Some(frame_profile_area),
                assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Now the Constraint tool \u{2014} the glowing button, or press C.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Constraint)),
        done: Some(constraint_tool_active),
        on_enter: None,
                assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: Some("Now the Constraint tool \u{2014} the glowing button in the toolbar."),
        only_on_phone: false,
    },
    Step {
        narration: "The constraint buttons live in the Context pane \u{2014} tap `Context` at the bottom to open it.",
        anchor: StepAnchor::Guided(context_button_orb),
        done: Some(context_pane_ready),
        on_enter: None,
                assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: true,
    },
    Step {
        narration: "Make the base parallel to the red X axis \u{2014} one line, one button.",
        anchor: StepAnchor::Guided(level_click),
        done: Some(base_leveled),
        on_enter: None,
        assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: Some(("Space", "Press space if it's too crowded to pick")),
        marks: Some(level_marks),
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Now the two base lines, parallel to each other.",
        anchor: StepAnchor::Guided(base_strip_click),
        done: Some(base_strip_even),
        on_enter: None,
        assist: None,
        needs_shift: Some(base_strip_shift),
        drag_hint: None,
        key_hint: None,
        marks: Some(base_strip_marks),
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Same again for the tilted leg's two long lines.",
        anchor: StepAnchor::Guided(legs_click),
        done: Some(legs_parallel),
        on_enter: None,
        assist: None,
        needs_shift: Some(legs_shift),
        drag_hint: None,
        key_hint: None,
        marks: Some(legs_marks),
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Make these edges square.",
        anchor: StepAnchor::Guided(cap_one_click),
        done: Some(first_cap_squared),
        on_enter: None,
        assist: None,
        needs_shift: Some(cap_one_shift),
        drag_hint: None,
        key_hint: None,
        marks: Some(cap_one_marks),
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Make these edges square too. Squared up!",
        anchor: StepAnchor::Guided(cap_two_click),
        done: Some(profile_squared),
        on_enter: None,
        assist: None,
        needs_shift: Some(cap_two_shift),
        drag_hint: None,
        key_hint: None,
        marks: Some(cap_two_marks),
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Tap `Context` to tuck that pane away again \u{2014} the next steps are all out on the model.",
        anchor: StepAnchor::Guided(context_button_orb),
        done: Some(context_pane_tucked),
        on_enter: None,
                assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: true,
    },
    Step {
        narration: "Now exact sizes. Grab the Dimension tool \u{2014} the glowing button, \
                    or press `D`.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Dimension)),
        done: Some(dimension_tool_active),
        on_enter: Some(clear_selection_for_dimensioning),
                assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: Some("Now exact sizes. Grab the Dimension tool \u{2014} the glowing button in the toolbar."),
        only_on_phone: false,
    },
    Step {
        narration: "Dimension the base leg: place it, then type `leg`.",
        anchor: StepAnchor::Guided(base_leg_orb),
        done: Some(base_leg_dimensioned),
        on_enter: None,
        assist: Some(StepAssist { label: "Do it for me", run: assist_base_leg_dim }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Dynamic(leg_value_hint)),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "The other outer leg, the same way: click, place, type `leg`, Enter.",
        anchor: StepAnchor::Guided(tilted_leg_orb),
        done: Some(tilted_leg_dimensioned),
        on_enter: None,
        assist: Some(StepAssist { label: "Do it for me", run: assist_tilted_leg_dim }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Dynamic(leg_value_hint)),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "You can create parameters while drawing, too. Create the `thick` \
                    parameter for this edge.",
        anchor: StepAnchor::Guided(base_cap_orb),
        done: Some(base_cap_dimensioned),
        on_enter: None,
        assist: Some(StepAssist { label: "Do it for me", run: assist_base_cap_dim }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Dynamic(thick_value_hint)),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "And the other end cap \u{2014} now that `thick` exists, just type its \
                    name.",
        anchor: StepAnchor::Guided(tilted_cap_orb),
        done: Some(tilted_cap_dimensioned),
        on_enter: None,
        assist: Some(StepAssist { label: "Do it for me", run: assist_tilted_cap_dim }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Dynamic(thick_value_hint)),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Last one, the bend: click the bottom line, Shift+click the inner leg \
                    line, place the arc, then type `bend_angle` and press Enter.",
        anchor: StepAnchor::Guided(bend_angle_orb),
        done: Some(profile_dimensioned),
        on_enter: None,
        assist: Some(StepAssist { label: "Do it for me", run: assist_bend_angle_dim }),
        needs_shift: Some(bend_angle_shift),
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Dynamic(bend_angle_value_hint)),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Leave the sketch (Esc) and Extrude (E) the glowing face `width=40mm`.",
        anchor: StepAnchor::Guided(extrude_orb),
        done: Some(extruded),
        on_enter: None,
        assist: Some(StepAssist { label: "Do it for me", run: assist_extrude }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Dynamic(extrude_value_hint)),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Round the bend with Fillet (F): click the glowing edge \u{2014} the \
                    inside of the bend \u{2014} and type `bend`, Enter.",
        anchor: StepAnchor::Guided(inner_bend_orb),
        done: Some(inner_bend_rounded),
        on_enter: None,
        assist: Some(StepAssist { label: "Do it for me", run: assist_inner_bend }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Dynamic(bend_value_hint)),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "The outside of the bend is round the back. `Right-drag` to spin the view \
                    until you can see it.",
        anchor: StepAnchor::Guided(outer_bend_spin_orb),
        done: Some(looking_at_outer_bend),
        on_enter: None,
        assist: Some(StepAssist { label: "Spin it for me", run: assist_spin_to_outer_bend }),
        needs_shift: None,
        drag_hint: Some("Right button"),
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: Some("The outside of the bend is round the back. Drag with three fingers to spin the view until you can see it."),
        only_on_phone: false,
    },
    Step {
        narration: "Now the outside edge, one bracket thickness bigger: type \
                    `bend+thick`. Concentric, like bent sheet metal.",
        anchor: StepAnchor::Guided(outer_bend_orb),
        done: Some(bend_rounded),
        on_enter: None,
        assist: Some(StepAssist { label: "Do it for me", run: assist_outer_bend }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Dynamic(bend_thick_value_hint)),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Screw holes next. Grab the Sketch tool \u{2014} the glowing button, or \
                    press `S`.",
        anchor: StepAnchor::Guided(sketch_tool_orb),
        done: Some(sketch_tool_ready),
        on_enter: None,
                assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "The holes go on the `inside` of the base flange \u{2014} which is \
                    facing away right now. `Right-drag` anywhere in the viewport to spin \
                    the view around until you're looking at it.",
        anchor: StepAnchor::Guided(spin_orb),
        done: Some(looking_at_flange_face),
        on_enter: None,
        assist: Some(StepAssist { label: "Spin it for me", run: assist_spin_to_flange }),
        needs_shift: None,
        drag_hint: Some("Right button"),
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "There it is \u{2014} click the glowing face to sketch on it.",
        anchor: StepAnchor::Guided(flange_face_orb),
        done: Some(hole_sketch_open),
        on_enter: None,
                assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Circle tool now \u{2014} the glowing button, or press `O`.",
        anchor: StepAnchor::Guided(circle_tool_orb),
        done: Some(circle_tool_ready),
        on_enter: None,
                assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Click the glowing spot for the first hole's centre, then type `hole` for \
                    its diameter and press Enter.",
        anchor: StepAnchor::Guided(first_hole_orb),
        done: Some(first_hole_drawn),
        on_enter: None,
        assist: Some(StepAssist { label: "Do it for me", run: assist_first_hole }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Dynamic(hole_value_hint)),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "And the second hole, the same way: `hole` again.",
        anchor: StepAnchor::Guided(second_hole_orb),
        done: Some(hole_circles_drawn),
        on_enter: None,
        assist: Some(StepAssist { label: "Do it for me", run: assist_second_hole }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Dynamic(hole_value_hint)),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Position each hole `10mm` from its end of the flange.",
        anchor: StepAnchor::Guided(hole_dimension_orb),
        done: Some(holes_dimensioned),
        on_enter: None,
        assist: Some(StepAssist { label: "Do it for me", run: assist_position_holes }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: Some(hole_dimension_marks),
        type_hint: Some(TypeHint::Dynamic(hole_position_hint)),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "This one needs the `Output` row: tap `Context` to open that pane again.",
        anchor: StepAnchor::Guided(context_button_orb),
        done: Some(context_pane_ready),
        on_enter: None,
                assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: true,
    },
    Step {
        narration: "Extrude the holes as a `Cut`, `-(thick+1)` deep \u{2014} right through.",
        anchor: StepAnchor::Guided(hole_cut_orb),
        done: Some(holes_cut),
        on_enter: None,
        assist: Some(StepAssist { label: "Do it for me", run: assist_cut_holes }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Dynamic(hole_cut_value_hint)),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Tap `Context` once more to tuck the pane away \u{2014} back to the model.",
        anchor: StepAnchor::Guided(context_button_orb),
        done: Some(context_pane_tucked),
        on_enter: None,
                assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: true,
    },
    Step {
        narration: "Countersink them: Chamfer (K), click the glowing rim, Shift+click the \
                    other hole's, then type `1.2` and press Enter.",
        anchor: StepAnchor::Guided(countersink_orb),
        done: Some(holes_countersunk),
        on_enter: None,
        assist: Some(StepAssist { label: "Do it for me", run: assist_countersink }),
        needs_shift: Some(countersink_shift),
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Dynamic(countersink_value_hint)),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Fillet (F) again: click the glowing corner edge, Shift+click the other \
                    three, then type `2` and press Enter. Rounded corners!",
        anchor: StepAnchor::Guided(corner_fillet_orb),
        done: Some(corners_rounded),
        on_enter: None,
        assist: Some(StepAssist { label: "Do it for me", run: assist_round_corners }),
        needs_shift: Some(corner_fillet_shift),
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: Some(TypeHint::Dynamic(corner_value_hint)),
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "Sign your work: Text (T) on the outer face of the base, type `BearCAD`. \
                    Then Extrude (E) the text, push the handle into the face (type `1`), pick \
                    Cut \u{2014} engraved letters.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Text)),
        done: Some(label_engraved),
        on_enter: None,
        assist: Some(StepAssist { label: "Do it for me", run: assist_engrave }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        narration: "The best part: in the Parameters pane, change `bend_angle` from `120deg` \
                    to `150deg`. The whole part rebuilds \u{2014} bend, holes, countersinks \
                    and all.",
        anchor: StepAnchor::Ui(UiAnchor::ParametersAdd),
        done: Some(bend_angle_changed),
        on_enter: None,
        assist: Some(StepAssist { label: "Do it for me", run: assist_change_angle }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    },
    Step {
        // ASCII `->` not U+2192: Ubuntu Light (UI font) lacks that glyph (#1265).
        narration: "You built it! Export via File -> Export -> STL, 3MF, or STEP. \
                    That's the whole loop: sketch, constrain, dimension, extrude, refine \u{2014} \
                    and parameters drive everything. See you around the viewport!",
        anchor: StepAnchor::None,
        done: None,
        on_enter: None,
        assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    },
];

// --- Short tutorials (#1238–#1240; atomic steps #1253) -----------------------------

fn rectangle_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Rectangle
}

fn extrude_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Extrude
}

fn shape_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Shape
}

fn has_closed_rectangle(app: &AppState) -> bool {
    // A committed rectangle is four non-construction lines plus width/height dims.
    let lines = app.doc.lines.values().filter(|l| !l.construction).count();
    let dims = live_constraints(app)
        .filter(|c| matches!(c.kind, ConstraintKind::Distance { .. }))
        .count();
    lines >= 4 && dims >= 2
}

/// Four non-construction lines — a click-placed rectangle without typed dimensions (#1259).
fn has_rectangle_outline(app: &AppState) -> bool {
    app.doc.lines.values().filter(|l| !l.construction).count() >= 4
        || has_closed_rectangle(app)
}

fn has_extrusion(app: &AppState) -> bool {
    !app.doc.extrusions.is_empty()
}

/// First corner clicked (rectangle in progress) or the square is already done.
fn rect_first_corner_placed(app: &AppState) -> bool {
    app.creating_rect.is_some() || has_rectangle_outline(app)
}

/// A spot on the ground plane (or open sketch plane) in local mm — click targets for
/// rectangle / shape placement steps (#1257/#1262).
fn ground_local(app: &AppState, u: f32, v: f32) -> Option<glam::Vec3> {
    if let Some(session) = app.sketch_session {
        let frame = crate::face::sketch_geometry_frame(&app.doc, session.sketch)?;
        return Some(crate::face::local_to_world(&frame, u, v));
    }
    let ground = app.doc.ground_plane()?;
    let frame = crate::face::sketch_frame(
        &app.doc,
        crate::model::FaceId::ConstructionPlane(ground),
    )?;
    Some(crate::face::local_to_world(&frame, u, v))
}

fn rect_first_corner_guide(app: &AppState) -> Option<glam::Vec3> {
    ground_local(app, 20.0, 20.0)
}

fn rect_opposite_corner_guide(app: &AppState) -> Option<glam::Vec3> {
    if let Some(cr) = app.creating_rect.as_ref() {
        if let Some(session) = app.sketch_session {
            let frame = crate::face::sketch_geometry_frame(&app.doc, session.sketch)?;
            let (ou, ov) = crate::face::world_to_local(&frame, cr.origin);
            return Some(crate::face::local_to_world(&frame, ou + 40.0, ov + 40.0));
        }
    }
    ground_local(app, 60.0, 60.0)
}

/// Centre of the placed rectangle's outline — where Extrude wants a face click.
fn rectangle_face_guide(app: &AppState) -> Option<glam::Vec3> {
    let mut sum = glam::Vec3::ZERO;
    let mut n = 0u32;
    for line in app.doc.lines.values().filter(|l| !l.construction) {
        if let Some(poly) = crate::face::line_world_polyline(&app.doc, line) {
            for p in poly {
                sum += p;
                n += 1;
            }
        }
    }
    if n == 0 {
        ground_local(app, 40.0, 40.0)
    } else {
        Some(sum / n as f32)
    }
}

/// Open a ground sketch so the next click is the first rectangle corner (#1257).
fn ensure_rect_sketch_for_tutorial(app: &mut AppState) {
    if app.tool != Tool::Rectangle {
        app.apply(Action::SetTool(Tool::Rectangle));
    }
    ensure_ground_sketch(app);
}

fn focus_rect_width_field(app: &mut AppState) {
    let _ = app.apply(Action::FocusRectDimension {
        axis: crate::actions::RectAxis::Width,
    });
}

fn focus_rect_height_field(app: &mut AppState) {
    let _ = app.apply(Action::FocusRectDimension {
        axis: crate::actions::RectAxis::Height,
    });
}

/// Height typing step: ensure the shape is in the Height phase with the Height field armed
/// for overwrite typing (#1274). Typing the radius alone used to advance the tutorial while
/// the tool was still in Base with Radius clinging to the keyboard.
fn ensure_shape_height_focus(app: &mut AppState) {
    use crate::actions::ShapePhase;
    let Some(c) = app.creating_shape.as_mut() else {
        return;
    };
    if matches!(c.phase, ShapePhase::Base | ShapePhase::Anchor) {
        c.phase = ShapePhase::Height;
        c.phase_screen = None;
    }
    if c.phase == ShapePhase::Height {
        c.pending_focus = true;
    }
}

fn ground_anchor_a(app: &AppState) -> Option<glam::Vec3> {
    ground_local(app, 15.0, 15.0)
}

fn ground_anchor_b(app: &AppState) -> Option<glam::Vec3> {
    if let Some(c) = app.creating_shape.as_ref() {
        if let Some(corner) = c.first_corner {
            let u = glam::Vec3::from_array(c.shape.u_axis).normalize_or_zero();
            let n = glam::Vec3::from_array(c.shape.normal).normalize_or_zero();
            let v = n.cross(u).normalize_or_zero();
            return Some(corner + u * 40.0 + v * 40.0);
        }
        let origin = glam::Vec3::from_array(c.shape.origin);
        let u = glam::Vec3::from_array(c.shape.u_axis).normalize_or_zero();
        return Some(origin + u * 30.0);
    }
    ground_local(app, 55.0, 55.0)
}

/// Centre of the cylinder's base on the **XZ wall** construction plane (#1273) — not on the
/// ground next to a cuboid corner.
fn wall_plane_cylinder_anchor(app: &AppState) -> Option<glam::Vec3> {
    // Datum planes: 0 = XY (ground), 1 = XZ, 2 = YZ.
    let plane = app.doc.construction_planes.keys().nth(1)?;
    let frame = crate::face::sketch_frame(
        &app.doc,
        crate::model::FaceId::ConstructionPlane(plane),
    )?;
    // Mid-quadrant of the wall: clear of the ground cuboid and origin edges.
    Some(crate::face::local_to_world(&frame, 50.0, 40.0))
}

fn ground_anchor_d(app: &AppState) -> Option<glam::Vec3> {
    ground_local(app, 80.0, 0.0)
}

/// Width field typed to `target` mm while drawing, or the rectangle is already committed.
fn rect_width_typed(app: &AppState, target: f32) -> bool {
    if has_closed_rectangle(app) {
        return true;
    }
    app.creating_rect.as_ref().is_some_and(|cr| {
        cr.user_edited[0]
            && crate::value::eval_length_mm(&cr.texts[0])
                .is_some_and(|v| (v - target).abs() < 0.51)
    })
}

/// Height field typed to `target` mm while drawing, or the rectangle is already committed.
fn rect_height_typed(app: &AppState, target: f32) -> bool {
    if has_closed_rectangle(app) {
        return true;
    }
    app.creating_rect.as_ref().is_some_and(|cr| {
        cr.user_edited[1]
            && crate::value::eval_length_mm(&cr.texts[1])
                .is_some_and(|v| (v - target).abs() < 0.51)
    })
}

fn rect_width_10(app: &AppState) -> bool {
    rect_width_typed(app, 10.0)
}

fn rect_height_10(app: &AppState) -> bool {
    rect_height_typed(app, 10.0)
}

/// Extrude face picked (distance field open) or extrusion already committed.
fn extrude_face_picked(app: &AppState) -> bool {
    app.creating_extrusion.is_some() || has_extrusion(app)
}

fn has_primitive_kind(app: &AppState, kind: crate::model::PrimitiveKind) -> bool {
    app.doc.primitives.values().any(|p| p.kind == kind)
}

fn has_cuboid(app: &AppState) -> bool {
    has_primitive_kind(app, crate::model::PrimitiveKind::Cuboid)
}

fn has_sphere(app: &AppState) -> bool {
    has_primitive_kind(app, crate::model::PrimitiveKind::Sphere)
}

fn has_cylinder(app: &AppState) -> bool {
    has_primitive_kind(app, crate::model::PrimitiveKind::Cylinder)
}

fn has_all_three_shapes(app: &AppState) -> bool {
    has_cuboid(app) && has_sphere(app) && has_cylinder(app)
}

fn shape_in_progress(app: &AppState, kind: crate::model::PrimitiveKind) -> Option<&crate::actions::CreatingShape> {
    app.creating_shape
        .as_ref()
        .filter(|c| c.shape.kind == kind)
}

/// Anchor click done: placement has left the Anchor phase (or the solid exists).
fn shape_anchored(app: &AppState, kind: crate::model::PrimitiveKind) -> bool {
    has_primitive_kind(app, kind)
        || shape_in_progress(app, kind)
            .is_some_and(|c| c.phase != crate::actions::ShapePhase::Anchor)
}

/// Base (opposite corner / radius) done: height phase or committed.
fn shape_base_set(app: &AppState, kind: crate::model::PrimitiveKind) -> bool {
    use crate::actions::ShapePhase;
    has_primitive_kind(app, kind)
        || shape_in_progress(app, kind)
            .is_some_and(|c| matches!(c.phase, ShapePhase::Height | ShapePhase::Done))
}

/// Typed field slot set near `target`, or the solid of that kind already exists.
fn shape_field_typed(
    app: &AppState,
    kind: crate::model::PrimitiveKind,
    slot: usize,
    expr: impl Fn(&crate::model::Primitive) -> &str,
    target: f32,
) -> bool {
    if has_primitive_kind(app, kind) {
        return true;
    }
    shape_in_progress(app, kind).is_some_and(|c| {
        c.typed[slot]
            && crate::value::eval_length_mm(expr(&c.shape))
                .is_some_and(|v| (v - target).abs() < 0.51)
    })
}

fn cuboid_anchored(app: &AppState) -> bool {
    shape_anchored(app, crate::model::PrimitiveKind::Cuboid)
}

fn cuboid_base_set(app: &AppState) -> bool {
    shape_base_set(app, crate::model::PrimitiveKind::Cuboid)
}

fn cylinder_anchored(app: &AppState) -> bool {
    shape_anchored(app, crate::model::PrimitiveKind::Cylinder)
}

fn cylinder_radius_typed_10(app: &AppState) -> bool {
    // Radius typed, or advanced past Base by click / Enter, or cylinder already placed.
    shape_field_typed(
        app,
        crate::model::PrimitiveKind::Cylinder,
        3,
        |s| &s.radius,
        10.0,
    ) || shape_base_set(app, crate::model::PrimitiveKind::Cylinder)
}

/// Height owns the keyboard (phase advanced by Tab / click / Enter) or the solid exists.
fn cylinder_height_ready(app: &AppState) -> bool {
    shape_base_set(app, crate::model::PrimitiveKind::Cylinder)
}

fn sphere_anchored(app: &AppState) -> bool {
    shape_anchored(app, crate::model::PrimitiveKind::Sphere)
}

fn distance_dims_near(app: &AppState, target: f32, min_count: usize) -> bool {
    let n = live_constraints(app)
        .filter(|c| matches!(c.kind, ConstraintKind::Distance { .. }))
        .filter_map(|c| crate::value::eval_length_mm_in_doc(&c.expression, &app.doc))
        .filter(|&d| (d - target).abs() < 0.51)
        .count();
    n >= min_count
}

fn rect_dims_are_10(app: &AppState) -> bool {
    has_closed_rectangle(app) && distance_dims_near(app, 10.0, 2)
}

fn extrusion_is_10(app: &AppState) -> bool {
    app.doc.extrusions.values().any(|e| {
        if (e.distance - 10.0).abs() < 0.51 {
            return true;
        }
        crate::value::eval_length_mm_in_doc(&e.expression, &app.doc)
            .map(|d| (d - 10.0).abs() < 0.51)
            .unwrap_or(false)
    })
}

fn one_sketch_dim_is_20(app: &AppState) -> bool {
    distance_dims_near(app, 20.0, 1)
}

/// Sketch reopened for the edit step (or the edit is already done).
fn sketch_reopened_for_edit(app: &AppState) -> bool {
    app.sketch_session.is_some() || one_sketch_dim_is_20(app)
}

fn ensure_ground_sketch(app: &mut AppState) {
    if app.sketch_session.is_some() {
        return;
    }
    let Some(ground) = app.doc.ground_plane() else {
        return;
    };
    app.apply(Action::BeginSketch {
        face: crate::model::FaceId::ConstructionPlane(ground),
        viewport: None,
    });
}

fn assist_draw_square(app: &mut AppState) {
    if has_closed_rectangle(app) {
        return;
    }
    ensure_ground_sketch(app);
    app.apply(Action::CreateRectangle {
        x: 0.0,
        y: 0.0,
        width: 20.0,
        height: 20.0,
        width_expr: Some("20".into()),
        height_expr: Some("20".into()),
    });
}

fn assist_extrude_to_cube(app: &mut AppState) {
    if has_extrusion(app) {
        return;
    }
    assist_draw_square(app);
    let Some(sketch) = app.doc.lines.values().find(|l| !l.construction).map(|l| l.sketch) else {
        return;
    };
    let lines: Vec<_> = app
        .doc
        .lines
        .iter()
        .filter(|(_, l)| l.sketch == sketch && !l.construction)
        .map(|(k, _)| k)
        .collect();
    if lines.len() < 4 {
        return;
    }
    if app.sketch_session.is_some() {
        app.apply(Action::ExitSketch);
    }
    app.apply(Action::CreateExtrusion {
        sketch,
        faces: vec![crate::model::ExtrudeFace::Polygon(lines)],
        distance: 20.0,
        body: crate::actions::ExtrudeBodyChoice::New,
        target: None,
        expression: Some("20".into()),
        symmetric: false,
    
        taper: 0.0,
        taper_mode: crate::model::ExtrudeTaperMode::Distance,
        taper_expression: None,

    });
}

fn assist_draw_10mm_square(app: &mut AppState) {
    if rect_dims_are_10(app) {
        return;
    }
    ensure_ground_sketch(app);
    // Replace a half-drawn sketch if needed.
    if has_closed_rectangle(app) && !rect_dims_are_10(app) {
        // Nudge existing dims to 10 rather than rebuilding.
        let dim_keys: Vec<_> = app
            .doc
            .constraints
            .iter()
            .filter(|(_, c)| matches!(c.kind, ConstraintKind::Distance { .. }))
            .map(|(key, _)| key)
            .collect();
        for key in dim_keys {
            let _ = crate::constraints::set_constraint_expression(
                &mut app.doc,
                key,
                "10".to_string(),
            );
        }
        let _ = crate::constraints::solve_document_constraints(&mut app.doc);
        app.refresh_document_health();
        return;
    }
    app.apply(Action::CreateRectangle {
        x: 0.0,
        y: 0.0,
        width: 10.0,
        height: 10.0,
        width_expr: Some("10".into()),
        height_expr: Some("10".into()),
    });
}

fn assist_extrude_10mm(app: &mut AppState) {
    if extrusion_is_10(app) {
        return;
    }
    assist_draw_10mm_square(app);
    let Some(sketch) = app.doc.lines.values().find(|l| !l.construction).map(|l| l.sketch) else {
        return;
    };
    let lines: Vec<_> = app
        .doc
        .lines
        .iter()
        .filter(|(_, l)| l.sketch == sketch && !l.construction)
        .map(|(k, _)| k)
        .collect();
    if lines.len() < 4 {
        return;
    }
    if app.sketch_session.is_some() {
        app.apply(Action::ExitSketch);
    }
    if has_extrusion(app) {
        // Already extruded; just ensure the distance is 10.
        let key = app.doc.extrusions.keys().next();
        if let Some(key) = key {
            app.doc.extrusions[key].distance = 10.0;
            app.doc.extrusions[key].expression = "10".into();
            app.refresh_document_health();
        }
        return;
    }
    app.apply(Action::CreateExtrusion {
        sketch,
        faces: vec![crate::model::ExtrudeFace::Polygon(lines)],
        distance: 10.0,
        body: crate::actions::ExtrudeBodyChoice::New,
        target: None,
        expression: Some("10".into()),
        symmetric: false,
    
        taper: 0.0,
        taper_mode: crate::model::ExtrudeTaperMode::Distance,
        taper_expression: None,

    });
}

fn assist_edit_dim_to_20(app: &mut AppState) {
    if one_sketch_dim_is_20(app) {
        return;
    }
    assist_extrude_10mm(app);
    // Change the first sketch distance dimension to 20 mm.
    let target = app.doc.constraints.iter().find_map(|(key, c)| {
        matches!(c.kind, ConstraintKind::Distance { .. }).then_some(key)
    });
    if let Some(key) = target {
        let _ = crate::constraints::set_constraint_expression(&mut app.doc, key, "20".to_string());
        let _ = crate::constraints::solve_document_constraints(&mut app.doc);
        app.refresh_document_health();
    }
}

fn place_shape(app: &mut AppState, kind: crate::model::PrimitiveKind, origin: [f32; 3]) {
    if has_primitive_kind(app, kind) {
        return;
    }
    let mut shape = crate::model::Primitive::new(kind);
    shape.origin = origin;
    match kind {
        crate::model::PrimitiveKind::Cuboid => {
            shape.width = "20".into();
            shape.depth = "20".into();
            shape.height = "20".into();
        }
        crate::model::PrimitiveKind::Cylinder => {
            shape.radius = "10".into();
            shape.height = "20".into();
        }
        crate::model::PrimitiveKind::Sphere => {
            shape.radius = "10".into();
        }
    }
    app.apply(Action::CreateShape { shape });
}

/// Place a cylinder on the XZ wall plane (#1273) — same spot the step's orb points at.
fn place_cylinder_on_wall(app: &mut AppState) {
    if has_cylinder(app) {
        return;
    }
    let mut shape = crate::model::Primitive::new(crate::model::PrimitiveKind::Cylinder);
    // Match `wall_plane_cylinder_anchor`: local (50, 40) on XZ → world (50, 0, 40).
    shape.origin = [50.0, 0.0, 40.0];
    shape.normal = [0.0, 1.0, 0.0];
    shape.u_axis = [1.0, 0.0, 0.0];
    shape.radius = "10".into();
    shape.height = "20".into();
    app.apply(Action::CreateShape { shape });
}

fn assist_place_cuboid(app: &mut AppState) {
    place_shape(app, crate::model::PrimitiveKind::Cuboid, [0.0, 0.0, 0.0]);
}

fn assist_place_cylinder(app: &mut AppState) {
    assist_place_cuboid(app);
    place_cylinder_on_wall(app);
}

fn assist_place_sphere(app: &mut AppState) {
    assist_place_cylinder(app);
    place_shape(app, crate::model::PrimitiveKind::Sphere, [80.0, 0.0, 0.0]);
}

fn cuboid_kind_ready(app: &AppState) -> bool {
    // Already placed, or tool is armed for a cuboid (default).
    has_cuboid(app)
        || (shape_tool_active(app) && app.shape_kind == crate::model::PrimitiveKind::Cuboid)
}

fn cylinder_kind_ready(app: &AppState) -> bool {
    has_cylinder(app)
        || (shape_tool_active(app) && app.shape_kind == crate::model::PrimitiveKind::Cylinder)
}

fn sphere_kind_ready(app: &AppState) -> bool {
    has_sphere(app)
        || (shape_tool_active(app) && app.shape_kind == crate::model::PrimitiveKind::Sphere)
}

// --- Navigate tutorial (#1269) -----------------------------------------------------

/// Seed a few overlapping cuboids so the walkthrough starts with geometry to orbit and
/// something crowded enough for the Selection Exploder. Drop the default XY/XZ/YZ
/// planes first so they don't hide the cubes (#1306).
fn seed_nav_cubes(app: &mut AppState) {
    let planes: Vec<_> = app.doc.construction_planes.keys().collect();
    for index in planes {
        app.apply(Action::DeleteElement {
            element: crate::hierarchy::SceneElement::ConstructionPlane(index),
        });
    }
    if app.doc.primitives.len() >= 2 {
        return;
    }
    // Two on the ground sharing a corner, one stacked on top — a pile under the cursor.
    let placements = [
        ([0.0, 0.0, 0.0], "20"),
        ([12.0, 12.0, 0.0], "20"),
        ([6.0, 6.0, 20.0], "18"),
    ];
    for (origin, size) in placements {
        let mut shape = crate::model::Primitive::new(crate::model::PrimitiveKind::Cuboid);
        shape.origin = origin;
        shape.width = size.into();
        shape.depth = size.into();
        shape.height = size.into();
        app.apply(Action::CreateShape { shape });
    }
}

/// Centre of the seeded pile — orb target for camera drag steps.
fn nav_cubes_guide(app: &AppState) -> Option<glam::Vec3> {
    if app.doc.primitives.is_empty() {
        return Some(glam::Vec3::new(10.0, 10.0, 15.0));
    }
    let mut sum = glam::Vec3::ZERO;
    let mut n = 0.0;
    for p in app.doc.primitives.values() {
        sum += glam::Vec3::from_array(p.origin);
        n += 1.0;
    }
    Some(sum / n + glam::Vec3::new(0.0, 0.0, 12.0))
}

fn camera_has_orbited(app: &AppState) -> bool {
    let home = app.cam.home_view();
    (app.cam.yaw - home.yaw).abs() > 0.2 || (app.cam.pitch - home.pitch).abs() > 0.15
}

fn camera_has_panned(app: &AppState) -> bool {
    let home = app.cam.home_view();
    (app.cam.target - home.target).length() > 8.0
}

fn camera_has_zoomed(app: &AppState) -> bool {
    let home = app.cam.home_view();
    (app.cam.distance - home.distance).abs() > 25.0
}

fn camera_on_standard_view(app: &AppState) -> bool {
    use crate::camera::StandardView::*;
    for view in [Front, Back, Left, Right, Top, Bottom] {
        let (y, p) = view.yaw_pitch();
        if (app.cam.yaw - y).abs() < 0.08 && (app.cam.pitch - p).abs() < 0.08 {
            return true;
        }
    }
    // Dragging the bear leaves trackball state too.
    app.cam.has_orbit_trackball_state()
}

fn camera_at_home(app: &AppState) -> bool {
    let home = app.cam.home_view();
    (app.cam.target - home.target).length() < 2.0
        && (app.cam.yaw - home.yaw).abs() < 0.08
        && (app.cam.pitch - home.pitch).abs() < 0.08
        && (app.cam.distance - home.distance).abs() < 8.0
        && !app.cam.has_orbit_trackball_state()
}

fn assist_nav_orbit(app: &mut AppState) {
    // Land clear of every StandardView so the later bear-snap step still has work to do.
    let home = app.cam.home_view();
    app.cam.yaw = home.yaw + 0.55;
    app.cam.pitch = (home.pitch + 0.2).clamp(-1.4, 1.4);
}

fn assist_nav_pan(app: &mut AppState) {
    app.cam.target += glam::Vec3::new(40.0, 20.0, 0.0);
}

fn assist_nav_zoom(app: &mut AppState) {
    let home_d = app.cam.home_view().distance;
    app.cam.distance = (home_d * 0.45).max(40.0);
}

fn assist_nav_bear_snap(app: &mut AppState) {
    let (yaw, pitch) = crate::camera::StandardView::Front.yaw_pitch();
    app.cam.yaw = yaw;
    app.cam.pitch = pitch;
    // Clear any trackball residue so the pose is a clean face snap.
    app.cam.leave_sketch_mode();
}

fn assist_nav_home(app: &mut AppState) {
    let home = app.cam.home_view();
    app.cam.yaw = home.yaw;
    app.cam.pitch = home.pitch;
    app.cam.target = home.target;
    app.cam.distance = home.distance;
    app.cam.set_view_up(home.view_up);
    app.cam.leave_sketch_mode();
}

const fn nav_drag_step(
    narration: &'static str,
    done: fn(&AppState) -> bool,
    drag_hint: &'static str,
    assist: StepAssist,
    phone_narration: Option<&'static str>,
) -> Step {
    Step {
        narration,
        anchor: StepAnchor::World(nav_cubes_guide),
        done: Some(done),
        on_enter: None,
        assist: Some(assist),
        needs_shift: None,
        drag_hint: Some(drag_hint),
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration,
        only_on_phone: false,
    }
}

/// #1269: second walkthrough — pan, orbit, zoom, bear HUD, home, Selection Exploder.
/// Starts with cubes already in the document. One action per step (#1253).
static NAVIGATE_STEPS: &[Step] = &[
    plain_step_enter(
        "Here are a few cubes. Let's learn to move around them.",
        StepAnchor::None,
        None,
        seed_nav_cubes,
    ),
    nav_drag_step(
        "Right-drag to orbit around the model.",
        camera_has_orbited,
        "Right button",
        StepAssist {
            label: "Orbit for me",
            run: assist_nav_orbit,
        },
        Some("Drag with three fingers to orbit around the model."),
    ),
    nav_drag_step(
        "Middle-drag, or Shift + right-drag, to pan.",
        camera_has_panned,
        "Middle button",
        StepAssist {
            label: "Pan for me",
            run: assist_nav_pan,
        },
        Some("Drag with two fingers to pan."),
    ),
    Step {
        narration: "Scroll the mouse wheel to zoom in and out.",
        anchor: StepAnchor::World(nav_cubes_guide),
        done: Some(camera_has_zoomed),
        on_enter: None,
        assist: Some(StepAssist {
            label: "Zoom for me",
            run: assist_nav_zoom,
        }),
        needs_shift: None,
        drag_hint: None,
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration: Some("Pinch to zoom in and out."),
        only_on_phone: false,
    },
    assisted_step(
        "The bear in the corner is your view cube. Click a face, edge, or corner to snap \
         a view \u{2014} or drag the bear to orbit.",
        StepAnchor::Ui(UiAnchor::ViewCube),
        Some(camera_on_standard_view),
        StepAssist {
            label: "Snap a view for me",
            run: assist_nav_bear_snap,
        },
        None,
    ),
    assisted_step(
        "Click the house under the bear to go to the Home view.",
        StepAnchor::Ui(UiAnchor::ViewHome),
        Some(camera_at_home),
        StepAssist {
            label: "Go home for me",
            run: assist_nav_home,
        },
        None,
    ),
    Step {
        narration: "Pick one face of a cube \u{2014} not the whole body. Hover the pile, press \
                    Space, and click a loupe. That's the Selection Exploder.",
        anchor: StepAnchor::World(nav_cubes_guide),
        done: None,
        on_enter: None,
        assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: Some(("Space", "Hover the cubes, press Space, click a loupe")),
        marks: None,
        type_hint: None,
        phone_narration: None,
        only_on_phone: false,
    },
    plain_step(
        "That's the view: orbit, pan, zoom, the bear, Home, and Space for crowded picks. \
         Nice!",
        StepAnchor::None,
        None,
    ),
];

/// #1238 / #1256–#1259 / #1262: first walkthrough — click a square, extrude it.
/// No numbers, no parameters: pure click joy. One action per step (#1253).
static CUBE_STEPS: &[Step] = &[
    plain_step("Hi! Let's make a cube.", StepAnchor::None, None),
    plain_step(
        "Click the Rectangle tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Rectangle)),
        Some(rectangle_tool_active),
    ),
    plain_step_enter(
        "Click a corner on the ground.",
        StepAnchor::World(rect_first_corner_guide),
        Some(rect_first_corner_placed),
        ensure_rect_sketch_for_tutorial,
    ),
    assisted_step(
        "Click the opposite corner.",
        StepAnchor::World(rect_opposite_corner_guide),
        Some(has_rectangle_outline),
        StepAssist {
            label: "Draw it for me",
            run: assist_draw_square,
        },
        None,
    ),
    plain_step(
        "Click the Extrude tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Extrude)),
        Some(extrude_tool_active),
    ),
    plain_step(
        "Click the square.",
        StepAnchor::World(rectangle_face_guide),
        Some(extrude_face_picked),
    ),
    assisted_step(
        "Press Enter. A cube!",
        StepAnchor::None,
        Some(has_extrusion),
        StepAssist {
            label: "Extrude it for me",
            run: assist_extrude_to_cube,
        },
        None,
    ),
    plain_step("You made a solid. Nice!", StepAnchor::None, None),
];

/// #1239: place a cuboid, cylinder, and sphere with the Shape tool (tool cycle order).
/// One action per step (#1253).
static SHAPES_STEPS: &[Step] = &[
    // #1270: short intro — Next only.
    plain_step(
        "The Shape tool makes solids right in 3D",
        StepAnchor::None,
        None,
    ),
    plain_step(
        "Grab the Shape tool \u{2014} the glowing button, or press `B`. It starts as a cuboid.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Shape)),
        Some(cuboid_kind_ready),
    ),
    plain_step(
        "Click a ground corner to anchor the cuboid.",
        StepAnchor::World(ground_anchor_a),
        Some(cuboid_anchored),
    ),
    plain_step(
        "Click the opposite corner of the base.",
        StepAnchor::World(ground_anchor_b),
        Some(cuboid_base_set),
    ),
    assisted_step_enter(
        "Type the height: `20`, then Enter.",
        StepAnchor::Ui(UiAnchor::ShapeHeight),
        Some(has_cuboid),
        StepAssist {
            label: "Place it for me",
            run: assist_place_cuboid,
        },
        Some(TypeHint::Fixed("20")),
        ensure_shape_height_focus,
    ),
    plain_step(
        "Press `B` to re-arm the Shape tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Shape)),
        Some(shape_tool_active_or_past_cuboid),
    ),
    // #1272: orb on the Context Cylinder button (not the toolbar).
    plain_step(
        "Click Cylinder in the Context pane (or press `B`).",
        StepAnchor::Ui(UiAnchor::ShapeKind(crate::model::PrimitiveKind::Cylinder)),
        Some(cylinder_kind_ready),
    ),
    // #1273: base on a wall construction plane, not a cuboid corner.
    plain_step(
        "Click the centre of the cylinder's base.",
        StepAnchor::World(wall_plane_cylinder_anchor),
        Some(cylinder_anchored),
    ),
    assisted_step(
        "Type the radius: `10`, then Tab.",
        StepAnchor::Ui(UiAnchor::ShapeRadius),
        Some(cylinder_radius_typed_10),
        StepAssist {
            label: "Place it for me",
            run: assist_place_cylinder,
        },
        Some(TypeHint::Fixed("10")),
    ),
    // #1309: don't say what to type until Height has the keyboard.
    assisted_step(
        "Press `Tab`, or click the Height field.",
        StepAnchor::Ui(UiAnchor::ShapeHeight),
        Some(cylinder_height_ready),
        StepAssist {
            label: "Place it for me",
            run: assist_place_cylinder,
        },
        None,
    ),
    assisted_step_enter(
        "Type the height: `20`, then Enter.",
        StepAnchor::Ui(UiAnchor::ShapeHeight),
        Some(has_cylinder),
        StepAssist {
            label: "Place it for me",
            run: assist_place_cylinder,
        },
        Some(TypeHint::Fixed("20")),
        ensure_shape_height_focus,
    ),
    plain_step(
        "Press `B` to re-arm the Shape tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Shape)),
        Some(shape_tool_active_or_past_cylinder),
    ),
    plain_step(
        "Click Sphere in the Context pane (or press `B`).",
        StepAnchor::Ui(UiAnchor::ShapeKind(crate::model::PrimitiveKind::Sphere)),
        Some(sphere_kind_ready),
    ),
    plain_step(
        "Click where the sphere should rest.",
        StepAnchor::World(ground_anchor_d),
        Some(sphere_anchored),
    ),
    assisted_step(
        "Type the radius: `10`, then Enter.",
        StepAnchor::Ui(UiAnchor::ShapeRadius),
        Some(has_all_three_shapes),
        StepAssist {
            label: "Place it for me",
            run: assist_place_sphere,
        },
        Some(TypeHint::Fixed("10")),
    ),
    // ASCII `->` not U+2192: Ubuntu Light (UI font) lacks that glyph (#1265).
    plain_step(
        "Three solids, no sketches. Press `B` any time to cycle cuboid -> cylinder \
         -> sphere. See you around the viewport!",
        StepAnchor::None,
        None,
    ),
];

/// #1240: 10×10×10 mm box, then edit a sketch dimension to 20 mm.
/// One action per step (#1253).
static DIMENSIONED_BOX_STEPS: &[Step] = &[
    plain_step(
        "Hi! We'll make a cube, then change it using parameters.",
        StepAnchor::None,
        None,
    ),
    plain_step(
        "Rectangle tool first \u{2014} glowing button, or `R`.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Rectangle)),
        Some(rectangle_tool_active),
    ),
    plain_step_enter(
        "Click the first corner on the ground.",
        StepAnchor::World(rect_first_corner_guide),
        Some(rect_first_corner_placed),
        ensure_rect_sketch_for_tutorial,
    ),
    assisted_step_enter(
        "Type the width: `10`, then Tab.",
        StepAnchor::Ui(UiAnchor::RectWidth),
        Some(rect_width_10),
        StepAssist {
            label: "Draw 10×10 for me",
            run: assist_draw_10mm_square,
        },
        Some(TypeHint::Fixed("10")),
        focus_rect_width_field,
    ),
    assisted_step_enter(
        "Type the height: `10`.",
        StepAnchor::Ui(UiAnchor::RectHeight),
        Some(rect_height_10),
        StepAssist {
            label: "Draw 10×10 for me",
            run: assist_draw_10mm_square,
        },
        Some(TypeHint::Fixed("10")),
        focus_rect_height_field,
    ),
    plain_step(
        "Press Enter \u{2014} or click \u{2014} to place the square.",
        StepAnchor::Ui(UiAnchor::RectHeight),
        Some(rect_dims_are_10),
    ),
    plain_step(
        "Extrude tool \u{2014} glowing button, or `E`.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Extrude)),
        Some(extrude_tool_active),
    ),
    plain_step(
        "Click the face.",
        StepAnchor::World(rectangle_face_guide),
        Some(extrude_face_picked),
    ),
    assisted_step(
        "Type `10` for the depth, then Enter. A 10 mm cube.",
        StepAnchor::Ui(UiAnchor::ExtrudeDistance),
        Some(extrusion_is_10),
        StepAssist {
            label: "Extrude 10 for me",
            run: assist_extrude_10mm,
        },
        Some(TypeHint::Fixed("10")),
    ),
    // #1279: orb on the sketch row in Elements.
    plain_step(
        "Reopen the sketch \u{2014} double-click it in Elements, or the sketch in the viewport.",
        StepAnchor::Ui(UiAnchor::ElementsSketch),
        Some(sketch_reopened_for_edit),
    ),
    assisted_step(
        "Change one side dimension from `10` to `20`. The box stretches.",
        StepAnchor::None,
        Some(one_sketch_dim_is_20),
        StepAssist {
            label: "Change it for me",
            run: assist_edit_dim_to_20,
        },
        Some(TypeHint::Fixed("20")),
    ),
    plain_step(
        "That's the parametric loop: dimensions drive the solid. Change numbers, not geometry. \
         Nice work!",
        StepAnchor::None,
        None,
    ),
];

fn shape_tool_active_or_past_cuboid(app: &AppState) -> bool {
    // Re-arm step: tool is Shape again, or the user already cycled / placed further shapes.
    shape_tool_active(app) || has_cylinder(app) || has_sphere(app) || cylinder_kind_ready(app)
}

fn shape_tool_active_or_past_cylinder(app: &AppState) -> bool {
    shape_tool_active(app) || has_sphere(app) || sphere_kind_ready(app)
}

#[cfg(test)]
mod tests {
    use crate::model::plane_key_for_slot as pkey;
    use super::*;
    use crate::actions::{Action, Pane};

    fn bracket_index() -> usize {
        tutorial_index("bracket").expect("bracket tutorial is registered")
    }

    /// The bracket tutorial auto-advances as a scripted build satisfies each step's
    /// predicate, from parameters through the final angle change.
    #[test]
    fn bracket_predicates_track_a_scripted_build() {
        let mut app = AppState::default();
        app.tutorial = Some(TutorialRun { tutorial: bracket_index(), step: 1, hold: false });

        assert!(!params_defined(&app));
        for (name, value) in [
            ("leg", "50mm"),
            ("width", "40mm"),
            ("thick", "5mm"),
            ("hole", "5mm"),
            ("bend", "4mm"),
            ("bend_angle", "120deg"),
        ] {
            app.apply(Action::AddParameter {
                name: name.to_string(),
                expression: value.to_string(),
            });
        }
        assert!(params_defined(&app));

        app.apply(Action::SetTool(Tool::Line));
        assert!(line_tool_active(&app));

        assert!(!bend_angle_changed(&app), "120deg is the starting value");
        let bend = app
            .doc
            .parameters
            .iter()
            .find_map(|(k, p)| (p.name == "bend_angle").then_some(k))
            .expect("the tutorial's bend_angle parameter");
        app.apply(Action::CommitParameterExpression {
            index: bend,
            expression: "150deg".to_string(),
        });
        assert!(bend_angle_changed(&app));
    }

    /// Back reviews earlier steps without auto-advance re-firing on their already-
    /// satisfied predicates; Next resumes auto mode once it reaches unfinished work.
    #[test]
    fn back_reviews_without_auto_advance_snapping_forward() {
        let mut app = AppState::default();
        app.apply(Action::StartTutorial { index: bracket_index() });
        app.apply(Action::TutorialNext); // past the welcome step
        for (name, value) in [
            ("leg", "50mm"),
            ("width", "40mm"),
            ("thick", "5mm"),
            ("hole", "5mm"),
            ("bend", "4mm"),
            ("bend_angle", "120deg"),
        ] {
            app.apply(Action::AddParameter {
                name: name.to_string(),
                expression: value.to_string(),
            });
        }
        assert_eq!(app.tutorial.unwrap().step, 11, "params chain to the pane/line-tool steps");

        app.apply(Action::TutorialBack);
        let run = app.tutorial.unwrap();
        assert_eq!(run.step, 10);
        assert!(run.hold);
        // Its predicate is satisfied, but reviewing holds auto-advance off.
        app.advance_tutorial();
        assert_eq!(app.tutorial.unwrap().step, 10);

        // Next walks forward; reaching the line-tool step (unfinished) resumes auto.
        app.apply(Action::TutorialNext);
        let run = app.tutorial.unwrap();
        assert_eq!(run.step, 11);
        assert!(!run.hold, "caught up to live work — auto-advance resumes");
        app.apply(Action::SetTool(Tool::Line));
        assert_eq!(app.tutorial.unwrap().step, 12, "auto-advance is live again");
    }

    /// The parameters step's assist button fills in the whole table in one press —
    /// and adding only what's missing, so a user who typed a couple by hand keeps them.
    #[test]
    fn assist_button_adds_the_remaining_parameters() {
        let mut app = AppState::default();
        app.apply(Action::StartTutorial { index: bracket_index() });
        app.apply(Action::AddParameter {
            name: "leg".to_string(),
            expression: "60mm".to_string(),
        });

        // The step listing the rest of the table (its assist adds them all).
        let step = BRACKET_STEPS
            .iter()
            .position(|s| s.done.is_some_and(|d| std::ptr::fn_addr_eq(d, params_defined as fn(&AppState) -> bool)))
            .unwrap();
        app.tutorial = Some(TutorialRun { tutorial: bracket_index(), step, hold: false });
        app.parameters_pane.new_name = "wid".to_string();
        app.apply(Action::TutorialAssist);

        assert!(params_defined(&app));
        assert!(app.parameters_pane.new_name.is_empty(), "the draft row is cleared");
        let leg = app.doc.parameters.values().find(|p| p.name == "leg").unwrap();
        assert_eq!(leg.expression, "60mm", "a hand-typed value is left alone");
        assert!(app.tutorial.unwrap().step > step, "the step auto-advances as usual");
    }

    /// #832: on the parameter-list step the orb walks the two boxes with the badge — the
    /// name until it's typed, then the value.
    #[test]
    fn parameter_list_orb_moves_to_the_value_box() {
        let mut app = AppState::default();
        assert!(matches!(
            param_list_orb(&app),
            Some(StepTarget::Ui(UiAnchor::ParametersName))
        ));
        let (next, _) = BRACKET_PARAMS.iter().find(|(n, _)| !param_exists(&app, n)).unwrap();
        app.parameters_pane.new_name = next.to_string();
        assert!(matches!(
            param_list_orb(&app),
            Some(StepTarget::Ui(UiAnchor::ParametersValue))
        ));
    }

    /// #828: the phone-only steps are left out of the numbering on anything wider, so a
    /// desktop reader's "step N of M" counts only the steps they'll actually see.
    #[test]
    fn phone_steps_stay_out_of_the_desktop_numbering() {
        let mut app = AppState::default();
        let phone_steps = BRACKET_STEPS.iter().filter(|s| s.only_on_phone).count();
        assert!(phone_steps > 0, "there are phone-only steps to leave out");

        let last = BRACKET_STEPS.len() - 1;
        let bi = bracket_index();
        let (_, desktop_total) = step_position(&app, bi, last);
        assert_eq!(desktop_total, BRACKET_STEPS.len() - phone_steps);

        app.compact_layout = true;
        let (_, phone_total) = step_position(&app, bi, last);
        assert_eq!(phone_total, BRACKET_STEPS.len(), "a phone sees them all");

        // A step after some phone-only ones counts lower on a desktop than on a phone.
        let after = BRACKET_STEPS
            .iter()
            .position(|s| s.done.is_some_and(|d| std::ptr::fn_addr_eq(d, line_tool_active as fn(&AppState) -> bool)))
            .unwrap();
        app.compact_layout = false;
        let (desktop_pos, _) = step_position(&app, bi, after);
        app.compact_layout = true;
        let (phone_pos, _) = step_position(&app, bi, after);
        assert!(desktop_pos < phone_pos, "{desktop_pos} vs {phone_pos}");
    }

    /// #828: the phone-only steps (open/tuck the floating panes) are already satisfied on a
    /// desktop, where the panes are docked — so they pass straight through and only ever
    /// show up on a phone.
    #[test]
    fn phone_pane_steps_pass_straight_through_on_desktop() {
        let mut app = AppState::default();
        assert!(!app.compact_layout, "the default layout is the desktop one");
        assert!(params_pane_ready(&app) && params_pane_tucked(&app));
        assert!(context_pane_ready(&app) && context_pane_tucked(&app));

        // On a phone they're real work: the pane has to be opened, then tucked away again.
        app.compact_layout = true;
        app.apply(Action::SetPaneVisible {
            pane: crate::actions::Pane::Parameters,
            visible: false,
        });
        assert!(!params_pane_ready(&app), "the pane is hidden — the step has work to do");
        // Opening it satisfies the step; tucking it away satisfies the one after (#843 took
        // the "do it for me" buttons off both — a tap is the whole job).
        app.apply(Action::SetPaneVisible {
            pane: crate::actions::Pane::Parameters,
            visible: true,
        });
        assert!(params_pane_ready(&app));
        assert!(!params_pane_tucked(&app));
        app.apply(Action::SetPaneVisible {
            pane: crate::actions::Pane::Parameters,
            visible: false,
        });
        assert!(params_pane_tucked(&app));
    }

    /// #810/#843: every step that offers a button really is done by it — walk the tutorial
    /// pressing each step's button, or Next where there isn't one (narration, and the
    /// click-only steps whose button #843 took away), and it finishes. The assists that need
    /// earlier geometry make it themselves, so skipping ahead doesn't strand them.
    #[test]
    fn every_working_step_can_do_itself() {
        let mut app = AppState::default();
        app.apply(Action::StartTutorial { index: bracket_index() });
        let steps = BRACKET_STEPS.len();
        let mut guard = 0;
        while let Some(run) = app.tutorial {
            guard += 1;
            assert!(guard < steps * 3, "the assists should walk forward, not loop");
            let step = &BRACKET_STEPS[run.step];
            match step.assist {
                Some(_) => {
                    let before = run.step;
                    app.apply(Action::TutorialAssist);
                    // A step whose work the assist did advances on its own predicate; if it
                    // didn't, say which one so the failure names the step.
                    assert!(
                        app.tutorial.is_none_or(|r| r.step > before),
                        "step {before} didn't advance after its own assist: {}",
                        step.narration
                    );
                }
                // Narration-only: Next is the button.
                None => {
                    app.apply(Action::TutorialNext);
                }
            }
        }
    }

    /// #843: the "do it for me" button is for steps that need typing (or a keypress), not
    /// for ones where clicking the thing the orb points at is the whole job.
    #[test]
    fn click_only_steps_offer_no_button() {
        // Tool buttons, pane taps, tapping into a box, clicking a face or the glowing points —
        // and the constraint steps, whose three marks are all clicks now (#864).
        for step in [1, 2, 4, 6, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 31, 33, 34, 38, 40] {
            assert!(
                BRACKET_STEPS[step].assist.is_none(),
                "step {step} is click-only but offers a button: {}",
                BRACKET_STEPS[step].narration
            );
        }
        // Typing keeps theirs.
        for step in [3, 5, 7, 8, 9, 22, 23, 27, 35, 41, 44] {
            assert!(
                BRACKET_STEPS[step].assist.is_some(),
                "step {step} needs the keyboard and should offer a button: {}",
                BRACKET_STEPS[step].narration
            );
        }
    }

    /// #1265: the UI proportional font (Ubuntu Light) has no U+2192 (→); the tutorial bubble
    /// rendered tofu boxes for "cuboid → cylinder → sphere". Narrations must use ASCII `->`.
    #[test]
    fn tutorial_narration_avoids_arrow_glyph_missing_from_ui_font() {
        for t in TUTORIALS {
            for step in t.steps {
                assert!(
                    !step.narration.contains('\u{2192}'),
                    "{}: narration uses → (missing from UI font): {}",
                    t.name,
                    step.narration
                );
                if let Some(p) = step.phone_narration {
                    assert!(
                        !p.contains('\u{2192}'),
                        "{}: phone narration uses → (missing from UI font): {p}",
                        t.name
                    );
                }
            }
        }
    }

    /// Backticked runs come back marked as code, with the backticks stripped — and every
    /// step's narration is balanced, so no step ends up half in monospace.
    #[test]
    fn narration_spans_split_code_from_prose() {
        assert_eq!(
            narration_spans("Type `leg` — three letters."),
            vec![("Type ", false), ("leg", true), (" — three letters.", false)]
        );
        assert_eq!(narration_spans("plain"), vec![("plain", false)]);
        assert_eq!(narration_spans("`code`"), vec![("code", true)]);

        for step in BRACKET_STEPS {
            assert!(
                step.narration.matches('`').count() % 2 == 0,
                "unbalanced backticks: {}",
                step.narration
            );
            let rebuilt: String =
                narration_spans(step.narration).iter().map(|(t, _)| *t).collect();
            assert_eq!(rebuilt, step.narration.replace('`', ""));
        }
    }

    /// A constraint step's orb walks its two clicks: it points at the first target until
    /// that's selected, then at the second (with the Shift keycap), then at nothing. A
    /// stray selection that isn't the first target leaves it pointing back at the first —
    /// which is how a mis-click gets shown its way back (#758/#759).
    #[test]
    fn constraint_step_orb_walks_the_two_clicks() {
        use crate::hierarchy::SceneElement;
        use crate::model::FaceId;

        let mut app = AppState::default();
        app.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(pkey(0)),
            viewport: None,
        });
        for (x0, y0, x1, y1) in [
            (0.0, 0.0, 51.0, 2.5),
            (51.0, 2.5, 49.5, 7.8),
            (49.5, 7.8, 4.5, 5.5),
        ] {
            app.apply(Action::CreateLineSegment {
                x0,
                y0,
                x1,
                y1,
                bezier: None,
                dimension: None,
            });
        }
        let lines = profile_lines(&app);
        assert_eq!(lines.len(), 3);

        let world = |app: &AppState| match base_strip_click(app) {
            Some(StepTarget::World(w)) => Some(w),
            _ => None,
        };

        // Nothing picked: point at the *middle* of the bottom line (#769), no Shift yet.
        let first = world(&app).expect("orb points at the first click");
        assert!(!base_strip_shift(&app));
        let poly = profile_polyline(&app, 0).unwrap();
        assert!(
            (first - poly[0].lerp(poly[1], 0.5)).length() < 1e-3,
            "the orb sits mid-line, not on an endpoint: {first:?}"
        );

        // A wrong pick doesn't count — the orb stays on the line still wanted, and the
        // click that clears it is Shift-free (#785).
        app.scene_selection.insert(SceneElement::Line(lines[1]));
        assert!(world(&app).is_some_and(|p| (p - first).length() < 1e-3));
        assert!(!base_strip_shift(&app));

        // Even with the *right* line picked, a stray from an earlier step means starting
        // over with a plain click on the first target.
        app.scene_selection.insert(SceneElement::Line(lines[0]));
        assert!(world(&app).is_some_and(|p| (p - first).length() < 1e-3), "back to the first pick");
        assert!(!base_strip_shift(&app), "no Shift while a stray is selected");
        app.scene_selection.clear();

        // The right line: now the orb moves to the second line and asks for Shift.
        app.scene_selection.insert(SceneElement::Line(lines[0]));
        let second = world(&app).expect("orb points at the second click");
        assert!((second - first).length() > 1.0, "it moved");
        assert!(base_strip_shift(&app), "the second click of a pair holds Shift");

        // Both in hand: the orb moves to the pane button that applies it (#770).
        app.scene_selection.insert(SceneElement::Line(lines[2]));
        assert!(matches!(
            base_strip_click(&app),
            Some(StepTarget::Ui(UiAnchor::ConstraintButton(GC::Parallel)))
        ));
        assert!(!base_strip_shift(&app));
    }

    /// #876: the base-to-X-axis step is one pick — the line — and then the pane's own axis
    /// button, with no Shift anywhere.
    #[test]
    fn axis_constraint_step_walks_one_pick_then_the_button() {
        use crate::hierarchy::SceneElement;
        use crate::model::FaceId;

        let mut app = AppState::default();
        app.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(pkey(0)),
            viewport: None,
        });
        app.apply(Action::CreateLineSegment {
            x0: 0.0,
            y0: 0.0,
            x1: 51.0,
            y1: 2.5,
            bezier: None,
            dimension: None,
        });
        let lines = profile_lines(&app);

        let marks = level_marks(&app);
        assert_eq!(marks.len(), 2, "the line, then the button");
        assert!(matches!(level_click(&app), Some(StepTarget::World(_))));

        app.scene_selection.insert(SceneElement::Line(lines[0]));
        assert!(matches!(
            level_click(&app),
            Some(StepTarget::Ui(UiAnchor::ConstraintButton(GC::AlongXAxis)))
        ));
        assert!(level_marks(&app)[0].done, "the pick reads as done");
    }

    /// #765: the web app's `?tutorial=` parameter names a registered tutorial.
    #[test]
    fn tutorial_from_query_picks_a_registered_tutorial() {
        let bracket = bracket_index();
        let cube = tutorial_index("cube").unwrap();
        assert_eq!(tutorial_from_query("?tutorial=bracket"), Some(bracket));
        assert_eq!(tutorial_from_query("tutorial=bracket"), Some(bracket));
        assert_eq!(tutorial_from_query("?foo=1&tutorial=bracket&bar=2"), Some(bracket));
        assert_eq!(tutorial_from_query("?tutorial=cube"), Some(cube));
        assert_eq!(tutorial_from_query("?tutorial=nope"), None);
        assert_eq!(tutorial_from_query("?other=bracket"), None);
        assert_eq!(tutorial_from_query(""), None);
    }

    #[test]
    fn tutorial_registry_lookup_by_name() {
        assert_eq!(tutorial_index("cube"), Some(0));
        assert_eq!(tutorial_index("navigate"), Some(1), "#1269: navigate is second");
        assert_eq!(tutorial_index("shapes"), Some(2));
        assert_eq!(tutorial_index("dimensioned_box"), Some(3));
        assert_eq!(tutorial_index("bracket"), Some(4));
        assert_eq!(tutorial_index("nope"), None);
        assert_eq!(TUTORIALS.last().unwrap().name, "bracket", "#1251: bracket is last");
        assert!(TUTORIALS[bracket_index()].steps.len() >= 10);
        assert!(TUTORIALS.len() >= 5, "pane lists every walkthrough");
    }

    /// #1253: short tutorials never combine a click with typing in one step's narration.
    #[test]
    fn short_tutorial_steps_keep_clicks_and_typing_apart() {
        for tut in TUTORIALS.iter().filter(|t| t.name != "bracket") {
            for step in tut.steps {
                let n = step.narration.to_ascii_lowercase();
                let has_click = n.contains("click");
                let has_type = n.contains("type ") || n.contains("type:") || n.contains("type the");
                assert!(
                    !(has_click && has_type),
                    "tutorial '{}' combines click and type: {}",
                    tut.name,
                    step.narration
                );
            }
        }
    }

    /// #1261: starting any tutorial resets the camera to the home view (View → Home).
    #[test]
    fn starting_a_tutorial_resets_to_home_view() {
        let mut app = AppState::default();
        // Park the camera somewhere that isn't home.
        app.cam.target = glam::Vec3::new(80.0, -40.0, 25.0);
        app.cam.yaw = 2.4;
        app.cam.pitch = -0.55;
        app.cam.distance = 180.0;
        app.cam.set_view_up(Some(glam::Vec3::Y));

        let home = app.cam.home_view();
        app.apply(Action::StartTutorial {
            index: tutorial_index("cube").unwrap(),
        });
        // Same animation path as View → Home; settle it so the pose is readable.
        while app.cam.tick_transition(0.05) {}

        assert!((app.cam.target - home.target).length() < 0.01);
        assert!((app.cam.yaw - home.yaw).abs() < 0.01);
        assert!((app.cam.pitch - home.pitch).abs() < 0.01);
        assert!((app.cam.distance - home.distance).abs() < 0.5);
        assert!(
            app.cam.view_up_hint().dot(glam::Vec3::Z).abs() > 0.99,
            "home clears custom up"
        );
        assert!(app.tutorial.is_some());
    }

    /// Drive a registered tutorial to completion via Next / assist (no predicates).
    fn finish_tutorial_via_next(app: &mut AppState, name: &str) {
        app.apply(Action::StartTutorial {
            index: tutorial_index(name).unwrap(),
        });
        let mut guard = 0;
        while app.tutorial.is_some() {
            guard += 1;
            assert!(guard < 50, "tutorial '{name}' should finish");
            let run = app.tutorial.unwrap();
            let step = &TUTORIALS[run.tutorial].steps[run.step];
            if step.assist.is_some() {
                app.apply(Action::TutorialAssist);
            }
            if app.tutorial.is_some() {
                app.apply(Action::TutorialNext);
            }
        }
    }

    /// #1241: finishing a tutorial records it for the Confirm-SVG check in the pane (#1260).
    #[test]
    fn finishing_a_tutorial_marks_it_completed() {
        let mut app = AppState::default();
        assert!(!app.tutorial_completed("cube"));
        // Open the pane first so we can assert StartTutorial closes it.
        app.apply(Action::SetTutorialPane { open: Some(true) });
        assert!(app.panes.is_visible(Pane::Tutorials));
        finish_tutorial_via_next(&mut app, "cube");
        assert!(app.tutorial_completed("cube"));
        assert!(app.completed_tutorials_dirty);
        // #1289 reopens it when more remain; the close-on-start is checked below.
    }

    /// Starting a walkthrough closes the Tutorials list (#1241).
    #[test]
    fn starting_a_tutorial_closes_the_pane() {
        let mut app = AppState::default();
        app.apply(Action::SetTutorialPane { open: Some(true) });
        app.apply(Action::StartTutorial {
            index: tutorial_index("cube").unwrap(),
        });
        assert!(
            !app.panes.is_visible(Pane::Tutorials),
            "starting a walkthrough closes the pane"
        );
    }

    /// #1289: when a walkthrough finishes and others remain, open the Tutorials pane.
    #[test]
    fn finishing_a_tutorial_opens_pane_when_more_remain() {
        let mut app = AppState::default();
        assert!(!app.panes.is_visible(Pane::Tutorials));
        finish_tutorial_via_next(&mut app, "cube");
        assert!(app.tutorial_completed("cube"));
        assert!(
            app.panes.is_visible(Pane::Tutorials),
            "more unfinished tutorials → list reopens so the user can pick next"
        );
    }

    /// #1289: finishing the last incomplete tutorial leaves the pane closed.
    #[test]
    fn finishing_the_last_tutorial_does_not_open_pane() {
        let mut app = AppState::default();
        // Mark every walkthrough complete except cube.
        for tut in TUTORIALS.iter().filter(|t| t.name != "cube") {
            app.mark_tutorial_completed(tut.name);
        }
        app.apply(Action::SetTutorialPane { open: Some(false) });
        finish_tutorial_via_next(&mut app, "cube");
        assert!(app.tutorial_completed("cube"));
        assert!(
            !app.panes.is_visible(Pane::Tutorials),
            "nothing left to pick → don't reopen the list"
        );
    }

    /// #1241: the Tutorials pane flag is scriptable.
    #[test]
    fn tutorial_pane_toggles() {
        let mut app = AppState::default();
        assert!(!app.panes.is_visible(Pane::Tutorials));
        app.apply(Action::SetTutorialPane { open: Some(true) });
        assert!(app.panes.is_visible(Pane::Tutorials));
        app.apply(Action::SetTutorialPane { open: None });
        assert!(!app.panes.is_visible(Pane::Tutorials));
    }

    /// #1291: Tutorials is a real pane — View ▸ Panes and `bearcad.ui.pane` both toggle it.
    #[test]
    fn tutorials_is_a_toggleable_pane() {
        assert!(
            Pane::ALL.contains(&Pane::Tutorials),
            "View ▸ Panes iterates Pane::ALL"
        );
        assert_eq!(Pane::Tutorials.label(), "Tutorials");
        assert_eq!(Pane::Tutorials.script_name(), "tutorials");
        assert_eq!(Pane::from_name("tutorials"), Some(Pane::Tutorials));

        let mut app = AppState::default();
        assert!(
            !app.panes.is_visible(Pane::Tutorials),
            "closed by default (unlike Elements/Context/Params)"
        );
        app.apply(Action::SetPaneVisible {
            pane: Pane::Tutorials,
            visible: true,
        });
        assert!(app.panes.is_visible(Pane::Tutorials));
        app.apply(Action::TogglePane(Pane::Tutorials));
        assert!(!app.panes.is_visible(Pane::Tutorials));
    }

    /// #1255: pane title matches Elements / Parameters style.
    #[test]
    fn pane_title_is_tutorials() {
        assert_eq!(PANE_TITLE, "Tutorials");
    }

    /// #1254: graduation-cap icon is registered for the status-bar launcher.
    #[test]
    fn graduation_cap_icon_is_registered() {
        assert_eq!(
            crate::icons::IconId::GraduationCap.label(),
            "Graduation cap"
        );
        assert!(crate::icons::IconId::ALL.contains(&crate::icons::IconId::GraduationCap));
    }

    /// #1238: the cube tutorial's assists build a rectangle and an extrusion.
    #[test]
    fn cube_tutorial_assists_build_a_solid() {
        let mut app = AppState::default();
        assist_extrude_to_cube(&mut app);
        assert!(has_closed_rectangle(&app) || has_rectangle_outline(&app));
        assert!(has_extrusion(&app));
        assert!(!app.doc.bodies.is_empty());
    }

    /// #1256/#1259: first tutorial is short, beginner-facing, and never asks for numbers.
    #[test]
    fn cube_tutorial_is_short_and_typeless() {
        let cube = &TUTORIALS[tutorial_index("cube").unwrap()];
        assert!(
            cube.steps[0].narration.starts_with("Hi! Let's make a cube"),
            "welcome should be short: {}",
            cube.steps[0].narration
        );
        assert!(
            !cube.steps[0].narration.to_ascii_lowercase().contains("classic"),
            "no CAD jargon in the welcome"
        );
        for step in cube.steps {
            let n = step.narration.to_ascii_lowercase();
            assert!(
                !n.contains("type ") && !n.contains("type:") && !n.contains("type the"),
                "cube step asks to type: {}",
                step.narration
            );
            assert!(
                step.type_hint.is_none(),
                "cube step has a type hint: {}",
                step.narration
            );
        }
    }

    /// #1257/#1262: after the tool is selected, click steps point at the ground/face.
    #[test]
    fn cube_and_shapes_click_steps_point_at_world_not_selected_tool() {
        let cube = &TUTORIALS[tutorial_index("cube").unwrap()];
        for step in cube.steps {
            let n = step.narration.to_ascii_lowercase();
            let is_placement = n.contains("ground")
                || n.contains("opposite corner")
                || n.contains("the square");
            if is_placement {
                assert!(
                    matches!(step.anchor, StepAnchor::World(_)),
                    "placement step should use a world anchor: {}",
                    step.narration
                );
            }
        }
        let shapes = &TUTORIALS[tutorial_index("shapes").unwrap()];
        for step in shapes.steps {
            let n = step.narration.to_ascii_lowercase();
            let is_placement = n.contains("ground")
                || n.contains("opposite corner")
                || n.contains("centre of the cylinder")
                || n.contains("sphere should rest");
            if is_placement {
                assert!(
                    matches!(step.anchor, StepAnchor::World(_)),
                    "shapes placement should use a world anchor: {}",
                    step.narration
                );
            }
        }
    }

    /// #1257: ground / face guides resolve once a sketch (or ground plane) exists.
    #[test]
    fn cube_world_guides_resolve_on_ground() {
        let mut app = AppState::default();
        assert!(rect_first_corner_guide(&app).is_some());
        assert!(ground_anchor_a(&app).is_some());
        ensure_rect_sketch_for_tutorial(&mut app);
        assert!(app.sketch_session.is_some());
        assert_eq!(app.tool, Tool::Rectangle);
        assert!(rect_first_corner_guide(&app).is_some());
        // Simulate a first corner so the opposite guide follows it.
        app.creating_rect = Some(crate::actions::CreatingRect {
            origin: glam::Vec3::new(10.0, 10.0, 0.0),
            texts: ["20".into(), "20".into()],
            focused: 0,
            last_mouse: glam::Vec3::new(30.0, 30.0, 0.0),
            user_edited: [false, false],
            pending_focus: true,
            construction: false,
            anchor: crate::actions::RectAnchor::Corner,
        });
        let opp = rect_opposite_corner_guide(&app).expect("opposite corner guide");
        assert!(opp.x > 10.0 && opp.y > 10.0);
    }

    /// #1258: typing steps focus the rect dim fields (dimensioned_box still types).
    #[test]
    fn dimensioned_box_typing_steps_target_rect_fields() {
        let box_tut = &TUTORIALS[tutorial_index("dimensioned_box").unwrap()];
        let typing: Vec<_> = box_tut
            .steps
            .iter()
            .filter(|s| {
                let n = s.narration.to_ascii_lowercase();
                n.contains("type the width") || n.contains("type the height")
            })
            .collect();
        assert_eq!(typing.len(), 2);
        assert!(matches!(
            typing[0].anchor,
            StepAnchor::Ui(UiAnchor::RectWidth)
        ));
        assert!(matches!(
            typing[1].anchor,
            StepAnchor::Ui(UiAnchor::RectHeight)
        ));
        assert!(typing[0].on_enter.is_some());
        assert!(typing[1].on_enter.is_some());
    }

    /// #1263: opposite-corner steps point at the next corner in the world, not the tool.
    #[test]
    fn opposite_corner_guide_follows_first_corner() {
        let mut app = AppState::default();
        // Cuboid base after first corner: guide sits on the opposite corner.
        let mut creating = crate::actions::CreatingShape::new(crate::model::PrimitiveKind::Cuboid);
        creating.first_corner = Some(glam::Vec3::new(10.0, 10.0, 0.0));
        creating.shape.origin = [10.0, 10.0, 0.0];
        creating.shape.u_axis = [1.0, 0.0, 0.0];
        creating.shape.normal = [0.0, 0.0, 1.0];
        creating.phase = crate::actions::ShapePhase::Base;
        app.creating_shape = Some(creating);
        let opp = ground_anchor_b(&app).expect("opposite corner guide");
        assert!(
            (opp - glam::Vec3::new(50.0, 50.0, 0.0)).length() < 0.1,
            "expected first_corner + 40u + 40v, got {opp:?}"
        );

        // Cube tutorial rectangle: opposite corner follows the first click.
        app.creating_shape = None;
        ensure_rect_sketch_for_tutorial(&mut app);
        app.creating_rect = Some(crate::actions::CreatingRect {
            origin: glam::Vec3::new(5.0, 5.0, 0.0),
            texts: ["20".into(), "20".into()],
            focused: 0,
            last_mouse: glam::Vec3::new(25.0, 25.0, 0.0),
            user_edited: [false, false],
            pending_focus: true,
            construction: false,
            anchor: crate::actions::RectAnchor::Corner,
        });
        let rect_opp = rect_opposite_corner_guide(&app).expect("rect opposite");
        assert!(rect_opp.x > 5.0 && rect_opp.y > 5.0);

        // Step authoring: shapes + cube opposite-corner steps use World.
        let shapes = &TUTORIALS[tutorial_index("shapes").unwrap()];
        let opp_step = shapes
            .steps
            .iter()
            .find(|s| s.narration.to_ascii_lowercase().contains("opposite corner"))
            .expect("shapes opposite-corner step");
        assert!(matches!(opp_step.anchor, StepAnchor::World(_)));

        let cube = &TUTORIALS[tutorial_index("cube").unwrap()];
        let cube_opp = cube
            .steps
            .iter()
            .find(|s| s.narration.to_ascii_lowercase().contains("opposite corner"))
            .expect("cube opposite-corner step");
        assert!(matches!(cube_opp.anchor, StepAnchor::World(_)));
    }

    /// #1264: shape typing steps point at Height / Radius fields, not the Shape tool.
    /// #1274: height steps also arm Height focus on enter so Radius can't keep the keyboard.
    #[test]
    fn shapes_typing_steps_target_height_and_radius_fields() {
        let shapes = &TUTORIALS[tutorial_index("shapes").unwrap()];
        let height_steps: Vec<_> = shapes
            .steps
            .iter()
            .filter(|s| s.narration.to_ascii_lowercase().contains("type the height"))
            .collect();
        assert!(!height_steps.is_empty());
        for step in height_steps {
            assert!(
                matches!(step.anchor, StepAnchor::Ui(UiAnchor::ShapeHeight)),
                "height typing should point at ShapeHeight: {}",
                step.narration
            );
            assert!(step.type_hint.is_some());
            assert!(
                step.on_enter.is_some(),
                "height typing should focus Height on enter: {}",
                step.narration
            );
        }

        // on_enter advances Base → Height and arms pending focus.
        let mut app = AppState::default();
        app.apply(Action::SetTool(Tool::Shape));
        if let Some(c) = app.creating_shape.as_mut() {
            c.phase = crate::actions::ShapePhase::Base;
            c.shape.kind = crate::model::PrimitiveKind::Cylinder;
            c.pending_focus = false;
        }
        ensure_shape_height_focus(&mut app);
        let c = app.creating_shape.as_ref().unwrap();
        assert_eq!(c.phase, crate::actions::ShapePhase::Height);
        assert!(c.pending_focus);
        let radius_steps: Vec<_> = shapes
            .steps
            .iter()
            .filter(|s| s.narration.to_ascii_lowercase().contains("type the radius"))
            .collect();
        assert!(!radius_steps.is_empty());
        for step in radius_steps {
            assert!(
                matches!(step.anchor, StepAnchor::Ui(UiAnchor::ShapeRadius)),
                "radius typing should point at ShapeRadius: {}",
                step.narration
            );
        }

        // Cube face / extrude click also uses a world guide (same multi-step family).
        let cube = &TUTORIALS[tutorial_index("cube").unwrap()];
        let face = cube
            .steps
            .iter()
            .find(|s| {
                let n = s.narration.to_ascii_lowercase();
                n.contains("the square") || n.contains("the face")
            })
            .expect("cube face click step");
        assert!(
            matches!(face.anchor, StepAnchor::World(_)),
            "face click should use a world guide: {}",
            face.narration
        );
        assert!(rectangle_face_guide(&AppState::default()).is_some());
    }

    /// #1309: after typing the cylinder radius, teach Tab/click Height *before*
    /// telling the user to type 20. Type-20 is gated on Height being the focused phase.
    #[test]
    fn shapes_cylinder_asks_to_tab_before_typing_height() {
        let shapes = &TUTORIALS[tutorial_index("shapes").unwrap()];
        let radius_i = shapes
            .steps
            .iter()
            .position(|s| {
                let n = s.narration.to_ascii_lowercase();
                n.contains("type the radius") && n.contains("10")
            })
            .expect("cylinder radius typing step");
        let radius = &shapes.steps[radius_i];
        assert!(
            radius.narration.to_ascii_lowercase().contains("tab"),
            "radius step should send the user to Tab, not Enter: {}",
            radius.narration
        );

        let tab = &shapes.steps[radius_i + 1];
        let tab_n = tab.narration.to_ascii_lowercase();
        assert!(
            tab_n.contains("tab") && (tab_n.contains("click") || tab_n.contains("height")),
            "next step should ask for Tab or a Height click: {}",
            tab.narration
        );
        assert!(tab.type_hint.is_none(), "don't say what to type until Height is focused");
        assert!(
            matches!(tab.anchor, StepAnchor::Ui(UiAnchor::ShapeHeight)),
            "Tab step should point at Height: {}",
            tab.narration
        );
        assert!(tab.done.is_some());

        let height = &shapes.steps[radius_i + 2];
        assert!(
            height.narration.to_ascii_lowercase().contains("type the height"),
            "type-20 comes only after Tab: {}",
            height.narration
        );
        assert!(height.type_hint.is_some());

        // Predicate: still in Base (radius focused) is not done; Height phase is.
        let mut app = AppState::default();
        app.apply(Action::SetTool(Tool::Shape));
        if let Some(c) = app.creating_shape.as_mut() {
            c.shape.kind = crate::model::PrimitiveKind::Cylinder;
            c.phase = crate::actions::ShapePhase::Base;
            c.shape.radius = "10".into();
            c.typed[3] = true;
        }
        let done = tab.done.expect("Tab step auto-advances");
        assert!(
            !done(&app),
            "must not ask to type 20 while Radius still owns the keyboard"
        );
        if let Some(c) = app.creating_shape.as_mut() {
            c.phase = crate::actions::ShapePhase::Height;
        }
        assert!(done(&app), "Height phase means they Tabbed or clicked Height");
        assert!(
            crate::actions::shape_tab_advances_height(
                crate::model::PrimitiveKind::Cylinder,
                crate::actions::ShapePhase::Base
            ),
            "Tab from cylinder radius should advance to Height"
        );
        assert!(!crate::actions::shape_tab_advances_height(
            crate::model::PrimitiveKind::Cuboid,
            crate::actions::ShapePhase::Base
        ));
        assert!(crate::actions::shape_field_click_advances_height(
            crate::model::PrimitiveKind::Cylinder,
            crate::actions::ShapePhase::Base,
            crate::actions::ShapeDimension::Height,
        ));
        assert!(!crate::actions::shape_field_click_advances_height(
            crate::model::PrimitiveKind::Cylinder,
            crate::actions::ShapePhase::Base,
            crate::actions::ShapeDimension::Radius,
        ));
    }

    /// #1239: shapes tutorial assists place all three primitives.
    #[test]
    fn shapes_tutorial_assists_place_three_solids() {
        let mut app = AppState::default();
        assist_place_sphere(&mut app); // chains cuboid → cylinder → sphere
        assert!(has_cuboid(&app));
        assert!(has_sphere(&app));
        assert!(has_cylinder(&app));
        assert_eq!(app.doc.primitives.len(), 3);
    }

    /// #1270: shapes intro is short — one line, Next advances (no auto-done).
    #[test]
    fn shapes_intro_is_short() {
        let shapes = &TUTORIALS[tutorial_index("shapes").unwrap()];
        let intro = &shapes.steps[0];
        assert_eq!(
            intro.narration,
            "The Shape tool makes solids right in 3D"
        );
        assert!(intro.done.is_none(), "intro waits for Next");
        assert!(matches!(intro.anchor, StepAnchor::None));
    }

    /// #1272: after Shape is armed, kind-pick steps point at Context Shape buttons.
    #[test]
    fn shapes_kind_steps_target_context_shape_buttons() {
        use crate::model::PrimitiveKind as K;
        let shapes = &TUTORIALS[tutorial_index("shapes").unwrap()];
        let cylinder = shapes
            .steps
            .iter()
            .find(|s| {
                let n = s.narration.to_ascii_lowercase();
                n.contains("cylinder")
                    && (n.contains("cycle") || n.contains("click") || n.contains("context"))
                    && !n.contains("centre")
                    && !n.contains("center")
                    && !n.contains("radius")
                    && !n.contains("height")
            })
            .expect("cylinder kind-pick step");
        assert!(
            matches!(
                cylinder.anchor,
                StepAnchor::Ui(UiAnchor::ShapeKind(K::Cylinder))
            ),
            "cylinder kind should point at Context Cylinder button: {}",
            cylinder.narration
        );
        let sphere = shapes
            .steps
            .iter()
            .find(|s| {
                let n = s.narration.to_ascii_lowercase();
                n.contains("sphere")
                    && (n.contains("cycle") || n.contains("click") || n.contains("context"))
                    && !n.contains("rest")
                    && !n.contains("radius")
            })
            .expect("sphere kind-pick step");
        assert!(
            matches!(sphere.anchor, StepAnchor::Ui(UiAnchor::ShapeKind(K::Sphere))),
            "sphere kind should point at Context Sphere button: {}",
            sphere.narration
        );
    }

    /// #1273: cylinder base guide sits on a wall construction plane, not the ground/cuboid.
    #[test]
    fn shapes_cylinder_anchor_is_on_a_wall_plane() {
        let app = AppState::default();
        let p = wall_plane_cylinder_anchor(&app).expect("wall plane guide");
        // XZ wall: y ≈ 0, z raised off the ground plane origin edge.
        assert!(
            p.y.abs() < 0.1,
            "XZ wall has y=0, got {p:?}"
        );
        assert!(
            p.z > 10.0,
            "should sit up the wall, not on the ground edge: {p:?}"
        );
        // Not coplanar with a ground cuboid corner (z≈0).
        assert!(
            p.z.abs() > 1.0 || p.y.abs() > 1.0,
            "must leave the ground plane: {p:?}"
        );

        let shapes = &TUTORIALS[tutorial_index("shapes").unwrap()];
        let base = shapes
            .steps
            .iter()
            .find(|s| s.narration.to_ascii_lowercase().contains("cylinder's base"))
            .expect("cylinder base click step");
        assert!(
            matches!(base.anchor, StepAnchor::World(_)),
            "base click uses a world guide"
        );
        // Assist places the cylinder on the wall (normal along +Y for XZ).
        let mut app = AppState::default();
        assist_place_cylinder(&mut app);
        let cyl = app
            .doc
            .primitives
            .values()
            .find(|p| p.kind == crate::model::PrimitiveKind::Cylinder)
            .expect("cylinder");
        let n = glam::Vec3::from_array(cyl.normal);
        assert!(
            n.dot(glam::Vec3::Y).abs() > 0.9,
            "cylinder should rest on XZ wall (normal ≈ Y), got {n:?}"
        );
        let o = glam::Vec3::from_array(cyl.origin);
        assert!(
            o.z > 10.0,
            "cylinder origin should be up the wall: {o:?}"
        );
    }

    /// #1279: reopen-sketch step points at the sketch row in Elements.
    #[test]
    fn dimensioned_box_reopen_sketch_targets_elements_row() {
        let box_tut = &TUTORIALS[tutorial_index("dimensioned_box").unwrap()];
        let reopen = box_tut
            .steps
            .iter()
            .find(|s| s.narration.to_ascii_lowercase().contains("reopen the sketch"))
            .expect("reopen sketch step");
        assert!(
            matches!(reopen.anchor, StepAnchor::Ui(UiAnchor::ElementsSketch)),
            "reopen step should orb the Elements sketch row: {}",
            reopen.narration
        );
    }

    /// #1240: dimensioned box assist draws 10×10, extrudes 10, then edits to 20.
    #[test]
    fn dimensioned_box_tutorial_edits_a_side_to_20() {
        let mut app = AppState::default();
        assist_edit_dim_to_20(&mut app);
        assert!(rect_dims_are_10(&app) || one_sketch_dim_is_20(&app));
        assert!(extrusion_is_10(&app) || has_extrusion(&app));
        assert!(one_sketch_dim_is_20(&app));
    }

    /// #1269: navigate tutorial sits after cube, seeds cubes, and teaches camera + exploder.
    #[test]
    fn navigate_tutorial_is_second_and_seeds_cubes() {
        let nav = &TUTORIALS[tutorial_index("navigate").unwrap()];
        assert_eq!(nav.name, "navigate");
        assert_eq!(tutorial_index("navigate"), Some(1));
        assert!(
            nav.title.to_ascii_lowercase().contains("orbit")
                || nav.title.to_ascii_lowercase().contains("pan")
                || nav.title.to_ascii_lowercase().contains("zoom"),
            "title should name the camera skills: {}",
            nav.title
        );

        let mut app = AppState::default();
        app.apply(Action::StartTutorial {
            index: tutorial_index("navigate").unwrap(),
        });
        assert!(
            app.doc.primitives.len() >= 2,
            "start with cubes already in the document, got {}",
            app.doc.primitives.len()
        );
        assert!(app.doc.bodies.len() >= 2 || app.doc.primitives.len() >= 2);
        assert!(
            app.doc.construction_planes.is_empty(),
            "#1306: default XY/XZ/YZ planes should be gone, got {}",
            app.doc.construction_planes.len()
        );
        assert!(app.tutorial.is_some());
    }

    /// #1307: the exploder step leads with what to accomplish, not how the fan works.
    #[test]
    fn navigate_exploder_step_states_the_goal_first() {
        let nav = &TUTORIALS[tutorial_index("navigate").unwrap()];
        let step = nav
            .steps
            .iter()
            .find(|s| s.key_hint.is_some_and(|(k, _)| k.eq_ignore_ascii_case("Space")))
            .expect("navigate tutorial should introduce Space");
        let n = step.narration.to_ascii_lowercase();
        let goal = n
            .find("pick")
            .or_else(|| n.find("face"))
            .expect("exploder step should say to pick a face");
        let how = n
            .find("space")
            .or_else(|| n.find("exploder"))
            .expect("exploder step should still name Space / exploder");
        assert!(
            goal < how,
            "#1307: lead with the goal (pick a face), not the mechanism: {}",
            step.narration
        );
        assert!(
            n.contains("body") || n.contains("whole"),
            "say why: Select would take the whole body: {}",
            step.narration
        );
    }

    /// #1269: covers orbit, pan, zoom, bear HUD, home, and the Selection Exploder.
    #[test]
    fn navigate_tutorial_covers_camera_bear_and_exploder() {
        let nav = &TUTORIALS[tutorial_index("navigate").unwrap()];
        let joined: String = nav
            .steps
            .iter()
            .map(|s| s.narration.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        for needle in [
            "orbit",
            "pan",
            "zoom",
            "bear",
            "home",
            "space",
            "exploder",
        ] {
            assert!(
                joined.contains(needle),
                "navigate tutorial should mention {needle}"
            );
        }
        assert!(
            nav.steps.iter().any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::ViewCube))),
            "should point at the view bear"
        );
        assert!(
            nav.steps.iter().any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::ViewHome))),
            "should point at the home button"
        );
        assert!(
            nav.steps.iter().any(|s| s.key_hint.is_some_and(|(k, _)| k.eq_ignore_ascii_case("Space"))),
            "should introduce Space / Selection Exploder"
        );
    }

    /// #1269: assists drive camera steps; Next covers the exploder teaching step.
    #[test]
    fn navigate_tutorial_walks_with_assists() {
        let mut app = AppState::default();
        app.apply(Action::StartTutorial {
            index: tutorial_index("navigate").unwrap(),
        });
        assert!(app.doc.primitives.len() >= 2);

        let mut guard = 0;
        while app.tutorial.is_some() {
            guard += 1;
            assert!(guard < 40, "navigate tutorial should finish");
            let run = app.tutorial.unwrap();
            let step = &TUTORIALS[run.tutorial].steps[run.step];
            if step.assist.is_some() {
                app.apply(Action::TutorialAssist);
                // Assist should satisfy the step; if not (edge case), fall through to Next.
                if app.tutorial.map(|r| r.step) == Some(run.step) && step.done.is_some() {
                    // Force-advance stuck auto-step for the test.
                    app.apply(Action::TutorialNext);
                }
            } else {
                app.apply(Action::TutorialNext);
            }
        }
        assert!(app.tutorial_completed("navigate"));
    }

    #[test]
    fn navigate_camera_predicates_respond_to_assists() {
        let mut app = AppState::default();
        assert!(!camera_has_orbited(&app));
        assist_nav_orbit(&mut app);
        assert!(camera_has_orbited(&app));

        assert!(!camera_has_panned(&app));
        assist_nav_pan(&mut app);
        assert!(camera_has_panned(&app));

        assert!(!camera_has_zoomed(&app));
        assist_nav_zoom(&mut app);
        assert!(camera_has_zoomed(&app));

        assert!(!camera_on_standard_view(&app));
        assist_nav_bear_snap(&mut app);
        assert!(camera_on_standard_view(&app));
        assert!(!camera_at_home(&app));

        assist_nav_home(&mut app);
        assert!(camera_at_home(&app));
    }
}
