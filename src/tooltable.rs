//! The per-tool behaviour table (#1508).
//!
//! `opsigs` is the compile-time contract for what an operation *makes*. This is the contract
//! for how its tool *behaves*: where it lives, what its drafts are, what Enter and Esc do,
//! whether the pointer places a value, and whether the tool has a New/Add/Cut output.
//!
//! Before this module those policies were a dozen separate `matches!` lists spread across
//! `SetTool`, `CancelOperation`, `handle_shortcuts`, `is_sketch_edit_tool` and seventeen
//! copy-pasted Enter handlers, and no two agreed (#1481–#1485). The lists are gone: the
//! runtime reads a row, and a test walks every row.
//!
//! ## Adding a tool
//!
//! 1. Add the variant to the `tools!` list in `actions.rs` — that generates [`Tool::ALL`],
//!    so a new tool can never fall out of an exhaustive walk (#1481).
//! 2. Add its arm to [`spaces`] and to [`row`]. Both matches are exhaustive, so **the build
//!    fails until the row exists**. That is the point: a new tool cannot skip the table.
//! 3. `tooltable::tests::every_tool_has_a_row` and the per-column walks below then check the
//!    row is coherent (a tool with a draft must clear it on Esc, a `Placement` gizmo must own
//!    the next click, and so on).
//!
//! ## Adding a column
//!
//! Add the field to [`ToolRow`], fill it in for every arm of [`row`] (the compiler lists the
//! arms for you), point the handler that used to `matches!` at the field, and add a walk in
//! `tests`. Columns that exist so far are documented on [`ToolRow`]; later issues still move
//! commit buttons, SceneElement / re-edit path, preview bounds and default amounts here.
//! Expression storage is [`stored_value_fields`] (#1489).

use crate::actions::Tool;
use eframe::egui;

/// Which space a row applies in. Dual-mode tools (Chamfer, Fillet, Mirror, Repeat, Slice,
/// Dimension) contribute one row per space, the way [`crate::opsigs::OpSpace`] already splits
/// the operation tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolSpace {
    /// Inside an open sketch.
    Sketch,
    /// 3D solid modelling — no sketch open.
    Solid,
    /// The drawing workbench's sheet.
    Drawing,
}

impl ToolSpace {
    /// The space the app is in right now.
    pub fn current(in_sketch: bool, in_drawing: bool) -> Self {
        if in_drawing {
            Self::Drawing
        } else if in_sketch {
            Self::Sketch
        } else {
            Self::Solid
        }
    }
}

/// What the pointer does for a tool while its draft is open (#1497/#1502).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gizmo {
    /// Nothing follows the pointer; clicks only fill pickers.
    None,
    /// The pointer **is** the value: between the first click and the last, the draft tracks
    /// the cursor, so the next click belongs to the draft and nothing else may take it —
    /// that is the one rule behind [`crate::actions::AppState::draft_blocks_tool_switch`].
    Placement,
    /// A dragged handle plus a typed field: the value sticks where it is left, and typing
    /// into the field locks it (`user_edited`). Picks still go to the tool's pickers, so a
    /// value gizmo never blocks a tool switch.
    Value,
}

/// What Esc does (#1484). One rule, so no tool can drift again: the first press empties what
/// the tool has picked and leaves the tool armed, the second returns to Select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Esc {
    /// Empty this row's [`Draft`] if it holds anything, else return to the Select tool.
    ClearThenSelect,
    /// The Select tool has nothing of its own to empty; Esc leaves an open sketch instead.
    LeaveSketch,
}

/// The in-progress state a row owns — what Esc empties and what leaving the tool drops.
///
/// One variant per `AppState::creating_*` field that a tool can fill, plus [`Draft::Selection`]
/// for the tools whose picker *is* the live selection and [`Draft::None`] for tools that keep
/// nothing between clicks. Naming the draft here is what replaced `CancelOperation`'s
/// hand-ordered if/else chain, whose order meant an armed Extrude beat an armed Move
/// regardless of which tool was actually active (#1484).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Draft {
    None,
    /// The tool picks into `AppState::scene_selection` (Constraint, Project).
    Selection,
    Rect,
    Line,
    Circle,
    Plane,
    Extrusion,
    VertexTreatment,
    EdgeTreatment,
    SketchOffset,
    Loft,
    Revolve,
    Sweep,
    Shape,
    Boolean,
    Move,
    Mirror,
    SketchMirror,
    Repeat,
    SketchRepeat,
    Slice,
    SketchSlice,
    Shell,
    Joint,
}

