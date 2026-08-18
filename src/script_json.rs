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

use crate::actions::{DimLabelAxis, ExtrudeBodyChoice, Pane, RectAxis, RevolveBodyChoice};
use crate::camera::{GroundDisplay, ProjectionMode, ShadingMode, StandardView};
use crate::construction::PlaneDim;
use crate::geometric_constraints::GeometricConstraintType;
use crate::hierarchy::{HierarchyViewMode, SceneElement};
use crate::model::{
    BooleanOp, BooleanOpKind, ConstraintKind, ConstraintPoint, DistanceTarget, Document,
    DrawingOrientation, ExtrudeFace, ExtrudeTarget, ExtrusionEdgeRef, FaceId, LineEnd, RepeatMode,
    RevolveAxis, VertexTreatmentKind,
};
use crate::script::Instruction;
use crate::view_cube::{CubeCornerId, CubeEdgeId};
use serde_json::{json, Map, Value};

/// Commands that draw into a sketch and, like their mlua closures, begin one on the ground
/// (XY) construction plane when no sketch is active. The caller checks live state and
/// prepends [`Instruction::BeginSketch`] before executing the returned instruction.
pub fn opens_sketch_when_none_active(name: &str) -> bool {
    matches!(name, "rect" | "line" | "circle" | "text")
}

/// The body a JSON ordinal names (#1055): a script counts live bodies, it cannot spell a key.
fn body_key_from_ordinal(
    doc: &crate::model::Document,
    ordinal: usize,
) -> Result<crate::model::BodyKey, String> {
    doc.body_at(ordinal)
        .ok_or_else(|| format!("no body {ordinal}"))
}

/// The line at `ordinal` among the live ones (#1055).
fn line_key_from_ordinal(
    doc: &crate::model::Document,
    ordinal: usize,
) -> Result<crate::model::LineKey, String> {
    doc.lines
        .keys()
        .nth(ordinal)
        .ok_or_else(|| format!("no line {ordinal}"))
}

/// The lines named by a list of ordinals (#1055).
fn line_keys_from_ordinals(
    doc: &crate::model::Document,
    ordinals: Vec<usize>,
) -> Result<Vec<crate::model::LineKey>, String> {
    ordinals
        .into_iter()
        .map(|o| line_key_from_ordinal(doc, o))
        .collect()
}

/// The unit instance at `ordinal` among the live ones (#1055).
fn unit_instance_key_from_ordinal(
    doc: &crate::model::Document,
    ordinal: usize,
) -> Result<crate::model::UnitInstanceKey, String> {
    doc.unit_instances
        .keys()
        .nth(ordinal)
        .ok_or_else(|| format!("no unit instance {ordinal}"))
}

/// The construction plane a script ordinal names (#1055).
fn plane_key_from_ordinal(
    doc: &crate::model::Document,
    ordinal: usize,
) -> Result<crate::model::ConstructionPlaneKey, String> {
    doc.construction_planes
        .keys()
        .nth(ordinal)
        .ok_or_else(|| format!("no construction plane {ordinal}"))
}

/// A whole scene element from a `(kind, index)` pair (mirrors `lua_script::
/// scene_element_from_kind`). Used to resolve `select`/`set_name`/`set_visible`/
/// `set_construction`/`find` element arguments in the stateful dispatch path.
pub fn scene_element_from_kind(
    doc: &crate::model::Document,
    kind: &str,
    index: usize,
) -> Option<SceneElement> {
    match kind.to_ascii_lowercase().as_str() {
        "plane" | "construction_plane" | "constructionplane" => Some(
            SceneElement::ConstructionPlane(doc.construction_planes.keys().nth(index)?),
        ),
        "sketch" => Some(SceneElement::Sketch(doc.sketches.keys().nth(index)?)),
        "line" => Some(SceneElement::Line(doc.lines.keys().nth(index)?)),
        "circle" => Some(SceneElement::Circle(doc.circles.keys().nth(index)?)),
        "constraint" => Some(SceneElement::Constraint(doc.constraints.keys().nth(index)?)),
        "extrusion" => Some(SceneElement::Extrusion(doc.extrusions.keys().nth(index)?)),
        "body" => Some(SceneElement::Body(doc.bodies.keys().nth(index)?)),
        "sketch_text" | "text" => {
            Some(SceneElement::SketchText(doc.sketch_texts.keys().nth(index)?))
        }
        "joint" => Some(SceneElement::Joint(doc.joints.keys().nth(index)?)),
        _ => None,
    }
}

/// The script kind name for any scene element (mirrors `lua_script::element_kind_name`), for
/// the `selection` query. Covers every variant, including the point/edge selectors that have
/// no flat `(kind, index)` handle.
pub fn scene_element_full_kind_name(element: &SceneElement) -> &'static str {
    match element {
        SceneElement::ConstructionPlane(_) => "construction_plane",
        SceneElement::Sketch(_) => "sketch",
        SceneElement::Line(_) => "line",
        SceneElement::Circle(_) => "circle",
        SceneElement::Constraint(_) => "constraint",
        SceneElement::Point(_) => "point",
        SceneElement::Extrusion(_) => "extrusion",
        SceneElement::Body(_) => "body",
        SceneElement::FaceEdge(_) => "face_edge",
        SceneElement::Origin => "origin",
        SceneElement::GlobalAxis(_) => "axis",
        SceneElement::BodyEdge { .. } => "body_edge",
        SceneElement::BodyVertex { .. } => "body_vertex",
        SceneElement::BodyFace { .. } => "body_face",
        SceneElement::BodyCylinder { .. } => "cylinder",
        SceneElement::BodyAxis { .. } => "body_axis",
        SceneElement::SketchFace(_) => "face",
        SceneElement::MovePoint(_) => "move_point",
        SceneElement::ExtrusionEdge { .. } => "extrusion_edge",
        SceneElement::PrimitiveEdge { .. } => "primitive_edge",
        SceneElement::RepeatedFace { .. } => "repeated_face",
        SceneElement::Image(_) => "image",
        SceneElement::BooleanOp(_) => "boolean_op",
        SceneElement::MoveOp(_) => "move_op",
        SceneElement::MirrorOp(_) => "mirror_op",
        SceneElement::RepeatOp(_) => "repeat_op",
        SceneElement::SketchRepeatOp(_) => "sketch_repeat_op",
        SceneElement::SketchOffsetOp(_) => "sketch_offset_op",
        SceneElement::SketchMirrorOp(_) => "sketch_mirror_op",
        SceneElement::SketchVertexTreatmentOp(_) => "sketch_vertex_treatment_op",
        SceneElement::SketchSliceOp(_) => "sketch_slice_op",
        SceneElement::SketchText(_) => "sketch_text",
        SceneElement::SliceOp(_) => "slice_op",
        SceneElement::ShellOp(_) => "shell_op",
        SceneElement::EdgeTreatmentOp(_) => "edge_treatment_op",
        SceneElement::Revolution(_) => "revolution",
        SceneElement::Shape(_) => "shape",
        SceneElement::SweepOp(_) => "sweep",
        SceneElement::Loft(_) => "loft",
        SceneElement::Component(_) => "component",
        SceneElement::UnitInstance(_) => "unit_instance",
        SceneElement::Joint(_) => "joint",
        // The drawing workbench's three page-item kinds (#363/#967).
        SceneElement::DrawingElement { element, .. } => {
            use crate::context::DrawingElementRef as D;
            match element {
                D::Projection(_) => "projection",
                D::Text(_) => "annotation",
                D::Dimension { .. } => "drawing_dimension",
            }
        }
    }
}

/// The index reported for a selected element (mirrors the `selection` query): the element's
/// index, or `None` for the point/edge selectors that name a sub-feature of another element
/// rather than a whole element (`Point`/`FaceEdge`).
pub fn scene_element_selection_index(
    doc: &crate::model::Document,
    element: &SceneElement,
) -> Option<usize> {
    match element {
        // An arena-backed element reports its **ordinal** among the live ones of its kind
        // (#1055) — the same integer `scene_element_from_kind` takes back.
        SceneElement::Image(key) => doc.tracing_images.keys().position(|k| k == *key),
        SceneElement::Body(key) => doc.bodies.keys().position(|k| k == *key),
        SceneElement::BooleanOp(key) => doc.boolean_ops.keys().position(|k| k == *key),
        SceneElement::MoveOp(key) => doc.move_ops.keys().position(|k| k == *key),
        SceneElement::MirrorOp(key) => doc.mirror_ops.keys().position(|k| k == *key),
        SceneElement::RepeatOp(key) => doc.repeat_ops.keys().position(|k| k == *key),
        SceneElement::SliceOp(key) => doc.slice_ops.keys().position(|k| k == *key),
        SceneElement::ShellOp(key) => doc.shell_ops.keys().position(|k| k == *key),
        SceneElement::SketchRepeatOp(key) => {
            doc.sketch_repeat_ops.keys().position(|k| k == *key)
        }
        SceneElement::SketchOffsetOp(key) => {
            doc.sketch_offset_ops.keys().position(|k| k == *key)
        }
        SceneElement::SketchMirrorOp(key) => {
            doc.sketch_mirror_ops.keys().position(|k| k == *key)
        }
        SceneElement::SketchVertexTreatmentOp(key) => {
            doc.sketch_vertex_treatment_ops.keys().position(|k| k == *key)
        }
        SceneElement::SketchSliceOp(key) => {
            doc.sketch_slice_ops.keys().position(|k| k == *key)
        }
        SceneElement::EdgeTreatmentOp(key) => {
            doc.edge_treatment_ops.keys().position(|k| k == *key)
        }
        SceneElement::Revolution(key) => doc.revolutions.keys().position(|k| k == *key),
        SceneElement::SweepOp(key) => doc.sweeps.keys().position(|k| k == *key),
        SceneElement::Loft(key) => doc.lofts.keys().position(|k| k == *key),
        SceneElement::Shape(key) => doc.primitives.keys().position(|k| k == *key),
        // A body face (#555) names a sub-feature with no flat index, like Point/FaceEdge.
        SceneElement::Point(_)
        | SceneElement::FaceEdge(_)
        | SceneElement::BodyFace { .. }
        // A cylinder and its centre line are keyed by geometry, not by an index (#1013).
        | SceneElement::BodyCylinder { .. }
        | SceneElement::BodyAxis { .. }
        | SceneElement::SketchFace(_)
        | SceneElement::MovePoint(_) => None,
        SceneElement::ExtrusionEdge { extrusion, .. } => {
            doc.extrusions.keys().position(|k| k == *extrusion)
        }
        SceneElement::PrimitiveEdge { primitive, .. } => {
            doc.primitives.keys().position(|k| k == *primitive)
        }
        SceneElement::RepeatedFace { instance, .. } => Some(*instance),
        // A page item indexes by its place on the page; a dimension has no index of its own,
        // so it reports the view it is shown on (#967).
        SceneElement::DrawingElement { drawing, element } => {
            use crate::context::DrawingElementRef as D;
            Some(match element {
                D::Projection(i) => *i,
                D::Text(key) => doc
                    .drawings
                    .get(*drawing)
                    .and_then(|d| d.annotations.keys().position(|k| k == *key))?,
                D::Dimension { view, .. } => *view,
            })
        }
        // X/Y/Z report as 0/1/2 (#952), matching `lua_script::element_index`.
        SceneElement::GlobalAxis(axis) => Some(match axis {
            crate::construction::GlobalAxis::X => 0,
            crate::construction::GlobalAxis::Y => 1,
            crate::construction::GlobalAxis::Z => 2,
        }),
        SceneElement::Line(key) => doc.lines.keys().position(|k| k == *key),
        SceneElement::ConstructionPlane(key) => {
            doc.construction_planes.keys().position(|k| k == *key)
        }
        SceneElement::Circle(key) => doc.circles.keys().position(|k| k == *key),
        SceneElement::Sketch(key) => doc.sketches.keys().position(|k| k == *key),
        SceneElement::Constraint(key) => doc.constraints.keys().position(|k| k == *key),
        SceneElement::SketchText(key) => doc.sketch_texts.keys().position(|k| k == *key),
        SceneElement::Extrusion(key) => doc.extrusions.keys().position(|k| k == *key),
        SceneElement::Component(key) => doc.components.keys().position(|k| k == *key),
        SceneElement::UnitInstance(key) => doc.unit_instances.keys().position(|k| k == *key),
        SceneElement::Joint(key) => doc.joints.keys().position(|k| k == *key),
        SceneElement::Origin
        | SceneElement::BodyEdge { .. }
        | SceneElement::BodyVertex { .. } => Some(0),
    }
}

/// The script name for a whole scene element's kind, for `find`'s return value. `None` for
/// element variants that `scene_element_from_kind` can't round-trip. Arena-backed kinds
/// report their ordinal among the live ones (#1055), so this needs the document.
pub fn scene_element_kind_name(
    doc: &Document,
    element: &SceneElement,
) -> Option<(&'static str, usize)> {
    let kind = match element {
        SceneElement::ConstructionPlane(_) => "plane",
        SceneElement::Sketch(_) => "sketch",
        SceneElement::Line(_) => "line",
        SceneElement::Circle(_) => "circle",
        SceneElement::Constraint(_) => "constraint",
        SceneElement::Extrusion(_) => "extrusion",
        SceneElement::Body(_) => "body",
        SceneElement::Joint(_) => "joint",
        _ => return None,
    };
    Some((kind, scene_element_selection_index(doc, element)?))
}

/// Map a positional argument list to the named-argument object the dispatcher expects.
///
/// Many desktop verbs take positional arguments (`bearcad.tool("circle")`,
/// `bearcad.orbit(dx, dy)`) rather than a table. The web prelude forwards those as an
/// `__args` array (see `cpp/bearcad_lua.cpp`); this assigns them to the same keys the table
/// form uses, so both call styles reach the identical [`instruction_from_json`] path. Keys
/// are positional and trailing ones may be omitted (a missing optional argument). A verb with
/// no positional form here reports that it needs a table.
pub fn positional_to_named(name: &str, args: &[Value]) -> Result<Value, String> {
    let keys: &[&str] = match name {
        "tool" => &["name"],
        "open" | "import_stl" | "import_step" | "import_lua" => &["path"],
        "save" => &["path"],
        "export_stl" | "export_step" | "export_3mf" => &["path", "body"],
        "open_sketch" => &["sketch"],
        "begin_sketch" => &["kind", "index"],
        "count" => &["kind"],
        "body_stats" => &["index"],
        "shading" | "ground" | "elements_view" => &["mode"],
        "pane" => &["pane", "visible"],
        "orbit" | "pan" | "fps_look" => &["dx", "dy"],
        "wheel" => &["scroll"],
        "fps" | "fps_fly" => &["on"],
        "fps_advance" => &["seconds"],
        "fps_scale" => &["scale"],
        "set_dim" => &["axis", "value"],
        "focus_dim" | "edit_dim" => &["axis"],
        "set_dim_label_offset" => &["axis", "offset"],
        "add_geometric_constraint" => &["name"],
        "constraint_shortcut" => &["key"],
        "add_constraint" => &["target", "expression"],
        "view" => &["view", "id"],
        "palette" => &["action", "query"],
        "select" => &["element", "additive"],
        "set_name" => &["element", "name"],
        "set_visible" => &["element", "visible"],
        "set_construction" => &["element", "construction"],
        "find" => &["name"],
        "set_joint_rest" | "revert_joint" => &["index"],
        "revert_joints" => &[],
        _ => return Err(format!("'{name}' expects named arguments (a table)")),
    };
    let mut map = Map::new();
    for (i, key) in keys.iter().enumerate() {
        match args.get(i) {
            None | Some(Value::Null) => {}
            Some(v) => {
                map.insert((*key).to_string(), v.clone());
            }
        }
    }
    Ok(Value::Object(map))
}

