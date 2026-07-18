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
}

/// Where a step's glowing ring points.
#[derive(Clone, Copy, Debug)]
pub enum StepAnchor {
    Ui(UiAnchor),
    /// The world origin, projected into the viewport (for "click the ground" steps).
    WorldOrigin,
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

fn fillet_count(app: &AppState) -> usize {
    app.doc
        .extrusions
        .iter()
        .filter(|e| !e.deleted)
        .flat_map(|e| &e.edge_treatments)
        .filter(|t| t.kind == VertexTreatmentKind::Fillet)
        .count()
}

fn chamfer_count(app: &AppState) -> usize {
    app.doc
        .extrusions
        .iter()
        .filter(|e| !e.deleted)
        .flat_map(|e| &e.edge_treatments)
        .filter(|t| t.kind == VertexTreatmentKind::Chamfer)
        .count()
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
        narration: "First, names for our numbers. In the Parameters pane on the right, type a \
                    name and value, then press + \u{2014} six times:\n\
                    leg = 50mm, width = 40mm, thick = 5mm,\n\
                    hole = 5mm, bend = 4mm, bend_angle = 120deg",
        anchor: StepAnchor::Ui(UiAnchor::ParametersAdd),
        done: Some(params_defined),
    },
    Step {
        narration: "Grab the Line tool \u{2014} the glowing button up top, or press L.",
        anchor: StepAnchor::Ui(UiAnchor::Tool(Tool::Line)),
        done: Some(line_tool_active),
    },
    Step {
        narration: "Click on the ground near the origin and chain six sloppy lines: a long \
                    base leg, a short end cap, back along the inside, up the tilted leg \
                    (roughly 120\u{b0} from the base), its end cap, and close the loop back at \
                    the start. Sloppy is fine \u{2014} we'll square it up next!",
        anchor: StepAnchor::WorldOrigin,
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
        app.tutorial = Some(TutorialRun { tutorial: 0, step: 1 });

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

    #[test]
    fn tutorial_registry_lookup_by_name() {
        assert_eq!(tutorial_index("bracket"), Some(0));
        assert_eq!(tutorial_index("nope"), None);
        assert!(TUTORIALS[0].steps.len() >= 10);
    }
}