/// One picker this tool can arm (#1485/#1508).
///
/// Headings are the names `bearcad.ui.picker_focus` and the context pane use. A tool that
/// shows the same set under two labels (Combine's "Bodies" / "Side A") lists both, so a
/// script can name either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolPicker {
    pub target: crate::context::PickerTarget,
    pub heading: &'static str,
}

/// One of a tool's own value fields, named by the id the widget is built with.
///
/// Enter from one of these commits the tool; Enter from any *other* keyboard-holding widget
/// (the Elements pane's rename box, a Parameters row) must not (#1483).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolField {
    /// `egui::Id::new(name)`.
    Named(&'static str),
    /// `egui::Id::new((prefix, label))` for each label — the pattern every multi-row tool
    /// section uses (`("move_field", salt)`, `("repeat_var_field", label)`, …).
    Keyed(&'static str, &'static [&'static str]),
}

impl ToolField {
    /// Whether `id` is this field (or one of them, for a keyed group).
    pub fn matches(self, id: egui::Id) -> bool {
        match self {
            Self::Named(name) => egui::Id::new(name) == id,
            Self::Keyed(prefix, labels) => labels.iter().any(|l| egui::Id::new((prefix, *l)) == id),
        }
    }
}

/// One row of the table: everything the runtime needs to know about a tool in one space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolRow {
    pub tool: Tool,
    pub space: ToolSpace,
    /// Clicking a face with this tool outside a sketch opens a sketch on that face and the
    /// tool survives into it — `Tool::is_sketch_edit_tool` reads this (#1494).
    pub face_click_opens_sketch: bool,
    /// What the pointer drives while the draft is open (#1497/#1502).
    pub gizmo: Gizmo,
    /// Whether Enter commits this tool at all, and from which of its own fields (#1483).
    /// Empty with `commit_on_enter` set means "only when nothing holds the keyboard".
    pub commit_on_enter: bool,
    pub commit_fields: &'static [ToolField],
    /// What Esc does (#1484).
    pub esc: Esc,
    /// The state Esc empties and a tool switch drops.
    pub draft: Draft,
    /// Whether the tool has a New/Add/Cut output the `Y` shortcut cycles (#1397/#1501).
    pub output_modes: bool,
    /// Pickers this tool can arm (#1485). `focus_tool_picker` / `picker_focus` succeed
    /// only for these; a name that is not here is an error, not a silent no-op.
    pub pickers: &'static [ToolPicker],
}

/// The spaces a tool has a row in. Exhaustive: a new `Tool` variant does not compile until
/// it is listed.
pub fn spaces(tool: Tool) -> &'static [ToolSpace] {
    use ToolSpace::*;
    match tool {
        // Select works everywhere; the drawing workbench's selection is still Select's.
        Tool::Select => &[Sketch, Solid, Drawing],
        // Sketch-only tools. Outside a sketch their first click opens one (#1494).
        Tool::Rectangle
        | Tool::Line
        | Tool::Circle
        | Tool::Constraint
        | Tool::Offset
        | Tool::Project
        | Tool::Text => &[Sketch],
        // Dual-mode: a distinct draft and a distinct pick vocabulary per space.
        Tool::Dimension | Tool::Chamfer | Tool::Fillet | Tool::Mirror | Tool::Repeat
        | Tool::Slice => &[Sketch, Solid],
        // 3D-only.
        Tool::ConstructionPlane
        | Tool::Sketch
        | Tool::Extrude
        | Tool::Loft
        | Tool::Revolve
        | Tool::Shape
        | Tool::Sweep
        | Tool::Combine
        | Tool::Move
        | Tool::Shell
        | Tool::Joint => &[Solid],
        // Drawing workbench only.
        Tool::DrawingAdd | Tool::DrawingAlign => &[Drawing],
    }
}


// ── Field groups ────────────────────────────────────────────────────────────
//
// The ids the tools' own value inputs are built with. Kept next to the rows so a renamed
// field is renamed in one place; the `tool_fields_are_real_ids` walk keeps them non-empty.

