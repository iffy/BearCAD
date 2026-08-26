//! The per-tool behaviour table (#1508).
//!
//! `opsigs` is the compile-time contract for what an operation *makes*. This is the contract
//! for how its tool *behaves*: where it lives, what its drafts are, what Enter and Esc do,
//! whether the pointer places a value, whether the tool has a New/Add/Cut output, whether
//! it stays armed after commit (#1498), whether SetTool arms an empty draft (#1499),
//! which last-used options the session remembers (#1500), how multi-item clicks
//! toggle membership (#1504), which commit widget the pane shows (#1505), and which
//! tools the current workbench toolbar — and therefore the letter keys — offer (#1506).
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
//!    `spaces` is also the sketch-mode classification (#1494/#1495/#1496): sketch-only
//!    tools start a sketch on a face click, 3D-only tools leave a sketch on `SetTool`,
//!    dual-mode tools survive `BeginSketch`.
//! 3. `tooltable::tests::every_tool_has_a_row` and the per-column walks below then check the
//!    row is coherent (a tool with a draft must clear it on Esc, a `Placement` gizmo must own
//!    the next click, and so on).
//!
//! ## Adding a column
//!
//! Add the field to [`ToolRow`], fill it in for every arm of [`row`] (the compiler lists the
//! arms for you), point the handler that used to `matches!` at the field, and add a walk in
//! `tests`. Columns that exist so far are documented on [`ToolRow`]; later issues still move
//! commit buttons, preview bounds and default amounts here.
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
    /// The View workbench: setting up a cross-section view (#1671).
    View,
}

impl ToolSpace {
    /// The space the app is in right now.
    /// The space the app is in, straight off the workbench it is on (#1686).
    pub fn current(workbench: crate::actions::Workbench) -> Self {
        use crate::actions::Workbench;
        match workbench {
            Workbench::Drawing => Self::Drawing,
            Workbench::View => Self::View,
            Workbench::Sketch => Self::Sketch,
            Workbench::Model => Self::Solid,
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
    /// A dragged handle plus a typed field: click-to-stick, the second click *releases*
    /// (Enter/✓ commits), and typing into the field locks it (`user_edited`). On touch,
    /// finger-up drops the handle — a finger cannot click-to-stick. Picks still go to the
    /// tool's pickers, so a value gizmo never blocks a tool switch.
    Value,
}

impl Gizmo {
    /// #1497: a following value-gizmo handle drops on the second click (mouse) or
    /// finger-up (touch). Placement tools finish on a click because the pointer *is*
    /// the value; this never returns true for them.
    pub fn should_release(
        self,
        following: bool,
        primary_pressed: bool,
        primary_released: bool,
        touch: bool,
    ) -> bool {
        matches!(self, Self::Value)
            && following
            && if touch {
                primary_released
            } else {
                primary_pressed
            }
    }
}

/// Drag writes the live number; only refresh the field text when it is not typed (#1502).
pub fn refresh_gizmo_field_text(
    user_edited: bool,
    text: &mut String,
    formatted: impl Into<String>,
) {
    if !user_edited {
        *text = formatted.into();
    }
}

/// How a tool gathers a multi-item set on click (#1504).
///
/// Chamfer/Fillet used to have three rules (2D add-only, 3D replace/Shift-toggle, picker
/// toggle). One column, one rule: click toggles membership, Shift is not required — the same
/// rule Offset already uses. Both the 2D vertex click and the 3D edge picker read this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultiPick {
    /// Click toggles membership. Shift is not required.
    Toggle,
    /// This row does not collect a multi-item set via this rule.
    None,
}

impl MultiPick {
    /// Apply this row's rule to `set` for one clicked `item`.
    pub fn apply<T: PartialEq>(self, set: &mut Vec<T>, item: T) {
        match self {
            Self::Toggle => crate::element_picker::toggle_picked(set, item),
            Self::None => {}
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Toggle => "toggle",
            Self::None => "none",
        }
    }
}

/// The pane commit control (#1505). One widget so Sketch Mirror/Slice cannot drift
/// back to a grey text button while Offset/Repeat (and every 3D sibling) use the blue
/// confirm in the right column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitWidget {
    /// Blue confirm icon; Enter fires it when nothing else holds the keyboard.
    Primary,
    /// No pane commit button (Select, placement tools, drawing-only tools).
    None,
}

impl CommitWidget {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::None => "none",
        }
    }
}

/// What Esc does (#1484). One rule, so no tool can drift again: the first press empties what
/// the tool has picked and leaves the tool armed, the second returns to Select.
///
/// An armed-but-empty draft is empty — including a post-commit Shape hover ghost that the
/// pointer has sized but the user has not clicked or typed (#1529). First Esc then returns
/// to Select; do not require a no-op clear first.
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
    /// The View workbench's cutting-plane draft (#1745).
    SectionPlane,
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

/// A session last-used option this row remembers (#1500).
///
/// These are "how I last used this tool" — not file state. One store on `AppState`, keyed
/// by tool, seeds each new `creating_*` and is written back on change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pref {
    /// New body / add / cut — Extrude, Revolve, Sweep, Loft, Mirror.
    OutputMode,
    /// Extrude or Revolve symmetric.
    Symmetric,
    /// Chamfer distance / fillet radius.
    Amount,
    BooleanKind,
    KeepB,
    JointKind,
    OffsetDistance,
    OffsetConstruction,
    ShellThickness,
    RepeatAround,
    RepeatCount,
    RepeatSpacing,
    RevolveAngle,
    RevolvePitch,
}

impl Pref {
    pub const fn label(self) -> &'static str {
        match self {
            Self::OutputMode => "output mode",
            Self::Symmetric => "symmetric",
            Self::Amount => "amount",
            Self::BooleanKind => "boolean kind",
            Self::KeepB => "keep b",
            Self::JointKind => "joint kind",
            Self::OffsetDistance => "offset distance",
            Self::OffsetConstruction => "offset construction",
            Self::ShellThickness => "shell thickness",
            Self::RepeatAround => "repeat around",
            Self::RepeatCount => "repeat count",
            Self::RepeatSpacing => "repeat spacing",
            Self::RevolveAngle => "revolve angle",
            Self::RevolvePitch => "revolve pitch",
        }
    }
}