/// Translate one `bearcad.<name>{ ...args }` call into its [`Instruction`]. `args` is the
/// JSON object of named arguments (an empty object for no-arg calls). Returns a
/// human-readable message for an unknown command or a bad argument, which the web runner
/// surfaces the way native mlua raises a Lua error.
///
/// Coverage is every `bearcad.*` verb whose `Instruction` is a pure function of its
/// arguments: the document/IO actions, tool actions, 2D primitives, and the declarative
/// modeling ops (revolve, loft, booleans, move, repeat, slice, project, and their `edit_*` forms).
///
/// `extrude`/`extrude_face` are intentionally absent: their closures read the live document
/// to infer the owning sketch (`extrude_face_sketch`) before building the `Instruction`, so
/// they can't be a pure `(name, args)` function — they belong to the stateful dispatch path
/// alongside the query getters. Likewise the read-back getters (`get`/`count`/`selection`/
/// `body_stats`/`sketch_dof`) return JSON data rather than an `Instruction`.
pub fn instruction_from_json(
    doc: &crate::model::Document,
    name: &str,
    args: &Value,
) -> Result<Instruction, String> {
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
            let face = FaceId::from_script(doc, &kind, index)
                .ok_or_else(|| format!("unknown sketch face kind '{kind}'"))?;
            Ok(Instruction::BeginSketch { face })
        }
        "plane" => Ok(Instruction::CreatePlane {
            offset: opt_f32(o, "offset")?.unwrap_or(0.0),
            from: opt_usize(o, "from")?.unwrap_or(0),
        }),
        "rect" => {
            let (width, width_expr) =
                opt_scalar(o, "width")?.ok_or("rect requires `width`")?;
            let (height, height_expr) =
                opt_scalar(o, "height")?.ok_or("rect requires `height`")?;
            Ok(Instruction::CreateRect {
                x: opt_f32(o, "x")?.unwrap_or(0.0),
                y: opt_f32(o, "y")?.unwrap_or(0.0),
                width,
                height,
                width_expr,
                height_expr,
            })
        }
        "circle" => {
            let cx = opt_f32(o, "x")?.unwrap_or(0.0);
            let cy = opt_f32(o, "y")?.unwrap_or(0.0);
            // Same precedence as the mlua closure: `r`, then `radius`, then `diameter`.
            // Each accepts a parameter expression too (#402); a radius expression doubles
            // into the diameter constraint.
            let (r, diameter_expr) = if let Some((r, e)) = opt_scalar(o, "r")? {
                (r, e.map(|e| format!("({e}) * 2")))
            } else if let Some((radius, e)) = opt_scalar(o, "radius")? {
                (radius, e.map(|e| format!("({e}) * 2")))
            } else if let Some((d, e)) = opt_scalar(o, "diameter")? {
                (d * 0.5, e)
            } else {
                return Err("circle requires a size: one of `r`, `radius`, or `diameter`".into());
            };
            Ok(Instruction::CreateCircle { cx, cy, r, diameter_expr })
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
        "text" => {
            // Size accepts a number or an expression string, like the mlua closure.
            let size = match o.get("size") {
                None | Some(Value::Null) => "10".to_string(),
                Some(Value::Number(n)) => n.to_string(),
                Some(Value::String(s)) => s.clone(),
                Some(other) => {
                    return Err(format!(
                        "text size must be a number or expression string, got {other}"
                    ))
                }
            };
            Ok(Instruction::CreateSketchText {
                text: req_str(o, "text", "text")?,
                font: opt_str(o, "font")?,
                bold: opt_bool(o, "bold")?.unwrap_or(false),
                italic: opt_bool(o, "italic")?.unwrap_or(false),
                underline: opt_bool(o, "underline")?.unwrap_or(false),
                size,
                x: opt_f32(o, "x")?.unwrap_or(0.0),
                y: opt_f32(o, "y")?.unwrap_or(0.0),
                rotation_deg: opt_f32(o, "rotation")?.unwrap_or(0.0),
                wrap: opt_f32(o, "wrap")?,
            })
        }

        // ----- File / import-export (mirrors the desktop closures, which take positional
        // path strings; over JSON every argument is named). -----
        "open" => Ok(Instruction::Open(req_str(o, "path", "open")?)),
        "save" => Ok(Instruction::Save(opt_str(o, "path")?)),
        "rebuild_geometry" => Ok(Instruction::RebuildGeometry),
        "export_stl" => Ok(Instruction::ExportStl {
            path: req_str(o, "path", "export_stl")?,
            body: opt_str(o, "body")?,
        }),
        "export_3mf" => Ok(Instruction::Export3mf {
            path: req_str(o, "path", "export_3mf")?,
            body: opt_str(o, "body")?,
        }),
        "export_step" => Ok(Instruction::ExportStep {
            path: req_str(o, "path", "export_step")?,
            body: opt_str(o, "body")?,
        }),
        "export_preview" => Ok(Instruction::ExportPreview {
            path: req_str(o, "path", "export_preview")?,
        }),
        "import_stl" => Ok(Instruction::ImportStl { path: req_str(o, "path", "import_stl")? }),
        "import_step" => Ok(Instruction::ImportStep { path: req_str(o, "path", "import_step")? }),
        "import_lua" => Ok(Instruction::ImportLua {
            path: req_str(o, "path", "import_lua")?,
            force: match o.get("force") {
                None | Some(serde_json::Value::Null) => false,
                Some(serde_json::Value::Bool(b)) => *b,
                Some(other) => {
                    return Err(format!("import_lua force must be a boolean, got {other}"));
                }
            },
        }),
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
            let faces = collect_profile_faces(doc, o, false)?;
            if faces.is_empty() {
                return Err("revolve requires a `circle`/`circles`/`polygon` face".into());
            }
            let axis = match o.get("axis") {
                None | Some(Value::Null) => {
                    return Err("revolve requires `axis` (\"x\"|\"y\"|\"z\" or {line = i})".into())
                }
                Some(v) => revolve_axis_from_value(doc, v)?,
            };
            // Angle (degrees) or revolutions (turns × 360); revolutions wins if both given (#1242).
            let angle_deg = if let Some(turns) = opt_f32(o, "revolutions")? {
                turns * 360.0
            } else {
                opt_f32(o, "angle")?.unwrap_or(360.0)
            };
            // Helical pitch: `pitch`/`offset` is start-to-start; `gap` is clear gap (no axial
            // extent correction here — scripts use pitch/offset for the stored value) (#1242).
            let pitch_mm = opt_f32(o, "pitch")?
                .or(opt_f32(o, "offset")?)
                .or(opt_f32(o, "gap")?)
                .unwrap_or(0.0);
            let symmetric = opt_bool(o, "symmetric")?.unwrap_or(false);
            let bodies = usize_list(o, "bodies")?;
            // Same mapping as the closure: "add"→AddTouching, "cut"→Cut, else NewBody.
            let body = match opt_str(o, "body")?.as_deref() {
                Some("add") => RevolveBodyChoice::AddTouching,
                Some("cut") => RevolveBodyChoice::Cut,
                _ => RevolveBodyChoice::NewBody,
            };
            Ok(Instruction::Revolve {
                faces,
                axis,
                angle_deg,
                pitch_mm,
                symmetric,
                body,
                bodies,
            })
        }
        "loft" => {
            let faces = collect_profile_faces(doc, o, true)?;
            if faces.len() < 2 {
                return Err("loft requires at least two sections (`circles`/`polygons`)".into());
            }
            let bodies = usize_list(o, "bodies")?;
            let body = match opt_str(o, "body")?.as_deref() {
                Some("add") => RevolveBodyChoice::AddTouching,
                Some("cut") => RevolveBodyChoice::Cut,
                _ => RevolveBodyChoice::NewBody,
            };
            Ok(Instruction::Loft { faces, body, bodies })
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
            let (targets, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin,
                 face_offset, start_point_a, end_point_a, start_point_b, end_point_b,
                 start_point_c, end_point_c) =
                move_op_args(doc, o)?;
            Ok(Instruction::CreateMoveOp { targets, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin, face_offset, start_point_a, end_point_a, start_point_b, end_point_b, start_point_c, end_point_c })
        }
        "joint" => {
            let (members, base, kind, placement, frame, position, position2, position3, limits) =
                joint_op_args(doc, o)?;
            Ok(Instruction::CreateJointOp { members, base, kind, placement, frame, position, position2, position3, limits })
        }
        "begin_joint" => {
            let (members, base, kind, placement, frame, position, position2, position3, limits) =
                joint_op_args(doc, o)?;
            Ok(Instruction::BeginJointOp { members, base, kind, placement, frame, position, position2, position3, limits })
        }
        "set_joint_rest" => Ok(Instruction::SetJointRest {
            op: req_usize(o, "index", "set_joint_rest")?,
        }),
        "revert_joint" => Ok(Instruction::RevertJoint {
            op: req_usize(o, "index", "revert_joint")?,
        }),
        "revert_joints" => Ok(Instruction::RevertAllJoints),
        "edit_joint" => {
            let op = req_usize(o, "index", "edit_joint")?;
            let (members, base, kind, placement, frame, position, position2, position3, limits) =
                joint_op_args(doc, o)?;
            Ok(Instruction::EditJointOp { op, members, base, kind, placement, frame, position, position2, position3, limits })
        }
        "begin_move" => {
            let (targets, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin,
                 face_offset, start_point_a, end_point_a, start_point_b, end_point_b,
                 start_point_c, end_point_c) =
                move_op_args(doc, o)?;
            Ok(Instruction::BeginMoveOp { targets, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin, face_offset, start_point_a, end_point_a, start_point_b, end_point_b, start_point_c, end_point_c })
        }
        "edit_move" => {
            let op = req_usize(o, "index", "edit_move")?;
            let (targets, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin,
                 face_offset, start_point_a, end_point_a, start_point_b, end_point_b,
                 start_point_c, end_point_c) =
                move_op_args(doc, o)?;
            Ok(Instruction::EditMoveOp { op, targets, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin, face_offset, start_point_a, end_point_a, start_point_b, end_point_b, start_point_c, end_point_c })
        }
        "mirror_bodies" => {
            let (plane, targets, mode) = mirror_op_args(doc, o)?;
            Ok(Instruction::CreateMirrorOp { plane, targets, mode })
        }
        "edit_mirror" => {
            let op = req_usize(o, "index", "edit_mirror")?;
            let (plane, targets, mode) = mirror_op_args(doc, o)?;
            Ok(Instruction::EditMirrorOp { op, plane, targets, mode })
        }
        "repeat_bodies" => {
            let (targets, axis, around_axis, flip, mode, count, spacing, length, length_target) =
                repeat_op_args(doc, o)?;
            Ok(Instruction::CreateRepeatOp { targets, axis, around_axis, flip, mode, count, spacing, length, length_target })
        }
        "edit_repeat" => {
            let op = req_usize(o, "index", "edit_repeat")?;
            let (targets, axis, around_axis, flip, mode, count, spacing, length, length_target) =
                repeat_op_args(doc, o)?;
            Ok(Instruction::EditRepeatOp { op, targets, axis, around_axis, flip, mode, count, spacing, length, length_target })
        }
        "slice" => {
            let (targets, cutters, extend_infinite) = slice_op_args(doc, o)?;
            Ok(Instruction::CreateSliceOp { targets, cutters, extend_infinite })
        }
        "edit_slice" => {
            let op = req_usize(o, "index", "edit_slice")?;
            let (targets, cutters, extend_infinite) = slice_op_args(doc, o)?;
            Ok(Instruction::EditSliceOp { op, targets, cutters, extend_infinite })
        }
        "project" => Ok(Instruction::Project {
            elements: parse_project_elements(doc, o)?,
        }),

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
                target: distance_target_from_json(doc, target)?,
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
        "apply_visibility" => Ok(Instruction::ApplySelectionVisibility {
            visible: req_bool_flag(o, "visible", "apply_visibility")?,
        }),
        "toggle_visibility" => Ok(Instruction::ToggleSelectionVisibility),
        "clear_selection" => Ok(Instruction::ClearSceneSelection),
        "delete_selection" => Ok(Instruction::DeleteSelection),

        // ----- Chamfer/fillet a sketch vertex (#37/#38) or an extrusion's 3D edge (#77). -----
        "chamfer_vertex" | "fillet_vertex" => {
            let point = constraint_point_from_json(
                doc,
                o.get("point").ok_or_else(|| format!("{name} requires a `point`"))?,
            )?;
            let (kind, amount_key) = if name == "chamfer_vertex" {
                (VertexTreatmentKind::Chamfer, "distance")
            } else {
                (VertexTreatmentKind::Fillet, "radius")
            };
            Ok(Instruction::VertexTreatment {
                point,
                kind,
                amount: req_amount_expr(o, amount_key, name)?,
            })
        }
        "chamfer_edge" | "fillet_edge" => {
            let (kind, amount_key) = if name == "chamfer_edge" {
                (VertexTreatmentKind::Chamfer, "distance")
            } else {
                (VertexTreatmentKind::Fillet, "radius")
            };
            Ok(Instruction::EdgeTreatment {
                edges: extrusion_edge_set_from_json(o, name)?,
                kind,
                amount: req_f32(o, amount_key, name)?,
            })
        }

        // ----- Camera / view navigation (the `bearcad.ui.*` verbs). -----
        "orbit" => Ok(Instruction::Orbit {
            dx: req_f32(o, "dx", "orbit")?,
            dy: req_f32(o, "dy", "orbit")?,
        }),
        "pan" => Ok(Instruction::Pan {
            dx: req_f32(o, "dx", "pan")?,
            dy: req_f32(o, "dy", "pan")?,
        }),
        "wheel" => Ok(Instruction::Zoom { scroll: req_f32(o, "scroll", "wheel")? }),
        "view" => {
            // `view` names a projection mode, "edge"/"corner" (+ an `id`), or a standard view —
            // the same dispatch order as the `_view` closure.
            let name = req_str(o, "view", "view")?;
            if let Some(mode) = ProjectionMode::from_name(&name) {
                return Ok(Instruction::ProjectionMode(mode));
            }
            if name.eq_ignore_ascii_case("edge") {
                let id = req_str(o, "id", "view edge")?;
                let edge = CubeEdgeId::from_name(&id)
                    .ok_or_else(|| format!("unknown view edge '{id}'"))?;
                return Ok(Instruction::ViewEdge(edge));
            }
            if name.eq_ignore_ascii_case("corner") {
                let id = req_str(o, "id", "view corner")?;
                let corner = CubeCornerId::from_name(&id)
                    .ok_or_else(|| format!("unknown view corner '{id}'"))?;
                return Ok(Instruction::ViewCorner(corner));
            }
            let view = StandardView::from_name(&name)
                .ok_or_else(|| format!("unknown standard view '{name}'"))?;
            Ok(Instruction::View(view))
        }
        "view_home" => Ok(Instruction::ViewHome),
        "set_home_view" => Ok(Instruction::SetHomeView),
        "toggle_projection" => Ok(Instruction::ToggleProjectionMode),
        "shading" => {
            let name = req_str(o, "mode", "shading")?;
            let mode = ShadingMode::from_name(&name)
                .ok_or_else(|| format!("unknown shading mode '{name}'"))?;
            Ok(Instruction::ShadingMode(mode))
        }
        "ground" => {
            let name = req_str(o, "mode", "ground")?;
            let mode = GroundDisplay::from_name(&name)
                .ok_or_else(|| format!("unknown ground display '{name}'"))?;
            Ok(Instruction::GroundDisplay(mode))
        }
        "camera" => {
            let yaw = opt_f32(o, "yaw")?;
            let pitch = opt_f32(o, "pitch")?;
            let distance = opt_f32(o, "distance")?;
            let target = match o.get("target") {
                None | Some(Value::Null) => None,
                Some(_) => Some(xyz(o, "target")?),
            };
            // With no pose keys the closure is a pure read of the live camera — that path
            // needs `AppState`, so it belongs to the stateful dispatcher, not here.
            if yaw.is_none() && pitch.is_none() && distance.is_none() && target.is_none() {
                return Err("camera with no pose keys is a query, not an action".into());
            }
            Ok(Instruction::SetCamera { yaw, pitch, distance, target })
        }
        "zoom_fit" => Ok(Instruction::ZoomFit),
        "elements_view" => {
            let name = req_str(o, "mode", "elements_view")?;
            let mode = HierarchyViewMode::from_name(&name).ok_or_else(|| {
                format!("unknown elements view '{name}' (expected 'list', 'tree', or 'graph')")
            })?;
            Ok(Instruction::SetElementsView { mode })
        }
        "pane" => {
            let pane = req_str(o, "pane", "pane")?;
            let pane = Pane::from_name(&pane).ok_or_else(|| format!("unknown pane '{pane}'"))?;
            Ok(Instruction::SetPane { pane, visible: visibility(o.get("visible"))? })
        }
        "palette" => match opt_str(o, "action")?.as_deref() {
            None | Some("toggle") => Ok(Instruction::SetCommandPalette { open: None }),
            Some("run") => Ok(Instruction::RunPaletteCommand {
                query: req_str(o, "query", "palette run")?,
                // What a command that prompts for an argument (#1022) would have been given.
                argument: opt_str(o, "argument")?,
            }),
            Some("show") | Some("open") => {
                Ok(Instruction::SetCommandPalette { open: Some(true) })
            }
            Some("hide") | Some("close") => {
                Ok(Instruction::SetCommandPalette { open: Some(false) })
            }
            Some(other) => Err(format!("unknown palette action '{other}'")),
        },

        // ----- First-person mode (#91). -----
        "fps" => Ok(Instruction::FpsMode { on: opt_bool(o, "on")? }),
        "fps_look" => Ok(Instruction::FpsLook {
            dx: req_f32(o, "dx", "fps_look")?,
            dy: req_f32(o, "dy", "fps_look")?,
        }),
        "fps_move" => Ok(Instruction::FpsMove {
            forward: opt_f32(o, "forward")?.unwrap_or(0.0),
            strafe: opt_f32(o, "strafe")?.unwrap_or(0.0),
        }),
        "fps_jump" => Ok(Instruction::FpsJump),
        "fps_fly" => Ok(Instruction::FpsFly { on: opt_bool(o, "on")? }),
        "fps_advance" => Ok(Instruction::FpsAdvance { seconds: req_f32(o, "seconds", "fps_advance")? }),
        "fps_scale" => Ok(Instruction::FpsScale { scale: req_f32(o, "scale", "fps_scale")? }),

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
            let drawing = req_usize(o, "drawing", "drawing_view")?;
            // A view projects a body, several bodies, a component, or a sketch (#278/#403/#1190/#1191).
            let body = opt_usize(o, "body")?;
            let bodies = match o.get("bodies") {
                Some(Value::Array(_)) => Some(usize_list(o, "bodies")?),
                Some(_) => return Err("drawing_view `bodies` must be an array".into()),
                None => None,
            };
            let component = opt_usize(o, "component")?;
            let sketch = opt_usize(o, "sketch")?;
            let source_count = usize::from(body.is_some())
                + usize::from(bodies.is_some())
                + usize::from(component.is_some())
                + usize::from(sketch.is_some());
            if source_count != 1 {
                return Err(
                    "drawing_view requires exactly one of `body`, `bodies`, `component`, or `sketch`"
                        .into(),
                );
            }
            if let Some(sketch) = sketch {
                return Ok(Instruction::AddDrawingSketchView {
                    drawing,
                    sketch,
                    orientation,
                });
            }
            let bodies = if let Some(body) = body {
                vec![body]
            } else if let Some(bodies) = bodies {
                if bodies.is_empty() {
                    return Err("drawing_view `bodies` must not be empty".into());
                }
                bodies
            } else {
                // Component ordinal → every body currently inside it (#1190).
                let ci = component.expect("component set");
                let Some(ck) = doc.components.keys().nth(ci) else {
                    return Err(format!("No component {ci}"));
                };
                // Expand via ownership the same way export does — without needing AppState.
                let bodies: Vec<usize> = doc
                    .bodies
                    .keys()
                    .enumerate()
                    .filter(|(_, bi)| {
                        crate::hierarchy::owning_component(
                            doc,
                            &crate::hierarchy::SceneElement::Body(*bi),
                        )
                        .is_some_and(|owner| doc.component_chain(owner).contains(&ck))
                    })
                    .map(|(ord, _)| ord)
                    .collect();
                if bodies.is_empty() {
                    return Err("This component has no bodies to project".into());
                }
                bodies
            };
            Ok(Instruction::AddDrawingView {
                drawing,
                bodies,
                orientation,
            })
        }
        "drawing_view_add" => {
            let drawing = req_usize(o, "drawing", "drawing_view_add")?;
            let view = req_usize(o, "view", "drawing_view_add")?;
            let body = opt_usize(o, "body")?;
            let bodies = match o.get("bodies") {
                Some(Value::Array(_)) => Some(usize_list(o, "bodies")?),
                Some(_) => return Err("drawing_view_add `bodies` must be an array".into()),
                None => None,
            };
            let component = opt_usize(o, "component")?;
            let source_count = usize::from(body.is_some())
                + usize::from(bodies.is_some())
                + usize::from(component.is_some());
            if source_count != 1 {
                return Err(
                    "drawing_view_add requires exactly one of `body`, `bodies`, or `component`"
                        .into(),
                );
            }
            let bodies = if let Some(body) = body {
                vec![body]
            } else if let Some(bodies) = bodies {
                if bodies.is_empty() {
                    return Err("drawing_view_add `bodies` must not be empty".into());
                }
                bodies
            } else {
                let ci = component.expect("component set");
                let Some(ck) = doc.components.keys().nth(ci) else {
                    return Err(format!("No component {ci}"));
                };
                let bodies: Vec<usize> = doc
                    .bodies
                    .keys()
                    .enumerate()
                    .filter(|(_, bi)| {
                        crate::hierarchy::owning_component(
                            doc,
                            &crate::hierarchy::SceneElement::Body(*bi),
                        )
                        .is_some_and(|owner| doc.component_chain(owner).contains(&ck))
                    })
                    .map(|(ord, _)| ord)
                    .collect();
                if bodies.is_empty() {
                    return Err("This component has no bodies to project".into());
                }
                bodies
            };
            Ok(Instruction::AddBodiesToDrawingView {
                drawing,
                view,
                bodies,
            })
        }
        "drawing_page" => Ok(Instruction::SetDrawingPage {
            drawing: req_usize(o, "drawing", "drawing_page")?,
            width_mm: opt_f32(o, "width")?,
            height_mm: opt_f32(o, "height")?,
            margin_mm: opt_f32(o, "margin")?,
        }),
        "export_drawing_svg" => Ok(Instruction::ExportDrawingSvg {
            drawing: req_usize(o, "drawing", "export_drawing_svg")?,
            path: req_str(o, "path", "export_drawing_svg")?,
        }),
        "export_drawing_pdf" => Ok(Instruction::ExportDrawingPdf {
            drawing: req_usize(o, "drawing", "export_drawing_pdf")?,
            path: req_str(o, "path", "export_drawing_pdf")?,
        }),
        "drawing_dimension" => Ok(Instruction::ToggleDrawingDimension {
            drawing: req_usize(o, "drawing", "drawing_dimension")?,
            view: req_usize(o, "view", "drawing_dimension")?,
            a: xyz(o, "a")?,
            b: xyz(o, "b")?,
        }),
        "drawing_circle_dimension" => Ok(Instruction::ToggleDrawingCircleDimension {
            drawing: req_usize(o, "drawing", "drawing_circle_dimension")?,
            view: req_usize(o, "view", "drawing_circle_dimension")?,
            center: xyz(o, "center")?,
        }),
        "drawing_dim_offset" => Ok(Instruction::SetDrawingDimensionOffset {
            drawing: req_usize(o, "drawing", "drawing_dim_offset")?,
            view: req_usize(o, "view", "drawing_dim_offset")?,
            a: xyz(o, "a")?,
            b: xyz(o, "b")?,
            offset: opt_f32(o, "offset")?,
        }),
        "drawing_circle_dim_offset" => Ok(Instruction::SetDrawingCircleDimOffset {
            drawing: req_usize(o, "drawing", "drawing_circle_dim_offset")?,
            view: req_usize(o, "view", "drawing_circle_dim_offset")?,
            center: xyz(o, "center")?,
            offset: opt_f32(o, "offset")?,
        }),
        "drawing_view_align_lines" => Ok(Instruction::SetDrawingViewAlignLines {
            drawing: req_usize(o, "drawing", "drawing_view_align_lines")?,
            view: req_usize(o, "view", "drawing_view_align_lines")?,
            show: opt_bool(o, "show")?.ok_or("drawing_view_align_lines requires `show`")?,
        }),
        "drawing_view_label" => Ok(Instruction::SetDrawingViewLabel {
            drawing: req_usize(o, "drawing", "drawing_view_label")?,
            view: req_usize(o, "view", "drawing_view_label")?,
            hidden: o.get("hidden").and_then(|v| v.as_bool()),
            pos: opt_str(o, "pos")?,
            text: opt_str(o, "text")?,
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

/// Parses a `visible` argument into `Some(true|false)` (show/hide) or `None` (toggle),
/// mirroring the mlua `parse_visibility`: a boolean, one of the show/hide string aliases, or
/// `"toggle"`/absent for a toggle.
fn visibility(v: Option<&Value>) -> Result<Option<bool>, String> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(Value::String(s)) => match s.to_ascii_lowercase().as_str() {
            "show" | "on" | "true" | "yes" | "1" => Ok(Some(true)),
            "hide" | "off" | "false" | "no" | "0" => Ok(Some(false)),
            "toggle" => Ok(None),
            other => Err(format!("unknown visibility value '{other}'")),
        },
        Some(_) => Err("expected boolean or string for visibility".into()),
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

/// The doc-dependent extrude verbs (`extrude`/`extrude_face`/`edit_extrusion`): unlike the
/// pure verbs, these read the live document — `extrude` infers the owning sketch from the
/// first face's geometry, and `edit_extrusion`'s `by` delta reads the extrusion's current
/// effective depth — so they take `doc` and live on the stateful dispatch path
/// ([`crate::web_lua`]) rather than in [`instruction_from_json`].
pub fn extrude_instruction(name: &str, args: &Value, doc: &Document) -> Result<Instruction, String> {
    let o = as_object(args)?;
    match name {
        "extrude" => {
            let target = extrude_target_opt(doc, o)?;
            // `distance` accepts a plain number or a parameter expression string (#402).
            let (distance, expression) = match opt_scalar(o, "distance")? {
                Some(d) => d,
                None if target.is_some() => (0.0, None),
                None => return Err("extrude requires a `distance` or `to`".into()),
            };
            let mut faces = Vec::new();
            // A script names a circle by its ordinal among the live ones (#1055).
            let circle_key = |ordinal: usize| {
                doc.circles
                    .keys()
                    .nth(ordinal)
                    .ok_or_else(|| format!("no circle {ordinal}"))
            };
            if let Some(i) = opt_usize(o, "circle")? {
                faces.push(ExtrudeFace::Circle(circle_key(i)?));
            }
            for i in usize_list(o, "circles")? {
                faces.push(ExtrudeFace::Circle(circle_key(i)?));
            }
            if let Some(lines) = opt_usize_array(o, "polygon")? {
                faces.push(ExtrudeFace::Polygon(line_keys_from_ordinals(doc, lines)?));
            }
            if let Some(b) = o.get("boolean") {
                if !b.is_null() {
                    faces.push(boolean_face_from_json(doc, b)?);
                }
            }
            if faces.is_empty() {
                return Err(
                    "extrude requires a `circle`/`polygon`/`boolean` or `circles` face list".into(),
                );
            }
            let body = body_choice(o);
            // The instruction names the sketch by its ordinal (#1055).
            let sketch = crate::actions::extrude_face_sketch(doc, &faces[0])
                .and_then(|key| doc.sketches.keys().position(|k| k == key))
                .ok_or("extrude face does not exist")?;
            let symmetric = opt_bool(o, "symmetric")?.unwrap_or(false);
            let taper_mode = match o.get("taper_mode").and_then(|v| v.as_str()) {
                None => crate::model::ExtrudeTaperMode::Distance,
                Some(s) => crate::model::ExtrudeTaperMode::from_name(s)
                    .ok_or_else(|| format!("unknown taper_mode '{s}' (distance|angle)"))?,
            };
            let (taper, taper_expression) = match opt_scalar(o, "taper")? {
                Some((v, e)) => (v, e),
                None => (0.0, None),
            };
            Ok(Instruction::Extrude {
                sketch,
                faces,
                distance,
                body,
                target,
                expression,
                symmetric,
                taper,
                taper_mode,
                taper_expression,
            })
        }
        "extrude_face" => {
            let face = face_id_from_json(
                doc,
                o.get("face").ok_or("extrude_face requires a `face` table")?,
            )?;
            let target = extrude_target_opt(doc, o)?;
            let distance = match opt_f32(o, "distance")? {
                Some(d) => d,
                None if target.is_some() => 0.0,
                None => return Err("extrude_face requires a `distance` or `to`".into()),
            };
            Ok(Instruction::ExtrudeBodyFace { face, distance, body: body_choice(o), target })
        }
        "edit_extrusion" => {
            let extrusion = req_usize(o, "extrusion", "edit_extrusion")?;
            // `distance` accepts a plain number or a parameter expression string (#402).
            let (mut distance, expression) = match opt_scalar(o, "distance")? {
                Some((d, e)) => (Some(d), e),
                None => (None, None),
            };
            let by = opt_f32(o, "by")?;
            let target = extrude_target_opt(doc, o)?;
            if let Some(by) = by {
                if distance.is_some() {
                    return Err("edit_extrusion takes `distance` or `by`, not both".into());
                }
                let ext = doc
                    .extrusions
                    .keys()
                    .nth(extrusion)
                    .map(|k| &doc.extrusions[k])
                    .ok_or_else(|| format!("no extrusion {extrusion}"))?;
                distance = Some(crate::extrude::effective_distance(doc, ext) + by);
            }
            if distance.is_none() && target.is_none() {
                return Err("edit_extrusion requires `distance`, `by`, or `to`".into());
            }
            Ok(Instruction::UpdateExtrusion { extrusion, distance, target, expression })
        }
        other => Err(format!("unknown extrude verb '{other}'")),
    }
}

/// `body = "merge" | "cut"` attaches the extrusion (else a new body), matching the closures.
fn body_choice(o: &Map<String, Value>) -> ExtrudeBodyChoice {
    match o.get("body").and_then(Value::as_str) {
        Some("merge") => ExtrudeBodyChoice::Merge,
        Some("cut") => ExtrudeBodyChoice::Cut,
        _ => ExtrudeBodyChoice::New,
    }
}

/// An optional `to = {...}` extrude target.
fn extrude_target_opt(
    doc: &crate::model::Document,
    o: &Map<String, Value>,
) -> Result<Option<ExtrudeTarget>, String> {
    match o.get("to") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => Ok(Some(extrude_target_from_json(doc, v)?)),
    }
}

/// An `ExtrudeTarget` from a `to = {...}` object (mirrors `parse_extrude_target_table`):
/// `{plane=i}`, `{face=<face spec | FaceId>}`, or `{vertex=<point>}`.
fn extrude_target_from_json(
    doc: &crate::model::Document,
    v: &Value,
) -> Result<ExtrudeTarget, String> {
    let t = v.as_object().ok_or("extrude `to` must be an object")?;
    if let Some(ordinal) = opt_usize(t, "plane")? {
        let key = doc
            .construction_planes
            .keys()
            .nth(ordinal)
            .ok_or_else(|| format!("no construction plane {ordinal}"))?;
        return Ok(ExtrudeTarget::Plane(key));
    }
    if let Some(face) = t.get("face") {
        if !face.is_null() {
            let fo = face.as_object().ok_or("extrude `to.face` must be an object")?;
            // A `kind`/`type` key marks a 3D body face (FaceId); otherwise it's a flat profile.
            if fo.contains_key("kind") || fo.contains_key("type") {
                return Ok(ExtrudeTarget::BodyFace(face_id_from_json(doc, face)?));
            }
            return Ok(ExtrudeTarget::Face(extrude_face_from_json(doc, face)?));
        }
    }
    if let Some(vertex) = t.get("vertex") {
        if !vertex.is_null() {
            return Ok(ExtrudeTarget::Vertex(constraint_point_from_json(doc, vertex)?));
        }
    }
    Err("extrude target requires one of plane/face/vertex".into())
}

/// An `ExtrudeFace` from a face-spec object: `{circle=i}`, `{polygon=[..]}`, or a nested
/// `{boolean={op,a,b}}` (mirrors `parse_extrude_face_table`).
fn extrude_face_from_json(
    doc: &crate::model::Document,
    v: &Value,
) -> Result<ExtrudeFace, String> {
    let t = v.as_object().ok_or("face spec must be an object")?;
    if let Some(ordinal) = opt_usize(t, "circle")? {
        let key = doc
            .circles
            .keys()
            .nth(ordinal)
            .ok_or_else(|| format!("no circle {ordinal}"))?;
        return Ok(ExtrudeFace::Circle(key));
    }
    if let Some(lines) = opt_usize_array(t, "polygon")? {
        return Ok(ExtrudeFace::Polygon(line_keys_from_ordinals(doc, lines)?));
    }
    if let Some(b) = t.get("boolean") {
        if !b.is_null() {
            return boolean_face_from_json(doc, b);
        }
    }
    Err("face spec requires one of circle/polygon/boolean".into())
}

/// A `{ op, a, b }` boolean region (mirrors `parse_boolean_face_table`).
fn boolean_face_from_json(doc: &crate::model::Document, v: &Value) -> Result<ExtrudeFace, String> {
    let t = v.as_object().ok_or("boolean face must be an object")?;
    let op = match req_str(t, "op", "boolean")?.to_ascii_lowercase().as_str() {
        "intersection" => BooleanOp::Intersection,
        "difference" => BooleanOp::Difference,
        other => {
            return Err(format!(
                "unknown boolean op '{other}' (expected 'intersection' or 'difference')"
            ))
        }
    };
    let a = extrude_face_from_json(doc, t.get("a").ok_or("boolean face requires `a`")?)?;
    let b = extrude_face_from_json(doc, t.get("b").ok_or("boolean face requires `b`")?)?;
    Ok(ExtrudeFace::Boolean { op, a: Box::new(a), b: Box::new(b) })
}

/// A `ConstraintPoint` from a point object (mirrors `parse_constraint_point_table`): a line
/// endpoint (`{kind="line", index, end}`), a circle center (`{kind="circle", index}`), or a
/// body-face vertex (`{kind="face", face={...}, index}`).
fn constraint_point_from_json(
    doc: &crate::model::Document,
    v: &Value,
) -> Result<ConstraintPoint, String> {
    let t = v.as_object().ok_or("point must be an object")?;
    let kind = t
        .get("kind")
        .or_else(|| t.get("type"))
        .and_then(Value::as_str)
        .ok_or("point requires a string `kind`")?;
    if kind.eq_ignore_ascii_case("origin") {
        return Ok(ConstraintPoint::Origin);
    }
    if kind.eq_ignore_ascii_case("face") {
        let face = face_id_from_json(doc, t.get("face").ok_or("face vertex requires `face`")?)?;
        let index = req_usize(t, "index", "point")?;
        return Ok(ConstraintPoint::FaceVertex { face, index });
    }
    let index = req_usize(t, "index", "point")?;
    match kind.to_ascii_lowercase().as_str() {
        "line" => {
            let end = match req_str(t, "end", "point")?.to_ascii_lowercase().as_str() {
                "start" | "0" => LineEnd::Start,
                "end" | "1" => LineEnd::End,
                other => return Err(format!("unknown line endpoint '{other}'")),
            };
            Ok(ConstraintPoint::LineEndpoint {
                line: line_key_from_ordinal(doc, index)?,
                end,
            })
        }
        "circle" => Ok(ConstraintPoint::CircleCenter(
            doc.circles
                .keys()
                .nth(index)
                .ok_or_else(|| format!("no circle {index}"))?,
        )),
        other => Err(format!("unknown point parent '{other}'")),
    }
}

/// An `ExtrusionEdgeRef` from an `edge = {...}` object (mirrors `parse_extrusion_edge_table`):
/// `{kind="vertical", face, edge}` or `{kind="cap", face, edge, top?}`.
fn extrusion_edge_from_json(v: &Value) -> Result<ExtrusionEdgeRef, String> {
    let t = v.as_object().ok_or("edge spec must be an object")?;
    let kind = t
        .get("kind")
        .or_else(|| t.get("type"))
        .and_then(Value::as_str)
        .ok_or("edge spec requires a string `kind`")?;
    let face = req_usize(t, "face", "edge")?;
    let edge = req_usize(t, "edge", "edge")?;
    match kind.to_ascii_lowercase().as_str() {
        "vertical" => Ok(ExtrusionEdgeRef::Vertical { face, edge }),
        "cap" => Ok(ExtrusionEdgeRef::Cap {
            face,
            edge,
            top: opt_bool(t, "top")?.unwrap_or(false),
        }),
        other => Err(format!(
            "unknown extrusion edge kind '{other}' (expected 'vertical' or 'cap')"
        )),
    }
}

/// The edge argument of a `chamfer_edge`/`fillet_edge` object: either a single `edge` beside a
/// top-level `extrusion`, or an `edges` array treated by one operation (#672). An `edges` entry
/// is `{ "extrusion": i, "edge": {...} }`, or the edge object itself when the top-level
/// `extrusion` covers it.
fn extrusion_edge_set_from_json(
    o: &serde_json::Map<String, Value>,
    name: &str,
) -> Result<Vec<(crate::script::TreatableSolidRef, ExtrusionEdgeRef)>, String> {
    let default_host = treatable_solid_ref_from_json(o)?;
    if let Some(list) = o.get("edges") {
        let list = list.as_array().ok_or_else(|| format!("{name} `edges` must be an array"))?;
        if list.is_empty() {
            return Err(format!("{name} `edges` must name at least one edge"));
        }
        return list
            .iter()
            .map(|entry| {
                let obj = entry.as_object().ok_or("edge spec must be an object")?;
                // `edge` is an object in the wrapped form and an index in the bare edge spec,
                // whose own `edge` field numbers the edge — so the shape, not the key, decides.
                let (host, edge_value) = match obj.get("edge").filter(|v| v.is_object()) {
                    Some(inner) => (treatable_solid_ref_from_json(obj)?, inner),
                    None => (None, entry),
                };
                let host = host.or(default_host).ok_or_else(|| {
                    format!("{name} `edges` entry requires an `extrusion` or `primitive`")
                })?;
                Ok((host, extrusion_edge_from_json(edge_value)?))
            })
            .collect();
    }
    let edge = extrusion_edge_from_json(
        o.get("edge").ok_or_else(|| format!("{name} requires an `edge`"))?,
    )?;
    let host = default_host
        .ok_or_else(|| format!("{name} requires an `extrusion` or `primitive`"))?;
    Ok(vec![(host, edge)])
}

fn treatable_solid_ref_from_json(
    o: &serde_json::Map<String, Value>,
) -> Result<Option<crate::script::TreatableSolidRef>, String> {
    let extrusion = opt_usize(o, "extrusion")?;
    let primitive = opt_usize(o, "primitive")?;
    match (extrusion, primitive) {
        (Some(i), None) => Ok(Some(crate::script::TreatableSolidRef::Extrusion(i))),
        (None, Some(i)) => Ok(Some(crate::script::TreatableSolidRef::Primitive(i))),
        (Some(_), Some(_)) => Err("give `extrusion` or `primitive`, not both".into()),
        (None, None) => Ok(None),
    }
}

/// A distance-constraint target from a `{ kind, index }` object (mirrors
/// `parse_distance_target`): a line's length or a circle's diameter.
fn distance_target_from_json(
    doc: &crate::model::Document,
    v: &Value,
) -> Result<DistanceTarget, String> {
    let t = v.as_object().ok_or("constraint target must be an object")?;
    let kind = req_str(t, "kind", "target")?;
    let index = req_usize(t, "index", "target")?;
    match kind.to_ascii_lowercase().as_str() {
        "line" => Ok(DistanceTarget::LineLength(line_key_from_ordinal(doc, index)?)),
        "circle" => Ok(DistanceTarget::CircleDiameter(
            doc.circles
                .keys()
                .nth(index)
                .ok_or_else(|| format!("no circle {index}"))?,
        )),
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
        "horizontal" | "along_x" | "parallel_x" => Some(GeometricConstraintType::AlongXAxis),
        "vertical" | "along_y" | "parallel_y" => Some(GeometricConstraintType::AlongYAxis),
        _ => None,
    }
}

/// Collect the profile faces shared by `revolve`/`loft` (and, in the stateful path,
/// `extrude`): a single `circle`, a `circles` list, a single `polygon` loop, and — only for
/// `loft` (`allow_polygons`) — a `polygons` list of loops. Order matches the closures: single
/// circle, circles list, polygon, polygons.
fn collect_profile_faces(
    doc: &crate::model::Document,
    o: &Map<String, Value>,
    allow_polygons: bool,
) -> Result<Vec<ExtrudeFace>, String> {
    let mut faces = Vec::new();
    let circle_key = |ordinal: usize| {
        doc.circles
            .keys()
            .nth(ordinal)
            .ok_or_else(|| format!("no circle {ordinal}"))
    };
    if let Some(i) = opt_usize(o, "circle")? {
        faces.push(ExtrudeFace::Circle(circle_key(i)?));
    }
    for i in usize_list(o, "circles")? {
        faces.push(ExtrudeFace::Circle(circle_key(i)?));
    }
    if let Some(lines) = opt_usize_array(o, "polygon")? {
        faces.push(ExtrudeFace::Polygon(line_keys_from_ordinals(doc, lines)?));
    }
    if allow_polygons {
        for lines in usize_array_list(o, "polygons")? {
            faces.push(ExtrudeFace::Polygon(line_keys_from_ordinals(doc, lines)?));
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
#[allow(clippy::type_complexity)]
fn move_op_args(
    doc: &crate::model::Document,
    o: &Map<String, Value>,
) -> Result<
    (
        Vec<usize>,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        bool,
        String,
        String,
        Option<crate::model::MovePointRef>,
        Option<crate::model::MovePointRef>,
        Option<crate::model::MovePointRef>,
        Option<crate::model::MovePointRef>,
        Option<crate::model::MovePointRef>,
        Option<crate::model::MovePointRef>,
    ),
    String,
> {
    let targets = usize_list(o, "bodies")?;
    Ok((
        targets,
        expr_arg(o, "x")?,
        expr_arg(o, "y")?,
        expr_arg(o, "z")?,
        // Free-mode turns about the world axes (#1076), in degrees.
        expr_arg(o, "rx")?,
        expr_arg(o, "ry")?,
        expr_arg(o, "rz")?,
        // The third pair as an angle (#1078).
        expr_arg(o, "roll")?,
        // Face Snap's side flip and its turn about the target normal (#1077).
        o.get("flip").and_then(Value::as_bool).unwrap_or(false),
        expr_arg(o, "spin")?,
        expr_arg(o, "gap")?,
        // Naming both points makes the translation a snap (#648/#649/#650).
        move_point_from_json(doc, o.get("from"), "from")?,
        move_point_from_json(doc, o.get("to"), "to")?,
        // The optional B pair (#669) adds the rotation.
        move_point_from_json(doc, o.get("from_b"), "from_b")?,
        move_point_from_json(doc, o.get("to_b"), "to_b")?,
        // The optional C pair pins the spin B leaves free.
        move_point_from_json(doc, o.get("from_c"), "from_c")?,
        move_point_from_json(doc, o.get("to_c"), "to_c")?,
    ))
}

/// `joint`/`edit_joint`/`begin_joint` shared arguments (#894): the members (`a`/`b` or
/// `parts`), kind (+ screw `lead`), base side, the mate that places them (#1020 — a `face`
/// pair plus `line_up` rows), and the position expressions — the JSON twin of
/// `lua_script::parse_joint_op_args`.
#[allow(clippy::type_complexity)]
fn joint_op_args(
    doc: &crate::model::Document,
    o: &Map<String, Value>,
) -> Result<
    (
        Vec<crate::model::JointRef>,
        usize,
        crate::model::JointKind,
        crate::model::MoveOperation,
        crate::model::JointFrame,
        String,
        String,
        String,
        crate::model::JointLimits,
    ),
    String,
> {
    let member = |v: &Value, what: &str| -> Result<crate::model::JointRef, String> {
        if let Some(i) = v.as_u64() {
            return Ok(crate::model::JointRef::Body(body_key_from_ordinal(doc, i as usize)?));
        }
        let t = v
            .as_object()
            .ok_or_else(|| format!("joint `{what}` must be a body index or {{kind, index}}"))?;
        let kind = t
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("joint `{what}` needs a `kind`"))?;
        let index = req_usize(t, "index", what)?;
        match kind {
            "body" => Ok(crate::model::JointRef::Body(body_key_from_ordinal(doc, index)?)),
            "component" => Ok(crate::model::JointRef::Component(
                doc.components
                    .keys()
                    .nth(index)
                    .ok_or_else(|| format!("no component {index}"))?,
            )),
            "unit_instance" | "unit" => Ok(crate::model::JointRef::UnitInstance(
                unit_instance_key_from_ordinal(doc, index)?,
            )),
            other => Err(format!(
                "joint `{what}` kind '{other}' (body|component|unit_instance)"
            )),
        }
    };
    let mut members = Vec::new();
    if let Some(parts) = o.get("parts").and_then(Value::as_array) {
        for v in parts {
            members.push(member(v, "parts")?);
        }
    } else {
        if let Some(a) = o.get("a").filter(|v| !v.is_null()) {
            members.push(member(a, "a")?);
        }
        if let Some(b) = o.get("b").filter(|v| !v.is_null()) {
            members.push(member(b, "b")?);
        }
    }
    let mut kind = match o.get("kind").and_then(Value::as_str) {
        None => crate::model::JointKind::Rigid,
        Some(name) => crate::model::JointKind::from_name(name).ok_or_else(|| {
            format!(
                "unknown joint kind '{name}' (rigid|slider|revolute|cylindrical|planar|ball|pin_slot|screw)"
            )
        })?,
    };
    if let Some(lead) = o.get("lead").filter(|v| !v.is_null()) {
        let lead = match lead {
            Value::String(s) => s.clone(),
            other => other
                .as_f64()
                .map(|n| n.to_string())
                .ok_or("lead takes a number or an expression string")?,
        };
        match &mut kind {
            crate::model::JointKind::Screw { lead: l } => *l = lead,
            _ => return Err("lead only applies to a screw joint".to_string()),
        }
    }
    let base = match o.get("base").and_then(Value::as_str) {
        None | Some("a") => 0,
        Some("b") => 1,
        Some(other) => return Err(format!("unknown base '{other}' (expected 'a' or 'b')")),
    };
    let mate = mate_from_json(doc, o)?;
    // Travel limits (#896): expressions on either end, or a stop picked as geometry.
    let stop = |key: &str| -> Result<Option<ExtrudeTarget>, String> {
        match o.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => Ok(Some(extrude_target_from_json(doc, v)?)),
        }
    };
    let limits = crate::model::JointLimits {
        slide_min: expr_arg(o, "slide_min")?,
        slide_max: expr_arg(o, "slide_max")?,
        slide_min_target: stop("slide_min_to")?,
        slide_max_target: stop("slide_max_to")?,
        turn_min: expr_arg(o, "turn_min")?,
        turn_max: expr_arg(o, "turn_max")?,
    };
    // How the joint works (#1079): its own frame, seeded by the mate when left out.
    let frame = crate::model::JointFrame {
        origin: move_point_from_json(doc, o.get("frame_origin"), "frame_origin")?,
        primary: mate_ref_from_json(doc, o.get("frame_axis"), "frame_axis")?,
        secondary: mate_ref_from_json(doc, o.get("frame_axis2"), "frame_axis2")?,
    };
    Ok((
        members,
        base,
        kind,
        mate,
        frame,
        expr_arg(o, "position")?,
        expr_arg(o, "position2")?,
        expr_arg(o, "position3")?,
        limits,
    ))
}

/// The `face` pair and `line_up` rows of a joint call (#1020), the JSON twin of
/// `lua_script::parse_mate`.
fn mate_from_json(
    doc: &crate::model::Document,
    o: &Map<String, Value>,
) -> Result<crate::model::MoveOperation, String> {
    let mut placement = crate::model::MoveOperation::default();
    let Some(face) = o.get("face").and_then(Value::as_object) else {
        return Ok(placement);
    };
    let point = |r: Option<crate::model::MateRef>| match r {
        Some(crate::model::MateRef::Face { body, centroid, normal }) => Ok(Some(
            crate::model::MovePointRef::OnFace {
                body,
                centroid,
                normal,
                // The face's **middle**, accurately (#1080).
                uv: crate::extrude::face_middle_uv(doc, body, centroid, normal),
            },
        )),
        Some(_) => Err("a joint's `face` picks must be flat faces".to_string()),
        None => Ok(None),
    };
    placement.translate_mode = crate::model::MoveTranslateMode::FaceSnap;
    placement.start_point_a = point(mate_ref_from_json(doc, face.get("moving"), "face.moving")?)?;
    placement.end_point_a = point(mate_ref_from_json(doc, face.get("fixed"), "face.fixed")?)?;
    placement.face_flip = face.get("flip").and_then(Value::as_bool).unwrap_or(false);
    placement.face_offset = expr_arg(face, "offset")?;
    placement.face_spin = expr_arg(face, "spin")?;
    Ok(placement)
}

/// One side of a mate pick (#1020): a body `face` + `normal`, a datum `plane`, a body
/// `edge`, a world `axis`, or a point — the point spellings are the Move tool's, except that
/// an edge **midpoint** is `midpoint` here, since `edge` names the whole edge.
fn mate_ref_from_json(
    doc: &crate::model::Document,
    v: Option<&Value>,
    what: &str,
) -> Result<Option<crate::model::MateRef>, String> {
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let t = v
        .as_object()
        .ok_or_else(|| format!("`{what}` must be an object"))?;
    let point = |v: &Value| -> Result<[i32; 3], String> {
        let a = v
            .as_array()
            .filter(|a| a.len() == 3)
            .ok_or_else(|| format!("`{what}` points must be [x, y, z] in mm"))?;
        let n = |i: usize| -> Result<f32, String> {
            a[i].as_f64()
                .map(|f| f as f32)
                .ok_or_else(|| format!("`{what}` points must be numbers"))
        };
        Ok(crate::hierarchy::quantize_body_point(glam::Vec3::new(
            n(0)?,
            n(1)?,
            n(2)?,
        )))
    };
    if let Some(ordinal) = t.get("plane").and_then(Value::as_u64) {
        let key = doc
            .construction_planes
            .keys()
            .nth(ordinal as usize)
            .ok_or_else(|| format!("no construction plane {ordinal}"))?;
        return Ok(Some(crate::model::MateRef::Plane(key)));
    }
    if let Some(v) = t.get("hole_axis").filter(|v| !v.is_null()) {
        let d = t
            .get("direction")
            .filter(|v| !v.is_null())
            .ok_or_else(|| format!("`{what}.hole_axis` needs a `direction`"))?;
        return Ok(Some(crate::model::MateRef::HoleAxis {
            body: body_key_from_ordinal(doc, req_usize(t, "body", what)?)?,
            origin: point(v)?,
            dir: point(d)?,
        }));
    }
    if let Some(name) = t.get("axis").and_then(Value::as_str) {
        return Ok(Some(crate::model::MateRef::Axis(match name {
            "x" => crate::construction::GlobalAxis::X,
            "y" => crate::construction::GlobalAxis::Y,
            "z" => crate::construction::GlobalAxis::Z,
            other => return Err(format!("unknown axis '{other}' (expected 'x', 'y' or 'z')")),
        })));
    }
    if let Some(v) = t.get("face").filter(|v| !v.is_null()) {
        let n = t
            .get("normal")
            .filter(|v| !v.is_null())
            .ok_or_else(|| format!("`{what}.face` needs a `normal`"))?;
        return Ok(Some(crate::model::MateRef::Face {
            body: body_key_from_ordinal(doc, req_usize(t, "body", what)?)?,
            centroid: point(v)?,
            normal: point(n)?,
        }));
    }
    for (key, whole) in [("edge", true), ("midpoint", false)] {
        let Some(ends) = t.get(key).and_then(Value::as_array).filter(|a| a.len() == 2) else {
            continue;
        };
        let body = body_key_from_ordinal(doc, req_usize(t, "body", what)?)?;
        let (a, b) = (point(&ends[0])?, point(&ends[1])?);
        return Ok(Some(if whole {
            crate::model::MateRef::Edge { body, a, b }
        } else {
            crate::model::MateRef::Point(crate::model::MovePointRef::EdgeMidpoint { body, a, b })
        }));
    }
    move_point_from_json(doc, Some(v), what).map(|p| p.map(crate::model::MateRef::Point))
}

/// A [`crate::model::MovePointRef`] from `{ "body": i, "vertex": [x,y,z] }` or
/// `{ "body": i, "edge": [[x,y,z], [x,y,z]] }` — millimetres, re-quantized (#649/#650).
fn move_point_from_json(
    doc: &crate::model::Document,
    v: Option<&Value>,
    what: &str,
) -> Result<Option<crate::model::MovePointRef>, String> {
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let t = v
        .as_object()
        .ok_or_else(|| format!("move `{what}` must be an object"))?;
    let body = body_key_from_ordinal(doc, req_usize(t, "body", what)?)?;
    let point = |v: &Value| -> Result<[i32; 3], String> {
        let a = v
            .as_array()
            .filter(|a| a.len() == 3)
            .ok_or_else(|| format!("move `{what}` points must be [x, y, z] in mm"))?;
        let n = |i: usize| -> Result<f32, String> {
            a[i].as_f64()
                .map(|f| f as f32)
                .ok_or_else(|| format!("move `{what}` points must be numbers"))
        };
        Ok(crate::hierarchy::quantize_body_point(glam::Vec3::new(
            n(0)?,
            n(1)?,
            n(2)?,
        )))
    };
    if let Some(v) = t.get("vertex").filter(|v| !v.is_null()) {
        return Ok(Some(crate::model::MovePointRef::Vertex { body, p: point(v)? }));
    }
    // A point on a face (#738/#1074): the face's centroid plus its normal — the selection
    // key — and optionally how far across the face to sit, in the face's own axes.
    if let Some(v) = t
        .get("on_face")
        .or_else(|| t.get("face_center"))
        .filter(|v| !v.is_null())
    {
        let n = t
            .get("normal")
            .filter(|v| !v.is_null())
            .ok_or_else(|| format!("move `{what}.on_face` needs a `normal`"))?;
        let uv = match t.get("uv").and_then(Value::as_array) {
            Some(a) if a.len() == 2 => {
                let n = |i: usize| -> Result<i32, String> {
                    a[i].as_f64()
                        .map(|v| (v * 100.0).round() as i32)
                        .ok_or_else(|| format!("move `{what}.uv` needs two numbers"))
                };
                [n(0)?, n(1)?]
            }
            Some(_) => return Err(format!("move `{what}.uv` needs two numbers")),
            None => [0, 0],
        };
        return Ok(Some(crate::model::MovePointRef::OnFace {
            body,
            centroid: point(v)?,
            normal: point(n)?,
            uv,
        }));
    }
    let ends = t
        .get("edge")
        .and_then(Value::as_array)
        .filter(|a| a.len() == 2)
        .ok_or_else(|| format!("move `{what}` needs a `vertex` or a two-point `edge`"))?;
    Ok(Some(crate::model::MovePointRef::EdgeMidpoint {
        body,
        a: point(&ends[0])?,
        b: point(&ends[1])?,
    }))
}

/// `repeat_bodies`/`edit_repeat` shared arguments: target bodies, axis (default X), mode
/// (default "count_gap"), and count/spacing/length expression fields.
#[allow(clippy::type_complexity)]
fn repeat_op_args(
    doc: &crate::model::Document,
    o: &Map<String, Value>,
) -> Result<
    (Vec<usize>, RevolveAxis, bool, bool, RepeatMode, String, String, String, Option<ExtrudeTarget>),
    String,
> {
    let targets = usize_list(o, "bodies")?;
    let axis = match o.get("axis") {
        None | Some(Value::Null) => RevolveAxis::X,
        Some(v) => revolve_axis_from_value(doc, v)?,
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
        // `around = true` turns the copies about the axis instead (#839).
        opt_bool(o, "around")?.unwrap_or(false),
        // `flip = true` runs the pattern the other way along the path (#989).
        opt_bool(o, "flip")?.unwrap_or(false),
        mode,
        expr_arg(o, "count")?,
        expr_arg(o, "spacing")?,
        expr_arg(o, "length")?,
        // `to` picks a face/plane/vertex the fill length is measured to (#645).
        extrude_target_opt(doc, o)?,
    ))
}

/// `slice`/`edit_slice` shared arguments: target bodies, the cutters (face-spec objects
/// or `{ kind = "line", index = i }` laser paths, #1126), and the extend-to-infinity flag
/// (default true).
fn slice_op_args(
    doc: &crate::model::Document,
    o: &Map<String, Value>,
) -> Result<(Vec<usize>, Vec<crate::model::SliceCutter>, bool), String> {
    let targets = usize_list(o, "bodies")?;
    let mut cutters = Vec::new();
    match o.get("cutters") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            for t in list {
                cutters.push(slice_cutter_from_json(doc, t)?);
            }
        }
        Some(_) => {
            return Err("slice `cutters` must be a list of face specs or line cutters".into())
        }
    }
    Ok((targets, cutters, opt_bool(o, "extend")?.unwrap_or(true)))
}

