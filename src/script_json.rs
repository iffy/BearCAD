//! JSON command dispatcher for the web build's Lua scripting (todoer #179).
//!
//! On desktop, `mlua` closures ([`crate::lua_script`]) implement the `bearcad.*` API
//! directly. The browser can't compile mlua's bundled Lua C for `wasm32-unknown-unknown`,
//! so the web build runs the interpreter as a *second* Emscripten module (mirroring the
//! OCCT kernel) that forwards every `bearcad.*` call to a single hook,
//! `bearcad_call(name, json_args) -> json`. This module is the Rust side of that hook: it
//! turns a command name plus JSON arguments into the very same [`Instruction`] the native
//! closures build, so both frontends drive the identical Instruction/Action layer.
//!
//! The translation is deliberately data-only (name + args → `Instruction`); executing the
//! instruction and any live-state–dependent bookkeeping (auto-starting a sketch, returning
//! element handles) stays with the caller, exactly as the mlua closures do it. Keeping this
//! layer pure makes it testable off-browser: every command here is checked against the
//! `Instruction` its `bearcad.*` closure produces for the same inputs.

use crate::actions::{DimLabelAxis, RectAxis, RevolveBodyChoice};
use crate::construction::PlaneDim;
use crate::geometric_constraints::GeometricConstraintType;
use crate::model::{
    BooleanOpKind, ConstraintKind, DistanceTarget, Document, DrawingOrientation, ExtrudeFace,
    FaceId, RepeatMode, RevolveAxis,
};
use crate::script::Instruction;
use serde_json::{json, Map, Value};

/// Commands that draw into a sketch and, like their mlua closures, begin one on the ground
/// (XY) construction plane when no sketch is active. The caller checks live state and
/// prepends [`Instruction::BeginSketch`] before executing the returned instruction.
pub fn opens_sketch_when_none_active(name: &str) -> bool {
    matches!(name, "rect" | "line" | "circle")
}

