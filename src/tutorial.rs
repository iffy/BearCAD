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
    /// A named parameter's **value** cell in the Parameters pane (#1347). The name matters
    /// (#1728): every row used to overwrite one shared anchor, so a step that asked for
    /// `plate` rang whichever row the pane drew last.
    ParametersExistingValue(&'static str),
    /// A constraint button in the Context pane's Constraints list (#770) — where a
    /// squaring-up step points once both of its picks are made.
    ConstraintButton(crate::geometric_constraints::GeometricConstraintType),
    /// An Output-row mode button — `"new"` / `"join"` / `"cut"` (#804/#1592). Shared by
    /// Extrude, Revolve, Sweep, Loft and Mirror, so the tutorial names the button, not the tool.
    OutputMode(&'static str),
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
    /// A body row in the Elements pane (#1647) — where the Add-view tool takes its click.
    ElementsBody,
    /// The view-cube bear HUD (#1269).
    ViewCube,
    /// The house (Home view) button under the view cube (#1269).
    ViewHome,
    /// The newest construction-plane row in the Elements pane (#1673) — double-click to
    /// reopen the plane and move it.
    ElementsPlane,
    /// A Context-pane checkbox row, by its label (#1677) — e.g. the Line tool's **Curve**.
    CheckboxRow(&'static str),
    /// One of the Repeat tool's Count / Gap / Distance rows (#1679), named by the variable
    /// so the orb keeps up when the label flips between Gap and Offset.
    RepeatVar(crate::model::RepeatVar),
    /// The **measure icon** at the head of one of those rows (#1741/#1743) — the toggle that
    /// flips Gap to Offset, or Distance between the last copy's far end and its start. A step
    /// that asks for the icon has to ring the icon, not the value field beside it.
    RepeatVarIcon(crate::model::RepeatVar),
    /// The **lock** at the tail of one of those rows (#1742): grey on a value you set, green
    /// on the one the app computes from the other two.
    RepeatVarLock(crate::model::RepeatVar),
    /// A spot on the open drawing page (#1681), one card-width right and one card-height
    /// **up** from the named view's card — where the Aligned-view tool wants its click.
    DrawingSpot {
        view: usize,
        right: i8,
        up: i8,
    },
    /// A **line** on the named view (#1709): the longest edge the view shows, which is the
    /// one the Dimension step asks for — the card's centre rings no line at all.
    DrawingViewEdge {
        view: usize,
    },
    /// The toolbar Zoom to Fit (magnifying glass) button (#1583).
    ZoomToFit,
    /// A status-bar pane toggle (phone layout only, #828): Elements / Context / Params.
    PaneButton(crate::actions::Pane),
    /// The status-bar Tutorials launcher (#1434): the launch prompt points here.
    TutorialsButton,
    /// A Combine-tool Mode button in the Context pane (#1556): Combine / Cut / Intersect / Difference.
    CombineKind(crate::model::BooleanOpKind),
    /// The Text tool's string field in the Context pane (#1557).
    TextContent,
    /// The Dimension tool's derived-parameter **name** field, and the **Derive parameter**
    /// button under it (#1729).
    DeriveName,
    DeriveButton,
    /// The Construction Plane tool's **Tilt** field in the Context pane (#1723) — the one the
    /// walkthrough asks for, as opposed to the Offset field above it.
    PlaneTilt,
    /// The navigation bear in a selected drawing view's Context pane (#1640): where the
    /// view's orientation is picked.
    DrawingViewBear,
    /// The **Style** combo of a selected drawing view (#1640).
    DrawingViewStyle,
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
    // #1591: draw a polygon with the Line tool, then pin it with constraints.
    Tutorial {
        name: "constraints",
        title: "Constraints",
        steps: CONSTRAINTS_STEPS,
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
    // #1640: a page of views — front, two aligned to it, a three-quarter view, dimensions.
    Tutorial {
        name: "drawing",
        title: "Technical drawing",
        steps: DRAWING_STEPS,
    },
    // #1672: spin a square into a ring, then revolve-cut a groove into it.
    Tutorial {
        name: "revolve",
        title: "Revolve",
        steps: REVOLVE_STEPS,
    },
    // #1673: tilt a plane off an axis, build on it, then move the plane.
    Tutorial {
        name: "tilted_plane",
        title: "Angled plane",
        steps: TILTED_PLANE_STEPS,
    },
    // #1674: offset a circle, then extrude the ring between the two.
    Tutorial {
        name: "offset",
        title: "Offset",
        steps: OFFSET_STEPS,
    },
    // #1675: hollow a block into a four-sided box.
    Tutorial {
        name: "shell",
        title: "Shell",
        steps: SHELL_STEPS,
    },
    // #1676: one parameter worked out from another.
    Tutorial {
        name: "derived_parameter",
        title: "Derived parameters",
        steps: DERIVED_PARAMETER_STEPS,
    },
    // #1677: draw an outline with curved sides.
    Tutorial {
        name: "curves",
        title: "Curves",
        steps: CURVES_STEPS,
    },
    // #1678: cut a block in two with a slanted sketch line.
    Tutorial {
        name: "slice",
        title: "Slice",
        steps: SLICE_STEPS,
    },
    // #1679: pattern a block along an axis, working every measure toggle.
    Tutorial {
        name: "repeat",
        title: "Repeat",
        steps: REPEAT_STEPS,
    },
    // #1680: reflect a sketch shape across an axis.
    Tutorial {
        name: "sketch_mirror",
        title: "Mirror in a sketch",
        steps: SKETCH_MIRROR_STEPS,
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

const fn shift_click_step(
    narration: &'static str,
    anchor: StepAnchor,
    done: Option<fn(&AppState) -> bool>,
    needs_shift: fn(&AppState) -> bool,
) -> Step {
    Step {
        narration,
        anchor,
        done,
        on_enter: None,
        assist: None,
        needs_shift: Some(needs_shift),
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

/// Extrude face picked (distance field open) or extrusion already committed. Picking the
/// tool arms an *empty* draft (#1499), which doesn't count -- otherwise the "click the
/// face" step advances the instant the tool button is pressed (#1697).
fn extrude_face_picked(app: &AppState) -> bool {
    app.creating_extrusion
        .as_ref()
        .is_some_and(|ce| !ce.faces.is_empty())
        || has_extrusion(app)
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

/// Keep only the datum planes a walkthrough actually draws on (#1722/#1725). The rest are
/// big translucent slabs standing in front of the very thing the step points at. `keep` names
/// them by datum order — 0 = XY (the ground), 1 = XZ, 2 = YZ.
fn keep_datum_planes(app: &mut AppState, keep: &[usize]) {
    let planes: Vec<_> = app.doc.construction_planes.keys().collect();
    for (nth, index) in planes.into_iter().enumerate() {
        if keep.contains(&nth) {
            continue;
        }
        app.apply(Action::DeleteElement {
            element: crate::hierarchy::SceneElement::ConstructionPlane(index),
        });
    }
}

/// Every walkthrough but the first sketches on the ground, so XZ and YZ only get in the way.
fn keep_the_ground_plane(app: &mut AppState) {
    keep_datum_planes(app, &[0]);
}

/// The Shapes walkthrough stands its cylinder on the XZ wall (#1273), so that one stays.
fn keep_the_ground_and_wall_planes(app: &mut AppState) {
    keep_datum_planes(app, &[0, 1]);
}

/// The angled-plane walkthrough builds its own plane off an axis and never touches a datum,
/// so all three go — leaving nothing but the axes it asks you to click (#1722).
fn keep_no_datum_planes(app: &mut AppState) {
    keep_datum_planes(app, &[]);
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

/// Grab-Shape-tool step: the toolbar is armed, even if the last-used kind isn't cuboid
/// (#1569). A following Cuboid-kind step then points at the Context button.
fn shape_tool_active_or_has_cuboid(app: &AppState) -> bool {
    shape_tool_active(app) || has_cuboid(app)
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

/// #1269: second walkthrough — pan, orbit, zoom, Zoom to Fit, bear HUD, home.
/// Starts with cubes already in the document. One action per step (#1253).
/// The Selection Exploder step is gone (#1330): its tooltip covered the loupes.
/// Orbit / pan / zoom / Zoom to Fit / home have no "for me" assist (#1550–#1554 / #1583);
/// Next-only "Good job" steps sit after orbit and pan so the next action is obvious.
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
    Step {
        narration: "Click Zoom to Fit, or press `Z`, to frame the model.",
        anchor: StepAnchor::Ui(UiAnchor::ZoomToFit),
        done: Some(zoomed_to_fit),
        on_enter: None,
        assist: None,
        needs_shift: None,
        drag_hint: None,
        key_hint: Some(("Z", "Zoom to Fit")),
        marks: None,
        type_hint: None,
        phone_narration: Some("Tap Zoom to Fit to frame the model."),
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
        "That's the view: orbit, pan, zoom, Zoom to Fit, the bear, and Home. Nice!",
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
    plain_step_enter(
        "The Shape tool makes solids right in 3D",
        StepAnchor::None,
        None,
        keep_the_ground_and_wall_planes,
    ),
    plain_step(
        "Grab the Shape tool \u{2014} the glowing button, or press `B`.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Shape)),
        Some(shape_tool_active_or_has_cuboid),
    ),
    // #1569: last-used kind may be cylinder/sphere; skip this when already cuboid.
    plain_step(
        "Click Cuboid in the Context pane (or press `B`).",
        StepAnchor::Ui(UiAnchor::ShapeKind(crate::model::PrimitiveKind::Cuboid)),
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
        "Type the radius: `10`.",
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
    plain_step_enter(
        "Hi! We'll make a cube, then change it with dimensions.",
        StepAnchor::None,
        None,
        keep_the_ground_plane,
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
    plain_step_enter(
        "Hi! Let's drive a box with parameters.",
        StepAnchor::None,
        None,
        keep_the_ground_plane,
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
        "Type `width` in the width field.",
        StepAnchor::Ui(UiAnchor::RectWidth),
        Some(rect_width_is_width),
        StepAssist {
            label: "Draw it for me",
            run: assist_draw_parametric_rect,
        },
        Some(TypeHint::Fixed("width")),
    ),
    assisted_step(
        "Press `Tab` to reach the height field.",
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
        StepAnchor::Ui(UiAnchor::ParametersExistingValue("width")),
        Some(width_value_open),
    ),
    assisted_step(
        "Change it to `30mm`. The rectangle stretches.",
        StepAnchor::Ui(UiAnchor::ParametersExistingValue("width")),
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
        StepAnchor::Ui(UiAnchor::ParametersExistingValue("height")),
        Some(height_value_open),
    ),
    assisted_step(
        "Change it to `50mm`. The solid grows.",
        StepAnchor::Ui(UiAnchor::ParametersExistingValue("height")),
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

fn line_is_parallel_to_axis(
    app: &AppState,
    line: crate::model::LineKey,
    axis: crate::model::SketchAxis,
) -> bool {
    use crate::model::ConstraintLine;
    live_constraints(app).any(|c| match &c.kind {
        ConstraintKind::Parallel { line_a, line_b } => {
            let this = ConstraintLine::Line(line);
            let ax = ConstraintLine::OriginAxis(axis);
            (*line_a == this && *line_b == ax) || (*line_b == this && *line_a == ax)
        }
        _ => false,
    })
}

fn has_axis_parallel(app: &AppState, axis: crate::model::SketchAxis) -> bool {
    app.doc
        .lines
        .keys()
        .any(|line| line_is_parallel_to_axis(app, line, axis))
}

fn has_equal_constraint(app: &AppState) -> bool {
    live_constraints(app).any(|c| matches!(c.kind, ConstraintKind::Equal { .. }))
}

/// A Parallel between two *lines* (#1700) -- the axis-based Horizontal/Vertical buttons also
/// store Parallel, but against an `OriginAxis`, so those don't count here.
fn has_line_parallel_constraint(app: &AppState) -> bool {
    use crate::model::ConstraintLine;
    live_constraints(app).any(|c| {
        matches!(
            &c.kind,
            ConstraintKind::Parallel { line_a, line_b }
                if matches!(line_a, ConstraintLine::Line(_))
                    && matches!(line_b, ConstraintLine::Line(_))
        )
    })
}

fn has_horizontal_constraint(app: &AppState) -> bool {
    has_axis_parallel(app, crate::model::SketchAxis::X)
}

fn has_vertical_constraint(app: &AppState) -> bool {
    has_axis_parallel(app, crate::model::SketchAxis::Y)
}

/// Slightly irregular quad — a freehand polygon, not a rectangle.
const POLY_UV: [(f32, f32); 4] = [
    (20.0, 20.0),
    (55.0, 18.0),
    (58.0, 52.0),
    (18.0, 50.0),
];

fn sketch_drawn_line_count(app: &AppState) -> usize {
    app.doc.lines.values().filter(|l| !l.construction).count()
}

/// A closed outline in any sketch. Three sides is the least that encloses anything (#1733) --
/// requiring four missed a user who joined three curved sides back to the start.
fn has_closed_polygon(app: &AppState) -> bool {
    app.doc.sketches.keys().any(|sk| {
        crate::polygon::closed_line_loops(&app.doc, sk)
            .iter()
            .any(|loop_| loop_.len() >= 3)
    })
}

fn line_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Line
        || app.creating_line.is_some()
        || sketch_drawn_line_count(app) > 0
        || has_closed_polygon(app)
}

fn constraint_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Constraint
        || has_axis_parallel(app, crate::model::SketchAxis::X)
        || has_equal_constraint(app)
}

fn first_poly_vertex_placed(app: &AppState) -> bool {
    app.creating_line.is_some() || sketch_drawn_line_count(app) >= 1 || has_closed_polygon(app)
}

fn poly_has_one_side(app: &AppState) -> bool {
    sketch_drawn_line_count(app) >= 1 || has_closed_polygon(app)
}

fn poly_has_two_sides(app: &AppState) -> bool {
    sketch_drawn_line_count(app) >= 2 || has_closed_polygon(app)
}

fn poly_has_three_sides(app: &AppState) -> bool {
    sketch_drawn_line_count(app) >= 3 || has_closed_polygon(app)
}

fn selected_line_keys(app: &AppState) -> Vec<crate::model::LineKey> {
    app.scene_selection
        .iter()
        .filter_map(|e| match e {
            crate::hierarchy::SceneElement::Line(k) => Some(k),
            _ => None,
        })
        .collect()
}

fn horizontal_side_picked(app: &AppState) -> bool {
    has_axis_parallel(app, crate::model::SketchAxis::X) || !selected_line_keys(app).is_empty()
}

fn vertical_side_picked(app: &AppState) -> bool {
    has_axis_parallel(app, crate::model::SketchAxis::Y)
        || selected_line_keys(app)
            .iter()
            .any(|&k| !line_is_parallel_to_axis(app, k, crate::model::SketchAxis::X))
}

fn equal_first_side_picked(app: &AppState) -> bool {
    has_equal_constraint(app)
        || selected_line_keys(app).len() >= 2
        || selected_line_keys(app).iter().any(|&k| {
            !line_is_parallel_to_axis(app, k, crate::model::SketchAxis::X)
                && !line_is_parallel_to_axis(app, k, crate::model::SketchAxis::Y)
        })
}

fn equal_second_side_picked(app: &AppState) -> bool {
    has_equal_constraint(app) || selected_line_keys(app).len() >= 2
}

fn equal_needs_shift(app: &AppState) -> bool {
    !equal_second_side_picked(app)
}

fn poly_vertex_guide(app: &AppState, nth: usize) -> Option<glam::Vec3> {
    let (u, v) = POLY_UV[nth % POLY_UV.len()];
    ground_local(app, u, v)
}

fn poly_vertex_0(app: &AppState) -> Option<glam::Vec3> {
    poly_vertex_guide(app, 0)
}

fn poly_vertex_1(app: &AppState) -> Option<glam::Vec3> {
    poly_vertex_guide(app, 1)
}

fn poly_vertex_2(app: &AppState) -> Option<glam::Vec3> {
    poly_vertex_guide(app, 2)
}

fn poly_vertex_3(app: &AppState) -> Option<glam::Vec3> {
    poly_vertex_guide(app, 3)
}

fn poly_side_mid(app: &AppState, nth: usize) -> Option<glam::Vec3> {
    let lines = first_sketch_rect_lines(app);
    if let Some(&key) = lines.get(nth) {
        if let Some(l) = app.doc.lines.get(key) {
            if let Some(frame) = crate::face::sketch_geometry_frame(&app.doc, l.sketch) {
                return Some(crate::face::local_to_world(
                    &frame,
                    (l.x0 + l.x1) * 0.5,
                    (l.y0 + l.y1) * 0.5,
                ));
            }
        }
    }
    let (u0, v0) = POLY_UV[nth % POLY_UV.len()];
    let (u1, v1) = POLY_UV[(nth + 1) % POLY_UV.len()];
    ground_local(app, (u0 + u1) * 0.5, (v0 + v1) * 0.5)
}

fn poly_bottom_mid(app: &AppState) -> Option<glam::Vec3> {
    poly_side_mid(app, 0)
}

fn poly_right_mid(app: &AppState) -> Option<glam::Vec3> {
    poly_side_mid(app, 1)
}

fn poly_top_mid(app: &AppState) -> Option<glam::Vec3> {
    poly_side_mid(app, 2)
}

fn ensure_line_sketch_for_tutorial(app: &mut AppState) {
    if app.tool != Tool::Line {
        app.apply(Action::SetTool(Tool::Line));
    }
    ensure_ground_sketch(app);
}

fn ensure_constraint_step(app: &mut AppState) {
    if !has_closed_polygon(app) {
        assist_draw_polygon(app);
    }
    if app.tool != Tool::Constraint {
        app.apply(Action::SetTool(Tool::Constraint));
    }
}

fn select_poly_line(app: &mut AppState, nth: usize) {
    app.scene_selection.clear();
    if let Some(&key) = first_sketch_rect_lines(app).get(nth) {
        app.scene_selection
            .insert(crate::hierarchy::SceneElement::Line(key));
    }
}

fn select_poly_lines(app: &mut AppState, a: usize, b: usize) {
    app.scene_selection.clear();
    let lines = first_sketch_rect_lines(app);
    for nth in [a, b] {
        if let Some(&key) = lines.get(nth) {
            app.scene_selection
                .insert(crate::hierarchy::SceneElement::Line(key));
        }
    }
}

fn ensure_poly_sketch_open(app: &mut AppState) {
    if app.sketch_session.is_some() {
        return;
    }
    let existing = app.doc.sketches.keys().next();
    if let Some(sketch) = existing {
        app.apply(Action::OpenSketch {
            sketch,
            viewport: None,
        });
        return;
    }
    ensure_ground_sketch(app);
}

fn assist_draw_polygon(app: &mut AppState) {
    if has_closed_polygon(app) {
        ensure_poly_sketch_open(app);
        return;
    }
    ensure_poly_sketch_open(app);
    let Some(session) = app.sketch_session else {
        return;
    };
    crate::construction::add_line_polygon(&mut app.doc, session.sketch, &POLY_UV);
    app.creating_line = None;
    app.refresh_document_health();
}

fn assist_horizontal(app: &mut AppState) {
    if has_axis_parallel(app, crate::model::SketchAxis::X) {
        return;
    }
    assist_draw_polygon(app);
    select_poly_line(app, 0);
    let _ = app.apply(Action::AddGeometricConstraint(
        crate::geometric_constraints::GeometricConstraintType::AlongXAxis,
    ));
}

fn assist_vertical(app: &mut AppState) {
    if has_axis_parallel(app, crate::model::SketchAxis::Y) {
        return;
    }
    assist_horizontal(app);
    select_poly_line(app, 1);
    let _ = app.apply(Action::AddGeometricConstraint(
        crate::geometric_constraints::GeometricConstraintType::AlongYAxis,
    ));
}

fn assist_equal(app: &mut AppState) {
    if has_equal_constraint(app) {
        return;
    }
    assist_vertical(app);
    select_poly_lines(app, 0, 2);
    let _ = app.apply(Action::AddGeometricConstraint(
        crate::geometric_constraints::GeometricConstraintType::Equal,
    ));
}

fn assist_parallel(app: &mut AppState) {
    if has_line_parallel_constraint(app) {
        return;
    }
    assist_equal(app);
    select_poly_lines(app, 0, 2);
    let _ = app.apply(Action::AddGeometricConstraint(
        crate::geometric_constraints::GeometricConstraintType::Parallel,
    ));
}

/// #1591: draw a four-sided polygon with the Line tool, then pin it with
/// horizontal, vertical, and equal constraints.
static CONSTRAINTS_STEPS: &[Step] = &[
    plain_step_enter(
        "Let's draw a polygon and pin it with constraints.",
        StepAnchor::None,
        None,
        keep_the_ground_plane,
    ),
    plain_step(
        "Click the Line tool \u{2014} glowing button, or `L`.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Line)),
        Some(line_tool_active),
    ),
    plain_step_enter(
        "Click a first corner on the ground.",
        StepAnchor::World(poly_vertex_0),
        Some(first_poly_vertex_placed),
        ensure_line_sketch_for_tutorial,
    ),
    plain_step(
        "Click the next corner.",
        StepAnchor::World(poly_vertex_1),
        Some(poly_has_one_side),
    ),
    plain_step(
        "Click the next.",
        StepAnchor::World(poly_vertex_2),
        Some(poly_has_two_sides),
    ),
    plain_step(
        "Click the next.",
        StepAnchor::World(poly_vertex_3),
        Some(poly_has_three_sides),
    ),
    assisted_step(
        "Click the first corner again to close the polygon.",
        StepAnchor::World(poly_vertex_0),
        Some(has_closed_polygon),
        StepAssist {
            label: "Draw it for me",
            run: assist_draw_polygon,
        },
        None,
    ),
    plain_step(
        "Click the Constraint tool \u{2014} glowing button, or `C`.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Constraint)),
        Some(constraint_tool_active),
    ),
    plain_step_enter(
        "Click the bottom side.",
        StepAnchor::World(poly_bottom_mid),
        Some(horizontal_side_picked),
        ensure_constraint_step,
    ),
    assisted_step(
        "Click Parallel to X axis (or press `6`) \u{2014} that makes it horizontal.",
        StepAnchor::Ui(UiAnchor::ConstraintButton(
            crate::geometric_constraints::GeometricConstraintType::AlongXAxis,
        )),
        Some(has_horizontal_constraint),
        StepAssist {
            label: "Do it for me",
            run: assist_horizontal,
        },
        None,
    ),
    plain_step(
        "Click a neighbouring side.",
        StepAnchor::World(poly_right_mid),
        Some(vertical_side_picked),
    ),
    assisted_step(
        "Click Parallel to Y axis (or press `7`) \u{2014} now it's vertical.",
        StepAnchor::Ui(UiAnchor::ConstraintButton(
            crate::geometric_constraints::GeometricConstraintType::AlongYAxis,
        )),
        Some(has_vertical_constraint),
        StepAssist {
            label: "Do it for me",
            run: assist_vertical,
        },
        None,
    ),
    plain_step(
        "Click one more side.",
        StepAnchor::World(poly_top_mid),
        Some(equal_first_side_picked),
    ),
    shift_click_step(
        "Shift-click another side.",
        StepAnchor::World(poly_bottom_mid),
        Some(equal_second_side_picked),
        equal_needs_shift,
    ),
    assisted_step(
        "Click Equal (or press `3`) \u{2014} those two stay the same length.",
        StepAnchor::Ui(UiAnchor::ConstraintButton(
            crate::geometric_constraints::GeometricConstraintType::Equal,
        )),
        Some(has_equal_constraint),
        StepAssist {
            label: "Do it for me",
            run: assist_equal,
        },
        None,
    ),
    assisted_step(
        "Both are still picked \u{2014} click Parallel (or press `1`). The top swings level \
         with the bottom.",
        StepAnchor::Ui(UiAnchor::ConstraintButton(
            crate::geometric_constraints::GeometricConstraintType::Parallel,
        )),
        Some(has_line_parallel_constraint),
        StepAssist {
            label: "Do it for me",
            run: assist_parallel,
        },
        None,
    ),
    plain_step(
        "That's constraints: facts the solver keeps true. Drag a corner \u{2014} those \
         sides stay put. Nice!",
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
    let n = app
        .doc
        .extrusions
        .get(extrusion)
        .and_then(|ext| ext.faces.first())
        .map(crate::extrude::side_face_count)
        .unwrap_or(0);
    // Every top-cap edge of the profile — the four remaining square sides plus the
    // cutoff corner after the sketch chamfer.
    let edges: Vec<_> = (0..n)
        .map(|edge| {
            (
                crate::model::TreatableSolid::Extrusion(extrusion),
                crate::model::ExtrusionEdgeRef::Cap {
                    face: 0,
                    edge,
                    top: true,
                },
            )
        })
        .collect();
    let _ = app.apply(Action::CommitEdgeTreatments {
        edges,
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
            flip: existing.flip,
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
        flip: false,
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
    plain_step_enter(
        "Let's chamfer!",
        StepAnchor::None,
        None,
        keep_the_ground_plane,
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
    plain_step_enter(
        "Let's learn how to cut shapes with the Combine tool.",
        StepAnchor::None,
        None,
        keep_the_ground_plane,
    ),
    plain_step(
        "Grab the Shape tool \u{2014} the glowing button, or press `B`.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Shape)),
        Some(shape_tool_active_or_has_cuboid),
    ),
    // #1569: last-used kind may be cylinder/sphere; skip this when already cuboid.
    plain_step(
        "Click Cuboid in the Context pane (or press `B`).",
        StepAnchor::Ui(UiAnchor::ShapeKind(crate::model::PrimitiveKind::Cuboid)),
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
    plain_step_enter(
        "Let's stamp raised letters on a cube.",
        StepAnchor::None,
        None,
        keep_the_ground_plane,
    ),
    plain_step(
        "Grab the Shape tool \u{2014} the glowing button, or press `B`.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Shape)),
        Some(shape_tool_active_or_has_cuboid),
    ),
    // #1569: last-used kind may be cylinder/sphere; skip this when already cuboid.
    plain_step(
        "Click Cuboid in the Context pane (or press `B`).",
        StepAnchor::Ui(UiAnchor::ShapeKind(crate::model::PrimitiveKind::Cuboid)),
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

// --- Technical drawing tutorial (#1640) --------------------------------------------

/// Seed an L-shaped bracket: a 60x20x20 base with a 20x20x50 upright at one end. Every
/// straight-on view of it is different, so the walkthrough's three aligned views each say
/// something — and the three-quarter view has a corner worth looking at.
fn seed_drawing_bracket(app: &mut AppState) {
    if !app.doc.bodies.is_empty() {
        return;
    }
    // Named, so the Elements row and every view label read "Bracket" — the word the
    // narration uses — instead of "Body 2" (#1702).
    let planes: Vec<_> = app.doc.construction_planes.keys().collect();
    for index in planes {
        app.apply(Action::DeleteElement {
            element: crate::hierarchy::SceneElement::ConstructionPlane(index),
        });
    }
    for (origin, w, d, h) in [
        ([0.0, 0.0, 0.0], "60", "20", "20"),
        ([0.0, 0.0, 20.0], "20", "20", "50"),
    ] {
        let mut shape = crate::model::Primitive::new(crate::model::PrimitiveKind::Cuboid);
        shape.origin = origin;
        shape.width = w.into();
        shape.depth = d.into();
        shape.height = h.into();
        app.apply(Action::CreateShape { shape });
    }
    let bodies: Vec<_> = app.doc.bodies.keys().collect();
    if bodies.len() == 2 {
        app.apply(Action::CreateBooleanOperation {
            kind: crate::model::BooleanOpKind::Combine,
            a: vec![bodies[0], bodies[1]],
            b: Vec::new(),
            keep_b: false,
            solid_count: None,
        });
    }
    if let Some(body) = app.doc.bodies.keys().last() {
        app.apply(Action::CommitElementName {
            element: crate::hierarchy::SceneElement::Body(body),
            name: "Bracket".to_string(),
        });
    }
}

/// The drawing the walkthrough is building, if it exists yet.
fn tutorial_drawing(app: &AppState) -> Option<crate::model::DrawingKey> {
    app.doc.drawings.keys().next()
}

fn drawing_views(app: &AppState) -> usize {
    tutorial_drawing(app)
        .and_then(|d| app.doc.drawings.get(d))
        .map(|d| d.views.len())
        .unwrap_or(0)
}

fn has_drawing(app: &AppState) -> bool {
    tutorial_drawing(app).is_some()
}

fn drawing_add_ready(app: &AppState) -> bool {
    app.tool == Tool::DrawingAdd || drawing_views(app) >= 1
}

/// The Add-view tool a second time round (#1649), for the three-quarter view.
fn drawing_add_ready_again(app: &AppState) -> bool {
    app.tool == Tool::DrawingAdd || drawing_has_extra_view(app)
}

/// Where the walkthrough wants each view. The Add-view tool drops a projection at the page
/// centre and leaves it to be dragged, so the tutorial parks it itself (#1648) — the steps
/// that follow ask for views *above* and *right* of the base one, which needs the room.
const FRONT_VIEW_SPOT: (f32, f32) = (0.3, 0.62);
const EXTRA_VIEW_SPOT: (f32, f32) = (0.72, 0.68);

fn park_view(app: &mut AppState, view: usize, (pos_x, pos_y): (f32, f32)) {
    let Some(drawing) = tutorial_drawing(app) else {
        return;
    };
    if app.doc.drawings.get(drawing).is_none_or(|d| d.views.len() <= view) {
        return;
    }
    app.apply(Action::MoveDrawingView { drawing, view, pos_x, pos_y });
}

/// Park the base view lower-left before asking for the views that line up with it (#1648).
fn park_front_view(app: &mut AppState) {
    park_view(app, 0, FRONT_VIEW_SPOT);
}

/// Park the fourth view in the empty corner before turning it (#1648).
fn park_extra_view(app: &mut AppState) {
    if let Some(view) = free_extra_view(app) {
        park_view(app, view, EXTRA_VIEW_SPOT);
    }
}

fn drawing_has_a_view(app: &AppState) -> bool {
    drawing_views(app) >= 1
}

fn aligned_children(app: &AppState) -> usize {
    tutorial_drawing(app)
        .and_then(|d| app.doc.drawings.get(d))
        .map(|d| d.views.iter().filter(|v| v.aligned_parent.is_some()).count())
        .unwrap_or(0)
}

fn drawing_align_ready(app: &AppState) -> bool {
    app.tool == Tool::DrawingAlign || aligned_children(app) >= 1
}

/// #1704: the Aligned-view tool has its base view. Picked by clicking a projection, or
/// seeded from a lone selected one when the tool comes up.
fn drawing_align_base_picked(app: &AppState) -> bool {
    app.drawing_align_parent.is_some() || aligned_children(app) >= 1
}

fn drawing_has_one_aligned(app: &AppState) -> bool {
    aligned_children(app) >= 1
}

fn drawing_has_two_aligned(app: &AppState) -> bool {
    aligned_children(app) >= 2
}

/// A fourth view, not aligned to anything — the one that becomes the three-quarter look.
fn free_extra_view(app: &AppState) -> Option<usize> {
    let d = app.doc.drawings.get(tutorial_drawing(app)?)?;
    d.views
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, v)| v.aligned_parent.is_none())
        .map(|(i, _)| i)
}

fn drawing_has_extra_view(app: &AppState) -> bool {
    free_extra_view(app).is_some()
}

fn extra_view_is_angled(app: &AppState) -> bool {
    let Some(key) = tutorial_drawing(app) else {
        return false;
    };
    let Some(index) = free_extra_view(app) else {
        return false;
    };
    app.doc.drawings.get(key).and_then(|d| d.views.get(index)).is_some_and(|v| {
        matches!(
            v.orientation,
            crate::model::DrawingOrientation::Corner(_)
                | crate::model::DrawingOrientation::Isometric
                | crate::model::DrawingOrientation::Edge(_)
        )
    })
}

fn extra_view_is_shaded(app: &AppState) -> bool {
    let Some(key) = tutorial_drawing(app) else {
        return false;
    };
    let Some(index) = free_extra_view(app) else {
        return false;
    };
    app.doc
        .drawings
        .get(key)
        .and_then(|d| d.views.get(index))
        .is_some_and(|v| v.style == crate::model::DrawingViewStyle::Shaded)
}

fn drawing_dimension_ready(app: &AppState) -> bool {
    app.tool == Tool::Dimension || drawing_has_a_dimension(app)
}

fn drawing_has_a_dimension(app: &AppState) -> bool {
    tutorial_drawing(app)
        .and_then(|d| app.doc.drawings.get(d))
        .is_some_and(|d| {
            d.views
                .iter()
                .any(|v| !v.dimensioned_edges.is_empty() || !v.point_dims.is_empty())
        })
}

fn assist_make_drawing(app: &mut AppState) {
    if has_drawing(app) {
        return;
    }
    app.apply(Action::CreateDrawing { name: None });
}

fn assist_add_front_view(app: &mut AppState) {
    assist_make_drawing(app);
    if drawing_has_a_view(app) {
        return;
    }
    let (Some(drawing), Some(body)) = (tutorial_drawing(app), app.doc.bodies.keys().last()) else {
        return;
    };
    app.apply(Action::AddDrawingView {
        drawing,
        bodies: vec![body],
        orientation: crate::model::DrawingOrientation::Front,
    });
    // Bottom-left of the page, so the aligned views have room above and to the right.
    park_view(app, 0, FRONT_VIEW_SPOT);
}

fn assist_align(app: &mut AppState, dir: crate::model::AlignDir, pos: f32) {
    assist_add_front_view(app);
    let Some(drawing) = tutorial_drawing(app) else {
        return;
    };
    app.apply(Action::AddAlignedDrawingView { drawing, parent: 0, dir, pos });
}

fn assist_align_top(app: &mut AppState) {
    if drawing_has_one_aligned(app) {
        return;
    }
    assist_align(app, crate::model::AlignDir::Above, 0.25);
}

fn assist_align_side(app: &mut AppState) {
    if drawing_has_two_aligned(app) {
        return;
    }
    assist_align_top(app);
    assist_align(app, crate::model::AlignDir::Right, 0.72);
}

fn assist_add_extra_view(app: &mut AppState) {
    assist_align_side(app);
    if drawing_has_extra_view(app) {
        return;
    }
    let (Some(drawing), Some(body)) = (tutorial_drawing(app), app.doc.bodies.keys().last()) else {
        return;
    };
    app.apply(Action::AddDrawingView {
        drawing,
        bodies: vec![body],
        orientation: crate::model::DrawingOrientation::Front,
    });
    let view = drawing_views(app) - 1;
    park_view(app, view, EXTRA_VIEW_SPOT);
}

fn assist_angle_extra_view(app: &mut AppState) {
    assist_add_extra_view(app);
    let (Some(drawing), Some(view)) = (tutorial_drawing(app), free_extra_view(app)) else {
        return;
    };
    app.apply(Action::SetDrawingViewOrientation {
        drawing,
        view,
        orientation: crate::model::DrawingOrientation::Corner(
            crate::model::CornerView::FrontRightTop,
        ),
    });
}

fn assist_shade_extra_view(app: &mut AppState) {
    assist_angle_extra_view(app);
    let (Some(drawing), Some(view)) = (tutorial_drawing(app), free_extra_view(app)) else {
        return;
    };
    app.apply(Action::SetDrawingViewStyle {
        drawing,
        view,
        style: crate::model::DrawingViewStyle::Shaded,
    });
}

/// Dimension the base's long bottom edge on the front view — the one anyone would reach for.
fn assist_dimension_front(app: &mut AppState) {
    assist_add_front_view(app);
    if drawing_has_a_dimension(app) {
        return;
    }
    let Some(drawing) = tutorial_drawing(app) else {
        return;
    };
    let Some(view) = app.doc.drawings.get(drawing).and_then(|d| d.views.first()).cloned() else {
        return;
    };
    let views = app.doc.drawings[drawing].views.clone();
    let edges = crate::drawing::drawing_view_dimensionable_edges(&app.doc, &views, &view);
    let (right, up) = crate::drawing::resolved_view_axes(&views, &view);
    // The longest edge that isn't edge-on in this view: the one the drawing shows off.
    let best = edges
        .iter()
        .filter(|(a, b)| {
            let (pa, pb) = (
                glam::Vec2::new(a.dot(right), a.dot(up)),
                glam::Vec2::new(b.dot(right), b.dot(up)),
            );
            (pb - pa).length() > 1e-3
        })
        .max_by(|x, y| {
            (x.1 - x.0)
                .length()
                .partial_cmp(&(y.1 - y.0).length())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied();
    let Some((a, b)) = best else {
        return;
    };
    let q = crate::hierarchy::quantize_body_point;
    app.apply(Action::ToggleDrawingDimension {
        drawing,
        view: 0,
        a: q(a),
        b: q(b),
    });
}

/// #1640: a page of views. Add the first, align two more to it, add a three-quarter view and
/// shade it, then dimension something. One action per step; every drawing action has a
/// "do it for me" button, because the page is a workbench of its own and the orb can only
/// point at the tools and the Context pane from here.
static DRAWING_STEPS: &[Step] = &[
    plain_step_enter(
        "Here's a bracket. Let's put it on a technical drawing.",
        StepAnchor::None,
        None,
        seed_drawing_bracket,
    ),
    assisted_step(
        "A drawing is a page of views. Make one from the CAD menu's `New Drawing`.",
        StepAnchor::None,
        Some(has_drawing),
        StepAssist {
            label: "Make the drawing",
            run: assist_make_drawing,
        },
        None,
    ),
    plain_step(
        "Click the Projection tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::DrawingAdd)),
        Some(drawing_add_ready),
    ),
    assisted_step(
        "Now click the bracket in the Elements pane: a front view lands on the page.",
        StepAnchor::Ui(UiAnchor::ElementsBody),
        Some(drawing_has_a_view),
        StepAssist {
            label: "Place it for me",
            run: assist_add_front_view,
        },
        None,
    ),
    plain_step_enter(
        "Click the Aligned view tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::DrawingAlign)),
        Some(drawing_align_ready),
        // The view is parked lower-left first, so "above" and "to the right" have room.
        park_front_view,
    ),
    plain_step(
        "Click the front view: that's what the next two views line up with.",
        StepAnchor::Ui(UiAnchor::DrawingViewEdge { view: 0 }),
        Some(drawing_align_base_picked),
    ),
    assisted_step(
        "Click above the front view: that's the top view, lined up with it.",
        StepAnchor::Ui(UiAnchor::DrawingSpot { view: 0, right: 0, up: 1 }),
        Some(drawing_has_one_aligned),
        StepAssist {
            label: "Add the top view",
            run: assist_align_top,
        },
        None,
    ),
    assisted_step(
        "Now click to the right of the front view for the side view.",
        StepAnchor::Ui(UiAnchor::DrawingSpot { view: 0, right: 1, up: 0 }),
        Some(drawing_has_two_aligned),
        StepAssist {
            label: "Add the side view",
            run: assist_align_side,
        },
        None,
    ),
    plain_step(
        "Three views, lined up and sharing a scale. Dashed projection lines join them.",
        StepAnchor::None,
        None,
    ),
    plain_step(
        "Back to the Projection tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::DrawingAdd)),
        Some(drawing_add_ready_again),
    ),
    assisted_step(
        "Click the bracket in the Elements pane again: that's a fourth view.",
        StepAnchor::Ui(UiAnchor::ElementsBody),
        Some(drawing_has_extra_view),
        StepAssist {
            label: "Add the view",
            run: assist_add_extra_view,
        },
        None,
    ),
    assisted_step_enter(
        "In the Context pane, click a corner dot on the bear: the view turns to that angle.",
        StepAnchor::Ui(UiAnchor::DrawingViewBear),
        Some(extra_view_is_angled),
        StepAssist {
            label: "Turn it for me",
            run: assist_angle_extra_view,
        },
        None,
        // Parked in the empty corner first, so the three-quarter view has the page to itself.
        park_extra_view,
    ),
    assisted_step(
        "Set that view's Style to Shaded, so the three-quarter view reads as a solid.",
        StepAnchor::Ui(UiAnchor::DrawingViewStyle),
        Some(extra_view_is_shaded),
        StepAssist {
            label: "Shade it for me",
            run: assist_shade_extra_view,
        },
        None,
    ),
    plain_step(
        "Click the Dimension tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Dimension)),
        Some(drawing_dimension_ready),
    ),
    assisted_step(
        "Click a line on the front view to dimension it.",
        StepAnchor::Ui(UiAnchor::DrawingViewEdge { view: 0 }),
        Some(drawing_has_a_dimension),
        StepAssist {
            label: "Dimension one for me",
            run: assist_dimension_front,
        },
        None,
    ),
    plain_step(
        "That's a drawing: three aligned views, a three-quarter view, and a dimension. \
         File \u{25b8} Export sends it to SVG or PDF.",
        StepAnchor::None,
        None,
    ),
];

// --- Revolve tutorial (#1672) ----------------------------------------------------------

fn revolve_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Revolve
}

fn circle_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Circle
}

fn has_revolution(app: &AppState) -> bool {
    !app.doc.revolutions.is_empty()
}

/// The revolve tutorial's profile sketch, in insertion order: the rectangle's sketch.
fn revolve_sketch(app: &AppState) -> Option<crate::model::SketchId> {
    app.doc
        .lines
        .values()
        .find(|l| !l.construction)
        .map(|l| l.sketch)
        .or_else(|| app.doc.sketches.keys().next())
}

/// Bounds of the drawn square in ground-local mm: `(u_min, v_min, u_max, v_max)`. The
/// revolve axis is world X, which is the ground frame's `u`, so `v` is the radius.
fn revolve_rect_bounds(app: &AppState) -> Option<(f32, f32, f32, f32)> {
    let sketch = revolve_sketch(app)?;
    let frame = crate::face::sketch_geometry_frame(&app.doc, sketch)?;
    let mut bounds: Option<(f32, f32, f32, f32)> = None;
    for line in app.doc.lines.values().filter(|l| !l.construction && l.sketch == sketch) {
        for p in crate::face::line_world_polyline(&app.doc, line)? {
            let (u, v) = crate::face::world_to_local(&frame, p);
            bounds = Some(match bounds {
                None => (u, v, u, v),
                Some((u0, v0, u1, v1)) => (u0.min(u), v0.min(v), u1.max(u), v1.max(v)),
            });
        }
    }
    bounds
}

fn revolve_rect_corner_a(app: &AppState) -> Option<glam::Vec3> {
    ground_local(app, 10.0, 10.0)
}

fn revolve_rect_corner_b(app: &AppState) -> Option<glam::Vec3> {
    if let (Some(cr), Some(session)) = (app.creating_rect.as_ref(), app.sketch_session) {
        let frame = crate::face::sketch_geometry_frame(&app.doc, session.sketch)?;
        let (ou, ov) = crate::face::world_to_local(&frame, cr.origin);
        return Some(crate::face::local_to_world(&frame, ou + 20.0, ov + 20.0));
    }
    ground_local(app, 30.0, 30.0)
}

/// A point out along the global X axis — clear of the square, so the axis click can't
/// land on the profile instead.
fn revolve_axis_guide(app: &AppState) -> Option<glam::Vec3> {
    ground_local(app, 60.0, 0.0)
}

fn revolve_face_picked(app: &AppState) -> bool {
    has_revolution(app)
        || app
            .creating_revolve
            .as_ref()
            .is_some_and(|c| !c.faces.is_empty())
}

fn revolve_axis_picked(app: &AppState) -> bool {
    has_revolution(app)
        || app.creating_revolve.as_ref().is_some_and(|c| c.axis.is_some())
}

/// Sketch reopened so the groove profile can go in beside the square.
fn revolve_sketch_reopened(app: &AppState) -> bool {
    app.sketch_session.is_some() || has_groove_circle(app)
}

fn has_groove_circle(app: &AppState) -> bool {
    !app.doc.circles.is_empty()
}

fn groove_circle_started(app: &AppState) -> bool {
    app.creating_circle.is_some() || has_groove_circle(app)
}

/// Middle of the square's outer edge — where the groove circle is centred, half in the
/// material and half out of it.
fn groove_center_guide(app: &AppState) -> Option<glam::Vec3> {
    let (u0, _, u1, v1) = revolve_rect_bounds(app)?;
    ground_local(app, (u0 + u1) * 0.5, v1)
}

/// The top of the revolved ring: the outer surface, a quarter turn round from the sketch.
fn ring_body_guide(app: &AppState) -> Option<glam::Vec3> {
    let (u0, _, u1, v1) = revolve_rect_bounds(app)?;
    Some(glam::Vec3::new((u0 + u1) * 0.5, 0.0, v1))
}

/// #1719: any profile the fresh Revolve pick took. The groove circle straddles the square's
/// outer edge, so a click inside it lands on the **overlap** — a `Boolean` region of circle
/// and square, not the bare `Circle` this used to insist on, and the walkthrough sat there
/// with the profile plainly picked in the pane.
fn groove_profile_picked(app: &AppState) -> bool {
    has_groove_cut(app)
        || app.creating_revolve.as_ref().is_some_and(|c| !c.faces.is_empty())
}

fn revolve_cut_mode_ready(app: &AppState) -> bool {
    has_groove_cut(app)
        || app
            .creating_revolve
            .as_ref()
            .is_some_and(|c| c.body_choice == crate::actions::RevolveBodyChoice::Cut)
}

fn revolve_cut_body_picked(app: &AppState) -> bool {
    has_groove_cut(app)
        || app
            .creating_revolve
            .as_ref()
            .is_some_and(|c| !c.cut_bodies.is_empty())
}

fn has_groove_cut(app: &AppState) -> bool {
    app.doc
        .revolutions
        .values()
        .any(|r| matches!(r.mode, crate::model::RevolveMode::Cut(_)))
}

fn assist_draw_revolve_square(app: &mut AppState) {
    if has_rectangle_outline(app) {
        return;
    }
    ensure_ground_sketch(app);
    app.apply(Action::CreateRectangle {
        x: 10.0,
        y: 10.0,
        width: 20.0,
        height: 20.0,
        width_expr: Some("20".into()),
        height_expr: Some("20".into()),
    });
}

/// Every non-construction line of the profile sketch, as one closed profile.
fn revolve_profile_faces(app: &AppState) -> Option<(crate::model::SketchId, Vec<crate::model::ExtrudeFace>)> {
    let sketch = revolve_sketch(app)?;
    let lines = crate::polygon::closed_line_loops(&app.doc, sketch)
        .into_iter()
        .max_by_key(|l| l.len())?;
    (lines.len() >= 4).then(|| (sketch, vec![crate::model::ExtrudeFace::Polygon(lines)]))
}

fn assist_revolve_ring(app: &mut AppState) {
    if has_revolution(app) {
        return;
    }
    assist_draw_revolve_square(app);
    let Some((sketch, faces)) = revolve_profile_faces(app) else {
        return;
    };
    if app.sketch_session.is_some() {
        app.apply(Action::ExitSketch);
    }
    app.apply(Action::CreateRevolution {
        sketch,
        faces,
        axis: crate::model::RevolveAxis::X,
        angle_deg: 360.0,
        angle_expression: "360".into(),
        angle_is_revolutions: false,
        pitch_mm: 0.0,
        pitch_expression: "0".into(),
        gap_is_offset: true,
        symmetric: false,
        body: crate::actions::RevolveBodyChoice::NewBody,
        bodies: Vec::new(),
    });
}

fn assist_draw_groove_circle(app: &mut AppState) {
    if has_groove_circle(app) {
        return;
    }
    assist_revolve_ring(app);
    let Some(sketch) = revolve_sketch(app) else {
        return;
    };
    let Some((u0, _, u1, v1)) = revolve_rect_bounds(app) else {
        return;
    };
    if app.sketch_session.is_none() {
        app.apply(Action::OpenSketch { sketch, viewport: None });
    }
    app.apply(Action::CreateCircle {
        cx: (u0 + u1) * 0.5,
        cy: v1,
        r: 5.0,
        diameter_expr: Some("10".into()),
    });
}

fn assist_cut_groove(app: &mut AppState) {
    if has_groove_cut(app) {
        return;
    }
    assist_draw_groove_circle(app);
    let Some(sketch) = revolve_sketch(app) else {
        return;
    };
    let Some(circle) = app.doc.circles.keys().next() else {
        return;
    };
    let Some(ring) = app.doc.bodies.iter().find_map(|(bi, b)| {
        (!b.shadow && matches!(b.source, crate::model::BodySource::Revolve(_))).then_some(bi)
    }) else {
        return;
    };
    if app.sketch_session.is_some() {
        app.apply(Action::ExitSketch);
    }
    app.apply(Action::CreateRevolution {
        sketch,
        faces: vec![crate::model::ExtrudeFace::Circle(circle)],
        axis: crate::model::RevolveAxis::X,
        angle_deg: 360.0,
        angle_expression: "360".into(),
        angle_is_revolutions: false,
        pitch_mm: 0.0,
        pitch_expression: "0".into(),
        gap_is_offset: true,
        symmetric: false,
        body: crate::actions::RevolveBodyChoice::Cut,
        bodies: vec![ring],
    });
}

/// #1672: spin a square into a ring, then revolve-cut a half-round groove into its face.
static REVOLVE_STEPS: &[Step] = &[
    plain_step_enter(
        "Revolve spins a flat profile around an axis. Let's make a ring.",
        StepAnchor::None,
        None,
        keep_the_ground_plane,
    ),
    plain_step(
        "Click the Rectangle tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Rectangle)),
        Some(rectangle_tool_active),
    ),
    plain_step_enter(
        "Click a corner on the ground, out beside the red X axis.",
        StepAnchor::World(revolve_rect_corner_a),
        Some(rect_first_corner_placed),
        ensure_rect_sketch_for_tutorial,
    ),
    assisted_step(
        "Click the opposite corner to close the square.",
        StepAnchor::World(revolve_rect_corner_b),
        Some(has_rectangle_outline),
        StepAssist {
            label: "Draw it for me",
            run: assist_draw_revolve_square,
        },
        None,
    ),
    plain_step(
        "Click the Revolve tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Revolve)),
        Some(revolve_tool_active),
    ),
    plain_step(
        "Click inside the square \u{2014} that's the profile.",
        StepAnchor::World(rectangle_face_guide),
        Some(revolve_face_picked),
    ),
    plain_step(
        "Click the red X axis. The square will spin around it.",
        StepAnchor::World(revolve_axis_guide),
        Some(revolve_axis_picked),
    ),
    assisted_step(
        "Press Enter. A full turn sweeps the square into a ring.",
        StepAnchor::None,
        Some(has_revolution),
        StepAssist {
            label: "Revolve it for me",
            run: assist_revolve_ring,
        },
        None,
    ),
    plain_step(
        "Now a groove. Reopen the sketch \u{2014} double-click it in the Elements pane.",
        StepAnchor::Ui(UiAnchor::ElementsSketch),
        Some(revolve_sketch_reopened),
    ),
    plain_step(
        "Click the Circle tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Circle)),
        Some(circle_tool_active),
    ),
    plain_step(
        "Click the middle of the square's outer edge \u{2014} the far side from the axis.",
        StepAnchor::World(groove_center_guide),
        Some(groove_circle_started),
    ),
    assisted_step(
        "Type `10` for the diameter, then Enter. Half the circle hangs outside the square.",
        StepAnchor::None,
        Some(has_groove_circle),
        StepAssist {
            label: "Draw it for me",
            run: assist_draw_groove_circle,
        },
        Some(TypeHint::Fixed("10")),
    ),
    plain_step(
        "Click the Revolve tool again.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Revolve)),
        Some(revolve_tool_active),
    ),
    plain_step(
        "Click inside the circle.",
        StepAnchor::World(groove_center_guide),
        Some(groove_profile_picked),
    ),
    plain_step(
        "Click the red X axis again.",
        StepAnchor::World(revolve_axis_guide),
        Some(revolve_axis_picked),
    ),
    plain_step(
        "Click Cut in the Output row \u{2014} this revolve takes material away.",
        StepAnchor::Ui(UiAnchor::OutputMode("cut")),
        Some(revolve_cut_mode_ready),
    ),
    plain_step(
        "Click the ring to say what gets cut.",
        StepAnchor::World(ring_body_guide),
        Some(revolve_cut_body_picked),
    ),
    assisted_step(
        "Press Enter. A half-round groove runs right round the ring's face.",
        StepAnchor::None,
        Some(has_groove_cut),
        StepAssist {
            label: "Cut it for me",
            run: assist_cut_groove,
        },
        None,
    ),
    plain_step(
        "Revolve builds anything round: rings, shafts, pulleys. Nice work!",
        StepAnchor::None,
        None,
    ),
];


// --- Angled construction plane tutorial (#1673) ----------------------------------------

/// The tilt the walkthrough builds with, and the one it moves to.
const TILT_FIRST: &str = "30";
const TILT_SECOND: &str = "60";

/// Put the keyboard on **Tilt**, not the Offset field above it (#1723): the step says to
/// type the tilt, and the plane tool opens with Offset armed.
fn ensure_plane_tilt_focus(app: &mut AppState) {
    if app.creating_plane.is_some() {
        app.apply(Action::FocusPlaneDim { dim: crate::construction::PlaneDim::Angle });
    }
}

fn plane_tool_active(app: &AppState) -> bool {
    app.tool == Tool::ConstructionPlane
}

/// The plane this walkthrough makes: the only one hung off an axis. The document's three
/// datum planes are face-anchored, so this picks out the user's.
fn tutorial_tilted_plane(app: &AppState) -> Option<crate::model::ConstructionPlaneKey> {
    app.doc
        .construction_planes
        .iter()
        .find(|(_, p)| p.definition.is_axis())
        .map(|(k, _)| k)
}

fn tilt_axis_picked(app: &AppState) -> bool {
    tutorial_tilted_plane(app).is_some()
        || app
            .creating_plane
            .as_ref()
            .is_some_and(|c| c.reference.is_axis())
}

fn has_tilted_plane(app: &AppState) -> bool {
    tutorial_tilted_plane(app).is_some()
}

/// The plane has been re-tilted by the last step.
fn plane_moved(app: &AppState) -> bool {
    tutorial_tilted_plane(app).is_some_and(|k| {
        expr_eq(&app.doc.construction_planes[k].definition.angle_expression, TILT_SECOND)
    })
}

fn plane_reopened(app: &AppState) -> bool {
    plane_moved(app)
        || app
            .creating_plane
            .as_ref()
            .is_some_and(|c| c.edit_index.is_some())
}

/// A point out along the global Y axis, clear of the origin gizmo.
fn tilt_axis_guide(app: &AppState) -> Option<glam::Vec3> {
    let _ = app;
    Some(glam::Vec3::new(0.0, 60.0, 0.0))
}

/// The middle of the tilted plane — where the Sketch tool takes its click.
fn tilted_plane_guide(app: &AppState) -> Option<glam::Vec3> {
    let plane = tutorial_tilted_plane(app)?;
    let frame = crate::face::sketch_frame(
        &app.doc,
        crate::model::FaceId::ConstructionPlane(plane),
    )?;
    Some(crate::face::local_to_world(&frame, 30.0, 30.0))
}

fn sketch_on_tilted_plane(app: &AppState) -> bool {
    let Some(plane) = tutorial_tilted_plane(app) else {
        return false;
    };
    let face = crate::model::FaceId::ConstructionPlane(plane);
    app.doc.sketches.values().any(|s| s.face == face)
}

fn tilted_rect_corner_a(app: &AppState) -> Option<glam::Vec3> {
    ground_local(app, 10.0, 10.0)
}

fn tilted_rect_corner_b(app: &AppState) -> Option<glam::Vec3> {
    if let (Some(cr), Some(session)) = (app.creating_rect.as_ref(), app.sketch_session) {
        let frame = crate::face::sketch_geometry_frame(&app.doc, session.sketch)?;
        let (ou, ov) = crate::face::world_to_local(&frame, cr.origin);
        return Some(crate::face::local_to_world(&frame, ou + 30.0, ov + 30.0));
    }
    ground_local(app, 40.0, 40.0)
}

fn assist_tilt_plane(app: &mut AppState) {
    if has_tilted_plane(app) {
        return;
    }
    app.apply(Action::BeginConstructionPlane {
        reference: crate::construction::PlaneReference::Axis {
            origin: glam::Vec3::ZERO,
            direction: glam::Vec3::Y,
            label: "Y axis".to_string(),
        },
        parent: crate::model::ConstructionPlaneParent::Root,
    });
    app.apply(Action::SetPlaneAngle { value: TILT_FIRST.to_string() });
    app.apply(Action::CommitConstructionPlane);
}

fn assist_sketch_on_tilted_plane(app: &mut AppState) {
    if sketch_on_tilted_plane(app) {
        return;
    }
    assist_tilt_plane(app);
    let Some(plane) = tutorial_tilted_plane(app) else {
        return;
    };
    app.apply(Action::BeginSketch {
        face: crate::model::FaceId::ConstructionPlane(plane),
        viewport: None,
    });
}

fn assist_draw_tilted_rect(app: &mut AppState) {
    if has_rectangle_outline(app) {
        return;
    }
    assist_sketch_on_tilted_plane(app);
    app.apply(Action::CreateRectangle {
        x: 10.0,
        y: 10.0,
        width: 30.0,
        height: 30.0,
        width_expr: Some("30".into()),
        height_expr: Some("30".into()),
    });
}

fn assist_extrude_off_tilted_plane(app: &mut AppState) {
    if has_extrusion(app) {
        return;
    }
    assist_draw_tilted_rect(app);
    let Some(sketch) = app.doc.lines.values().find(|l| !l.construction).map(|l| l.sketch) else {
        return;
    };
    let Some(lines) = crate::polygon::closed_line_loops(&app.doc, sketch)
        .into_iter()
        .max_by_key(|l| l.len())
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

fn assist_move_tilted_plane(app: &mut AppState) {
    if plane_moved(app) {
        return;
    }
    assist_extrude_off_tilted_plane(app);
    let Some(plane) = tutorial_tilted_plane(app) else {
        return;
    };
    app.apply(Action::BeginEditConstructionPlane { index: plane });
    app.apply(Action::SetPlaneAngle { value: TILT_SECOND.to_string() });
    app.apply(Action::CommitConstructionPlane);
}

/// #1673: tilt a plane off an axis, build a solid on it, then move the plane and watch the
/// solid come with it.
static TILTED_PLANE_STEPS: &[Step] = &[
    plain_step_enter(
        "Sketches live on planes. Tilt the plane and everything on it tilts too.",
        StepAnchor::None,
        None,
        keep_no_datum_planes,
    ),
    plain_step(
        "Click the Construction Plane tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::ConstructionPlane)),
        Some(plane_tool_active),
    ),
    plain_step(
        "Click the green Y axis. The new plane will pivot around it.",
        StepAnchor::World(tilt_axis_guide),
        Some(tilt_axis_picked),
    ),
    assisted_step_enter(
        "Type `30` for the tilt, then Enter.",
        StepAnchor::Ui(UiAnchor::PlaneTilt),
        Some(has_tilted_plane),
        StepAssist {
            label: "Tilt it for me",
            run: assist_tilt_plane,
        },
        Some(TypeHint::Fixed(TILT_FIRST)),
        ensure_plane_tilt_focus,
    ),
    plain_step(
        "Click the Sketch tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Sketch)),
        Some(sketch_tool_active),
    ),
    assisted_step(
        "Click the tilted plane to draw on it.",
        StepAnchor::World(tilted_plane_guide),
        Some(sketch_on_tilted_plane),
        StepAssist {
            label: "Open it for me",
            run: assist_sketch_on_tilted_plane,
        },
        None,
    ),
    plain_step(
        "Click the Rectangle tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Rectangle)),
        Some(rectangle_tool_active),
    ),
    plain_step(
        "Click a corner on the tilted plane.",
        StepAnchor::World(tilted_rect_corner_a),
        Some(rect_first_corner_placed),
    ),
    assisted_step(
        "Click the opposite corner.",
        StepAnchor::World(tilted_rect_corner_b),
        Some(has_rectangle_outline),
        StepAssist {
            label: "Draw it for me",
            run: assist_draw_tilted_rect,
        },
        None,
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
        "Type `10`, then Enter. The solid grows square to the tilted plane.",
        StepAnchor::Ui(UiAnchor::ExtrudeDistance),
        Some(has_extrusion),
        StepAssist {
            label: "Extrude it for me",
            run: assist_extrude_off_tilted_plane,
        },
        Some(TypeHint::Fixed("10")),
    ),
    plain_step(
        "Now move the plane. Double-click it in the Elements pane.",
        StepAnchor::Ui(UiAnchor::ElementsPlane),
        Some(plane_reopened),
    ),
    assisted_step(
        "Change the tilt to `60`, then Enter. The solid swings round with the plane.",
        StepAnchor::None,
        Some(plane_moved),
        StepAssist {
            label: "Move it for me",
            run: assist_move_tilted_plane,
        },
        Some(TypeHint::Fixed(TILT_SECOND)),
    ),
    plain_step(
        "That's the chain: plane holds sketch, sketch drives solid. Move the plane, \
         move the part. Nice!",
        StepAnchor::None,
        None,
    ),
];

// --- Offset tutorial (#1674) -----------------------------------------------------------

/// Where the walkthrough's circle sits on the ground, and how big it and its copy are.
const OFFSET_CENTRE_MM: (f32, f32) = (20.0, 20.0);
const OFFSET_RADIUS_MM: f32 = 20.0;
/// How far the offset copy sits outside the original.
const OFFSET_DISTANCE_MM: f32 = 5.0;

fn offset_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Offset
}

fn has_offset_op(app: &AppState) -> bool {
    !app.doc.sketch_offset_ops.is_empty()
}

fn offset_circle_picked(app: &AppState) -> bool {
    has_offset_op(app)
        || app
            .creating_sketch_offset
            .as_ref()
            .is_some_and(|c| !c.circle_targets.is_empty())
}

/// The circle the walkthrough draws — the first one, before the offset copies it.
fn offset_source_circle(app: &AppState) -> Option<crate::model::CircleKey> {
    app.doc.circles.keys().next()
}

fn offset_circle_drawn(app: &AppState) -> bool {
    !app.doc.circles.is_empty()
}

fn offset_circle_started(app: &AppState) -> bool {
    app.creating_circle.is_some() || offset_circle_drawn(app)
}

fn offset_centre_guide(app: &AppState) -> Option<glam::Vec3> {
    ground_local(app, OFFSET_CENTRE_MM.0, OFFSET_CENTRE_MM.1)
}

/// A point on the drawn circle — where Offset takes its pick.
fn offset_circle_edge_guide(app: &AppState) -> Option<glam::Vec3> {
    let (cx, cy, r) = match offset_source_circle(app).map(|k| &app.doc.circles[k]) {
        Some(c) => (c.cx, c.cy, c.r),
        None => (OFFSET_CENTRE_MM.0, OFFSET_CENTRE_MM.1, OFFSET_RADIUS_MM),
    };
    ground_local(app, cx + r, cy)
}

/// A point in the ring between the circle and its offset copy — where Extrude picks.
fn offset_ring_guide(app: &AppState) -> Option<glam::Vec3> {
    let (cx, cy, r) = match offset_source_circle(app).map(|k| &app.doc.circles[k]) {
        Some(c) => (c.cx, c.cy, c.r),
        None => (OFFSET_CENTRE_MM.0, OFFSET_CENTRE_MM.1, OFFSET_RADIUS_MM),
    };
    ground_local(app, cx + r + OFFSET_DISTANCE_MM * 0.5, cy)
}

fn assist_draw_offset_circle(app: &mut AppState) {
    if offset_circle_drawn(app) {
        return;
    }
    ensure_ground_sketch(app);
    app.apply(Action::CreateCircle {
        cx: OFFSET_CENTRE_MM.0,
        cy: OFFSET_CENTRE_MM.1,
        r: OFFSET_RADIUS_MM,
        diameter_expr: Some(format!("{}", OFFSET_RADIUS_MM * 2.0)),
    });
}

fn assist_offset_circle(app: &mut AppState) {
    if has_offset_op(app) {
        return;
    }
    assist_draw_offset_circle(app);
    let Some(circle) = offset_source_circle(app) else {
        return;
    };
    let Some(sketch) = app.doc.circles.get(circle).map(|c| c.sketch) else {
        return;
    };
    app.apply(Action::CreateSketchOffsetOperation {
        sketch,
        line_targets: Vec::new(),
        circle_targets: vec![circle],
        distance: OFFSET_DISTANCE_MM.to_string(),
        construction: false,
    });
}

fn assist_extrude_offset_ring(app: &mut AppState) {
    if has_extrusion(app) {
        return;
    }
    assist_offset_circle(app);
    // Inner is the circle that was drawn; outer is the offset copy.
    let mut circles: Vec<_> = app
        .doc
        .circles
        .iter()
        .map(|(k, c)| (k, c.r, c.sketch))
        .collect();
    circles.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let [inner, outer] = circles.as_slice() else {
        return;
    };
    let sketch = outer.2;
    if app.sketch_session.is_some() {
        app.apply(Action::ExitSketch);
    }
    app.apply(Action::CreateExtrusion {
        sketch,
        faces: vec![crate::model::ExtrudeFace::Boolean {
            op: crate::model::BooleanOp::Difference,
            a: Box::new(crate::model::ExtrudeFace::Circle(outer.0)),
            b: Box::new(crate::model::ExtrudeFace::Circle(inner.0)),
        }],
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

/// #1674: offset a circle to get a parallel copy, then extrude the ring between them.
static OFFSET_STEPS: &[Step] = &[
    plain_step_enter(
        "Offset copies sketch geometry a set distance away \u{2014} that's how you get a wall.",
        StepAnchor::None,
        None,
        keep_the_ground_plane,
    ),
    plain_step(
        "Click the Circle tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Circle)),
        Some(circle_tool_active),
    ),
    plain_step_enter(
        "Click the ground to set the centre.",
        StepAnchor::World(offset_centre_guide),
        Some(offset_circle_started),
        ensure_ground_sketch,
    ),
    assisted_step(
        "Type `40` for the diameter, then Enter.",
        StepAnchor::None,
        Some(offset_circle_drawn),
        StepAssist {
            label: "Draw it for me",
            run: assist_draw_offset_circle,
        },
        Some(TypeHint::Fixed("40")),
    ),
    plain_step(
        "Click the Offset tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Offset)),
        Some(offset_tool_active),
    ),
    plain_step(
        "Click the circle.",
        StepAnchor::World(offset_circle_edge_guide),
        Some(offset_circle_picked),
    ),
    assisted_step(
        "Type `5`, then Enter. A second circle appears 5 mm outside the first.",
        StepAnchor::None,
        Some(has_offset_op),
        StepAssist {
            label: "Offset it for me",
            run: assist_offset_circle,
        },
        Some(TypeHint::Fixed("5")),
    ),
    plain_step(
        "Click the Extrude tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Extrude)),
        Some(extrude_tool_active),
    ),
    plain_step(
        "Click the ring between the two circles.",
        StepAnchor::World(offset_ring_guide),
        Some(extrude_face_picked),
    ),
    assisted_step(
        "Type `10`, then Enter. A tube with a 5 mm wall.",
        StepAnchor::Ui(UiAnchor::ExtrudeDistance),
        Some(has_extrusion),
        StepAssist {
            label: "Extrude it for me",
            run: assist_extrude_offset_ring,
        },
        Some(TypeHint::Fixed("10")),
    ),
    plain_step(
        "The offset stays linked: change the circle and the wall follows. Nice!",
        StepAnchor::None,
        None,
    ),
];

// --- Shell tutorial (#1675) ------------------------------------------------------------

/// Wall thickness the walkthrough shells to.
const SHELL_THICKNESS_MM: f32 = 2.0;

fn shell_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Shell
}

fn has_shell(app: &AppState) -> bool {
    !app.doc.shell_ops.is_empty()
}

fn shell_body_picked(app: &AppState) -> bool {
    has_shell(app)
        || app
            .creating_shell
            .as_ref()
            .is_some_and(|c| !c.targets.is_empty())
}

/// How many faces the in-progress shell has opened (or the committed one holds).
fn shell_open_face_count(app: &AppState) -> usize {
    if let Some(op) = app.doc.shell_ops.values().next() {
        return op.open_faces.len();
    }
    app.creating_shell
        .as_ref()
        .map(|c| c.open_faces.len())
        .unwrap_or(0)
}

fn shell_top_opened(app: &AppState) -> bool {
    shell_open_face_count(app) >= 1
}

fn shell_both_ends_opened(app: &AppState) -> bool {
    shell_open_face_count(app) >= 2
}

/// Which of the block's four sides the camera is looking at (#1727) — the walkthrough opens
/// the top and one **side**, so the second face has to be one you can see without turning the
/// model over. `None` until the block exists.
fn shell_open_side(app: &AppState) -> Option<(crate::model::PrimitiveKey, u8)> {
    let (prim, shape) = app
        .doc
        .primitives
        .iter()
        .find(|(_, p)| p.kind == crate::model::PrimitiveKind::Cuboid)?;
    let eye = app.cam.eye();
    let best = (0u8..4)
        .filter_map(|edge| {
            let poly = crate::primitives::face_polygon(
                &app.doc,
                shape,
                crate::model::PrimitiveFace::CuboidSide { edge },
            )?;
            let centre: glam::Vec3 =
                poly.iter().copied().sum::<glam::Vec3>() / poly.len() as f32;
            // Outward normal of a CCW loop about it: two edges of the quad.
            let n = (poly[1] - poly[0]).cross(poly[3] - poly[0]).normalize_or_zero();
            Some((edge, n.dot((eye - centre).normalize_or_zero())))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    Some((prim, best.0))
}

/// The tutorial's cuboid and the two faces it leaves open: the top, and the side facing you.
fn shell_cuboid_faces(
    app: &AppState,
) -> Option<(crate::model::BodyKey, Vec<crate::model::FaceId>)> {
    let (prim, side) = shell_open_side(app)?;
    let body = live_body_for_primitive(app, crate::model::PrimitiveKind::Cuboid)?;
    Some((
        body,
        vec![
            crate::model::FaceId::PrimitiveFace {
                primitive: prim,
                face: crate::model::PrimitiveFace::CuboidTop,
            },
            crate::model::FaceId::PrimitiveFace {
                primitive: prim,
                face: crate::model::PrimitiveFace::CuboidSide { edge: side },
            },
        ],
    ))
}

/// The middle of that side, for the orb to ring.
fn cuboid_near_side_guide(app: &AppState) -> Option<glam::Vec3> {
    let (prim, side) = shell_open_side(app)?;
    let poly = crate::primitives::face_polygon(
        &app.doc,
        &app.doc.primitives[prim],
        crate::model::PrimitiveFace::CuboidSide { edge: side },
    )?;
    Some(poly.iter().copied().sum::<glam::Vec3>() / poly.len() as f32)
}

/// Open one more of the block's caps, the way the two click steps do.
fn assist_open_shell_face(app: &mut AppState, want: usize) {
    if shell_open_face_count(app) >= want || has_shell(app) {
        return;
    }
    assist_place_cuboid(app);
    let Some((body, faces)) = shell_cuboid_faces(app) else {
        return;
    };
    let cs = app
        .creating_shell
        .get_or_insert_with(crate::actions::CreatingShell::default);
    if cs.targets.is_empty() {
        cs.targets.push(body);
    }
    cs.picking_faces = true;
    cs.open_faces = faces.into_iter().take(want).collect();
}

fn assist_open_shell_top(app: &mut AppState) {
    assist_open_shell_face(app, 1);
}

fn assist_open_shell_side(app: &mut AppState) {
    assist_open_shell_face(app, 2);
}

fn assist_shell_the_box(app: &mut AppState) {
    if has_shell(app) {
        return;
    }
    assist_open_shell_face(app, 2);
    let Some((body, faces)) = shell_cuboid_faces(app) else {
        return;
    };
    app.apply(Action::CreateShellOperation {
        targets: vec![body],
        open_faces: faces,
        thickness: SHELL_THICKNESS_MM.to_string(),
    });
}

/// #1675: shell a block with both caps open, leaving a four-walled box.
static SHELL_STEPS: &[Step] = &[
    plain_step_enter(
        "Shell hollows a solid out to a wall. Leave two faces open and a block becomes a \
         tray you can see into.",
        StepAnchor::None,
        None,
        keep_the_ground_plane,
    ),
    plain_step(
        "Grab the Shape tool \u{2014} the glowing button, or press `B`.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Shape)),
        Some(shape_tool_active_or_has_cuboid),
    ),
    plain_step(
        "Click Cuboid in the Context pane (or press `B`).",
        StepAnchor::Ui(UiAnchor::ShapeKind(crate::model::PrimitiveKind::Cuboid)),
        Some(cuboid_kind_ready),
    ),
    plain_step(
        "Click a ground corner to anchor the block.",
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
        "Click the Shell tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Shell)),
        Some(shell_tool_active),
    ),
    plain_step(
        "Click the block \u{2014} that's what gets hollowed.",
        StepAnchor::World(cuboid_body_guide),
        Some(shell_body_picked),
    ),
    assisted_step(
        "Click the top face to leave it open.",
        StepAnchor::World(cuboid_top_guide),
        Some(shell_top_opened),
        StepAssist {
            label: "Open it for me",
            run: assist_open_shell_top,
        },
        None,
    ),
    assisted_step(
        "Click the side facing you to open that one too.",
        StepAnchor::World(cuboid_near_side_guide),
        Some(shell_both_ends_opened),
        StepAssist {
            label: "Open it for me",
            run: assist_open_shell_side,
        },
        None,
    ),
    assisted_step(
        "Type `2` for the wall thickness, then Enter.",
        StepAnchor::None,
        Some(has_shell),
        StepAssist {
            label: "Shell it for me",
            run: assist_shell_the_box,
        },
        Some(TypeHint::Fixed("2")),
    ),
    plain_step(
        "Walls 2 mm thick, open at the top and down one side. That's Shell. Nice work!",
        StepAnchor::None,
        None,
    ),
];

// --- Derived parameter tutorial (#1676) ------------------------------------------------

/// The name the walkthrough gives the measurement it takes.
const DERIVED_NAME: &str = "width";

/// #1676/#1729: measure a sketch edge with the Dimension tool and record it as a parameter —
/// a value the model owns, which follows the geometry instead of being typed.
static DERIVED_PARAMETER_STEPS: &[Step] = &[
    plain_step_enter(
        "A derived parameter is measured off the model. Pick some geometry, name the \
         measurement, and the number follows the shape.",
        StepAnchor::None,
        None,
        keep_the_ground_plane,
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
        "Click the opposite corner. Leave it free \u{2014} this one gets measured, not set.",
        StepAnchor::World(rect_opposite_corner_guide),
        Some(has_rectangle_outline),
        StepAssist {
            label: "Draw it for me",
            run: assist_draw_free_derived_rect,
        },
        None,
    ),
    plain_step_enter(
        "Click the Dimension tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Dimension)),
        Some(dimension_tool_active),
        clear_selection_for_dimensioning,
    ),
    assisted_step(
        "Click the rectangle's bottom edge. The Context pane measures it.",
        StepAnchor::World(derived_edge_guide),
        Some(derive_source_ready),
        StepAssist {
            label: "Pick it for me",
            run: assist_pick_derived_edge,
        },
        None,
    ),
    assisted_step(
        "Type `width` for the name.",
        StepAnchor::Ui(UiAnchor::DeriveName),
        Some(derive_name_typed),
        StepAssist {
            label: "Name it for me",
            run: assist_name_derived_parameter,
        },
        Some(TypeHint::Fixed(DERIVED_NAME)),
    ),
    assisted_step(
        "Press Derive parameter.",
        StepAnchor::Ui(UiAnchor::DeriveButton),
        Some(has_derived_parameter),
        StepAssist {
            label: "Derive it for me",
            run: assist_derive_parameter,
        },
        None,
    ),
    plain_step(
        "There it is in the Parameters pane, greyed out \u{2014} you can't type over a \
         measurement. Move the edge and `width` follows it. Nice!",
        StepAnchor::None,
        None,
    ),
];

/// The tutorial's rectangle, drawn free so its sides are unconstrained — a side with a
/// length constraint already has its number and can't be measured into one (#1729).
fn assist_draw_free_derived_rect(app: &mut AppState) {
    if has_rectangle_outline(app) {
        return;
    }
    ensure_rect_sketch_for_tutorial(app);
    let Some(session) = app.sketch_session else { return };
    crate::construction::add_line_rectangle(
        &mut app.doc,
        session.sketch,
        DERIVED_RECT_MM.0,
        DERIVED_RECT_MM.1,
        DERIVED_RECT_MM.2,
        DERIVED_RECT_MM.3,
        [false; 4],
    );
    app.creating_rect = None;
}

/// Where the walkthrough's rectangle sits, and how big: (u, v, width, height) in mm.
const DERIVED_RECT_MM: (f32, f32, f32, f32) = (20.0, 20.0, 40.0, 40.0);

/// The bottom edge of that rectangle — the one the walkthrough measures.
fn derived_measured_line(app: &AppState) -> Option<crate::model::LineKey> {
    first_sketch_rect_lines(app).first().copied()
}

fn derived_edge_guide(app: &AppState) -> Option<glam::Vec3> {
    let line = derived_measured_line(app)?;
    let (a, b) = crate::face::line_world_endpoints(&app.doc, app.doc.lines.get(line)?)?;
    Some((a + b) * 0.5)
}

/// The pane can measure what is picked — the state its "Derive parameter" button reads.
fn derive_source_ready(app: &AppState) -> bool {
    has_derived_parameter(app)
        || crate::parameters::derived_source_from_selection(&app.doc, &app.scene_selection)
            .is_some()
}

fn derive_name_typed(app: &AppState) -> bool {
    has_derived_parameter(app)
        || app.dimension_param_name.trim().eq_ignore_ascii_case(DERIVED_NAME)
}

fn has_derived_parameter(app: &AppState) -> bool {
    app.doc.parameters.values().any(|p| p.source.is_some())
}

fn assist_pick_derived_edge(app: &mut AppState) {
    if derive_source_ready(app) {
        return;
    }
    assist_draw_free_derived_rect(app);
    let Some(line) = derived_measured_line(app) else { return };
    app.scene_selection.clear();
    app.scene_selection
        .insert(crate::hierarchy::SceneElement::Line(line));
}

fn assist_name_derived_parameter(app: &mut AppState) {
    assist_pick_derived_edge(app);
    if !has_derived_parameter(app) {
        app.dimension_param_name = DERIVED_NAME.to_string();
    }
}

fn assist_derive_parameter(app: &mut AppState) {
    if has_derived_parameter(app) {
        return;
    }
    assist_name_derived_parameter(app);
    let name = Some(app.dimension_param_name.trim().to_string()).filter(|n| !n.is_empty());
    app.apply(Action::DeriveParameterFromSelection { name });
}

// --- Curves tutorial (#1677) -----------------------------------------------------------

/// A four-cornered outline whose top two corners the walkthrough bends into curves.
const CURVE_UV: [(f32, f32); 4] = [
    (10.0, 10.0),
    (60.0, 10.0),
    (60.0, 50.0),
    (10.0, 50.0),
];

fn curve_vertex_guide(app: &AppState, nth: usize) -> Option<glam::Vec3> {
    let (u, v) = CURVE_UV[nth % CURVE_UV.len()];
    ground_local(app, u, v)
}

fn curve_vertex_0(app: &AppState) -> Option<glam::Vec3> {
    curve_vertex_guide(app, 0)
}

fn curve_vertex_1(app: &AppState) -> Option<glam::Vec3> {
    curve_vertex_guide(app, 1)
}

fn curve_vertex_2(app: &AppState) -> Option<glam::Vec3> {
    curve_vertex_guide(app, 2)
}

fn curve_vertex_3(app: &AppState) -> Option<glam::Vec3> {
    curve_vertex_guide(app, 3)
}

fn curved_line_count(app: &AppState) -> usize {
    app.doc
        .lines
        .values()
        .filter(|l| !l.construction && l.is_curved())
        .count()
}

/// Curve mode is armed, or the curves it makes are already drawn.
fn curve_mode_on(app: &AppState) -> bool {
    app.draw_curve_mode || curved_line_count(app) > 0
}

fn has_one_curve(app: &AppState) -> bool {
    curved_line_count(app) >= 1
}

fn has_two_curves(app: &AppState) -> bool {
    curved_line_count(app) >= 2
}

/// Draw the outline, then bend the two corners that Curve mode would have rounded.
fn assist_draw_curved_outline(app: &mut AppState) {
    if has_closed_polygon(app) && has_two_curves(app) {
        return;
    }
    if !has_closed_polygon(app) {
        ensure_poly_sketch_open(app);
        let Some(session) = app.sketch_session else {
            return;
        };
        // Close what's already on the page (#1734). Stamping the whole outline regardless
        // dropped a fresh four-sided box on top of the sides the user had just curved.
        if close_drawn_outline(app, session.sketch).is_none() {
            crate::construction::add_line_polygon(&mut app.doc, session.sketch, &CURVE_UV);
        }
        app.refresh_document_health();
    }
    let lines = first_sketch_rect_lines(app);
    // Corners 1 and 2 of the outline — the far side, where the two curves land.
    for nth in [1usize, 2] {
        if curved_line_count(app) > nth - 1 {
            continue;
        }
        let Some(&line) = lines.get(nth) else {
            continue;
        };
        app.apply(Action::ConvertVertexToBezier {
            point: crate::model::ConstraintPoint::LineEndpoint {
                line,
                end: crate::model::LineEnd::End,
            },
        });
    }
}

/// Join the open chain of lines in `sketch` back to where it started, the way a last click on
/// the first point does (#1734). `None` when there is nothing to close -- no lines, or a chain
/// whose ends are already together.
fn close_drawn_outline(app: &mut AppState, sketch: crate::model::SketchId) -> Option<()> {
    use crate::model::{Constraint, ConstraintEntity, ConstraintKind, LineEnd, ShapeKind};
    let drawn: Vec<crate::model::LineKey> = app
        .doc
        .lines
        .iter()
        .filter(|(_, l)| l.sketch == sketch && !l.construction)
        .map(|(k, _)| k)
        .collect();
    let (&first, &last) = (drawn.first()?, drawn.last()?);
    let start = app.doc.lines.get(first)?;
    let (u0, v0) = (start.x0, start.y0);
    let end = app.doc.lines.get(last)?;
    let (u1, v1) = (end.x1, end.y1);
    if (u1 - u0).hypot(v1 - v0) < 1e-3 {
        return None;
    }
    let closing = app
        .doc
        .lines
        .insert(crate::model::Line::from_local_endpoints(sketch, u1, v1, u0, v0));
    app.doc.shape_order.push(ShapeKind::Line);
    for (a, a_end, b, b_end) in [
        (last, LineEnd::End, closing, LineEnd::Start),
        (closing, LineEnd::End, first, LineEnd::Start),
    ] {
        app.doc.constraints.insert(Constraint {
            sketch,
            kind: ConstraintKind::Coincident {
                a: ConstraintEntity::Point(crate::model::ConstraintPoint::LineEndpoint {
                    line: a,
                    end: a_end,
                }),
                b: ConstraintEntity::Point(crate::model::ConstraintPoint::LineEndpoint {
                    line: b,
                    end: b_end,
                }),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
        });
        app.doc.shape_order.push(ShapeKind::Constraint);
    }
    Some(())
}

fn assist_curve_mode_on(app: &mut AppState) {
    if curve_mode_on(app) {
        return;
    }
    app.apply(Action::ApplyCurveMode { curve_mode: true });
}

fn assist_extrude_curved_outline(app: &mut AppState) {
    if has_extrusion(app) {
        return;
    }
    assist_draw_curved_outline(app);
    let Some(sketch) = app.doc.lines.values().find(|l| !l.construction).map(|l| l.sketch) else {
        return;
    };
    let Some(lines) = crate::polygon::closed_line_loops(&app.doc, sketch)
        .into_iter()
        .max_by_key(|l| l.len())
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

/// #1677: draw an outline whose far side bends, using the Line tool's Curve mode, then
/// extrude it like any other profile.
static CURVES_STEPS: &[Step] = &[
    plain_step_enter(
        "Curves come from the Line tool: tick Curve and the next point arrives smooth \
         instead of sharp.",
        StepAnchor::None,
        None,
        keep_the_ground_plane,
    ),
    plain_step(
        "Click the Line tool \u{2014} glowing button, or `L`.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Line)),
        Some(line_tool_active),
    ),
    plain_step_enter(
        "Click a first point on the ground.",
        StepAnchor::World(curve_vertex_0),
        Some(first_poly_vertex_placed),
        ensure_line_sketch_for_tutorial,
    ),
    plain_step(
        "Click a second point. That segment is straight.",
        StepAnchor::World(curve_vertex_1),
        Some(poly_has_one_side),
    ),
    assisted_step(
        "Now tick `Curve` in the Context pane \u{2014} or press `Ctrl`/`Cmd` + `B`.",
        StepAnchor::Ui(UiAnchor::CheckboxRow("Curve")),
        Some(curve_mode_on),
        StepAssist {
            label: "Turn it on for me",
            run: assist_curve_mode_on,
        },
        None,
    ),
    assisted_step(
        "Click the next point. It bends in instead of turning a corner.",
        StepAnchor::World(curve_vertex_2),
        Some(has_one_curve),
        StepAssist {
            label: "Draw it for me",
            run: assist_draw_curved_outline,
        },
        None,
    ),
    assisted_step(
        "Click another point \u{2014} another curve.",
        StepAnchor::World(curve_vertex_3),
        Some(has_two_curves),
        StepAssist {
            label: "Draw it for me",
            run: assist_draw_curved_outline,
        },
        None,
    ),
    assisted_step(
        "Click back on the first point to close the outline.",
        StepAnchor::World(curve_vertex_0),
        Some(has_closed_polygon),
        StepAssist {
            label: "Close it for me",
            run: assist_draw_curved_outline,
        },
        None,
    ),
    plain_step(
        "Click the Extrude tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Extrude)),
        Some(extrude_tool_active),
    ),
    plain_step(
        "Click inside the outline.",
        StepAnchor::World(rectangle_face_guide),
        Some(extrude_face_picked),
    ),
    assisted_step(
        "Type `10`, then Enter. Curved sides extrude like straight ones.",
        StepAnchor::Ui(UiAnchor::ExtrudeDistance),
        Some(has_extrusion),
        StepAssist {
            label: "Extrude it for me",
            run: assist_extrude_curved_outline,
        },
        Some(TypeHint::Fixed("10")),
    ),
    plain_step(
        "Reopen the sketch any time and drag a curve's round handles to reshape it. Nice!",
        StepAnchor::None,
        None,
    ),
];

// --- Slice tutorial (#1678) ------------------------------------------------------------

fn slice_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Slice
}

fn has_slice(app: &AppState) -> bool {
    !app.doc.slice_ops.is_empty()
}

/// The block's top face — where the cutting line is drawn.
fn slice_top_face(app: &AppState) -> Option<crate::model::FaceId> {
    let prim = app
        .doc
        .primitives
        .iter()
        .find(|(_, p)| p.kind == crate::model::PrimitiveKind::Cuboid)
        .map(|(k, _)| k)?;
    Some(crate::model::FaceId::PrimitiveFace {
        primitive: prim,
        face: crate::model::PrimitiveFace::CuboidTop,
    })
}

fn sketch_on_block_top(app: &AppState) -> bool {
    let Some(face) = slice_top_face(app) else {
        return false;
    };
    app.doc.sketches.values().any(|s| s.face == face)
}

/// The slanted line's two ends, in the top-face sketch's local millimetres: a diagonal
/// run from one corner of the top to the opposite one.
fn slice_line_local(app: &AppState) -> Option<[(f32, f32); 2]> {
    let sketch = app
        .doc
        .sketches
        .iter()
        .find(|(_, s)| Some(s.face.clone()) == slice_top_face(app))
        .map(|(k, _)| k)?;
    let frame = crate::face::sketch_geometry_frame(&app.doc, sketch)?;
    let prim = app
        .doc
        .primitives
        .values()
        .find(|p| p.kind == crate::model::PrimitiveKind::Cuboid)?;
    let r = crate::primitives::resolve(&app.doc, prim)?;
    let base = r.cuboid_base();
    let lift = r.normal * r.height;
    let a = crate::face::world_to_local(&frame, base[0] + lift);
    let c = crate::face::world_to_local(&frame, base[2] + lift);
    // Push each end a little past the corner so the laser clears the block.
    let (du, dv) = (c.0 - a.0, c.1 - a.1);
    Some([
        (a.0 - du * 0.15, a.1 - dv * 0.15),
        (c.0 + du * 0.15, c.1 + dv * 0.15),
    ])
}

/// A world point at each end of the slanted line, for the two drawing orbs.
fn slice_line_guide(app: &AppState, end: usize) -> Option<glam::Vec3> {
    let sketch = app.sketch_session.map(|s| s.sketch).or_else(|| {
        app.doc
            .sketches
            .iter()
            .find(|(_, s)| Some(s.face.clone()) == slice_top_face(app))
            .map(|(k, _)| k)
    })?;
    let frame = crate::face::sketch_geometry_frame(&app.doc, sketch)?;
    let ends = slice_line_local(app)?;
    let (u, v) = ends[end];
    Some(crate::face::local_to_world(&frame, u, v))
}

/// Where Slice's *body* click should land (#1736): the middle of one half of the block's
/// top, well clear of the diagonal cutter line running corner to corner -- the block's
/// centre sits right on that line, so the orb read as pointing at the line, not the block.
fn slice_block_guide(app: &AppState) -> Option<glam::Vec3> {
    let prim = app
        .doc
        .primitives
        .values()
        .find(|p| p.kind == crate::model::PrimitiveKind::Cuboid)?;
    let r = crate::primitives::resolve(&app.doc, prim)?;
    let base = r.cuboid_base();
    let lift = r.normal * r.height;
    // Centroid of the triangle the diagonal (corner 0 -> corner 2) cuts off.
    Some((base[0] + base[1] + base[2]) / 3.0 + lift)
}

fn slice_line_start_guide(app: &AppState) -> Option<glam::Vec3> {
    slice_line_guide(app, 0)
}

fn slice_line_end_guide(app: &AppState) -> Option<glam::Vec3> {
    slice_line_guide(app, 1)
}

/// The middle of the drawn line — where Slice's cutter click should land, rather than the
/// endpoint that hangs off the block (#1681).
fn slice_line_mid_guide(app: &AppState) -> Option<glam::Vec3> {
    let a = slice_line_guide(app, 0)?;
    let b = slice_line_guide(app, 1)?;
    Some((a + b) * 0.5)
}

/// The line drawn across the block's top — the one Slice uses as its cutter.
fn slice_cutter_line(app: &AppState) -> Option<crate::model::LineKey> {
    let face = slice_top_face(app)?;
    let sketch = app
        .doc
        .sketches
        .iter()
        .find(|(_, s)| s.face == face)
        .map(|(k, _)| k)?;
    app.doc
        .lines
        .iter()
        .find(|(_, l)| l.sketch == sketch && !l.construction)
        .map(|(k, _)| k)
}

fn slice_line_drawn(app: &AppState) -> bool {
    slice_cutter_line(app).is_some()
}

fn slice_body_picked(app: &AppState) -> bool {
    has_slice(app)
        || app
            .creating_slice
            .as_ref()
            .is_some_and(|c| !c.targets.is_empty())
}

fn slice_cutter_picked(app: &AppState) -> bool {
    has_slice(app)
        || app
            .creating_slice
            .as_ref()
            .is_some_and(|c| !c.cutters.is_empty())
}

fn assist_sketch_on_block_top(app: &mut AppState) {
    if sketch_on_block_top(app) {
        return;
    }
    assist_place_cuboid(app);
    let Some(face) = slice_top_face(app) else {
        return;
    };
    app.apply(Action::BeginSketch { face, viewport: None });
}

fn assist_draw_slice_line(app: &mut AppState) {
    if slice_line_drawn(app) {
        return;
    }
    assist_sketch_on_block_top(app);
    let Some([(x0, y0), (x1, y1)]) = slice_line_local(app) else {
        return;
    };
    app.apply(Action::CreateLineSegment {
        x0,
        y0,
        x1,
        y1,
        bezier: None,
        dimension: None,
    });
}

fn assist_slice_the_block(app: &mut AppState) {
    if has_slice(app) {
        return;
    }
    assist_draw_slice_line(app);
    let Some(line) = slice_cutter_line(app) else {
        return;
    };
    let Some(body) = live_body_for_primitive(app, crate::model::PrimitiveKind::Cuboid) else {
        return;
    };
    if app.sketch_session.is_some() {
        app.apply(Action::ExitSketch);
    }
    app.apply(Action::CreateSliceOperation {
        targets: vec![body],
        cutters: vec![crate::model::SliceCutter::Line { line }],
        extend_infinite: true,
    });
}

/// #1678: draw a slanted line across a block's top face and use it as a laser cutter.
static SLICE_STEPS: &[Step] = &[
    plain_step_enter(
        "Slice cuts a solid into pieces. A sketch line works like a laser \u{2014} it cuts \
         straight down through whatever is under it.",
        StepAnchor::None,
        None,
        keep_the_ground_plane,
    ),
    plain_step(
        "Grab the Shape tool \u{2014} the glowing button, or press `B`.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Shape)),
        Some(shape_tool_active_or_has_cuboid),
    ),
    plain_step(
        "Click Cuboid in the Context pane (or press `B`).",
        StepAnchor::Ui(UiAnchor::ShapeKind(crate::model::PrimitiveKind::Cuboid)),
        Some(cuboid_kind_ready),
    ),
    plain_step(
        "Click a ground corner to anchor the block.",
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
    assisted_step(
        "Click the top of the block to draw on it.",
        StepAnchor::World(cuboid_top_guide),
        Some(sketch_on_block_top),
        StepAssist {
            label: "Open it for me",
            run: assist_sketch_on_block_top,
        },
        None,
    ),
    plain_step(
        "Click the Line tool \u{2014} glowing button, or `L`.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Line)),
        Some(line_tool_active),
    ),
    plain_step(
        "Click just past one corner of the top.",
        StepAnchor::World(slice_line_start_guide),
        Some(first_poly_vertex_placed),
    ),
    assisted_step(
        "Click just past the opposite corner \u{2014} a slanted line right across the block.",
        StepAnchor::World(slice_line_end_guide),
        Some(slice_line_drawn),
        StepAssist {
            label: "Draw it for me",
            run: assist_draw_slice_line,
        },
        None,
    ),
    keyed_assist_step(
        "Press `Esc` to leave the sketch \u{2014} Slice cuts solids.",
        sketch_exited,
        "Esc",
        "to leave the sketch",
        StepAssist {
            label: "Leave for me",
            run: assist_exit_sketch,
        },
    ),
    plain_step(
        "Click the Slice tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Slice)),
        Some(slice_tool_active),
    ),
    plain_step(
        "Click the block \u{2014} that's what gets cut.",
        StepAnchor::World(slice_block_guide),
        Some(slice_body_picked),
    ),
    plain_step(
        "Click the slanted line \u{2014} that's the cutter.",
        StepAnchor::World(slice_line_mid_guide),
        Some(slice_cutter_picked),
    ),
    assisted_step(
        "Press Enter. The block falls into two wedges.",
        StepAnchor::None,
        Some(has_slice),
        StepAssist {
            label: "Slice it for me",
            run: assist_slice_the_block,
        },
        None,
    ),
    plain_step(
        "Each piece is its own body now. Click the Select tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Select)),
        Some(select_tool_active),
    ),
    plain_step(
        "Click one of the wedges to select it.",
        StepAnchor::World(slice_block_guide),
        Some(a_body_is_selected),
    ),
    keyed_assist_step(
        "Press `V`. The piece vanishes \u{2014} press it again to bring it back. Nice!",
        a_body_is_hidden,
        "V",
        "to hide the selected piece",
        StepAssist {
            label: "Hide it for me",
            run: assist_hide_a_slice_piece,
        },
    ),
];

fn select_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Select
}

/// A whole body is in the selection -- what V acts on (#1739).
fn a_body_is_selected(app: &AppState) -> bool {
    a_body_is_hidden(app)
        || app
            .scene_selection
            .iter()
            .any(|e| matches!(e, crate::hierarchy::SceneElement::Body(_)))
}

fn a_body_is_hidden(app: &AppState) -> bool {
    app.doc
        .bodies
        .keys()
        .any(|b| !app.element_visibility.is_visible(crate::hierarchy::SceneElement::Body(b)))
}

/// Select one of the pieces and hide it, exactly as the two steps ask by hand.
fn assist_hide_a_slice_piece(app: &mut AppState) {
    if a_body_is_hidden(app) {
        return;
    }
    if app.tool != Tool::Select {
        app.apply(Action::SetTool(Tool::Select));
    }
    let Some(body) = app.doc.bodies.keys().next() else {
        return;
    };
    app.scene_selection.clear();
    app.scene_selection
        .insert(crate::hierarchy::SceneElement::Body(body));
    app.apply(Action::ToggleSelectionVisibility);
}

// --- Repeat tutorial (#1679) -----------------------------------------------------------

/// The numbers the walkthrough types into the Repeat tool's interlinked fields.
const REPEAT_COUNT: &str = "5";
const REPEAT_GAP: &str = "40";
const REPEAT_DISTANCE: &str = "300";

fn repeat_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Repeat
}

fn has_repeat(app: &AppState) -> bool {
    !app.doc.repeat_ops.is_empty()
}

fn creating_repeat(app: &AppState) -> Option<&crate::actions::CreatingRepeat> {
    app.creating_repeat.as_ref()
}

fn repeat_body_picked(app: &AppState) -> bool {
    has_repeat(app) || creating_repeat(app).is_some_and(|c| !c.targets.is_empty())
}

fn repeat_axis_picked(app: &AppState) -> bool {
    has_repeat(app) || creating_repeat(app).is_some_and(|c| c.axis.is_some())
}

fn repeat_count_typed(app: &AppState) -> bool {
    has_repeat(app) || creating_repeat(app).is_some_and(|c| expr_eq(&c.count, REPEAT_COUNT))
}

fn repeat_gap_typed(app: &AppState) -> bool {
    has_repeat(app) || creating_repeat(app).is_some_and(|c| expr_eq(&c.spacing, REPEAT_GAP))
}

fn repeat_gap_is_offset(app: &AppState) -> bool {
    has_repeat(app) || creating_repeat(app).is_some_and(|c| c.gap_is_offset)
}

/// Gap has taken the lock, so Count and Distance are the two the user sets.
fn repeat_gap_is_computed(app: &AppState) -> bool {
    has_repeat(app)
        || creating_repeat(app)
            .is_some_and(|c| c.var_mru[2] == crate::model::RepeatVar::Gap)
}

fn repeat_distance_typed(app: &AppState) -> bool {
    has_repeat(app) || creating_repeat(app).is_some_and(|c| expr_eq(&c.length, REPEAT_DISTANCE))
}

/// The Distance toggle has been worked: it now measures to the last copy's *start*.
fn repeat_distance_to_start(app: &AppState) -> bool {
    has_repeat(app) || creating_repeat(app).is_some_and(|c| !c.distance_is_end)
}

/// A point out along the global X axis, clear of the block.
fn repeat_axis_guide(app: &AppState) -> Option<glam::Vec3> {
    let _ = app;
    Some(glam::Vec3::new(120.0, 0.0, 0.0))
}

/// Arm the Repeat tool on the block, the way clicking it does.
fn ensure_repeat_in_progress(app: &mut AppState) {
    assist_place_cuboid(app);
    if app.tool != Tool::Repeat {
        app.apply(Action::SetTool(Tool::Repeat));
    }
    let Some(body) = live_body_for_primitive(app, crate::model::PrimitiveKind::Cuboid) else {
        return;
    };
    let cr = app
        .creating_repeat
        .get_or_insert_with(crate::actions::CreatingRepeat::default);
    if cr.targets.is_empty() {
        cr.targets.push(body);
    }
    if cr.axis.is_none() {
        cr.axis = Some(crate::model::RevolveAxis::X);
    }
}

/// Every one of these goes through the same action the Context pane's rows fire (#1693).
fn edit_repeat(app: &mut AppState, edit: crate::actions::RepeatToolEdit) {
    app.apply(Action::EditRepeatTool(edit));
}

fn assist_repeat_count(app: &mut AppState) {
    if repeat_count_typed(app) {
        return;
    }
    ensure_repeat_in_progress(app);
    edit_repeat(
        app,
        crate::actions::RepeatToolEdit {
            count: Some(REPEAT_COUNT.into()),
            ..Default::default()
        },
    );
}

fn assist_repeat_gap(app: &mut AppState) {
    if repeat_gap_typed(app) {
        return;
    }
    assist_repeat_count(app);
    edit_repeat(
        app,
        crate::actions::RepeatToolEdit {
            gap: Some(REPEAT_GAP.into()),
            ..Default::default()
        },
    );
}

fn assist_repeat_offset_toggle(app: &mut AppState) {
    if repeat_gap_is_offset(app) {
        return;
    }
    assist_repeat_gap(app);
    edit_repeat(
        app,
        crate::actions::RepeatToolEdit {
            gap_is_offset: Some(true),
            ..Default::default()
        },
    );
}

fn assist_repeat_lock_gap(app: &mut AppState) {
    if repeat_gap_is_computed(app) {
        return;
    }
    assist_repeat_offset_toggle(app);
    edit_repeat(
        app,
        crate::actions::RepeatToolEdit {
            computed: Some(crate::model::RepeatVar::Gap),
            ..Default::default()
        },
    );
}

fn assist_repeat_distance(app: &mut AppState) {
    if repeat_distance_typed(app) {
        return;
    }
    assist_repeat_lock_gap(app);
    // Typing Distance would hand it the lock; Gap keeps it, so Count + Distance drive.
    edit_repeat(
        app,
        crate::actions::RepeatToolEdit {
            distance: Some(REPEAT_DISTANCE.into()),
            computed: Some(crate::model::RepeatVar::Gap),
            ..Default::default()
        },
    );
}

fn assist_repeat_distance_toggle(app: &mut AppState) {
    if repeat_distance_to_start(app) {
        return;
    }
    assist_repeat_distance(app);
    edit_repeat(
        app,
        crate::actions::RepeatToolEdit {
            distance_is_end: Some(false),
            ..Default::default()
        },
    );
}

fn assist_commit_repeat(app: &mut AppState) {
    if has_repeat(app) {
        return;
    }
    assist_repeat_distance_toggle(app);
    app.apply(Action::CommitRepeat);
}

/// #1679: pattern a block along the X axis, working all three interlinked fields and both
/// of the Repeat tool's measure toggles.
static REPEAT_STEPS: &[Step] = &[
    plain_step_enter(
        "Repeat stamps copies of a body along an axis.",
        StepAnchor::None,
        None,
        keep_the_ground_plane,
    ),
    plain_step(
        "Grab the Shape tool \u{2014} the glowing button, or press `B`.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Shape)),
        Some(shape_tool_active_or_has_cuboid),
    ),
    plain_step(
        "Click Cuboid in the Context pane (or press `B`).",
        StepAnchor::Ui(UiAnchor::ShapeKind(crate::model::PrimitiveKind::Cuboid)),
        Some(cuboid_kind_ready),
    ),
    plain_step(
        "Click a ground corner to anchor the block.",
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
        "Click the Repeat tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Repeat)),
        Some(repeat_tool_active),
    ),
    plain_step(
        "Click the block \u{2014} that's what gets copied.",
        StepAnchor::World(cuboid_body_guide),
        Some(repeat_body_picked),
    ),
    plain_step(
        "Click the red X axis \u{2014} that's the direction the copies march.",
        StepAnchor::World(repeat_axis_guide),
        Some(repeat_axis_picked),
    ),
    assisted_step(
        "Type `5` in the Count field.",
        StepAnchor::Ui(UiAnchor::RepeatVar(crate::model::RepeatVar::Count)),
        Some(repeat_count_typed),
        StepAssist {
            label: "Type it for me",
            run: assist_repeat_count,
        },
        Some(TypeHint::Fixed(REPEAT_COUNT)),
    ),
    assisted_step(
        "Type `40` in the Gap field \u{2014} the clear space between one copy and the next.",
        StepAnchor::Ui(UiAnchor::RepeatVar(crate::model::RepeatVar::Gap)),
        Some(repeat_gap_typed),
        StepAssist {
            label: "Type it for me",
            run: assist_repeat_gap,
        },
        Some(TypeHint::Fixed(REPEAT_GAP)),
    ),
    assisted_step(
        "Click the Gap icon. It flips to Offset: the same 40 mm now measures start to \
         start, so the copies close up.",
        StepAnchor::Ui(UiAnchor::RepeatVarIcon(crate::model::RepeatVar::Gap)),
        Some(repeat_gap_is_offset),
        StepAssist {
            label: "Flip it for me",
            run: assist_repeat_offset_toggle,
        },
        None,
    ),
    assisted_step(
        "Distance is greyed out because it's the computed one. Click Offset's grey lock to \
         compute the Offset instead.",
        StepAnchor::Ui(UiAnchor::RepeatVarLock(crate::model::RepeatVar::Gap)),
        Some(repeat_gap_is_computed),
        StepAssist {
            label: "Move the lock for me",
            run: assist_repeat_lock_gap,
        },
        None,
    ),
    assisted_step(
        "Type `300` in Distance. The spacing works itself out to fill it.",
        StepAnchor::Ui(UiAnchor::RepeatVar(crate::model::RepeatVar::Distance)),
        Some(repeat_distance_typed),
        StepAssist {
            label: "Type it for me",
            run: assist_repeat_distance,
        },
        Some(TypeHint::Fixed(REPEAT_DISTANCE)),
    ),
    assisted_step(
        "Click the Distance icon. It flips between measuring to the last copy's far end \
         and to its start.",
        StepAnchor::Ui(UiAnchor::RepeatVarIcon(crate::model::RepeatVar::Distance)),
        Some(repeat_distance_to_start),
        StepAssist {
            label: "Flip it for me",
            run: assist_repeat_distance_toggle,
        },
        None,
    ),
    assisted_step(
        "Press Enter. Five blocks, evenly spread over 300 mm.",
        StepAnchor::None,
        Some(has_repeat),
        StepAssist {
            label: "Repeat it for me",
            run: assist_commit_repeat,
        },
        None,
    ),
    plain_step(
        "Two numbers in, the third out \u{2014} and the little icons say how each one is \
         measured. Nice work!",
        StepAnchor::None,
        None,
    ),
];

// --- In-sketch Mirror tutorial (#1680) -------------------------------------------------

/// Where the source circle sits, off to one side of the sketch's vertical axis.
const MIRROR_CENTRE_MM: (f32, f32) = (30.0, 25.0);
const MIRROR_RADIUS_MM: f32 = 10.0;

fn mirror_tool_active(app: &AppState) -> bool {
    app.tool == Tool::Mirror
}

fn has_sketch_mirror(app: &AppState) -> bool {
    !app.doc.sketch_mirror_ops.is_empty()
}

fn mirror_circle_drawn(app: &AppState) -> bool {
    !app.doc.circles.is_empty()
}

fn mirror_circle_started(app: &AppState) -> bool {
    app.creating_circle.is_some() || mirror_circle_drawn(app)
}

fn mirror_shape_picked(app: &AppState) -> bool {
    has_sketch_mirror(app)
        || app
            .creating_sketch_mirror
            .as_ref()
            .is_some_and(|c| c.has_targets())
}

fn mirror_line_picked(app: &AppState) -> bool {
    has_sketch_mirror(app)
        || app
            .creating_sketch_mirror
            .as_ref()
            .is_some_and(|c| c.line.is_some())
}

/// The circle the walkthrough draws — the one the mirror reflects.
fn mirror_source_circle(app: &AppState) -> Option<crate::model::CircleKey> {
    app.doc.circles.keys().next()
}

fn mirror_centre_guide(app: &AppState) -> Option<glam::Vec3> {
    ground_local(app, MIRROR_CENTRE_MM.0, MIRROR_CENTRE_MM.1)
}

/// A point up the green Y axis, clear of the circle — the mirror line.
fn mirror_axis_guide(app: &AppState) -> Option<glam::Vec3> {
    let _ = app;
    Some(glam::Vec3::new(0.0, 80.0, 0.0))
}

/// Centre of the nth circle in the sketch, for the two extrude picks.
fn mirror_circle_guide(app: &AppState, nth: usize) -> Option<glam::Vec3> {
    let c = app.doc.circles.keys().nth(nth).map(|k| &app.doc.circles[k])?;
    ground_local(app, c.cx, c.cy)
}

fn mirror_first_circle_guide(app: &AppState) -> Option<glam::Vec3> {
    mirror_circle_guide(app, 0)
}

/// A point on the source circle's rim: the Mirror tool picks the circle itself, so the orb
/// belongs on the line, not in the middle of it (#1681).
fn mirror_circle_edge_guide(app: &AppState) -> Option<glam::Vec3> {
    let (cx, cy, r) = match mirror_source_circle(app).map(|k| &app.doc.circles[k]) {
        Some(c) => (c.cx, c.cy, c.r),
        None => (MIRROR_CENTRE_MM.0, MIRROR_CENTRE_MM.1, MIRROR_RADIUS_MM),
    };
    ground_local(app, cx + r, cy)
}

fn mirror_second_circle_guide(app: &AppState) -> Option<glam::Vec3> {
    mirror_circle_guide(app, 1).or_else(|| mirror_circle_guide(app, 0))
}

fn extrude_faces_picked(app: &AppState, want: usize) -> bool {
    has_extrusion(app)
        || app
            .creating_extrusion
            .as_ref()
            .is_some_and(|c| c.faces.len() >= want)
}

fn mirror_one_face_picked(app: &AppState) -> bool {
    extrude_faces_picked(app, 1)
}

fn mirror_both_faces_picked(app: &AppState) -> bool {
    extrude_faces_picked(app, 2)
}

fn assist_draw_mirror_circle(app: &mut AppState) {
    if mirror_circle_drawn(app) {
        return;
    }
    ensure_ground_sketch(app);
    app.apply(Action::CreateCircle {
        cx: MIRROR_CENTRE_MM.0,
        cy: MIRROR_CENTRE_MM.1,
        r: MIRROR_RADIUS_MM,
        diameter_expr: Some(format!("{}", MIRROR_RADIUS_MM * 2.0)),
    });
}

fn assist_mirror_the_circle(app: &mut AppState) {
    if has_sketch_mirror(app) {
        return;
    }
    assist_draw_mirror_circle(app);
    let Some(circle) = mirror_source_circle(app) else {
        return;
    };
    let Some(sketch) = app.doc.circles.get(circle).map(|c| c.sketch) else {
        return;
    };
    app.apply(Action::CreateSketchMirrorOperation {
        sketch,
        line: crate::model::SketchMirrorAxis::Y,
        line_targets: Vec::new(),
        circle_targets: vec![circle],
    });
}

fn assist_extrude_mirrored_pair(app: &mut AppState) {
    if has_extrusion(app) {
        return;
    }
    assist_mirror_the_circle(app);
    let circles: Vec<_> = app.doc.circles.keys().collect();
    let Some(&first) = circles.first() else {
        return;
    };
    let Some(sketch) = app.doc.circles.get(first).map(|c| c.sketch) else {
        return;
    };
    let faces: Vec<_> = circles
        .iter()
        .map(|&k| crate::model::ExtrudeFace::Circle(k))
        .collect();
    if app.sketch_session.is_some() {
        app.apply(Action::ExitSketch);
    }
    app.apply(Action::CreateExtrusion {
        sketch,
        faces,
        distance: 15.0,
        body: crate::actions::ExtrudeBodyChoice::New,
        target: None,
        expression: Some("15".into()),
        symmetric: false,
        taper: 0.0,
        taper_mode: crate::model::ExtrudeTaperMode::Distance,
        taper_expression: None,
    });
}

/// #1680: reflect a sketch circle across the Y axis, then extrude the pair.
static SKETCH_MIRROR_STEPS: &[Step] = &[
    plain_step_enter(
        "Mirror reflects sketch shapes across a line. Draw one side and the other comes \
         free \u{2014} and stays linked.",
        StepAnchor::None,
        None,
        keep_the_ground_plane,
    ),
    plain_step(
        "Click the Circle tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Circle)),
        Some(circle_tool_active),
    ),
    plain_step_enter(
        "Click the ground to one side of the green Y axis.",
        StepAnchor::World(mirror_centre_guide),
        Some(mirror_circle_started),
        ensure_ground_sketch,
    ),
    assisted_step(
        "Type `20` for the diameter, then Enter.",
        StepAnchor::None,
        Some(mirror_circle_drawn),
        StepAssist {
            label: "Draw it for me",
            run: assist_draw_mirror_circle,
        },
        Some(TypeHint::Fixed("20")),
    ),
    plain_step(
        "Click the Mirror tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Mirror)),
        Some(mirror_tool_active),
    ),
    // The Mirror tool arms its Mirror-line picker first, so the walkthrough asks in that
    // order too (#1744) -- pointing at the circle while the line picker was armed sent the
    // click somewhere the tutorial never noticed.
    plain_step(
        "Click the green Y axis \u{2014} that's the mirror line.",
        StepAnchor::World(mirror_axis_guide),
        Some(mirror_line_picked),
    ),
    plain_step(
        "Click the circle \u{2014} that's what gets reflected.",
        StepAnchor::World(mirror_circle_edge_guide),
        Some(mirror_shape_picked),
    ),
    assisted_step(
        "Press Enter. A matching circle lands on the far side.",
        StepAnchor::None,
        Some(has_sketch_mirror),
        StepAssist {
            label: "Mirror it for me",
            run: assist_mirror_the_circle,
        },
        None,
    ),
    plain_step(
        "Click the Extrude tool.",
        StepAnchor::Ui(UiAnchor::Tool(Tool::Extrude)),
        Some(extrude_tool_active),
    ),
    plain_step(
        "Click one circle.",
        StepAnchor::World(mirror_first_circle_guide),
        Some(mirror_one_face_picked),
    ),
    plain_step(
        "Click the other one too \u{2014} Extrude takes both.",
        StepAnchor::World(mirror_second_circle_guide),
        Some(mirror_both_faces_picked),
    ),
    assisted_step(
        "Type `15`, then Enter. Two matching posts.",
        StepAnchor::Ui(UiAnchor::ExtrudeDistance),
        Some(has_extrusion),
        StepAssist {
            label: "Extrude it for me",
            run: assist_extrude_mirrored_pair,
        },
        Some(TypeHint::Fixed("15")),
    ),
    plain_step(
        "The reflection stays tied to its source: change the original and the copy \
         follows. Nice!",
        StepAnchor::None,
        None,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{Action, Pane};

    /// #1646/#1647/#1649: the drawing walkthrough names the CAD menu, and each of its two
    /// Add-view rounds is two steps — pick the tool, then click the body in the Elements
    /// pane, which is where that tool actually takes its click.
    #[test]
    fn drawing_walkthrough_splits_picking_the_add_view_tool_from_placing_the_view() {
        let make = DRAWING_STEPS
            .iter()
            .find(|s| s.narration.contains("New Drawing"))
            .expect("a step makes the drawing");
        assert!(make.narration.contains("CAD menu"), "{}", make.narration);
        assert!(!make.narration.contains("Insert"), "{}", make.narration);

        let add_tool: Vec<usize> = DRAWING_STEPS
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::DrawingAdd)))
            })
            .map(|(i, _)| i)
            .collect();
        assert_eq!(add_tool.len(), 2, "the Add-view tool is picked twice, one step each");
        for i in add_tool {
            let place = &DRAWING_STEPS[i + 1];
            assert!(
                matches!(place.anchor, StepAnchor::Ui(UiAnchor::ElementsBody)),
                "the step after picking the tool should ring the Elements-pane body row, \
                 got {:?} for {:?}",
                place.anchor,
                place.narration
            );
            assert!(
                place.narration.contains("Elements pane"),
                "and say so: {:?}",
                place.narration
            );
        }
    }

    /// #1697/#1698/#1699/#1724/#1735: picking the Extrude tool arms an empty draft
    /// (#1499), which must NOT satisfy the "click the face" step — otherwise every
    /// extruding tutorial skips straight past it.
    #[test]
    fn arming_the_extrude_tool_does_not_count_as_picking_a_face() {
        let mut app = AppState::default();
        app.apply(Action::SetTool(Tool::Extrude));
        assert!(
            app.creating_extrusion.is_some(),
            "SetTool arms an empty extrude draft (#1499)"
        );
        assert!(
            !extrude_face_picked(&app),
            "an armed draft with no faces is not a picked face"
        );
    }

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
        assert_eq!(tutorial_index("constraints"), Some(6), "#1591: constraints is seventh");
        assert_eq!(tutorial_index("combine"), Some(7), "#1556: combine is eighth");
        assert_eq!(tutorial_index("raised_text"), Some(8), "#1557: raised text is ninth");
        assert_eq!(tutorial_index("drawing"), Some(9), "#1640: technical drawing is tenth");
        assert_eq!(tutorial_index("revolve"), Some(10), "#1672: revolve is eleventh");
        assert_eq!(tutorial_index("tilted_plane"), Some(11), "#1673: angled plane is twelfth");
        assert_eq!(tutorial_index("offset"), Some(12), "#1674: offset is thirteenth");
        assert_eq!(tutorial_index("shell"), Some(13), "#1675: shell is fourteenth");
        assert_eq!(
            tutorial_index("derived_parameter"),
            Some(14),
            "#1676: derived parameters is fifteenth"
        );
        assert_eq!(tutorial_index("curves"), Some(15), "#1677: curves is sixteenth");
        assert_eq!(tutorial_index("slice"), Some(16), "#1678: slice is seventeenth");
        assert_eq!(tutorial_index("repeat"), Some(17), "#1679: repeat is eighteenth");
        assert_eq!(
            tutorial_index("sketch_mirror"),
            Some(18),
            "#1680: sketch mirror is nineteenth"
        );
        assert_eq!(tutorial_index("bracket"), None, "#1334: build-a-bracket tutorial is gone");
        assert_eq!(tutorial_index("nope"), None);
        assert_eq!(TUTORIALS.last().unwrap().name, "sketch_mirror");
        assert_eq!(TUTORIALS.len(), 19, "pane lists every remaining walkthrough");
        for tut in TUTORIALS {
            assert_ne!(tut.name, "bracket");
            assert_ne!(tut.title, "Build an angle bracket");
        }
    }

    /// #1681: no step asks for two actions. A narration that says "click" twice, or joins
    /// two moves with "and then", is two steps wearing one coat.
    #[test]
    fn tutorial_steps_ask_for_one_action_each() {
        for tut in TUTORIALS {
            for step in tut.steps {
                for text in [Some(step.narration), step.phone_narration].into_iter().flatten() {
                    let n = text.to_ascii_lowercase();
                    // "then Enter" is the one exception: committing what was just typed is
                    // part of typing it, not a second target to find. "Then Tab" is not —
                    // moving to another field is its own step (#1681).
                    assert!(
                        !n.contains("then tab"),
                        "tutorial '{}' types and tabs in one step: {text}",
                        tut.name
                    );
                    let rest = n.replace("then enter", "").replace("then hit enter", "");
                    // " or " offers a second *way* to do the one action, not a second
                    // action — so each alternative is counted on its own.
                    for alternative in rest.split(" or ") {
                        let clicks = alternative.matches("click").count();
                        assert!(
                            clicks <= 1,
                            "tutorial '{}' asks for {clicks} clicks in one step: {text}",
                            tut.name
                        );
                    }
                    assert!(
                        !rest.contains(" and then "),
                        "tutorial '{}' chains two moves in one step: {text}",
                        tut.name
                    );
                }
            }
        }
    }

    /// #1681: every step that points somewhere in the world resolves an orb position by
    /// the time it is the live step — an orb that resolves to `None` points nowhere.
    #[test]
    fn tutorial_world_orbs_resolve_at_the_step_that_uses_them() {
        for tut in TUTORIALS {
            let mut app = AppState::default();
            app.apply(Action::StartTutorial {
                index: tutorial_index(tut.name).unwrap(),
            });
            let mut guard = 0;
            while app.tutorial.is_some() {
                guard += 1;
                assert!(guard < 60, "tutorial '{}' should finish", tut.name);
                let run = app.tutorial.unwrap();
                let step = &TUTORIALS[run.tutorial].steps[run.step];
                match step.anchor {
                    StepAnchor::World(point) => assert!(
                        point(&app).is_some(),
                        "tutorial '{}' step {} has no orb position: {}",
                        tut.name,
                        run.step,
                        step.narration
                    ),
                    StepAnchor::Guided(target) => assert!(
                        target(&app).is_some(),
                        "tutorial '{}' step {} has no orb target: {}",
                        tut.name,
                        run.step,
                        step.narration
                    ),
                    StepAnchor::Ui(_) | StepAnchor::None => {}
                }
                if step.assist.is_some() {
                    app.apply(Action::TutorialAssist);
                }
                if app.tutorial.is_some() && app.tutorial != Some(run) {
                    continue;
                }
                if app.tutorial.is_some() {
                    app.apply(Action::TutorialNext);
                }
            }
        }
    }

    /// #1681: a step that names a tool points its orb at that tool's button.
    #[test]
    fn tutorial_tool_steps_point_at_the_tool_they_name() {
        for tut in TUTORIALS {
            for step in tut.steps {
                let StepAnchor::Ui(UiAnchor::Tool(tool)) = step.anchor else {
                    continue;
                };
                // The button's own label, or the scripting name — the step has to call the
                // tool what the app calls it, or the orb points at a stranger.
                let label = crate::opsigs::tool_label(tool).to_ascii_lowercase();
                let script = crate::shortcuts::tool_script_name(tool).replace('_', " ");
                let n = step.narration.to_ascii_lowercase();
                // A two-word label reads by its distinctive last word on the toolbar
                // ("Drawing projection" is the Projection button), so that counts too.
                let short = label.rsplit(' ').next().unwrap_or(&label).to_string();
                assert!(
                    [label.clone(), short, script.clone(), script.replace(' ', "")]
                        .iter()
                        .any(|name| n.contains(name.as_str())),
                    "tutorial '{}' points at the {tool:?} button but says: {}",
                    tut.name,
                    step.narration
                );
            }
        }
    }

    /// #1681: a step that asks for a click points the orb somewhere. A "click this" with no
    /// anchor leaves the reader hunting.
    #[test]
    fn tutorial_click_steps_have_an_orb() {
        for tut in TUTORIALS {
            for step in tut.steps {
                let n = step.narration.to_ascii_lowercase();
                if !n.contains("click") && !n.contains("tap ") {
                    continue;
                }
                assert!(
                    !matches!(step.anchor, StepAnchor::None),
                    "tutorial '{}' asks for a click with no orb: {}",
                    tut.name,
                    step.narration
                );
            }
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
    ///
    /// An assist that satisfies its own step auto-advances (`AppState::apply` runs
    /// `advance_tutorial`), exactly as it does for a user who presses the button. Pressing
    /// Next on top of that would skip the *following* step — and its assist with it — so
    /// Next is only pressed when the assist left us where we were.
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
            if app.tutorial.is_some() && app.tutorial != Some(run) {
                continue;
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
    /// #1681: reaching Height is its own step, so the radius step never says "then Tab".
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
            !radius.narration.to_ascii_lowercase().contains("enter"),
            "radius step must not send the user to Enter — that commits the cylinder: {}",
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
        let cuboid = shapes
            .steps
            .iter()
            .find(|s| {
                let n = s.narration.to_ascii_lowercase();
                n.contains("cuboid")
                    && (n.contains("click") || n.contains("context"))
                    && !n.contains("anchor")
                    && !n.contains("corner")
                    && !n.contains("height")
                    && !n.contains("cycle")
            })
            .expect("cuboid kind-pick step");
        assert!(
            matches!(
                cuboid.anchor,
                StepAnchor::Ui(UiAnchor::ShapeKind(K::Cuboid))
            ),
            "cuboid kind should point at Context Cuboid button: {}",
            cuboid.narration
        );
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
            TUTORIALS[tutorial_index("constraints").unwrap()].title,
            "Constraints"
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
        for needle in ["orbit", "pan", "zoom", "bear", "home", "zoom to fit"] {
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
            nav.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::ZoomToFit))),
            "should point at the Zoom to Fit button"
        );
    }

    /// #1583: Zoom to Fit (toolbar button + Z) sits after wheel zoom, is hands-on,
    /// and auto-advances when the view is framed.
    #[test]
    fn navigate_tutorial_teaches_zoom_to_fit() {
        let s = nav_step("zoom to fit");
        assert!(
            matches!(s.anchor, StepAnchor::Ui(UiAnchor::ZoomToFit)),
            "should point at the Zoom to Fit button: {}",
            s.narration
        );
        assert!(s.done.is_some(), "should auto-advance when Zoom to Fit lands");
        assert!(s.assist.is_none(), "zoom-to-fit should have no assist: {}", s.narration);
        assert!(
            s.key_hint.is_some_and(|(k, _)| k.eq_ignore_ascii_case("z")),
            "should show a Z key hint: {}",
            s.narration
        );
        let n = s.narration.to_ascii_lowercase();
        assert!(
            n.contains("`z`") || n.contains("press `z`") || n.contains(" or press"),
            "should mention the Z shortcut: {}",
            s.narration
        );

        let wheel_i = nav_step_index("scroll the mouse wheel");
        let fit_i = nav_step_index("zoom to fit");
        let bear_i = nav_step_index("view cube");
        assert!(
            wheel_i < fit_i && fit_i < bear_i,
            "wheel zoom, then Zoom to Fit, then the bear (got {wheel_i}, {fit_i}, {bear_i})"
        );

        let mut app = AppState::default();
        seed_nav_cubes(&mut app);
        assert!(!zoomed_to_fit(&app));
        app.apply(Action::ZoomToFit);
        assert!(zoomed_to_fit(&app));
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

    /// #1550–#1554 / #1583: orbit / pan / zoom / Zoom to Fit / home have no "for me"
    /// shortcut. After orbit and after pan, a Next-only "Good job" step makes the next
    /// action obvious.
    #[test]
    fn navigate_tutorial_camera_steps_are_hands_on() {
        let nav = &TUTORIALS[tutorial_index("navigate").unwrap()];
        for needle in [
            "right-drag to orbit",
            "middle-drag",
            "scroll the mouse wheel",
            "zoom to fit",
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

    /// #1269 / #1550–#1554 / #1583: camera motion advances orbit / pan / zoom / home;
    /// Zoom to Fit advances on Z / the toolbar button; Next covers the good-job
    /// interstitials; the bear still has an assist.
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
            } else if n.contains("click zoom to fit") {
                assist_zoom_to_fit(&mut app);
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
        assert_eq!(numbered_title(8, "Raised text"), "9. Raised text");
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
    }

    /// #1640: walking the technical-drawing tutorial with its assists leaves the page the
    /// narration describes — a front view, a top and a side aligned to it, a shaded
    /// three-quarter view, and a dimension.
    #[test]
    fn the_drawing_tutorial_builds_the_page_it_describes() {
        let mut app = AppState::default();
        finish_tutorial_via_next(&mut app, "drawing");
        let drawing = app.doc.drawings.values().next().expect("the drawing");
        assert_eq!(drawing.views.len(), 4, "front + top + side + three-quarter");
        assert_eq!(drawing.views[0].orientation, crate::model::DrawingOrientation::Front);
        let aligned: Vec<_> = drawing
            .views
            .iter()
            .filter(|v| v.aligned_parent == Some(0))
            .map(|v| (v.orientation, v.aligned_dir))
            .collect();
        assert_eq!(aligned.len(), 2, "two views aligned to the front, got {aligned:?}");
        assert!(
            aligned.iter().any(|(o, _)| *o == crate::model::DrawingOrientation::Top),
            "one of them is the top view: {aligned:?}"
        );
        assert!(
            aligned.iter().any(|(o, _)| *o == crate::model::DrawingOrientation::Right),
            "and one the side view: {aligned:?}"
        );
        // Every aligned view shows its projection lines (#1642).
        assert!(drawing.views.iter().filter(|v| v.aligned_parent.is_some()).all(|v| v.align_lines));

        let angled = drawing
            .views
            .iter()
            .find(|v| matches!(v.orientation, crate::model::DrawingOrientation::Corner(_)))
            .expect("a three-quarter view");
        assert!(angled.aligned_parent.is_none(), "the angled view stands on its own");
        assert_eq!(
            angled.style,
            crate::model::DrawingViewStyle::Shaded,
            "an at-an-angle view reads as a solid, not wireframe"
        );
        assert!(
            drawing.views.iter().any(|v| !v.dimensioned_edges.is_empty()),
            "and something is dimensioned"
        );
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
            treated >= 5,
            "every edge of the top (four sides plus the cutoff) should be chamfered, got {treated}, status={}",
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

    /// #1569: cuboid-first walkthroughs must point at Cuboid in the Context pane
    /// when the last-used Shape kind isn't cuboid. Grabbing the Shape tool (already
    /// armed as a sphere) must not stall on the toolbar button.
    #[test]
    fn cuboid_start_tutorials_guide_back_to_cuboid_when_sphere_is_armed() {
        for name in ["shapes", "combine", "raised_text"] {
            let mut app = AppState::default();
            app.apply(Action::SetTool(Tool::Shape));
            app.apply(Action::SetShapeKind {
                kind: crate::model::PrimitiveKind::Sphere,
            });
            app.apply(Action::StartTutorial {
                index: tutorial_index(name).unwrap(),
            });
            assert_eq!(app.tool, Tool::Select, "{name}: start resets the tool");
            assert_eq!(
                app.shape_kind,
                crate::model::PrimitiveKind::Sphere,
                "{name}: last-used kind survives a new document"
            );
            app.apply(Action::TutorialNext); // past the intro
            let run = app.tutorial.expect("{name} running");
            let step = &TUTORIALS[run.tutorial].steps[run.step];
            assert!(
                step.narration.to_ascii_lowercase().contains("shape tool"),
                "{name}: should be on grab-shape: {}",
                step.narration
            );

            app.apply(Action::SetTool(Tool::Shape));
            let run = app.tutorial.expect("{name} still running");
            let step = &TUTORIALS[run.tutorial].steps[run.step];
            let n = step.narration.to_ascii_lowercase();
            assert!(
                n.contains("cuboid") && (n.contains("click") || n.contains("context")),
                "{name}: after grabbing Shape (sphere), should ask for Cuboid, got: {}",
                step.narration
            );
            assert!(
                matches!(
                    step.anchor,
                    StepAnchor::Ui(UiAnchor::ShapeKind(crate::model::PrimitiveKind::Cuboid))
                ),
                "{name}: orb should sit on the Cuboid button: {:?}",
                step.anchor
            );

            app.apply(Action::SetShapeKind {
                kind: crate::model::PrimitiveKind::Cuboid,
            });
            let run = app.tutorial.expect("{name} still running");
            let step = &TUTORIALS[run.tutorial].steps[run.step];
            let n = step.narration.to_ascii_lowercase();
            assert!(
                n.contains("corner") || n.contains("anchor"),
                "{name}: cuboid armed → place the cuboid, got: {}",
                step.narration
            );
        }
    }

    /// #1569: default cuboid kind skips the "click Cuboid" step.
    #[test]
    fn cuboid_start_tutorials_skip_cuboid_kind_when_already_cuboid() {
        for name in ["shapes", "combine", "raised_text"] {
            let mut app = AppState::default();
            app.apply(Action::StartTutorial {
                index: tutorial_index(name).unwrap(),
            });
            app.apply(Action::TutorialNext);
            app.apply(Action::SetTool(Tool::Shape));
            let run = app.tutorial.expect("{name} running");
            let step = &TUTORIALS[run.tutorial].steps[run.step];
            let n = step.narration.to_ascii_lowercase();
            assert!(
                n.contains("corner") || n.contains("anchor"),
                "{name}: default cuboid should skip the kind-pick, got: {}",
                step.narration
            );
        }
    }

    /// #1700: the walkthrough ends with the quad squared up -- a line-to-line Parallel
    /// on the top side, so the solver visibly swings it level with the bottom.
    #[test]
    fn constraints_tutorial_finishes_by_making_the_top_parallel_to_the_bottom() {
        let mut app = AppState::default();
        app.apply(Action::StartTutorial {
            index: tutorial_index("constraints").unwrap(),
        });
        // Run every assist in order; the last one is the Parallel finish.
        let steps = constraints_tut().steps;
        for step in steps {
            if let Some(assist) = &step.assist {
                (assist.run)(&mut app);
            }
        }
        assert!(
            has_line_parallel_constraint(&app),
            "the tutorial should leave a line-to-line Parallel constraint"
        );
        let lines = first_sketch_rect_lines(&app);
        let bottom = app.doc.lines.get(lines[0]).expect("bottom side");
        let top = app.doc.lines.get(lines[2]).expect("top side");
        let cross = (bottom.x1 - bottom.x0) * (top.y1 - top.y0)
            - (bottom.y1 - bottom.y0) * (top.x1 - top.x0);
        assert!(
            cross.abs() < 1e-2,
            "top and bottom should end up parallel (cross = {cross})"
        );
    }

    fn constraints_tut() -> &'static Tutorial {
        &TUTORIALS[tutorial_index("constraints").expect("constraints tutorial is registered")]
    }

    /// #1591: draw a polygon with the Line tool, then pin it with a few constraints.
    #[test]
    fn constraints_tutorial_is_registered_and_covers_a_drawn_polygon() {
        let tut = constraints_tut();
        assert_eq!(tut.name, "constraints");
        assert_eq!(tut.title, "Constraints");
        assert_eq!(
            tutorial_index("constraints"),
            tutorial_index("chamfer").map(|i| i + 1),
            "after the chamfer tutorial"
        );
        assert_eq!(
            tutorial_index("combine"),
            tutorial_index("constraints").map(|i| i + 1),
            "before the combine tutorial"
        );
        let joined: String = tut
            .steps
            .iter()
            .map(|s| s.narration.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains("polygon"),
            "should ask them to draw a polygon: {joined}"
        );
        assert!(
            joined.contains("constraint"),
            "should teach constraints: {joined}"
        );
        assert!(
            joined.contains("equal"),
            "should apply an equal constraint: {joined}"
        );
        assert!(
            joined.contains("parallel to x") || joined.contains("horizontal"),
            "should square a side to the X axis: {joined}"
        );
        assert!(
            joined.contains("parallel to y") || joined.contains("vertical"),
            "should square a side to the Y axis: {joined}"
        );
        assert!(
            tut.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Line)))),
            "should pick the Line tool so they draw the polygon"
        );
        assert!(
            tut.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Constraint)))),
            "should pick the Constraint tool"
        );
        assert!(
            tut.steps.iter().any(|s| matches!(
                s.anchor,
                StepAnchor::Ui(UiAnchor::ConstraintButton(_))
            )),
            "should point at a constraint button in the Context pane"
        );
    }

    /// #1591: assists draw a closed polygon and apply horizontal, vertical, and equal.
    #[test]
    fn constraints_tutorial_assists_draw_and_constrain_a_polygon() {
        let mut app = AppState::default();
        finish_tutorial_via_next(&mut app, "constraints");
        let sketch = app
            .doc
            .sketches
            .keys()
            .next()
            .expect("polygon sketch");
        let loops = crate::polygon::closed_line_loops(&app.doc, sketch);
        assert!(
            loops.iter().any(|l| l.len() >= 4),
            "should leave a closed four-sided polygon, loops={loops:?}, status={}",
            app.status
        );
        assert!(
            has_axis_parallel(&app, crate::model::SketchAxis::X),
            "a side should be parallel to X, status={}",
            app.status
        );
        assert!(
            has_axis_parallel(&app, crate::model::SketchAxis::Y),
            "a side should be parallel to Y, status={}",
            app.status
        );
        assert!(
            has_equal_constraint(&app),
            "two sides should be equal, status={}",
            app.status
        );
    }

    fn sketch_mirror_tut() -> &'static Tutorial {
        &TUTORIALS[tutorial_index("sketch_mirror").expect("sketch_mirror tutorial is registered")]
    }

    /// #1719: the groove circle straddles the square's outer edge, so a click inside it takes
    /// the **overlap** — a Boolean of circle and square. The step used to insist on a bare
    /// `Circle` and sat there with the profile plainly picked in the pane.
    #[test]
    fn the_groove_step_notices_a_click_inside_the_circle() {
        use crate::model::{BooleanOp, ExtrudeFace};
        let mut app = AppState::default();
        let overlap = ExtrudeFace::Boolean {
            op: BooleanOp::Intersection,
            a: Box::new(ExtrudeFace::Circle(crate::arena::Key::from_bits(0))),
            b: Box::new(ExtrudeFace::Polygon(Vec::new())),
        };
        app.creating_revolve = Some(crate::actions::CreatingRevolve {
            faces: vec![overlap],
            ..Default::default()
        });
        assert!(super::groove_profile_picked(&app));
    }

    /// #1722/#1725: every walkthrough but the first clears away the datum planes it does not
    /// draw on — they are big translucent slabs standing in front of the step's target. The
    /// first one keeps all three: that is where you meet them.
    #[test]
    fn tutorials_clear_the_datum_planes_they_do_not_use() {
        // Shapes stands its cylinder on the XZ wall (#1273); the angled-plane walkthrough
        // builds its own plane off an axis and wants nothing but the axes showing (#1722).
        let expected = |name: &str| match name {
            "cube" => 3,
            "shapes" => 2,
            "navigate" | "drawing" | "tilted_plane" => 0,
            _ => 1,
        };
        for tut in TUTORIALS.iter() {
            let mut app = AppState::default();
            app.apply(Action::StartTutorial {
                index: tutorial_index(tut.name).unwrap(),
            });
            assert_eq!(
                app.doc.construction_planes.len(),
                expected(tut.name),
                "{} starts with the wrong datum planes",
                tut.name
            );
        }
    }

    /// #1702: the bracket is named, so the pane row and every view label say "Bracket"
    /// rather than "Body 2".
    #[test]
    fn drawing_tutorial_names_its_body_bracket() {
        let mut app = AppState::default();
        super::seed_drawing_bracket(&mut app);
        // The combine leaves its two inputs behind as shadows; the live result is the last.
        let live = app.doc.bodies.keys().last().expect("a body");
        assert_eq!(
            app.doc.bodies[live].name.as_deref(),
            Some("Bracket"),
            "status={}",
            app.status
        );
    }

    /// #1704: the Aligned-view tool wants its base view picked before it can place anything,
    /// so the walkthrough asks for that click instead of leaving it to a lucky selection.
    #[test]
    fn drawing_tutorial_picks_the_base_view_before_aligning() {
        let tut = &TUTORIALS[tutorial_index("drawing").unwrap()];
        let index_of = |needle: &str| {
            tut.steps
                .iter()
                .position(|s| s.narration.contains(needle))
                .unwrap_or_else(|| panic!("no step saying {needle:?}"))
        };
        let tool = index_of("Click the Aligned view tool");
        let base = index_of("what the next two views");
        let place = index_of("Click above the front view");
        assert!(tool < base && base < place, "tool {tool}, base {base}, place {place}");
        assert!(
            matches!(
                tut.steps[base].anchor,
                StepAnchor::Ui(UiAnchor::DrawingViewEdge { view: 0 })
            ),
            "the base step rings the front view"
        );
    }

    /// #1709: the Dimension step rings a line to click, not the middle of the card.
    #[test]
    fn drawing_tutorial_dimension_step_rings_a_line() {
        let tut = &TUTORIALS[tutorial_index("drawing").unwrap()];
        let step = tut
            .steps
            .iter()
            .find(|s| s.narration.contains("to dimension it"))
            .expect("the Dimension step");
        assert!(matches!(
            step.anchor,
            StepAnchor::Ui(UiAnchor::DrawingViewEdge { view: 0 })
        ));
    }

    /// #1680: reflect a sketch circle across the Y axis, then extrude both.
    #[test]
    fn sketch_mirror_tutorial_is_registered_and_mirrors_across_an_axis() {
        let tut = sketch_mirror_tut();
        assert_eq!(tut.name, "sketch_mirror");
        assert_eq!(tut.title, "Mirror in a sketch");
        let joined: String = tut
            .steps
            .iter()
            .map(|s| s.narration.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("mirror"), "{joined}");
        assert!(joined.contains("axis"), "{joined}");
        for tool in [Tool::Circle, Tool::Mirror, Tool::Extrude] {
            assert!(
                tut.steps
                    .iter()
                    .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(t)) if t == tool)),
                "the walkthrough picks {tool:?}"
            );
        }
    }

    /// #1680: the assists leave a mirrored pair of circles, both extruded.
    #[test]
    fn sketch_mirror_tutorial_assists_reflect_a_circle_and_extrude_the_pair() {
        let mut app = AppState::default();
        finish_tutorial_via_next(&mut app, "sketch_mirror");
        assert_eq!(
            app.doc.sketch_mirror_ops.len(),
            1,
            "one mirror operation, status={}",
            app.status
        );
        assert_eq!(app.doc.circles.len(), 2, "the circle plus its reflection");
        let xs: Vec<f32> = app.doc.circles.values().map(|c| c.cx).collect();
        assert!(
            xs.iter().any(|x| *x > 0.0) && xs.iter().any(|x| *x < 0.0),
            "the pair straddles the axis, centres at {xs:?}"
        );
        // Disjoint profiles become one extrusion each, so both circles show up as faces.
        let faces: usize = app.doc.extrusions.values().map(|e| e.faces.len()).sum();
        assert_eq!(faces, 2, "both circles extrude, status={}", app.status);
    }

    /// #1744: the Mirror tool arms its Mirror-line picker first, so the walkthrough asks for
    /// the line first too -- it used to send the user at the circle while Mirror line was armed.
    #[test]
    fn sketch_mirror_tutorial_picks_the_mirror_line_before_the_shape() {
        let tut = sketch_mirror_tut();
        let index_of = |needle: &str| {
            tut.steps
                .iter()
                .position(|s| s.narration.contains(needle))
                .unwrap_or_else(|| panic!("no step saying {needle:?}"))
        };
        assert!(
            index_of("that's the mirror line") < index_of("that's what gets reflected"),
            "the mirror line is picked first"
        );
    }

    /// #1739: the Slice walkthrough ends by hiding one of the two pieces with V.
    #[test]
    fn slice_tutorial_ends_by_hiding_a_piece_with_v() {
        let tut = &TUTORIALS[tutorial_index("slice").unwrap()];
        assert!(
            tut.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Select)))),
            "it arms the Select tool for the hide"
        );
        assert!(
            tut.steps.iter().any(|s| s.key_hint == Some(("V", "to hide the selected piece"))),
            "and shows the V keycap"
        );
        let mut app = AppState::default();
        finish_tutorial_via_next(&mut app, "slice");
        assert!(
            app.doc.bodies.keys().any(|b| !app
                .element_visibility
                .is_visible(crate::hierarchy::SceneElement::Body(b))),
            "one piece ends up hidden, status={}",
            app.status
        );
    }

    /// #1740: the Repeat intro is one sentence -- how the three fields interlink is what the
    /// steps themselves teach.
    #[test]
    fn repeat_intro_is_one_sentence() {
        let intro = &repeat_tut().steps[0];
        assert_eq!(
            intro.narration,
            "Repeat stamps copies of a body along an axis."
        );
    }

    fn repeat_tut() -> &'static Tutorial {
        &TUTORIALS[tutorial_index("repeat").expect("repeat tutorial is registered")]
    }

    /// #1679: the Repeat walkthrough works all three interlinked fields and both toggles.
    #[test]
    fn repeat_tutorial_is_registered_and_covers_every_toggle() {
        use crate::model::RepeatVar;
        let tut = repeat_tut();
        assert_eq!(tut.name, "repeat");
        assert_eq!(tut.title, "Repeat");
        let joined: String = tut
            .steps
            .iter()
            .map(|s| s.narration.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        for word in ["count", "gap", "offset", "distance"] {
            assert!(joined.contains(word), "the walkthrough names {word}: {joined}");
        }
        for var in [RepeatVar::Count, RepeatVar::Gap, RepeatVar::Distance] {
            assert!(
                tut.steps
                    .iter()
                    .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::RepeatVar(v)) if v == var)),
                "a step points at the {var:?} row"
            );
        }
    }

    /// #1679: the assists leave a committed repeat whose toggles were all exercised — the
    /// gap reads start-to-start, the distance measures to the far end, and the gap is the
    /// value the app computes.
    #[test]
    fn repeat_tutorial_assists_pattern_a_block_along_an_axis() {
        let mut app = AppState::default();
        finish_tutorial_via_next(&mut app, "repeat");
        assert_eq!(
            app.doc.repeat_ops.len(),
            1,
            "one repeat operation, status={}",
            app.status
        );
        let op = app.doc.repeat_ops.values().next().unwrap();
        assert_eq!(op.axis, crate::model::RevolveAxis::X);
        assert_eq!(op.count, REPEAT_COUNT);
        assert_eq!(op.spacing, REPEAT_GAP, "the typed gap is kept even once computed");
        assert_eq!(op.length, REPEAT_DISTANCE);
        assert!(
            !op.outputs.is_empty(),
            "the pattern makes copies, status={}",
            app.status
        );
        // Both toggles were worked: Gap holds the lock (so Count + Distance drive the
        // pattern) and Distance measures to the last copy's *start*.
        assert_eq!(
            op.mode,
            crate::model::RepeatMode::CountFitCenters,
            "count + distance-to-start drive the pattern"
        );
        let (computed, _, distance_is_end) = op.mode.to_repeat_ui();
        assert_eq!(computed, crate::model::RepeatVar::Gap);
        assert!(!distance_is_end);
    }

    fn slice_tut() -> &'static Tutorial {
        &TUTORIALS[tutorial_index("slice").expect("slice tutorial is registered")]
    }

    /// #1678: cut a block in two along a slanted sketch line.
    #[test]
    fn slice_tutorial_is_registered_and_cuts_with_a_slanted_line() {
        let tut = slice_tut();
        assert_eq!(tut.name, "slice");
        assert_eq!(tut.title, "Slice");
        let joined: String = tut
            .steps
            .iter()
            .map(|s| s.narration.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("slant") || joined.contains("diagonal"), "{joined}");
        for tool in [Tool::Line, Tool::Slice] {
            assert!(
                tut.steps
                    .iter()
                    .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(t)) if t == tool)),
                "the walkthrough picks {tool:?}"
            );
        }
    }

    /// #1736: the block orb sits clear of the diagonal cutter line. The block's *centre*
    /// projects onto the middle of the top face, right where the line runs, so the orb read
    /// as pointing at the line rather than at the block.
    #[test]
    fn slice_tutorial_block_orb_stays_off_the_cutter_line() {
        let mut app = AppState::default();
        finish_tutorial_via_next(&mut app, "slice");
        // Measure in the top face's own plane: the orb's height above it is not what makes
        // it look on or off the line -- where it lands across the face is.
        let ends = slice_line_local(&app).expect("cutter line ends");
        let sketch = app
            .doc
            .sketches
            .iter()
            .find(|(_, sk)| Some(sk.face.clone()) == slice_top_face(&app))
            .map(|(k, _)| k)
            .expect("the top-face sketch");
        let frame = crate::face::sketch_geometry_frame(&app.doc, sketch).expect("frame");
        let flat = |p: glam::Vec3| {
            let (u, v) = crate::face::world_to_local(&frame, p);
            glam::Vec2::new(u, v)
        };
        let (a, b) = (
            glam::Vec2::new(ends[0].0, ends[0].1),
            glam::Vec2::new(ends[1].0, ends[1].1),
        );
        let dir = (b - a).normalize();
        let off_line = |p: glam::Vec2| {
            let d = p - a;
            (d - dir * d.dot(dir)).length()
        };
        assert!(
            off_line(flat(cuboid_body_guide(&app).expect("block centre"))) < 1.0,
            "the block's centre lands on the cutter line -- that is the bug"
        );
        assert!(
            off_line(flat(slice_block_guide(&app).expect("block orb"))) > 3.0,
            "the block orb should sit clear of the cutter line"
        );
    }

    /// #1738: Slice cuts solids, so the walkthrough leaves the sketch before picking the
    /// tool -- inside a sketch the Slice tool takes sketch entities and the block is unpickable.
    #[test]
    fn slice_tutorial_leaves_the_sketch_before_the_slice_tool() {
        let steps = slice_tut().steps;
        let slice_i = steps
            .iter()
            .position(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Slice))))
            .expect("Slice tool step");
        let exit_i = steps
            .iter()
            .position(|s| {
                let n = s.narration.to_ascii_lowercase();
                n.contains("esc") && n.contains("sketch")
            })
            .expect("a step that leaves the sketch");
        assert!(
            exit_i < slice_i,
            "leave the sketch (step {exit_i}) before picking Slice (step {slice_i})"
        );
    }

    /// #1678: the assists leave the block split into two fragments by a line cutter.
    #[test]
    fn slice_tutorial_assists_split_a_block_in_two() {
        let mut app = AppState::default();
        finish_tutorial_via_next(&mut app, "slice");
        assert_eq!(
            app.doc.slice_ops.len(),
            1,
            "one slice operation, status={}",
            app.status
        );
        let op = app.doc.slice_ops.values().next().unwrap();
        assert!(
            matches!(op.cutters.first(), Some(crate::model::SliceCutter::Line { .. })),
            "the cutter is the sketch line, got {:?}",
            op.cutters
        );
        assert!(
            op.outputs.len() >= 2,
            "the block falls into at least two pieces, got {}",
            op.outputs.len()
        );
    }

    fn curves_tut() -> &'static Tutorial {
        &TUTORIALS[tutorial_index("curves").expect("curves tutorial is registered")]
    }

    /// An open run of lines through `points`, joined end to start -- what a row of Line-tool
    /// clicks leaves behind, minus the closing side.
    fn add_line_chain(
        doc: &mut crate::model::Document,
        sketch: crate::model::SketchId,
        points: &[(f32, f32)],
    ) -> Vec<crate::model::LineKey> {
        use crate::model::{
            Constraint, ConstraintEntity, ConstraintKind, ConstraintPoint, Line, LineEnd,
            ShapeKind,
        };
        let mut idx = Vec::new();
        for pair in points.windows(2) {
            let ((u0, v0), (u1, v1)) = (pair[0], pair[1]);
            idx.push(doc.lines.insert(Line::from_local_endpoints(sketch, u0, v0, u1, v1)));
            doc.shape_order.push(ShapeKind::Line);
        }
        for pair in idx.windows(2) {
            doc.constraints.insert(Constraint {
                sketch,
                kind: ConstraintKind::Coincident {
                    a: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                        line: pair[0],
                        end: LineEnd::End,
                    }),
                    b: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                        line: pair[1],
                        end: LineEnd::Start,
                    }),
                },
                expression: String::new(),
                dim_offset: None,
                name: None,
            });
            doc.shape_order.push(ShapeKind::Constraint);
        }
        idx
    }

    /// #1733: a three-sided closed outline *is* a closed outline. The predicate wanted four
    /// lines, so a user who joined three curved sides back to the start went unnoticed.
    #[test]
    fn a_three_sided_loop_counts_as_a_closed_outline() {
        let mut app = AppState::default();
        ensure_ground_sketch(&mut app);
        let sketch = app.sketch_session.expect("a sketch").sketch;
        crate::construction::add_line_polygon(
            &mut app.doc,
            sketch,
            &[(10.0, 10.0), (60.0, 10.0), (10.0, 50.0)],
        );
        app.refresh_document_health();
        assert!(
            has_closed_polygon(&app),
            "a closed triangle should read as a closed outline"
        );
    }

    /// #1734: "Close it for me" joins up the outline already drawn -- it used to stamp a
    /// whole fresh four-sided box on top of the user's curved sides.
    #[test]
    fn closing_a_drawn_outline_adds_one_line_not_a_new_box() {
        let mut app = AppState::default();
        ensure_ground_sketch(&mut app);
        let sketch = app.sketch_session.expect("a sketch").sketch;
        // Two sides drawn by hand, chained end to start, left open.
        add_line_chain(
            &mut app.doc,
            sketch,
            &[(10.0, 10.0), (60.0, 10.0), (10.0, 50.0)],
        );
        app.refresh_document_health();
        let before = app.doc.lines.len();
        assert_eq!(before, 2, "two open sides to start");

        assist_draw_curved_outline(&mut app);
        assert_eq!(
            app.doc.lines.len(),
            before + 1,
            "closing adds exactly the one missing side, not a new outline"
        );
        assert!(has_closed_polygon(&app), "and the outline is closed");
    }

    /// #1677: draw a shape whose sides bend, using the Line tool's Curve mode.
    #[test]
    fn curves_tutorial_is_registered_and_turns_curve_mode_on() {
        let tut = curves_tut();
        assert_eq!(tut.name, "curves");
        assert_eq!(tut.title, "Curves");
        let joined: String = tut
            .steps
            .iter()
            .map(|s| s.narration.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("curve"), "{joined}");
        assert!(
            tut.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::CheckboxRow("Curve")))),
            "a step points at the Curve tick in the Context pane"
        );
        assert!(
            tut.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Line)))),
            "curves are drawn with the Line tool"
        );
    }

    /// #1677: the assists leave a closed outline with curved sides, extruded into a solid.
    #[test]
    fn curves_tutorial_assists_draw_a_curved_outline_and_extrude_it() {
        let mut app = AppState::default();
        finish_tutorial_via_next(&mut app, "curves");
        assert!(
            has_closed_polygon(&app),
            "the outline closes, status={}",
            app.status
        );
        let curved = app
            .doc
            .lines
            .values()
            .filter(|l| !l.construction && l.is_curved())
            .count();
        assert!(curved >= 2, "at least two sides are curves, got {curved}");
        assert!(has_extrusion(&app), "and it extrudes, status={}", app.status);
    }

    fn derived_tut() -> &'static Tutorial {
        &TUTORIALS[tutorial_index("derived_parameter")
            .expect("derived_parameter tutorial is registered")]
    }

    /// #1729: it teaches the real thing — measure geometry with the Dimension tool and press
    /// "Derive parameter". It used to teach expressions (one parameter naming another), which
    /// is a different feature entirely.
    #[test]
    fn derived_parameter_tutorial_measures_geometry_with_the_dimension_tool() {
        let tut = derived_tut();
        assert_eq!(tut.name, "derived_parameter");
        assert_eq!(tut.title, "Derived parameters");
        let joined: String = tut
            .steps
            .iter()
            .map(|s| s.narration)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("measured off the model"), "{joined}");
        assert!(joined.contains("Derive parameter"), "{joined}");
        assert!(
            !joined.contains('/'),
            "no expression arithmetic — that is the Parameters walkthrough's job: {joined}"
        );
        for anchor in [UiAnchor::DeriveName, UiAnchor::DeriveButton] {
            assert!(
                tut.steps.iter().any(|s| matches!(&s.anchor, StepAnchor::Ui(a) if *a == anchor)),
                "the walkthrough points at {anchor:?}"
            );
        }
        assert!(
            tut.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Dimension)))),
            "and arms the Dimension tool"
        );
    }

    /// #1729: the assists leave a real derived parameter — one the model measured, which the
    /// Parameters pane will not let you type over.
    #[test]
    fn derived_parameter_tutorial_assists_measure_an_edge() {
        let mut app = AppState::default();
        finish_tutorial_via_next(&mut app, "derived_parameter");
        let measured: Vec<_> = app
            .doc
            .parameters
            .values()
            .filter(|p| p.source.is_some())
            .collect();
        assert_eq!(measured.len(), 1, "one measured parameter, status={}", app.status);
        assert_eq!(measured[0].name, DERIVED_NAME);
        assert!(
            crate::parameters::parameter_value_is_readonly(measured[0]),
            "a measured value is read-only"
        );
        assert!(
            param_length_near(&app, DERIVED_NAME, DERIVED_RECT_MM.2),
            "and it measures the edge it was taken from, got {:?}",
            measured[0].expression
        );
    }

    fn shell_tut() -> &'static Tutorial {
        &TUTORIALS[tutorial_index("shell").expect("shell tutorial is registered")]
    }

    /// #1675/#1727: hollow a block into a tray — open at the top and down one side, both of
    /// which you can see without turning the model over.
    #[test]
    fn shell_tutorial_is_registered_and_opens_two_faces() {
        let tut = shell_tut();
        assert_eq!(tut.name, "shell");
        assert_eq!(tut.title, "Shell");
        let joined: String = tut
            .steps
            .iter()
            .map(|s| s.narration.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("top"), "{joined}");
        assert!(joined.contains("side facing you"), "{joined}");
        assert!(!joined.contains("bottom"), "no face you have to turn over for: {joined}");
        assert!(
            tut.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Shell)))),
            "the walkthrough picks the Shell tool"
        );
        assert!(
            !tut.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::ViewCube))),
            "and never has to turn the model over"
        );
    }

    /// #1675: the assists leave a shell with both caps open — a four-walled box.
    #[test]
    fn shell_tutorial_assists_hollow_a_four_sided_box() {
        let mut app = AppState::default();
        finish_tutorial_via_next(&mut app, "shell");
        assert_eq!(
            app.doc.shell_ops.len(),
            1,
            "one shell operation, status={}",
            app.status
        );
        let op = app.doc.shell_ops.values().next().unwrap();
        assert_eq!(op.open_faces.len(), 2, "top and bottom are both open");
        assert_eq!(op.thickness, SHELL_THICKNESS_MM.to_string());
        assert!(!op.outputs.is_empty(), "the shell makes a body");
    }

    fn offset_tut() -> &'static Tutorial {
        &TUTORIALS[tutorial_index("offset").expect("offset tutorial is registered")]
    }

    /// #1674: offset a circle to get a wall, then extrude the ring between them.
    #[test]
    fn offset_tutorial_is_registered_and_extrudes_the_ring() {
        let tut = offset_tut();
        assert_eq!(tut.name, "offset");
        assert_eq!(tut.title, "Offset");
        let joined: String = tut
            .steps
            .iter()
            .map(|s| s.narration.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("offset"), "{joined}");
        assert!(joined.contains("ring") || joined.contains("wall"), "{joined}");
        for tool in [Tool::Circle, Tool::Offset, Tool::Extrude] {
            assert!(
                tut.steps
                    .iter()
                    .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(t)) if t == tool)),
                "the walkthrough picks {tool:?}"
            );
        }
    }

    /// #1674: the assists leave two concentric circles and a tube extruded between them.
    #[test]
    fn offset_tutorial_assists_build_a_tube() {
        let mut app = AppState::default();
        finish_tutorial_via_next(&mut app, "offset");
        assert_eq!(
            app.doc.sketch_offset_ops.len(),
            1,
            "one offset operation, status={}",
            app.status
        );
        assert_eq!(app.doc.circles.len(), 2, "the circle plus its offset copy");
        let radii: Vec<f32> = app.doc.circles.values().map(|c| c.r).collect();
        let (min, max) = (
            radii.iter().cloned().fold(f32::MAX, f32::min),
            radii.iter().cloned().fold(0.0f32, f32::max),
        );
        assert!(
            (max - min - OFFSET_DISTANCE_MM).abs() < 0.01,
            "the copy sits {OFFSET_DISTANCE_MM} mm out, radii={radii:?}"
        );
        assert!(has_extrusion(&app), "the ring is extruded, status={}", app.status);
        let faces = &app.doc.extrusions.values().next().unwrap().faces;
        assert!(
            matches!(faces.first(), Some(crate::model::ExtrudeFace::Boolean { .. })),
            "the profile is the ring between the two circles, got {faces:?}"
        );
    }

    fn tilted_plane_tut() -> &'static Tutorial {
        &TUTORIALS[tutorial_index("tilted_plane").expect("tilted_plane tutorial is registered")]
    }

    /// #1673: tilt a construction plane off an axis, build on it, then move the plane and
    /// watch the solid follow.
    #[test]
    fn tilted_plane_tutorial_is_registered_and_ends_by_moving_the_plane() {
        let tut = tilted_plane_tut();
        assert_eq!(tut.name, "tilted_plane");
        assert_eq!(tut.title, "Angled plane");
        let joined: String = tut
            .steps
            .iter()
            .map(|s| s.narration.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("axis"), "{joined}");
        assert!(joined.contains("tilt") || joined.contains("angle"), "{joined}");
        assert!(
            tut.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::ConstructionPlane)))),
            "the walkthrough picks the Construction Plane tool"
        );
        assert!(
            tut.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::ElementsPlane))),
            "and reopens the plane from the Elements pane"
        );
        // Moving the plane is the point, so it comes after the solid is built.
        let extrude = tut
            .steps
            .iter()
            .position(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Extrude))))
            .expect("an extrude step");
        let reopen = tut
            .steps
            .iter()
            .position(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::ElementsPlane)))
            .expect("a reopen step");
        assert!(reopen > extrude, "move the plane after the solid exists");
    }

    /// #1673: the assists leave a tilted plane carrying a solid, and the final step
    /// re-tilts the plane — the solid's frame moves with it.
    #[test]
    fn tilted_plane_tutorial_assists_build_on_a_plane_that_then_moves() {
        let mut app = AppState::default();
        finish_tutorial_via_next(&mut app, "tilted_plane");
        let plane = tutorial_tilted_plane(&app).expect("an axis-anchored plane");
        let def = &app.doc.construction_planes[plane].definition;
        assert!(
            (def.angle_deg - 60.0).abs() < 0.01,
            "the last step re-tilts the plane to 60, got {}",
            def.angle_deg
        );
        assert!(has_extrusion(&app), "a solid stands on the plane, status={}", app.status);
        // The sketch really is hosted on the tilted plane, so the solid follows it.
        assert!(
            app.doc
                .sketches
                .values()
                .any(|s| s.face == crate::model::FaceId::ConstructionPlane(plane)),
            "the sketch is hosted on the tilted plane"
        );
        let normal = app.doc.construction_planes[plane].normal;
        assert!(
            normal.dot(glam::Vec3::Z).abs() < 0.99,
            "the plane really is tilted off the ground, normal={normal:?}"
        );
    }

    fn revolve_tut() -> &'static Tutorial {
        &TUTORIALS[tutorial_index("revolve").expect("revolve tutorial is registered")]
    }

    /// #1672: spin a square into a ring, then cut a half-round groove into its face.
    #[test]
    fn revolve_tutorial_is_registered_and_covers_a_cut_groove() {
        let tut = revolve_tut();
        assert_eq!(tut.name, "revolve");
        assert_eq!(tut.title, "Revolve");
        let joined: String = tut
            .steps
            .iter()
            .map(|s| s.narration.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("axis"), "{joined}");
        assert!(joined.contains("groove"), "{joined}");
        assert!(
            tut.steps
                .iter()
                .filter(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Revolve))))
                .count()
                >= 2,
            "the Revolve tool is picked twice: once to spin, once to cut"
        );
        assert!(
            tut.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::OutputMode("cut")))),
            "a step points at Cut in the Output row"
        );
        assert!(
            tut.steps
                .iter()
                .any(|s| matches!(s.anchor, StepAnchor::Ui(UiAnchor::Tool(Tool::Circle)))),
            "the groove profile is a circle"
        );
    }

    /// #1672: the assists leave a revolved ring with a cut groove around it.
    #[test]
    fn revolve_tutorial_assists_groove_a_revolved_ring() {
        let mut app = AppState::default();
        finish_tutorial_via_next(&mut app, "revolve");
        assert_eq!(
            app.doc.revolutions.len(),
            2,
            "one revolve for the ring, one for the groove, status={}",
            app.status
        );
        let modes: Vec<_> = app.doc.revolutions.values().map(|r| r.mode.clone()).collect();
        assert!(
            modes
                .iter()
                .any(|m| matches!(m, crate::model::RevolveMode::NewBody)),
            "the ring is a new body, got {modes:?}"
        );
        assert!(
            modes
                .iter()
                .any(|m| matches!(m, crate::model::RevolveMode::Cut(b) if !b.is_empty())),
            "the groove cuts the ring, got {modes:?}"
        );
        assert!(
            app.doc.circles.len() == 1,
            "the groove profile is one circle, status={}",
            app.status
        );
    }
}