const EXTRUDE_FIELDS: &[ToolField] = &[
    ToolField::Named("extrude_distance_input"),
    ToolField::Named("extrude_distance"),
    ToolField::Named("extrude_taper"),
];
const REVOLVE_FIELDS: &[ToolField] = &[
    ToolField::Named("revolve_angle_input"),
    ToolField::Named("revolve_angle_field"),
    ToolField::Named("revolve_gap_field"),
];
const SHELL_FIELDS: &[ToolField] = &[ToolField::Named("shell_thickness")];
const SKETCH_OFFSET_FIELDS: &[ToolField] = &[
    ToolField::Named("sketch_offset_distance_input"),
    ToolField::Named("sketch_offset_distance"),
];
const VERTEX_TREATMENT_FIELDS: &[ToolField] = &[
    ToolField::Named("vertex_treatment_amount_input"),
    ToolField::Named("treatment_amount"),
];
const EDGE_TREATMENT_FIELDS: &[ToolField] = &[
    ToolField::Named("edge_treatment_amount_input"),
    ToolField::Named("treatment_amount"),
];
const REPEAT_FIELDS: &[ToolField] = &[ToolField::Keyed(
    "repeat_var_field",
    // Both labels the gap row can carry ("Gap" and "Offset") count — the row's id follows
    // its display label (#646).
    &["Count", "Gap", "Offset", "Distance"],
)];
const SKETCH_REPEAT_FIELDS: &[ToolField] = &[ToolField::Keyed(
    "sketch_repeat_var_field",
    &["Count", "Gap", "Offset", "Distance"],
)];
const MOVE_FIELDS: &[ToolField] = &[ToolField::Keyed(
    "move_field",
    crate::context::MOVE_VALUE_SLOTS,
)];
const JOINT_FIELDS: &[ToolField] = &[ToolField::Keyed(
    "joint_field",
    &["Lead", "Offset", "Min", "Max", "Angle", "Distance"],
)];
const SHAPE_FIELDS: &[ToolField] = &[
    ToolField::Keyed("shape_field", &["Width", "Depth", "Height", "Radius"]),
    // The cuboid Base phase's two floating fields (#1102).
    ToolField::Named("shape_base_width"),
    ToolField::Named("shape_base_depth"),
];
const PLANE_FIELDS: &[ToolField] = &[
    // Pane ids and the floating viewport fields they stay in lock-step with.
    ToolField::Named("plane_offset_ctx"),
    ToolField::Named("plane_angle_ctx"),
    ToolField::Named("cp_offset"),
    ToolField::Named("cp_angle"),
];

// ── Picker groups (#1485) ───────────────────────────────────────────────────

use crate::context::PickerTarget as P;

const SELECTION: &[ToolPicker] = &[ToolPicker { target: P::Selection, heading: "Selection" }];
const PLANE_PICKERS: &[ToolPicker] = &[ToolPicker { target: P::PlaneAnchor, heading: "Anchor" }];
const EXTRUDE_PICKERS: &[ToolPicker] = &[
    ToolPicker { target: P::ExtrudeProfile, heading: "Faces" },
    ToolPicker { target: P::ExtrudeUpTo, heading: "Up to" },
];
const EDGE_TREATMENT_PICKERS: &[ToolPicker] =
    &[ToolPicker { target: P::TreatmentEdges, heading: "Edges" }];
