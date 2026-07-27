//! Interactive tutorial mode: Bear (the view cube) walks a first-time user through
//! building a real part, pointing with glowing rings and narrating in a speech
//! bubble. Tutorials live in a registry ([`TUTORIALS`]) so more can be added; each
//! is a list of [`Step`]s that either auto-advance when a document predicate is
//! satisfied or wait for the bubble's Next button.

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
    /// What the button does, computed from the live state so it only fills in
    /// whatever the user hasn't already done themselves.
    pub actions: fn(&AppState) -> Vec<Action>,
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
    /// A `(key, explanation)` badge under the orb (#777) — used to introduce **Space**, the
    /// Selection Exploder, on steps whose target sits under other geometry.
    pub key_hint: Option<(&'static str, &'static str)>,
    /// The words this step wants typed (#778), shown in code blue beside the orb — right
    /// where the typing lands.
    pub type_hint: Option<TypeHint>,
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

pub static TUTORIALS: &[Tutorial] = &[Tutorial {
    name: "bracket",
    title: "Build an angle bracket",
    steps: BRACKET_STEPS,
}];

pub fn tutorial_index(name: &str) -> Option<usize> {
    TUTORIALS.iter().position(|t| t.name == name)
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
    app.doc.constraints.iter().filter(|c| !c.deleted)
}

fn param_exists(app: &AppState, name: &str) -> bool {
    app.doc
        .parameters
        .iter()
        .any(|p| !p.deleted && p.name.eq_ignore_ascii_case(name))
}

fn name_box_tapped(app: &AppState) -> bool {
    app.parameters_pane.new_name_focused
        || !app.parameters_pane.new_name.trim().is_empty()
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

/// Every number the bracket is built from, in the order the tutorial introduces them.
const BRACKET_PARAMS: [(&str, &str); 6] = [
    ("leg", "50mm"),
    ("width", "40mm"),
    ("thick", "5mm"),
    ("hole", "5mm"),
    ("bend", "4mm"),
    ("bend_angle", "120deg"),
];

fn params_defined(app: &AppState) -> bool {
    BRACKET_PARAMS.iter().all(|(name, _)| param_exists(app, name))
}

/// The "Add them for me" button: adds whichever bracket parameters are still
/// missing, leaving any the user already typed (or renamed the value of) alone.
fn add_missing_params(app: &AppState) -> Vec<Action> {
    BRACKET_PARAMS
        .iter()
        .filter(|(name, _)| !param_exists(app, name))
        .map(|(name, expression)| Action::AddParameter {
            name: name.to_string(),
            expression: expression.to_string(),
        })
        .collect()
}

/// The next bracket parameter still missing, as "name = value" (#782) — what the parameter
/// list step wants typed next.
fn next_missing_param(app: &AppState) -> Option<String> {
    BRACKET_PARAMS
        .iter()
        .find(|(name, _)| !param_exists(app, name))
        .map(|(name, value)| format!("{name} = {value}"))
}

fn line_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Line
}

/// The sloppy bracket profile the tutorial leads the user around, in sketch-local
/// millimetres (mirrors the quickstart's rough hexagon; the constraint steps square
/// it up afterwards).
const PROFILE_POINTS: [(f32, f32); 6] = [
    (0.0, 0.0),
    (51.0, 2.5),
    (49.5, 7.8),
    (4.5, 5.5),
    (-17.5, 47.0),
    (-25.5, 43.0),
];

/// The next profile vertex to click while drawing the sloppy outline: follows the
/// chain (placed lines + the in-progress segment) and finally points back at the
/// start to close the loop.
fn next_profile_point(app: &AppState) -> Option<glam::Vec3> {
    // No sketch open yet: the first click is on the ground plane itself — point there.
    let Some(session) = app.sketch_session else {
        return Some(glam::Vec3::ZERO);
    };
    let frame = crate::face::sketch_geometry_frame(&app.doc, session.sketch)?;
    let placed = app
        .doc
        .lines
        .iter()
        .filter(|l| !l.deleted && l.sketch == session.sketch && !l.construction)
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
    app.doc.lines.iter().filter(|l| !l.deleted && !l.construction).count() >= 6
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
    app.cam.frame_bounds_animated(
        glam::Vec3::new(-35.0, -10.0, 0.0),
        glam::Vec3::new(60.0, 55.0, 10.0),
        app.viewport_aspect,
        0.35,
    );
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
    /// The start corner of the nth profile line (shared with the line before it).
    ProfileCorner(usize),
    /// The sketch's origin point.
    Origin,
    /// The sketch's red X axis.
    XAxis,
}