/// The operation a tool re-opens from an Elements-pane row via
/// [`crate::hierarchy::node_editable_operation`] (#546 / #1486). Dual-mode tools have one
/// per space. `None` for tools with a dedicated edit path (Extrude, 3D Chamfer/Fillet,
/// Text, Drawing) or that do not commit a row-editable operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowEdit {
    Boolean,
    Move,
    Mirror,
    SketchMirror,
    Repeat,
    SketchRepeat,
    SketchOffset,
    Slice,
    SketchSlice,
    Shell,
    Revolution,
    Shape,
    Sweep,
    Loft,
    Joint,
    SketchVertexTreatment,
}

impl RowEdit {
    /// A dummy [`crate::hierarchy::HierarchyNode`] of this kind, for the row-entry walk.
    #[cfg(test)]
    pub fn dummy_node(self) -> crate::hierarchy::HierarchyNode {
        use crate::arena::Key;
        use crate::hierarchy::HierarchyNode as H;
        match self {
            Self::Boolean => H::BooleanOp(Key::from_bits(0)),
            Self::Move => H::MoveOp(Key::from_bits(0)),
            Self::Mirror => H::MirrorOp(Key::from_bits(0)),
            Self::SketchMirror => H::SketchMirrorOp(Key::from_bits(0)),
            Self::Repeat => H::RepeatOp(Key::from_bits(0)),
            Self::SketchRepeat => H::SketchRepeatOp(Key::from_bits(0)),
            Self::SketchOffset => H::SketchOffsetOp(Key::from_bits(0)),
            Self::Slice => H::SliceOp(Key::from_bits(0)),
            Self::SketchSlice => H::SketchSliceOp(Key::from_bits(0)),
            Self::Shell => H::ShellOp(Key::from_bits(0)),
            Self::Revolution => H::Revolution(Key::from_bits(0)),
            Self::Shape => H::Shape(Key::from_bits(0)),
            Self::Sweep => H::SweepOp(Key::from_bits(0)),
            Self::Loft => H::Loft(Key::from_bits(0)),
            Self::Joint => H::Joint(Key::from_bits(0)),
            Self::SketchVertexTreatment => H::SketchVertexTreatmentOp(Key::from_bits(0)),
        }
    }
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
    /// Clicking a face with this tool outside a sketch opens a sketch on that face.
    /// Sketch-only tools plus the Sketch tool (#1494). Dual-mode tools keep their 3D click.
    /// Must match [`opens_sketch_on_face_click`].
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
    /// The Elements-pane operation this tool re-opens via double-click / right-click
    /// Edit (#546 / #1486). `None` when the tool has a dedicated edit path or no
    /// row-editable operation.
    pub row_edit: Option<RowEdit>,
    /// After a successful commit, stay on this tool with an empty draft (#1498).
    /// Feature tools (Extrude, Revolve, …) and placement tools (Line/Rect/Circle).
    pub stay_armed: bool,
    /// Arm an empty `creating_*` on `SetTool` so options (Y / Output / amounts) work
    /// before the first pick (#1499).
    pub arm_on_set_tool: bool,
    /// Session last-used options this row remembers (#1500).
    pub prefs: &'static [Pref],
    /// How this tool gathers a multi-item set on click (#1504).
    pub multi_pick: MultiPick,
    /// The pane commit control (#1505). `Primary` for every `commit_on_enter` row.
    pub commit_widget: CommitWidget,
}

/// The spaces a tool has a row in. Exhaustive: a new `Tool` variant does not compile until
/// it is listed.
pub fn spaces(tool: Tool) -> &'static [ToolSpace] {
    use ToolSpace::*;
    match tool {
        // Select works everywhere; the drawing workbench's selection is still Select's.
        Tool::Select => &[Sketch, Solid, Drawing, View],
        // Sketch-only tools. Outside a sketch their first click opens one (#1494).
        Tool::Rectangle
        | Tool::Line
        | Tool::Circle
        | Tool::Constraint
        | Tool::Offset
        | Tool::Project
        | Tool::Text => &[Sketch],
        // Dual-mode: a distinct draft and a distinct pick vocabulary per space.
        // Move's 3D draft is CreatingMove; in a sketch it gizmo-drags the selection (#1496).
        Tool::Dimension | Tool::Chamfer | Tool::Fillet | Tool::Mirror | Tool::Repeat
        | Tool::Slice | Tool::Move => &[Sketch, Solid],
        // 3D-only: SetTool leaves an open sketch (#1495).
        Tool::ConstructionPlane
        | Tool::Sketch
        | Tool::Extrude
        | Tool::Loft
        | Tool::Revolve
        | Tool::Shape
        | Tool::Sweep
        | Tool::Combine
        | Tool::Shell
        | Tool::Joint => &[Solid],
        // Drawing workbench only.
        Tool::DrawingAdd | Tool::DrawingAlign => &[Drawing],
        // View workbench only (#1671).
        Tool::SectionPlane => &[View],
    }
}

/// Whether this tool has a row in `space`.
pub fn has_space(tool: Tool, space: ToolSpace) -> bool {
    spaces(tool).contains(&space)
}

/// Solid only — no sketch mode. `SetTool` leaves an open sketch (#1495).
pub fn is_3d_only(tool: Tool) -> bool {
    matches!(spaces(tool), [ToolSpace::Solid])
}

/// Sketch only — no 3D mode. A face click starts a sketch and the tool survives into it
/// (#1494). The Sketch tool itself is 3D-only (it *is* the face click) and is included
/// separately by [`opens_sketch_on_face_click`].
pub fn is_sketch_only(tool: Tool) -> bool {
    matches!(spaces(tool), [ToolSpace::Sketch])
}