/// Translate one `bearcad.<name>{ ...args }` call into its [`Instruction`]. `args` is the
/// JSON object of named arguments (an empty object for no-arg calls). Returns a
/// human-readable message for an unknown command or a bad argument, which the web runner
/// surfaces the way native mlua raises a Lua error.
///
/// Coverage is every `bearcad.*` verb whose `Instruction` is a pure function of its
/// arguments: the document/IO actions, tool actions, 2D primitives, and the declarative
/// modeling ops (revolve, loft, booleans, move, repeat, slice, and their `edit_*` forms).
///
/// `extrude`/`extrude_face` are intentionally absent: their closures read the live document
/// to infer the owning sketch (`extrude_face_sketch`) before building the `Instruction`, so
/// they can't be a pure `(name, args)` function — they belong to the stateful dispatch path
/// alongside the query getters. Likewise the read-back getters (`get`/`count`/`selection`/
/// `body_stats`/`sketch_dof`) return JSON data rather than an `Instruction`.
pub fn instruction_from_json(name: &str, args: &Value) -> Result<Instruction, String> {
    let o = as_object(args)?;
    match name {
        "new" => Ok(Instruction::New),
        "clear" => Ok(Instruction::Clear),
        "undo" => Ok(Instruction::Undo),
        "quit" => Ok(Instruction::Quit),
        "exit_sketch" => Ok(Instruction::ExitSketch),
        "tool" => {
            let name = req_str(o, "name", "tool")?;
            let tool = crate::actions::Tool::from_name(&name)
                .ok_or_else(|| format!("unknown tool '{name}'"))?;
            Ok(Instruction::Tool(tool))
        }
        "open_sketch" => {
            let sketch = req_usize(o, "sketch", "open_sketch")?;
            Ok(Instruction::OpenSketch { sketch })
        }
        "begin_sketch" => {
            let kind = req_str(o, "kind", "begin_sketch")?;
            let index = req_usize(o, "index", "begin_sketch")?;
            let face = FaceId::from_script(&kind, index)
                .ok_or_else(|| format!("unknown sketch face kind '{kind}'"))?;
            Ok(Instruction::BeginSketch { face })
        }
        "plane" => Ok(Instruction::CreatePlane {
            offset: opt_f32(o, "offset")?.unwrap_or(0.0),
            from: opt_usize(o, "from")?.unwrap_or(0),
        }),
        "rect" => Ok(Instruction::CreateRect {
            x: opt_f32(o, "x")?.unwrap_or(0.0),
            y: opt_f32(o, "y")?.unwrap_or(0.0),
            width: req_f32(o, "width", "rect")?,
            height: req_f32(o, "height", "rect")?,
        }),
        "circle" => {
            let cx = opt_f32(o, "x")?.unwrap_or(0.0);
            let cy = opt_f32(o, "y")?.unwrap_or(0.0);
            // Same precedence as the mlua closure: `r`, then `radius`, then `diameter`.
            let r = if let Some(r) = opt_f32(o, "r")? {
                r
            } else if let Some(radius) = opt_f32(o, "radius")? {
                radius
            } else if let Some(diameter) = opt_f32(o, "diameter")? {
                diameter * 0.5
            } else {
                return Err("circle requires a size: one of `r`, `radius`, or `diameter`".into());
            };
            Ok(Instruction::CreateCircle { cx, cy, r })
        }
        "line" => {
            let x0 = opt_f32(o, "x")?.unwrap_or(0.0);
            let y0 = opt_f32(o, "y")?.unwrap_or(0.0);
            let (x1, y1) = match (opt_f32(o, "x1")?, opt_f32(o, "y1")?) {
                (Some(x1), Some(y1)) => (x1, y1),
                _ => {
                    let length = req_f32(o, "length", "line")?;
                    let a = opt_f32(o, "angle")?.unwrap_or(0.0).to_radians();
                    (x0 + length * a.cos(), y0 + length * a.sin())
                }
            };
            let bezier = parse_bezier(o)?;
            let dimension = parse_dimension(o, x0, y0, x1, y1)?;
            Ok(Instruction::CreateLine { x0, y0, x1, y1, bezier, dimension })
        }

        // ----- File / import-export (mirrors the desktop closures, which take positional
        // path strings; over JSON every argument is named). -----
        "open" => Ok(Instruction::Open(req_str(o, "path", "open")?)),
        "save" => Ok(Instruction::Save(opt_str(o, "path")?)),
        "export_stl" => Ok(Instruction::ExportStl {
            path: req_str(o, "path", "export_stl")?,
            body: opt_str(o, "body")?,
        }),
        "export_step" => Ok(Instruction::ExportStep {
            path: req_str(o, "path", "export_step")?,
            body: opt_str(o, "body")?,
        }),
        "import_stl" => Ok(Instruction::ImportStl { path: req_str(o, "path", "import_stl")? }),
        "import_step" => Ok(Instruction::ImportStep { path: req_str(o, "path", "import_step")? }),
        "import_image" => Ok(Instruction::ImportImage {
            path: req_str(o, "path", "import_image")?,
            plane: opt_usize(o, "plane")?,
        }),
        "calibrate_image" => Ok(Instruction::CalibrateImage {
            image: req_usize(o, "image", "calibrate_image")?,
            a: xy_pair(o, "from")?,
            b: xy_pair(o, "to")?,
            length: req_f32(o, "length", "calibrate_image")?,
        }),

        // ----- Declarative 3D modeling ops. -----
        "revolve" => {
            let faces = collect_profile_faces(o, false)?;
            if faces.is_empty() {
                return Err("revolve requires a `circle`/`circles`/`polygon` face".into());
            }
            let axis = match o.get("axis") {
                None | Some(Value::Null) => {
                    return Err("revolve requires `axis` (\"x\"|\"y\"|\"z\" or {line = i})".into())
                }
                Some(v) => revolve_axis_from_value(v)?,
            };
            let angle_deg = opt_f32(o, "angle")?.unwrap_or(360.0);
            let symmetric = opt_bool(o, "symmetric")?.unwrap_or(false);
            let bodies = usize_list(o, "bodies")?;
            // Same mapping as the closure: "add"→AddTouching, "cut"→Cut, else NewBody.
            let body = match opt_str(o, "body")?.as_deref() {
                Some("add") => RevolveBodyChoice::AddTouching,
                Some("cut") => RevolveBodyChoice::Cut,
                _ => RevolveBodyChoice::NewBody,
            };
            Ok(Instruction::Revolve { faces, axis, angle_deg, symmetric, body, bodies })
        }
        "loft" => {
            let faces = collect_profile_faces(o, true)?;
            if faces.len() < 2 {
                return Err("loft requires at least two sections (`circles`/`polygons`)".into());
            }
            Ok(Instruction::Loft { faces })
        }
        "combine" => {
            let (kind, a, b, keep_b) = boolean_op_args(o)?;
            Ok(Instruction::CreateBooleanOp { kind, a, b, keep_b })
        }
        "edit_boolean" => {
            let op = req_usize(o, "index", "edit_boolean")?;
            let (kind, a, b, keep_b) = boolean_op_args(o)?;
            Ok(Instruction::EditBooleanOp { op, kind, a, b, keep_b })
        }
        "move_bodies" => {
            let (targets, tx, ty, tz, axis, angle) = move_op_args(o)?;
            Ok(Instruction::CreateMoveOp { targets, tx, ty, tz, axis, angle })
        }
        "edit_move" => {
            let op = req_usize(o, "index", "edit_move")?;
            let (targets, tx, ty, tz, axis, angle) = move_op_args(o)?;
            Ok(Instruction::EditMoveOp { op, targets, tx, ty, tz, axis, angle })
        }
        "repeat_bodies" => {
            let (targets, axis, mode, count, spacing, length) = repeat_op_args(o)?;
            Ok(Instruction::CreateRepeatOp { targets, axis, mode, count, spacing, length })
        }
        "edit_repeat" => {
            let op = req_usize(o, "index", "edit_repeat")?;
            let (targets, axis, mode, count, spacing, length) = repeat_op_args(o)?;
            Ok(Instruction::EditRepeatOp { op, targets, axis, mode, count, spacing, length })
        }
        "slice" => {
            let (targets, cutters, extend_infinite) = slice_op_args(o)?;
            Ok(Instruction::CreateSliceOp { targets, cutters, extend_infinite })
        }
        "edit_slice" => {
            let op = req_usize(o, "index", "edit_slice")?;
            let (targets, cutters, extend_infinite) = slice_op_args(o)?;
            Ok(Instruction::EditSliceOp { op, targets, cutters, extend_infinite })
        }

        // ----- Sketch dimensions & constraints. -----
        "set_dim" => {
            let axis = req_str(o, "axis", "set_dim")?;
            let value = req_expr(o, "value", "set_dim")?;
            // Same dispatch order as the closure: rect axis, then line length, circle
            // diameter, plane offset, plane angle.
            if let Some(axis) = RectAxis::from_name(&axis) {
                Ok(Instruction::SetDim { axis, value })
            } else if axis.eq_ignore_ascii_case("length") || axis.eq_ignore_ascii_case("len") {
                Ok(Instruction::SetLineLength { value })
            } else if axis.eq_ignore_ascii_case("diameter") || axis.eq_ignore_ascii_case("diam") {
                Ok(Instruction::SetCircleDiameter { value })
            } else if axis.eq_ignore_ascii_case("offset") {
                Ok(Instruction::SetPlaneOffset { value })
            } else if axis.eq_ignore_ascii_case("angle") {
                Ok(Instruction::SetPlaneAngle { value })
            } else {
                Err(format!("unknown dimension '{axis}'"))
            }
        }
        "focus_dim" => {
            let axis = req_str(o, "axis", "focus_dim")?;
            if let Some(axis) = RectAxis::from_name(&axis) {
                Ok(Instruction::FocusDim(axis))
            } else if axis.eq_ignore_ascii_case("length") {
                Ok(Instruction::FocusLineLength)
            } else if axis.eq_ignore_ascii_case("diameter") {
                Ok(Instruction::FocusCircleDiameter)
            } else if let Some(dim) = PlaneDim::from_name(&axis) {
                Ok(Instruction::FocusPlaneDim(dim))
            } else {
                Err(format!("unknown dimension '{axis}'"))
            }
        }
        "edit_dim" => {
            let axis = req_str(o, "axis", "edit_dim")?;
            let axis = DimLabelAxis::from_name(&axis)
                .ok_or_else(|| format!("unknown dimension '{axis}'"))?;
            Ok(Instruction::BeginEditCommittedDim { axis })
        }
        "commit_dim" => Ok(Instruction::CommitCommittedDim),
        "set_dim_label_offset" => {
            let axis = req_str(o, "axis", "set_dim_label_offset")?;
            let axis = DimLabelAxis::from_name(&axis)
                .ok_or_else(|| format!("unknown dimension '{axis}'"))?;
            Ok(Instruction::SetDimLabelOffset {
                axis,
                offset: req_f32(o, "offset", "set_dim_label_offset")?,
            })
        }
        "add_constraint" => {
            let target = o
                .get("target")
                .ok_or("add_constraint requires a `target`")?;
            Ok(Instruction::AddDistanceConstraint {
                target: distance_target_from_json(target)?,
                expression: req_expr(o, "expression", "add_constraint")?,
            })
        }
        "add_angle_constraint" => {
            // `value` (an expression) or `angle` (a number) gives the angle; `sign` picks the
            // wedge (default +1).
            let expression = match (o.get("value"), o.get("angle")) {
                (Some(v), _) if !v.is_null() => value_to_expr(v, "value")?,
                (_, Some(a)) if !a.is_null() => value_to_expr(a, "angle")?,
                _ => return Err("add_angle_constraint requires `value`".into()),
            };
            Ok(Instruction::AddAngleConstraint {
                line_a: req_usize(o, "a", "add_angle_constraint")?,
                line_b: req_usize(o, "b", "add_angle_constraint")?,
                rotation_sign: opt_i8(o, "sign")?.unwrap_or(1),
                expression,
            })
        }
        "add_geometric_constraint" => {
            let name = req_str(o, "name", "add_geometric_constraint")?;
            let kind = geometric_constraint_from_name(&name)
                .ok_or_else(|| format!("unknown geometric constraint '{name}'"))?;
            Ok(Instruction::AddGeometricConstraint(kind))
        }
        "constraint_shortcut" => {
            let key = req_str(o, "key", "constraint_shortcut")?;
            let ch = key
                .chars()
                .next()
                .ok_or("constraint_shortcut requires a key")?;
            Ok(Instruction::ApplyConstraintShortcut(ch))
        }

        // ----- Construction-plane editing, naming, construction flag, deletion. -----
        "edit_plane" => Ok(Instruction::BeginEditConstructionPlane {
            index: req_usize(o, "index", "edit_plane")?,
        }),
        "commit_plane" => Ok(Instruction::CommitConstructionPlane),
        "focus_name" => Ok(Instruction::FocusElementName),
        "apply_construction" => Ok(Instruction::ApplyConstruction {
            construction: req_bool_flag(o, "construction", "apply_construction")?,
        }),
        "toggle_construction" => Ok(Instruction::ToggleConstruction),
        "delete_selection" => Ok(Instruction::DeleteSelection),

        // ----- Technical drawings (#180). `drawing` returns the new index on the desktop,
        // but the Instruction it builds is a pure `CreateDrawing`; the handle return, like
        // every other element handle, is the caller's job. -----
        "drawing" => Ok(Instruction::CreateDrawing { name: opt_str(o, "name")? }),
        "drawing_view" => {
            let orientation = match opt_str(o, "orientation")? {
                Some(name) => DrawingOrientation::from_name(&name)
                    .ok_or_else(|| format!("unknown drawing orientation '{name}'"))?,
                None => DrawingOrientation::default(),
            };
            Ok(Instruction::AddDrawingView {
                drawing: req_usize(o, "drawing", "drawing_view")?,
                body: req_usize(o, "body", "drawing_view")?,
                orientation,
            })
        }
        "export_drawing_svg" => Ok(Instruction::ExportDrawingSvg {
            drawing: req_usize(o, "drawing", "export_drawing_svg")?,
            path: req_str(o, "path", "export_drawing_svg")?,
        }),
        "drawing_dimension" => Ok(Instruction::ToggleDrawingDimension {
            drawing: req_usize(o, "drawing", "drawing_dimension")?,
            view: req_usize(o, "view", "drawing_dimension")?,
            a: xyz(o, "a")?,
            b: xyz(o, "b")?,
        }),
        "drawing_angle" => {
            let edge = |key: &str| -> Result<((f32, f32, f32), (f32, f32, f32)), String> {
                let t = o
                    .get(key)
                    .and_then(Value::as_object)
                    .ok_or_else(|| format!("drawing_angle `{key}` must be an edge object"))?;
                Ok((xyz(t, "a")?, xyz(t, "b")?))
            };
            Ok(Instruction::ToggleDrawingAngle {
                drawing: req_usize(o, "drawing", "drawing_angle")?,
                view: req_usize(o, "view", "drawing_angle")?,
                edge1: edge("edge1")?,
                edge2: edge("edge2")?,
            })
        }

        other => Err(format!("unknown command '{other}'")),
    }
}