/// One slice cutter from JSON: a line laser path (`kind = "line"`) or a planar face-spec.
fn slice_cutter_from_json(
    doc: &crate::model::Document,
    v: &Value,
) -> Result<crate::model::SliceCutter, String> {
    let t = v.as_object().ok_or("cutter must be an object")?;
    let kind = t
        .get("kind")
        .or_else(|| t.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if kind.eq_ignore_ascii_case("line") {
        let index = req_usize(t, "index", "line cutter")?;
        let line = line_key_from_ordinal(doc, index)?;
        return Ok(crate::model::SliceCutter::Line { line });
    }
    Ok(crate::model::SliceCutter::Face(face_id_from_json(doc, v)?))
}

/// `mirror_bodies`/`edit_mirror` shared arguments (#523): the mirror plane (a face spec) and
/// the target bodies.
fn mirror_op_args(
    doc: &crate::model::Document,
    o: &Map<String, Value>,
) -> Result<(FaceId, Vec<usize>, crate::model::MirrorMode), String> {
    use crate::model::MirrorMode;
    // A bare number is a construction-plane ordinal (`plane = 0`); a table is a face spec (#1354).
    let plane = match o.get("plane") {
        Some(Value::Number(n)) => {
            let ordinal = n
                .as_f64()
                .filter(|n| *n >= 0.0)
                .map(|n| n.round() as usize)
                .ok_or("`plane` must be a non-negative integer")?;
            FaceId::ConstructionPlane(plane_key_from_ordinal(doc, ordinal)?)
        }
        Some(v) if v.is_object() => face_id_from_json(doc, v)?,
        Some(v) if !v.is_null() => {
            return Err(
                "`plane` must be a construction-plane ordinal or a face spec table, \
                 e.g. {kind=\"construction_plane\", index=0}"
                    .into(),
            )
        }
        _ => {
            return Err(
                "mirror `plane` is required (a construction-plane ordinal or a face spec table, \
                 e.g. {kind=\"construction_plane\", index=0})"
                    .into(),
            )
        }
    };
    // `output` mirrors the pane's Output row (#639); omitted means a new body each.
    let mode = match o.get("output").and_then(Value::as_str) {
        None | Some("new") | Some("new_body") => MirrorMode::NewBody,
        Some("join") | Some("add") | Some("combine") => MirrorMode::Join,
        Some("cut") => MirrorMode::Cut,
        Some(other) => return Err(format!("unknown mirror output '{other}' (new|join|cut)")),
    };
    Ok((plane, usize_list(o, "bodies")?, mode))
}

/// A rotation/revolve axis from `"x"`/`"y"`/`"z"` or an object `{ line = i }`.
fn revolve_axis_from_value(
    doc: &crate::model::Document,
    v: &Value,
) -> Result<RevolveAxis, String> {
    match v {
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "x" => Ok(RevolveAxis::X),
            "y" => Ok(RevolveAxis::Y),
            "z" => Ok(RevolveAxis::Z),
            other => Err(format!("unknown axis '{other}' (x|y|z or {{line = i}})")),
        },
        Value::Object(t) => {
            if t.contains_key("line") {
                return Ok(RevolveAxis::Line(line_key_from_ordinal(
                    doc,
                    req_usize(t, "line", "axis")?,
                )?));
            }
            // A body feature edge (#643), by the body plus the edge's world endpoints in mm.
            let body = body_key_from_ordinal(doc, req_usize(t, "body", "axis")?)?;
            let point = |key: &str| -> Result<glam::Vec3, String> {
                let v = t
                    .get(key)
                    .and_then(Value::as_array)
                    .ok_or_else(|| format!("axis `{key}` must be [x, y, z]"))?;
                if v.len() != 3 {
                    return Err(format!("axis `{key}` must be [x, y, z]"));
                }
                let n = |i: usize| -> Result<f32, String> {
                    v[i].as_f64()
                        .map(|f| f as f32)
                        .ok_or_else(|| format!("axis `{key}` must be numbers"))
                };
                Ok(glam::Vec3::new(n(0)?, n(1)?, n(2)?))
            };
            Ok(RevolveAxis::BodyEdge { body, a: point("from")?, b: point("to")? })
        }
        _ => Err(
            "axis must be \"x\"|\"y\"|\"z\", {line = i}, or {body = i, from = [x,y,z], to = [x,y,z]}"
                .into(),
        ),
    }
}