fn sketch_frame(app: &AppState) -> Option<crate::face::SketchFrame> {
    let session = app.sketch_session?;
    crate::face::sketch_geometry_frame(&app.doc, session.sketch)
}

/// Document indices of the open sketch's drawn lines, in creation order.
fn profile_lines(app: &AppState) -> Vec<usize> {
    let Some(session) = app.sketch_session else {
        return Vec::new();
    };
    app.doc
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.deleted && l.sketch == session.sketch && !l.construction)
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

/// Where the orb sits for a target: a line's middle, a corner's vertex, the origin, or a
/// clear stretch of the X axis away from the profile.
fn target_point(app: &AppState, target: ClickTarget) -> Option<glam::Vec3> {
    match target {
        ClickTarget::ProfileLine(n) => polyline_midpoint(&profile_polyline(app, n)?),
        ClickTarget::ProfileCorner(n) => profile_polyline(app, n)?.first().copied(),
        ClickTarget::Origin => {
            Some(crate::face::local_to_world(&sketch_frame(app)?, 0.0, 0.0))
        }
        ClickTarget::XAxis => {
            Some(crate::face::local_to_world(&sketch_frame(app)?, -22.0, 0.0))
        }
    }
}

/// Whether a selected point sits on `world` (which line's endpoint it counts as doesn't
/// matter — the corner is shared).
fn point_selected_at(app: &AppState, world: glam::Vec3) -> bool {
    use crate::hierarchy::SceneElement;
    app.scene_selection.iter().any(|element| match element {
        SceneElement::Point(cp) => crate::construction::point_world_position(&app.doc, cp)
            .is_some_and(|p| (p - world).length() < 0.5),
        SceneElement::Origin => (crate::face::local_to_world(
            &match sketch_frame(app) {
                Some(f) => f,
                None => return false,
            },
            0.0,
            0.0,
        ) - world)
            .length()
            < 0.5,
        _ => false,
    })
}

fn target_selected(app: &AppState, target: ClickTarget) -> bool {
    use crate::hierarchy::SceneElement;
    use crate::model::{ConstraintLine, SketchAxis};
    match target {
        ClickTarget::ProfileLine(n) => profile_lines(app)
            .get(n)
            .is_some_and(|i| app.scene_selection.is_selected(SceneElement::Line(*i))),
        ClickTarget::ProfileCorner(_) | ClickTarget::Origin => {
            target_point(app, target).is_some_and(|w| point_selected_at(app, w))
        }
        ClickTarget::XAxis => app
            .scene_selection
            .is_selected(SceneElement::FaceEdge(ConstraintLine::OriginAxis(SketchAxis::X))),
    }
}