/// A world-space `[x, y, z]` point (drawing dimension/angle endpoints).
fn xyz(o: &Map<String, Value>, key: &str) -> Result<(f32, f32, f32), String> {
    let arr = o
        .get(key)
        .and_then(Value::as_array)
        .filter(|a| a.len() == 3)
        .ok_or_else(|| format!("`{key}` must be a {{x, y, z}} point"))?;
    let coord = |i: usize| {
        arr[i]
            .as_f64()
            .map(|n| n as f32)
            .ok_or_else(|| format!("`{key}` point needs numeric x, y, z"))
    };
    Ok((coord(0)?, coord(1)?, coord(2)?))
}

/// A distance-constraint target from a `{ kind, index }` object (mirrors
/// `parse_distance_target`): a line's length or a circle's diameter.
fn distance_target_from_json(v: &Value) -> Result<DistanceTarget, String> {
    let t = v.as_object().ok_or("constraint target must be an object")?;
    let kind = req_str(t, "kind", "target")?;
    let index = req_usize(t, "index", "target")?;
    match kind.to_ascii_lowercase().as_str() {
        "line" => Ok(DistanceTarget::LineLength(index)),
        "circle" => Ok(DistanceTarget::CircleDiameter(index)),
        other => Err(format!("unknown constraint target '{other}'")),
    }
}

/// Maps a geometric-constraint name to its type (mirrors `parse_geometric_constraint`).
fn geometric_constraint_from_name(name: &str) -> Option<GeometricConstraintType> {
    match name.to_ascii_lowercase().as_str() {
        "parallel" => Some(GeometricConstraintType::Parallel),
        "perpendicular" => Some(GeometricConstraintType::Perpendicular),
        "equal" => Some(GeometricConstraintType::Equal),
        "coincident" => Some(GeometricConstraintType::Coincident),
        "midpoint" => Some(GeometricConstraintType::Midpoint),
        "horizontal" => Some(GeometricConstraintType::Horizontal),
        "vertical" => Some(GeometricConstraintType::Vertical),
        _ => None,
    }
}

/// Collect the profile faces shared by `revolve`/`loft` (and, in the stateful path,
/// `extrude`): a single `circle`, a `circles` list, a single `polygon` loop, and — only for
/// `loft` (`allow_polygons`) — a `polygons` list of loops. Order matches the closures: single
/// circle, circles list, polygon, polygons.
fn collect_profile_faces(o: &Map<String, Value>, allow_polygons: bool) -> Result<Vec<ExtrudeFace>, String> {
    let mut faces = Vec::new();
    if let Some(i) = opt_usize(o, "circle")? {
        faces.push(ExtrudeFace::Circle(i));
    }
    for i in usize_list(o, "circles")? {
        faces.push(ExtrudeFace::Circle(i));
    }
    if let Some(lines) = opt_usize_array(o, "polygon")? {
        faces.push(ExtrudeFace::Polygon(lines));
    }
    if allow_polygons {
        for lines in usize_array_list(o, "polygons")? {
            faces.push(ExtrudeFace::Polygon(lines));
        }
    }
    Ok(faces)
}

/// `combine`/`edit_boolean` shared arguments: op kind (default "combine"), the A and B body
/// lists, and the keep-B flag.
fn boolean_op_args(o: &Map<String, Value>) -> Result<(BooleanOpKind, Vec<usize>, Vec<usize>, bool), String> {
    let op_name = opt_str(o, "op")?.unwrap_or_else(|| "combine".to_string());
    let kind = BooleanOpKind::from_name(&op_name)
        .ok_or_else(|| format!("unknown boolean op '{op_name}' (combine|cut|intersect|difference)"))?;
    Ok((kind, usize_list(o, "a")?, usize_list(o, "b")?, opt_bool(o, "keep_b")?.unwrap_or(false)))
}

/// `move_bodies`/`edit_move` shared arguments: target bodies, X/Y/Z/angle expression fields,
/// and an optional rotation axis.
#[allow(clippy::type_complexity)]
fn move_op_args(
    o: &Map<String, Value>,
) -> Result<(Vec<usize>, String, String, String, Option<RevolveAxis>, String), String> {
    let targets = usize_list(o, "bodies")?;
    let axis = match o.get("axis") {
        None | Some(Value::Null) => None,
        Some(v) => Some(revolve_axis_from_value(v)?),
    };
    Ok((
        targets,
        expr_arg(o, "x")?,
        expr_arg(o, "y")?,
        expr_arg(o, "z")?,
        axis,
        expr_arg(o, "angle")?,
    ))
}

/// `repeat_bodies`/`edit_repeat` shared arguments: target bodies, axis (default X), mode
/// (default "count_gap"), and count/spacing/length expression fields.
fn repeat_op_args(
    o: &Map<String, Value>,
) -> Result<(Vec<usize>, RevolveAxis, RepeatMode, String, String, String), String> {
    let targets = usize_list(o, "bodies")?;
    let axis = match o.get("axis") {
        None | Some(Value::Null) => RevolveAxis::X,
        Some(v) => revolve_axis_from_value(v)?,
    };
    let mode_name = opt_str(o, "mode")?.unwrap_or_else(|| "count_gap".to_string());
    let mode = RepeatMode::from_name(&mode_name).ok_or_else(|| {
        format!(
            "unknown repeat mode '{mode_name}' (count_gap|count_fit_ends|count_fit_centers|\
             fill_gap|fill_pitch|fill_max_pitch)"
        )
    })?;
    Ok((
        targets,
        axis,
        mode,
        expr_arg(o, "count")?,
        expr_arg(o, "spacing")?,
        expr_arg(o, "length")?,
    ))
}

/// `slice`/`edit_slice` shared arguments: target bodies, the planar cutters (face-spec
/// objects), and the extend-to-infinity flag (default true).
fn slice_op_args(o: &Map<String, Value>) -> Result<(Vec<usize>, Vec<FaceId>, bool), String> {
    let targets = usize_list(o, "bodies")?;
    let mut cutters = Vec::new();
    match o.get("cutters") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            for t in list {
                cutters.push(face_id_from_json(t)?);
            }
        }
        Some(_) => return Err("slice `cutters` must be a list of face specs".into()),
    }
    Ok((targets, cutters, opt_bool(o, "extend")?.unwrap_or(true)))
}

/// A rotation/revolve axis from `"x"`/`"y"`/`"z"` or an object `{ line = i }`.
fn revolve_axis_from_value(v: &Value) -> Result<RevolveAxis, String> {
    match v {
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "x" => Ok(RevolveAxis::X),
            "y" => Ok(RevolveAxis::Y),
            "z" => Ok(RevolveAxis::Z),
            other => Err(format!("unknown axis '{other}' (x|y|z or {{line = i}})")),
        },
        Value::Object(t) => {
            let line = req_usize(t, "line", "axis")?;
            Ok(RevolveAxis::Line(line))
        }
        _ => Err("axis must be \"x\"|\"y\"|\"z\" or {line = i}".into()),
    }
}

