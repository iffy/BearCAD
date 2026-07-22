//! Interactive tutorial mode: the view-cube bear walks a first-time user through
//! building a real part, pointing with glowing rings and narrating in a speech
//! bubble. Tutorials live in a registry ([`TUTORIALS`]) so more can be added; each
//! is a list of [`Step`]s that either auto-advance when a document predicate is
//! satisfied or wait for the bubble's Next button.

use crate::actions::{AppState, Tool};
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
}

/// Where a step's glowing ring points.
#[derive(Clone, Copy, Debug)]
pub enum StepAnchor {
    Ui(UiAnchor),
    /// A computed world point, projected into the viewport — e.g. the next profile
    /// vertex to click, so a drawing step leads point by point.
    World(fn(&AppState) -> Option<glam::Vec3>),
    /// No ring — narration only.
    None,
}

pub struct Step {
    /// What the bear says for this step.
    pub narration: &'static str,
    pub anchor: StepAnchor,
    /// Auto-advance when this returns true; `None` shows a Next button instead.
    pub done: Option<fn(&AppState) -> bool>,
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

fn params_defined(app: &AppState) -> bool {
    ["leg", "width", "thick", "hole", "bend", "bend_angle"].iter().all(|name| {
        app.doc
            .parameters
            .iter()
            .any(|p| !p.deleted && p.name.eq_ignore_ascii_case(name))
    })
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

fn profile_squared(app: &AppState) -> bool {
    let count = |f: fn(&ConstraintKind) -> bool| live_constraints(app).filter(|c| f(&c.kind)).count();
    count(|k| matches!(k, ConstraintKind::Coincident { .. })) >= 1
        && count(|k| matches!(k, ConstraintKind::Horizontal { .. } | ConstraintKind::Vertical { .. })) >= 1
        && count(|k| matches!(k, ConstraintKind::Parallel { .. })) >= 2
        && count(|k| matches!(k, ConstraintKind::Perpendicular { .. })) >= 2
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
        narration: "Hi, I'm the bear! Let's build a real part together: a 120\u{b0} angle \
                    bracket with a rounded bend and countersunk screw holes. I'll point with \
                    glowing rings; you do the clicking. I've opened a fresh document for us.",
        anchor: StepAnchor::None,
        done: None,
    },
    Step {
        narration: "First, a name for our first number. See the Parameters pane on the \
                    right? Tap inside the name box \u{2014} the pulsing ring marks it.",
        anchor: StepAnchor::Ui(UiAnchor::ParametersName),
        done: Some(name_box_tapped),
    },
    Step {
        narration: "Type leg \u{2014} just those three letters. It's the length of each \
                    of the bracket's legs.",
        anchor: StepAnchor::Ui(UiAnchor::ParametersName),
        done: Some(name_says_leg),
    },
    Step {
        narration: "Now tap the value box beside it and type 50mm.",
        anchor: StepAnchor::Ui(UiAnchor::ParametersValue),
        done: Some(value_says_50),
    },
    Step {
        narration: "Press + to add it. Your first parameter!",
        anchor: StepAnchor::Ui(UiAnchor::ParametersAdd),
        done: Some(leg_added),
    },
    Step {
        narration: "Five more, exactly the same moves:\n\
                    width = 40mm\nthick = 5mm\nhole = 5mm\nbend = 4mm\nbend_angle = 120deg",
        anchor: StepAnchor::Ui(UiAnchor::ParametersName),
        done: Some(params_defined),
    },
    Step {
        narration: "Grab the Line tool \u{2014} the glowing button up top, or press L.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Line)),
        done: Some(line_tool_active),
    },
    Step {
        narration: "Now follow me around the profile: click each glowing point in turn \
                    \u{2014} down the base leg, a short end cap, back along the inside, up \
                    the tilted leg, and finally back to the start to close the loop. \
                    Sloppy is fine \u{2014} we'll square it up next!",
        anchor: StepAnchor::World(next_profile_point),
        done: Some(profile_drawn),
    },
    Step {
        narration: "Now the Constraint tool \u{2014} the glowing button, or press C.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Constraint)),
        done: Some(constraint_tool_active),
    },
    Step {
        narration: "Square it up! Select things, then press a number:\n\
                    \u{2022} bend corner + origin \u{2192} 4 (Coincident)\n\
                    \u{2022} bottom base line \u{2192} 7 (Horizontal)\n\
                    \u{2022} bottom + inner base lines \u{2192} 1 (Parallel)\n\
                    \u{2022} the two leg lines \u{2192} 1 (Parallel)\n\
                    \u{2022} each end cap + its leg \u{2192} 2 (Perpendicular)",
        anchor: StepAnchor::None,
        done: Some(profile_squared),
    },
    Step {
        narration: "Exact sizes with the Dimension tool (D): click each outer leg and type \
                    leg; each end cap gets thick. For the bend: select the bottom line and \
                    the inner leg line, press D, type bend_angle.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Dimension)),
        done: Some(profile_dimensioned),
    },
    Step {
        narration: "Esc to leave the sketch, then Extrude (E): click the profile face, type \
                    width, press Enter. A solid!",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Extrude)),
        done: Some(extruded),
    },
    Step {
        narration: "Round the bend with Fillet (F): click the inside edge of the bend and \
                    type bend. Then the outside edge: bend + thick. Concentric, like bent \
                    sheet metal.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Fillet)),
        done: Some(bend_rounded),
    },
    Step {
        narration: "Screw holes! Sketch (S) on the inside face of the base flange, then \
                    Circle (O): place two circles near the flange tip, typing hole for each \
                    diameter. Position them with the Dimension tool (D) against the face \
                    edges.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Sketch)),
        done: Some(hole_circles_drawn),
    },
    Step {
        narration: "Esc, then Extrude (E): click both circles, drag the handle into the \
                    bracket (or type thick + 1), pick Cut, press Enter.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Extrude)),
        done: Some(holes_cut),
    },
    Step {
        narration: "Countersink them: Chamfer (K), click one hole's rim where it meets the \
                    face, Shift+click the other, type 1.2, Enter.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Chamfer)),
        done: Some(holes_countersunk),
    },
    Step {
        narration: "Fillet (F) again: click a vertical edge at a flange tip, Shift+click the \
                    other corners, type 2, Enter. Rounded corners!",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Fillet)),
        done: Some(corners_rounded),
    },
    Step {
        narration: "Sign your work: Text (T) on the outer face of the base, type BearCAD. \
                    Then Extrude (E) the text, push the handle into the face (type 1), pick \
                    Cut \u{2014} engraved letters.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Text)),
        done: Some(label_engraved),
    },
    Step {
        narration: "The best part: in the Parameters pane, change bend_angle from 120deg to \
                    150deg. The whole part rebuilds \u{2014} bend, holes, countersinks and \
                    all.",
        anchor: StepAnchor::Ui(UiAnchor::ParametersAdd),
        done: Some(bend_angle_changed),
    },
    Step {
        narration: "You built it! Export via File \u{2192} Export \u{2192} STL or STEP. \
                    That's the whole loop: sketch, constrain, dimension, extrude, refine \u{2014} \
                    and parameters drive everything. See you around the viewport!",
        anchor: StepAnchor::None,
        done: None,
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

    #[test]
    fn tutorial_registry_lookup_by_name() {
        assert_eq!(tutorial_index("bracket"), Some(0));
        assert_eq!(tutorial_index("nope"), None);
        assert!(TUTORIALS[0].steps.len() >= 10);
    }
}