const LOFT_PICKERS: &[ToolPicker] = &[
    ToolPicker { target: P::LoftSections, heading: "Sections" },
    ToolPicker { target: P::LoftCut, heading: "Cut bodies" },
];
const REVOLVE_PICKERS: &[ToolPicker] = &[
    ToolPicker { target: P::RevolveProfile, heading: "Profile" },
    ToolPicker { target: P::RevolveAxis, heading: "Axis" },
    ToolPicker { target: P::RevolveCut, heading: "Cut bodies" },
];
const SWEEP_PICKERS: &[ToolPicker] = &[
    ToolPicker { target: P::SweepProfile, heading: "Profile" },
    ToolPicker { target: P::SweepPath, heading: "Path" },
    ToolPicker { target: P::SweepCut, heading: "Cut bodies" },
];
const COMBINE_PICKERS: &[ToolPicker] = &[
    ToolPicker { target: P::CombineA, heading: "Bodies" },
    ToolPicker { target: P::CombineA, heading: "Side A" },
    ToolPicker { target: P::CombineB, heading: "Side B" },
];
const MOVE_PICKERS: &[ToolPicker] = &[
    ToolPicker { target: P::MoveTargets, heading: "Bodies" },
    ToolPicker { target: P::MoveFaceMoving, heading: "Moving face" },
    ToolPicker { target: P::MoveFaceFixed, heading: "Fixed face" },
    ToolPicker { target: P::MoveStartA, heading: "Start point A" },
    ToolPicker { target: P::MoveStartA, heading: "Reference Point" },
    ToolPicker { target: P::MoveEndA, heading: "End point A" },
    ToolPicker { target: P::MoveStartB, heading: "Start point B" },
    ToolPicker { target: P::MoveEndB, heading: "End point B" },
    ToolPicker { target: P::MoveStartC, heading: "Start point C" },
    ToolPicker { target: P::MoveEndC, heading: "End point C" },
];
const MIRROR_SOLID_PICKERS: &[ToolPicker] = &[
    ToolPicker { target: P::MirrorPlane, heading: "Mirror plane" },
    ToolPicker { target: P::MirrorTargets, heading: "Bodies" },
];
const MIRROR_SKETCH_PICKERS: &[ToolPicker] = &[
    ToolPicker { target: P::SketchMirrorLine, heading: "Mirror line" },
    ToolPicker { target: P::SketchMirrorShapes, heading: "Shapes" },
];
const REPEAT_SOLID_PICKERS: &[ToolPicker] = &[
    ToolPicker { target: P::RepeatTargets, heading: "Bodies" },
    ToolPicker { target: P::RepeatPath, heading: "Path" },
    ToolPicker { target: P::RepeatDistanceTo, heading: "Distance to" },
];
const REPEAT_SKETCH_PICKERS: &[ToolPicker] = &[
    ToolPicker { target: P::SketchRepeatEntities, heading: "Entities" },
    ToolPicker { target: P::SketchRepeatDirection, heading: "Direction" },
];
const SLICE_SOLID_PICKERS: &[ToolPicker] = &[
    ToolPicker { target: P::SliceTargets, heading: "Targets" },
    ToolPicker { target: P::SliceCutters, heading: "Cutters" },
];
const SLICE_SKETCH_PICKERS: &[ToolPicker] = &[
    ToolPicker { target: P::SketchSliceTargets, heading: "Targets" },
    ToolPicker { target: P::SketchSliceCutters, heading: "Cutters" },
];
const SHELL_PICKERS: &[ToolPicker] = &[
    ToolPicker { target: P::ShellTargets, heading: "Targets" },
    ToolPicker { target: P::ShellOpenFaces, heading: "Open faces" },
];
const OFFSET_PICKERS: &[ToolPicker] = &[
    ToolPicker { target: P::Selection, heading: "Selection" },
    ToolPicker { target: P::SketchOffsetEntities, heading: "Entities" },
];
const JOINT_PICKERS: &[ToolPicker] = &[
    ToolPicker { target: P::JointMembers, heading: "Parts" },
    ToolPicker { target: P::JointMobile, heading: "Moving part" },
    ToolPicker { target: P::JointFixed, heading: "Fixed part" },
    ToolPicker { target: P::JointMovingFace, heading: "Moving face" },
    ToolPicker { target: P::JointFixedFace, heading: "Fixed face" },
    ToolPicker { target: P::JointFrameOrigin, heading: "Origin" },
    ToolPicker { target: P::JointFramePrimary, heading: "Axis" },
    ToolPicker { target: P::JointFrameSecondary, heading: "Second axis" },
    ToolPicker { target: P::JointMinStop, heading: "Min stop" },
    ToolPicker { target: P::JointMaxStop, heading: "Max stop" },
];
const DRAWING_SELECT_PICKERS: &[ToolPicker] =
    &[ToolPicker { target: P::DrawingSelection, heading: "Selection" }];
const DRAWING_ALIGN_PICKERS: &[ToolPicker] =
    &[ToolPicker { target: P::DrawingAlignBase, heading: "Base view" }];

impl ToolSpace {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sketch => "sketch",
            Self::Solid => "solid",
            Self::Drawing => "drawing",
        }
    }
}

impl Gizmo {
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Placement => "placement",
            Self::Value => "value",
        }
    }
}

impl Esc {
    pub const fn label(self) -> &'static str {
        match self {
            Self::ClearThenSelect => "clear then select",
            Self::LeaveSketch => "leave sketch",
        }
    }
}

