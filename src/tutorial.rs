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
//! the same step.

use crate::actions::{Action, AppState, Tool};
use crate::model::ConstraintKind;

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
    /// An existing parameter's **value** cell in the Parameters pane (#1347).
    ParametersExistingValue,
    /// A constraint button in the Context pane's Constraints list (#770) — where a
    /// squaring-up step points once both of its picks are made.
    #[allow(dead_code)]
    ConstraintButton(crate::geometric_constraints::GeometricConstraintType),
    /// The extrude Output row's **Cut** button (#804).
    #[allow(dead_code)]
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
    /// The status-bar Tutorials launcher (#1434): the launch prompt points here.
    TutorialsButton,
    /// A Combine-tool Mode button in the Context pane (#1556): Combine / Cut / Intersect / Difference.
    CombineKind(crate::model::BooleanOpKind),
    /// The Text tool's string field in the Context pane (#1557).
    TextContent,
}

/// What a step's glowing orb points at, once resolved against the live state.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub enum StepTarget {
    World(glam::Vec3),
    Ui(UiAnchor),
}

/// One mark of a step's numbered guide (#854): where it points and whether that part is
/// already done. A step whose work is a short sequence shows them all at once, numbered.
#[derive(Clone, Copy, Debug)]
pub struct GuideMark {
    pub target: StepTarget,
    pub done: bool,
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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

/// Launch tooltip on the Tutorials button (#1434).
pub const PROMPT_TEXT: &str = "Want to try some tutorials?";
/// Fresh-install window during which the launch tooltip may appear (#1434).
pub const PROMPT_WINDOW_DAYS: f64 = 30.0;
/// How long the tooltip stays fully visible after the user starts working (#1434).
pub const PROMPT_FADE_AFTER_SECS: f32 = 3.0;
/// Fade duration after [`PROMPT_FADE_AFTER_SECS`] (#1434).
pub const PROMPT_FADE_SECS: f32 = 0.8;
/// Label of the Tutorials pane button that marks every walkthrough finished.
pub const COMPLETE_ALL_LABEL: &str = "Mark all complete";
/// Label of the Tutorials pane button that clears every completion check.
pub const UNSTART_ALL_LABEL: &str = "Mark all unstarted";

/// The first-launch tooltip that points at the Tutorials button (#1434).
#[derive(Clone, Debug, PartialEq)]
pub struct TutorialPrompt {
    /// The user has started working on the document; the fade clock is running.
    pub work_started: bool,
    /// Seconds since [`Self::work_started`] became true.
    pub work_elapsed: f32,
}

impl TutorialPrompt {
    pub fn new() -> Self {
        Self {
            work_started: false,
            work_elapsed: 0.0,
        }
    }

    /// 1 while idle or during the hold; ramps to 0 over [`PROMPT_FADE_SECS`].
    pub fn alpha(&self) -> f32 {
        if !self.work_started || self.work_elapsed <= PROMPT_FADE_AFTER_SECS {
            1.0
        } else {
            let t = (self.work_elapsed - PROMPT_FADE_AFTER_SECS) / PROMPT_FADE_SECS;
            (1.0 - t).clamp(0.0, 1.0)
        }
    }

    /// Advance the fade clock. Returns `false` once the tooltip should be gone.
    pub fn tick(&mut self, dt: f32) -> bool {
        if self.work_started {
            self.work_elapsed += dt.max(0.0);
        }
        self.alpha() > 0.0
    }
}

pub struct Tutorial {
    /// Stable name for scripting (`bearcad.ui.tutorial("cube")`).
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
        title: "Sketch & Extrude",
        steps: CUBE_STEPS,
    },
    // Second walkthrough (#1269): camera, with cubes already in the document.
    Tutorial {
        name: "navigate",
        title: "Pan, orbit & zoom",
        steps: NAVIGATE_STEPS,
    },
    Tutorial {
        name: "shapes",
        title: "3D Bodies",
        steps: SHAPES_STEPS,
    },
    Tutorial {
        name: "dimensioned_box",
        title: "Dimensions",
        steps: DIMENSIONED_BOX_STEPS,
    },
    // #1347: named parameters, expressions on a rectangle, inline `name=value`.
    Tutorial {
        name: "parameters",
        title: "Parameters",
        steps: PARAMETERS_STEPS,
    },
    // #1555: sketch-chamfer a rectangle, extrude, chamfer the top of the solid.
    Tutorial {
        name: "chamfer",
        title: "Chamfer",
        steps: CHAMFER_STEPS,
    },
    // #1556: cut a sphere out of a cube with the Combine tool.
    Tutorial {
        name: "combine",
        title: "Combine",
        steps: COMBINE_STEPS,
    },
    // #1557: stamp raised letters on a cube.
    Tutorial {
        name: "raised_text",
        title: "Raised text",
        steps: RAISED_TEXT_STEPS,
    },
];

pub fn tutorial_index(name: &str) -> Option<usize> {
    TUTORIALS.iter().position(|t| t.name == name)
}