/// A row in both Sketch and Solid (and nowhere else). Stays when a sketch opens;
/// does not start one (#1496). Select lives in every space and is not dual-mode.
pub fn is_dual_mode(tool: Tool) -> bool {
    matches!(
        spaces(tool),
        [ToolSpace::Sketch, ToolSpace::Solid] | [ToolSpace::Solid, ToolSpace::Sketch]
    )
}

/// Clicking a face outside a sketch begins one. Sketch-only tools plus the Sketch tool
/// (#1494) — not dual-mode tools, whose 3D mode owns the click.
pub fn opens_sketch_on_face_click(tool: Tool) -> bool {
    is_sketch_only(tool) || tool == Tool::Sketch
}

/// Has an in-sketch mode, so `BeginSketch` / `enter_sketch` keeps the tool (#1496).
pub fn survives_begin_sketch(tool: Tool) -> bool {
    has_space(tool, ToolSpace::Sketch)
}

/// Tools on the current workbench toolbar, left to right (#1506).
///
/// The letter keys, the toolbar, and `EditDrawing`'s drop-to-Select all read this list
/// so a new drawing tool or a new 3D letter cannot drift.
/// The toolbar for a workbench (#1686). Each bar carries only the tools that mean something
/// there, and switching workbenches drops a tool the new bar doesn't have.
pub fn workbench_tools(workbench: crate::actions::Workbench) -> Vec<Tool> {
    use crate::actions::Workbench;
    match workbench {
        Workbench::Drawing => visible_toolbar_tools(true, false),
        // The View workbench sets up cross-section planes (#1671/#1687).
        Workbench::View => vec![Tool::Select, Tool::SectionPlane],
        Workbench::Model | Workbench::Sketch => visible_toolbar_tools(false, false),
    }
}

pub fn visible_toolbar_tools(drawing: bool, _in_sketch: bool) -> Vec<Tool> {
    if drawing {
        return vec![
            Tool::Select,
            Tool::DrawingAdd,
            Tool::DrawingAlign,
            Tool::Dimension,
            Tool::Text,
        ];
    }
    let mut tools = vec![
        Tool::Select,
        Tool::Sketch,
        Tool::Rectangle,
        Tool::Line,
        Tool::Circle,
        Tool::Shape,
        Tool::Fillet,
        Tool::Chamfer,
        Tool::Offset,
        Tool::Text,
        // Sketch-only like Offset: stays on the bar and clicks a face to start (#1494).
        Tool::Project,
    ];
    tools.extend([
        Tool::ConstructionPlane,
        Tool::Extrude,
        Tool::Sweep,
        Tool::Loft,
        Tool::Revolve,
        Tool::Combine,
        Tool::Move,
        Tool::Mirror,
        Tool::Repeat,
        Tool::Slice,
        Tool::Shell,
        Tool::Joint,
        Tool::Dimension,
        Tool::Constraint,
    ]);
    tools
}

/// Whether a letter key may arm this tool right now (#1506).
///
/// Same list as [`visible_toolbar_tools`] for the current workbench. Project is on the
/// 3D bar outside a sketch (it clicks a face) but its letter only fires inside one.
pub fn letter_shortcut_arms(tool: Tool, drawing: bool, in_sketch: bool) -> bool {
    if tool == Tool::Project && !in_sketch {
        return false;
    }
    visible_toolbar_tools(drawing, in_sketch).contains(&tool)
}