/// Whether `element` is one of the two things this step asks for.
fn element_is_target(app: &AppState, element: &crate::hierarchy::SceneElement, target: ClickTarget) -> bool {
    use crate::hierarchy::SceneElement;
    use crate::model::{ConstraintLine, SketchAxis};
    match target {
        ClickTarget::ProfileLine(n) => matches!(element, SceneElement::Line(i)
            if profile_lines(app).get(n) == Some(i)),
        ClickTarget::ProfileCorner(_) | ClickTarget::Origin => match element {
            SceneElement::Origin => matches!(target, ClickTarget::Origin),
            SceneElement::Point(cp) => target_point(app, target).is_some_and(|w| {
                crate::construction::point_world_position(&app.doc, cp.clone())
                    .is_some_and(|p| (p - w).length() < 0.5)
            }),
            _ => false,
        },
        ClickTarget::XAxis => matches!(
            element,
            SceneElement::FaceEdge(ConstraintLine::OriginAxis(SketchAxis::X))
        ),
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

/// Generates a step's orb-target and Shift-hint functions for a two-click pair (the step
/// table needs plain `fn` pointers, so each pair gets its own pair of functions). Once both
/// picks are in hand the orb moves to the pane button that applies the constraint (#770) —
/// the last thing left to do.
macro_rules! constraint_step {
    ($point:ident, $shift:ident, $a:expr, $b:expr, $kind:expr) => {
        fn $point(app: &AppState) -> Option<StepTarget> {
            match constraint_click_point(app, $a, $b) {
                Some(world) => Some(StepTarget::World(world)),
                None => Some(StepTarget::Ui(UiAnchor::ConstraintButton($kind))),
            }
        }
        fn $shift(app: &AppState) -> bool {
            constraint_needs_shift(app, $a, $b)
        }
    };
}

// The profile is drawn as six lines: 0 base bottom, 1 base end cap, 2 inner base,
// 3 tilted leg outer, 4 tilted leg end cap, 5 tilted leg inner (back to the bend corner).
use crate::geometric_constraints::GeometricConstraintType as GC;
constraint_step!(
    pin_click,
    pin_shift,
    ClickTarget::ProfileCorner(0),
    ClickTarget::Origin,
    GC::Coincident
);
constraint_step!(
    level_click,
    level_shift,
    ClickTarget::ProfileLine(0),
    ClickTarget::XAxis,
    GC::Parallel
);
constraint_step!(
    base_strip_click,
    base_strip_shift,
    ClickTarget::ProfileLine(0),
    ClickTarget::ProfileLine(2),
    GC::Parallel
);
constraint_step!(
    legs_click,
    legs_shift,
    ClickTarget::ProfileLine(3),
    ClickTarget::ProfileLine(5),
    GC::Parallel
);
constraint_step!(
    cap_one_click,
    cap_one_shift,
    ClickTarget::ProfileLine(1),
    ClickTarget::ProfileLine(0),
    GC::Perpendicular
);
constraint_step!(
    cap_two_click,
    cap_two_shift,
    ClickTarget::ProfileLine(4),
    ClickTarget::ProfileLine(3),
    GC::Perpendicular
);

// The squaring-up steps, one constraint application each. Every predicate is cumulative
// (each includes the ones before it), so a user who works ahead skips ahead and Back
// reviews hold their ground.

fn bend_pinned(app: &AppState) -> bool {
    // Specifically a coincidence WITH THE ORIGIN — the endpoint-joining coincidences the
    // drawing phase snaps into place don't count as pinning the profile down.
    use crate::model::ConstraintEntity;
    constraint_count(app, |k| {
        matches!(k, ConstraintKind::Coincident { a, b }
            if matches!(a, ConstraintEntity::Origin) || matches!(b, ConstraintEntity::Origin))
    }) >= 1
}

fn base_leveled(app: &AppState) -> bool {
    bend_pinned(app) && constraint_count(app, axis_parallel_kind) >= 1
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
/// spot to click to drop the dimension there (#779), and nothing once it's dimensioned.
fn dimension_line_orb(app: &AppState, nth: usize) -> Option<StepTarget> {
    if line_has_length_dim(app, nth) {
        return None;
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
    typed_value_hint(app, "thick")
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
    app.doc.extrusions.iter().any(|e| !e.deleted)
}

/// Count treated edges of `kind` across both the first-class edge-treatment operations (#531)
/// and any legacy extrusion-baked treatments (old files), so tutorial progress tracks either.
fn edge_treatment_count(app: &AppState, kind: VertexTreatmentKind) -> usize {
    let ops: usize = app
        .doc
        .edge_treatment_ops
        .iter()
        .filter(|o| !o.deleted && o.kind == kind)
        .map(|o| o.edges.len())
        .sum();
    let legacy = app
        .doc
        .extrusions
        .iter()
        .filter(|e| !e.deleted)
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

fn hole_circles_drawn(app: &AppState) -> bool {
    app.doc.circles.iter().filter(|c| !c.deleted && !c.construction).count() >= 2
}

fn cut_extrusion_count(app: &AppState) -> usize {
    app.doc
        .bodies
        .iter()
        .filter(|b| !b.deleted)
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
    app.doc.sketch_texts.iter().any(|t| !t.deleted) && cut_extrusion_count(app) >= 2
}

fn bend_angle_changed(app: &AppState) -> bool {
    crate::value::eval_angle_rad_in_doc("bend_angle", &app.doc)
        .is_some_and(|rad| (rad.to_degrees() - 120.0).abs() > 1.0)
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
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "First, a name for our first number. See the Parameters pane on the \
                    right? Tap inside the name box \u{2014} the pulsing ring marks it.",
        anchor: StepAnchor::Ui(UiAnchor::ParametersName),
        done: Some(name_box_tapped),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "Type `leg` \u{2014} just those three letters. It's the length of each \
                    of the bracket's legs.",
        anchor: StepAnchor::Ui(UiAnchor::ParametersName),
        done: Some(name_says_leg),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: Some(TypeHint::Fixed("leg")),
    },
    Step {
        narration: "Now tap the value box beside it and type `50mm`.",
        anchor: StepAnchor::Ui(UiAnchor::ParametersValue),
        done: Some(value_says_50),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: Some(TypeHint::Fixed("50mm")),
    },
    Step {
        narration: "Press + to add it. Your first parameter!",
        anchor: StepAnchor::Ui(UiAnchor::ParametersAdd),
        done: Some(leg_added),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "Five more, exactly the same moves:\n\
                    `width` = `40mm`\n`thick` = `5mm`\n`hole` = `5mm`\n`bend` = `4mm`\n\
                    `bend_angle` = `120deg`\n\
                    \u{2014} or let me type them in for you.",
        anchor: StepAnchor::Ui(UiAnchor::ParametersName),
        done: Some(params_defined),
        on_enter: None,
        assist: Some(StepAssist { label: "Add them for me", actions: add_missing_params }),
        needs_shift: None,
        key_hint: None,
        type_hint: Some(TypeHint::Dynamic(next_missing_param)),
    },
    Step {
        narration: "Grab the Line tool \u{2014} the glowing button up top, or press L.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Line)),
        done: Some(line_tool_active),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "I've brought us in over the drawing area. Now click each glowing point \
                    in turn to draw a loose sketch.",
        anchor: StepAnchor::World(next_profile_point),
        done: Some(profile_drawn),
        on_enter: Some(frame_profile_area),
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "Now the Constraint tool \u{2014} the glowing button, or press C.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Constraint)),
        done: Some(constraint_tool_active),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "Pin the profile down: click the bend corner, Shift+click the origin, \
                    press `4` \u{2014} Coincident.",
        anchor: StepAnchor::Guided(pin_click),
        done: Some(bend_pinned),
        on_enter: None,
        assist: None,
        needs_shift: Some(pin_shift),
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "Level the base: click the bottom line, Shift+click the red X axis, \
                    press `1` \u{2014} Parallel.",
        anchor: StepAnchor::Guided(level_click),
        done: Some(base_leveled),
        on_enter: None,
        assist: None,
        needs_shift: Some(level_shift),
        key_hint: Some((
            "Space",
            "fans out whatever is crowded under the cursor",
        )),
        type_hint: None,
    },
    Step {
        narration: "Click the bottom line, Shift+click the inner base line, press `1`.",
        anchor: StepAnchor::Guided(base_strip_click),
        done: Some(base_strip_even),
        on_enter: None,
        assist: None,
        needs_shift: Some(base_strip_shift),
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "The tilted leg: click one long line, Shift+click the other, \
                    press `1`.",
        anchor: StepAnchor::Guided(legs_click),
        done: Some(legs_parallel),
        on_enter: None,
        assist: None,
        needs_shift: Some(legs_shift),
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "Click the base leg's end cap, Shift+click the bottom line, press `2` \
                    \u{2014} Perpendicular.",
        anchor: StepAnchor::Guided(cap_one_click),
        done: Some(first_cap_squared),
        on_enter: None,
        assist: None,
        needs_shift: Some(cap_one_shift),
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "Click the tilted leg's end cap, Shift+click its long line, \
                    press `2`. Squared up!",
        anchor: StepAnchor::Guided(cap_two_click),
        done: Some(profile_squared),
        on_enter: None,
        assist: None,
        needs_shift: Some(cap_two_shift),
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "Now exact sizes. Grab the Dimension tool \u{2014} the glowing button, \
                    or press `D`.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Dimension)),
        done: Some(dimension_tool_active),
        on_enter: Some(clear_selection_for_dimensioning),
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "Click the glowing line, move the mouse to place the dimension, click \
                    again to drop it there, then type `leg` and press Enter.",
        anchor: StepAnchor::Guided(base_leg_orb),
        done: Some(base_leg_dimensioned),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: Some(TypeHint::Dynamic(leg_value_hint)),
    },
    Step {
        narration: "The other outer leg, the same way: click, place, type `leg`, Enter.",
        anchor: StepAnchor::Guided(tilted_leg_orb),
        done: Some(tilted_leg_dimensioned),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: Some(TypeHint::Dynamic(leg_value_hint)),
    },
    Step {
        narration: "Now an end cap \u{2014} that's the bracket's thickness: click, place, \
                    type `thick`, Enter.",
        anchor: StepAnchor::Guided(base_cap_orb),
        done: Some(base_cap_dimensioned),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: Some(TypeHint::Dynamic(thick_value_hint)),
    },
    Step {
        narration: "And the other end cap: `thick` again.",
        anchor: StepAnchor::Guided(tilted_cap_orb),
        done: Some(tilted_cap_dimensioned),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: Some(TypeHint::Dynamic(thick_value_hint)),
    },
    Step {
        narration: "Last one, the bend: click the bottom line, Shift+click the inner leg \
                    line, place the arc, then type `bend_angle` and press Enter.",
        anchor: StepAnchor::Guided(bend_angle_orb),
        done: Some(profile_dimensioned),
        on_enter: None,
        assist: None,
        needs_shift: Some(bend_angle_shift),
        key_hint: None,
        type_hint: Some(TypeHint::Dynamic(bend_angle_value_hint)),
    },
    Step {
        narration: "Esc to leave the sketch, then Extrude (E): click the profile face, type \
                    `width`, press Enter. A solid!",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Extrude)),
        done: Some(extruded),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: Some(TypeHint::Fixed("width")),
    },
    Step {
        narration: "Round the bend with Fillet (F): click the inside edge of the bend and \
                    type `bend`. Then the outside edge: `bend + thick`. Concentric, like bent \
                    sheet metal.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Fillet)),
        done: Some(bend_rounded),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "Screw holes! Sketch (S) on the inside face of the base flange, then \
                    Circle (O): place two circles near the flange tip, typing `hole` for each \
                    diameter. Position them with the Dimension tool (D) against the face \
                    edges.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Sketch)),
        done: Some(hole_circles_drawn),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "Esc, then Extrude (E): click both circles, drag the handle into the \
                    bracket (or type `thick + 1`), pick Cut, press Enter.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Extrude)),
        done: Some(holes_cut),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "Countersink them: Chamfer (K), click one hole's rim where it meets the \
                    face, Shift+click the other, type `1.2`, Enter.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Chamfer)),
        done: Some(holes_countersunk),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "Fillet (F) again: click a vertical edge at a flange tip, Shift+click the \
                    other corners, type `2`, Enter. Rounded corners!",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Fillet)),
        done: Some(corners_rounded),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "Sign your work: Text (T) on the outer face of the base, type `BearCAD`. \
                    Then Extrude (E) the text, push the handle into the face (type `1`), pick \
                    Cut \u{2014} engraved letters.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Text)),
        done: Some(label_engraved),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "The best part: in the Parameters pane, change `bend_angle` from `120deg` \
                    to `150deg`. The whole part rebuilds \u{2014} bend, holes, countersinks \
                    and all.",
        anchor: StepAnchor::Ui(UiAnchor::ParametersAdd),
        done: Some(bend_angle_changed),
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: None,
    },
    Step {
        narration: "You built it! Export via File \u{2192} Export \u{2192} STL or STEP. \
                    That's the whole loop: sketch, constrain, dimension, extrude, refine \u{2014} \
                    and parameters drive everything. See you around the viewport!",
        anchor: StepAnchor::None,
        done: None,
        on_enter: None,
        assist: None,
        needs_shift: None,
        key_hint: None,
        type_hint: None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::Action;

    /// The bracket tutorial auto-advances as a scripted build satisfies each step's
    /// predicate, from parameters through the final angle change.
    #[test]
    fn bracket_predicates_track_a_scripted_build() {
        let mut app = AppState::default();
        app.tutorial = Some(TutorialRun { tutorial: 0, step: 1, hold: false });

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
        app.apply(Action::CommitParameterExpression {
            index: 5,
            expression: "150deg".to_string(),
        });
        assert!(bend_angle_changed(&app));
    }

    /// Back reviews earlier steps without auto-advance re-firing on their already-
    /// satisfied predicates; Next resumes auto mode once it reaches unfinished work.
    #[test]
    fn back_reviews_without_auto_advance_snapping_forward() {
        let mut app = AppState::default();
        app.apply(Action::StartTutorial { index: 0 });
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
        assert_eq!(app.tutorial.unwrap().step, 6, "params chain to the line-tool step");

        app.apply(Action::TutorialBack);
        let run = app.tutorial.unwrap();
        assert_eq!(run.step, 5);
        assert!(run.hold);
        // Its predicate is satisfied, but reviewing holds auto-advance off.
        app.advance_tutorial();
        assert_eq!(app.tutorial.unwrap().step, 5);

        // Next walks forward; reaching the line-tool step (unfinished) resumes auto.
        app.apply(Action::TutorialNext);
        let run = app.tutorial.unwrap();
        assert_eq!(run.step, 6);
        assert!(!run.hold, "caught up to live work — auto-advance resumes");
        app.apply(Action::SetTool(Tool::Line));
        assert_eq!(app.tutorial.unwrap().step, 7, "auto-advance is live again");
    }

    /// The parameters step's assist button fills in the whole table in one press —
    /// and adding only what's missing, so a user who typed a couple by hand keeps them.
    #[test]
    fn assist_button_adds_the_remaining_parameters() {
        let mut app = AppState::default();
        app.apply(Action::StartTutorial { index: 0 });
        app.apply(Action::AddParameter {
            name: "leg".to_string(),
            expression: "60mm".to_string(),
        });
        app.apply(Action::TutorialAssist); // welcome/name steps have no assist: a no-op
        assert!(!params_defined(&app), "no assist on the steps before the list");

        // Walk to the step whose narration lists the five remaining parameters.
        let step = BRACKET_STEPS.iter().position(|s| s.assist.is_some()).unwrap();
        app.tutorial = Some(TutorialRun { tutorial: 0, step, hold: false });
        app.parameters_pane.new_name = "wid".to_string();
        app.apply(Action::TutorialAssist);

        assert!(params_defined(&app));
        assert!(app.parameters_pane.new_name.is_empty(), "the draft row is cleared");
        let leg = app.doc.parameters.iter().find(|p| p.name == "leg").unwrap();
        assert_eq!(leg.expression, "60mm", "a hand-typed value is left alone");
        assert!(app.tutorial.unwrap().step > step, "the step auto-advances as usual");
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
        use crate::model::{ConstraintLine, FaceId, SketchAxis};

        let mut app = AppState::default();
        app.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
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

        let world = |app: &AppState| match level_click(app) {
            Some(StepTarget::World(w)) => Some(w),
            _ => None,
        };

        // Nothing picked: point at the *middle* of the bottom line (#769), no Shift yet.
        let first = world(&app).expect("orb points at the first click");
        assert!(!level_shift(&app));
        let poly = profile_polyline(&app, 0).unwrap();
        assert!(
            (first - poly[0].lerp(poly[1], 0.5)).length() < 1e-3,
            "the orb sits mid-line, not on an endpoint: {first:?}"
        );

        // A wrong pick doesn't count — the orb stays on the line still wanted, and the
        // click that clears it is Shift-free (#785).
        app.scene_selection.insert(SceneElement::Line(lines[2]));
        assert!(world(&app).is_some_and(|p| (p - first).length() < 1e-3));
        assert!(!level_shift(&app));

        // Even with the *right* line picked, a stray from an earlier step means starting
        // over with a plain click on the first target.
        app.scene_selection.insert(SceneElement::Line(lines[0]));
        assert!(world(&app).is_some_and(|p| (p - first).length() < 1e-3), "back to the first pick");
        assert!(!level_shift(&app), "no Shift while a stray is selected");
        app.scene_selection.clear();

        // The right line: now the orb moves to the X axis and asks for Shift.
        app.scene_selection.insert(SceneElement::Line(lines[0]));
        let second = world(&app).expect("orb points at the second click");
        assert!((second - first).length() > 1.0, "it moved");
        assert!(level_shift(&app), "the second click of a pair holds Shift");

        // Both in hand: the orb moves to the pane button that applies it (#770).
        app.scene_selection
            .insert(SceneElement::FaceEdge(ConstraintLine::OriginAxis(SketchAxis::X)));
        assert!(matches!(
            level_click(&app),
            Some(StepTarget::Ui(UiAnchor::ConstraintButton(GC::Parallel)))
        ));
        assert!(!level_shift(&app));
    }

    /// #765: the web app's `?tutorial=` parameter names a registered tutorial.
    #[test]
    fn tutorial_from_query_picks_a_registered_tutorial() {
        assert_eq!(tutorial_from_query("?tutorial=bracket"), Some(0));
        assert_eq!(tutorial_from_query("tutorial=bracket"), Some(0));
        assert_eq!(tutorial_from_query("?foo=1&tutorial=bracket&bar=2"), Some(0));
        assert_eq!(tutorial_from_query("?tutorial=nope"), None);
        assert_eq!(tutorial_from_query("?other=bracket"), None);
        assert_eq!(tutorial_from_query(""), None);
    }

    #[test]
    fn tutorial_registry_lookup_by_name() {
        assert_eq!(tutorial_index("bracket"), Some(0));
        assert_eq!(tutorial_index("nope"), None);
        assert!(TUTORIALS[0].steps.len() >= 10);
    }
}