/// A `FaceId` from a face-spec object (slice cutters; also the stateful path's targets).
/// Mirrors `parse_face_id_table`: a body cap/side wall (`extrude_cap`/`extrude_side`, with
/// its extrusion + profile descriptors) or, otherwise, a plain `(kind, index)` via
/// [`FaceId::from_script`] (a construction plane or a circle profile).
fn face_id_from_json(v: &Value) -> Result<FaceId, String> {
    let t = v.as_object().ok_or("face spec must be an object")?;
    let kind = t
        .get("kind")
        .or_else(|| t.get("type"))
        .and_then(Value::as_str)
        .ok_or("face spec requires a string `kind`")?;
    match kind.to_ascii_lowercase().as_str() {
        "extrude_cap" | "extrude_side" => {
            let extrusion = req_usize(t, "extrusion", "face")?;
            let profile_kind = t
                .get("profile")
                .or_else(|| t.get("profile_kind"))
                .and_then(Value::as_str)
                .ok_or("extrude face spec requires a `profile`")?;
            let profile_index = match opt_usize(t, "profile_index")? {
                Some(i) => i,
                None => opt_usize(t, "index")?.unwrap_or(0),
            };
            let profile = match profile_kind.to_ascii_lowercase().as_str() {
                "circle" => ExtrudeFace::Circle(profile_index),
                "polygon" => {
                    let lines = match opt_usize_array(t, "profile_lines")? {
                        Some(l) => l,
                        None => opt_usize_array(t, "lines")?
                            .ok_or("polygon profile requires `profile_lines`")?,
                    };
                    ExtrudeFace::Polygon(lines)
                }
                other => return Err(format!("unknown extrude profile kind '{other}'")),
            };
            if kind.eq_ignore_ascii_case("extrude_cap") {
                Ok(FaceId::ExtrudeCap {
                    extrusion,
                    profile,
                    top: opt_bool(t, "top")?.unwrap_or(true),
                })
            } else {
                Ok(FaceId::ExtrudeSide {
                    extrusion,
                    profile,
                    edge: opt_usize(t, "edge")?.unwrap_or(0) as u8,
                })
            }
        }
        _ => {
            let index = req_usize(t, "index", "face")?;
            FaceId::from_script(kind, index)
                .ok_or_else(|| format!("unknown sketch face kind '{kind}'"))
        }
    }
}

/// The read-back query verbs (#107): pure reads of the live document that return JSON data
/// rather than an [`Instruction`]. `count` → a number; `get` and `body_stats` → an object, or
/// JSON `null` when the index doesn't resolve. Mirrors the `count`/`get`/`body_stats` mlua
/// closures exactly.
///
/// The `selection`/`status`/`sketch_dof`/`sketch_conflicts` reads additionally need
/// `AppState` (the live selection / sketch session) beyond the document, so they join the
/// stateful dispatch path; this document-only slice is what's testable off-browser.
pub fn query_from_json(name: &str, args: &Value, doc: &Document) -> Result<Value, String> {
    let o = as_object(args)?;
    match name {
        "count" => {
            let kind = req_str(o, "kind", "count")?;
            let n = match kind.to_ascii_lowercase().as_str() {
                "line" => doc.lines.iter().filter(|e| !e.deleted).count(),
                "circle" => doc.circles.iter().filter(|e| !e.deleted).count(),
                "sketch" => doc.sketches.iter().filter(|e| !e.deleted).count(),
                "constraint" => doc.constraints.iter().filter(|e| !e.deleted).count(),
                "construction_plane" | "plane" => {
                    doc.construction_planes.iter().filter(|e| !e.deleted).count()
                }
                "extrusion" => doc.extrusions.iter().filter(|e| !e.deleted).count(),
                "body" => doc.bodies.iter().filter(|e| !e.deleted).count(),
                "drawing" => doc.drawings.iter().filter(|e| !e.deleted).count(),
                "parameter" => doc.parameters.iter().filter(|e| !e.deleted).count(),
                other => {
                    return Err(format!(
                        "unknown count kind '{other}' (valid kinds: line, circle, sketch, \
                         constraint, construction_plane, extrusion, body, drawing, parameter)"
                    ))
                }
            };
            Ok(json!(n))
        }
        "get" => {
            let kind = req_str(o, "kind", "get")?;
            let index = req_usize(o, "index", "get")?;
            Ok(get_element(doc, &kind, index)?)
        }
        "body_stats" => {
            let index = req_usize(o, "index", "body_stats")?;
            if !doc.bodies.get(index).is_some_and(|b| !b.deleted) {
                return Ok(Value::Null);
            }
            let Some(mesh) = crate::extrude::body_solid_mesh(doc, index) else {
                return Ok(Value::Null);
            };
            let Some((min, max)) = mesh.bounds() else {
                return Ok(Value::Null);
            };
            Ok(json!({
                "volume": crate::extrude::mesh_signed_volume(&mesh).abs(),
                "triangles": mesh.triangles.len(),
                "bbox": { "min": vec3_json(min), "max": vec3_json(max) },
            }))
        }
        other => Err(format!("unknown query '{other}'")),
    }
}

/// Body of `get`: the JSON object for one element, or `Value::Null` when it doesn't resolve.
fn get_element(doc: &Document, kind: &str, index: usize) -> Result<Value, String> {
    let mut t = Map::new();
    match kind.to_ascii_lowercase().as_str() {
        "line" => {
            let Some(line) = doc.lines.get(index).filter(|e| !e.deleted) else {
                return Ok(Value::Null);
            };
            t.insert("x0".into(), json!(line.x0));
            t.insert("y0".into(), json!(line.y0));
            t.insert("x1".into(), json!(line.x1));
            t.insert("y1".into(), json!(line.y1));
            t.insert("construction".into(), json!(line.construction));
            t.insert("curved".into(), json!(line.is_curved()));
            if let Some([c0, c1]) = line.bezier {
                t.insert("bezier".into(), json!([[c0.0, c0.1], [c1.0, c1.1]]));
            }
            t.insert("length".into(), json!(line.length()));
            if let Some(name) = &line.name {
                t.insert("name".into(), json!(name));
            }
            t.insert("sketch".into(), json!(line.sketch));
        }
        "circle" => {
            let Some(circle) = doc.circles.get(index).filter(|e| !e.deleted) else {
                return Ok(Value::Null);
            };
            t.insert("x".into(), json!(circle.cx));
            t.insert("y".into(), json!(circle.cy));
            t.insert("r".into(), json!(circle.r));
            t.insert("diameter".into(), json!(circle.diameter()));
            t.insert("construction".into(), json!(circle.construction));
            if let Some(name) = &circle.name {
                t.insert("name".into(), json!(name));
            }
            t.insert("sketch".into(), json!(circle.sketch));
        }
        "sketch" => {
            let Some(sketch) = doc.sketches.get(index).filter(|e| !e.deleted) else {
                return Ok(Value::Null);
            };
            t.insert("face".into(), json!(face_kind_name(&sketch.face)));
            if let Some(name) = &sketch.name {
                t.insert("name".into(), json!(name));
            }
        }
        "constraint" => {
            let Some(constraint) = doc.constraints.get(index).filter(|e| !e.deleted) else {
                return Ok(Value::Null);
            };
            t.insert("kind".into(), json!(constraint_kind_name(&constraint.kind)));
            t.insert("expression".into(), json!(constraint.expression));
            if let Some(name) = &constraint.name {
                t.insert("name".into(), json!(name));
            }
            t.insert("sketch".into(), json!(constraint.sketch));
        }
        "construction_plane" | "plane" => {
            let Some(plane) = doc.construction_planes.get(index).filter(|e| !e.deleted) else {
                return Ok(Value::Null);
            };
            t.insert("origin".into(), vec3_json(plane.origin));
            t.insert("normal".into(), vec3_json(plane.normal));
            if let Some(name) = &plane.name {
                t.insert("name".into(), json!(name));
            }
        }
        "extrusion" => {
            let Some(extrusion) = doc.extrusions.get(index).filter(|e| !e.deleted) else {
                return Ok(Value::Null);
            };
            t.insert("distance".into(), json!(extrusion.distance));
            t.insert("sketch".into(), json!(extrusion.sketch));
            t.insert("faces".into(), json!(extrusion.faces.len()));
            if let Some(name) = &extrusion.name {
                t.insert("name".into(), json!(name));
            }
        }
        "body" => {
            let Some(body) = doc.bodies.get(index).filter(|e| !e.deleted) else {
                return Ok(Value::Null);
            };
            if let Some(name) = &body.name {
                t.insert("name".into(), json!(name));
            }
            t.insert("add".into(), json!(body.source.extrusion_indices()));
            t.insert("cut".into(), json!(body.source.cut_extrusion_indices()));
        }
        "parameter" => {
            let Some(param) = doc.parameters.get(index).filter(|e| !e.deleted) else {
                return Ok(Value::Null);
            };
            t.insert("name".into(), json!(param.name));
            t.insert("expression".into(), json!(param.expression));
        }
        other => {
            return Err(format!(
                "unknown get kind '{other}' (valid kinds: line, circle, sketch, constraint, \
                 construction_plane, extrusion, body, parameter)"
            ))
        }
    }
    Ok(Value::Object(t))
}