/// The row for one tool in one space.
///
/// Exhaustive over `Tool`: a new variant does not compile until it has a row. Asking for a
/// space the tool has no row in still answers (with that tool's only row) rather than
/// panicking — the runtime asks before it knows whether the tool is usable here.
pub fn row(tool: Tool, space: ToolSpace) -> ToolRow {
    let base = ToolRow {
        tool,
        space,
        face_click_opens_sketch: false,
        gizmo: Gizmo::None,
        commit_on_enter: false,
        commit_fields: &[],
        esc: Esc::ClearThenSelect,
        draft: Draft::None,
        output_modes: false,
        pickers: &[],
    };
    let sketch = space == ToolSpace::Sketch;
    match tool {
        Tool::Select => ToolRow {
            esc: Esc::LeaveSketch,
            pickers: if space == ToolSpace::Drawing { DRAWING_SELECT_PICKERS } else { SELECTION },
            ..base
        },

        // ── Sketch drawing tools: the pointer is the value, so a half-drawn shape owns the
        // next click. Their own dimension fields commit them, not a bare Enter.
        Tool::Rectangle => ToolRow {
            face_click_opens_sketch: true,
            gizmo: Gizmo::Placement,
            draft: Draft::Rect,
            pickers: SELECTION,
            ..base
        },
        Tool::Line => ToolRow {
            face_click_opens_sketch: true,
            gizmo: Gizmo::Placement,
            draft: Draft::Line,
            pickers: SELECTION,
            ..base
        },
        Tool::Circle => ToolRow {
            face_click_opens_sketch: true,
            gizmo: Gizmo::Placement,
            draft: Draft::Circle,
            pickers: SELECTION,
            ..base
        },
        Tool::Text => ToolRow {
            face_click_opens_sketch: true,
            gizmo: Gizmo::Placement,
            pickers: SELECTION,
            ..base
        },
        Tool::ConstructionPlane => ToolRow {
            gizmo: Gizmo::Placement,
            commit_on_enter: true,
            commit_fields: PLANE_FIELDS,
            draft: Draft::Plane,
            pickers: PLANE_PICKERS,
            ..base
        },
        Tool::Sketch => ToolRow { pickers: SELECTION, ..base },
        Tool::Dimension => ToolRow {
            face_click_opens_sketch: true,
            gizmo: Gizmo::Placement,
            pickers: SELECTION,
            ..base
        },
        Tool::Constraint => ToolRow {
            face_click_opens_sketch: true,
            draft: Draft::Selection,
            pickers: SELECTION,
            ..base
        },
        Tool::Project => ToolRow {
            face_click_opens_sketch: true,
            commit_on_enter: true,
            draft: Draft::Selection,
            pickers: SELECTION,
            ..base
        },

        // ── Value-gizmo tools ───────────────────────────────────────────────
        Tool::Extrude => ToolRow {
            gizmo: Gizmo::Value,
            commit_on_enter: true,
            commit_fields: EXTRUDE_FIELDS,
            draft: Draft::Extrusion,
            output_modes: true,
            pickers: EXTRUDE_PICKERS,
            ..base
        },
        Tool::Chamfer | Tool::Fillet => ToolRow {
            face_click_opens_sketch: true,
            gizmo: Gizmo::Value,
            commit_on_enter: true,
            commit_fields: if sketch { VERTEX_TREATMENT_FIELDS } else { EDGE_TREATMENT_FIELDS },
            draft: if sketch { Draft::VertexTreatment } else { Draft::EdgeTreatment },
            pickers: if sketch { SELECTION } else { EDGE_TREATMENT_PICKERS },
            ..base
        },
        Tool::Offset => ToolRow {
            face_click_opens_sketch: true,
            gizmo: Gizmo::Value,
            commit_on_enter: true,
            commit_fields: SKETCH_OFFSET_FIELDS,
            draft: Draft::SketchOffset,
            pickers: OFFSET_PICKERS,
            ..base
        },
        Tool::Loft => ToolRow {
            commit_on_enter: true,
            draft: Draft::Loft,
            output_modes: true,
            pickers: LOFT_PICKERS,
            ..base
        },
        Tool::Revolve => ToolRow {
            gizmo: Gizmo::Value,
            commit_on_enter: true,
            commit_fields: REVOLVE_FIELDS,
            draft: Draft::Revolve,
            output_modes: true,
            pickers: REVOLVE_PICKERS,
            ..base
        },
        Tool::Sweep => ToolRow {
            commit_on_enter: true,
            draft: Draft::Sweep,
            output_modes: true,
            pickers: SWEEP_PICKERS,
            ..base
        },
        Tool::Shape => ToolRow {
            gizmo: Gizmo::Placement,
            commit_on_enter: true,
            commit_fields: SHAPE_FIELDS,
            draft: Draft::Shape,
            ..base
        },
        Tool::Combine => ToolRow {
            commit_on_enter: true,
            draft: Draft::Boolean,
            pickers: COMBINE_PICKERS,
            ..base
        },
        Tool::Move => ToolRow {
            gizmo: Gizmo::Value,
            commit_on_enter: true,
            commit_fields: MOVE_FIELDS,
            draft: Draft::Move,
            pickers: MOVE_PICKERS,
            ..base
        },
        Tool::Mirror => ToolRow {
            commit_on_enter: true,
            draft: if sketch { Draft::SketchMirror } else { Draft::Mirror },
            output_modes: !sketch,
            pickers: if sketch { MIRROR_SKETCH_PICKERS } else { MIRROR_SOLID_PICKERS },
            ..base
        },
        Tool::Repeat => ToolRow {
            gizmo: Gizmo::Value,
            commit_on_enter: true,
            commit_fields: if sketch { SKETCH_REPEAT_FIELDS } else { REPEAT_FIELDS },
            draft: if sketch { Draft::SketchRepeat } else { Draft::Repeat },
            pickers: if sketch { REPEAT_SKETCH_PICKERS } else { REPEAT_SOLID_PICKERS },
            ..base
        },
        Tool::Slice => ToolRow {
            commit_on_enter: true,
            draft: if sketch { Draft::SketchSlice } else { Draft::Slice },
            pickers: if sketch { SLICE_SKETCH_PICKERS } else { SLICE_SOLID_PICKERS },
            ..base
        },
        Tool::Shell => ToolRow {
            gizmo: Gizmo::Value,
            commit_on_enter: true,
            commit_fields: SHELL_FIELDS,
            draft: Draft::Shell,
            pickers: SHELL_PICKERS,
            ..base
        },
        Tool::Joint => ToolRow {
            commit_on_enter: true,
            commit_fields: JOINT_FIELDS,
            draft: Draft::Joint,
            pickers: JOINT_PICKERS,
            ..base
        },

        // ── Drawing workbench ───────────────────────────────────────────────
        Tool::DrawingAdd => ToolRow { gizmo: Gizmo::Placement, ..base },
        Tool::DrawingAlign => ToolRow {
            gizmo: Gizmo::Placement,
            pickers: DRAWING_ALIGN_PICKERS,
            ..base
        },
    }
}