/// A `FaceId` from a face-spec object (slice cutters; also the stateful path's targets).
/// Mirrors `parse_face_id_table`: a body cap/side wall (`extrude_cap`/`extrude_side`, with
/// its extrusion + profile descriptors) or, otherwise, a plain `(kind, index)` via
/// [`FaceId::from_script`] (a construction plane or a circle profile).
fn face_id_from_json(doc: &crate::model::Document, v: &Value) -> Result<FaceId, String> {
    let t = v.as_object().ok_or("face spec must be an object")?;
    let kind = t
        .get("kind")
        .or_else(|| t.get("type"))
        .and_then(Value::as_str)
        .ok_or("face spec requires a string `kind`")?;
    match kind.to_ascii_lowercase().as_str() {
        "extrude_cap" | "extrude_side" => {
            let ordinal = req_usize(t, "extrusion", "face")?;
            // A script names an extrusion by its ordinal among the live ones (#1055).
            let extrusion = doc
                .extrusions
                .keys()
                .nth(ordinal)
                .ok_or_else(|| format!("no extrusion {ordinal}"))?;
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
                "circle" => ExtrudeFace::Circle(
                    doc.circles
                        .keys()
                        .nth(profile_index)
                        .ok_or_else(|| format!("no circle {profile_index}"))?,
                ),
                "polygon" => {
                    let lines = match opt_usize_array(t, "profile_lines")? {
                        Some(l) => l,
                        None => opt_usize_array(t, "lines")?
                            .ok_or("polygon profile requires `profile_lines`")?,
                    };
                    ExtrudeFace::Polygon(line_keys_from_ordinals(doc, lines)?)
                }
                // A boolean-combined profile's cap (#406): same descriptor as `extrude`'s
                // `boolean =`.
                "boolean" => boolean_face_from_json(doc, 
                    t.get("boolean").ok_or("boolean profile requires a `boolean` table")?,
                )?,
                other => {
                    return Err(format!(
                        "unknown extrude profile kind '{other}' (circle|polygon|boolean)"
                    ))
                }
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
            FaceId::from_script(doc, kind, index)
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
                "line" => doc.lines.len(),
                "circle" => doc.circles.len(),
                "sketch" => doc.sketches.len(),
                "constraint" => doc.constraints.len(),
                "construction_plane" | "plane" => {
                    doc.construction_planes.len()
                }
                "extrusion" => doc.extrusions.len(),
                "body" => doc.bodies.len(),
                "drawing" => doc.drawings.len(),
                "parameter" => doc.parameters.len(),
                "sketch_text" | "text" => {
                    doc.sketch_texts.len()
                }
                other => {
                    return Err(format!(
                        "unknown count kind '{other}' (valid kinds: line, circle, sketch, \
                         constraint, construction_plane, extrusion, body, drawing, parameter, \
                         sketch_text)"
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
            let ordinal = req_usize(o, "index", "body_stats")?;
            let Some(index) = doc.bodies.keys().nth(ordinal) else {
                return Ok(Value::Null);
            };
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
            let Some(line) = doc.lines.keys().nth(index).and_then(|k| doc.lines.get(k)) else {
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
            t.insert(
                "sketch".into(),
                json!(doc.sketches.keys().position(|k| k == line.sketch)),
            );
        }
        "circle" => {
            // The script's `index` is the circle's ordinal (#1055).
            let Some(circle) = doc.circles.keys().nth(index).map(|k| &doc.circles[k]) else {
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
            t.insert(
                "sketch".into(),
                json!(doc.sketches.keys().position(|k| k == circle.sketch)),
            );
        }
        "sketch" => {
            // The script's `index` is the sketch's ordinal (#1055).
            let Some(sketch) = doc.sketches.keys().nth(index).map(|k| &doc.sketches[k]) else {
                return Ok(Value::Null);
            };
            t.insert("face".into(), json!(face_kind_name(&sketch.face)));
            if let Some(name) = &sketch.name {
                t.insert("name".into(), json!(name));
            }
        }
        "constraint" => {
            // The script's `index` is the constraint's ordinal (#1055).
            let Some(constraint) = doc.constraints.keys().nth(index).map(|k| &doc.constraints[k])
            else {
                return Ok(Value::Null);
            };
            t.insert("kind".into(), json!(constraint_kind_name(&constraint.kind)));
            t.insert("expression".into(), json!(constraint.expression));
            if let Some(name) = &constraint.name {
                t.insert("name".into(), json!(name));
            }
            t.insert(
                "sketch".into(),
                json!(doc.sketches.keys().position(|k| k == constraint.sketch)),
            );
        }
        "construction_plane" | "plane" => {
            // The script's `index` is the plane's ordinal (#1055).
            let Some(plane) = doc
                .construction_planes
                .keys()
                .nth(index)
                .map(|k| &doc.construction_planes[k])
            else {
                return Ok(Value::Null);
            };
            t.insert("origin".into(), vec3_json(plane.origin));
            t.insert("normal".into(), vec3_json(plane.normal));
            // The drawn rectangle's size in the plane's own u/v axes (#833).
            t.insert(
                "extent".into(),
                json!({
                    "u_min": plane.extent.u_min,
                    "u_max": plane.extent.u_max,
                    "v_min": plane.extent.v_min,
                    "v_max": plane.extent.v_max,
                }),
            );
            if let Some(name) = &plane.name {
                t.insert("name".into(), json!(name));
            }
        }
        "extrusion" => {
            // The script's `index` is the extrusion's ordinal among the live ones (#1055).
            let Some(extrusion) = doc.extrusions.keys().nth(index).map(|k| &doc.extrusions[k])
            else {
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
            // The script's `index` is the body's ordinal among the live ones (#1055).
            let Some(body) = doc.bodies.keys().nth(index).map(|k| &doc.bodies[k]) else {
                return Ok(Value::Null);
            };
            if let Some(name) = &body.name {
                t.insert("name".into(), json!(name));
            }
            t.insert("add".into(), json!(body.source.extrusion_indices()));
            t.insert("cut".into(), json!(body.source.cut_extrusion_indices()));
        }
        "parameter" => {
            // The script's `index` is the parameter's ordinal among the live ones (#1055).
            let Some(param) = doc.parameters.keys().nth(index).map(|k| &doc.parameters[k]) else {
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
        FaceId::RevolveCap { .. } => "revolve_cap",
        FaceId::RevolveSide { .. } => "revolve_side",
        FaceId::UnitFace { .. } => "unit_face",
        FaceId::PrimitiveFace { .. } => "primitive_face",
        FaceId::RepeatedFace { .. } => "repeated_face",
        FaceId::BodyMeshFace { .. } => "body_mesh_face",
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
        ConstraintKind::Angle { .. } => "angle",
        ConstraintKind::Tangent { .. } => "tangent",
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

/// A size that is a plain JSON number or a parameter-expression string (#402):
/// returns `(number, expression)`; the expression, when present, resolves at execution.
fn opt_scalar(
    o: &Map<String, Value>,
    key: &str,
) -> Result<Option<(f32, Option<String>)>, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some((0.0, Some(s.clone())))),
        Some(v) => v
            .as_f64()
            .map(|n| Some((n as f32, None)))
            .ok_or_else(|| format!("`{key}` must be a number or an expression string")),
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

/// A chamfer/fillet amount as a parametric expression (#554): a JSON number is formatted, a JSON
/// string is taken verbatim (so `"distance": "leg"` ties the treatment to a parameter).
fn req_amount_expr(o: &Map<String, Value>, key: &str, cmd: &str) -> Result<String, String> {
    match o.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(Value::Number(n)) => Ok(n.to_string()),
        _ => Err(format!("{cmd} requires a number or string `{key}`")),
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

/// Sources `bearcad.project{ ... }` should project (#1351). Empty means the current
/// scene selection (including un-project).
fn parse_project_elements(
    doc: &Document,
    o: &Map<String, Value>,
) -> Result<Vec<SceneElement>, String> {
    let mut elements = Vec::new();
    if let Some(ents) = o.get("entities") {
        match ents {
            Value::Null => {}
            Value::Array(arr) => {
                for v in arr {
                    elements.push(json_scene_element(doc, v)?);
                }
            }
            other => elements.push(json_scene_element(doc, other)?),
        }
    }
    if let Some(i) = opt_usize(o, "body")? {
        elements.push(SceneElement::Body(body_key_from_ordinal(doc, i)?));
    }
    for i in usize_list(o, "bodies")? {
        elements.push(SceneElement::Body(body_key_from_ordinal(doc, i)?));
    }
    if let Some(i) = opt_usize(o, "plane")? {
        let plane = doc
            .construction_planes
            .keys()
            .nth(i)
            .ok_or_else(|| format!("no construction plane {i}"))?;
        elements.push(SceneElement::ConstructionPlane(plane));
    }
    for i in usize_list(o, "planes")? {
        let plane = doc
            .construction_planes
            .keys()
            .nth(i)
            .ok_or_else(|| format!("no construction plane {i}"))?;
        elements.push(SceneElement::ConstructionPlane(plane));
    }
    if elements.is_empty()
        && (o.contains_key("kind") || o.contains_key("type") || o.contains_key("name"))
    {
        elements.push(json_scene_element(doc, &Value::Object(o.clone()))?);
    }
    Ok(elements)
}

/// A name string or `{ kind, index }` / `{ name }` table — the same values `select` takes.
fn json_scene_element(doc: &Document, v: &Value) -> Result<SceneElement, String> {
    match v {
        Value::String(name) => crate::names::find_element_by_name(doc, name)
            .ok_or_else(|| format!("no element named '{name}'")),
        Value::Object(o) => {
            if let Some(name) = o.get("name").and_then(Value::as_str) {
                return crate::names::find_element_by_name(doc, name)
                    .ok_or_else(|| format!("no element named '{name}'"));
            }
            let kind = o
                .get("kind")
                .or_else(|| o.get("type"))
                .and_then(Value::as_str)
                .ok_or("element requires a `kind` or `name`")?;
            let index = o
                .get("index")
                .and_then(Value::as_u64)
                .ok_or("element requires an `index`")? as usize;
            scene_element_from_kind(doc, kind, index)
                .ok_or_else(|| format!("unknown element kind '{kind}'"))
        }
        _ => Err("expected an element (name string or {kind, index})".into()),
    }
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
    use crate::model::line_key_for_slot as lkey;
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::circle_key_for_slot as rkey;
    use crate::model::sketch_key_for_slot as skey;
    use crate::model::extrusion_key_for_slot as xkey;
    use crate::model::unit_key_for_slot as ukey;
    use crate::model::unit_instance_key_for_slot as uikey;
    use crate::model::body_key_for_slot as bkey;
    use super::*;
    use crate::actions::Tool;
    use serde_json::json;

    #[test]
    fn document_and_tool_actions_map_to_instructions() {
        assert_eq!(instruction_from_json(&Document::default(), "new", &json!({})), Ok(Instruction::New));
        assert_eq!(instruction_from_json(&Document::default(), "clear", &json!({})), Ok(Instruction::Clear));
        assert_eq!(instruction_from_json(&Document::default(), "undo", &json!({})), Ok(Instruction::Undo));
        assert_eq!(instruction_from_json(&Document::default(), "quit", &json!({})), Ok(Instruction::Quit));
        assert_eq!(
            instruction_from_json(&Document::default(), "exit_sketch", &json!({})),
            Ok(Instruction::ExitSketch)
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "tool", &json!({ "name": "circle" })),
            Ok(Instruction::Tool(Tool::Circle))
        );
        assert!(instruction_from_json(&Document::default(), "tool", &json!({ "name": "nope" })).is_err());
    }

    #[test]
    fn rect_matches_the_native_defaults() {
        // Same as `bearcad.rect{ width = 40, height = 20 }`: x/y default to 0.
        assert_eq!(
            instruction_from_json(&Document::default(), "rect", &json!({ "width": 40, "height": 20 })),
            Ok(Instruction::CreateRect { x: 0.0, y: 0.0, width: 40.0, height: 20.0, width_expr: None, height_expr: None })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "rect", &json!({ "x": 5, "y": -3, "width": 40, "height": 20 })),
            Ok(Instruction::CreateRect { x: 5.0, y: -3.0, width: 40.0, height: 20.0, width_expr: None, height_expr: None })
        );
        assert!(instruction_from_json(&Document::default(), "rect", &json!({ "width": 40 })).is_err());
    }

    #[test]
    fn circle_accepts_r_radius_or_diameter() {
        let r = Instruction::CreateCircle { cx: 0.0, cy: 0.0, r: 5.0, diameter_expr: None };
        assert_eq!(instruction_from_json(&Document::default(), "circle", &json!({ "r": 5 })), Ok(r.clone()));
        assert_eq!(instruction_from_json(&Document::default(), "circle", &json!({ "radius": 5 })), Ok(r.clone()));
        assert_eq!(instruction_from_json(&Document::default(), "circle", &json!({ "diameter": 10 })), Ok(r));
        assert!(instruction_from_json(&Document::default(), "circle", &json!({ "x": 1 })).is_err());
    }

    #[test]
    fn line_supports_endpoints_and_length_angle() {
        assert_eq!(
            instruction_from_json(&Document::default(), "line", &json!({ "x1": 30, "y1": 0 })),
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
            instruction_from_json(&Document::default(), "line", &json!({ "length": 10 })).unwrap()
        else {
            panic!("expected a line");
        };
        assert!((x1 - 10.0).abs() < 1e-5 && y1.abs() < 1e-5);
    }

    #[test]
    fn line_dimension_true_locks_the_as_drawn_length() {
        let instr =
            instruction_from_json(&Document::default(), "line", &json!({ "x1": 3, "y1": 4, "dimension": true })).unwrap();
        let Instruction::CreateLine { dimension, .. } = instr else {
            panic!("expected a line");
        };
        assert_eq!(dimension.as_deref(), Some("5"));
    }

    #[test]
    fn line_bezier_reads_both_handles() {
        let instr = instruction_from_json(&Document::default(), 
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
            instruction_from_json(&Document::default(), "plane", &json!({ "offset": 12, "from": 1 })),
            Ok(Instruction::CreatePlane { offset: 12.0, from: 1 })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "plane", &json!({})),
            Ok(Instruction::CreatePlane { offset: 0.0, from: 0 })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "begin_sketch", &json!({ "kind": "plane", "index": 0 })),
            Ok(Instruction::BeginSketch {
                face: FaceId::from_script(&Document::default(), "plane", 0).unwrap(),
            })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "open_sketch", &json!({ "sketch": 2 })),
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
        assert!(instruction_from_json(&Document::default(), "frobnicate", &json!({})).is_err());
        assert!(instruction_from_json(&Document::default(), "rect", &json!("not an object")).is_err());
        assert!(instruction_from_json(&Document::default(), "tool", &json!({})).is_err());
    }

    /// #1351: `bearcad.project` is a pure instruction of its named sources.
    #[test]
    fn project_maps_to_instruction() {
        let doc = Document::default();
        assert_eq!(
            instruction_from_json(&doc, "project", &json!({})),
            Ok(Instruction::Project { elements: vec![] })
        );
        assert_eq!(
            instruction_from_json(&doc, "project", &json!({ "plane": 2 })),
            Ok(Instruction::Project {
                elements: vec![SceneElement::ConstructionPlane(pkey(2))],
            })
        );
        assert_eq!(
            instruction_from_json(
                &doc,
                "project",
                &json!({ "entities": [{ "kind": "construction_plane", "index": 1 }] })
            ),
            Ok(Instruction::Project {
                elements: vec![SceneElement::ConstructionPlane(pkey(1))],
            })
        );
        assert_eq!(
            instruction_from_json(
                &doc,
                "project",
                &json!({ "kind": "construction_plane", "index": 2 })
            ),
            Ok(Instruction::Project {
                elements: vec![SceneElement::ConstructionPlane(pkey(2))],
            })
        );
    }

    #[test]
    fn io_commands_map_to_instructions() {
        assert_eq!(
            instruction_from_json(&Document::default(), "open", &json!({ "path": "part.bcad" })),
            Ok(Instruction::Open("part.bcad".into()))
        );
        assert_eq!(instruction_from_json(&Document::default(), "save", &json!({})), Ok(Instruction::Save(None)));
        assert_eq!(
            instruction_from_json(&Document::default(), "save", &json!({ "path": "out.bcad" })),
            Ok(Instruction::Save(Some("out.bcad".into())))
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "export_stl", &json!({ "path": "a.stl", "body": "Plate" })),
            Ok(Instruction::ExportStl { path: "a.stl".into(), body: Some("Plate".into()) })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "export_3mf", &json!({ "path": "a.3mf" })),
            Ok(Instruction::Export3mf { path: "a.3mf".into(), body: None })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "export_step", &json!({ "path": "a.step" })),
            Ok(Instruction::ExportStep { path: "a.step".into(), body: None })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "import_image", &json!({ "path": "p.png", "plane": 2 })),
            Ok(Instruction::ImportImage { path: "p.png".into(), plane: Some(2) })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), 
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
        // Circles are named by ordinal (#1055), so the document must hold them.
        let doc = doc_with_circles(1);
        // Bare `bearcad.revolve{ polygon = {0,1,2,3}, axis = "y" }`: angle 360, not symmetric,
        // new body, no explicit body list.
        assert_eq!(
            instruction_from_json(&doc, "revolve", &json!({ "polygon": [0, 1, 2, 3], "axis": "y" })),
            Ok(Instruction::Revolve {
                faces: vec![ExtrudeFace::Polygon(vec![lkey(0), lkey(1), lkey(2), lkey(3)])],
                axis: RevolveAxis::Y,
                angle_deg: 360.0,
                pitch_mm: 0.0,
                symmetric: false,
                body: RevolveBodyChoice::NewBody,
                bodies: vec![],
            })
        );
        assert_eq!(
            instruction_from_json(&doc, 
                "revolve",
                &json!({ "circle": 0, "axis": { "line": 3 }, "angle": 90, "symmetric": true,
                         "body": "cut", "bodies": [1, 2] })
            ),
            Ok(Instruction::Revolve {
                faces: vec![ExtrudeFace::Circle(rkey(0))],
                axis: RevolveAxis::Line(lkey(3)),
                angle_deg: 90.0,
                pitch_mm: 0.0,
                symmetric: true,
                body: RevolveBodyChoice::Cut,
                bodies: vec![1, 2],
            })
        );
        // #1242: revolutions and pitch for springs.
        assert_eq!(
            instruction_from_json(
                &doc,
                "revolve",
                &json!({ "polygon": [0, 1, 2, 3], "axis": "y", "revolutions": 2.5, "pitch": 5.0 })
            ),
            Ok(Instruction::Revolve {
                faces: vec![ExtrudeFace::Polygon(vec![lkey(0), lkey(1), lkey(2), lkey(3)])],
                axis: RevolveAxis::Y,
                angle_deg: 900.0,
                pitch_mm: 5.0,
                symmetric: false,
                body: RevolveBodyChoice::NewBody,
                bodies: vec![],
            })
        );
        assert!(instruction_from_json(&doc, "revolve", &json!({ "circle": 0 })).is_err());
        assert!(instruction_from_json(&doc, "revolve", &json!({ "axis": "x" })).is_err());
    }

    #[test]
    fn loft_gathers_circles_and_polygons() {
        // Circles are named by ordinal (#1055), so the document must hold them.
        let doc = doc_with_circles(2);
        assert_eq!(
            instruction_from_json(&doc,
                "loft",
                &json!({ "circles": [0, 1], "polygons": [[2, 3, 4, 5]] })
            ),
            Ok(Instruction::Loft {
                faces: vec![
                    ExtrudeFace::Circle(rkey(0)),
                    ExtrudeFace::Circle(rkey(1)),
                    ExtrudeFace::Polygon(vec![lkey(2), lkey(3), lkey(4), lkey(5)]),
                ],
                body: RevolveBodyChoice::NewBody,
                bodies: vec![],
            })
        );
        // Fewer than two sections is rejected, as in the closure.
        assert!(instruction_from_json(&doc,"loft", &json!({ "circle": 0 })).is_err());
    }

    #[test]
    fn combine_defaults_and_edit() {
        assert_eq!(
            instruction_from_json(&Document::default(), "combine", &json!({ "a": [0], "b": [1] })),
            Ok(Instruction::CreateBooleanOp {
                kind: BooleanOpKind::Combine,
                a: vec![0],
                b: vec![1],
                keep_b: false,
            })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), 
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
        assert!(instruction_from_json(&Document::default(), "combine", &json!({ "op": "nope" })).is_err());
    }

    #[test]
    fn mirror_bodies_parses_plane_and_bodies() {
        assert_eq!(
            instruction_from_json(&Document::default(), 
                "mirror_bodies",
                &json!({ "plane": { "kind": "construction_plane", "index": 0 }, "bodies": [0, 1] })
            ),
            Ok(Instruction::CreateMirrorOp {
                plane: FaceId::ConstructionPlane(pkey(0)),
                targets: vec![0, 1],
                mode: crate::model::MirrorMode::NewBody,
            })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), 
                "edit_mirror",
                &json!({ "index": 2, "plane": { "kind": "construction_plane", "index": 1 }, "bodies": [3] })
            ),
            Ok(Instruction::EditMirrorOp {
                op: 2,
                plane: FaceId::ConstructionPlane(pkey(1)),
                targets: vec![3],
                mode: crate::model::MirrorMode::NewBody,
            })
        );
        // A missing plane is an error.
        assert!(instruction_from_json(&Document::default(), "mirror_bodies", &json!({ "bodies": [0] })).is_err());
        // #1354: a bare construction-plane ordinal, same as the table form above.
        assert_eq!(
            instruction_from_json(&Document::default(),
                "mirror_bodies",
                &json!({ "plane": 0, "bodies": [0] })
            ),
            Ok(Instruction::CreateMirrorOp {
                plane: FaceId::ConstructionPlane(pkey(0)),
                targets: vec![0],
                mode: crate::model::MirrorMode::NewBody,
            })
        );
        let err = instruction_from_json(
            &Document::default(),
            "mirror_bodies",
            &json!({ "plane": "xy", "bodies": [0] }),
        )
        .unwrap_err();
        assert!(
            err.contains("plane") && err.contains("construction_plane"),
            "{err}"
        );
    }

    /// #894: the web `joint` command builds the same instruction the mlua closure does —
    /// members from `a`/`b`, `to` points on the base side's frame, positions stringified.
    #[test]
    fn joint_maps_pairs_onto_frames_like_the_lua_closure() {
        // The members are named by ordinal (#1055), so the document must hold them.
        let mut doc = Document::default();
        for _ in 0..2 {
            doc.bodies.insert(crate::model::Body {
                source: crate::model::BodySource::Extrusion(xkey(0)),
                name: None,
                material: None,
                shadow: false,
            });
        }
        for _ in 0..3 {
            doc.unit_instances.insert(crate::model::UnitInstance {
                unit: ukey(0),
                name: None,
                parameter_overrides: Vec::new(),
                placement: Default::default(),
            });
        }
        assert_eq!(
            instruction_from_json(&doc,
                "joint",
                &json!({
                    "a": 0,
                    "b": { "kind": "unit_instance", "index": 2 },
                    "kind": "revolute",
                    "face": {
                        "moving": { "body": 1, "face": [40, 0, 0], "normal": [0, 0, 1] },
                        "fixed": { "body": 0, "face": [0, 0, 0], "normal": [0, 0, 1] },
                        "flip": true,
                        "offset": 2,
                    },
                    "frame_axis": { "axis": "x" },
                    "position": 90,
                })
            ),
            Ok(Instruction::CreateJointOp {
                members: vec![
                    crate::model::JointRef::Body(bkey(0)),
                    crate::model::JointRef::UnitInstance(uikey(2)),
                ],
                base: 0,
                kind: crate::model::JointKind::Revolute,
                // A joint's mate is a Face Snap move (#1079).
                placement: crate::model::MoveOperation {
                    translate_mode: crate::model::MoveTranslateMode::FaceSnap,
                    start_point_a: Some(crate::model::MovePointRef::OnFace {
                        body: bkey(1),
                        centroid: [4000, 0, 0],
                        normal: [0, 0, 100],
                        uv: [0, 0],
                    }),
                    end_point_a: Some(crate::model::MovePointRef::OnFace {
                        body: bkey(0),
                        centroid: [0, 0, 0],
                        normal: [0, 0, 100],
                        uv: [0, 0],
                    }),
                    face_flip: true,
                    face_offset: "2".into(),
                    ..Default::default()
                },
                frame: crate::model::JointFrame {
                    origin: None,
                    primary: Some(crate::model::MateRef::Axis(
                        crate::construction::GlobalAxis::X,
                    )),
                    secondary: None,
                },
                position: "90".into(),
                position2: String::new(),
                position3: String::new(),
                limits: Default::default(),
            })
        );
        // `base = "b"` names the second member as the held side.
        assert_eq!(
            instruction_from_json(&doc,
                "edit_joint",
                &json!({ "index": 0, "a": 0, "b": 1, "kind": "screw", "lead": "2", "base": "b" })
            ),
            Ok(Instruction::EditJointOp {
                op: 0,
                members: vec![
                    crate::model::JointRef::Body(bkey(0)),
                    crate::model::JointRef::Body(bkey(1)),
                ],
                base: 1,
                kind: crate::model::JointKind::Screw { lead: "2".into() },
                placement: Default::default(),
                position: String::new(),
                position2: String::new(),
                position3: String::new(),
                limits: Default::default(),
                frame: Default::default(),
            })
        );
    }

    /// #1074: a script names a point on a face by the face's selection key plus, optionally,
    /// how far across the face it sits in that face's own axes. `face_center` still spells
    /// the middle of one, which is all there used to be (#738).
    #[test]
    fn move_bodies_reads_a_point_on_a_face() {
        // The body is named by ordinal (#1055), so the document has to hold one.
        let mut doc = Document::default();
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(crate::arena::Key::from_bits(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let on_face = |uv: [i32; 2]| {
            Some(crate::model::MovePointRef::OnFace {
                body: bkey(0),
                centroid: [0, 0, 500],
                normal: [0, 0, 100],
                uv,
            })
        };
        let parse = |from: Value| {
            instruction_from_json(&doc, "move_bodies", &json!({ "bodies": [0], "from": from }))
                .map(|i| match i {
                    Instruction::CreateMoveOp { start_point_a, .. } => start_point_a,
                    other => panic!("expected a move: {other:?}"),
                })
        };
        assert_eq!(
            parse(json!({ "body": 0, "on_face": [0, 0, 5], "normal": [0, 0, 1], "uv": [3, -2] })),
            Ok(on_face([300, -200]))
        );
        // No `uv` is the middle of the face...
        assert_eq!(
            parse(json!({ "body": 0, "on_face": [0, 0, 5], "normal": [0, 0, 1] })),
            Ok(on_face([0, 0]))
        );
        // ...and so is the older `face_center` spelling.
        assert_eq!(
            parse(json!({ "body": 0, "face_center": [0, 0, 5], "normal": [0, 0, 1] })),
            Ok(on_face([0, 0]))
        );
        // The face key is meaningless without a normal, and `uv` is a pair or nothing.
        assert!(parse(json!({ "body": 0, "on_face": [0, 0, 5] })).is_err());
        assert!(parse(
            json!({ "body": 0, "on_face": [0, 0, 5], "normal": [0, 0, 1], "uv": [1, 2, 3] })
        )
        .is_err());
    }

    #[test]
    fn move_bodies_stringifies_expression_fields() {
        assert_eq!(
            instruction_from_json(&Document::default(), 
                "move_bodies",
                &json!({ "bodies": [0], "x": 10, "y": "w/2" })
            ),
            Ok(Instruction::CreateMoveOp {
                start_point_a: None,
                end_point_a: None,
                start_point_b: None,
                end_point_b: None,
                start_point_c: None,
                end_point_c: None,
                targets: vec![0],
                tx: "10".into(),
                ty: "w/2".into(),
                tz: String::new(),
                rx: String::new(),
                ry: String::new(),
                rz: String::new(),
                face_flip: false,
                face_spin: String::new(),
                roll_angle: String::new(),
                face_offset: String::new(),
            })
        );
        // Omitted expression fields become empty strings.
        assert_eq!(
            instruction_from_json(&Document::default(), "edit_move", &json!({ "index": 1, "bodies": [0], "z": 5 })),
            Ok(Instruction::EditMoveOp {
                start_point_a: None,
                end_point_a: None,
                start_point_b: None,
                end_point_b: None,
                start_point_c: None,
                end_point_c: None,
                op: 1,
                targets: vec![0],
                tx: String::new(),
                ty: String::new(),
                tz: "5".into(),
                rx: String::new(),
                ry: String::new(),
                rz: String::new(),
                face_flip: false,
                face_spin: String::new(),
                roll_angle: String::new(),
                face_offset: String::new(),
            })
        );
    }

    #[test]
    fn repeat_bodies_defaults_axis_and_mode() {
        assert_eq!(
            instruction_from_json(&Document::default(), "repeat_bodies", &json!({ "bodies": [0], "count": 5, "spacing": 20 })),
            Ok(Instruction::CreateRepeatOp {
                targets: vec![0],
                axis: RevolveAxis::X,
                around_axis: false,
                flip: false,
                mode: RepeatMode::CountGap,
                count: "5".into(),
                spacing: "20".into(),
                length: String::new(),
                length_target: None,
            })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), 
                "repeat_bodies",
                &json!({ "bodies": [0], "axis": "y", "mode": "fill_pitch", "length": 100, "spacing": 12 })
            ),
            Ok(Instruction::CreateRepeatOp {
                targets: vec![0],
                axis: RevolveAxis::Y,
                around_axis: false,
                flip: false,
                mode: RepeatMode::FillPitch,
                count: String::new(),
                spacing: "12".into(),
                length: "100".into(),
                length_target: None,
            })
        );
        assert!(instruction_from_json(&Document::default(), "repeat_bodies", &json!({ "mode": "nope" })).is_err());
    }

    #[test]
    fn dimension_verbs_route_by_axis() {
        assert_eq!(
            instruction_from_json(&Document::default(), "set_dim", &json!({ "axis": "width", "value": "40" })),
            Ok(Instruction::SetDim { axis: RectAxis::Width, value: "40".into() })
        );
        // A bare number for the value is stringified.
        assert_eq!(
            instruction_from_json(&Document::default(), "set_dim", &json!({ "axis": "length", "value": 25 })),
            Ok(Instruction::SetLineLength { value: "25".into() })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "set_dim", &json!({ "axis": "diameter", "value": "d" })),
            Ok(Instruction::SetCircleDiameter { value: "d".into() })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "set_dim", &json!({ "axis": "offset", "value": "5" })),
            Ok(Instruction::SetPlaneOffset { value: "5".into() })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "focus_dim", &json!({ "axis": "h" })),
            Ok(Instruction::FocusDim(RectAxis::Height))
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "focus_dim", &json!({ "axis": "angle" })),
            Ok(Instruction::FocusPlaneDim(PlaneDim::Angle))
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "edit_dim", &json!({ "axis": "length" })),
            Ok(Instruction::BeginEditCommittedDim { axis: DimLabelAxis::Length })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "edit_dim", &json!({ "axis": "diameter" })),
            Ok(Instruction::BeginEditCommittedDim { axis: DimLabelAxis::Diameter })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "commit_dim", &json!({})),
            Ok(Instruction::CommitCommittedDim)
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "set_dim_label_offset", &json!({ "axis": "w", "offset": 3 })),
            Ok(Instruction::SetDimLabelOffset { axis: DimLabelAxis::Width, offset: 3.0 })
        );
        assert!(instruction_from_json(&Document::default(), "set_dim", &json!({ "axis": "nope", "value": "1" })).is_err());
    }

    #[test]
    fn constraint_verbs_map_to_instructions() {
        // A circle-diameter target is named by ordinal (#1055).
        let doc = doc_with_circles(3);
        assert_eq!(
            instruction_from_json(&doc,
                "add_constraint",
                &json!({ "target": { "kind": "line", "index": 0 }, "expression": "40" })
            ),
            Ok(Instruction::AddDistanceConstraint {
                target: DistanceTarget::LineLength(lkey(0)),
                expression: "40".into(),
            })
        );
        assert_eq!(
            instruction_from_json(&doc,
                "add_constraint",
                &json!({ "target": { "kind": "circle", "index": 2 }, "expression": 12 })
            ),
            Ok(Instruction::AddDistanceConstraint {
                target: DistanceTarget::CircleDiameter(rkey(2)),
                expression: "12".into(),
            })
        );
        // Angle: `value` string form, and `angle`-number form; default sign +1.
        assert_eq!(
            instruction_from_json(&doc,
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
            instruction_from_json(&doc,
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
            instruction_from_json(&doc,"add_geometric_constraint", &json!({ "name": "parallel" })),
            Ok(Instruction::AddGeometricConstraint(GeometricConstraintType::Parallel))
        );
        assert_eq!(
            instruction_from_json(&doc,"constraint_shortcut", &json!({ "key": "p" })),
            Ok(Instruction::ApplyConstraintShortcut('p'))
        );
        assert!(
            instruction_from_json(&doc,"add_geometric_constraint", &json!({ "name": "nope" })).is_err()
        );
        assert!(instruction_from_json(&doc,"add_angle_constraint", &json!({ "a": 0, "b": 5 })).is_err());
    }

    #[test]
    fn plane_edit_naming_and_deletion_verbs() {
        assert_eq!(
            instruction_from_json(&Document::default(), "edit_plane", &json!({ "index": 1 })),
            Ok(Instruction::BeginEditConstructionPlane { index: 1 })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "commit_plane", &json!({})),
            Ok(Instruction::CommitConstructionPlane)
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "focus_name", &json!({})),
            Ok(Instruction::FocusElementName)
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "apply_construction", &json!({ "construction": true })),
            Ok(Instruction::ApplyConstruction { construction: true })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "apply_construction", &json!({ "construction": "off" })),
            Ok(Instruction::ApplyConstruction { construction: false })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "toggle_construction", &json!({})),
            Ok(Instruction::ToggleConstruction)
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "clear_selection", &json!({})),
            Ok(Instruction::ClearSceneSelection)
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "delete_selection", &json!({})),
            Ok(Instruction::DeleteSelection)
        );
    }

    #[test]
    fn positional_args_map_to_named_and_reach_instructions() {
        // `bearcad.tool("circle")` → { name = "circle" } → the tool instruction.
        assert_eq!(
            positional_to_named("tool", &[json!("circle")]),
            Ok(json!({ "name": "circle" }))
        );
        // Trailing optional args may be omitted; `save()` → {}.
        assert_eq!(positional_to_named("save", &[]), Ok(json!({})));
        assert_eq!(
            positional_to_named("export_stl", &[json!("a.stl")]),
            Ok(json!({ "path": "a.stl" }))
        );
        assert_eq!(
            positional_to_named("export_stl", &[json!("a.stl"), json!("Body")]),
            Ok(json!({ "path": "a.stl", "body": "Body" }))
        );
        assert_eq!(
            positional_to_named("export_3mf", &[json!("a.3mf"), json!("Body")]),
            Ok(json!({ "path": "a.3mf", "body": "Body" }))
        );
        assert_eq!(
            positional_to_named("orbit", &[json!(10), json!(-5)]),
            Ok(json!({ "dx": 10, "dy": -5 }))
        );
        assert_eq!(
            positional_to_named("view", &[json!("edge"), json!("fr")]),
            Ok(json!({ "view": "edge", "id": "fr" }))
        );
        // The mapped object drives the same instruction as the table form.
        let mapped = positional_to_named("set_dim", &[json!("width"), json!("40")]).unwrap();
        assert_eq!(
            instruction_from_json(&Document::default(), "set_dim", &mapped),
            Ok(Instruction::SetDim { axis: RectAxis::Width, value: "40".into() })
        );
        // Element verbs carry the element object through positionally.
        assert_eq!(
            positional_to_named("set_name", &[json!({ "kind": "body", "index": 0 }), json!("Lid")]),
            Ok(json!({ "element": { "kind": "body", "index": 0 }, "name": "Lid" }))
        );
        // A table-only verb has no positional form.
        assert!(positional_to_named("extrude", &[json!(1)]).is_err());
    }

    #[test]
    fn scene_element_kind_round_trips() {
        // A body is named by its ordinal among the live ones (#1055), so the document has to
        // actually hold that many.
        let mut doc = Document::default();
        for _ in 0..5 {
            doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
            doc.bodies.insert(crate::model::Body {
                source: crate::model::BodySource::Extrusion(xkey(0)),
                name: None,
                material: None,
                shadow: false,
            });
            doc.extrusions.insert(crate::model::Extrusion {
                sketch: skey(0),
                faces: Vec::new(),
                distance: 1.0,
                target: None,
                expression: String::new(),
                symmetric: false,
                name: None,
                taper: 0.0,
                taper_mode: crate::model::ExtrudeTaperMode::Distance,
                taper_expression: String::new(),
                edge_treatments: Vec::new(),
            });
        }
        for i in 0..8 {
            doc.lines.insert(crate::model::Line::from_local_endpoints(
                doc.sketches.keys().next().unwrap(),
                0.0,
                i as f32,
                10.0,
                i as f32,
            ));
        }
        for i in 0..4 {
            doc.circles.insert(crate::model::Circle::from_local_center_radius(
                doc.sketches.keys().next().unwrap(),
                0.0,
                0.0,
                i as f32 + 1.0,
                0.0,
            ));
            doc.constraints.insert(crate::model::Constraint {
                sketch: skey(0),
                kind: crate::model::ConstraintKind::Coincident {
                    a: crate::model::ConstraintEntity::Line(
                        crate::model::ConstraintLine::Line(lkey(0)),
                    ),
                    b: crate::model::ConstraintEntity::Line(
                        crate::model::ConstraintLine::Line(lkey(1)),
                    ),
                },
                expression: String::new(),
                dim_offset: None,
                name: None,
            });
        }
        for (kind, idx) in [("plane", 2), ("sketch", 0), ("line", 5), ("circle", 1),
            ("constraint", 3), ("extrusion", 0), ("body", 4)]
        {
            let el = scene_element_from_kind(&doc, kind, idx).unwrap();
            assert_eq!(scene_element_kind_name(&doc, &el), Some((kind, idx)));
            assert_eq!(scene_element_selection_index(&doc, &el), Some(idx));
        }
        // Full kind name covers non-round-tripping variants too.
        assert_eq!(
            scene_element_full_kind_name(&SceneElement::Body(bkey(0))),
            "body"
        );
        assert_eq!(scene_element_full_kind_name(&SceneElement::Origin), "origin");
        assert_eq!(
            scene_element_selection_index(&Document::default(), &SceneElement::Origin),
            Some(0)
        );
        assert!(scene_element_from_kind(&Document::default(), "nope", 0).is_none());
        // The `construction_plane` alias resolves to the `plane` element.
        assert_eq!(
            scene_element_from_kind(&Document::default(), "construction_plane", 1),
            scene_element_from_kind(&Document::default(), "plane", 1)
        );
    }

    #[test]
    fn navigation_and_view_verbs() {
        assert_eq!(
            instruction_from_json(&Document::default(), "orbit", &json!({ "dx": 10, "dy": -5 })),
            Ok(Instruction::Orbit { dx: 10.0, dy: -5.0 })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "wheel", &json!({ "scroll": 2 })),
            Ok(Instruction::Zoom { scroll: 2.0 })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "view", &json!({ "view": "top" })),
            Ok(Instruction::View(StandardView::from_name("top").unwrap()))
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "view", &json!({ "view": "orthographic" })),
            Ok(Instruction::ProjectionMode(ProjectionMode::from_name("orthographic").unwrap()))
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "view_home", &json!({})),
            Ok(Instruction::ViewHome)
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "toggle_projection", &json!({})),
            Ok(Instruction::ToggleProjectionMode)
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "shading", &json!({ "mode": "wireframe" })),
            Ok(Instruction::ShadingMode(ShadingMode::from_name("wireframe").unwrap()))
        );
        assert!(instruction_from_json(&Document::default(), "view", &json!({ "view": "nope" })).is_err());
        assert!(instruction_from_json(&Document::default(), "shading", &json!({ "mode": "nope" })).is_err());
    }

    #[test]
    fn camera_pane_palette_and_fps() {
        assert_eq!(
            instruction_from_json(&Document::default(), "camera", &json!({ "yaw": 30, "target": [0, 0, 5] })),
            Ok(Instruction::SetCamera {
                yaw: Some(30.0),
                pitch: None,
                distance: None,
                target: Some((0.0, 0.0, 5.0)),
            })
        );
        // No pose keys is a read, not an action.
        assert!(instruction_from_json(&Document::default(), "camera", &json!({})).is_err());
        assert_eq!(
            instruction_from_json(&Document::default(), "zoom_fit", &json!({})),
            Ok(Instruction::ZoomFit)
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "pane", &json!({ "pane": "elements", "visible": "hide" })),
            Ok(Instruction::SetPane {
                pane: Pane::from_name("elements").unwrap(),
                visible: Some(false),
            })
        );
        // Absent `visible` means toggle.
        assert_eq!(
            instruction_from_json(&Document::default(), "pane", &json!({ "pane": "elements" })),
            Ok(Instruction::SetPane {
                pane: Pane::from_name("elements").unwrap(),
                visible: None,
            })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "palette", &json!({})),
            Ok(Instruction::SetCommandPalette { open: None })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "palette", &json!({ "action": "run", "query": "extrude" })),
            Ok(Instruction::RunPaletteCommand { query: "extrude".into(), argument: None })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "palette", &json!({ "action": "show" })),
            Ok(Instruction::SetCommandPalette { open: Some(true) })
        );
        // fps family.
        assert_eq!(
            instruction_from_json(&Document::default(), "fps", &json!({ "on": true })),
            Ok(Instruction::FpsMode { on: Some(true) })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "fps", &json!({})),
            Ok(Instruction::FpsMode { on: None })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "fps_move", &json!({ "forward": 100 })),
            Ok(Instruction::FpsMove { forward: 100.0, strafe: 0.0 })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "fps_advance", &json!({ "seconds": 0.5 })),
            Ok(Instruction::FpsAdvance { seconds: 0.5 })
        );
    }

    #[test]
    fn chamfer_and_fillet_verbs() {
        // A circle-centre point is named by ordinal (#1055).
        let doc = doc_with_circles(2);
        assert_eq!(
            instruction_from_json(&doc,
                "chamfer_vertex",
                &json!({ "point": { "kind": "line", "index": 0, "end": "start" }, "distance": 2 })
            ),
            Ok(Instruction::VertexTreatment {
                point: ConstraintPoint::LineEndpoint { line: lkey(0), end: LineEnd::Start },
                kind: VertexTreatmentKind::Chamfer,
                amount: "2".to_string(),
            })
        );
        assert_eq!(
            instruction_from_json(&doc,
                "fillet_vertex",
                &json!({ "point": { "kind": "circle", "index": 1 }, "radius": 3 })
            ),
            Ok(Instruction::VertexTreatment {
                point: ConstraintPoint::CircleCenter(rkey(1)),
                kind: VertexTreatmentKind::Fillet,
                amount: "3".to_string(),
            })
        );
        assert_eq!(
            instruction_from_json(&doc,
                "fillet_edge",
                &json!({ "extrusion": 0, "edge": { "kind": "vertical", "face": 0, "edge": 2 }, "radius": 1.5 })
            ),
            Ok(Instruction::EdgeTreatment {
                edges: vec![(crate::script::TreatableSolidRef::Extrusion(0), ExtrusionEdgeRef::Vertical { face: 0, edge: 2 })],
                kind: VertexTreatmentKind::Fillet,
                amount: 1.5,
            })
        );
        assert_eq!(
            instruction_from_json(&doc,
                "chamfer_edge",
                &json!({ "extrusion": 1, "edge": { "kind": "cap", "face": 0, "edge": 3, "top": true }, "distance": 2 })
            ),
            Ok(Instruction::EdgeTreatment {
                edges: vec![(crate::script::TreatableSolidRef::Extrusion(1), ExtrusionEdgeRef::Cap { face: 0, edge: 3, top: true })],
                kind: VertexTreatmentKind::Chamfer,
                amount: 2.0,
            })
        );
        // The plural form (#672): one call, one operation over the whole set.
        assert_eq!(
            instruction_from_json(&doc,
                "fillet_edge",
                &json!({ "extrusion": 0, "edges": [
                    { "kind": "vertical", "face": 0, "edge": 0 },
                    { "extrusion": 1, "edge": { "kind": "vertical", "face": 0, "edge": 2 } }
                ], "radius": 8 })
            ),
            Ok(Instruction::EdgeTreatment {
                edges: vec![
                    (crate::script::TreatableSolidRef::Extrusion(0), ExtrusionEdgeRef::Vertical { face: 0, edge: 0 }),
                    (crate::script::TreatableSolidRef::Extrusion(1), ExtrusionEdgeRef::Vertical { face: 0, edge: 2 }),
                ],
                kind: VertexTreatmentKind::Fillet,
                amount: 8.0,
            })
        );
        assert!(instruction_from_json(&doc,"fillet_edge", &json!({ "edges": [], "radius": 1 })).is_err());
        assert!(instruction_from_json(&doc,"chamfer_vertex", &json!({ "distance": 2 })).is_err());
    }

    #[test]
    fn drawing_verbs_map_to_instructions() {
        assert_eq!(
            instruction_from_json(&Document::default(), "drawing", &json!({ "name": "Plate" })),
            Ok(Instruction::CreateDrawing { name: Some("Plate".into()) })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), "drawing", &json!({})),
            Ok(Instruction::CreateDrawing { name: None })
        );
        // orientation defaults to Front; "iso" is accepted.
        assert_eq!(
            instruction_from_json(&Document::default(), "drawing_view", &json!({ "drawing": 0, "body": 1 })),
            Ok(Instruction::AddDrawingView {
                drawing: 0,
                bodies: vec![1],
                orientation: DrawingOrientation::Front,
            })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), 
                "drawing_view",
                &json!({ "drawing": 0, "body": 0, "orientation": "iso" })
            ),
            Ok(Instruction::AddDrawingView {
                drawing: 0,
                bodies: vec![0],
                orientation: DrawingOrientation::Isometric,
            })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), 
                "export_drawing_svg",
                &json!({ "drawing": 2, "path": "plate.svg" })
            ),
            Ok(Instruction::ExportDrawingSvg { drawing: 2, path: "plate.svg".into() })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), 
                "export_drawing_pdf",
                &json!({ "drawing": 2, "path": "plate.pdf" })
            ),
            Ok(Instruction::ExportDrawingPdf { drawing: 2, path: "plate.pdf".into() })
        );
        assert_eq!(
            instruction_from_json(&Document::default(), 
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
            instruction_from_json(&Document::default(), 
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
            instruction_from_json(&Document::default(), "drawing_view", &json!({ "drawing": 0, "body": 0, "orientation": "nope" }))
                .is_err()
        );
        assert!(
            instruction_from_json(&Document::default(), "drawing_dimension", &json!({ "drawing": 0, "view": 0, "a": [0, 0], "b": [1, 1, 1] }))
                .is_err()
        );
    }

    /// A document with two sketches, and lines/circles named by their **ordinal** in the
    /// `sketch` field — a key serializes as a `[slot, generation]` pair (#1055), which is not
    /// something a test fixture should have to spell.
    fn doc_with(lines: Value, circles: Value) -> Document {
        let mut doc = Document::default();
        for _ in 0..2 {
            doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        }
        let keys: Vec<_> = doc.sketches.keys().collect();
        let resolve = |mut v: Value| -> Value {
            if let Some(list) = v.as_array_mut() {
                for entry in list {
                    if let Some(ordinal) = entry.get("sketch").and_then(Value::as_u64) {
                        entry["sketch"] = json!(keys[ordinal as usize]);
                    }
                }
            }
            v
        };
        for line in serde_json::from_value::<Vec<crate::model::Line>>(resolve(lines)).unwrap() {
            doc.lines.insert(line);
        }
        for circle in serde_json::from_value::<Vec<crate::model::Circle>>(resolve(circles)).unwrap()
        {
            doc.circles.insert(circle);
        }
        doc
    }

    /// A document holding `n` circles plus a handful of lines, for the verbs that name either
    /// by ordinal (#1055).
    fn doc_with_circles(n: usize) -> Document {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        for i in 0..n {
            doc.circles.insert(crate::model::Circle::from_local_center_radius(
                sketch,
                0.0,
                0.0,
                i as f32 + 1.0,
                0.0,
            ));
        }
        for i in 0..8 {
            doc.lines.insert(crate::model::Line::from_local_endpoints(
                sketch,
                0.0,
                i as f32,
                10.0,
                i as f32,
            ));
        }
        doc
    }

    #[test]
    fn count_counts_the_live_entities() {
        let doc = doc_with(
            json!([
                { "sketch": 0, "x0": 0, "y0": 0, "x1": 30, "y1": 0 },
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
    fn extrude_infers_sketch_and_reads_targets() {
        let doc = doc_with(json!([]), json!([{ "sketch": 0, "cx": 0, "cy": 0, "r": 5 }]));
        assert_eq!(
            extrude_instruction("extrude", &json!({ "circle": 0, "distance": 10 }), &doc),
            Ok(Instruction::Extrude {
                expression: None,
                sketch: 0,
                faces: vec![ExtrudeFace::Circle(rkey(0))],
                distance: 10.0,
                body: ExtrudeBodyChoice::New,
                target: None,
                symmetric: false,
            
                taper: 0.0,
                taper_mode: crate::model::ExtrudeTaperMode::Distance,
                taper_expression: None,

            })
        );
        // A `to` target lets distance default to 0.
        let instr =
            extrude_instruction("extrude", &json!({ "circle": 0, "to": { "plane": 1 } }), &doc)
                .unwrap();
        assert!(matches!(
            instr,
            Instruction::Extrude { distance, target: Some(ExtrudeTarget::Plane(_)), .. }
                if distance == 0.0
        ));
        // extrude_face pushes/pulls a body face (here a construction plane) with a cut.
        assert_eq!(
            extrude_instruction(
                "extrude_face",
                &json!({ "face": { "kind": "plane", "index": 0 }, "distance": 5, "body": "cut" }),
                &doc
            ),
            Ok(Instruction::ExtrudeBodyFace {
                face: FaceId::ConstructionPlane(pkey(0)),
                distance: 5.0,
                body: ExtrudeBodyChoice::Cut,
                target: None,
            })
        );
        assert!(extrude_instruction("extrude", &json!({ "distance": 10 }), &doc).is_err());
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
        // The cap cutter below names an extrusion and its profile lines by ordinal (#1055).
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        for i in 0..4 {
            doc.lines.insert(crate::model::Line::from_local_endpoints(
                sketch,
                0.0,
                i as f32,
                10.0,
                i as f32,
            ));
        }
        doc.extrusions.insert(crate::model::Extrusion {
            sketch: skey(0),
            faces: Vec::new(),
            distance: 1.0,
            target: None,
            expression: String::new(),
            symmetric: false,
            name: None,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });
        assert_eq!(
            instruction_from_json(&doc,
                "slice",
                &json!({ "bodies": [0], "cutters": [{ "kind": "plane", "index": 1 }] })
            ),
            Ok(Instruction::CreateSliceOp {
                targets: vec![0],
                cutters: vec![crate::model::SliceCutter::Face(FaceId::ConstructionPlane(
                    pkey(1)
                ))],
                extend_infinite: true,
            })
        );
        // A body cap cutter, and the extend flag turned off.
        assert_eq!(
            instruction_from_json(&doc,
                "edit_slice",
                &json!({ "index": 0, "bodies": [1], "extend": false,
                         "cutters": [{ "kind": "extrude_cap", "extrusion": 0, "profile": "polygon",
                                       "profile_lines": [0, 1, 2, 3], "top": false }] })
            ),
            Ok(Instruction::EditSliceOp {
                op: 0,
                targets: vec![1],
                cutters: vec![crate::model::SliceCutter::Face(FaceId::ExtrudeCap {
                    extrusion: xkey(0),
                    profile: ExtrudeFace::Polygon(vec![lkey(0), lkey(1), lkey(2), lkey(3)]),
                    top: false,
                })],
                extend_infinite: false,
            })
        );
        // #1126: a sketch line as a laser-style path cutter.
        assert_eq!(
            instruction_from_json(
                &doc,
                "slice",
                &json!({ "bodies": [0], "cutters": [{ "kind": "line", "index": 2 }] })
            ),
            Ok(Instruction::CreateSliceOp {
                targets: vec![0],
                cutters: vec![crate::model::SliceCutter::Line { line: lkey(2) }],
                extend_infinite: true,
            })
        );
    }
}