/// A world-space vector as a positional JSON triple `[x, y, z]` (matching the mlua getters'
/// `vec3_lua`, which returns a 1-based Lua array).
fn vec3_json(v: glam::Vec3) -> Value {
    json!([v.x, v.y, v.z])
}

/// Short script name for the face a sketch is hosted on (mirrors `lua_script::face_kind_name`).
fn face_kind_name(face: &FaceId) -> &'static str {
    match face {
        FaceId::Circle(_) => "circle",
        FaceId::Polygon(_) => "polygon",
        FaceId::ConstructionPlane(_) => "construction_plane",
        FaceId::ExtrudeCap { .. } => "extrude_cap",
        FaceId::ExtrudeSide { .. } => "extrude_side",
    }
}

/// Short script name for a constraint's kind (mirrors `lua_script::constraint_kind_name`).
fn constraint_kind_name(kind: &ConstraintKind) -> &'static str {
    match kind {
        ConstraintKind::Distance { .. } => "distance",
        ConstraintKind::Parallel { .. } => "parallel",
        ConstraintKind::Perpendicular { .. } => "perpendicular",
        ConstraintKind::Equal { .. } => "equal",
        ConstraintKind::Coincident { .. } => "coincident",
        ConstraintKind::Midpoint { .. } => "midpoint",
        ConstraintKind::Horizontal { .. } => "horizontal",
        ConstraintKind::Vertical { .. } => "vertical",
        ConstraintKind::Angle { .. } => "angle",
    }
}

/// `bezier = [[cx0, cy0], [cx1, cy1]]` (#54): tangent handles near each endpoint.
fn parse_bezier(o: &Map<String, Value>) -> Result<Option<[(f32, f32); 2]>, String> {
    let Some(v) = o.get("bezier") else {
        return Ok(None);
    };
    if v.is_null() {
        return Ok(None);
    }
    let arr = v.as_array().ok_or("line `bezier` must be a pair of handles")?;
    let handle = |i: usize| -> Result<(f32, f32), String> {
        let h = arr
            .get(i)
            .and_then(Value::as_array)
            .ok_or("line `bezier` must be a pair of [x, y] handles")?;
        let coord = |j: usize| {
            h.get(j)
                .and_then(Value::as_f64)
                .map(|n| n as f32)
                .ok_or_else(|| "line `bezier` handle needs numeric x and y".to_string())
        };
        Ok((coord(0)?, coord(1)?))
    };
    Ok(Some([handle(0)?, handle(1)?]))
}

/// `dimension`: an expression string, a number, or `true` (lock at the as-drawn length) —
/// matching the mlua closure's accepted forms.
fn parse_dimension(
    o: &Map<String, Value>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
) -> Result<Option<String>, String> {
    match o.get("dimension") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Ok(None),
        Some(Value::Bool(true)) => Ok(Some(((x1 - x0).hypot(y1 - y0)).to_string())),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::Number(n)) => Ok(Some(n.to_string())),
        Some(_) => Err("line `dimension` must be an expression string, a number, or true".into()),
    }
}

fn as_object(v: &Value) -> Result<&Map<String, Value>, String> {
    match v {
        Value::Object(m) => Ok(m),
        Value::Null => Err("expected an argument object".into()),
        _ => Err("arguments must be a JSON object".into()),
    }
}

fn opt_f32(o: &Map<String, Value>, key: &str) -> Result<Option<f32>, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .map(|n| Some(n as f32))
            .ok_or_else(|| format!("`{key}` must be a number")),
    }
}

fn req_f32(o: &Map<String, Value>, key: &str, cmd: &str) -> Result<f32, String> {
    opt_f32(o, key)?.ok_or_else(|| format!("{cmd} requires `{key}`"))
}

fn opt_usize(o: &Map<String, Value>, key: &str) -> Result<Option<usize>, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|n| *n >= 0.0)
            .map(|n| Some(n.round() as usize))
            .ok_or_else(|| format!("`{key}` must be a non-negative integer")),
    }
}

fn req_usize(o: &Map<String, Value>, key: &str, cmd: &str) -> Result<usize, String> {
    opt_usize(o, key)?.ok_or_else(|| format!("{cmd} requires `{key}`"))
}

fn req_str(o: &Map<String, Value>, key: &str, cmd: &str) -> Result<String, String> {
    match o.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        _ => Err(format!("{cmd} requires a string `{key}`")),
    }
}