/// Hover/Enter label for the pane commit button (#1505).
///
/// New tool: the tool name. Re-editing: `"Apply changes"`. Plane and Dimension keep
/// the phrasing they already show ("Create plane", "Set dimension").
pub fn commit_label(tool: Tool, editing: bool) -> &'static str {
    if editing {
        return "Apply changes";
    }
    match tool {
        Tool::ConstructionPlane => "Create plane",
        Tool::SectionPlane => "Add cutting plane",
        Tool::Dimension => "Set dimension",
        // The toolbar says Projection; the commit button kept the shorter verb.
        Tool::Project => "Project",
        other => crate::opsigs::tool_label(other),
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
const SECTION_PLANE_FIELDS: &[ToolField] = &[
    ToolField::Named("section_plane_offset_ctx"),
    ToolField::Named("section_plane_roll_ctx"),
    ToolField::Named("section_plane_tilt_v_ctx"),
];

// ── Picker groups (#1485) ───────────────────────────────────────────────────

use crate::context::PickerTarget as P;

const SELECTION: &[ToolPicker] = &[ToolPicker { target: P::Selection, heading: "Selection" }];
const PLANE_PICKERS: &[ToolPicker] = &[ToolPicker { target: P::PlaneAnchor, heading: "Anchor" }];
const SECTION_PLANE_PICKERS: &[ToolPicker] = &[
    ToolPicker { target: P::SectionPlaneAnchor, heading: "Anchor" },
    ToolPicker { target: P::SectionPlaneCutBodies, heading: "Cut bodies" },
    ToolPicker { target: P::SectionPlaneExcludeBodies, heading: "Exclude" },
];
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
    ToolPicker { target: P::MirrorTargets, heading: "Bodies" },
    ToolPicker { target: P::MirrorPlane, heading: "Mirror plane" },
];
const MIRROR_SKETCH_PICKERS: &[ToolPicker] = &[
    ToolPicker { target: P::SketchMirrorShapes, heading: "Shapes" },
    ToolPicker { target: P::SketchMirrorLine, heading: "Mirror line" },
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
    ToolPicker { target: P::SketchOffsetEntities, heading: "Entities" },
    ToolPicker { target: P::Selection, heading: "Selection" },
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

// ── Last-used prefs (#1500) ─────────────────────────────────────────────────

const EXTRUDE_PREFS: &[Pref] = &[Pref::OutputMode, Pref::Symmetric];
const REVOLVE_PREFS: &[Pref] = &[
    Pref::OutputMode,
    Pref::Symmetric,
    Pref::RevolveAngle,
    Pref::RevolvePitch,
];
const OUTPUT_MODE_PREFS: &[Pref] = &[Pref::OutputMode];
const COMBINE_PREFS: &[Pref] = &[Pref::BooleanKind, Pref::KeepB];
const JOINT_PREFS: &[Pref] = &[Pref::JointKind];
const AMOUNT_PREFS: &[Pref] = &[Pref::Amount];
const OFFSET_PREFS: &[Pref] = &[Pref::OffsetDistance, Pref::OffsetConstruction];
const SHELL_PREFS: &[Pref] = &[Pref::ShellThickness];
const REPEAT_PREFS: &[Pref] = &[Pref::RepeatAround, Pref::RepeatCount, Pref::RepeatSpacing];
const SKETCH_REPEAT_PREFS: &[Pref] = &[Pref::RepeatCount, Pref::RepeatSpacing];

impl ToolSpace {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sketch => "sketch",
            Self::Solid => "solid",
            Self::Drawing => "drawing",
            Self::View => "view",
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
        row_edit: None,
        stay_armed: false,
        arm_on_set_tool: false,
        prefs: &[],
        multi_pick: MultiPick::None,
        commit_widget: CommitWidget::None,
    };
    let sketch = space == ToolSpace::Sketch;
    let mut r = match tool {
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
            stay_armed: true,
            ..base
        },
        Tool::Line => ToolRow {
            face_click_opens_sketch: true,
            gizmo: Gizmo::Placement,
            draft: Draft::Line,
            pickers: SELECTION,
            stay_armed: true,
            ..base
        },
        Tool::Circle => ToolRow {
            face_click_opens_sketch: true,
            gizmo: Gizmo::Placement,
            draft: Draft::Circle,
            pickers: SELECTION,
            stay_armed: true,
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
            stay_armed: true,
            ..base
        },
        Tool::Sketch => ToolRow {
            // The Sketch tool *is* the face click: it starts a sketch and then
            // resets to Select (`begin_sketch_from_sketch_tool_resets_to_select`).
            face_click_opens_sketch: true,
            pickers: SELECTION,
            ..base
        },
        Tool::Dimension => ToolRow {
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
            stay_armed: true,
            arm_on_set_tool: true,
            prefs: EXTRUDE_PREFS,
            ..base
        },
        Tool::Chamfer | Tool::Fillet => ToolRow {
            gizmo: Gizmo::Value,
            commit_on_enter: true,
            commit_fields: if sketch { VERTEX_TREATMENT_FIELDS } else { EDGE_TREATMENT_FIELDS },
            draft: if sketch { Draft::VertexTreatment } else { Draft::EdgeTreatment },
            pickers: if sketch { SELECTION } else { EDGE_TREATMENT_PICKERS },
            // 2D chamfer/fillet re-opens through the universal row; 3D has its own
            // `EditEdgeTreatmentOp` path (#531).
            row_edit: if sketch { Some(RowEdit::SketchVertexTreatment) } else { None },
            stay_armed: true,
            prefs: AMOUNT_PREFS,
            multi_pick: MultiPick::Toggle,
            ..base
        },
        Tool::Offset => ToolRow {
            face_click_opens_sketch: true,
            gizmo: Gizmo::Value,
            commit_on_enter: true,
            commit_fields: SKETCH_OFFSET_FIELDS,
            draft: Draft::SketchOffset,
            pickers: OFFSET_PICKERS,
            row_edit: Some(RowEdit::SketchOffset),
            stay_armed: true,
            arm_on_set_tool: true,
            prefs: OFFSET_PREFS,
            multi_pick: MultiPick::Toggle,
            ..base
        },
        Tool::Loft => ToolRow {
            commit_on_enter: true,
            draft: Draft::Loft,
            output_modes: true,
            pickers: LOFT_PICKERS,
            row_edit: Some(RowEdit::Loft),
            stay_armed: true,
            arm_on_set_tool: true,
            prefs: OUTPUT_MODE_PREFS,
            ..base
        },
        Tool::Revolve => ToolRow {
            gizmo: Gizmo::Value,
            commit_on_enter: true,
            commit_fields: REVOLVE_FIELDS,
            draft: Draft::Revolve,
            output_modes: true,
            pickers: REVOLVE_PICKERS,
            row_edit: Some(RowEdit::Revolution),
            stay_armed: true,
            arm_on_set_tool: true,
            prefs: REVOLVE_PREFS,
            ..base
        },
        Tool::Sweep => ToolRow {
            commit_on_enter: true,
            draft: Draft::Sweep,
            output_modes: true,
            pickers: SWEEP_PICKERS,
            row_edit: Some(RowEdit::Sweep),
            stay_armed: true,
            arm_on_set_tool: true,
            prefs: OUTPUT_MODE_PREFS,
            ..base
        },
        Tool::Shape => ToolRow {
            gizmo: Gizmo::Placement,
            commit_on_enter: true,
            commit_fields: SHAPE_FIELDS,
            draft: Draft::Shape,
            row_edit: Some(RowEdit::Shape),
            stay_armed: true,
            arm_on_set_tool: true,
            ..base
        },
        Tool::Combine => ToolRow {
            commit_on_enter: true,
            draft: Draft::Boolean,
            pickers: COMBINE_PICKERS,
            row_edit: Some(RowEdit::Boolean),
            stay_armed: true,
            arm_on_set_tool: true,
            prefs: COMBINE_PREFS,
            ..base
        },
        Tool::Move => ToolRow {
            gizmo: if sketch { Gizmo::None } else { Gizmo::Value },
            commit_on_enter: !sketch,
            commit_fields: if sketch { &[] } else { MOVE_FIELDS },
            // In-sketch Move gizmo-drags the current selection (#306); 3D Move has its own draft.
            draft: if sketch { Draft::Selection } else { Draft::Move },
            pickers: if sketch { SELECTION } else { MOVE_PICKERS },
            row_edit: if sketch { None } else { Some(RowEdit::Move) },
            stay_armed: !sketch,
            arm_on_set_tool: !sketch,
            ..base
        },
        Tool::Mirror => ToolRow {
            commit_on_enter: true,
            draft: if sketch { Draft::SketchMirror } else { Draft::Mirror },
            output_modes: !sketch,
            pickers: if sketch { MIRROR_SKETCH_PICKERS } else { MIRROR_SOLID_PICKERS },
            row_edit: Some(if sketch { RowEdit::SketchMirror } else { RowEdit::Mirror }),
            stay_armed: true,
            arm_on_set_tool: true,
            prefs: if sketch { &[] } else { OUTPUT_MODE_PREFS },
            ..base
        },
        Tool::Repeat => ToolRow {
            gizmo: Gizmo::Value,
            commit_on_enter: true,
            commit_fields: if sketch { SKETCH_REPEAT_FIELDS } else { REPEAT_FIELDS },
            draft: if sketch { Draft::SketchRepeat } else { Draft::Repeat },
            pickers: if sketch { REPEAT_SKETCH_PICKERS } else { REPEAT_SOLID_PICKERS },
            row_edit: Some(if sketch { RowEdit::SketchRepeat } else { RowEdit::Repeat }),
            stay_armed: true,
            arm_on_set_tool: true,
            prefs: if sketch { SKETCH_REPEAT_PREFS } else { REPEAT_PREFS },
            ..base
        },
        // The cutting-plane tool (#1687/#1745): the same picker/gizmo/accept machinery
        // every other tool uses. A click fills the Anchor picker; offset/rotate gizmos
        // follow; Enter / the blue primary button hangs the plane on the open view.
        Tool::SectionPlane => ToolRow {
            gizmo: Gizmo::Value,
            commit_on_enter: true,
            commit_fields: SECTION_PLANE_FIELDS,
            draft: Draft::SectionPlane,
            pickers: SECTION_PLANE_PICKERS,
            stay_armed: true,
            ..base
        },
        Tool::Slice => ToolRow {
            commit_on_enter: true,
            draft: if sketch { Draft::SketchSlice } else { Draft::Slice },
            pickers: if sketch { SLICE_SKETCH_PICKERS } else { SLICE_SOLID_PICKERS },
            row_edit: Some(if sketch { RowEdit::SketchSlice } else { RowEdit::Slice }),
            stay_armed: true,
            arm_on_set_tool: true,
            ..base
        },
        Tool::Shell => ToolRow {
            gizmo: Gizmo::Value,
            commit_on_enter: true,
            commit_fields: SHELL_FIELDS,
            draft: Draft::Shell,
            pickers: SHELL_PICKERS,
            row_edit: Some(RowEdit::Shell),
            stay_armed: true,
            arm_on_set_tool: true,
            prefs: SHELL_PREFS,
            ..base
        },
        Tool::Joint => ToolRow {
            commit_on_enter: true,
            commit_fields: JOINT_FIELDS,
            draft: Draft::Joint,
            pickers: JOINT_PICKERS,
            row_edit: Some(RowEdit::Joint),
            stay_armed: true,
            arm_on_set_tool: true,
            prefs: JOINT_PREFS,
            ..base
        },

        // ── Drawing workbench ───────────────────────────────────────────────
        Tool::DrawingAdd => ToolRow { gizmo: Gizmo::Placement, ..base },
        Tool::DrawingAlign => ToolRow {
            gizmo: Gizmo::Placement,
            pickers: DRAWING_ALIGN_PICKERS,
            ..base
        },
    };
    // #1505: every Enter-to-commit tool uses the same blue primary button. Derived
    // here so a new `commit_on_enter` row cannot ship a grey text `Button`.
    if r.commit_on_enter {
        r.commit_widget = CommitWidget::Primary;
    }
    r
}