impl ToolRow {
    /// The picker a heading names on this row, if any (#1485).
    pub fn picker_named(self, name: &str) -> Option<crate::context::PickerTarget> {
        self.pickers
            .iter()
            .find(|p| p.heading.eq_ignore_ascii_case(name))
            .map(|p| p.target)
    }
}

/// A committed numeric field that sits behind a value input (#1489).
///
/// The name is the storage field (e.g. `"angle"`, `"thickness"`), not the widget id.
/// [`stored_value_fields`] is exhaustive over [`Tool`]: a new tool does not compile
/// until it declares its fields (or explicitly none). The walk in
/// `actions::tests::every_value_input_round_trips_its_expression` then commits each
/// field with a distinctive expression and asserts the committed op stores that
/// text and that re-edit restores it verbatim.
#[cfg(test)]
pub fn stored_value_fields(tool: Tool, _space: ToolSpace) -> &'static [&'static str] {
    match tool {
        Tool::Select
        | Tool::Rectangle
        | Tool::Line
        | Tool::Circle
        | Tool::Sketch
        | Tool::Dimension
        | Tool::Constraint
        | Tool::Project
        | Tool::Loft
        | Tool::Sweep
        | Tool::Combine
        | Tool::Mirror
        | Tool::Slice
        | Tool::Text
        | Tool::DrawingAdd
        | Tool::DrawingAlign => &[],
        Tool::ConstructionPlane => &["offset", "angle"],
        Tool::Extrude => &["distance", "taper"],
        Tool::Chamfer | Tool::Fillet => &["amount"],
        Tool::Offset => &["distance"],
        Tool::Revolve => &["angle", "pitch"],
        Tool::Shape => &["width", "depth", "height", "radius"],
        Tool::Move => &[
            "tx",
            "ty",
            "tz",
            "rx",
            "ry",
            "rz",
            "roll_angle",
            "face_spin",
            "face_offset",
        ],
        Tool::Repeat => &["count", "spacing", "length"],
        Tool::Shell => &["thickness"],
        Tool::Joint => &["lead", "offset", "min", "max", "angle", "distance"],
    }
}

/// Every row: each tool once per space it lives in. The walks in `tests` (and any future
/// exhaustive check) iterate this, so a tool that falls out of [`Tool::ALL`] cannot hide.
pub fn all_rows() -> Vec<ToolRow> {
    Tool::ALL
        .iter()
        .flat_map(|&tool| spaces(tool).iter().map(move |&space| row(tool, space)))
        .collect()
}

// ── Enter-to-commit (#1483) ─────────────────────────────────────────────────

/// Whether Enter should commit, given who holds the keyboard.
///
/// The one rule, replacing the four that grew up separately: commit when nothing holds the
/// keyboard, **or** when what holds it is one of this tool's own value fields. Typing a
/// thickness and pressing Enter finishes the shell; pressing Enter to finish renaming an
/// element in the Elements pane does not.
pub fn enter_focus_allows_commit(
    wants_keyboard: bool,
    focused: Option<egui::Id>,
    fields: &[ToolField],
) -> bool {
    if !wants_keyboard {
        return true;
    }
    match focused {
        // A widget wants the keyboard but nothing is focused: a floating value field whose
        // Area focus is already gone by the time its own handler runs (#1275).
        None => true,
        Some(id) => fields.iter().any(|f| f.matches(id)),
    }
}