fn opt_str(o: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

fn opt_bool(o: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{key}` must be a boolean")),
    }
}

/// An expression field (move/repeat translation, angle, count, spacing, length): a string
/// expression, or a number stringified the way the mlua closures stringify Lua numbers
/// (integers without a decimal point). Missing/null → empty string, matching the closures'
/// `Value::Nil => String::new()`.
fn expr_arg(o: &Map<String, Value>, key: &str) -> Result<String, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(Value::Number(n)) => match n.as_i64() {
            Some(i) => Ok(i.to_string()),
            None => Ok(n.as_f64().map(|f| f.to_string()).unwrap_or_default()),
        },
        Some(_) => Err(format!("`{key}` must be an expression string or a number")),
    }
}

/// An expression `Value` (string or number) stringified like [`expr_arg`], for a value that
/// may be either. Used where a number is a shorthand for its literal expression.
fn value_to_expr(v: &Value, key: &str) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(match n.as_i64() {
            Some(i) => i.to_string(),
            None => n.as_f64().map(|f| f.to_string()).unwrap_or_default(),
        }),
        _ => Err(format!("`{key}` must be an expression string or a number")),
    }
}

/// A required expression field (a dimension value): a string, or a number stringified.
fn req_expr(o: &Map<String, Value>, key: &str, cmd: &str) -> Result<String, String> {
    match o.get(key) {
        None | Some(Value::Null) => Err(format!("{cmd} requires `{key}`")),
        Some(v) => value_to_expr(v, key),
    }
}

fn opt_i8(o: &Map<String, Value>, key: &str) -> Result<Option<i8>, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_i64()
            .map(|n| Some(n as i8))
            .ok_or_else(|| format!("`{key}` must be an integer")),
    }
}

/// A required boolean flag accepting `true`/`false` or the string forms the mlua `parse_bool`
/// accepts (`on`/`off`, `yes`/`no`, `1`/`0`).
fn req_bool_flag(o: &Map<String, Value>, key: &str, cmd: &str) -> Result<bool, String> {
    match o.get(key) {
        Some(Value::Bool(b)) => Ok(*b),
        Some(Value::String(s)) => match s.to_ascii_lowercase().as_str() {
            "true" | "on" | "yes" | "1" => Ok(true),
            "false" | "off" | "no" | "0" => Ok(false),
            other => Err(format!("unknown {key} value '{other}'")),
        },
        _ => Err(format!("{cmd} requires a boolean `{key}`")),
    }
}

/// A single non-negative integer element of an array, for the list helpers below.
fn as_index(v: &Value, key: &str) -> Result<usize, String> {
    v.as_f64()
        .filter(|n| *n >= 0.0)
        .map(|n| n.round() as usize)
        .ok_or_else(|| format!("`{key}` must be non-negative integers"))
}

/// A list of non-negative integer indices (`bodies`, `a`, `b`, `circles`). Missing/null →
/// empty (matching the closures' `unwrap_or_default()` on an optional `Vec<usize>`).
fn usize_list(o: &Map<String, Value>, key: &str) -> Result<Vec<usize>, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(arr)) => arr.iter().map(|v| as_index(v, key)).collect(),
        Some(_) => Err(format!("`{key}` must be a list of non-negative integers")),
    }
}

/// A single required-when-present integer array (a `polygon` line loop). `None` when absent.
fn opt_usize_array(o: &Map<String, Value>, key: &str) -> Result<Option<Vec<usize>>, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(arr)) => {
            arr.iter().map(|v| as_index(v, key)).collect::<Result<_, _>>().map(Some)
        }
        Some(_) => Err(format!("`{key}` must be a list of line indices")),
    }
}

/// A list of integer arrays (`polygons`: several line loops). Missing/null → empty.
fn usize_array_list(o: &Map<String, Value>, key: &str) -> Result<Vec<Vec<usize>>, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|loop_v| {
                loop_v
                    .as_array()
                    .ok_or_else(|| format!("`{key}` must be a list of line-index lists"))?
                    .iter()
                    .map(|v| as_index(v, key))
                    .collect()
            })
            .collect(),
        Some(_) => Err(format!("`{key}` must be a list of line-index lists")),
    }
}

/// A plane-local `[x, y]` point pair (`calibrate_image`'s `from`/`to`).
fn xy_pair(o: &Map<String, Value>, key: &str) -> Result<(f32, f32), String> {
    let arr = o
        .get(key)
        .and_then(Value::as_array)
        .filter(|a| a.len() == 2)
        .ok_or_else(|| format!("`{key}` must be a two-element [x, y] point"))?;
    let coord = |i: usize| {
        arr[i]
            .as_f64()
            .map(|n| n as f32)
            .ok_or_else(|| format!("`{key}` point needs numeric x and y"))
    };
    Ok((coord(0)?, coord(1)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::Tool;
    use serde_json::json;

    #[test]
    fn document_and_tool_actions_map_to_instructions() {
        assert_eq!(instruction_from_json("new", &json!({})), Ok(Instruction::New));
        assert_eq!(instruction_from_json("clear", &json!({})), Ok(Instruction::Clear));
        assert_eq!(instruction_from_json("undo", &json!({})), Ok(Instruction::Undo));
        assert_eq!(instruction_from_json("quit", &json!({})), Ok(Instruction::Quit));
        assert_eq!(
            instruction_from_json("exit_sketch", &json!({})),
            Ok(Instruction::ExitSketch)
        );
        assert_eq!(
            instruction_from_json("tool", &json!({ "name": "circle" })),
            Ok(Instruction::Tool(Tool::Circle))
        );
        assert!(instruction_from_json("tool", &json!({ "name": "nope" })).is_err());
    }

    #[test]
    fn rect_matches_the_native_defaults() {
        // Same as `bearcad.rect{ width = 40, height = 20 }`: x/y default to 0.
        assert_eq!(
            instruction_from_json("rect", &json!({ "width": 40, "height": 20 })),
            Ok(Instruction::CreateRect { x: 0.0, y: 0.0, width: 40.0, height: 20.0 })
        );
        assert_eq!(
            instruction_from_json("rect", &json!({ "x": 5, "y": -3, "width": 40, "height": 20 })),
            Ok(Instruction::CreateRect { x: 5.0, y: -3.0, width: 40.0, height: 20.0 })
        );
        assert!(instruction_from_json("rect", &json!({ "width": 40 })).is_err());
    }

    #[test]
    fn circle_accepts_r_radius_or_diameter() {
        let r = Instruction::CreateCircle { cx: 0.0, cy: 0.0, r: 5.0 };
        assert_eq!(instruction_from_json("circle", &json!({ "r": 5 })), Ok(r.clone()));
        assert_eq!(instruction_from_json("circle", &json!({ "radius": 5 })), Ok(r.clone()));
        assert_eq!(instruction_from_json("circle", &json!({ "diameter": 10 })), Ok(r));
        assert!(instruction_from_json("circle", &json!({ "x": 1 })).is_err());
    }

    #[test]
    fn line_supports_endpoints_and_length_angle() {
        assert_eq!(
            instruction_from_json("line", &json!({ "x1": 30, "y1": 0 })),
            Ok(Instruction::CreateLine {
                x0: 0.0,
                y0: 0.0,
                x1: 30.0,
                y1: 0.0,
                bezier: None,
                dimension: None,
            })
        );
        // length + default angle 0 lands at (length, 0).
        let Instruction::CreateLine { x1, y1, .. } =
            instruction_from_json("line", &json!({ "length": 10 })).unwrap()
        else {
            panic!("expected a line");
        };
        assert!((x1 - 10.0).abs() < 1e-5 && y1.abs() < 1e-5);
    }

    #[test]
    fn line_dimension_true_locks_the_as_drawn_length() {
        let instr =
            instruction_from_json("line", &json!({ "x1": 3, "y1": 4, "dimension": true })).unwrap();
        let Instruction::CreateLine { dimension, .. } = instr else {
            panic!("expected a line");
        };
        assert_eq!(dimension.as_deref(), Some("5"));
    }

    #[test]
    fn line_bezier_reads_both_handles() {
        let instr = instruction_from_json(
            "line",
            &json!({ "x1": 10, "y1": 0, "bezier": [[2, 3], [8, -1]] }),
        )
        .unwrap();
        let Instruction::CreateLine { bezier, .. } = instr else {
            panic!("expected a line");
        };
        assert_eq!(bezier, Some([(2.0, 3.0), (8.0, -1.0)]));
    }

    #[test]
    fn plane_and_begin_sketch_and_open_sketch() {
        assert_eq!(
            instruction_from_json("plane", &json!({ "offset": 12, "from": 1 })),
            Ok(Instruction::CreatePlane { offset: 12.0, from: 1 })
        );
        assert_eq!(
            instruction_from_json("plane", &json!({})),
            Ok(Instruction::CreatePlane { offset: 0.0, from: 0 })
        );
        assert_eq!(
            instruction_from_json("begin_sketch", &json!({ "kind": "plane", "index": 0 })),
            Ok(Instruction::BeginSketch { face: FaceId::from_script("plane", 0).unwrap() })
        );
        assert_eq!(
            instruction_from_json("open_sketch", &json!({ "sketch": 2 })),
            Ok(Instruction::OpenSketch { sketch: 2 })
        );
    }

    #[test]
    fn sketch_primitives_open_a_sketch_when_none_is_active() {
        assert!(opens_sketch_when_none_active("rect"));
        assert!(opens_sketch_when_none_active("line"));
        assert!(opens_sketch_when_none_active("circle"));
        assert!(!opens_sketch_when_none_active("plane"));
        assert!(!opens_sketch_when_none_active("extrude"));
    }

    #[test]
    fn unknown_command_and_bad_args_report_errors() {
        assert!(instruction_from_json("frobnicate", &json!({})).is_err());
        assert!(instruction_from_json("rect", &json!("not an object")).is_err());
        assert!(instruction_from_json("tool", &json!({})).is_err());
    }

    #[test]
    fn io_commands_map_to_instructions() {
        assert_eq!(
            instruction_from_json("open", &json!({ "path": "part.bcad" })),
            Ok(Instruction::Open("part.bcad".into()))
        );
        assert_eq!(instruction_from_json("save", &json!({})), Ok(Instruction::Save(None)));
        assert_eq!(
            instruction_from_json("save", &json!({ "path": "out.bcad" })),
            Ok(Instruction::Save(Some("out.bcad".into())))
        );
        assert_eq!(
            instruction_from_json("export_stl", &json!({ "path": "a.stl", "body": "Plate" })),
            Ok(Instruction::ExportStl { path: "a.stl".into(), body: Some("Plate".into()) })
        );
        assert_eq!(
            instruction_from_json("export_step", &json!({ "path": "a.step" })),
            Ok(Instruction::ExportStep { path: "a.step".into(), body: None })
        );
        assert_eq!(
            instruction_from_json("import_image", &json!({ "path": "p.png", "plane": 2 })),
            Ok(Instruction::ImportImage { path: "p.png".into(), plane: Some(2) })
        );
        assert_eq!(
            instruction_from_json(
                "calibrate_image",
                &json!({ "image": 0, "from": [0, 0], "to": [10, 0], "length": 25 })
            ),
            Ok(Instruction::CalibrateImage {
                image: 0,
                a: (0.0, 0.0),
                b: (10.0, 0.0),
                length: 25.0,
            })
        );
    }

    #[test]
    fn revolve_defaults_match_the_closure() {
        // Bare `bearcad.revolve{ polygon = {0,1,2,3}, axis = "y" }`: angle 360, not symmetric,
        // new body, no explicit body list.
        assert_eq!(
            instruction_from_json("revolve", &json!({ "polygon": [0, 1, 2, 3], "axis": "y" })),
            Ok(Instruction::Revolve {
                faces: vec![ExtrudeFace::Polygon(vec![0, 1, 2, 3])],
                axis: RevolveAxis::Y,
                angle_deg: 360.0,
                symmetric: false,
                body: RevolveBodyChoice::NewBody,
                bodies: vec![],
            })
        );
        assert_eq!(
            instruction_from_json(
                "revolve",
                &json!({ "circle": 0, "axis": { "line": 3 }, "angle": 90, "symmetric": true,
                         "body": "cut", "bodies": [1, 2] })
            ),
            Ok(Instruction::Revolve {
                faces: vec![ExtrudeFace::Circle(0)],
                axis: RevolveAxis::Line(3),
                angle_deg: 90.0,
                symmetric: true,
                body: RevolveBodyChoice::Cut,
                bodies: vec![1, 2],
            })
        );
        assert!(instruction_from_json("revolve", &json!({ "circle": 0 })).is_err());
        assert!(instruction_from_json("revolve", &json!({ "axis": "x" })).is_err());
    }

    #[test]
    fn loft_gathers_circles_and_polygons() {
        assert_eq!(
            instruction_from_json(
                "loft",
                &json!({ "circles": [0, 1], "polygons": [[2, 3, 4, 5]] })
            ),
            Ok(Instruction::Loft {
                faces: vec![
                    ExtrudeFace::Circle(0),
                    ExtrudeFace::Circle(1),
                    ExtrudeFace::Polygon(vec![2, 3, 4, 5]),
                ],
            })
        );
        // Fewer than two sections is rejected, as in the closure.
        assert!(instruction_from_json("loft", &json!({ "circle": 0 })).is_err());
    }

    #[test]
    fn combine_defaults_and_edit() {
        assert_eq!(
            instruction_from_json("combine", &json!({ "a": [0], "b": [1] })),
            Ok(Instruction::CreateBooleanOp {
                kind: BooleanOpKind::Combine,
                a: vec![0],
                b: vec![1],
                keep_b: false,
            })
        );
        assert_eq!(
            instruction_from_json(
                "edit_boolean",
                &json!({ "index": 2, "op": "cut", "a": [0], "b": [1], "keep_b": true })
            ),
            Ok(Instruction::EditBooleanOp {
                op: 2,
                kind: BooleanOpKind::Cut,
                a: vec![0],
                b: vec![1],
                keep_b: true,
            })
        );
        assert!(instruction_from_json("combine", &json!({ "op": "nope" })).is_err());
    }

    #[test]
    fn move_bodies_stringifies_expression_fields() {
        assert_eq!(
            instruction_from_json(
                "move_bodies",
                &json!({ "bodies": [0], "x": 10, "y": "w/2", "angle": 45, "axis": "z" })
            ),
            Ok(Instruction::CreateMoveOp {
                targets: vec![0],
                tx: "10".into(),
                ty: "w/2".into(),
                tz: String::new(),
                axis: Some(RevolveAxis::Z),
                angle: "45".into(),
            })
        );
        // No axis → no rotation; omitted expression fields become empty strings.
        assert_eq!(
            instruction_from_json("edit_move", &json!({ "index": 1, "bodies": [0], "z": 5 })),
            Ok(Instruction::EditMoveOp {
                op: 1,
                targets: vec![0],
                tx: String::new(),
                ty: String::new(),
                tz: "5".into(),
                axis: None,
                angle: String::new(),
            })
        );
    }

    #[test]
    fn repeat_bodies_defaults_axis_and_mode() {
        assert_eq!(
            instruction_from_json("repeat_bodies", &json!({ "bodies": [0], "count": 5, "spacing": 20 })),
            Ok(Instruction::CreateRepeatOp {
                targets: vec![0],
                axis: RevolveAxis::X,
                mode: RepeatMode::CountGap,
                count: "5".into(),
                spacing: "20".into(),
                length: String::new(),
            })
        );
        assert_eq!(
            instruction_from_json(
                "repeat_bodies",
                &json!({ "bodies": [0], "axis": "y", "mode": "fill_pitch", "length": 100, "spacing": 12 })
            ),
            Ok(Instruction::CreateRepeatOp {
                targets: vec![0],
                axis: RevolveAxis::Y,
                mode: RepeatMode::FillPitch,
                count: String::new(),
                spacing: "12".into(),
                length: "100".into(),
            })
        );
        assert!(instruction_from_json("repeat_bodies", &json!({ "mode": "nope" })).is_err());
    }

    #[test]
    fn dimension_verbs_route_by_axis() {
        assert_eq!(
            instruction_from_json("set_dim", &json!({ "axis": "width", "value": "40" })),
            Ok(Instruction::SetDim { axis: RectAxis::Width, value: "40".into() })
        );
        // A bare number for the value is stringified.
        assert_eq!(
            instruction_from_json("set_dim", &json!({ "axis": "length", "value": 25 })),
            Ok(Instruction::SetLineLength { value: "25".into() })
        );
        assert_eq!(
            instruction_from_json("set_dim", &json!({ "axis": "diameter", "value": "d" })),
            Ok(Instruction::SetCircleDiameter { value: "d".into() })
        );
        assert_eq!(
            instruction_from_json("set_dim", &json!({ "axis": "offset", "value": "5" })),
            Ok(Instruction::SetPlaneOffset { value: "5".into() })
        );
        assert_eq!(
            instruction_from_json("focus_dim", &json!({ "axis": "h" })),
            Ok(Instruction::FocusDim(RectAxis::Height))
        );
        assert_eq!(
            instruction_from_json("focus_dim", &json!({ "axis": "angle" })),
            Ok(Instruction::FocusPlaneDim(PlaneDim::Angle))
        );
        assert_eq!(
            instruction_from_json("edit_dim", &json!({ "axis": "length" })),
            Ok(Instruction::BeginEditCommittedDim { axis: DimLabelAxis::Length })
        );
        assert_eq!(
            instruction_from_json("commit_dim", &json!({})),
            Ok(Instruction::CommitCommittedDim)
        );
        assert_eq!(
            instruction_from_json("set_dim_label_offset", &json!({ "axis": "w", "offset": 3 })),
            Ok(Instruction::SetDimLabelOffset { axis: DimLabelAxis::Width, offset: 3.0 })
        );
        assert!(instruction_from_json("set_dim", &json!({ "axis": "nope", "value": "1" })).is_err());
    }

    #[test]
    fn constraint_verbs_map_to_instructions() {
        assert_eq!(
            instruction_from_json(
                "add_constraint",
                &json!({ "target": { "kind": "line", "index": 0 }, "expression": "40" })
            ),
            Ok(Instruction::AddDistanceConstraint {
                target: DistanceTarget::LineLength(0),
                expression: "40".into(),
            })
        );
        assert_eq!(
            instruction_from_json(
                "add_constraint",
                &json!({ "target": { "kind": "circle", "index": 2 }, "expression": 12 })
            ),
            Ok(Instruction::AddDistanceConstraint {
                target: DistanceTarget::CircleDiameter(2),
                expression: "12".into(),
            })
        );
        // Angle: `value` string form, and `angle`-number form; default sign +1.
        assert_eq!(
            instruction_from_json(
                "add_angle_constraint",
                &json!({ "a": 0, "b": 5, "value": "120" })
            ),
            Ok(Instruction::AddAngleConstraint {
                line_a: 0,
                line_b: 5,
                rotation_sign: 1,
                expression: "120".into(),
            })
        );
        assert_eq!(
            instruction_from_json(
                "add_angle_constraint",
                &json!({ "a": 0, "b": 5, "angle": 90, "sign": -1 })
            ),
            Ok(Instruction::AddAngleConstraint {
                line_a: 0,
                line_b: 5,
                rotation_sign: -1,
                expression: "90".into(),
            })
        );
        assert_eq!(
            instruction_from_json("add_geometric_constraint", &json!({ "name": "parallel" })),
            Ok(Instruction::AddGeometricConstraint(GeometricConstraintType::Parallel))
        );
        assert_eq!(
            instruction_from_json("constraint_shortcut", &json!({ "key": "p" })),
            Ok(Instruction::ApplyConstraintShortcut('p'))
        );
        assert!(
            instruction_from_json("add_geometric_constraint", &json!({ "name": "nope" })).is_err()
        );
        assert!(instruction_from_json("add_angle_constraint", &json!({ "a": 0, "b": 5 })).is_err());
    }

    #[test]
    fn plane_edit_naming_and_deletion_verbs() {
        assert_eq!(
            instruction_from_json("edit_plane", &json!({ "index": 1 })),
            Ok(Instruction::BeginEditConstructionPlane { index: 1 })
        );
        assert_eq!(
            instruction_from_json("commit_plane", &json!({})),
            Ok(Instruction::CommitConstructionPlane)
        );
        assert_eq!(
            instruction_from_json("focus_name", &json!({})),
            Ok(Instruction::FocusElementName)
        );
        assert_eq!(
            instruction_from_json("apply_construction", &json!({ "construction": true })),
            Ok(Instruction::ApplyConstruction { construction: true })
        );
        assert_eq!(
            instruction_from_json("apply_construction", &json!({ "construction": "off" })),
            Ok(Instruction::ApplyConstruction { construction: false })
        );
        assert_eq!(
            instruction_from_json("toggle_construction", &json!({})),
            Ok(Instruction::ToggleConstruction)
        );
        assert_eq!(
            instruction_from_json("delete_selection", &json!({})),
            Ok(Instruction::DeleteSelection)
        );
    }

    #[test]
    fn drawing_verbs_map_to_instructions() {
        assert_eq!(
            instruction_from_json("drawing", &json!({ "name": "Plate" })),
            Ok(Instruction::CreateDrawing { name: Some("Plate".into()) })
        );
        assert_eq!(
            instruction_from_json("drawing", &json!({})),
            Ok(Instruction::CreateDrawing { name: None })
        );
        // orientation defaults to Front; "iso" is accepted.
        assert_eq!(
            instruction_from_json("drawing_view", &json!({ "drawing": 0, "body": 1 })),
            Ok(Instruction::AddDrawingView {
                drawing: 0,
                body: 1,
                orientation: DrawingOrientation::Front,
            })
        );
        assert_eq!(
            instruction_from_json(
                "drawing_view",
                &json!({ "drawing": 0, "body": 0, "orientation": "iso" })
            ),
            Ok(Instruction::AddDrawingView {
                drawing: 0,
                body: 0,
                orientation: DrawingOrientation::Isometric,
            })
        );
        assert_eq!(
            instruction_from_json(
                "export_drawing_svg",
                &json!({ "drawing": 2, "path": "plate.svg" })
            ),
            Ok(Instruction::ExportDrawingSvg { drawing: 2, path: "plate.svg".into() })
        );
        assert_eq!(
            instruction_from_json(
                "drawing_dimension",
                &json!({ "drawing": 0, "view": 1, "a": [0, 0, 0], "b": [40, 0, 0] })
            ),
            Ok(Instruction::ToggleDrawingDimension {
                drawing: 0,
                view: 1,
                a: (0.0, 0.0, 0.0),
                b: (40.0, 0.0, 0.0),
            })
        );
        assert_eq!(
            instruction_from_json(
                "drawing_angle",
                &json!({ "drawing": 0, "view": 0,
                         "edge1": { "a": [0, 0, 0], "b": [40, 0, 0] },
                         "edge2": { "a": [0, 0, 0], "b": [0, 0, 15] } })
            ),
            Ok(Instruction::ToggleDrawingAngle {
                drawing: 0,
                view: 0,
                edge1: ((0.0, 0.0, 0.0), (40.0, 0.0, 0.0)),
                edge2: ((0.0, 0.0, 0.0), (0.0, 0.0, 15.0)),
            })
        );
        assert!(
            instruction_from_json("drawing_view", &json!({ "drawing": 0, "body": 0, "orientation": "nope" }))
                .is_err()
        );
        assert!(
            instruction_from_json("drawing_dimension", &json!({ "drawing": 0, "view": 0, "a": [0, 0], "b": [1, 1, 1] }))
                .is_err()
        );
    }

    fn doc_with(lines: Value, circles: Value) -> Document {
        let mut doc = Document::default();
        doc.lines = serde_json::from_value(lines).unwrap();
        doc.circles = serde_json::from_value(circles).unwrap();
        doc
    }

    #[test]
    fn count_ignores_deleted_entities() {
        let doc = doc_with(
            json!([
                { "sketch": 0, "x0": 0, "y0": 0, "x1": 30, "y1": 0 },
                { "sketch": 0, "x0": 0, "y0": 0, "x1": 0, "y1": 10, "deleted": true },
                { "sketch": 0, "x0": 0, "y0": 10, "x1": 30, "y1": 10 },
            ]),
            json!([{ "sketch": 0, "cx": 5, "cy": 5, "r": 3 }]),
        );
        assert_eq!(query_from_json("count", &json!({ "kind": "line" }), &doc), Ok(json!(2)));
        assert_eq!(query_from_json("count", &json!({ "kind": "circle" }), &doc), Ok(json!(1)));
        assert_eq!(query_from_json("count", &json!({ "kind": "body" }), &doc), Ok(json!(0)));
        assert!(query_from_json("count", &json!({ "kind": "nope" }), &doc).is_err());
    }

    #[test]
    fn get_line_and_circle_report_geometry() {
        let doc = doc_with(
            json!([{ "sketch": 0, "x0": 0, "y0": 0, "x1": 3, "y1": 4 }]),
            json!([{ "sketch": 1, "cx": 5, "cy": 6, "r": 2 }]),
        );
        let line = query_from_json("get", &json!({ "kind": "line", "index": 0 }), &doc).unwrap();
        assert_eq!(line["x1"], json!(3.0));
        assert_eq!(line["y1"], json!(4.0));
        assert_eq!(line["length"], json!(5.0));
        assert_eq!(line["construction"], json!(false));
        assert_eq!(line["curved"], json!(false));
        assert_eq!(line["sketch"], json!(0));

        let circle = query_from_json("get", &json!({ "kind": "circle", "index": 0 }), &doc).unwrap();
        assert_eq!(circle["x"], json!(5.0));
        assert_eq!(circle["r"], json!(2.0));
        assert_eq!(circle["diameter"], json!(4.0));
        assert_eq!(circle["sketch"], json!(1));
    }

    #[test]
    fn get_out_of_range_index_is_null() {
        let doc = doc_with(json!([]), json!([]));
        assert_eq!(
            query_from_json("get", &json!({ "kind": "line", "index": 7 }), &doc),
            Ok(Value::Null)
        );
        assert_eq!(
            query_from_json("body_stats", &json!({ "index": 0 }), &doc),
            Ok(Value::Null)
        );
        assert!(query_from_json("get", &json!({ "kind": "nope", "index": 0 }), &doc).is_err());
        assert!(query_from_json("frobnicate", &json!({}), &doc).is_err());
    }

    #[test]
    fn slice_reads_plane_and_body_cutters() {
        assert_eq!(
            instruction_from_json(
                "slice",
                &json!({ "bodies": [0], "cutters": [{ "kind": "plane", "index": 1 }] })
            ),
            Ok(Instruction::CreateSliceOp {
                targets: vec![0],
                cutters: vec![FaceId::ConstructionPlane(1)],
                extend_infinite: true,
            })
        );
        // A body cap cutter, and the extend flag turned off.
        assert_eq!(
            instruction_from_json(
                "edit_slice",
                &json!({ "index": 0, "bodies": [1], "extend": false,
                         "cutters": [{ "kind": "extrude_cap", "extrusion": 0, "profile": "polygon",
                                       "profile_lines": [0, 1, 2, 3], "top": false }] })
            ),
            Ok(Instruction::EditSliceOp {
                op: 0,
                targets: vec![1],
                cutters: vec![FaceId::ExtrudeCap {
                    extrusion: 0,
                    profile: ExtrudeFace::Polygon(vec![0, 1, 2, 3]),
                    top: false,
                }],
                extend_infinite: false,
            })
        );
    }
}