impl ToolRow {
    /// The picker a heading names on this row, if any (#1485).
    pub fn picker_named(self, name: &str) -> Option<crate::context::PickerTarget> {
        self.pickers
            .iter()
            .find(|p| p.heading.eq_ignore_ascii_case(name))
            .map(|p| p.target)
    }

    /// The main set this tool works on (#496/#1490). Listed first so a tool switch can
    /// hand the outgoing picks to the incoming filter without a per-tool block.
    pub fn primary_picker(self) -> Option<ToolPicker> {
        self.pickers.first().copied()
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
pub fn stored_value_fields(tool: Tool, space: ToolSpace) -> &'static [&'static str] {
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
        Tool::SectionPlane => &["offset", "roll", "tilt_v"],
        Tool::Extrude => &["distance", "taper"],
        Tool::Chamfer | Tool::Fillet => &["amount"],
        Tool::Offset => &["distance"],
        Tool::Revolve => &["angle", "pitch"],
        Tool::Shape => &["width", "depth", "height", "radius"],
        Tool::Move => match space {
            ToolSpace::Sketch => &[],
            _ => &[
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
        },
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

    /// #1529: every 3D (Solid) tool uses the two-Esc rule. An empty post-commit draft
    /// is empty, so the first Esc returns to Select — Shape's hover ghost is not a pick.
    #[test]
    fn three_d_tools_use_clear_then_select() {
        for r in all_rows() {
            if r.space != ToolSpace::Solid || r.tool == Tool::Select {
                continue;
            }
            assert_eq!(
                r.esc,
                Esc::ClearThenSelect,
                "{:?} must return to Select on Esc when its draft is empty (#1529)",
                r.tool
            );
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

    /// #1494/#1495/#1496: `spaces()` is the one classification. Every tool is sketch-only,
    /// dual-mode, 3D-only, drawing-only, or Select (all three). A new tool cannot skip it.
    #[test]
    fn spaces_partition_every_tool() {
        for tool in Tool::ALL {
            let n = spaces(tool).len();
            assert!(n >= 1, "{tool:?} claims no space");
            let flags = (
                is_sketch_only(tool),
                is_dual_mode(tool),
                is_3d_only(tool),
                matches!(spaces(tool), [ToolSpace::Drawing]),
                matches!(spaces(tool), [ToolSpace::View]),
                tool == Tool::Select,
            );
            let kinds = [flags.0, flags.1, flags.2, flags.3, flags.4, flags.5]
                .into_iter()
                .filter(|b| *b)
                .count();
            assert_eq!(
                kinds, 1,
                "{tool:?} must be exactly one of sketch-only / dual / 3D-only / drawing / view / Select, got {flags:?}"
            );
        }
        let sketch_only: HashSet<Tool> = Tool::ALL.iter().copied().filter(|t| is_sketch_only(*t)).collect();
        assert_eq!(
            sketch_only,
            [
                Tool::Rectangle,
                Tool::Line,
                Tool::Circle,
                Tool::Constraint,
                Tool::Offset,
                Tool::Project,
                Tool::Text,
            ]
            .into_iter()
            .collect()
        );
        let dual: HashSet<Tool> = Tool::ALL.iter().copied().filter(|t| is_dual_mode(*t)).collect();
        assert_eq!(
            dual,
            [
                Tool::Dimension,
                Tool::Chamfer,
                Tool::Fillet,
                Tool::Mirror,
                Tool::Repeat,
                Tool::Slice,
                Tool::Move,
            ]
            .into_iter()
            .collect()
        );
        let three_d: HashSet<Tool> = Tool::ALL.iter().copied().filter(|t| is_3d_only(*t)).collect();
        assert_eq!(
            three_d,
            [
                Tool::ConstructionPlane,
                Tool::Sketch,
                Tool::Extrude,
                Tool::Loft,
                Tool::Revolve,
                Tool::Shape,
                Tool::Sweep,
                Tool::Combine,
                Tool::Shell,
                Tool::Joint,
            ]
            .into_iter()
            .collect()
        );
    }

    /// #1494: face-click BeginSketch is sketch-only tools plus Sketch — not dual-mode
    /// (their 3D mode owns the click) and not a second hand-written list.
    #[test]
    fn face_click_opens_sketch_is_sketch_only_plus_sketch_tool() {
        for tool in Tool::ALL {
            let from_row = spaces(tool)
                .iter()
                .any(|&s| row(tool, s).face_click_opens_sketch);
            assert_eq!(
                from_row,
                opens_sketch_on_face_click(tool),
                "{tool:?} face_click column disagrees with the classification"
            );
            assert_eq!(
                opens_sketch_on_face_click(tool),
                is_sketch_only(tool) || tool == Tool::Sketch,
                "{tool:?}"
            );
        }
    }

    /// #1490: the first listed picker is the main set a tool switch seeds.
    #[test]
    fn primary_picker_is_the_first_listed() {
        for r in all_rows() {
            assert_eq!(r.primary_picker(), r.pickers.first().copied());
        }
        assert_eq!(
            row(Tool::Extrude, ToolSpace::Solid).primary_picker().map(|p| p.target),
            Some(P::ExtrudeProfile)
        );
        assert_eq!(
            row(Tool::Revolve, ToolSpace::Solid).primary_picker().map(|p| p.target),
            Some(P::RevolveProfile)
        );
        assert_eq!(
            row(Tool::Sweep, ToolSpace::Solid).primary_picker().map(|p| p.target),
            Some(P::SweepProfile)
        );
        assert_eq!(
            row(Tool::Repeat, ToolSpace::Solid).primary_picker().map(|p| p.target),
            Some(P::RepeatTargets)
        );
    }

    /// #1496: surviving BeginSketch is "has a Sketch space", not the face-click column.
    #[test]
    fn is_sketch_edit_tool_is_has_sketch_space() {
        for tool in Tool::ALL {
            assert_eq!(
                tool.is_sketch_edit_tool(),
                survives_begin_sketch(tool),
                "{tool:?} disagrees with survives_begin_sketch"
            );
            assert_eq!(
                tool.is_sketch_edit_tool(),
                has_space(tool, ToolSpace::Sketch),
                "{tool:?} is_sketch_edit_tool should be 'has a Sketch space'"
            );
        }
        // Dual-mode tools the old face-click list dropped.
        for tool in [Tool::Move, Tool::Mirror, Tool::Repeat, Tool::Slice] {
            assert!(
                tool.is_sketch_edit_tool(),
                "{tool:?} has an in-sketch mode and must survive BeginSketch"
            );
            assert!(!opens_sketch_on_face_click(tool), "{tool:?} is dual-mode");
        }
        // Sketch starts a sketch but does not survive into it.
        assert!(opens_sketch_on_face_click(Tool::Sketch));
        assert!(!Tool::Sketch.is_sketch_edit_tool());
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

    /// #1504: Chamfer/Fillet share Offset's toggle — one rule for both spaces, so the
    /// 2D click path and the 3D picker cannot drift again.
    #[test]
    fn chamfer_fillet_and_offset_toggle() {
        for tool in [Tool::Chamfer, Tool::Fillet] {
            for &space in spaces(tool) {
                assert_eq!(
                    row(tool, space).multi_pick,
                    MultiPick::Toggle,
                    "{tool:?}/{space:?} must toggle"
                );
            }
        }
        assert_eq!(
            row(Tool::Offset, ToolSpace::Sketch).multi_pick,
            MultiPick::Toggle
        );
    }

    /// #1486: a tool-table `row_edit` is the Elements-pane double-click / right-click
    /// Edit path. Walking every row so a new tool cannot claim a re-edit without a
    /// `node_editable_operation` arm.
    #[test]
    fn row_edit_column_has_a_hierarchy_row() {
        for r in all_rows() {
            let Some(kind) = r.row_edit else { continue };
            assert!(
                crate::hierarchy::node_editable_operation(kind.dummy_node()).is_some(),
                "{:?}/{:?} lists row_edit {:?} but the Elements pane has no Edit",
                r.tool,
                r.space,
                kind
            );
        }
    }

    /// #1497: one pointer rule for every value gizmo — click-to-stick, second click
    /// releases (does not commit), touch lift drops. Placement tools stay click-to-finish.
    #[test]
    fn value_gizmo_second_click_releases() {
        for r in all_rows() {
            match r.gizmo {
                Gizmo::Value => {
                    assert!(
                        r.gizmo.should_release(true, true, false, false),
                        "{:?}/{:?} value gizmo must drop on the second click",
                        r.tool,
                        r.space
                    );
                    assert!(
                        !r.gizmo.should_release(true, false, true, false),
                        "{:?}/{:?} mouse lift must not drop a following value gizmo",
                        r.tool,
                        r.space
                    );
                    assert!(
                        r.gizmo.should_release(true, false, true, true),
                        "{:?}/{:?} touch lift must drop a following value gizmo",
                        r.tool,
                        r.space
                    );
                    assert!(
                        !r.gizmo.should_release(false, true, false, false),
                        "{:?}/{:?} grab click is not a release",
                        r.tool,
                        r.space
                    );
                }
                Gizmo::Placement | Gizmo::None => {
                    assert!(
                        !r.gizmo.should_release(true, true, true, true),
                        "{:?}/{:?} is not a value gizmo and must not use the release rule",
                        r.tool,
                        r.space
                    );
                }
            }
        }
    }

    /// #1498: feature tools and placement drawing tools stay armed after commit.
    /// Select / Sketch / drawing-only tools have nothing to stay on.
    #[test]
    fn stay_armed_column_covers_feature_and_placement_tools() {
        let stay: HashSet<Tool> = all_rows()
            .iter()
            .filter(|r| r.stay_armed)
            .map(|r| r.tool)
            .collect();
        for tool in [
            Tool::Extrude,
            Tool::Revolve,
            Tool::Sweep,
            Tool::Loft,
            Tool::Shape,
            Tool::Combine,
            Tool::Move,
            Tool::Joint,
            Tool::Mirror,
            Tool::Repeat,
            Tool::Slice,
            Tool::Shell,
            Tool::Offset,
            Tool::ConstructionPlane,
            Tool::SectionPlane,
            Tool::Chamfer,
            Tool::Fillet,
            Tool::Rectangle,
            Tool::Line,
            Tool::Circle,
        ] {
            assert!(stay.contains(&tool), "{tool:?} must stay armed after commit (#1498)");
        }
        for tool in [Tool::Select, Tool::Sketch, Tool::DrawingAdd, Tool::DrawingAlign] {
            assert!(!stay.contains(&tool), "{tool:?} is not a stay-armed feature tool");
        }
        for r in all_rows() {
            if r.stay_armed && r.space == ToolSpace::Solid && r.commit_on_enter {
                assert!(
                    r.draft != Draft::None,
                    "{:?}/{:?} stays armed but names no draft to empty",
                    r.tool,
                    r.space
                );
            }
        }
    }

    /// #1499: every Output-row tool, plus the pane-option tools, arm an empty draft on SetTool.
    #[test]
    fn arm_on_set_tool_covers_output_row_and_pane_option_tools() {
        for r in all_rows() {
            if r.output_modes {
                assert!(
                    r.arm_on_set_tool,
                    "{:?}/{:?} has an Output row but does not arm on SetTool (#1499)",
                    r.tool,
                    r.space
                );
            }
        }
        for tool in [
            Tool::Extrude,
            Tool::Revolve,
            Tool::Sweep,
            Tool::Loft,
            Tool::Mirror,
            Tool::Combine,
            Tool::Move,
            Tool::Joint,
            Tool::Repeat,
            Tool::Slice,
            Tool::Shell,
            Tool::Shape,
            Tool::Offset,
        ] {
            let armed = spaces(tool).iter().any(|&s| row(tool, s).arm_on_set_tool);
            assert!(armed, "{tool:?} must arm creating-state on SetTool (#1499)");
        }
    }

    /// #1500: the prefs column is the one list of last-used options. Output-row tools
    /// remember OutputMode; tools with a typed amount remember that amount.
    #[test]
    fn prefs_column_lists_last_used_options() {
        for r in all_rows() {
            if r.output_modes {
                assert!(
                    r.prefs.contains(&Pref::OutputMode),
                    "{:?}/{:?} has an Output row but does not remember it (#1500)",
                    r.tool,
                    r.space
                );
            }
            if r.prefs.contains(&Pref::OutputMode) {
                assert!(
                    r.output_modes,
                    "{:?}/{:?} remembers OutputMode but has no Output row",
                    r.tool,
                    r.space
                );
            }
        }
        assert!(row(Tool::Extrude, ToolSpace::Solid).prefs.contains(&Pref::Symmetric));
        assert!(row(Tool::Revolve, ToolSpace::Solid).prefs.contains(&Pref::Symmetric));
        assert!(row(Tool::Revolve, ToolSpace::Solid).prefs.contains(&Pref::RevolveAngle));
        assert!(row(Tool::Combine, ToolSpace::Solid).prefs.contains(&Pref::BooleanKind));
        assert!(row(Tool::Joint, ToolSpace::Solid).prefs.contains(&Pref::JointKind));
        assert!(row(Tool::Chamfer, ToolSpace::Solid).prefs.contains(&Pref::Amount));
        assert!(row(Tool::Fillet, ToolSpace::Sketch).prefs.contains(&Pref::Amount));
        assert!(row(Tool::Offset, ToolSpace::Sketch).prefs.contains(&Pref::OffsetDistance));
        assert!(row(Tool::Shell, ToolSpace::Solid).prefs.contains(&Pref::ShellThickness));
        assert!(row(Tool::Repeat, ToolSpace::Solid).prefs.contains(&Pref::RepeatCount));
        for r in all_rows() {
            for p in r.prefs {
                assert!(!p.label().is_empty(), "{:?} pref has an empty label", r.tool);
            }
        }
    }

    /// #1502: a typed field is not rewritten by a later live number.
    #[test]
    fn typed_gizmo_field_stays_put() {
        let mut text = "12".to_string();
        refresh_gizmo_field_text(true, &mut text, "20");
        assert_eq!(text, "12");
        refresh_gizmo_field_text(false, &mut text, "20");
        assert_eq!(text, "20");
    }

    /// #1745: the cutting-plane tool is a row like every other tool — an element picker, a
    /// value gizmo (offset/rotate), Enter-to-commit (the blue primary button), and a draft
    /// Esc empties. A one-off click-to-place path cannot grow those later.
    #[test]
    fn cutting_plane_uses_shared_picker_gizmo_and_commit() {
        let r = row(Tool::SectionPlane, ToolSpace::View);
        assert!(
            !r.pickers.is_empty(),
            "Cutting plane must register an element picker so hover, the Exploder, and scripts see it"
        );
        assert_eq!(
            r.pickers[0].heading, "Anchor",
            "the primary picker is the cutting-plane anchor"
        );
        assert!(r.commit_on_enter, "Enter commits, which also gives the blue primary button");
        assert_eq!(r.commit_widget, CommitWidget::Primary);
        assert_eq!(
            r.gizmo,
            Gizmo::Value,
            "offset and rotate are value gizmos, not a click-to-place"
        );
        assert_ne!(
            r.draft,
            Draft::None,
            "Esc must have a draft to empty instead of throwing the tool away"
        );
        assert!(r.stay_armed, "after accept, pick another plane");
        assert!(
            !r.commit_fields.is_empty(),
            "offset/rotate fields must commit on Enter"
        );
    }

    /// #1505: every tool that commits on Enter uses the shared blue primary button, so
    /// Sketch Mirror/Slice cannot drift back to a grey text `Button`.
    #[test]
    fn commit_on_enter_uses_the_primary_button() {
        for r in all_rows() {
            let expected = if r.commit_on_enter {
                CommitWidget::Primary
            } else {
                CommitWidget::None
            };
            assert_eq!(
                r.commit_widget, expected,
                "{:?}/{:?} commit_widget disagrees with commit_on_enter",
                r.tool, r.space
            );
        }
        assert_eq!(
            row(Tool::Mirror, ToolSpace::Sketch).commit_widget,
            CommitWidget::Primary
        );
        assert_eq!(
            row(Tool::Slice, ToolSpace::Sketch).commit_widget,
            CommitWidget::Primary
        );
        assert_eq!(
            row(Tool::Offset, ToolSpace::Sketch).commit_widget,
            CommitWidget::Primary
        );
        assert_eq!(
            row(Tool::Repeat, ToolSpace::Sketch).commit_widget,
            CommitWidget::Primary
        );
    }

    /// #1505: Combine's new-tool label is the tool name, not "Create"; re-edit is
    /// "Apply changes" for every committing tool.
    #[test]
    fn commit_label_is_the_tool_name_or_apply_changes() {
        assert_eq!(commit_label(Tool::Combine, false), "Combine");
        assert_eq!(commit_label(Tool::Combine, true), "Apply changes");
        assert_eq!(commit_label(Tool::Mirror, false), "Mirror");
        assert_eq!(commit_label(Tool::Slice, false), "Slice");
        assert_eq!(commit_label(Tool::Offset, false), "Offset");
        assert_eq!(commit_label(Tool::Repeat, true), "Apply changes");
        assert_eq!(commit_label(Tool::Extrude, false), "Extrude");
        assert_eq!(commit_label(Tool::Extrude, true), "Apply changes");
        assert_eq!(commit_label(Tool::Shape, false), "Shape");
        assert_eq!(commit_label(Tool::ConstructionPlane, false), "Create plane");
        assert_eq!(commit_label(Tool::Dimension, false), "Set dimension");
        assert_eq!(commit_label(Tool::Project, false), "Project");
        assert_ne!(commit_label(Tool::Combine, false), "Create");
        assert_ne!(commit_label(Tool::Shape, false), "Create");
    }

    /// #1506: letter keys only arm a tool the current workbench toolbar would show.
    #[test]
    fn letter_shortcuts_only_arm_visible_toolbar_tools() {
        for tool in [
            Tool::Extrude,
            Tool::Rectangle,
            Tool::Line,
            Tool::Sketch,
            Tool::Move,
            Tool::Chamfer,
            Tool::Fillet,
            Tool::Constraint,
            Tool::Circle,
            Tool::Shape,
            Tool::Joint,
        ] {
            assert!(
                !letter_shortcut_arms(tool, true, false),
                "{tool:?} must not arm from a letter while a drawing is open"
            );
            assert!(
                visible_toolbar_tools(false, false).contains(&tool),
                "{tool:?} should still sit on the 3D toolbar"
            );
        }
        for tool in [Tool::Select, Tool::Dimension, Tool::Text, Tool::DrawingAdd, Tool::DrawingAlign]
        {
            assert!(
                visible_toolbar_tools(true, false).contains(&tool),
                "{tool:?} belongs on the drawing toolbar"
            );
        }
        assert!(letter_shortcut_arms(Tool::Dimension, true, false));
        assert!(letter_shortcut_arms(Tool::Text, true, false));
        assert!(letter_shortcut_arms(Tool::Select, true, false));
        assert!(!letter_shortcut_arms(Tool::Project, false, false));
        assert!(letter_shortcut_arms(Tool::Project, false, true));
        assert!(letter_shortcut_arms(Tool::Extrude, false, false));
        assert!(letter_shortcut_arms(Tool::Rectangle, false, true));
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