/// Enter was pressed and it belongs to this tool: the single test every tool's viewport
/// handler runs instead of rolling its own focus rule (#1483).
pub fn enter_commits(ctx: &egui::Context, fields: &[ToolField]) -> bool {
    ctx.input(|i| i.key_pressed(egui::Key::Enter))
        && enter_focus_allows_commit(
            ctx.egui_wants_keyboard_input(),
            ctx.memory(|m| m.focused()),
            fields,
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// #1481/#1508: every tool has a row in every space it claims, and no tool is missing.
    #[test]
    fn every_tool_has_a_row() {
        let rows = all_rows();
        let tools: HashSet<Tool> = rows.iter().map(|r| r.tool).collect();
        for tool in Tool::ALL {
            assert!(tools.contains(&tool), "{tool:?} has no row in the tool table");
            assert!(
                !spaces(tool).is_empty(),
                "{tool:?} claims no space; every tool lives somewhere"
            );
        }
        assert_eq!(rows.len(), Tool::ALL.iter().map(|t| spaces(*t).len()).sum::<usize>());
        // #1481: Shape used to fall out of the hand-written ALL array.
        assert!(Tool::ALL.contains(&Tool::Shape));
        let mut seen = HashSet::new();
        for tool in Tool::ALL {
            assert!(seen.insert(tool), "{tool:?} listed twice in Tool::ALL");
        }
    }

    /// A row answers for the space it was asked about, so callers can key by the live space.
    #[test]
    fn rows_carry_the_space_they_were_asked_for() {
        for tool in Tool::ALL {
            for space in [ToolSpace::Sketch, ToolSpace::Solid, ToolSpace::Drawing] {
                assert_eq!(row(tool, space).space, space);
                assert_eq!(row(tool, space).tool, tool);
            }
        }
    }

    /// #1484: exactly one tool opts out of the Esc rule, and it is the one with nothing of
    /// its own to clear.
    #[test]
    fn only_select_opts_out_of_the_esc_rule() {
        for r in all_rows() {
            let expected = if r.tool == Tool::Select { Esc::LeaveSketch } else { Esc::ClearThenSelect };
            assert_eq!(r.esc, expected, "{:?}/{:?}", r.tool, r.space);
        }
    }

    /// #1484: a tool that keeps picks must name the draft they live in, or Esc has nothing
    /// to empty and would throw the tool away on the first press.
    #[test]
    fn tools_with_a_draft_clear_it_before_leaving() {
        for r in all_rows() {
            if r.esc != Esc::ClearThenSelect {
                continue;
            }
            // A tool with no draft is one that commits on the click that fills it.
            if r.draft == Draft::None {
                assert!(
                    matches!(
                        r.tool,
                        Tool::Sketch
                            | Tool::Text
                            | Tool::Dimension
                            | Tool::DrawingAdd
                            | Tool::DrawingAlign
                    ),
                    "{:?}/{:?} keeps picks but names no draft",
                    r.tool,
                    r.space
                );
            }
        }
    }

    /// #1483: a tool that commits on Enter and has a typed value must list that field, or
    /// Enter from its own box is swallowed (the Shell bug).
    #[test]
    fn value_gizmo_tools_list_their_own_commit_fields() {
        for r in all_rows() {
            if r.gizmo == Gizmo::Value && r.commit_on_enter {
                assert!(
                    !r.commit_fields.is_empty(),
                    "{:?}/{:?} has a typed value gizmo but lists no commit field",
                    r.tool,
                    r.space
                );
            }
            if !r.commit_on_enter {
                assert!(
                    r.commit_fields.is_empty(),
                    "{:?}/{:?} lists commit fields but never commits on Enter",
                    r.tool,
                    r.space
                );
            }
        }
    }

    /// #1489: a value input without a stored-expression field is how Revolve/3D fillet
    /// drifted. Every row that has commit fields must declare what it stores; a row with
    /// no typed value must declare none.
    #[test]
    fn value_inputs_declare_expression_storage() {
        for r in all_rows() {
            let stored = stored_value_fields(r.tool, r.space);
            if r.commit_fields.is_empty() {
                assert!(
                    stored.is_empty(),
                    "{:?}/{:?} lists stored value fields but has no value input",
                    r.tool,
                    r.space
                );
            } else {
                assert!(
                    !stored.is_empty(),
                    "{:?}/{:?} has a value input but lists no stored expression field",
                    r.tool,
                    r.space
                );
            }
        }
    }

    /// A keyed field group with no labels matches nothing — a silent no-op like the ones
    /// this table exists to end.
    #[test]
    fn tool_fields_are_real_ids() {
        for r in all_rows() {
            for f in r.commit_fields {
                match f {
                    ToolField::Named(name) => assert!(!name.is_empty()),
                    ToolField::Keyed(prefix, labels) => {
                        assert!(!prefix.is_empty());
                        assert!(!labels.is_empty(), "{:?} has an empty keyed group", r.tool);
                    }
                }
            }
        }
    }

    /// #1483: the focus rule itself. Nothing focused commits; a foreign field does not; the
    /// tool's own field does.
    #[test]
    fn enter_commits_only_from_our_own_fields() {
        let own = &[ToolField::Named("shell_thickness")][..];
        assert!(enter_focus_allows_commit(false, None, own));
        assert!(enter_focus_allows_commit(false, Some(egui::Id::new("element_name")), own));
        assert!(enter_focus_allows_commit(true, None, own));
        assert!(enter_focus_allows_commit(
            true,
            Some(egui::Id::new("shell_thickness")),
            own
        ));
        assert!(!enter_focus_allows_commit(
            true,
            Some(egui::Id::new("element_name")),
            own
        ));
        // Keyed groups match every label they list and nothing else.
        let repeat = &[ToolField::Keyed("repeat_var_field", &["Count", "Distance"])][..];
        assert!(enter_focus_allows_commit(
            true,
            Some(egui::Id::new(("repeat_var_field", "Count"))),
            repeat
        ));
        assert!(!enter_focus_allows_commit(
            true,
            Some(egui::Id::new(("repeat_var_field", "Gap"))),
            repeat
        ));
    }

    /// #1508: the table agrees with `opsigs` about which tools have an Output row — the two
    /// tables describe the same tools and must not drift apart.
    #[test]
    fn output_modes_match_the_tools_with_a_body_choice() {
        let with_output: HashSet<Tool> = all_rows()
            .iter()
            .filter(|r| r.output_modes)
            .map(|r| r.tool)
            .collect();
        let expected: HashSet<Tool> = [
            Tool::Extrude,
            Tool::Revolve,
            Tool::Sweep,
            Tool::Loft,
            Tool::Mirror,
        ]
        .into_iter()
        .collect();
        assert_eq!(with_output, expected);
    }

    /// #1494: the sketch-edit set the runtime uses is the column, not a second list.
    #[test]
    fn is_sketch_edit_tool_reads_the_row() {
        for tool in Tool::ALL {
            let from_row = spaces(tool)
                .iter()
                .any(|&s| row(tool, s).face_click_opens_sketch);
            assert_eq!(
                tool.is_sketch_edit_tool(),
                from_row,
                "{tool:?} disagrees with its row"
            );
        }
    }

    /// #1485/#1508: a heading listed on the row resolves, and a name that isn't there does not.
    #[test]
    fn picker_headings_resolve_from_the_row() {
        let revolve = row(Tool::Revolve, ToolSpace::Solid);
        assert_eq!(revolve.picker_named("Axis"), Some(P::RevolveAxis));
        assert_eq!(revolve.picker_named("axis"), Some(P::RevolveAxis));
        assert_eq!(revolve.picker_named("Profile"), Some(P::RevolveProfile));
        assert_eq!(revolve.picker_named("nope"), None);

        let combine = row(Tool::Combine, ToolSpace::Solid);
        assert_eq!(combine.picker_named("Bodies"), Some(P::CombineA));
        assert_eq!(combine.picker_named("Side A"), Some(P::CombineA));
        assert_eq!(combine.picker_named("Side B"), Some(P::CombineB));

        // Every listed heading is unique per target-or-is-an-alias for the same target.
        for r in all_rows() {
            for p in r.pickers {
                assert!(!p.heading.is_empty(), "{:?} has an empty picker heading", r.tool);
                assert_eq!(r.picker_named(p.heading), Some(p.target));
            }
        }
    }

    /// #1482/#1508: only a placement gizmo owns the next click, so only those rows block a
    /// letter-key tool switch. Value-gizmo and picker tools never do.
    #[test]
    fn only_placement_rows_own_the_next_click() {
        for r in all_rows() {
            match r.gizmo {
                Gizmo::Placement => assert!(
                    matches!(
                        r.draft,
                        Draft::Rect
                            | Draft::Line
                            | Draft::Circle
                            | Draft::Plane
                            | Draft::Shape
                            | Draft::None
                    ),
                    "{:?}/{:?} is a placement tool but names a picker draft",
                    r.tool,
                    r.space
                ),
                Gizmo::Value | Gizmo::None => {}
            }
        }
    }
}