/// Pane / list label: catalog order as a 1-based number plus the skill title (#1558).
pub fn numbered_title(index: usize, title: &str) -> String {
    format!("{}. {title}", index + 1)
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

const fn keyed_assist_step(
    narration: &'static str,
    done: fn(&AppState) -> bool,
    key: &'static str,
    key_why: &'static str,
    assist: StepAssist,
) -> Step {
    Step {
        narration,
        anchor: StepAnchor::None,
        done: Some(done),
        on_enter: None,
        assist: Some(assist),
        needs_shift: None,
        drag_hint: None,
        key_hint: Some((key, key_why)),
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
/// `?tutorial=cube` opens the web app with that walkthrough already running, so a docs
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

// --- Short tutorials (#1238–#1240; atomic steps #1253) -----------------------------

fn live_constraints(app: &AppState) -> impl Iterator<Item = &crate::model::Constraint> {
    app.doc.constraints.values()
}

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

/// Dimension label opened for typing (or the 20 mm edit is already done).
fn dim_label_opened_for_edit(app: &AppState) -> bool {
    app.editing_committed_dim.is_some() || one_sketch_dim_is_20(app)
}

/// Convert a screen-pixel length at the look-at plane into world millimetres.
fn pixel_offset_world_mm(app: &AppState, pixels: f32) -> f32 {
    let h = app.viewport_height.max(1.0);
    let (_, half_h) = app.cam.viewport_half_extents(app.viewport_aspect.max(0.01));
    pixels * (2.0 * half_h / h)
}

/// Midpoint of the first sketch length-dimension label, offset off the line
/// by the same pixel distance the drawn label uses (#1332/#1333).
fn first_rect_dim_label(app: &AppState) -> Option<glam::Vec3> {
    use crate::model::DistanceTarget;
    let sketch = app
        .sketch_session
        .map(|s| s.sketch)
        .or_else(|| app.doc.sketches.keys().next())?;
    let frame = crate::face::sketch_geometry_frame(&app.doc, sketch)?;
    let line = live_constraints(app).find_map(|c| match &c.kind {
        ConstraintKind::Distance {
            target: DistanceTarget::LineLength(i),
        } => app.doc.lines.get(*i).filter(|l| l.sketch == sketch),
        _ => None,
    })?;
    let (ua, va, ub, vb) = (line.x0, line.y0, line.x1, line.y1);
    let mut sum = (0.0f32, 0.0f32);
    let mut n = 0usize;
    for l in app.doc.lines.values().filter(|l| l.sketch == sketch) {
        sum.0 += l.x0 + l.x1;
        sum.1 += l.y0 + l.y1;
        n += 2;
    }
    let (cx, cy) = if n > 0 {
        (sum.0 / n as f32, sum.1 / n as f32)
    } else {
        (0.0, 0.0)
    };
    let (ou, ov) = crate::dimensions::outward_perpendicular_uv(ua, va, ub, vb, cx, cy);
    let away = pixel_offset_world_mm(
        app,
        crate::dimensions::effective_dim_offset(line.length_dim_offset)
            + crate::dimensions::LABEL_OUTSET,
    );
    Some(crate::face::local_to_world(
        &frame,
        (ua + ub) * 0.5 + ou * away,
        (va + vb) * 0.5 + ov * away,
    ))
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

/// Non-construction lines of the tutorial's first sketch, in insertion order
/// (rectangle: bottom, right, top, left).
fn first_sketch_rect_lines(app: &AppState) -> Vec<crate::model::LineKey> {
    let Some(sketch) = app
        .sketch_session
        .map(|s| s.sketch)
        .or_else(|| app.doc.sketches.keys().next())
    else {
        return Vec::new();
    };
    app.doc
        .lines
        .iter()
        .filter(|(_, l)| l.sketch == sketch && !l.construction)
        .map(|(k, _)| k)
        .collect()
}

fn line_key_has_length_dim(app: &AppState, line: crate::model::LineKey) -> bool {
    use crate::model::DistanceTarget;
    live_constraints(app).any(|c| {
        matches!(
            &c.kind,
            ConstraintKind::Distance {
                target: DistanceTarget::LineLength(i),
            } if *i == line
        )
    })
}

fn dimension_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Dimension
}

/// Drop leftover picks before a dimensioning step — under that tool a live selection is
/// already a dimension in the making.
fn clear_selection_for_dimensioning(app: &mut AppState) {
    app.scene_selection.clear();
    app.placing_dimension = None;
}

fn rect_length_dim_count(app: &AppState) -> usize {
    first_sketch_rect_lines(app)
        .iter()
        .filter(|&&line| line_key_has_length_dim(app, line))
        .count()
}

fn rect_dim_target_matches(
    target: &crate::model::DimensionTarget,
    lines: &[crate::model::LineKey],
) -> bool {
    use crate::model::{DimensionTarget, DistanceTarget};
    matches!(
        target,
        DimensionTarget::Distance(DistanceTarget::LineLength(i)) if lines.contains(i)
    )
}

fn dimensioning_rect_line(app: &AppState) -> bool {
    let lines = first_sketch_rect_lines(app);
    app.placing_dimension
        .as_ref()
        .is_some_and(|p| rect_dim_target_matches(&p.target, &lines))
        || app
            .editing_committed_dim
            .as_ref()
            .and_then(|e| e.target.dimension_target(&app.doc))
            .is_some_and(|t| rect_dim_target_matches(&t, &lines))
}

fn dimensioning_new_rect_line(app: &AppState) -> bool {
    use crate::model::{DimensionTarget, DistanceTarget};
    let lines = first_sketch_rect_lines(app);
    let is_new = |target: &DimensionTarget| {
        matches!(
            target,
            DimensionTarget::Distance(DistanceTarget::LineLength(i))
                if lines.contains(i) && !line_key_has_length_dim(app, *i)
        )
    };
    app.placing_dimension
        .as_ref()
        .is_some_and(|p| is_new(&p.target))
        || matches!(
            &app.editing_committed_dim.as_ref().map(|e| &e.target),
            Some(crate::actions::DimEditTarget::New(t)) if is_new(t)
        )
}

fn first_rect_side_picked(app: &AppState) -> bool {
    rect_length_dim_count(app) >= 1 || dimensioning_rect_line(app)
}

fn first_rect_dim_open(app: &AppState) -> bool {
    rect_length_dim_count(app) >= 1 || app.editing_committed_dim.is_some()
}

fn first_rect_dim_is_10(app: &AppState) -> bool {
    distance_dims_near(app, 10.0, 1)
}

fn second_rect_side_picked(app: &AppState) -> bool {
    rect_length_dim_count(app) >= 2 || dimensioning_new_rect_line(app)
}

fn second_rect_dim_open(app: &AppState) -> bool {
    rect_length_dim_count(app) >= 2
        || (rect_length_dim_count(app) >= 1
            && matches!(
                app.editing_committed_dim.as_ref().map(|e| &e.target),
                Some(crate::actions::DimEditTarget::New(_))
            ))
}

fn sketch_exited(app: &AppState) -> bool {
    app.sketch_session.is_none()
}

fn zoomed_to_fit(app: &AppState) -> bool {
    app.status.starts_with("Zoomed to")
}

fn rect_side_mid(app: &AppState, nth: usize) -> Option<glam::Vec3> {
    let line = *first_sketch_rect_lines(app).get(nth)?;
    let l = app.doc.lines.get(line)?;
    let frame = crate::face::sketch_geometry_frame(&app.doc, l.sketch)?;
    Some(crate::face::local_to_world(
        &frame,
        (l.x0 + l.x1) * 0.5,
        (l.y0 + l.y1) * 0.5,
    ))
}

fn rect_side_drop_spot(app: &AppState, nth: usize) -> Option<glam::Vec3> {
    let line = *first_sketch_rect_lines(app).get(nth)?;
    let l = app.doc.lines.get(line)?;
    let frame = crate::face::sketch_geometry_frame(&app.doc, l.sketch)?;
    let (ua, va, ub, vb) = (l.x0, l.y0, l.x1, l.y1);
    let mut sum = (0.0f32, 0.0f32);
    let mut n = 0usize;
    for ln in app.doc.lines.values().filter(|ln| ln.sketch == l.sketch) {
        sum.0 += ln.x0 + ln.x1;
        sum.1 += ln.y0 + ln.y1;
        n += 2;
    }
    let (cx, cy) = if n > 0 {
        (sum.0 / n as f32, sum.1 / n as f32)
    } else {
        (0.0, 0.0)
    };
    let (ou, ov) = crate::dimensions::outward_perpendicular_uv(ua, va, ub, vb, cx, cy);
    const AWAY_MM: f32 = 11.0;
    Some(crate::face::local_to_world(
        &frame,
        (ua + ub) * 0.5 + ou * AWAY_MM,
        (va + vb) * 0.5 + ov * AWAY_MM,
    ))
}

fn next_rect_side_nth(app: &AppState) -> usize {
    let lines = first_sketch_rect_lines(app);
    for (i, &line) in lines.iter().take(2).enumerate() {
        if !line_key_has_length_dim(app, line) {
            return i;
        }
    }
    0
}

fn placing_rect_side_nth(app: &AppState) -> Option<usize> {
    use crate::model::{DimensionTarget, DistanceTarget};
    let lines = first_sketch_rect_lines(app);
    let target = match app.placing_dimension.as_ref() {
        Some(p) => &p.target,
        None => match app.editing_committed_dim.as_ref().map(|e| &e.target) {
            Some(crate::actions::DimEditTarget::New(t)) => t,
            _ => return None,
        },
    };
    let DimensionTarget::Distance(DistanceTarget::LineLength(i)) = target else {
        return None;
    };
    lines.iter().position(|l| l == i)
}

fn next_rect_side_guide(app: &AppState) -> Option<glam::Vec3> {
    let nth = next_rect_side_nth(app);
    rect_side_mid(app, nth).or_else(|| {
        if nth == 0 {
            ground_local(app, 40.0, 20.0)
        } else {
            ground_local(app, 60.0, 40.0)
        }
    })
}

fn next_rect_drop_guide(app: &AppState) -> Option<glam::Vec3> {
    if let Some(nth) = placing_rect_side_nth(app) {
        if let Some(p) = rect_side_drop_spot(app, nth) {
            return Some(p);
        }
    }
    let nth = next_rect_side_nth(app);
    rect_side_drop_spot(app, nth).or_else(|| {
        if nth == 0 {
            ground_local(app, 40.0, 9.0)
        } else {
            ground_local(app, 71.0, 40.0)
        }
    })
}

fn assist_draw_free_square(app: &mut AppState) {
    if has_rectangle_outline(app) {
        return;
    }
    ensure_ground_sketch(app);
    let Some(session) = app.sketch_session else {
        return;
    };
    crate::construction::add_line_rectangle(
        &mut app.doc,
        session.sketch,
        20.0,
        20.0,
        40.0,
        40.0,
        [false; 4],
    );
    app.refresh_document_health();
}

fn dimension_rect_line(app: &mut AppState, nth: usize, expression: &str) {
    use crate::model::{DimensionTarget, DistanceTarget};
    let lines = first_sketch_rect_lines(app);
    let Some(&index) = lines.get(nth) else {
        return;
    };
    let Some(sketch) = app.doc.lines.get(index).map(|l| l.sketch) else {
        return;
    };
    if line_key_has_length_dim(app, index) {
        let target = app.doc.constraints.iter().find_map(|(key, c)| {
            matches!(
                &c.kind,
                ConstraintKind::Distance {
                    target: DistanceTarget::LineLength(i),
                } if *i == index
            )
            .then_some(key)
        });
        if let Some(key) = target {
            let _ = crate::constraints::set_constraint_expression(
                &mut app.doc,
                key,
                expression.to_string(),
            );
            let _ = crate::constraints::solve_document_constraints(&mut app.doc);
            app.refresh_document_health();
        }
        return;
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

fn assist_dimension_first_side(app: &mut AppState) {
    if first_rect_dim_is_10(app) {
        return;
    }
    assist_draw_free_square(app);
    dimension_rect_line(app, 0, "10");
}

fn assist_dimension_both_sides(app: &mut AppState) {
    if rect_dims_are_10(app) {
        return;
    }
    assist_dimension_first_side(app);
    dimension_rect_line(app, 1, "10");
}

fn assist_exit_sketch(app: &mut AppState) {
    if app.sketch_session.is_some() {
        app.apply(Action::ExitSketch);
    }
}

fn assist_zoom_to_fit(app: &mut AppState) {
    assist_exit_sketch(app);
    let _ = app.apply(Action::ZoomToFit);
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
    if has_rectangle_outline(app) {
        assist_dimension_both_sides(app);
        return;
    }
    ensure_ground_sketch(app);
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

/// Seed a few cuboids so the walkthrough starts with geometry to orbit.
/// Drop the default XY/XZ/YZ planes first so they don't hide the cubes (#1306).
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

#[cfg(test)]
fn assist_nav_orbit(app: &mut AppState) {
    // Land clear of every StandardView so the later bear-snap step still has work to do.
    let home = app.cam.home_view();
    app.cam.yaw = home.yaw + 0.55;
    app.cam.pitch = (home.pitch + 0.2).clamp(-1.4, 1.4);
}

#[cfg(test)]
fn assist_nav_pan(app: &mut AppState) {
    app.cam.target += glam::Vec3::new(40.0, 20.0, 0.0);
}

#[cfg(test)]
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

#[cfg(test)]
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
    phone_narration: Option<&'static str>,
) -> Step {
    Step {
        narration,
        anchor: StepAnchor::World(nav_cubes_guide),
        done: Some(done),
        on_enter: None,
        assist: None,
        needs_shift: None,
        drag_hint: Some(drag_hint),
        key_hint: None,
        marks: None,
        type_hint: None,
        phone_narration,
        only_on_phone: false,
    }
}

/// #1269: second walkthrough — pan, orbit, zoom, bear HUD, home.
/// Starts with cubes already in the document. One action per step (#1253).
/// The Selection Exploder step is gone (#1330): its tooltip covered the loupes.
/// Orbit / pan / zoom / home have no "for me" assist (#1550–#1554); Next-only
/// "Good job" steps sit after orbit and pan so the next action is obvious.
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
        Some("Drag with three fingers to orbit around the model."),
    ),
    plain_step("Good job orbiting!", StepAnchor::None, None),
    nav_drag_step(
        "Middle-drag, or Shift + right-drag, to pan.",
        camera_has_panned,
        "Middle button",
        Some("Drag with two fingers to pan."),
    ),
    plain_step("Good job!", StepAnchor::None, None),
    Step {
        narration: "Scroll the mouse wheel to zoom in and out.",
        anchor: StepAnchor::World(nav_cubes_guide),
        done: Some(camera_has_zoomed),
        on_enter: None,
        assist: None,
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
    plain_step(
        "Click the house under the bear to go to the Home view.",
        StepAnchor::Ui(UiAnchor::ViewHome),
        Some(camera_at_home),
    ),
    plain_step(
        "That's the view: orbit, pan, zoom, the bear, and Home. Nice!",
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

/// #1240 / #1315–#1318: draw a free rectangle, set sizes with the Dimension tool,
/// extrude, edit a dimension, then Esc and Zoom to Fit. One action per step (#1253).
static DIMENSIONED_BOX_STEPS: &[Step] = &[
    plain_step(
        "Hi! We'll make a cube, then change it with dimensions.",
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
    assisted_step(
        "Click the opposite corner.",
        StepAnchor::World(rect_opposite_corner_guide),
        Some(has_rectangle_outline),
        StepAssist {
            label: "Draw it for me",
            run: assist_draw_free_square,
        },
        None,
    ),
    plain_step_enter(
        "Now exact sizes. Grab the Dimension tool \u{2014} the glowing button, or press `D`.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Dimension)),
        Some(dimension_tool_active),
        clear_selection_for_dimensioning,
    ),
    plain_step(
        "Click one side of the square.",
        StepAnchor::World(next_rect_side_guide),
        Some(first_rect_side_picked),
    ),
    plain_step(
        "Click to drop the dimension.",
        StepAnchor::World(next_rect_drop_guide),
        Some(first_rect_dim_open),
    ),
    assisted_step(
        "Type `10`, then Enter.",
        StepAnchor::Ui(UiAnchor::DimensionValue),
        Some(first_rect_dim_is_10),
        StepAssist {
            label: "Do it for me",
            run: assist_dimension_first_side,
        },
        Some(TypeHint::Fixed("10")),
    ),
    plain_step(
        "Click another side.",
        StepAnchor::World(next_rect_side_guide),
        Some(second_rect_side_picked),
    ),
    plain_step(
        "Click to drop that dimension.",
        StepAnchor::World(next_rect_drop_guide),
        Some(second_rect_dim_open),
    ),
    assisted_step(
        "Type `10`, then Enter.",
        StepAnchor::Ui(UiAnchor::DimensionValue),
        Some(rect_dims_are_10),
        StepAssist {
            label: "Do it for me",
            run: assist_dimension_both_sides,
        },
        Some(TypeHint::Fixed("10")),
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
        "Type `10` for the height, then Enter. A 10 mm cube.",
        StepAnchor::Ui(UiAnchor::ExtrudeDistance),
        Some(extrusion_is_10),
        StepAssist {
            label: "Extrude 10 for me",
            run: assist_extrude_10mm,
        },
        Some(TypeHint::Fixed("10")),
    ),
    // #1279 / #1313: Elements double-click or right-click → Edit (not the viewport).
    plain_step(
        "Reopen the sketch \u{2014} double-click it in Elements, or right-click and choose Edit sketch.",
        StepAnchor::Ui(UiAnchor::ElementsSketch),
        Some(sketch_reopened_for_edit),
    ),
    // #1314: open the label before typing the new value.
    plain_step(
        "Double-click one of the dimension labels.",
        StepAnchor::World(first_rect_dim_label),
        Some(dim_label_opened_for_edit),
    ),
    assisted_step(
        "Change it from `10` to `20`. The box stretches.",
        StepAnchor::Ui(UiAnchor::DimensionValue),
        Some(one_sketch_dim_is_20),
        StepAssist {
            label: "Change it for me",
            run: assist_edit_dim_to_20,
        },
        Some(TypeHint::Fixed("20")),
    ),
    keyed_assist_step(
        "Press `Esc` to finish the sketch.",
        sketch_exited,
        "Esc",
        "to finish the sketch",
        StepAssist {
            label: "Leave for me",
            run: assist_exit_sketch,
        },
    ),
    keyed_assist_step(
        "Press `Z` to Zoom to Fit.",
        zoomed_to_fit,
        "Z",
        "Zoom to Fit",
        StepAssist {
            label: "Zoom for me",
            run: assist_zoom_to_fit,
        },
    ),
    plain_step(
        "That's the loop: dimensions drive the solid. Change numbers, not geometry. \
         Nice work!",
        StepAnchor::None,
        None,
    ),
];

// --- Parameters tutorial (#1347) -------------------------------------------------------

fn expr_norm(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect::<String>()
}

fn expr_eq(a: &str, b: &str) -> bool {
    expr_norm(a).eq_ignore_ascii_case(&expr_norm(b))
}

fn param_exists(app: &AppState, name: &str) -> bool {
    app.doc
        .parameters
        .values()
        .any(|p| p.name.eq_ignore_ascii_case(name))
}

fn param_key(app: &AppState, name: &str) -> Option<crate::model::ParameterKey> {
    app.doc
        .parameters
        .iter()
        .find_map(|(k, p)| p.name.eq_ignore_ascii_case(name).then_some(k))
}

fn param_length_near(app: &AppState, name: &str, mm: f32) -> bool {
    app.doc.parameters.values().any(|p| {
        p.name.eq_ignore_ascii_case(name)
            && crate::value::eval_length_mm_in_doc(&p.expression, &app.doc)
                .is_some_and(|v| (v - mm).abs() < 0.51)
    })
}

fn dim_expr_eq(app: &AppState, want: &str) -> bool {
    live_constraints(app).any(|c| {
        matches!(c.kind, ConstraintKind::Distance { .. }) && expr_eq(&c.expression, want)
    })
}

fn creating_rect_text_eq(app: &AppState, axis: usize, want: &str) -> bool {
    app.creating_rect.as_ref().is_some_and(|cr| {
        cr.user_edited[axis] && expr_eq(&cr.texts[axis], want)
    })
}

fn name_box_tapped(app: &AppState) -> bool {
    app.parameters_pane.new_name_focused
        || !app.parameters_pane.new_name.trim().is_empty()
        || param_exists(app, "width")
}

fn value_box_tapped(app: &AppState) -> bool {
    app.parameters_pane.new_value_focused
        || !app.parameters_pane.new_value.trim().is_empty()
        || param_exists(app, "width")
}

fn name_says_width(app: &AppState) -> bool {
    app.parameters_pane
        .new_name
        .trim()
        .eq_ignore_ascii_case("width")
        || param_exists(app, "width")
}

fn value_says_20(app: &AppState) -> bool {
    crate::value::eval_length_mm(&app.parameters_pane.new_value)
        .is_some_and(|v| (v - 20.0).abs() < 1e-3)
        || param_exists(app, "width")
}

fn width_added(app: &AppState) -> bool {
    param_exists(app, "width")
}

fn rect_width_is_width(app: &AppState) -> bool {
    creating_rect_text_eq(app, 0, "width") || dim_expr_eq(app, "width")
}

fn rect_height_focused(app: &AppState) -> bool {
    app.creating_rect.as_ref().is_some_and(|cr| cr.focused == 1)
        || parametric_rect_committed(app)
}

fn parametric_rect_committed(app: &AppState) -> bool {
    has_rectangle_outline(app) && dim_expr_eq(app, "width") && dim_expr_eq(app, "width*2")
}

fn editing_param_value(app: &AppState, name: &str) -> bool {
    match app.parameters_pane.editing {
        Some(crate::parameters::ParameterEditCell::Value(i)) => app
            .doc
            .parameters
            .get(i)
            .is_some_and(|p| p.name.eq_ignore_ascii_case(name)),
        _ => false,
    }
}

fn width_value_open(app: &AppState) -> bool {
    editing_param_value(app, "width") || param_length_near(app, "width", 30.0)
}

fn width_is_30(app: &AppState) -> bool {
    param_length_near(app, "width", 30.0)
}

fn extruded_with_height(app: &AppState) -> bool {
    param_exists(app, "height") && has_extrusion(app)
}

fn height_value_open(app: &AppState) -> bool {
    editing_param_value(app, "height") || param_length_near(app, "height", 50.0)
}

fn height_is_50(app: &AppState) -> bool {
    param_length_near(app, "height", 50.0)
}

fn ensure_param(app: &mut AppState, name: &str, expression: &str) {
    if !param_exists(app, name) {
        app.apply(Action::AddParameter {
            name: name.to_string(),
            expression: expression.to_string(),
        });
    }
}

fn set_param(app: &mut AppState, name: &str, expression: &str) {
    if let Some(index) = param_key(app, name) {
        app.apply(Action::CommitParameterExpression {
            index,
            expression: expression.to_string(),
        });
    } else {
        ensure_param(app, name, expression);
    }
}

fn add_width_param(app: &mut AppState) {
    ensure_param(app, "width", "20mm");
}

fn ensure_rect_height_focus(app: &mut AppState) {
    if let Some(cr) = app.creating_rect.as_mut() {
        cr.focused = 1;
        cr.pending_focus = true;
    }
}

fn assist_draw_parametric_rect(app: &mut AppState) {
    if parametric_rect_committed(app) {
        return;
    }
    ensure_param(app, "width", "20mm");
    if app.creating_rect.is_some() {
        if let Some(cr) = app.creating_rect.as_mut() {
            cr.texts[0] = "width".into();
            cr.texts[1] = "width*2".into();
            cr.user_edited = [true, true];
        }
        app.apply(Action::CommitRectangle);
        return;
    }
    if has_rectangle_outline(app) {
        dimension_rect_line(app, 0, "width");
        dimension_rect_line(app, 1, "width*2");
        return;
    }
    ensure_ground_sketch(app);
    let w = crate::value::eval_length_mm_in_doc("width", &app.doc).unwrap_or(20.0);
    app.apply(Action::CreateRectangle {
        x: 20.0,
        y: 20.0,
        width: w,
        height: w * 2.0,
        width_expr: Some("width".into()),
        height_expr: Some("width*2".into()),
    });
}

fn assist_change_width(app: &mut AppState) {
    assist_draw_parametric_rect(app);
    set_param(app, "width", "30mm");
}

fn assist_extrude_with_height(app: &mut AppState) {
    if extruded_with_height(app) {
        return;
    }
    assist_change_width(app);
    ensure_param(app, "height", "30mm");
    if has_extrusion(app) {
        let key = app.doc.extrusions.keys().next();
        let h = crate::value::eval_length_mm_in_doc("height", &app.doc).unwrap_or(30.0);
        if let Some(key) = key {
            app.doc.extrusions[key].distance = h;
            app.doc.extrusions[key].expression = "height".into();
            app.refresh_document_health();
        }
        return;
    }
    let Some(sketch) = app
        .doc
        .lines
        .values()
        .find(|l| !l.construction)
        .map(|l| l.sketch)
    else {
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
    let h = crate::value::eval_length_mm_in_doc("height", &app.doc).unwrap_or(30.0);
    app.apply(Action::CreateExtrusion {
        sketch,
        faces: vec![crate::model::ExtrudeFace::Polygon(lines)],
        distance: h,
        body: crate::actions::ExtrudeBodyChoice::New,
        target: None,
        expression: Some("height".into()),
        symmetric: false,
        taper: 0.0,
        taper_mode: crate::model::ExtrudeTaperMode::Distance,
        taper_expression: None,
    });
}

fn assist_change_height(app: &mut AppState) {
    assist_extrude_with_height(app);
    set_param(app, "height", "50mm");
}

/// #1347: create `width`, sketch a `width` × `width*2` rectangle, change `width`,
/// extrude with inline `height=30mm`, then change `height`. One action per step (#1253).
static PARAMETERS_STEPS: &[Step] = &[
    plain_step(
        "Hi! Let's drive a box with parameters.",
        StepAnchor::None,
        None,
    ),
    plain_step(
        "See the Parameters pane on the right? Tap inside the name box.",
        StepAnchor::Ui(UiAnchor::ParametersName),
        Some(name_box_tapped),
    ),
    assisted_step(
        "Type `width` \u{2014} just those five letters.",
        StepAnchor::Ui(UiAnchor::ParametersName),
        Some(name_says_width),
        StepAssist {
            label: "Add it for me",
            run: add_width_param,
        },
        Some(TypeHint::Fixed("width")),
    ),
    plain_step(
        "Now tap the value box beside it.",
        StepAnchor::Ui(UiAnchor::ParametersValue),
        Some(value_box_tapped),
    ),
    assisted_step(
        "Type `20mm`.",
        StepAnchor::Ui(UiAnchor::ParametersValue),
        Some(value_says_20),
        StepAssist {
            label: "Add it for me",
            run: add_width_param,
        },
        Some(TypeHint::Fixed("20mm")),
    ),
    plain_step(
        "Press + to add it. Your first parameter!",
        StepAnchor::Ui(UiAnchor::ParametersAdd),
        Some(width_added),
    ),
    plain_step(
        "Rectangle tool \u{2014} glowing button, or `R`.",
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
        "Type `width` in the width field, then Tab.",
        StepAnchor::Ui(UiAnchor::RectWidth),
        Some(rect_width_is_width),
        StepAssist {
            label: "Draw it for me",
            run: assist_draw_parametric_rect,
        },
        Some(TypeHint::Fixed("width")),
    ),
    assisted_step(
        "Press `Tab`, or click the height field.",
        StepAnchor::Ui(UiAnchor::RectHeight),
        Some(rect_height_focused),
        StepAssist {
            label: "Draw it for me",
            run: assist_draw_parametric_rect,
        },
        None,
    ),
    assisted_step_enter(
        "Type `width*2`, then Enter.",
        StepAnchor::Ui(UiAnchor::RectHeight),
        Some(parametric_rect_committed),
        StepAssist {
            label: "Draw it for me",
            run: assist_draw_parametric_rect,
        },
        Some(TypeHint::Fixed("width*2")),
        ensure_rect_height_focus,
    ),
    plain_step(
        "Click the `width` value in the Parameters pane.",
        StepAnchor::Ui(UiAnchor::ParametersExistingValue),
        Some(width_value_open),
    ),
    assisted_step(
        "Change it to `30mm`. The rectangle stretches.",
        StepAnchor::Ui(UiAnchor::ParametersExistingValue),
        Some(width_is_30),
        StepAssist {
            label: "Change it for me",
            run: assist_change_width,
        },
        Some(TypeHint::Fixed("30mm")),
    ),
    plain_step(
        "Extrude tool \u{2014} glowing button, or `E`.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Extrude)),
        Some(extrude_tool_active),
    ),
    plain_step(
        "Click the rectangle.",
        StepAnchor::World(rectangle_face_guide),
        Some(extrude_face_picked),
    ),
    assisted_step(
        "Type `height=30mm` \u{2014} that creates a parameter right in the field \u{2014} then Enter.",
        StepAnchor::Ui(UiAnchor::ExtrudeDistance),
        Some(extruded_with_height),
        StepAssist {
            label: "Extrude it for me",
            run: assist_extrude_with_height,
        },
        Some(TypeHint::Fixed("height=30mm")),
    ),
    plain_step(
        "Click the `height` value in the Parameters pane.",
        StepAnchor::Ui(UiAnchor::ParametersExistingValue),
        Some(height_value_open),
    ),
    assisted_step(
        "Change it to `50mm`. The solid grows.",
        StepAnchor::Ui(UiAnchor::ParametersExistingValue),
        Some(height_is_50),
        StepAssist {
            label: "Change it for me",
            run: assist_change_height,
        },
        Some(TypeHint::Fixed("50mm")),
    ),
    plain_step(
        "That's the loop: parameters drive the model. Change a number, the solid follows. \
         Nice!",
        StepAnchor::None,
        None,
    ),
];

fn chamfer_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Chamfer
}

fn combine_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Combine
}

fn text_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Text
}

fn sketch_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Sketch
}

fn has_sketch_chamfer(app: &AppState) -> bool {
    !app.doc.sketch_vertex_treatment_ops.is_empty()
}

fn sketch_chamfer_picked(app: &AppState) -> bool {
    app.creating_vertex_treatment
        .as_ref()
        .is_some_and(|c| c.kind == crate::model::VertexTreatmentKind::Chamfer)
        || has_sketch_chamfer(app)
}

fn has_solid_chamfer(app: &AppState) -> bool {
    app.doc
        .edge_treatment_ops
        .values()
        .any(|op| op.kind == crate::model::VertexTreatmentKind::Chamfer)
}

fn solid_chamfer_picked(app: &AppState) -> bool {
    app.creating_edge_treatment
        .as_ref()
        .is_some_and(|c| {
            c.kind == crate::model::VertexTreatmentKind::Chamfer && !c.edges.is_empty()
        })
        || has_solid_chamfer(app)
}

fn combine_cut_mode_ready(app: &AppState) -> bool {
    has_combine_cut(app)
        || app
            .creating_boolean
            .as_ref()
            .is_some_and(|cb| cb.kind == crate::model::BooleanOpKind::Cut)
}

fn combine_a_picked(app: &AppState) -> bool {
    has_combine_cut(app)
        || app
            .creating_boolean
            .as_ref()
            .is_some_and(|cb| !cb.a.is_empty())
}

fn combine_b_picked(app: &AppState) -> bool {
    has_combine_cut(app)
        || app
            .creating_boolean
            .as_ref()
            .is_some_and(|cb| !cb.b.is_empty())
}

fn has_combine_cut(app: &AppState) -> bool {
    app.doc
        .boolean_ops
        .values()
        .any(|op| op.kind == crate::model::BooleanOpKind::Cut && !op.outputs.is_empty())
}

fn has_sketch_text(app: &AppState) -> bool {
    !app.doc.sketch_texts.is_empty()
}

fn text_says_bear(app: &AppState) -> bool {
    app.doc
        .sketch_texts
        .values()
        .any(|t| t.text.to_ascii_uppercase().contains("BEAR"))
}

fn has_raised_text(app: &AppState) -> bool {
    app.doc.extrusions.values().any(|e| {
        e.faces
            .iter()
            .any(|f| matches!(f, crate::model::ExtrudeFace::TextGlyph { .. }))
    })
}

fn text_extrude_picked(app: &AppState) -> bool {
    has_raised_text(app)
        || app.creating_extrusion.as_ref().is_some_and(|ce| {
            ce.faces
                .iter()
                .any(|f| matches!(f, crate::model::ExtrudeFace::TextGlyph { .. }))
        })
}

fn sketch_on_cuboid(app: &AppState) -> bool {
    has_sketch_text(app)
        || app.sketch_session.is_some_and(|s| {
            matches!(
                app.doc.sketches.get(s.sketch).map(|sk| &sk.face),
                Some(crate::model::FaceId::PrimitiveFace { .. })
            )
        })
}

fn first_usable_font() -> Option<String> {
    for fam in ["Helvetica", "Arial", "Segoe UI", "DejaVu Sans", "Liberation Sans"] {
        if crate::text::font_bytes(fam, false, false).is_some() {
            return Some(fam.to_string());
        }
    }
    crate::text::system_font_families().into_iter().next()
}

fn rect_chamfer_corner_guide(app: &AppState) -> Option<glam::Vec3> {
    let lines = first_sketch_rect_lines(app);
    let line = app.doc.lines.get(*lines.get(1)?)?;
    let frame = crate::face::sketch_geometry_frame(&app.doc, line.sketch)?;
    Some(crate::face::local_to_world(&frame, line.x1, line.y1))
}

fn extrusion_top_guide(app: &AppState) -> Option<glam::Vec3> {
    let mut sum = glam::Vec3::ZERO;
    let mut n = 0u32;
    for (_, edge, a, b) in crate::extrude::treatable_edges(&app.doc) {
        if matches!(
            edge,
            crate::model::ExtrusionEdgeRef::Cap { top: true, .. }
        ) {
            sum += (a + b) * 0.5;
            n += 1;
        }
    }
    if n > 0 {
        return Some(sum / n as f32);
    }
    rectangle_face_guide(app).map(|p| p + glam::Vec3::Z * 10.0)
}

fn cuboid_body_guide(app: &AppState) -> Option<glam::Vec3> {
    let p = app
        .doc
        .primitives
        .values()
        .find(|p| p.kind == crate::model::PrimitiveKind::Cuboid)?;
    let r = crate::primitives::resolve(&app.doc, p)?;
    Some(r.origin + r.normal * (r.height * 0.5))
}

/// A bottom corner of the placed cuboid — where the Combine tutorial's overlap-sphere
/// click should land, rather than tracking the in-progress sphere ghost (#1566).
fn cuboid_bottom_corner_guide(app: &AppState) -> Option<glam::Vec3> {
    let p = app
        .doc
        .primitives
        .values()
        .find(|p| p.kind == crate::model::PrimitiveKind::Cuboid)?;
    let r = crate::primitives::resolve(&app.doc, p)?;
    Some(r.cuboid_base()[2])
}

fn sphere_body_guide(app: &AppState) -> Option<glam::Vec3> {
    let p = app
        .doc
        .primitives
        .values()
        .find(|p| p.kind == crate::model::PrimitiveKind::Sphere)?;
    crate::primitives::resolve(&app.doc, p).map(|r| r.sphere_center())
}

fn cuboid_top_guide(app: &AppState) -> Option<glam::Vec3> {
    let p = app
        .doc
        .primitives
        .values()
        .find(|p| p.kind == crate::model::PrimitiveKind::Cuboid)?;
    let r = crate::primitives::resolve(&app.doc, p)?;
    Some(r.origin + r.normal * r.height)
}

fn text_or_cuboid_top_guide(app: &AppState) -> Option<glam::Vec3> {
    if let Some(t) = app.doc.sketch_texts.values().next() {
        if let Some(frame) = crate::face::sketch_geometry_frame(&app.doc, t.sketch) {
            return Some(crate::face::local_to_world(&frame, t.origin.0 + 8.0, t.origin.1));
        }
    }
    cuboid_top_guide(app)
}

fn first_rect_corner_point(app: &AppState) -> Option<crate::model::ConstraintPoint> {
    let lines = first_sketch_rect_lines(app);
    let &line = lines.get(1)?;
    Some(crate::model::ConstraintPoint::LineEndpoint {
        line,
        end: crate::model::LineEnd::End,
    })
}

fn live_body_for_primitive(
    app: &AppState,
    kind: crate::model::PrimitiveKind,
) -> Option<crate::model::BodyKey> {
    let prim = app
        .doc
        .primitives
        .iter()
        .find(|(_, p)| p.kind == kind)
        .map(|(k, _)| k)?;
    app.doc.bodies.iter().find_map(|(bi, b)| {
        (!b.shadow && matches!(b.source, crate::model::BodySource::Primitive(p) if p == prim))
            .then_some(bi)
    })
}

fn assist_chamfer_rect_corner(app: &mut AppState) {
    if has_sketch_chamfer(app) {
        return;
    }
    assist_draw_square(app);
    let Some(point) = first_rect_corner_point(app) else {
        return;
    };
    app.apply(Action::CommitVertexTreatment {
        point,
        kind: crate::model::VertexTreatmentKind::Chamfer,
        amount: "5".into(),
    });
}

fn assist_extrude_chamfered_profile(app: &mut AppState) {
    if has_extrusion(app) {
        return;
    }
    assist_chamfer_rect_corner(app);
    let Some(sketch) = app
        .doc
        .lines
        .values()
        .find(|l| !l.construction)
        .map(|l| l.sketch)
    else {
        return;
    };
    let Some(lines) = crate::polygon::closed_line_loops(&app.doc, sketch)
        .into_iter()
        .max_by_key(|loop_| loop_.len())
    else {
        return;
    };
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

fn assist_chamfer_top_edges(app: &mut AppState) {
    if has_solid_chamfer(app) {
        return;
    }
    assist_extrude_chamfered_profile(app);
    let Some(extrusion) = app.doc.extrusions.keys().next() else {
        return;
    };
    // Opposite cap edges in one op — adjacent edges on the same face share a corner.
    let edges = vec![
        (
            crate::model::TreatableSolid::Extrusion(extrusion),
            crate::model::ExtrusionEdgeRef::Cap {
                face: 0,
                edge: 0,
                top: true,
            },
        ),
        (
            crate::model::TreatableSolid::Extrusion(extrusion),
            crate::model::ExtrusionEdgeRef::Cap {
                face: 0,
                edge: 2,
                top: true,
            },
        ),
        (
            crate::model::TreatableSolid::Extrusion(extrusion),
            crate::model::ExtrusionEdgeRef::Cap {
                face: 0,
                edge: 1,
                top: true,
            },
        ),
        (
            crate::model::TreatableSolid::Extrusion(extrusion),
            crate::model::ExtrusionEdgeRef::Cap {
                face: 0,
                edge: 3,
                top: true,
            },
        ),
    ];
    // Face-click fills every edge of the top; commit keeps the ones that don't share
    // a corner. Then stack the remaining pair so all four edges of that side land.
    let _ = app.apply(Action::CommitEdgeTreatments {
        edges: vec![edges[0], edges[1]],
        kind: crate::model::VertexTreatmentKind::Chamfer,
        amount: 3.0,
        expression: "3".into(),
    });
    let _ = app.apply(Action::CommitEdgeTreatments {
        edges: vec![edges[2], edges[3]],
        kind: crate::model::VertexTreatmentKind::Chamfer,
        amount: 3.0,
        expression: "3".into(),
    });
}

fn overlap_sphere_origin() -> [f32; 3] {
    [8.0, 8.0, 0.0]
}

fn assist_place_overlap_sphere(app: &mut AppState) {
    if has_sphere(app) {
        return;
    }
    assist_place_cuboid(app);
    let mut shape = crate::model::Primitive::new(crate::model::PrimitiveKind::Sphere);
    shape.origin = cuboid_bottom_corner_guide(app)
        .map(|p| p.to_array())
        .unwrap_or_else(overlap_sphere_origin);
    shape.radius = "12".into();
    app.apply(Action::CreateShape { shape });
}

fn assist_combine_cut_sphere(app: &mut AppState) {
    if has_combine_cut(app) {
        return;
    }
    assist_place_overlap_sphere(app);
    let Some(cube) = live_body_for_primitive(app, crate::model::PrimitiveKind::Cuboid) else {
        return;
    };
    let Some(sphere) = live_body_for_primitive(app, crate::model::PrimitiveKind::Sphere) else {
        return;
    };
    app.apply(Action::CreateBooleanOperation {
        kind: crate::model::BooleanOpKind::Cut,
        a: vec![cube],
        b: vec![sphere],
        keep_b: false,
        solid_count: None,
    });
}

fn assist_place_bear_text(app: &mut AppState) {
    if text_says_bear(app) {
        return;
    }
    assist_place_cuboid(app);
    let Some(prim) = app
        .doc
        .primitives
        .iter()
        .find(|(_, p)| p.kind == crate::model::PrimitiveKind::Cuboid)
        .map(|(k, _)| k)
    else {
        return;
    };
    if app.sketch_session.is_none() {
        app.apply(Action::BeginSketch {
            face: crate::model::FaceId::PrimitiveFace {
                primitive: prim,
                face: crate::model::PrimitiveFace::CuboidTop,
            },
            viewport: None,
        });
    }
    let existing_text = app.doc.sketch_texts.keys().next();
    if let Some(key) = existing_text {
        let existing = app.doc.sketch_texts[key].clone();
        app.apply(Action::EditSketchText {
            index: key,
            text: "BEAR".into(),
            font_family: existing.font_family,
            bold: existing.bold,
            italic: existing.italic,
            underline: existing.underline,
            size: existing.size,
            size_expr: existing.size_expr,
            rotation: existing.rotation,
            wrap_width: existing.wrap_width,
        });
        return;
    }
    let Some(session) = app.sketch_session else {
        return;
    };
    let Some(family) = first_usable_font() else {
        return;
    };
    app.apply(Action::CreateSketchText {
        sketch: session.sketch,
        text: "BEAR".into(),
        font_family: family,
        bold: false,
        italic: false,
        underline: false,
        size: 8.0,
        size_expr: "8".into(),
        origin: (-8.0, 3.0),
        rotation: 0.0,
        wrap_width: None,
    });
}

fn assist_extrude_raised_text(app: &mut AppState) {
    if has_raised_text(app) {
        return;
    }
    assist_place_bear_text(app);
    let Some((key, sketch, glyphs)) = app.doc.sketch_texts.iter().next().map(|(k, t)| {
        (k, t.sketch, crate::text::group_glyphs(&t.contours).len())
    }) else {
        return;
    };
    if glyphs == 0 {
        return;
    }
    let faces: Vec<_> = (0..glyphs)
        .map(|glyph| crate::model::ExtrudeFace::TextGlyph { text: key, glyph })
        .collect();
    if app.sketch_session.is_some() {
        app.apply(Action::ExitSketch);
    }
    app.apply(Action::CreateExtrusion {
        sketch,
        faces,
        distance: 2.0,
        body: crate::actions::ExtrudeBodyChoice::Merge,
        target: None,
        expression: Some("2".into()),
        symmetric: false,
        taper: 0.0,
        taper_mode: crate::model::ExtrudeTaperMode::Distance,
        taper_expression: None,
    });
}

fn sphere_kind_or_placed(app: &AppState) -> bool {
    has_sphere(app)
        || (shape_tool_active(app) && app.shape_kind == crate::model::PrimitiveKind::Sphere)
}

/// #1555: chamfer a rectangle corner, extrude, then chamfer the top of the solid.
static CHAMFER_STEPS: &[Step] = &[
    plain_step(
        "Let's chamfer!",
        StepAnchor::None,
        None,
    ),
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
        "Click the Chamfer tool \u{2014} glowing button, or `K`.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Chamfer)),
        Some(chamfer_tool_active),
    ),
    plain_step(
        "Click a corner of the square.",
        StepAnchor::World(rect_chamfer_corner_guide),
        Some(sketch_chamfer_picked),
    ),
    assisted_step(
        "Type `5`, then Enter.",
        StepAnchor::None,
        Some(has_sketch_chamfer),
        StepAssist {
            label: "Chamfer it for me",
            run: assist_chamfer_rect_corner,
        },
        Some(TypeHint::Fixed("5")),
    ),
    plain_step(
        "Click the Extrude tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Extrude)),
        Some(extrude_tool_active),
    ),
    plain_step(
        "Click the profile.",
        StepAnchor::World(rectangle_face_guide),
        Some(extrude_face_picked),
    ),
    assisted_step(
        "Press Enter. A solid with a cut corner.",
        StepAnchor::None,
        Some(has_extrusion),
        StepAssist {
            label: "Extrude it for me",
            run: assist_extrude_chamfered_profile,
        },
        None,
    ),
    plain_step(
        "Chamfer tool again \u{2014} glowing button, or `K`.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Chamfer)),
        Some(chamfer_tool_active),
    ),
    plain_step(
        "Click the top of the box \u{2014} Chamfer picks every edge of that side.",
        StepAnchor::World(extrusion_top_guide),
        Some(solid_chamfer_picked),
    ),
    assisted_step(
        "Type `3`, then Enter.",
        StepAnchor::None,
        Some(has_solid_chamfer),
        StepAssist {
            label: "Chamfer it for me",
            run: assist_chamfer_top_edges,
        },
        Some(TypeHint::Fixed("3")),
    ),
    plain_step(
        "A chamfered corner in the sketch, and a beveled top. Nice!",
        StepAnchor::None,
        None,
    ),
];

/// #1556: place a cube and a sphere, then cut the sphere out of the cube.
static COMBINE_STEPS: &[Step] = &[
    plain_step(
        "Let's learn how to cut shapes with the Combine tool.",
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
        Some(shape_tool_active),
    ),
    plain_step(
        "Click Sphere in the Context pane (or press `B`).",
        StepAnchor::Ui(UiAnchor::ShapeKind(crate::model::PrimitiveKind::Sphere)),
        Some(sphere_kind_or_placed),
    ),
    plain_step(
        "Click so the sphere overlaps the cube.",
        StepAnchor::World(cuboid_bottom_corner_guide),
        Some(sphere_anchored),
    ),
    assisted_step(
        "Type the radius: `12`, then Enter.",
        StepAnchor::Ui(UiAnchor::ShapeRadius),
        Some(has_sphere),
        StepAssist {
            label: "Place it for me",
            run: assist_place_overlap_sphere,
        },
        Some(TypeHint::Fixed("12")),
    ),
    plain_step(
        "Click the Combine tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Combine)),
        Some(combine_tool_active),
    ),
    plain_step(
        "Click Cut in the Context pane (or press `Y`).",
        StepAnchor::Ui(UiAnchor::CombineKind(crate::model::BooleanOpKind::Cut)),
        Some(combine_cut_mode_ready),
    ),
    plain_step(
        "Click the cube \u{2014} that's side A, the body we keep.",
        StepAnchor::World(cuboid_body_guide),
        Some(combine_a_picked),
    ),
    plain_step(
        "Click the sphere \u{2014} side B, the one we cut away.",
        StepAnchor::World(sphere_body_guide),
        Some(combine_b_picked),
    ),
    assisted_step(
        "Press Enter. The sphere bites the cube.",
        StepAnchor::None,
        Some(has_combine_cut),
        StepAssist {
            label: "Cut it for me",
            run: assist_combine_cut_sphere,
        },
        None,
    ),
    plain_step(
        "That's a cut: Combine's A minus B. Nice!",
        StepAnchor::None,
        None,
    ),
];

/// #1557: sketch letters on a cube and extrude them so they stand proud.
static RAISED_TEXT_STEPS: &[Step] = &[
    plain_step(
        "Let's stamp raised letters on a cube.",
        StepAnchor::None,
        None,
    ),
    plain_step(
        "Grab the Shape tool \u{2014} the glowing button, or press `B`.",
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
        "Click the Sketch tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Sketch)),
        Some(sketch_tool_active),
    ),
    plain_step(
        "Click the top of the cube.",
        StepAnchor::World(cuboid_top_guide),
        Some(sketch_on_cuboid),
    ),
    plain_step(
        "Click the Text tool \u{2014} glowing button, or `T`.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Text)),
        Some(text_tool_active),
    ),
    plain_step(
        "Click on the face to drop the text.",
        StepAnchor::World(cuboid_top_guide),
        Some(has_sketch_text),
    ),
    assisted_step(
        "Type `BEAR` in the Context pane.",
        StepAnchor::Ui(UiAnchor::TextContent),
        Some(text_says_bear),
        StepAssist {
            label: "Type it for me",
            run: assist_place_bear_text,
        },
        Some(TypeHint::Fixed("BEAR")),
    ),
    plain_step(
        "Click the Extrude tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Extrude)),
        Some(extrude_tool_active),
    ),
    plain_step(
        "Click the letters.",
        StepAnchor::World(text_or_cuboid_top_guide),
        Some(text_extrude_picked),
    ),
    assisted_step(
        "Type `2`, then Enter. The letters stand proud.",
        StepAnchor::Ui(UiAnchor::ExtrudeDistance),
        Some(has_raised_text),
        StepAssist {
            label: "Extrude it for me",
            run: assist_extrude_raised_text,
        },
        Some(TypeHint::Fixed("2")),
    ),
    plain_step(
        "That's raised text \u{2014} sketch on a face, type, extrude. Nice!",
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
    use super::*;
    use crate::actions::{Action, Pane};

    /// Back reviews earlier steps without auto-advance re-firing on their already-
    /// satisfied predicates; Next resumes auto mode once it reaches unfinished work.
    #[test]
    fn back_reviews_without_auto_advance_snapping_forward() {
        let mut app = AppState::default();
        app.apply(Action::StartTutorial {
            index: tutorial_index("cube").unwrap(),
        });
        app.apply(Action::TutorialNext); // past the welcome step
        app.apply(Action::SetTool(Tool::Rectangle));
        assert_eq!(app.tutorial.unwrap().step, 2, "rectangle tool -> first-corner step");

        app.apply(Action::TutorialBack);
        let run = app.tutorial.unwrap();
        assert_eq!(run.step, 1);
        assert!(run.hold);
        // Its predicate is satisfied, but reviewing holds auto-advance off.
        app.advance_tutorial();
        assert_eq!(app.tutorial.unwrap().step, 1);

        // Next walks forward; reaching unfinished work resumes auto.
        app.apply(Action::TutorialNext);
        let run = app.tutorial.unwrap();
        assert_eq!(run.step, 2);
        assert!(!run.hold, "caught up to live work — auto-advance resumes");
    }

    /// With no phone-only steps left, desktop and phone share the same numbering.
    #[test]
    fn phone_and_desktop_share_step_counts_when_no_phone_only_steps() {
        let mut app = AppState::default();
        let cube = tutorial_index("cube").unwrap();
        let last = TUTORIALS[cube].steps.len() - 1;
        let (_, desktop_total) = step_position(&app, cube, last);
        app.compact_layout = true;
        let (_, phone_total) = step_position(&app, cube, last);
        assert_eq!(desktop_total, TUTORIALS[cube].steps.len());
        assert_eq!(phone_total, desktop_total);
        assert!(
            TUTORIALS.iter().all(|t| t.steps.iter().all(|s| !s.only_on_phone)),
            "no remaining walkthrough has phone-only steps"
        );
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
                if let Some(ph) = step.phone_narration {
                    assert!(
                        !ph.contains('\u{2192}'),
                        "{}: phone narration uses → (missing from UI font): {ph}",
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

        for tut in TUTORIALS {
            for step in tut.steps {
                assert!(
                    step.narration.matches('`').count() % 2 == 0,
                    "unbalanced backticks in {}: {}",
                    tut.name,
                    step.narration
                );
                let rebuilt: String =
                    narration_spans(step.narration).iter().map(|(t, _)| *t).collect();
                assert_eq!(rebuilt, step.narration.replace('`', ""));
            }
        }
    }

    /// #765: the web app's `?tutorial=` parameter names a registered tutorial.
    #[test]
    fn tutorial_from_query_picks_a_registered_tutorial() {
        let cube = tutorial_index("cube").unwrap();
        assert_eq!(tutorial_from_query("?tutorial=cube"), Some(cube));
        assert_eq!(tutorial_from_query("tutorial=cube"), Some(cube));
        assert_eq!(tutorial_from_query("?foo=1&tutorial=cube&bar=2"), Some(cube));
        assert_eq!(tutorial_from_query("?tutorial=navigate"), tutorial_index("navigate"));
        assert_eq!(tutorial_from_query("?tutorial=nope"), None);
        assert_eq!(tutorial_from_query("?other=cube"), None);
        assert_eq!(tutorial_from_query("?tutorial=bracket"), None);
        assert_eq!(tutorial_from_query(""), None);
    }

    #[test]
    fn tutorial_registry_lookup_by_name() {
        assert_eq!(tutorial_index("cube"), Some(0));
        assert_eq!(tutorial_index("navigate"), Some(1), "#1269: navigate is second");
        assert_eq!(tutorial_index("shapes"), Some(2));
        assert_eq!(tutorial_index("dimensioned_box"), Some(3));
        assert_eq!(tutorial_index("parameters"), Some(4), "#1347: parameters is fifth");
        assert_eq!(tutorial_index("chamfer"), Some(5), "#1555: chamfer is sixth");
        assert_eq!(tutorial_index("combine"), Some(6), "#1556: combine is seventh");
        assert_eq!(tutorial_index("raised_text"), Some(7), "#1557: raised text is eighth");
        assert_eq!(tutorial_index("bracket"), None, "#1334: build-a-bracket tutorial is gone");
        assert_eq!(tutorial_index("nope"), None);
        assert_eq!(TUTORIALS.last().unwrap().name, "raised_text");
        assert_eq!(TUTORIALS.len(), 8, "pane lists every remaining walkthrough");
        for tut in TUTORIALS {
            assert_ne!(tut.name, "bracket");
            assert_ne!(tut.title, "Build an angle bracket");
        }
    }

    /// #1253: tutorials never combine a click with typing in one step's narration.
    #[test]
    fn short_tutorial_steps_keep_clicks_and_typing_apart() {
        for tut in TUTORIALS {
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

    fn dimensioned_box() -> &'static Tutorial {
        &TUTORIALS[tutorial_index("dimensioned_box").unwrap()]
    }

    fn box_step(needle: &str) -> &'static Step {
        dimensioned_box()
            .steps
            .iter()
            .find(|s| s.narration.to_ascii_lowercase().contains(needle))
            .unwrap_or_else(|| panic!("dimensioned_box step matching {needle:?}"))
    }

    fn box_step_index(needle: &str) -> usize {
        dimensioned_box()
            .steps
            .iter()
            .position(|s| s.narration.to_ascii_lowercase().contains(needle))
            .unwrap_or_else(|| panic!("dimensioned_box step matching {needle:?}"))
    }

    /// #1312: the extrude amount is a height, not a depth.
    #[test]
    fn dimensioned_box_extrude_calls_it_height() {
        let step = dimensioned_box()
            .steps
            .iter()
            .find(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::ExtrudeDistance)))
            .expect("extrude distance step");
        let n = step.narration.to_ascii_lowercase();
        assert!(
            n.contains("height"),
            "extrude typing should say height: {}",
            step.narration
        );
        assert!(
            !n.contains("depth"),
            "extrude typing should not say depth: {}",
            step.narration
        );
    }

    /// #1314: double-click a dimension label before changing 10 → 20.
    #[test]
    fn dimensioned_box_double_click_dim_before_changing() {
        let click_i = box_step_index("double-click one of the dimension");
        let change_i = dimensioned_box()
            .steps
            .iter()
            .position(|s| {
                let n = s.narration.to_ascii_lowercase();
                (n.contains("10") && n.contains("20")) || n.contains("to `20`") || n.contains("to 20")
            })
            .expect("change 10 to 20 step");
        assert!(
            click_i < change_i,
            "double-click the label before typing 20 (got {click_i}, {change_i})"
        );
        let click = &dimensioned_box().steps[click_i];
        assert!(
            matches!(click.anchor, StepAnchor::World(_)),
            "dim-label click should point at a label: {}",
            click.narration
        );
        let change = &dimensioned_box().steps[change_i];
        assert!(
            matches!(change.anchor, StepAnchor::Ui(UiAnchor::DimensionValue)),
            "typing 20 should point at the value field: {}",
            change.narration
        );
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

    /// #1279 / #1313: reopen via Elements double-click or right-click → Edit, not the viewport.
    #[test]
    fn dimensioned_box_reopen_sketch_targets_elements_row() {
        let reopen = box_step("reopen the sketch");
        let n = reopen.narration.to_ascii_lowercase();
        assert!(
            matches!(reopen.anchor, StepAnchor::Ui(UiAnchor::ElementsSketch)),
            "reopen step should orb the Elements sketch row: {}",
            reopen.narration
        );
        assert!(
            n.contains("double-click") && n.contains("edit"),
            "should say double-click or right-click Edit: {}",
            reopen.narration
        );
        assert!(
            n.contains("right-click") || n.contains("right click"),
            "should mention right-click → Edit: {}",
            reopen.narration
        );
        assert!(
            !n.contains("viewport"),
            "cannot edit the sketch from the viewport: {}",
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

    /// #1314: dim-label step points at a label and advances once the field is open.
    #[test]
    fn dimensioned_box_dim_label_step_tracks_edit() {
        let mut app = AppState::default();
        assist_extrude_10mm(&mut app);
        let sketch = app.doc.sketches.keys().next().expect("sketch");
        app.apply(Action::OpenSketch {
            sketch,
            viewport: None,
        });
        assert!(first_rect_dim_label(&app).is_some());
        assert!(!dim_label_opened_for_edit(&app));
        let target = app
            .doc
            .constraints
            .iter()
            .find_map(|(key, c)| {
                matches!(c.kind, ConstraintKind::Distance { .. }).then_some(key)
            })
            .expect("length dim");
        app.apply(Action::BeginEditCommittedDim { target });
        assert!(dim_label_opened_for_edit(&app));
        assert!(app.editing_committed_dim.is_some());
    }

    fn first_length_dim_line_mid(app: &AppState) -> Option<glam::Vec3> {
        use crate::model::DistanceTarget;
        let sketch = app
            .sketch_session
            .map(|s| s.sketch)
            .or_else(|| app.doc.sketches.keys().next())?;
        let frame = crate::face::sketch_geometry_frame(&app.doc, sketch)?;
        let line = live_constraints(app).find_map(|c| match &c.kind {
            ConstraintKind::Distance {
                target: DistanceTarget::LineLength(i),
            } => app.doc.lines.get(*i).filter(|l| l.sketch == sketch),
            _ => None,
        })?;
        Some(crate::face::local_to_world(
            &frame,
            (line.x0 + line.x1) * 0.5,
            (line.y0 + line.y1) * 0.5,
        ))
    }

    /// #1332/#1333: the orb sits on the pixel-offset dim label. A fixed 11 mm
    /// offset flies off-screen after sketch-entry zoom and covers the label
    /// when the user zooms out.
    #[test]
    fn dim_label_orb_sits_on_the_label_not_eleven_mm_out() {
        let mut app = AppState::default();
        assist_extrude_10mm(&mut app);
        let sketch = app.doc.sketches.keys().next().expect("sketch");
        app.apply(Action::OpenSketch {
            sketch,
            viewport: None,
        });
        app.viewport_height = 600.0;
        app.viewport_aspect = 1.5;
        // Close enough that a 10 mm square fills most of a 600 px viewport.
        app.cam.distance = 25.0;

        let mid = first_length_dim_line_mid(&app).expect("line mid");
        let label = first_rect_dim_label(&app).expect("label");
        let away = (label - mid).length();
        assert!(
            away < 4.0,
            "zoomed-in dim label should sit a few mm off the line, not {away:.1} mm (#1332)"
        );
        assert!(
            away > 0.05,
            "label should still sit outside the line, not on it"
        );
    }

    /// #1332/#1333: label offset is a screen-pixel distance, so world mm
    /// grows as the camera zooms out — matching the drawn dimension.
    #[test]
    fn dim_label_orb_offset_tracks_camera_zoom() {
        let mut app = AppState::default();
        assist_extrude_10mm(&mut app);
        let sketch = app.doc.sketches.keys().next().expect("sketch");
        app.apply(Action::OpenSketch {
            sketch,
            viewport: None,
        });
        app.viewport_height = 600.0;
        app.viewport_aspect = 1.5;
        let mid = first_length_dim_line_mid(&app).expect("line mid");

        app.cam.distance = 25.0;
        let near = (first_rect_dim_label(&app).expect("near") - mid).length();
        app.cam.distance = 250.0;
        let far = (first_rect_dim_label(&app).expect("far") - mid).length();
        assert!(
            far > near * 2.0,
            "world offset must grow with camera distance (near={near:.2} far={far:.2})"
        );
    }

    /// #1318: "Draw it for me" places an unconstrained rectangle.
    #[test]
    fn dimensions_tutorial_free_rect_has_no_dims() {
        let mut app = AppState::default();
        assist_draw_free_square(&mut app);
        assert!(has_rectangle_outline(&app));
        assert!(
            !has_closed_rectangle(&app),
            "free rectangle must not lock width/height yet"
        );
        assist_dimension_both_sides(&mut app);
        assert!(rect_dims_are_10(&app));
    }

    /// #1315: Escape closes the sketch; Z sets the Zoomed-to-fit status.
    #[test]
    fn dimensions_tutorial_esc_and_zoom_predicates() {
        let mut app = AppState::default();
        assist_edit_dim_to_20(&mut app);
        let sketch = app.doc.sketches.keys().next().expect("sketch");
        app.apply(Action::OpenSketch {
            sketch,
            viewport: None,
        });
        assert!(!sketch_exited(&app));
        assert!(!zoomed_to_fit(&app));
        app.apply(Action::ExitSketch);
        assert!(sketch_exited(&app));
        assert!(!zoomed_to_fit(&app));
        app.apply(Action::ZoomToFit);
        assert!(zoomed_to_fit(&app));
    }

    /// #1317: picker titles are short skill names.
    #[test]
    fn short_tutorial_titles_are_skill_names() {
        assert_eq!(
            TUTORIALS[tutorial_index("cube").unwrap()].title,
            "Sketch & Extrude"
        );
        assert_eq!(
            TUTORIALS[tutorial_index("shapes").unwrap()].title,
            "3D Bodies"
        );
        assert_eq!(dimensioned_box().title, "Dimensions");
        assert_eq!(
            TUTORIALS[tutorial_index("parameters").unwrap()].title,
            "Parameters"
        );
        assert_eq!(
            TUTORIALS[tutorial_index("chamfer").unwrap()].title,
            "Chamfer"
        );
        assert_eq!(
            TUTORIALS[tutorial_index("combine").unwrap()].title,
            "Combine"
        );
        assert_eq!(
            TUTORIALS[tutorial_index("raised_text").unwrap()].title,
            "Raised text"
        );
    }

    /// #1316: the Dimensions tutorial never mentions parameters.
    #[test]
    fn dimensions_tutorial_does_not_mention_parameters() {
        for step in dimensioned_box().steps {
            let n = step.narration.to_ascii_lowercase();
            assert!(
                !n.contains("parameter") && !n.contains("parametric"),
                "Dimensions tutorial should not mention parameters: {}",
                step.narration
            );
        }
    }

    /// #1318: draw a free rectangle, then set sizes with the Dimension tool.
    #[test]
    fn dimensions_tutorial_draws_free_then_uses_dimension_tool() {
        let steps = dimensioned_box().steps;
        assert!(
            steps.iter().any(|s| {
                let n = s.narration.to_ascii_lowercase();
                n.contains("opposite corner")
            }),
            "draw the rectangle by clicking corners, not by typing sizes"
        );
        assert!(
            !steps.iter().any(|s| {
                matches!(
                    s.anchor,
                    StepAnchor::Ui(UiAnchor::RectWidth) | StepAnchor::Ui(UiAnchor::RectHeight)
                )
            }),
            "must not type width/height into the rectangle tool"
        );
        assert!(
            !steps.iter().any(|s| {
                let n = s.narration.to_ascii_lowercase();
                n.contains("type the width") || n.contains("type the height")
            }),
            "must not ask to type rectangle width/height"
        );
        let dim_tool = steps
            .iter()
            .find(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Dimension))))
            .expect("should pick the Dimension tool");
        assert!(
            dim_tool.narration.to_ascii_lowercase().contains("dimension"),
            "dimension-tool step: {}",
            dim_tool.narration
        );
        let opp_i = box_step_index("opposite corner");
        let dim_i = steps
            .iter()
            .position(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Dimension))))
            .expect("Dimension tool step");
        let extrude_i = steps
            .iter()
            .position(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Extrude))))
            .expect("Extrude tool step");
        assert!(
            opp_i < dim_i && dim_i < extrude_i,
            "draw free, then Dimension tool, then Extrude (got {opp_i}, {dim_i}, {extrude_i})"
        );
        assert!(
            steps.iter().any(|s| {
                matches!(s.anchor, StepAnchor::Ui(UiAnchor::DimensionValue))
                    && s.narration.to_ascii_lowercase().contains("10")
                    && s.narration.to_ascii_lowercase().contains("type")
            }),
            "should type a size into the Dimension tool"
        );
    }

    /// #1315: after the edit, Escape finishes the sketch, then Z zooms to fit.
    #[test]
    fn dimensions_tutorial_escapes_then_zooms_before_the_end() {
        let steps = dimensioned_box().steps;
        let last = steps.len() - 1;
        let change_i = steps
            .iter()
            .position(|s| {
                let n = s.narration.to_ascii_lowercase();
                (n.contains("10") && n.contains("20"))
                    || n.contains("to `20`")
                    || n.contains("to 20")
            })
            .expect("change 10 to 20 step");
        let esc_i = steps
            .iter()
            .position(|s| {
                let n = s.narration.to_ascii_lowercase();
                n.contains("esc") && (n.contains("finish") || n.contains("exit") || n.contains("leave"))
            })
            .expect("Escape to finish the sketch");
        let zoom_i = steps
            .iter()
            .position(|s| {
                let n = s.narration.to_ascii_lowercase();
                n.contains('z') && n.contains("zoom")
            })
            .expect("Z to Zoom to Fit");
        assert!(
            change_i < esc_i && esc_i < zoom_i && zoom_i < last,
            "edit, then Esc, then Z, then the closer (got {change_i}, {esc_i}, {zoom_i}, last={last})"
        );
        let esc = &steps[esc_i];
        assert!(esc.done.is_some(), "Escape should auto-advance when the sketch closes");
        assert!(
            esc.key_hint.is_some_and(|(k, _)| k.eq_ignore_ascii_case("esc")
                || k.eq_ignore_ascii_case("escape")),
            "Escape step should show an Esc key hint: {}",
            esc.narration
        );
        let zoom = &steps[zoom_i];
        assert!(zoom.done.is_some(), "Z should auto-advance when Zoom to Fit lands");
        assert!(
            zoom.key_hint.is_some_and(|(k, _)| k.eq_ignore_ascii_case("z")),
            "Z step should show a Z key hint: {}",
            zoom.narration
        );
    }

    /// #1269: navigate tutorial sits after cube, seeds cubes, and teaches the camera.
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

    /// #1330: the tutorial tooltip covered the exploder loupes — drop that step.
    #[test]
    fn navigate_tutorial_has_no_exploder() {
        let nav = &TUTORIALS[tutorial_index("navigate").unwrap()];
        let joined: String = nav
            .steps
            .iter()
            .map(|s| s.narration.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        for needle in ["exploder", "loupe", "space"] {
            assert!(
                !joined.contains(needle),
                "navigate tutorial should not mention {needle}: {joined}"
            );
        }
        assert!(
            nav.steps
                .iter()
                .all(|s| !s.key_hint.is_some_and(|(k, _)| k.eq_ignore_ascii_case("Space"))),
            "navigate tutorial should not introduce Space / Selection Exploder"
        );
    }

    /// #1269: covers orbit, pan, zoom, bear HUD, and home.
    #[test]
    fn navigate_tutorial_covers_camera_and_bear() {
        let nav = &TUTORIALS[tutorial_index("navigate").unwrap()];
        let joined: String = nav
            .steps
            .iter()
            .map(|s| s.narration.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        for needle in ["orbit", "pan", "zoom", "bear", "home"] {
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
    }

    fn nav_step(needle: &str) -> &'static Step {
        let nav = &TUTORIALS[tutorial_index("navigate").unwrap()];
        nav.steps
            .iter()
            .find(|s| s.narration.to_ascii_lowercase().contains(needle))
            .unwrap_or_else(|| panic!("navigate step matching {needle:?}"))
    }

    fn nav_step_index(needle: &str) -> usize {
        let nav = &TUTORIALS[tutorial_index("navigate").unwrap()];
        nav.steps
            .iter()
            .position(|s| s.narration.to_ascii_lowercase().contains(needle))
            .unwrap_or_else(|| panic!("navigate step matching {needle:?}"))
    }

    /// #1550–#1554: orbit / pan / zoom / home have no "for me" shortcut. After
    /// orbit and after pan, a Next-only "Good job" step makes the next action obvious.
    #[test]
    fn navigate_tutorial_camera_steps_are_hands_on() {
        let nav = &TUTORIALS[tutorial_index("navigate").unwrap()];
        for needle in [
            "right-drag to orbit",
            "middle-drag",
            "scroll the mouse wheel",
            "house under the bear",
        ] {
            let s = nav_step(needle);
            assert!(
                s.assist.is_none(),
                "camera step should have no assist: {}",
                s.narration
            );
            assert!(
                s.done.is_some(),
                "camera step should auto-advance: {}",
                s.narration
            );
        }

        let orbit_i = nav_step_index("right-drag to orbit");
        let orbit_ok = &nav.steps[orbit_i + 1];
        assert_eq!(orbit_ok.narration, "Good job orbiting!");
        assert!(orbit_ok.done.is_none(), "good-job orbiting waits for Next");
        assert!(orbit_ok.assist.is_none());

        let pan_i = nav_step_index("middle-drag");
        let pan_ok = &nav.steps[pan_i + 1];
        assert_eq!(pan_ok.narration, "Good job!");
        assert!(pan_ok.done.is_none(), "good-job after pan waits for Next");
        assert!(pan_ok.assist.is_none());
        assert!(
            orbit_i + 1 < pan_i,
            "good-job orbiting sits between orbit and pan"
        );
        assert!(pan_i + 1 < nav_step_index("scroll the mouse wheel"));

        let bear = nav_step("view cube");
        assert!(
            bear.assist
                .as_ref()
                .is_some_and(|a| a.label.contains("Snap")),
            "bear snap still offers an assist"
        );
    }

    /// #1269 / #1550–#1554: camera motion advances orbit / pan / zoom / home;
    /// Next covers the good-job interstitials; the bear still has an assist.
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
            let n = step.narration.to_ascii_lowercase();
            if n.contains("right-drag to orbit") {
                assist_nav_orbit(&mut app);
                app.advance_tutorial();
            } else if n.contains("middle-drag") {
                assist_nav_pan(&mut app);
                app.advance_tutorial();
            } else if n.contains("scroll the mouse wheel") {
                assist_nav_zoom(&mut app);
                app.advance_tutorial();
            } else if n.contains("house under the bear") {
                assist_nav_home(&mut app);
                app.advance_tutorial();
            } else if step.assist.is_some() {
                app.apply(Action::TutorialAssist);
                if app.tutorial.map(|r| r.step) == Some(run.step) && step.done.is_some() {
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

    fn parameters_tut() -> &'static Tutorial {
        &TUTORIALS[tutorial_index("parameters").expect("parameters tutorial is registered")]
    }

    fn param_step(needle: &str) -> &'static Step {
        parameters_tut()
            .steps
            .iter()
            .find(|s| s.narration.to_ascii_lowercase().contains(needle))
            .unwrap_or_else(|| panic!("parameters step matching {needle:?}"))
    }

    fn param_step_index(needle: &str) -> usize {
        parameters_tut()
            .steps
            .iter()
            .position(|s| s.narration.to_ascii_lowercase().contains(needle))
            .unwrap_or_else(|| panic!("parameters step matching {needle:?}"))
    }

    /// #1347: parameters tutorial sits after Dimensions and teaches named + inline params.
    #[test]
    fn parameters_tutorial_is_registered() {
        let tut = parameters_tut();
        assert_eq!(tut.name, "parameters");
        assert_eq!(tut.title, "Parameters");
        assert_eq!(tutorial_index("parameters"), Some(4));
    }

    /// #1347: create width → rect width / width*2 → change width → extrude
    /// height=30mm → change height.
    #[test]
    fn parameters_tutorial_covers_width_rect_and_inline_height() {
        let steps = parameters_tut().steps;
        let joined: String = steps
            .iter()
            .map(|s| s.narration.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        for needle in [
            "width",
            "20mm",
            "width*2",
            "30mm",
            "height=30mm",
            "50mm",
        ] {
            assert!(
                joined.contains(needle),
                "parameters tutorial should mention {needle}: {joined}"
            );
        }
        assert!(
            steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::ParametersName))),
            "should point at the Parameters name box"
        );
        assert!(
            steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::ParametersValue))),
            "should point at the Parameters value box"
        );
        assert!(
            steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::ParametersAdd))),
            "should point at the Parameters + button"
        );
        assert!(
            steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::RectWidth))),
            "should type into the rectangle width field"
        );
        assert!(
            steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::RectHeight))),
            "should type into the rectangle height field"
        );
        assert!(
            steps.iter().any(|s| {
                matches!(s.anchor, StepAnchor::Ui(UiAnchor::ExtrudeDistance))
                    && s.narration.to_ascii_lowercase().contains("height=30mm")
            }),
            "should type height=30mm into the extrude ValueInput"
        );

        let add_i = param_step_index("your first parameter");
        let rect_w = steps
            .iter()
            .position(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::RectWidth)))
            .expect("rect width typing");
        let change_w = param_step_index("the rectangle stretches");
        let extrude_i = steps
            .iter()
            .position(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Extrude))))
            .expect("extrude tool");
        let inline_i = steps
            .iter()
            .position(|s| s.narration.to_ascii_lowercase().contains("height=30mm"))
            .expect("inline height");
        let change_h = param_step_index("the solid grows");
        assert!(
            add_i < rect_w && rect_w < change_w && change_w < extrude_i && extrude_i < inline_i && inline_i < change_h,
            "order: add width, type rect dims, change width, extrude, height=30mm, change height \
             (got {add_i}, {rect_w}, {change_w}, {extrude_i}, {inline_i}, {change_h})"
        );
    }

    /// #1347: assists create width, a width × width*2 rectangle, then height via the extrude.
    #[test]
    fn parameters_tutorial_assists_build_a_parametric_box() {
        let mut app = AppState::default();
        assist_change_height(&mut app);
        assert!(param_exists(&app, "width"), "width parameter");
        assert!(param_exists(&app, "height"), "height parameter");
        assert!(
            param_length_near(&app, "width", 30.0),
            "width should have been changed to 30mm"
        );
        assert!(
            param_length_near(&app, "height", 50.0),
            "height should have been changed to 50mm"
        );
        assert!(
            dim_expr_eq(&app, "width") && dim_expr_eq(&app, "width*2"),
            "rectangle sides should be width and width*2"
        );
        assert!(has_extrusion(&app));
        let height = crate::value::eval_length_mm_in_doc("height", &app.doc).unwrap_or(0.0);
        assert!(
            (height - 50.0).abs() < 0.51,
            "extrude should follow height, got {height}"
        );
        let ext = app.doc.extrusions.values().next().expect("extrusion");
        assert!(
            ext.expression.to_ascii_lowercase().contains("height"),
            "extrude expression should bind to height, got {}",
            ext.expression
        );
    }

    /// #1347: "do it for me" walks the whole parameters tutorial.
    #[test]
    fn parameters_tutorial_walks_with_assists() {
        let mut app = AppState::default();
        finish_tutorial_via_next(&mut app, "parameters");
        assert!(app.tutorial_completed("parameters"));
        assert!(param_exists(&app, "width"));
        assert!(param_exists(&app, "height"));
        assert!(has_extrusion(&app));
    }

    /// #1347: creating `width` is tap-name, type, tap-value, type, then +.
    #[test]
    fn parameters_tutorial_adds_width_one_action_at_a_time() {
        let name_tap = param_step("name box");
        assert!(matches!(
            name_tap.anchor,
            StepAnchor::Ui(UiAnchor::ParametersName)
        ));
        assert!(name_tap.type_hint.is_none(), "tap before type");

        let type_name = param_step("type `width`");
        assert!(matches!(
            type_name.anchor,
            StepAnchor::Ui(UiAnchor::ParametersName)
        ));
        assert!(matches!(type_name.type_hint, Some(TypeHint::Fixed("width"))));

        let value_tap = param_step("value box");
        assert!(matches!(
            value_tap.anchor,
            StepAnchor::Ui(UiAnchor::ParametersValue)
        ));

        let type_val = steps_containing("20mm");
        assert!(
            type_val
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::ParametersValue))),
            "20mm is typed in the new-parameter value box"
        );

        let add = param_step("your first parameter");
        assert!(matches!(
            add.anchor,
            StepAnchor::Ui(UiAnchor::ParametersAdd)
        ));
    }

    fn steps_containing(needle: &str) -> Vec<&'static Step> {
        parameters_tut()
            .steps
            .iter()
            .filter(|s| s.narration.to_ascii_lowercase().contains(needle))
            .collect()
    }

    /// #1434: unfinished tutorials light the status-bar button.
    #[test]
    fn unfinished_tutorials_highlight_the_button() {
        let mut app = AppState::default();
        assert!(
            app.has_unfinished_tutorials(),
            "a fresh session has walkthroughs left"
        );
        assert!(
            app.tutorials_button_highlighted(),
            "unfinished tutorials make the button bright blue"
        );

        for tut in TUTORIALS {
            app.mark_tutorial_completed(tut.name);
        }
        assert!(!app.has_unfinished_tutorials());
        assert!(
            !app.tutorials_button_highlighted(),
            "finishing every walkthrough drops the highlight"
        );
    }

    /// Mark all complete finishes every walkthrough and kills the blue button / launch prompt.
    #[test]
    fn complete_all_tutorials_marks_every_walkthrough_and_clears_prompt() {
        let mut app = AppState::default();
        app.set_install_age_days(Some(1.0));
        app.prepare_tutorial_prompt();
        assert!(app.tutorials_button_highlighted());
        assert_eq!(app.tutorial_prompt_text(), Some(PROMPT_TEXT));
        app.mark_tutorial_completed(TUTORIALS[0].name);

        app.apply(Action::CompleteAllTutorials);
        assert!(app.completed_tutorials_dirty);
        for tut in TUTORIALS {
            assert!(
                app.tutorial_completed(tut.name),
                "{} should be complete",
                tut.name
            );
        }
        assert!(!app.has_unfinished_tutorials());
        assert!(
            !app.tutorials_button_highlighted(),
            "all complete drops the blue button"
        );
        assert!(
            app.tutorial_prompt_text().is_none(),
            "all complete dismisses the launch prompt"
        );

        app.prepare_tutorial_prompt();
        assert!(
            app.tutorial_prompt_text().is_none(),
            "all complete also blocks re-arming the prompt"
        );
    }

    /// Mark all unstarted clears every completion check and restores the highlight.
    #[test]
    fn unstart_all_tutorials_clears_completion() {
        let mut app = AppState::default();
        app.set_install_age_days(Some(1.0));
        for tut in TUTORIALS {
            app.mark_tutorial_completed(tut.name);
        }
        app.completed_tutorials_dirty = false;
        assert!(!app.tutorials_button_highlighted());

        app.apply(Action::UnstartAllTutorials);
        assert!(app.completed_tutorials_dirty);
        for tut in TUTORIALS {
            assert!(
                !app.tutorial_completed(tut.name),
                "{} should be unstarted",
                tut.name
            );
        }
        assert!(app.has_unfinished_tutorials());
        assert!(
            app.tutorials_button_highlighted(),
            "unstarted walkthroughs light the button again"
        );

        app.prepare_tutorial_prompt();
        assert_eq!(
            app.tutorial_prompt_text(),
            Some(PROMPT_TEXT),
            "unstarting lets the launch prompt arm again"
        );
    }

    /// #1434: the launch tooltip only appears for a fresh install in the first 30 days.
    #[test]
    fn launch_prompt_is_only_for_fresh_installs_under_30_days() {
        let mut app = AppState::default();
        // Upgrade / unknown install age: no prompt, but the button still highlights.
        app.prepare_tutorial_prompt();
        assert!(app.tutorial_prompt_text().is_none());
        assert!(app.tutorials_button_highlighted());

        app.set_install_age_days(Some(31.0));
        app.prepare_tutorial_prompt();
        assert!(
            app.tutorial_prompt_text().is_none(),
            "day 31 is outside the window"
        );

        app.set_install_age_days(Some(0.0));
        app.prepare_tutorial_prompt();
        assert_eq!(app.tutorial_prompt_text(), Some(PROMPT_TEXT));
        assert_eq!(app.tutorial_prompt_alpha(), Some(1.0));

        app.dismiss_tutorial_prompt();
        app.set_install_age_days(Some(29.0));
        app.prepare_tutorial_prompt();
        assert_eq!(
            app.tutorial_prompt_text(),
            Some(PROMPT_TEXT),
            "still inside the first 30 days"
        );
    }

    /// #1434: the launch tooltip holds until the user works, then fades after a few seconds.
    #[test]
    fn launch_prompt_fades_after_the_user_starts_working() {
        let mut app = AppState::default();
        app.set_install_age_days(Some(2.0));
        app.prepare_tutorial_prompt();
        assert_eq!(app.tutorial_prompt_alpha(), Some(1.0));

        app.tick_tutorial_prompt(10.0);
        assert_eq!(
            app.tutorial_prompt_alpha(),
            Some(1.0),
            "idle time does not fade the prompt"
        );

        app.note_document_work();
        app.tick_tutorial_prompt(PROMPT_FADE_AFTER_SECS - 0.1);
        assert_eq!(
            app.tutorial_prompt_alpha(),
            Some(1.0),
            "still fully visible during the hold"
        );

        app.tick_tutorial_prompt(0.1 + PROMPT_FADE_SECS * 0.5);
        let mid = app.tutorial_prompt_alpha().expect("fading");
        assert!(
            mid > 0.0 && mid < 1.0,
            "halfway through the fade, alpha={mid}"
        );

        app.tick_tutorial_prompt(PROMPT_FADE_SECS);
        assert!(
            app.tutorial_prompt_text().is_none(),
            "gone after the fade finishes"
        );
    }

    /// #1434: editing the document counts as starting work (so the prompt can fade).
    #[test]
    fn document_edits_start_the_prompt_fade() {
        let mut app = AppState::default();
        app.set_install_age_days(Some(1.0));
        app.prepare_tutorial_prompt();
        app.apply(Action::SetTool(Tool::Line));
        // A tool change alone is not working on the document; drawing is.
        assert_eq!(app.tutorial_prompt_alpha(), Some(1.0));

        app.apply(Action::AddParameter {
            name: "width".into(),
            expression: "10".into(),
        });
        assert!(
            app.tutorial_prompt()
                .is_some_and(|p| p.work_started),
            "creating a parameter starts the fade clock"
        );
    }

    /// #1558: the Tutorials pane prefixes every walkthrough with its catalog number.
    #[test]
    fn tutorials_are_numbered_in_catalog_order() {
        assert_eq!(numbered_title(0, "Sketch & Extrude"), "1. Sketch & Extrude");
        assert_eq!(numbered_title(7, "Raised text"), "8. Raised text");
        for (i, tut) in TUTORIALS.iter().enumerate() {
            let shown = numbered_title(i, tut.title);
            assert!(
                shown.starts_with(&format!("{}. ", i + 1)),
                "catalog #{i} should start with its number: {shown}"
            );
            assert!(
                shown.ends_with(tut.title),
                "numbering must keep the skill title: {shown}"
            );
        }
        assert_eq!(TUTORIALS.len(), 8);
    }

    fn chamfer_tut() -> &'static Tutorial {
        &TUTORIALS[tutorial_index("chamfer").expect("chamfer tutorial is registered")]
    }

    fn combine_tut() -> &'static Tutorial {
        &TUTORIALS[tutorial_index("combine").expect("combine tutorial is registered")]
    }

    fn raised_text_tut() -> &'static Tutorial {
        &TUTORIALS[tutorial_index("raised_text").expect("raised_text tutorial is registered")]
    }

    /// #1555: sketch-chamfer a rectangle, extrude, then chamfer the top of the solid.
    #[test]
    fn chamfer_tutorial_is_registered_and_covers_sketch_then_solid() {
        let tut = chamfer_tut();
        assert_eq!(tut.name, "chamfer");
        assert_eq!(tut.title, "Chamfer");
        let joined: String = tut
            .steps
            .iter()
            .map(|s| s.narration.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("chamfer"), "{}", tut.steps[0].narration);
        assert!(joined.contains("rectangle") || joined.contains("square"), "{joined}");
        assert!(joined.contains("extrude"), "{joined}");
        assert!(
            joined.contains("top") || joined.contains("side") || joined.contains("edge"),
            "should chamfer a side of the solid: {joined}"
        );
        assert!(
            tut.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Chamfer)))),
            "should pick the Chamfer tool"
        );
        assert!(
            tut.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Extrude)))),
            "should pick the Extrude tool"
        );
    }

    /// #1555: assists produce a sketch chamfer plus a 3D edge chamfer.
    #[test]
    fn chamfer_tutorial_assists_bevel_a_box() {
        let mut app = AppState::default();
        finish_tutorial_via_next(&mut app, "chamfer");
        assert!(
            !app.doc.sketch_vertex_treatment_ops.is_empty(),
            "sketch corner should be chamfered, status={}",
            app.status
        );
        assert!(has_extrusion(&app), "profile should be extruded");
        assert!(
            !app.doc.edge_treatment_ops.is_empty(),
            "solid edges should be chamfered, status={}",
            app.status
        );
        let treated: usize = app
            .doc
            .edge_treatment_ops
            .values()
            .filter(|op| op.kind == crate::model::VertexTreatmentKind::Chamfer)
            .map(|op| op.edges.len())
            .sum();
        assert!(
            treated >= 4,
            "all four edges of one side should be chamfered, got {treated}, status={}",
            app.status
        );
    }

    /// #1556: place a cube and a sphere, then cut the sphere out of the cube.
    #[test]
    fn combine_tutorial_is_registered_and_covers_a_cut() {
        let tut = combine_tut();
        assert_eq!(tut.name, "combine");
        assert_eq!(tut.title, "Combine");
        let joined: String = tut
            .steps
            .iter()
            .map(|s| s.narration.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("combine") || joined.contains("cut"), "{joined}");
        assert!(joined.contains("sphere"), "{joined}");
        assert!(joined.contains("cube") || joined.contains("cuboid"), "{joined}");
        assert!(
            tut.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Combine)))),
            "should pick the Combine tool"
        );
        assert!(
            tut.steps.iter().any(|s| matches!(
                s.anchor,
                StepAnchor::Ui(UiAnchor::CombineKind(crate::model::BooleanOpKind::Cut))
            )),
            "should point at Cut in the Combine mode row"
        );
    }

    /// #1556: assists cut a sphere out of a cube.
    #[test]
    fn combine_tutorial_assists_cut_a_sphere_from_a_cube() {
        let mut app = AppState::default();
        finish_tutorial_via_next(&mut app, "combine");
        assert!(has_cuboid(&app), "cube");
        assert!(has_sphere(&app), "sphere");
        assert_eq!(app.doc.boolean_ops.len(), 1, "one combine op, status={}", app.status);
        let op = app.doc.boolean_ops.values().next().unwrap();
        assert_eq!(op.kind, crate::model::BooleanOpKind::Cut);
        assert!(!op.outputs.is_empty(), "cut should produce a body");
    }

    /// #1566: the overlap-sphere click points at a cuboid bottom corner, not the
    /// in-progress sphere ghost (which follows the cursor).
    #[test]
    fn combine_overlap_orb_sits_on_a_cuboid_bottom_corner() {
        let step = combine_tut()
            .steps
            .iter()
            .find(|s| s.narration.to_ascii_lowercase().contains("overlaps the cube"))
            .expect("overlap-sphere click step");
        let StepAnchor::World(point) = step.anchor else {
            panic!(
                "overlap click should point at a cuboid corner, got {:?}",
                step.anchor
            );
        };

        let mut app = AppState::default();
        assist_place_cuboid(&mut app);
        let cuboid = app
            .doc
            .primitives
            .values()
            .find(|p| p.kind == crate::model::PrimitiveKind::Cuboid)
            .expect("cuboid");
        let corners = crate::primitives::resolve(&app.doc, cuboid)
            .expect("sized cuboid")
            .cuboid_base();

        let mut creating =
            crate::actions::CreatingShape::new(crate::model::PrimitiveKind::Sphere);
        creating.shape.origin = [200.0, 180.0, 0.0];
        creating.phase = crate::actions::ShapePhase::Anchor;
        app.creating_shape = Some(creating);

        let at = point(&app).expect("overlap guide");
        assert!(
            corners.iter().any(|c| (*c - at).length() < 0.1),
            "orb should sit on a cuboid base corner {corners:?}, got {at:?}"
        );
        let ghost = glam::Vec3::from_array(
            app.creating_shape.as_ref().unwrap().shape.origin,
        );
        assert!(
            (at - ghost).length() > 50.0,
            "orb followed the sphere ghost at {ghost:?} instead of a cuboid corner"
        );

        // The orb stays on that corner when the ghost moves.
        app.creating_shape.as_mut().unwrap().shape.origin = [40.0, -90.0, 0.0];
        let again = point(&app).expect("overlap guide after ghost move");
        assert!(
            (again - at).length() < 0.1,
            "orb should not follow the cursor, moved from {at:?} to {again:?}"
        );
    }

    /// #1557: sketch text on a cube and extrude it so the letters stand proud.
    #[test]
    fn raised_text_tutorial_is_registered_and_covers_text_on_a_cube() {
        let tut = raised_text_tut();
        assert_eq!(tut.name, "raised_text");
        assert_eq!(tut.title, "Raised text");
        let joined: String = tut
            .steps
            .iter()
            .map(|s| s.narration.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("text"), "{joined}");
        assert!(joined.contains("cube") || joined.contains("cuboid"), "{joined}");
        assert!(joined.contains("extrude"), "{joined}");
        assert!(
            tut.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Text)))),
            "should pick the Text tool"
        );
        assert!(
            tut.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Extrude)))),
            "should pick the Extrude tool"
        );
    }

    /// #1557: assists stamp raised letters on a cube.
    #[test]
    fn raised_text_tutorial_assists_extrude_letters_on_a_cube() {
        if crate::text::font_bytes("Helvetica", false, false).is_none()
            && crate::text::font_bytes("Arial", false, false).is_none()
            && crate::text::font_bytes("DejaVu Sans", false, false).is_none()
            && crate::text::font_bytes("Liberation Sans", false, false).is_none()
            && crate::text::system_font_families().is_empty()
        {
            eprintln!("no usable system font; skipping");
            return;
        }
        let mut app = AppState::default();
        finish_tutorial_via_next(&mut app, "raised_text");
        assert!(has_cuboid(&app), "cube");
        assert!(
            !app.doc.sketch_texts.is_empty(),
            "letters should exist, status={}",
            app.status
        );
        let text = app.doc.sketch_texts.values().next().unwrap();
        assert!(
            text.text.to_ascii_uppercase().contains("BEAR"),
            "expected BEAR, got {:?}",
            text.text
        );
        assert!(
            app.doc.extrusions.values().any(|e| {
                e.faces
                    .iter()
                    .any(|f| matches!(f, crate::model::ExtrudeFace::TextGlyph { .. }))
            }),
            "text should be extruded, status={}",
            app.status
        );
    }
}
