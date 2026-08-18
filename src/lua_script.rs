//! Lua scripting API (`bearcad` global) for driving the live application.

use crate::actions::{DimLabelAxis, Pane, RectAxis, Tool};
use crate::camera::{GroundDisplay, ProjectionMode, ShadingMode, StandardView};
use crate::construction::PlaneDim;
use crate::geometric_constraints::GeometricConstraintType;
use crate::hierarchy::SceneElement;
use crate::model::{
    ConstraintKind, ConstraintLine, ConstraintPoint, DistanceTarget, ExtrusionEdgeRef, FaceId,
    LineEnd, VertexTreatmentKind,
};
use crate::names::find_element_by_name;
use crate::script::{
    parse_key, Instruction, ScreenshotRegion, ScriptRunner, SyntheticInput, TreatableSolidRef,
};
use crate::value::{AngleUnit, LengthUnit};
use crate::view_cube::{CubeCornerId, CubeEdgeId};

use crate::actions::AppState;
use eframe::egui;
use mlua::{Lua, MultiValue, Table, UserData, UserDataMethods, Value};
use std::path::Path;

/// Per-tick context passed to Lua callbacks via `Lua::set_app_data`.
pub struct ScriptTickData {
    pub runner: *mut ScriptRunner,
    pub state: *mut AppState,
    pub synthetic: *mut SyntheticInput,
    pub viewport: Option<egui::Rect>,
    pub ctx: *mut egui::Context,
}

unsafe impl Send for ScriptTickData {}
unsafe impl Sync for ScriptTickData {}

impl ScriptTickData {
    pub(crate) unsafe fn runner(&self) -> &mut ScriptRunner {
        &mut *self.runner
    }

    pub(crate) unsafe fn state(&self) -> &mut AppState {
        &mut *self.state
    }

    pub(crate) unsafe fn synthetic(&self) -> &mut SyntheticInput {
        &mut *self.synthetic
    }

    pub(crate) unsafe fn egui_ctx(&self) -> &egui::Context {
        &*self.ctx
    }

    pub(crate) unsafe fn exec(&self, instr: Instruction) -> mlua::Result<()> {
        let runner = self.runner();
        runner.last_action_error = None;
        let _ = runner.execute_instruction(
            instr,
            self.state(),
            self.synthetic(),
            self.viewport,
            self.egui_ctx(),
        );
        // Declarative modeling instructions record their action's rejection in
        // `last_action_error` (#104/#109/#110/#112): raise it so invalid input fails the
        // script (catchable with `pcall`) instead of silently succeeding with nothing
        // created. The GUI sees the same message through the status bar.
        match runner.last_action_error.take() {
            Some(e) => Err(mlua::Error::external(e)),
            None => Ok(()),
        }
    }
}

/// A reference to a scene element used by Lua scripts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LuaElement {
    pub element: SceneElement,
    /// The integer `index()` reports, resolved when the reference is made — a userdata
    /// method has no document to resolve an arena key's ordinal against (#1055).
    pub index: usize,
}

impl UserData for LuaElement {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("kind", |_, this, ()| Ok(element_kind_name(this.element.clone())));
        methods.add_method("index", |_, this, ()| Ok(this.index));
    }
}

/// A name for a **face** element, which nothing else distinguishes (#987/#988): every face
/// reports kind `face` and index `0`, so a hover flickering between a body's near face and the
/// one hidden behind it — or a fan offering both — read as identical from a script. That is how
/// both bugs went unnoticed. `None` for anything that isn't a face.
fn face_element_label(doc: &crate::model::Document, element: &SceneElement) -> Option<String> {
    match element {
        SceneElement::SketchFace(face) => Some(crate::face::face_label(doc, face.clone())),
        SceneElement::BodyFace { body, centroid, .. } => Some(format!(
            "Body {} face at ({:.3}, {:.3}, {:.3})",
            body.index(),
            centroid[0] as f32 / 1000.0,
            centroid[1] as f32 / 1000.0,
            centroid[2] as f32 / 1000.0
        )),
        _ => None,
    }
}

fn element_kind_name(element: SceneElement) -> &'static str {
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
        SceneElement::BodyFace { .. } | SceneElement::SketchFace(_) => "face",
        SceneElement::BodyCylinder { .. } => "cylinder",
        SceneElement::BodyAxis { .. } => "body_axis",
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
        SceneElement::Component(_) => "component",
        SceneElement::UnitInstance(_) => "unit_instance",
        SceneElement::Joint(_) => "joint",
        // A drawing's three item types keep their own script names (#363/#967).
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

/// The integer a script sees for an element (#1055). For a `Vec`-backed collection that is
/// still the stored index; for an arena-backed one it is the element's **ordinal** among the
/// live elements of its kind, in document order — a key is not something a hand-written
/// script can spell, and every example and doc page uses the ordinal.
fn element_index(doc: &crate::model::Document, element: SceneElement) -> usize {
    match element {
        SceneElement::Image(key) => {
            doc.tracing_images.keys().position(|k| k == key).unwrap_or(0)
        }
        SceneElement::Revolution(key) => {
            doc.revolutions.keys().position(|k| k == key).unwrap_or(0)
        }
        SceneElement::SweepOp(key) => doc.sweeps.keys().position(|k| k == key).unwrap_or(0),
        SceneElement::Shape(key) => doc.primitives.keys().position(|k| k == key).unwrap_or(0),
        SceneElement::Body(key) => doc.bodies.keys().position(|k| k == key).unwrap_or(0),
        SceneElement::BooleanOp(key) => {
            doc.boolean_ops.keys().position(|k| k == key).unwrap_or(0)
        }
        SceneElement::MoveOp(key) => doc.move_ops.keys().position(|k| k == key).unwrap_or(0),
        SceneElement::MirrorOp(key) => {
            doc.mirror_ops.keys().position(|k| k == key).unwrap_or(0)
        }
        SceneElement::RepeatOp(key) => {
            doc.repeat_ops.keys().position(|k| k == key).unwrap_or(0)
        }
        SceneElement::SliceOp(key) => doc.slice_ops.keys().position(|k| k == key).unwrap_or(0),
        SceneElement::ShellOp(key) => doc.shell_ops.keys().position(|k| k == key).unwrap_or(0),
        SceneElement::SketchRepeatOp(key) => {
            doc.sketch_repeat_ops.keys().position(|k| k == key).unwrap_or(0)
        }
        SceneElement::SketchOffsetOp(key) => {
            doc.sketch_offset_ops.keys().position(|k| k == key).unwrap_or(0)
        }
        SceneElement::SketchMirrorOp(key) => {
            doc.sketch_mirror_ops.keys().position(|k| k == key).unwrap_or(0)
        }
        SceneElement::SketchVertexTreatmentOp(key) => doc
            .sketch_vertex_treatment_ops
            .keys()
            .position(|k| k == key)
            .unwrap_or(0),
        SceneElement::SketchSliceOp(key) => {
            doc.sketch_slice_ops.keys().position(|k| k == key).unwrap_or(0)
        }
        SceneElement::Joint(key) => doc.joints.keys().position(|k| k == key).unwrap_or(0),
        SceneElement::EdgeTreatmentOp(key) => {
            doc.edge_treatment_ops.keys().position(|k| k == key).unwrap_or(0)
        }
        SceneElement::Line(key) => doc.lines.keys().position(|k| k == key).unwrap_or(0),
        SceneElement::ConstructionPlane(key) => {
            doc.construction_planes.keys().position(|k| k == key).unwrap_or(0)
        }
        SceneElement::Circle(key) => doc.circles.keys().position(|k| k == key).unwrap_or(0),
        SceneElement::Sketch(key) => doc.sketches.keys().position(|k| k == key).unwrap_or(0),
        SceneElement::Constraint(key) => {
            doc.constraints.keys().position(|k| k == key).unwrap_or(0)
        }
        SceneElement::SketchText(key) => {
            doc.sketch_texts.keys().position(|k| k == key).unwrap_or(0)
        }
        SceneElement::Extrusion(key) => {
            doc.extrusions.keys().position(|k| k == key).unwrap_or(0)
        }
        SceneElement::Component(key) => {
            doc.components.keys().position(|k| k == key).unwrap_or(0)
        }
        SceneElement::UnitInstance(key) => {
            doc.unit_instances.keys().position(|k| k == key).unwrap_or(0)
        }
        // X/Y/Z index as 0/1/2 so a script can name one (#952).
        SceneElement::GlobalAxis(axis) => match axis {
            crate::construction::GlobalAxis::X => 0,
            crate::construction::GlobalAxis::Y => 1,
            crate::construction::GlobalAxis::Z => 2,
        },
        SceneElement::Point(_)
        | SceneElement::FaceEdge(_)
        | SceneElement::Origin
        | SceneElement::BodyEdge { .. }
        | SceneElement::BodyVertex { .. }
        | SceneElement::BodyFace { .. }
        | SceneElement::BodyCylinder { .. }
        | SceneElement::BodyAxis { .. }
        | SceneElement::SketchFace(_)
        | SceneElement::MovePoint(_) => 0,
        SceneElement::ExtrusionEdge { extrusion, .. } => {
            doc.extrusions.keys().position(|k| k == extrusion).unwrap_or(0)
        }
        SceneElement::PrimitiveEdge { primitive, .. } => {
            doc.primitives.keys().position(|k| k == primitive).unwrap_or(0)
        }
        SceneElement::RepeatedFace { instance, .. } => instance,
        // A drawing item indexes by its place on the page; a dimension has no index of its
        // own, so it reports the view it's on.
        SceneElement::DrawingElement { drawing, element } => {
            use crate::context::DrawingElementRef as D;
            match element {
                D::Projection(i) => i,
                D::Text(key) => doc
                    .drawings
                    .get(drawing)
                    .and_then(|d| d.annotations.keys().position(|k| k == key))
                    .unwrap_or(0),
                D::Dimension { view, .. } => view,
            }
        }
    }
}

/// The element a script's `(kind, index)` names (#1055) — the inverse of [`element_index`],
/// so an arena-backed kind resolves its ordinal to the key that element actually holds.
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
        "boolean_op" | "boolean" => {
            Some(SceneElement::BooleanOp(doc.boolean_ops.keys().nth(index)?))
        }
        "move_op" | "move" => Some(SceneElement::MoveOp(doc.move_ops.keys().nth(index)?)),
        "sketch_text" | "text" => {
            Some(SceneElement::SketchText(doc.sketch_texts.keys().nth(index)?))
        }
        "component" => Some(SceneElement::Component(doc.components.keys().nth(index)?)),
        "sketch_offset_op" | "offset" => {
            Some(SceneElement::SketchOffsetOp(doc.sketch_offset_ops.keys().nth(index)?))
        }
        "sketch_mirror_op" => {
            Some(SceneElement::SketchMirrorOp(doc.sketch_mirror_ops.keys().nth(index)?))
        }
        "sketch_vertex_treatment_op" | "chamfer_op" | "fillet_op" => Some(
            SceneElement::SketchVertexTreatmentOp(doc.sketch_vertex_treatment_ops.keys().nth(index)?),
        ),
        "mirror_op" | "mirror" => Some(SceneElement::MirrorOp(doc.mirror_ops.keys().nth(index)?)),
        "unit_instance" | "unit" => {
            Some(SceneElement::UnitInstance(doc.unit_instances.keys().nth(index)?))
        }
        "image" | "tracing_image" => {
            Some(SceneElement::Image(doc.tracing_images.keys().nth(index)?))
        }
        "revolution" | "revolve" => {
            Some(SceneElement::Revolution(doc.revolutions.keys().nth(index)?))
        }
        "sweep" | "sweep_op" => Some(SceneElement::SweepOp(doc.sweeps.keys().nth(index)?)),
        "joint" => Some(SceneElement::Joint(doc.joints.keys().nth(index)?)),
        "shape" | "primitive" => Some(SceneElement::Shape(doc.primitives.keys().nth(index)?)),
        // The world axes (#952) index as 0/1/2 for X/Y/Z, matching `element_index`.
        "axis" | "global_axis" => Some(SceneElement::GlobalAxis(match index {
            0 => crate::construction::GlobalAxis::X,
            1 => crate::construction::GlobalAxis::Y,
            2 => crate::construction::GlobalAxis::Z,
            _ => return None,
        })),
        _ => None,
    }
}

fn parse_visibility(value: Value) -> mlua::Result<Option<bool>> {
    match value {
        Value::Nil => Ok(None),
        Value::Boolean(b) => Ok(Some(b)),
        Value::String(s) => match s.to_str()?.to_ascii_lowercase().as_str() {
            "show" | "on" | "true" | "yes" | "1" => Ok(Some(true)),
            "hide" | "off" | "false" | "no" | "0" => Ok(Some(false)),
            "toggle" => Ok(None),
            other => Err(mlua::Error::external(format!(
                "unknown visibility value '{other}'"
            ))),
        },
        other => Err(mlua::Error::external(format!(
            "expected boolean or string for visibility, got {other:?}"
        ))),
    }
}

fn parse_bool(value: Value, label: &str) -> mlua::Result<bool> {
    match value {
        Value::Boolean(b) => Ok(b),
        Value::String(s) => match s.to_str()?.to_ascii_lowercase().as_str() {
            "true" | "on" | "yes" | "1" => Ok(true),
            "false" | "off" | "no" | "0" => Ok(false),
            other => Err(mlua::Error::external(format!(
                "unknown {label} value '{other}'"
            ))),
        },
        other => Err(mlua::Error::external(format!(
            "expected boolean for {label}, got {other:?}"
        ))),
    }
}

fn make_element(lua: &Lua, element: SceneElement) -> mlua::Result<Value> {
    let tick = lua
        .app_data_ref::<ScriptTickData>()
        .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
    let index = element_index(unsafe { &tick.state().doc }, element.clone());
    drop(tick);
    Ok(Value::UserData(lua.create_userdata(LuaElement { element, index })?))
}

/// The body a script's ordinal names (#1055) — a script counts live bodies, it cannot spell
/// a key.
fn body_key_from_ordinal(lua: &Lua, ordinal: usize) -> mlua::Result<crate::model::BodyKey> {
    let tick = lua
        .app_data_ref::<ScriptTickData>()
        .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
    let key = unsafe { tick.state().doc.body_at(ordinal) };
    key.ok_or_else(|| mlua::Error::external(format!("no body {ordinal}")))
}

/// The construction plane a script ordinal names (#1055) — planes are keyed, scripts count.
fn plane_key_from_ordinal(
    lua: &Lua,
    ordinal: usize,
) -> mlua::Result<crate::model::ConstructionPlaneKey> {
    let tick = lua
        .app_data_ref::<ScriptTickData>()
        .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
    let key = unsafe { tick.state().doc.construction_planes.keys().nth(ordinal) };
    key.ok_or_else(|| mlua::Error::external(format!("no construction plane {ordinal}")))
}

/// The line a script ordinal names (#1055) — lines are keyed, scripts count.
fn line_key_from_ordinal(lua: &Lua, ordinal: usize) -> mlua::Result<crate::model::LineKey> {
    let tick = lua
        .app_data_ref::<ScriptTickData>()
        .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
    let key = unsafe { tick.state().doc.lines.keys().nth(ordinal) };
    key.ok_or_else(|| mlua::Error::external(format!("no line {ordinal}")))
}

/// Every line in a script's ordinal list (#1055).
fn line_keys_from_ordinals(
    lua: &Lua,
    ordinals: Vec<usize>,
) -> mlua::Result<Vec<crate::model::LineKey>> {
    ordinals.into_iter().map(|o| line_key_from_ordinal(lua, o)).collect()
}

/// The circle a script ordinal names (#1055) — circles are keyed, scripts count.
fn circle_key_from_ordinal(lua: &Lua, ordinal: usize) -> mlua::Result<crate::model::CircleKey> {
    let tick = lua
        .app_data_ref::<ScriptTickData>()
        .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
    let key = unsafe { tick.state().doc.circles.keys().nth(ordinal) };
    key.ok_or_else(|| mlua::Error::external(format!("no circle {ordinal}")))
}

/// The sketch a script ordinal names (#1055) — sketches are keyed, scripts count.
fn sketch_key_from_ordinal(lua: &Lua, ordinal: usize) -> mlua::Result<crate::model::SketchId> {
    let tick = lua
        .app_data_ref::<ScriptTickData>()
        .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
    let key = unsafe { tick.state().doc.sketches.keys().nth(ordinal) };
    key.ok_or_else(|| mlua::Error::external(format!("no sketch {ordinal}")))
}

/// The sketch text a script ordinal names (#1055) — texts are keyed, scripts count.
fn sketch_text_key_from_ordinal(
    lua: &Lua,
    ordinal: usize,
) -> mlua::Result<crate::model::SketchTextKey> {
    let tick = lua
        .app_data_ref::<ScriptTickData>()
        .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
    let key = unsafe { tick.state().doc.sketch_texts.keys().nth(ordinal) };
    key.ok_or_else(|| mlua::Error::external(format!("no sketch text {ordinal}")))
}

/// The extrusion a script ordinal names (#1055) — extrusions are keyed, scripts count.
fn extrusion_key_from_ordinal(
    lua: &Lua,
    ordinal: usize,
) -> mlua::Result<crate::model::ExtrusionKey> {
    let tick = lua
        .app_data_ref::<ScriptTickData>()
        .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
    let key = unsafe { tick.state().doc.extrusions.keys().nth(ordinal) };
    key.ok_or_else(|| mlua::Error::external(format!("no extrusion {ordinal}")))
}

/// The unit instance a script ordinal names (#1055) — instances are keyed, scripts count.
fn unit_instance_key_from_ordinal(
    lua: &Lua,
    ordinal: usize,
) -> mlua::Result<crate::model::UnitInstanceKey> {
    let tick = lua
        .app_data_ref::<ScriptTickData>()
        .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
    let key = unsafe { tick.state().doc.unit_instances.keys().nth(ordinal) };
    key.ok_or_else(|| mlua::Error::external(format!("no unit instance {ordinal}")))
}

/// A `#rrggbb` (or bare `rrggbb`) colour string (#834).
fn parse_hex_color(text: &str) -> mlua::Result<[u8; 3]> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(mlua::Error::external(format!(
            "colour must be #rrggbb, got '{text}'"
        )));
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
    Ok([byte(0), byte(2), byte(4)])
}

/// The optional `{ shift = true }` table a scripted click can carry (#835).
/// A scripted click's `{ shift = …, ctrl = …, cmd = … }` options table (#835/#984).
/// `cmd` (#1408) is the platform primary modifier (⌘/Ctrl) that the copy/paste shortcuts read.
fn click_mods(opts: Option<Table>) -> mlua::Result<crate::script::ClickMods> {
    match opts {
        Some(t) => Ok(crate::script::ClickMods {
            shift: t.get::<Option<bool>>("shift")?.unwrap_or(false),
            ctrl: t.get::<Option<bool>>("ctrl")?.unwrap_or(false),
            cmd: t.get::<Option<bool>>("cmd")?.unwrap_or(false),
        }),
        None => Ok(crate::script::ClickMods::default()),
    }
}

fn resolve_element(lua: &Lua, value: Value) -> mlua::Result<SceneElement> {
    match value {
        Value::UserData(ud) => {
            if let Ok(el) = ud.borrow::<LuaElement>() {
                return Ok(el.element.clone());
            }
            Err(mlua::Error::external("expected bearcad element"))
        }
        Value::Table(table) => parse_element_table(lua, table),
        Value::String(s) => {
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let name = s.to_str()?.to_string();
            unsafe {
                find_element_by_name(&tick.state().doc, &name)
                    .ok_or_else(|| mlua::Error::external(format!("no element named '{name}'")))
            }
        }
        other => Err(mlua::Error::external(format!(
            "expected element, name string, or table, got {other:?}"
        ))),
    }
}

fn parse_element_table(lua: &Lua, table: Table) -> mlua::Result<SceneElement> {
    if let Ok(name) = table.get::<String>("name") {
        let tick = lua
            .app_data_ref::<ScriptTickData>()
            .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
        return unsafe {
            find_element_by_name(&tick.state().doc, &name).ok_or_else(|| {
                mlua::Error::external(format!("no element named '{name}'"))
            })
        };
    }
    let kind: String = table.get("kind").or_else(|_| table.get("type"))?;
    // A face's own vertex or edge (#26/#27): `{ kind = "face", face = { ... }, index = 0 }` for
    // a `FaceVertex`, or the same shape plus `edge = true` for a `FaceEdge`. Unlike the other
    // point-level selectors below, `kind` itself (not a sibling flag) signals this one, and
    // there's no plain-element fallback for it.
    if kind.eq_ignore_ascii_case("face") {
        if table.get::<Option<bool>>("edge")?.unwrap_or(false) {
            return Ok(SceneElement::FaceEdge(parse_constraint_line_table(lua, table)?));
        }
        return Ok(SceneElement::Point(parse_constraint_point_table(lua, table)?));
    }
    // A sketch origin axis (#189): `{ kind = "axis", axis = "x" | "y" }`, selectable so a
    // point can be constrained onto it.
    if kind.eq_ignore_ascii_case("axis") {
        return Ok(SceneElement::FaceEdge(parse_constraint_line_table(lua, table)?));
    }
    // The origin (#189): `{ kind = "origin" }`.
    if kind.eq_ignore_ascii_case("origin") {
        return Ok(SceneElement::Origin);
    }
    let index: usize = table.get("index")?;
    // Point-level selector (#68): a line endpoint (`end = "start"|"end"`), or an explicit
    // `point = true` (e.g. a circle's center) — otherwise
    // `kind`/`index` alone resolve to the whole element as before.
    // `point` is `true` for a circle's centre (#68) or a calibration point index for an
    // image (#425); `false`/absent resolves to the whole element.
    let point_flagged = !matches!(table.get::<Value>("point")?, Value::Nil | Value::Boolean(false));
    if table.contains_key("end")?
        || table.contains_key("corner")?
        || table.contains_key("anchor")?
        || point_flagged
    {
        return Ok(SceneElement::Point(parse_constraint_point_table(lua, table)?));
    }
    let tick = lua
        .app_data_ref::<ScriptTickData>()
        .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
    scene_element_from_kind(unsafe { &tick.state().doc }, &kind, index)
        .ok_or_else(|| mlua::Error::external(format!("unknown element kind '{kind}'")))
}

/// A `{x, y, z}` / `{1,2,3}` triple used by `body_mesh_face`: already-quantized integers
/// (export_lua, |component| > 2) or world millimetres / a unit vector (`body_faces`).
fn parse_quantized_or_world(table: &Table, key: &str) -> mlua::Result<[i32; 3]> {
    let t: Table = table.get(key)?;
    let x: f32 = t.get(1).or_else(|_| t.get("x"))?;
    let y: f32 = t.get(2).or_else(|_| t.get("y"))?;
    let z: f32 = t.get(3).or_else(|_| t.get("z"))?;
    if x.abs() > 2.0 || y.abs() > 2.0 || z.abs() > 2.0 {
        Ok([x.round() as i32, y.round() as i32, z.round() as i32])
    } else {
        Ok(crate::hierarchy::quantize_body_point(glam::Vec3::new(x, y, z)))
    }
}

/// Parses a `begin_sketch`/`face = { ... }` table into a `FaceId`. 3D body faces
/// (`extrude_cap`/`extrude_side`) need extra descriptors (extrusion + profile + which face), so
/// they can't go through the plain `(kind, index)` `FaceId::from_script` path; everything else
/// does. Shared by `begin_sketch` and the `face` arms of `parse_constraint_point_table`/
/// `parse_constraint_line_table` below (#26/#27's `FaceVertex`/`FaceEdge` from a script).
fn parse_face_id_table(lua: &Lua, table: Table) -> mlua::Result<FaceId> {
    let kind: String = table.get("kind").or_else(|_| table.get("type"))?;
    match kind.to_ascii_lowercase().as_str() {
        "extrude_cap" | "extrude_side" => {
            let extrusion = extrusion_key_from_ordinal(lua, table.get("extrusion")?)?;
            let profile_kind: String =
                table.get("profile").or_else(|_| table.get("profile_kind"))?;
            let profile_index: usize = table
                .get("profile_index")
                .or_else(|_| table.get("index"))
                .unwrap_or(0);
            let profile = match profile_kind.to_ascii_lowercase().as_str() {
                "circle" => crate::model::ExtrudeFace::Circle(circle_key_from_ordinal(
                    lua,
                    profile_index,
                )?),
                // A rectangle is now a `Polygon` loop (#66); give its four line indices as
                // `profile_lines = {..}`.
                "polygon" => {
                    let lines: Vec<usize> = table
                        .get("profile_lines")
                        .or_else(|_| table.get("lines"))?;
                    crate::model::ExtrudeFace::Polygon(line_keys_from_ordinals(lua, lines)?)
                }
                // A boolean-combined profile's cap (#406): `profile = "boolean",
                // boolean = { op, a = <face spec>, b = <face spec> }` — the same
                // descriptor `extrude`'s `boolean =` takes.
                "boolean" => {
                    let spec: Table = table.get("boolean")?;
                    parse_boolean_face_table(lua, &spec)?
                }
                other => {
                    return Err(mlua::Error::external(format!(
                        "unknown extrude profile kind '{other}' (circle|polygon|boolean)"
                    )))
                }
            };
            if kind.eq_ignore_ascii_case("extrude_cap") {
                let top: bool = table.get("top").unwrap_or(true);
                Ok(FaceId::ExtrudeCap {
                    extrusion,
                    profile,
                    top,
                })
            } else {
                let edge: u8 = table.get("edge").unwrap_or(0);
                Ok(FaceId::ExtrudeSide {
                    extrusion,
                    profile,
                    edge,
                })
            }
        }
        // A revolve's flat sides (#621): same profile descriptors as extrude_cap/
        // extrude_side, owned by a revolution instead of an extrusion. `revolve_cap` is a
        // partial sweep's start/end profile face (`end = bool`); `revolve_side` is the flat
        // washer face swept by one axis-perpendicular profile edge (`edge = i`).
        "revolve_cap" | "revolve_side" => {
            // The script's `revolution` is its ordinal among the live ones (#1055).
            let ordinal: usize = table.get("revolution")?;
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let revolution = unsafe { &tick.state().doc }
                .revolutions
                .keys()
                .nth(ordinal)
                .ok_or_else(|| mlua::Error::external(format!("no revolution {ordinal}")))?;
            drop(tick);
            let profile_kind: String =
                table.get("profile").or_else(|_| table.get("profile_kind"))?;
            let profile_index: usize = table
                .get("profile_index")
                .or_else(|_| table.get("index"))
                .unwrap_or(0);
            let profile = match profile_kind.to_ascii_lowercase().as_str() {
                "circle" => crate::model::ExtrudeFace::Circle(circle_key_from_ordinal(
                    lua,
                    profile_index,
                )?),
                "polygon" => {
                    let lines: Vec<usize> = table
                        .get("profile_lines")
                        .or_else(|_| table.get("lines"))?;
                    crate::model::ExtrudeFace::Polygon(line_keys_from_ordinals(lua, lines)?)
                }
                "boolean" => {
                    let spec: Table = table.get("boolean")?;
                    parse_boolean_face_table(lua, &spec)?
                }
                other => {
                    return Err(mlua::Error::external(format!(
                        "unknown revolve profile kind '{other}' (circle|polygon|boolean)"
                    )))
                }
            };
            if kind.eq_ignore_ascii_case("revolve_cap") {
                let end: bool = table.get("end").unwrap_or(false);
                Ok(FaceId::RevolveCap {
                    revolution,
                    profile,
                    end,
                })
            } else {
                let edge: u8 = table.get("edge").unwrap_or(0);
                Ok(FaceId::RevolveSide {
                    revolution,
                    profile,
                    edge,
                })
            }
        }
        // A flat face of a Shape-tool primitive (#1103): `primitive` is the ordinal among
        // live shapes; `face` names which side (`"top"`/`"bottom"`/`"side"` + `edge`, or the
        // cylinder caps / the serde snake_case tags).
        "primitive_face" => {
            let ordinal: usize = table.get("primitive")?;
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let primitive = unsafe { &tick.state().doc }
                .primitives
                .keys()
                .nth(ordinal)
                .ok_or_else(|| mlua::Error::external(format!("no primitive {ordinal}")))?;
            drop(tick);
            let face = parse_primitive_face_field(&table)?;
            Ok(FaceId::PrimitiveFace { primitive, face })
        }
        // A remaining flat on a treated/boolean/imported body (#1173/#1338).
        // `centroid`/`normal` are either already-quantized integers (export_lua) or
        // world millimetres / a unit vector (`body_faces`); |n| > 2 means quantized.
        "body_mesh_face" => {
            let body = body_key_from_ordinal(lua, table.get("body")?)?;
            let centroid = parse_quantized_or_world(&table, "centroid")
                .or_else(|_| parse_quantized_or_world(&table, "face"))?;
            let normal = parse_quantized_or_world(&table, "normal")?;
            Ok(FaceId::BodyMeshFace {
                body,
                centroid,
                normal,
            })
        }
        _ => {
            let index: usize = table.get("index")?;
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let face = unsafe { FaceId::from_script(&tick.state().doc, &kind, index) };
            face.ok_or_else(|| mlua::Error::external(format!("unknown sketch face kind '{kind}'")))
        }
    }
}

/// Parse a `face =` field naming a [`crate::model::PrimitiveFace`] (#1103/#1104).
fn parse_primitive_face_field(table: &Table) -> mlua::Result<crate::model::PrimitiveFace> {
    use crate::model::PrimitiveFace as F;
    // `face` may be a string tag, or a table carrying `edge` for a side wall.
    let face_val: mlua::Value = table.get("face")?;
    match face_val {
        mlua::Value::String(s) => {
            let tag = s.to_str()?.to_ascii_lowercase();
            match tag.as_str() {
                "top" | "cuboid_top" => Ok(F::CuboidTop),
                "bottom" | "cuboid_bottom" => Ok(F::CuboidBottom),
                "side" | "cuboid_side" => {
                    let edge: u8 = table.get("edge").unwrap_or(0);
                    Ok(F::CuboidSide { edge })
                }
                "cylinder_top" => Ok(F::CylinderTop),
                "cylinder_bottom" => Ok(F::CylinderBottom),
                other => Err(mlua::Error::external(format!(
                    "unknown primitive face '{other}' (top|bottom|side|cylinder_top|cylinder_bottom)"
                ))),
            }
        }
        mlua::Value::Table(t) => {
            // Serde-ish: `{ cuboid_side = { edge = 2 } }` or `{ edge = 2 }` with face="side"
            // already handled via string path; accept `{ kind = "side", edge = n }` too.
            if let Ok(kind) = t.get::<String>("kind").or_else(|_| t.get("type")) {
                let edge: u8 = t.get("edge").unwrap_or(0);
                return match kind.to_ascii_lowercase().as_str() {
                    "top" | "cuboid_top" => Ok(F::CuboidTop),
                    "bottom" | "cuboid_bottom" => Ok(F::CuboidBottom),
                    "side" | "cuboid_side" => Ok(F::CuboidSide { edge }),
                    "cylinder_top" => Ok(F::CylinderTop),
                    "cylinder_bottom" => Ok(F::CylinderBottom),
                    other => Err(mlua::Error::external(format!(
                        "unknown primitive face kind '{other}'"
                    ))),
                };
            }
            if let Ok(edge) = t.get::<u8>("edge") {
                return Ok(F::CuboidSide { edge });
            }
            Err(mlua::Error::external(
                "primitive face table needs kind= or edge=",
            ))
        }
        _ => Err(mlua::Error::external(
            "primitive face requires face = \"top\"|\"bottom\"|\"side\"|…",
        )),
    }
}

/// An `ExtrudeFace` from a face-spec table: `{rect = i}`, `{circle = i}`, `{polygon = {..}}`,
/// or a nested `{boolean = {op = "intersection"|"difference", a = <face spec>, b = <face
/// spec>}}` (#16/#62). Mirrors `extrude_face_spec_table`/`boolean_face_lua_table` in
/// src/script.rs, which render this same shape back out for the recorded-script export.
fn parse_extrude_face_table(
    lua: &Lua,
    table: &Table,
) -> mlua::Result<crate::model::ExtrudeFace> {
    if let Some(i) = table.get::<Option<usize>>("circle")? {
        return Ok(crate::model::ExtrudeFace::Circle(circle_key_from_ordinal(lua, i)?));
    }
    if let Some(lines) = table.get::<Option<Vec<usize>>>("polygon")? {
        return Ok(crate::model::ExtrudeFace::Polygon(line_keys_from_ordinals(lua, lines)?));
    }
    if let Some(boolean) = table.get::<Option<Table>>("boolean")? {
        return parse_boolean_face_table(lua, &boolean);
    }
    Err(mlua::Error::external(
        "face spec requires one of circle/polygon/boolean",
    ))
}

/// Parse a text-anchor name like `"center"` / `"top_left"` (#356).
fn parse_text_anchor(name: &str) -> mlua::Result<crate::model::TextAnchor> {
    use crate::model::TextAnchor as A;
    Ok(match name.to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
        "top_left" => A::TopLeft,
        "top_center" | "top" => A::TopCenter,
        "top_right" => A::TopRight,
        "middle_left" | "left" => A::MiddleLeft,
        "center" | "middle" | "" => A::Center,
        "middle_right" | "right" => A::MiddleRight,
        "bottom_left" => A::BottomLeft,
        "bottom_center" | "bottom" => A::BottomCenter,
        "bottom_right" => A::BottomRight,
        other => return Err(mlua::Error::external(format!("unknown text anchor '{other}'"))),
    })
}

fn parse_boolean_face_table(lua: &Lua, table: &Table) -> mlua::Result<crate::model::ExtrudeFace> {
    let op: String = table.get("op")?;
    let op = match op.to_ascii_lowercase().as_str() {
        "intersection" => crate::model::BooleanOp::Intersection,
        "difference" => crate::model::BooleanOp::Difference,
        other => {
            return Err(mlua::Error::external(format!(
                "unknown boolean op '{other}' (expected 'intersection' or 'difference')"
            )))
        }
    };
    let a: Table = table.get("a")?;
    let b: Table = table.get("b")?;
    Ok(crate::model::ExtrudeFace::Boolean {
        op,
        a: Box::new(parse_extrude_face_table(lua, &a)?),
        b: Box::new(parse_extrude_face_table(lua, &b)?),
    })
}

/// An `ExtrudeTarget` from a `to = {...}` table (#114): `{plane = i}` (construction plane),
/// `{face = <face spec>}` (a flat sketch profile's extended plane), `{face = <FaceId table>}`
/// (a 3D body's cap/side wall, #126 — the same `{kind = "extrude_cap"|"extrude_side", ...}`
/// shape `parse_face_id_table`/`begin_sketch` use, distinguished from the flat-profile shape
/// by the presence of a `kind`/`type` key), or `{vertex = <point table>}` (the plane through
/// that vertex). Mirrors `extrude_target_lua_table` in src/script.rs.
fn parse_extrude_target_table(
    lua: &Lua,
    table: &Table,
) -> mlua::Result<crate::model::ExtrudeTarget> {
    if let Some(i) = table.get::<Option<usize>>("plane")? {
        return Ok(crate::model::ExtrudeTarget::Plane(plane_key_from_ordinal(lua, i)?));
    }
    if let Some(face) = table.get::<Option<Table>>("face")? {
        let is_face_id_ref = face.get::<Option<String>>("kind")?.is_some()
            || face.get::<Option<String>>("type")?.is_some();
        if is_face_id_ref {
            let face_id = parse_face_id_table(lua, face)?;
            // A repeated instance's face (#452): `{ face = {...}, repeat_op = i,
            // instance = n }` targets the source face translated to instance `n`.
            if let Some(ordinal) = table.get::<Option<usize>>("repeat_op")? {
                let instance: usize = table.get::<Option<usize>>("instance")?.unwrap_or(1);
                // The script's `repeat_op` is an ordinal among the live ones (#1055).
                let tick = lua
                    .app_data_ref::<ScriptTickData>()
                    .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
                let op = unsafe { &tick.state().doc }
                    .repeat_ops
                    .keys()
                    .nth(ordinal)
                    .ok_or_else(|| mlua::Error::external(format!("no repeat {ordinal}")))?;
                drop(tick);
                return Ok(crate::model::ExtrudeTarget::RepeatedFace {
                    face: face_id,
                    op,
                    instance,
                });
            }
            return Ok(crate::model::ExtrudeTarget::BodyFace(face_id));
        }
        return Ok(crate::model::ExtrudeTarget::Face(parse_extrude_face_table(lua, 
            &face,
        )?));
    }
    if let Some(point) = table.get::<Option<Table>>("vertex")? {
        return Ok(crate::model::ExtrudeTarget::Vertex(
            parse_constraint_point_table(lua, point)?,
        ));
    }
    Err(mlua::Error::external(
        "extrude target requires one of plane/face/vertex",
    ))
}

fn parse_constraint_line_table(lua: &Lua, table: Table) -> mlua::Result<ConstraintLine> {
    let kind: String = table.get("kind").or_else(|_| table.get("type"))?;
    if kind.eq_ignore_ascii_case("face") {
        // { kind = "face", face = { kind = "extrude_cap", extrusion = 0, profile = "polygon",
        //   profile_lines = { 0, 1, 2, 3 }, top = true }, index = 2 } — edge `index` of that face's own
        // boundary loop (#26/#27's `FaceEdge`).
        let face_table: Table = table.get("face")?;
        let face = parse_face_id_table(lua, face_table)?;
        let index: usize = table.get("index")?;
        return Ok(ConstraintLine::FaceEdge { face, index });
    }
    if kind.eq_ignore_ascii_case("axis") {
        // { kind = "axis", axis = "x" | "y" } — a sketch origin axis (#189).
        let axis: String = table.get("axis")?;
        return match axis.to_ascii_lowercase().as_str() {
            "x" => Ok(ConstraintLine::OriginAxis(crate::model::SketchAxis::X)),
            "y" => Ok(ConstraintLine::OriginAxis(crate::model::SketchAxis::Y)),
            other => Err(mlua::Error::external(format!("unknown axis '{other}' (x|y)"))),
        };
    }
    let index: usize = table.get("index")?;
    match kind.to_ascii_lowercase().as_str() {
        "line" => Ok(ConstraintLine::Line(line_key_from_ordinal(lua, index)?)),
        other => Err(mlua::Error::external(format!(
            "drag_line target must be line, not '{other}'"
        ))),
    }
}

/// Read a chamfer/fillet amount argument as either a number or an expression string, returning it
/// as a parametric expression (#554) so `distance = "leg"` ties the treatment to a parameter.
fn lua_amount_expr(opts: &Table, key: &str) -> mlua::Result<String> {
    match opts.get::<mlua::Value>(key)? {
        mlua::Value::String(s) => Ok(s.to_str()?.to_string()),
        mlua::Value::Integer(i) => Ok(i.to_string()),
        mlua::Value::Number(n) => Ok(n.to_string()),
        mlua::Value::Nil => Err(mlua::Error::external(format!("`{key}` is required"))),
        _ => Err(mlua::Error::external(format!(
            "`{key}` must be a number or an expression string"
        ))),
    }
}

fn parse_constraint_point_table(lua: &Lua, table: Table) -> mlua::Result<ConstraintPoint> {
    let kind: String = table.get("kind").or_else(|_| table.get("type"))?;
    if kind.eq_ignore_ascii_case("origin") {
        return Ok(ConstraintPoint::Origin);
    }
    if kind.eq_ignore_ascii_case("face") {
        // { kind = "face", face = { ... }, index = 0 } — vertex `index` of that face's own
        // boundary loop (#26/#27's `FaceVertex`).
        let face_table: Table = table.get("face")?;
        let face = parse_face_id_table(lua, face_table)?;
        let index: usize = table.get("index")?;
        return Ok(ConstraintPoint::FaceVertex { face, index });
    }
    let index: usize = table.get("index")?;
    match kind.to_ascii_lowercase().as_str() {
        "line" => {
            let end_name: String = table.get("end")?;
            let end = match end_name.to_ascii_lowercase().as_str() {
                "start" | "0" => LineEnd::Start,
                "end" | "1" => LineEnd::End,
                other => {
                    return Err(mlua::Error::external(format!(
                        "unknown line endpoint '{other}'"
                    )));
                }
            };
            Ok(ConstraintPoint::LineEndpoint {
                line: line_key_from_ordinal(lua, index)?,
                end,
            })
        }
        "circle" => Ok(ConstraintPoint::CircleCenter(circle_key_from_ordinal(lua, index)?)),
        // A calibrated image's reference point (#425): `{ kind = "image", index = i,
        // point = 0|1 }`.
        "image" => {
            let point: usize = table.get("point")?;
            // The script's `index` is the image's ordinal among the live ones (#1055).
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let image = unsafe { &tick.state().doc }
                .tracing_images
                .keys()
                .nth(index)
                .ok_or_else(|| mlua::Error::external(format!("no image {index}")))?;
            Ok(ConstraintPoint::ImageCalibrationPoint { image, index: point })
        }
        // One of a sketch text's nine anchor points (#408): `{ kind = "sketch_text",
        // index = i, anchor = "center" }` (anchor defaults to center).
        "text" | "sketch_text" => {
            let anchor =
                parse_text_anchor(&table.get::<Option<String>>("anchor")?.unwrap_or_default())?;
            Ok(ConstraintPoint::TextAnchor {
                text: sketch_text_key_from_ordinal(lua, index)?,
                anchor,
            })
        }
        other => Err(mlua::Error::external(format!(
            "unknown point parent '{other}'"
        ))),
    }
}

/// Parses a `bearcad.chamfer_edge`/`fillet_edge` `edge = { ... }` table (#77) into an
/// `ExtrusionEdgeRef`: `{ kind = "vertical", face = 0, edge = 2 }` for the vertical edge
/// between side walls 2 and 3 of face 0, or `{ kind = "cap", face = 0, edge = 2, top = true }`
/// for the edge where side wall 2 meets the top (or, with `top = false`/omitted, base) cap.
fn parse_extrusion_edge_table(table: Table) -> mlua::Result<ExtrusionEdgeRef> {
    let kind: String = table.get("kind").or_else(|_| table.get("type"))?;
    let face: usize = table.get("face").unwrap_or(0);
    let edge: usize = table.get("edge")?;
    match kind.to_ascii_lowercase().as_str() {
        "vertical" => Ok(ExtrusionEdgeRef::Vertical { face, edge }),
        "cap" => {
            let top: bool = table.get("top").unwrap_or(false);
            Ok(ExtrusionEdgeRef::Cap { face, edge, top })
        }
        other => Err(mlua::Error::external(format!(
            "unknown extrusion edge kind '{other}' (expected 'vertical' or 'cap')"
        ))),
    }
}

/// Parses the edge argument of `bearcad.chamfer_edge`/`fillet_edge`: either a single
/// `edge = { ... }` alongside a top-level `extrusion` or `primitive`, or `edges = { {...}, ... }`
/// — a whole set treated by one operation (#672). Each entry of `edges` may name its own
/// host, falling back to the top-level one. The plural form matters: two one-edge operations
/// each bevel the solid's own body, so their outputs overlap instead of compounding.
fn parse_extrusion_edge_set(
    opts: &Table,
) -> mlua::Result<Vec<(TreatableSolidRef, ExtrusionEdgeRef)>> {
    let default_host = parse_treatable_solid_ref(opts)?;
    if let Some(list) = opts.get::<Option<Vec<Table>>>("edges")? {
        if list.is_empty() {
            return Err(mlua::Error::external("`edges` must name at least one edge"));
        }
        return list
            .into_iter()
            .map(|entry| {
                // An entry is either { extrusion/primitive = i, edge = {...} } or the edge
                // table itself, whose own `edge` field is an index — so the shape, not the
                // key, decides.
                let wrapped = match entry.get::<Value>("edge")? {
                    Value::Table(inner) => Some(inner),
                    _ => None,
                };
                let (host, edge_table) = match wrapped {
                    Some(inner) => (parse_treatable_solid_ref(&entry)?, inner),
                    None => (None, entry),
                };
                let host = host.or(default_host).ok_or_else(|| {
                    mlua::Error::external(
                        "each `edges` entry needs an `extrusion` or `primitive`",
                    )
                })?;
                Ok((host, parse_extrusion_edge_table(edge_table)?))
            })
            .collect();
    }
    let host = default_host.ok_or_else(|| {
        mlua::Error::external("chamfer_edge/fillet_edge requires an `extrusion` or `primitive`")
    })?;
    let edge_table: Table = opts.get("edge")?;
    Ok(vec![(host, parse_extrusion_edge_table(edge_table)?)])
}

fn parse_treatable_solid_ref(opts: &Table) -> mlua::Result<Option<TreatableSolidRef>> {
    let extrusion: Option<usize> = opts.get("extrusion")?;
    let primitive: Option<usize> = opts.get("primitive")?;
    match (extrusion, primitive) {
        (Some(i), None) => Ok(Some(TreatableSolidRef::Extrusion(i))),
        (None, Some(i)) => Ok(Some(TreatableSolidRef::Primitive(i))),
        (Some(_), Some(_)) => Err(mlua::Error::external(
            "give `extrusion` or `primitive`, not both",
        )),
        (None, None) => Ok(None),
    }
}

/// Parses `bearcad.combine{}`/`bearcad.edit_boolean{}` arguments: the op kind, the A and
/// B input body lists, and the keep-B flag.
fn parse_boolean_op_args(
    opts: &Table,
) -> mlua::Result<(crate::model::BooleanOpKind, Vec<usize>, Vec<usize>, bool)> {
    let op_name: String = opts
        .get::<Option<String>>("op")?
        .unwrap_or_else(|| "combine".to_string());
    let kind = crate::model::BooleanOpKind::from_name(&op_name).ok_or_else(|| {
        mlua::Error::external(format!(
            "unknown boolean op '{op_name}' (combine|cut|intersect|difference)"
        ))
    })?;
    let a: Vec<usize> = opts.get::<Option<Vec<usize>>>("a")?.unwrap_or_default();
    let b: Vec<usize> = opts.get::<Option<Vec<usize>>>("b")?.unwrap_or_default();
    let keep_b: bool = opts.get::<Option<bool>>("keep_b")?.unwrap_or(false);
    Ok((kind, a, b, keep_b))
}

/// Parses an `axis = …` argument into a [`crate::model::RevolveAxis`]: `"x"`/`"y"`/`"z"` for
/// an origin axis, `{ line = i }` for a sketch line, or `{ body = i, from = {x,y,z},
/// to = {x,y,z} }` for a body edge (#643). `what` names the call in error messages.
fn parse_revolve_axis(
    lua: &Lua,
    value: Value,
    what: &str,
) -> mlua::Result<crate::model::RevolveAxis> {
    const SHAPES: &str = "\"x\"|\"y\"|\"z\", {line = i}, or {body = i, from = {x,y,z}, to = {x,y,z}}";
    match value {
        Value::String(sv) => match sv.to_string_lossy().to_lowercase().as_str() {
            "x" => Ok(crate::model::RevolveAxis::X),
            "y" => Ok(crate::model::RevolveAxis::Y),
            "z" => Ok(crate::model::RevolveAxis::Z),
            other => Err(mlua::Error::external(format!(
                "unknown {what} axis '{other}' ({SHAPES})"
            ))),
        },
        Value::Table(t) => {
            if let Some(li) = t.get::<Option<usize>>("line")? {
                return Ok(crate::model::RevolveAxis::Line(line_key_from_ordinal(lua, li)?));
            }
            let ordinal: usize = t.get("body").map_err(|_| {
                mlua::Error::external(format!("{what} `axis` table needs `line` or `body` ({SHAPES})"))
            })?;
            let body = body_key_from_ordinal(lua, ordinal)?;
            let point = |key: &str| -> mlua::Result<glam::Vec3> {
                let v: Vec<f32> = t.get(key)?;
                if v.len() != 3 {
                    return Err(mlua::Error::external(format!(
                        "{what} `axis.{key}` must be {{x, y, z}}"
                    )));
                }
                Ok(glam::Vec3::new(v[0], v[1], v[2]))
            };
            Ok(crate::model::RevolveAxis::BodyEdge {
                body,
                a: point("from")?,
                b: point("to")?,
            })
        }
        _ => Err(mlua::Error::external(format!(
            "{what} `axis` must be {SHAPES}"
        ))),
    }
}

/// Parses `bearcad.move_bodies{}`/`bearcad.edit_move{}` arguments. Numbers are accepted
/// for the expression fields and stringified.
#[allow(clippy::type_complexity)]
fn parse_move_op_args(
    lua: &Lua,
    opts: &Table,
) -> mlua::Result<(
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
)> {
    let targets: Vec<usize> = opts.get::<Option<Vec<usize>>>("bodies")?.unwrap_or_default();
    let expr = |key: &str| -> mlua::Result<String> {
        Ok(match opts.get::<Value>(key)? {
            Value::Nil => String::new(),
            Value::String(s) => s.to_str()?.to_string(),
            Value::Integer(i) => i.to_string(),
            Value::Number(n) => n.to_string(),
            _ => {
                return Err(mlua::Error::external(format!(
                    "move `{key}` must be an expression string or a number"
                )))
            }
        })
    };
    let (tx, ty, tz) = (expr("x")?, expr("y")?, expr("z")?);
    // Free-mode turns about the world axes (#1076), in degrees.
    let (rx, ry, rz) = (expr("rx")?, expr("ry")?, expr("rz")?);
    // The third pair as an angle (#1078).
    let roll_angle = expr("roll")?;
    // Face Snap's side flip and its turn about the target normal (#1077).
    let face_flip = opts.get::<Option<bool>>("flip")?.unwrap_or(false);
    let face_spin = expr("spin")?;
    let face_offset = expr("gap")?;
    // Naming both points makes the translation a **snap** (#648/#649/#650): the move lands
    // `from` exactly on `to`, and x/y/z are ignored.
    let start_point_a = parse_move_point(lua, opts.get::<Value>("from")?, "from")?;
    let end_point_a = parse_move_point(lua, opts.get::<Value>("to")?, "to")?;
    // The optional B pair (#669) adds the rotation about end point A.
    let start_point_b = parse_move_point(lua, opts.get::<Value>("from_b")?, "from_b")?;
    let end_point_b = parse_move_point(lua, opts.get::<Value>("to_b")?, "to_b")?;
    // The optional C pair pins the spin about `end A → end B` that B leaves free.
    let start_point_c = parse_move_point(lua, opts.get::<Value>("from_c")?, "from_c")?;
    let end_point_c = parse_move_point(lua, opts.get::<Value>("to_c")?, "to_c")?;
    Ok((
        targets,
        tx,
        ty,
        tz,
        rx,
        ry,
        rz,
        roll_angle,
        face_flip,
        face_spin,
        face_offset,
        start_point_a,
        end_point_a,
        start_point_b,
        end_point_b,
        start_point_c,
        end_point_c,
    ))
}

/// A [`crate::model::MovePointRef`] from a `{ body = i, vertex = {x,y,z} }` or
/// `{ body = i, edge = { {x,y,z}, {x,y,z} } }` table (#649/#650). Coordinates are plain
/// millimetres on the body's mesh, re-quantized to the selection grid.
fn parse_move_point(
    lua: &Lua,
    value: Value,
    what: &str,
) -> mlua::Result<Option<crate::model::MovePointRef>> {
    let Value::Table(t) = value else {
        return match value {
            Value::Nil => Ok(None),
            _ => Err(mlua::Error::external(format!(
                "move `{what}` must be {{body = i, vertex = {{x,y,z}}}}, \
                 {{body = i, edge = {{{{x,y,z}}, {{x,y,z}}}}}}, or {{origin = true}}"
            ))),
        };
    };
    // The world origin (#946): no body, so it's spelled on its own.
    if t.get::<Option<bool>>("origin")?.unwrap_or(false) {
        return Ok(Some(crate::model::MovePointRef::Origin));
    }
    let body = body_key_from_ordinal(lua, t.get("body")?)?;
    let mm = |v: Vec<f32>| -> mlua::Result<[i32; 3]> {
        if v.len() != 3 {
            return Err(mlua::Error::external(format!(
                "move `{what}` points must be {{x, y, z}} in mm"
            )));
        }
        Ok(crate::hierarchy::quantize_body_point(glam::Vec3::new(
            v[0], v[1], v[2],
        )))
    };
    if let Some(v) = t.get::<Option<Vec<f32>>>("vertex")? {
        return Ok(Some(crate::model::MovePointRef::Vertex { body, p: mm(v)? }));
    }
    // A point along an edge (#670), by its position rather than by which edge it's on.
    if let Some(v) = t.get::<Option<Vec<f32>>>("on_edge")? {
        return Ok(Some(crate::model::MovePointRef::OnEdge { body, p: mm(v)? }));
    }
    // A point on a face (#738/#1074): the face's centroid plus its normal — the selection
    // key — and optionally how far across the face to sit, in the face's own axes. Named
    // `on_face`, and `face_center` still spells the middle of one.
    let face_key = match t.get::<Option<Vec<f32>>>("on_face")? {
        Some(v) => Some(v),
        None => t.get::<Option<Vec<f32>>>("face_center")?,
    };
    if let Some(v) = face_key {
        let n: Vec<f32> = t.get("normal").map_err(|_| {
            mlua::Error::external(format!("move `{what}.on_face` needs a `normal`"))
        })?;
        let uv = match t.get::<Option<Vec<f32>>>("uv")? {
            Some(uv) if uv.len() == 2 => {
                [(uv[0] * 100.0).round() as i32, (uv[1] * 100.0).round() as i32]
            }
            Some(_) => {
                return Err(mlua::Error::external(format!(
                    "move `{what}.uv` needs two numbers"
                )))
            }
            None => [0, 0],
        };
        return Ok(Some(crate::model::MovePointRef::OnFace {
            body,
            centroid: mm(v)?,
            normal: mm(n)?,
            uv,
        }));
    }
    let ends: Vec<Vec<f32>> = t.get("edge").map_err(|_| {
        mlua::Error::external(format!("move `{what}` needs a `vertex` or an `edge`"))
    })?;
    if ends.len() != 2 {
        return Err(mlua::Error::external(format!(
            "move `{what}.edge` must be two {{x, y, z}} points"
        )));
    }
    Ok(Some(crate::model::MovePointRef::EdgeMidpoint {
        body,
        a: mm(ends[0].clone())?,
        b: mm(ends[1].clone())?,
    }))
}


/// One side of a mate pick (#1020): a body face, a datum plane, a body edge, a world axis,
/// or a point. The point spellings are the Move tool's, except that an edge **midpoint** is
/// `midpoint` here — `edge` names the whole edge, which is what a line-up row lines up.
fn parse_mate_ref(
    lua: &Lua,
    value: Value,
    what: &str,
) -> mlua::Result<Option<crate::model::MateRef>> {
    let Value::Table(t) = value else {
        return match value {
            Value::Nil => Ok(None),
            _ => Err(mlua::Error::external(format!(
                "`{what}` must be {{body = i, face = {{x,y,z}}, normal = {{x,y,z}}}}, \
                 {{plane = i}}, {{body = i, edge = {{{{x,y,z}}, {{x,y,z}}}}}}, \
                 {{axis = \"x\"}}, or a point"
            ))),
        };
    };
    let mm = |v: Vec<f32>| -> mlua::Result<[i32; 3]> {
        if v.len() != 3 {
            return Err(mlua::Error::external(format!(
                "`{what}` points must be {{x, y, z}} in mm"
            )));
        }
        Ok(crate::hierarchy::quantize_body_point(glam::Vec3::new(
            v[0], v[1], v[2],
        )))
    };
    if let Some(i) = t.get::<Option<usize>>("plane")? {
        return Ok(Some(crate::model::MateRef::Plane(plane_key_from_ordinal(lua, i)?)));
    }
    // A hole's or a shaft's centre line (#1013).
    if let Some(v) = t.get::<Option<Vec<f32>>>("hole_axis")? {
        let body = body_key_from_ordinal(lua, t.get("body")?)?;
        let d: Vec<f32> = t
            .get("direction")
            .map_err(|_| mlua::Error::external(format!("`{what}.hole_axis` needs a `direction`")))?;
        return Ok(Some(crate::model::MateRef::HoleAxis {
            body,
            origin: mm(v)?,
            dir: mm(d)?,
        }));
    }
    if let Some(name) = t.get::<Option<String>>("axis")? {
        let axis = match name.to_ascii_lowercase().as_str() {
            "x" => crate::construction::GlobalAxis::X,
            "y" => crate::construction::GlobalAxis::Y,
            "z" => crate::construction::GlobalAxis::Z,
            other => {
                return Err(mlua::Error::external(format!(
                    "unknown axis '{other}' (expected 'x', 'y' or 'z')"
                )))
            }
        };
        return Ok(Some(crate::model::MateRef::Axis(axis)));
    }
    if let Some(v) = t.get::<Option<Vec<f32>>>("face")? {
        let body = body_key_from_ordinal(lua, t.get("body")?)?;
        let n: Vec<f32> = t
            .get("normal")
            .map_err(|_| mlua::Error::external(format!("`{what}.face` needs a `normal`")))?;
        return Ok(Some(crate::model::MateRef::Face {
            body,
            centroid: mm(v)?,
            normal: mm(n)?,
        }));
    }
    if let Some(ends) = t.get::<Option<Vec<Vec<f32>>>>("edge")? {
        let body = body_key_from_ordinal(lua, t.get("body")?)?;
        if ends.len() != 2 {
            return Err(mlua::Error::external(format!(
                "`{what}.edge` must be two {{x, y, z}} points"
            )));
        }
        return Ok(Some(crate::model::MateRef::Edge {
            body,
            a: mm(ends[0].clone())?,
            b: mm(ends[1].clone())?,
        }));
    }
    if let Some(ends) = t.get::<Option<Vec<Vec<f32>>>>("midpoint")? {
        let body = body_key_from_ordinal(lua, t.get("body")?)?;
        if ends.len() != 2 {
            return Err(mlua::Error::external(format!(
                "`{what}.midpoint` must be two {{x, y, z}} points"
            )));
        }
        return Ok(Some(crate::model::MateRef::Point(
            crate::model::MovePointRef::EdgeMidpoint {
                body,
                a: mm(ends[0].clone())?,
                b: mm(ends[1].clone())?,
            },
        )));
    }
    parse_move_point(lua, Value::Table(t), what).map(|p| p.map(crate::model::MateRef::Point))
}

/// The `face = {…}` block of a joint call (#1020).
/// A joint's placement from its `face = { moving, fixed, flip?, offset?, spin? }` table
/// (#1020/#1079): a **Face Snap** move, which is what a mate always was.
fn parse_mate(lua: &Lua, opts: &Table) -> mlua::Result<crate::model::MoveOperation> {
    let mut placement = crate::model::MoveOperation::default();
    let Some(face) = opts.get::<Option<Table>>("face")? else {
        return Ok(placement);
    };
    check_keys(&face, "joint face", &["moving", "fixed", "flip", "offset", "spin"])?;
    // Naming a face and no point means its **middle** — the accurate one (#1080).
    let middle_uv = |body, centroid, normal| {
        let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
        crate::extrude::face_middle_uv(unsafe { &tick.state().doc }, body, centroid, normal)
    };
    let point = |r: Option<crate::model::MateRef>| match r {
        Some(crate::model::MateRef::Face { body, centroid, normal }) => {
            Ok(Some(crate::model::MovePointRef::OnFace {
                body,
                centroid,
                normal,
                uv: middle_uv(body, centroid, normal),
            }))
        }
        Some(_) => Err(mlua::Error::external(
            "a joint's `face` picks must be flat faces".to_string(),
        )),
        None => Ok(None),
    };
    placement.translate_mode = crate::model::MoveTranslateMode::FaceSnap;
    placement.start_point_a = point(parse_mate_ref(lua, face.get("moving")?, "face.moving")?)?;
    placement.end_point_a = point(parse_mate_ref(lua, face.get("fixed")?, "face.fixed")?)?;
    placement.face_flip = face.get::<Option<bool>>("flip")?.unwrap_or(false);
    placement.face_offset = joint_position_arg(&face, "offset")?;
    placement.face_spin = joint_position_arg(&face, "spin")?;
    Ok(placement)
}

/// Keys every shape call accepts (#909).
fn check_shape_keys(opts: &Table, call: &str) -> mlua::Result<()> {
    check_keys(
        opts,
        call,
        &[
            "index", "shape", "at", "normal", "u_axis", "width", "depth", "height",
            "radius", "name",
        ],
    )
}

/// Parse a shape call's arguments (#909) into a [`crate::model::Primitive`]. Dimensions
/// take a number or an expression string; the frame defaults to the ground at the origin.
fn parse_shape_args(
    lua: &Lua,
    opts: &Table,
    kind: crate::model::PrimitiveKind,
    call: &str,
) -> mlua::Result<crate::model::Primitive> {
    check_shape_keys(opts, call)?;
    let mut shape = crate::model::Primitive::new(kind);
    let point = |key: &str| -> mlua::Result<Option<[f32; 3]>> {
        match opts.get::<Option<Vec<f32>>>(key)? {
            Some(v) if v.len() == 3 => Ok(Some([v[0], v[1], v[2]])),
            Some(_) => Err(mlua::Error::external(format!(
                "`{key}` must be {{x, y, z}} in mm"
            ))),
            None => Ok(None),
        }
    };
    if let Some(p) = point("at")? {
        shape.origin = p;
    }
    if let Some(p) = point("normal")? {
        shape.normal = p;
    }
    if let Some(p) = point("u_axis")? {
        shape.u_axis = p;
    }
    let expression = |key: &str| -> mlua::Result<String> {
        Ok(match scalar_arg(lua, opts, key)? {
            Some((value, Some(expression))) => {
                let _ = value;
                expression
            }
            Some((value, None)) => format!("{value}"),
            None => String::new(),
        })
    };
    shape.width = expression("width")?;
    shape.depth = expression("depth")?;
    shape.height = expression("height")?;
    shape.radius = expression("radius")?;
    Ok(shape)
}

/// Parses `bearcad.repeat_bodies{}`/`bearcad.edit_repeat{}` arguments.
#[allow(clippy::type_complexity)]
fn parse_repeat_op_args(
    lua: &Lua,
    opts: &Table,
) -> mlua::Result<(
    Vec<usize>,
    crate::model::RevolveAxis,
    bool,
    bool,
    crate::model::RepeatMode,
    String,
    String,
    String,
    Option<crate::model::ExtrudeTarget>,
)> {
    let targets: Vec<usize> = opts.get::<Option<Vec<usize>>>("bodies")?.unwrap_or_default();
    let axis = match opts.get::<Value>("axis")? {
        Value::Nil => crate::model::RevolveAxis::X,
        value => parse_revolve_axis(lua, value, "repeat")?,
    };
    let mode_name: String = opts
        .get::<Option<String>>("mode")?
        .unwrap_or_else(|| "count_gap".to_string());
    let mode = crate::model::RepeatMode::from_name(&mode_name).ok_or_else(|| {
        mlua::Error::external(format!(
            "unknown repeat mode '{mode_name}' (count_gap|count_fit_ends|count_fit_centers|fill_gap|fill_pitch|fill_max_pitch)"
        ))
    })?;
    let expr = |key: &str| -> mlua::Result<String> {
        Ok(match opts.get::<Value>(key)? {
            Value::Nil => String::new(),
            Value::String(s) => s.to_str()?.to_string(),
            Value::Integer(i) => i.to_string(),
            Value::Number(n) => n.to_string(),
            _ => {
                return Err(mlua::Error::external(format!(
                    "repeat `{key}` must be an expression string or a number"
                )))
            }
        })
    };
    // `gap` is what the Repeat pane calls the field; accept it as an alias of `spacing` (#403).
    let spacing = match (expr("spacing")?, expr("gap")?) {
        (s, g) if !s.is_empty() && !g.is_empty() => {
            return Err(mlua::Error::external(
                "repeat takes `spacing` or its alias `gap`, not both",
            ))
        }
        (s, g) if s.is_empty() => g,
        (s, _) => s,
    };
    // `to = {...}` picks a face/plane/vertex the fill length is measured to (#645), the same
    // table shape the Extrude tool's "up to" takes.
    let length_target = match opts.get::<Value>("to")? {
        Value::Nil => None,
        Value::Table(t) => Some(parse_extrude_target_table(lua, &t)?),
        _ => {
            return Err(mlua::Error::external(
                "repeat `to` must be a target table, e.g. {plane = i} or {face = …}",
            ))
        }
    };
    // `around = true` turns the copies about the axis instead of sliding them along it
    // (#839); `spacing`/`length` are then angles in degrees.
    let around_axis: bool = opts.get::<Option<bool>>("around")?.unwrap_or(false);
    // `flip = true` runs the pattern the other way along the path (#989).
    let flip: bool = opts.get::<Option<bool>>("flip")?.unwrap_or(false);
    Ok((
        targets,
        axis,
        around_axis,
        flip,
        mode,
        expr("count")?,
        spacing,
        expr("length")?,
        length_target,
    ))
}

/// Parses `bearcad.offset_sketch{}`/`bearcad.edit_sketch_offset{}` arguments: the host
/// `sketch`, the `lines`/`circles` operand index lists, the signed `distance`
/// expression, and the `construction` output toggle.
fn parse_sketch_offset_op_args(
    opts: &Table,
) -> mlua::Result<(usize, Vec<usize>, Vec<usize>, String, bool)> {
    let sketch: usize = opts.get::<Option<usize>>("sketch")?.unwrap_or(0);
    let lines: Vec<usize> = opts.get::<Option<Vec<usize>>>("lines")?.unwrap_or_default();
    let circles: Vec<usize> = opts.get::<Option<Vec<usize>>>("circles")?.unwrap_or_default();
    let distance = match opts.get::<Value>("distance")? {
        Value::Nil => {
            return Err(mlua::Error::external("offset_sketch requires a `distance`"))
        }
        Value::String(s) => s.to_str()?.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        _ => {
            return Err(mlua::Error::external(
                "offset_sketch `distance` must be an expression string or a number",
            ))
        }
    };
    let construction: bool = opts.get::<Option<bool>>("construction")?.unwrap_or(false);
    Ok((sketch, lines, circles, distance, construction))
}

/// Parses `bearcad.repeat_sketch{}`/`bearcad.edit_sketch_repeat{}` arguments (#222): the host
/// `sketch`, the `lines`/`circles` operand index lists, the in-plane direction (`angle` in
/// degrees — 0 is +u — or an explicit `dir = {du, dv}`), and the shared spacing mode/expressions.
#[allow(clippy::type_complexity)]
fn parse_sketch_repeat_op_args(
    opts: &Table,
) -> mlua::Result<(
    usize,
    Vec<usize>,
    Vec<usize>,
    f32,
    f32,
    crate::model::RepeatMode,
    String,
    String,
    String,
)> {
    // `sketch` is required to create (which sketch to duplicate in) but ignored on edit (the op
    // already knows its sketch), so default it rather than erroring when omitted.
    let sketch: usize = opts.get::<Option<usize>>("sketch")?.unwrap_or(0);
    let lines: Vec<usize> = opts.get::<Option<Vec<usize>>>("lines")?.unwrap_or_default();
    let circles: Vec<usize> = opts.get::<Option<Vec<usize>>>("circles")?.unwrap_or_default();
    let (dir_u, dir_v) = match opts.get::<Value>("dir")? {
        Value::Table(t) => {
            let u: f32 = t.get::<f32>(1).or_else(|_| t.get("u"))?;
            let v: f32 = t.get::<f32>(2).or_else(|_| t.get("v"))?;
            (u, v)
        }
        _ => {
            let deg: f64 = match opts.get::<Value>("angle")? {
                Value::Nil => 0.0,
                Value::Integer(i) => i as f64,
                Value::Number(n) => n,
                Value::String(s) => s.to_str()?.parse().map_err(|_| {
                    mlua::Error::external("repeat_sketch `angle` must be a number of degrees")
                })?,
                _ => return Err(mlua::Error::external("repeat_sketch `angle` must be a number")),
            };
            let r = deg.to_radians();
            (r.cos() as f32, r.sin() as f32)
        }
    };
    let mode_name: String = opts
        .get::<Option<String>>("mode")?
        .unwrap_or_else(|| "count_gap".to_string());
    let mode = crate::model::RepeatMode::from_name(&mode_name).ok_or_else(|| {
        mlua::Error::external(format!("unknown repeat mode '{mode_name}'"))
    })?;
    let expr = |key: &str| -> mlua::Result<String> {
        Ok(match opts.get::<Value>(key)? {
            Value::Nil => String::new(),
            Value::String(s) => s.to_str()?.to_string(),
            Value::Integer(i) => i.to_string(),
            Value::Number(n) => n.to_string(),
            _ => {
                return Err(mlua::Error::external(format!(
                    "repeat_sketch `{key}` must be an expression string or a number"
                )))
            }
        })
    };
    // `gap` is the pane's name for the field; alias of `spacing` (#403).
    let spacing = match (expr("spacing")?, expr("gap")?) {
        (s, g) if !s.is_empty() && !g.is_empty() => {
            return Err(mlua::Error::external(
                "repeat takes `spacing` or its alias `gap`, not both",
            ))
        }
        (s, g) if s.is_empty() => g,
        (s, _) => s,
    };
    Ok((
        sketch,
        lines,
        circles,
        dir_u,
        dir_v,
        mode,
        expr("count")?,
        spacing,
        expr("length")?,
    ))
}

/// Parses `bearcad.slice{}`/`bearcad.edit_slice{}` arguments: the target body list, the
/// cutters (face-spec tables or `{ kind = "line", index = i }` laser paths, #1126), and
/// the extend-to-infinity flag.
fn parse_slice_op_args(
    lua: &Lua,
    opts: &Table,
) -> mlua::Result<(Vec<usize>, Vec<crate::model::SliceCutter>, bool)> {
    let targets: Vec<usize> = opts.get::<Option<Vec<usize>>>("bodies")?.unwrap_or_default();
    let mut cutters: Vec<crate::model::SliceCutter> = Vec::new();
    if let Some(list) = opts.get::<Option<Vec<Table>>>("cutters")? {
        for table in list {
            cutters.push(parse_slice_cutter_table(lua, table)?);
        }
    }
    let extend_infinite: bool = opts.get::<Option<bool>>("extend")?.unwrap_or(true);
    Ok((targets, cutters, extend_infinite))
}

/// Parse a `bearcad.shell`/`edit_shell` table into `(bodies, open faces, thickness)` (#1156).
fn parse_shell_op_args(
    lua: &Lua,
    opts: &Table,
) -> mlua::Result<(Vec<usize>, Vec<crate::model::FaceId>, String)> {
    let targets: Vec<usize> = opts.get::<Option<Vec<usize>>>("bodies")?.unwrap_or_default();
    let mut open_faces: Vec<crate::model::FaceId> = Vec::new();
    if let Some(list) = opts.get::<Option<Vec<Table>>>("faces")? {
        for table in list {
            open_faces.push(parse_face_id_table(lua, table)?);
        }
    }
    let thickness: String = opts
        .get::<Option<String>>("thickness")?
        .unwrap_or_else(|| "1".to_string());
    Ok((targets, open_faces, thickness))
}

/// One slice cutter table: `{ kind = "line", index = i }` or a planar face-spec.
fn parse_slice_cutter_table(lua: &Lua, table: Table) -> mlua::Result<crate::model::SliceCutter> {
    let kind: Option<String> = table.get("kind").or_else(|_| table.get("type")).ok();
    if kind
        .as_deref()
        .is_some_and(|k| k.eq_ignore_ascii_case("line"))
    {
        let index: usize = table.get("index")?;
        let line = line_key_from_ordinal(lua, index)?;
        return Ok(crate::model::SliceCutter::Line { line });
    }
    Ok(crate::model::SliceCutter::Face(parse_face_id_table(lua, table)?))
}

/// Parse a `bearcad.mirror_sketch`/`edit_sketch_mirror` table into
/// `(sketch, mirror_line, lines, circles)` (#523/#528).
fn parse_sketch_mirror_op_args(
    opts: &Table,
) -> mlua::Result<(usize, usize, Vec<usize>, Vec<usize>)> {
    let sketch: usize = opts.get::<Option<usize>>("sketch")?.unwrap_or(0);
    let line: usize = opts
        .get::<Option<usize>>("line")?
        .ok_or_else(|| mlua::Error::external("mirror_sketch requires a `line` (the mirror axis)"))?;
    let lines: Vec<usize> = opts.get::<Option<Vec<usize>>>("lines")?.unwrap_or_default();
    let circles: Vec<usize> = opts.get::<Option<Vec<usize>>>("circles")?.unwrap_or_default();
    Ok((sketch, line, lines, circles))
}

/// A construction-plane ordinal (`plane = 0`) or a face-spec table (#1354).
fn parse_mirror_plane(lua: &Lua, opts: &Table) -> mlua::Result<FaceId> {
    match opts.get::<Value>("plane")? {
        Value::Integer(i) if i >= 0 => Ok(FaceId::ConstructionPlane(plane_key_from_ordinal(
            lua,
            i as usize,
        )?)),
        Value::Number(n) if n >= 0.0 => Ok(FaceId::ConstructionPlane(plane_key_from_ordinal(
            lua,
            n.round() as usize,
        )?)),
        Value::Table(t) => parse_face_id_table(lua, t),
        _ => Err(mlua::Error::external(
            "`plane` must be a construction-plane ordinal or a face spec table, \
             e.g. {kind=\"construction_plane\", index=0}",
        )),
    }
}

/// Parse a `bearcad.mirror_bodies`/`edit_mirror` table into `(plane_face, bodies)` (#523).
fn parse_mirror_op_args(
    lua: &Lua,
    opts: &Table,
) -> mlua::Result<(FaceId, Vec<usize>, crate::model::MirrorMode)> {
    let plane = parse_mirror_plane(lua, opts)?;
    let targets: Vec<usize> = opts.get::<Option<Vec<usize>>>("bodies")?.unwrap_or_default();
    // `output` mirrors the pane's Output row (#639); omitted means a new body each.
    let mode = match opts.get::<Option<String>>("output")?.as_deref() {
        None | Some("new") | Some("new_body") => crate::model::MirrorMode::NewBody,
        Some("join") | Some("add") | Some("combine") => crate::model::MirrorMode::Join,
        Some("cut") => crate::model::MirrorMode::Cut,
        Some(other) => {
            return Err(mlua::Error::external(format!(
                "unknown mirror output '{other}' (new|join|cut)"
            )))
        }
    };
    Ok((plane, targets, mode))
}

fn parse_geometric_constraint(name: &str) -> Option<GeometricConstraintType> {
    match name.to_ascii_lowercase().as_str() {
        "parallel" => Some(GeometricConstraintType::Parallel),
        "perpendicular" => Some(GeometricConstraintType::Perpendicular),
        "equal" => Some(GeometricConstraintType::Equal),
        "coincident" => Some(GeometricConstraintType::Coincident),
        "midpoint" => Some(GeometricConstraintType::Midpoint),
        // The axis-parallel buttons (#583); the legacy names map to them for script back-compat.
        "horizontal" | "along_x" | "parallel_x" => Some(GeometricConstraintType::AlongXAxis),
        "vertical" | "along_y" | "parallel_y" => Some(GeometricConstraintType::AlongYAxis),
        _ => None,
    }
}

fn parse_distance_target(lua: &Lua, table: Table) -> mlua::Result<DistanceTarget> {
    let kind: String = table.get("kind").or_else(|_| table.get("type"))?;
    match kind.to_ascii_lowercase().as_str() {
        "line" => Ok(DistanceTarget::LineLength(line_key_from_ordinal(
            lua,
            table.get("index")?,
        )?)),
        "circle" => Ok(DistanceTarget::CircleDiameter(circle_key_from_ordinal(
            lua,
            table.get("index")?,
        )?)),
        // Positioning dimensions (#809), the scripted twins of picking two things under the
        // Dimension tool. The side/direction each one is measured on is captured from the
        // current geometry by `constraints::finalize_distance_target`, exactly as it is for
        // an interactive pick, so a script only names the two things.
        "point_line" | "point_edge" => Ok(DistanceTarget::PointLineDistance {
            point: parse_constraint_point_table(lua, table.get("point")?)?,
            line: parse_constraint_line_table(lua, table.get("line")?)?,
            side: crate::model::default_constraint_sign(),
        }),
        "point_point" | "points" => Ok(DistanceTarget::PointPointDistance {
            anchor: parse_constraint_point_table(lua, table.get("anchor")?)?,
            mover: parse_constraint_point_table(lua, table.get("mover")?)?,
            dir_u: 0.0,
            dir_v: 0.0,
        }),
        "line_line" | "lines" => Ok(DistanceTarget::LineLineDistance {
            line_a: parse_constraint_line_table(lua, table.get("a")?)?,
            line_b: parse_constraint_line_table(lua, table.get("b")?)?,
            side: crate::model::default_constraint_sign(),
        }),
        other => Err(mlua::Error::external(format!(
            "unknown constraint target '{other}' (line, circle, point_line, point_point, \
             line_line)"
        ))),
    }
}

/// A world-space vector as a positional Lua triple `{x, y, z}` (for `bearcad.get`'s plane
/// origin/normal, `bearcad.body_stats`' bbox corners, and `bearcad.ui.camera{}`'s target).
fn vec3_lua(lua: &Lua, v: glam::Vec3) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set(1, v.x)?;
    t.set(2, v.y)?;
    t.set(3, v.z)?;
    Ok(t)
}

/// Short script name for the face a sketch is hosted on (`bearcad.get`, #107).
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

/// Short script name for a constraint's kind (`bearcad.get`, #107).
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

/// Sources `bearcad.project{ ... }` should project into the open sketch (#1351).
/// Empty means "the current scene selection" (including un-project).
fn parse_project_elements(lua: &Lua, opts: Option<Table>) -> mlua::Result<Vec<SceneElement>> {
    let Some(opts) = opts else {
        return Ok(Vec::new());
    };
    check_keys(
        &opts,
        "project",
        &[
            "entities",
            "body",
            "bodies",
            "plane",
            "planes",
            "kind",
            "index",
            "name",
            "type",
        ],
    )?;
    let mut elements = Vec::new();
    if let Some(ents) = opts.get::<Option<Table>>("entities")? {
        for i in 1..=ents.raw_len() {
            elements.push(resolve_element(lua, ents.get(i)?)?);
        }
    }
    if let Some(i) = opts.get::<Option<usize>>("body")? {
        elements.push(SceneElement::Body(body_key_from_ordinal(lua, i)?));
    }
    if let Some(bodies) = opts.get::<Option<Table>>("bodies")? {
        for i in 1..=bodies.raw_len() {
            let idx: usize = bodies.get(i)?;
            elements.push(SceneElement::Body(body_key_from_ordinal(lua, idx)?));
        }
    }
    if let Some(i) = opts.get::<Option<usize>>("plane")? {
        elements.push(SceneElement::ConstructionPlane(plane_key_from_ordinal(
            lua, i,
        )?));
    }
    if let Some(planes) = opts.get::<Option<Table>>("planes")? {
        for i in 1..=planes.raw_len() {
            let idx: usize = planes.get(i)?;
            elements.push(SceneElement::ConstructionPlane(plane_key_from_ordinal(
                lua, idx,
            )?));
        }
    }
    if elements.is_empty()
        && (opts.contains_key("kind")? || opts.contains_key("type")? || opts.contains_key("name")?)
    {
        elements.push(parse_element_table(lua, opts)?);
    }
    Ok(elements)
}

/// Reject unrecognized keys in an options table (#403): a typo like `gap` for `spacing`
/// used to be silently ignored and fail confusingly downstream ("Repeat doesn't
/// evaluate…"). The error names every accepted key.
fn check_keys(opts: &Table, call: &str, allowed: &[&str]) -> mlua::Result<()> {
    for pair in opts.clone().pairs::<Value, Value>() {
        let (key, _) = pair?;
        let Value::String(s) = key else { continue };
        let key = s.to_str()?.to_string();
        if !allowed.contains(&key.as_str()) {
            return Err(mlua::Error::external(format!(
                "{call}: unknown key `{key}` (accepted keys: {})",
                allowed.join(", ")
            )));
        }
    }
    Ok(())
}

/// A size argument that is either a plain number or a parameter-expression string
/// (#402) — what the GUI's dimension fields accept. Returns `(number, expression)`;
/// the expression, when present, is evaluated at execution against the document's
/// parameters (the number is a placeholder then).
fn scalar_arg(lua: &Lua, opts: &Table, key: &str) -> mlua::Result<Option<(f32, Option<String>)>> {
    use mlua::FromLua;
    match opts.get::<Option<Value>>(key)? {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some((0.0, Some(s.to_str()?.to_string())))),
        Some(v) => Ok(Some((f32::from_lua(v, lua)?, None))),
    }
}

/// One joint member (#894): a bare body index, an element table/name, or anything
/// `resolve_element` takes — as long as it lands on a body, component, or unit instance.
fn parse_joint_member(lua: &Lua, value: Value) -> mlua::Result<crate::model::JointRef> {
    match value {
        Value::Integer(i) => Ok(crate::model::JointRef::Body(body_key_from_ordinal(
            lua,
            i as usize,
        )?)),
        Value::Number(n) => Ok(crate::model::JointRef::Body(body_key_from_ordinal(
            lua,
            n as usize,
        )?)),
        other => match resolve_element(lua, other)? {
            SceneElement::Body(i) => Ok(crate::model::JointRef::Body(i)),
            SceneElement::Component(i) => Ok(crate::model::JointRef::Component(i)),
            SceneElement::UnitInstance(i) => Ok(crate::model::JointRef::UnitInstance(i)),
            element => Err(mlua::Error::external(format!(
                "a joint joins bodies, components, or unit instances, got {}",
                element_kind_name(element)
            ))),
        },
    }
}

/// A position expression: a number (mm or degrees, per the freedom) or an expression
/// string; missing reads as the empty expression (zero).
fn joint_position_arg(opts: &Table, key: &str) -> mlua::Result<String> {
    match opts.get::<Option<Value>>(key)? {
        None => Ok(String::new()),
        Some(Value::String(s)) => Ok(s.to_str()?.to_string()),
        Some(Value::Integer(i)) => Ok(i.to_string()),
        Some(Value::Number(n)) => Ok(n.to_string()),
        Some(other) => Err(mlua::Error::external(format!(
            "{key} takes a number or an expression string, got {other:?}"
        ))),
    }
}

type JointOpArgs = (
    Vec<crate::model::JointRef>,
    usize,
    crate::model::JointKind,
    crate::model::MoveOperation,
    crate::model::JointFrame,
    String,
    String,
    String,
    crate::model::JointLimits,
);

/// Shared parsing for `joint` / `edit_joint` / `begin_joint` (#894/#901): the members
/// (`a`/`b`, or `parts` for a rigid group), the kind (+ `lead` for a screw), which side is
/// the base, the mate that places them (#1020 — a `face` pair plus `line_up` rows), and the
/// position expressions.
fn parse_joint_op_args(lua: &Lua, opts: &Table, call: &str) -> mlua::Result<JointOpArgs> {
    check_keys(
        opts,
        call,
        &[
            "index", "a", "b", "parts", "kind", "lead", "base", "face", "line_up",
            "frame_origin", "frame_axis", "frame_axis2",
            "position", "position2", "position3", "slide_min",
            "slide_max", "slide_min_to", "slide_max_to", "turn_min", "turn_max", "name",
        ],
    )?;
    let mut members = Vec::new();
    if let Some(parts) = opts.get::<Option<Table>>("parts")? {
        for value in parts.sequence_values::<Value>() {
            members.push(parse_joint_member(lua, value?)?);
        }
    } else {
        if let Some(a) = opts.get::<Option<Value>>("a")? {
            members.push(parse_joint_member(lua, a)?);
        }
        if let Some(b) = opts.get::<Option<Value>>("b")? {
            members.push(parse_joint_member(lua, b)?);
        }
    }
    let mut kind = match opts.get::<Option<String>>("kind")? {
        None => crate::model::JointKind::Rigid,
        Some(name) => crate::model::JointKind::from_name(&name).ok_or_else(|| {
            mlua::Error::external(format!(
                "unknown joint kind '{name}' (rigid|slider|revolute|cylindrical|planar|ball|pin_slot|screw)"
            ))
        })?,
    };
    if let Some(lead) = opts.get::<Option<Value>>("lead")? {
        let lead = match lead {
            Value::String(s) => s.to_str()?.to_string(),
            Value::Integer(i) => i.to_string(),
            Value::Number(n) => n.to_string(),
            other => {
                return Err(mlua::Error::external(format!(
                    "lead takes a number or an expression string, got {other:?}"
                )))
            }
        };
        match &mut kind {
            crate::model::JointKind::Screw { lead: l } => *l = lead,
            _ => {
                return Err(mlua::Error::external(
                    "lead only applies to a screw joint",
                ))
            }
        }
    }
    let base = match opts.get::<Option<String>>("base")?.as_deref() {
        None | Some("a") => 0,
        Some("b") => 1,
        Some(other) => {
            return Err(mlua::Error::external(format!(
                "unknown base '{other}' (expected 'a' or 'b')"
            )))
        }
    };
    let mate = parse_mate(lua, opts)?;
    // Travel limits (#896): expressions on either end, or a stop picked as geometry.
    let limits = crate::model::JointLimits {
        slide_min: joint_position_arg(opts, "slide_min")?,
        slide_max: joint_position_arg(opts, "slide_max")?,
        slide_min_target: opts
            .get::<Option<Table>>("slide_min_to")?
            .map(|t| parse_extrude_target_table(lua, &t))
            .transpose()?,
        slide_max_target: opts
            .get::<Option<Table>>("slide_max_to")?
            .map(|t| parse_extrude_target_table(lua, &t))
            .transpose()?,
        turn_min: joint_position_arg(opts, "turn_min")?,
        turn_max: joint_position_arg(opts, "turn_max")?,
    };
    // How the joint works (#1079): its own frame, seeded by the mate when left out.
    let frame = crate::model::JointFrame {
        origin: parse_move_point(lua, opts.get("frame_origin")?, "frame_origin")?,
        primary: parse_mate_ref(lua, opts.get("frame_axis")?, "frame_axis")?,
        secondary: parse_mate_ref(lua, opts.get("frame_axis2")?, "frame_axis2")?,
    };
    Ok((
        members,
        base,
        kind,
        mate,
        frame,
        joint_position_arg(opts, "position")?,
        joint_position_arg(opts, "position2")?,
        joint_position_arg(opts, "position3")?,
        limits,
    ))
}

fn apply_optional_name(
    lua: &Lua,
    element: SceneElement,
    opts: Option<Table>,
) -> mlua::Result<()> {
    let Some(opts) = opts else { return Ok(()) };
    let Ok(name) = opts.get::<String>("name") else {
        return Ok(());
    };
    let tick = lua
        .app_data_ref::<ScriptTickData>()
        .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
    // The rename rides along on a creation call: keep the creation's status
    // ("Added extrusion (12.0 mm)") instead of clobbering it with "Renamed to …".
    let creation_status = unsafe { tick.state().status.clone() };
    unsafe { tick.exec(Instruction::SetElementName { element, name })? };
    unsafe { tick.state().status = creation_status };
    Ok(())
}

/// Register the global `bearcad` API table on a Lua state.
pub fn register_api(lua: &Lua) -> mlua::Result<()> {
    let api = lua.create_table()?;

    api.set(
        "new",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::New) }
        })?,
    )?;

    api.set(
        "open",
        lua.create_function(|lua, path: String| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Open(path)) }
        })?,
    )?;

    // Simulate a Finder / file-manager double-click (#1326): same queue as the OS
    // open-documents handler. Drained on the next script/GUI tick.
    api.set(
        "os_open",
        lua.create_function(|_lua, path: String| {
            crate::file_association::queue_open_path(path);
            Ok(())
        })?,
    )?;

    api.set(
        "save",
        lua.create_function(|lua, path: Option<String>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Save(path)) }
        })?,
    )?;

    // #1343: discard persisted tessellation (SPEC §4.4 force-rebuild).
    api.set(
        "rebuild_geometry",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::RebuildGeometry) }
        })?,
    )?;

    // #1343: `{ warmed, hits, misses }` for geometry_cache / BODY_MESH_CACHE probes.
    api.set(
        "mesh_cache",
        lua.create_function(|lua, ()| {
            let s = crate::extrude::mesh_cache_stats();
            let t = lua.create_table()?;
            t.set("warmed", s.warmed)?;
            t.set("hits", s.hits)?;
            t.set("misses", s.misses)?;
            Ok(t)
        })?,
    )?;

    // #1341: last incremental flush, so a script can assert only the changed
    // tables were written. `{ bodies = { inserts = 1, updates = 0, deletes = 0 }, ... }`.
    api.set(
        "session_writes",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let state = unsafe { tick.state() };
            let out = lua.create_table()?;
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(session) = &state.document_session {
                let session = session.borrow();
                for (name, t) in &session.last_write().tables {
                    let row = lua.create_table()?;
                    row.set("inserts", t.inserts)?;
                    row.set("updates", t.updates)?;
                    row.set("deletes", t.deletes)?;
                    out.set(name.as_str(), row)?;
                }
            }
            Ok(out)
        })?,
    )?;

    // #1341: read one scalar from the *committed* file via a new connection
    // (the last Save, not the open transaction).
    api.set(
        "sqlite_scalar",
        lua.create_function(|lua, sql: String| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let state = unsafe { tick.state() };
            #[cfg(not(target_arch = "wasm32"))]
            {
                let path = state
                    .path
                    .as_deref()
                    .ok_or_else(|| mlua::Error::external("sqlite_scalar: document has no path"))?;
                let conn = rusqlite::Connection::open(path)
                    .map_err(|e| mlua::Error::external(e.to_string()))?;
                let val: rusqlite::types::Value = conn
                    .query_row(&sql, [], |row| row.get(0))
                    .map_err(|e| mlua::Error::external(e.to_string()))?;
                return match val {
                    rusqlite::types::Value::Null => Ok(Value::Nil),
                    rusqlite::types::Value::Integer(i) => Ok(Value::Integer(i)),
                    rusqlite::types::Value::Real(f) => Ok(Value::Number(f)),
                    rusqlite::types::Value::Text(s) => Ok(Value::String(lua.create_string(s)?)),
                    rusqlite::types::Value::Blob(_) => Err(mlua::Error::external(
                        "sqlite_scalar: blob columns are not returned",
                    )),
                };
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = (lua, sql, state);
                Err(mlua::Error::external(
                    "sqlite_scalar is not available in the browser",
                ))
            }
        })?,
    )?;

    api.set(
        "export_stl",
        lua.create_function(|lua, (path, body): (String, Option<String>)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ExportStl { path, body }) }
        })?,
    )?;

    api.set(
        "export_3mf",
        lua.create_function(|lua, (path, body): (String, Option<String>)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Export3mf { path, body }) }
        })?,
    )?;

    api.set(
        "export_step",
        lua.create_function(|lua, (path, body): (String, Option<String>)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ExportStep { path, body }) }
        })?,
    )?;

    // #1223: Home zoom-to-fit PNG preview (same image saved into .bearcad for Finder).
    api.set(
        "export_preview",
        lua.create_function(|lua, path: String| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ExportPreview { path }) }
        })?,
    )?;

    api.set(
        "import_stl",
        lua.create_function(|lua, path: String| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ImportStl { path }) }
        })?,
    )?;

    // #721: import another BearCAD document as a unit. `import_unit("a.bearcad")` or
    // `import_unit{ path = "a.bearcad", link = "dynamic"|"static", name = "bracket" }`.
    api.set(
        "import_unit",
        lua.create_function(|lua, value: Value| {
            let (path, link, name) = match value {
                Value::String(s) => (s.to_str()?.to_string(), None, None),
                Value::Table(t) => {
                    check_keys(&t, "import_unit", &["path", "link", "name"])?;
                    let link = match t.get::<Option<String>>("link")?.as_deref() {
                        None => None,
                        Some("dynamic") => Some(crate::model::LinkMode::Dynamic),
                        Some("static") => Some(crate::model::LinkMode::Static),
                        Some(other) => {
                            return Err(mlua::Error::external(format!(
                                "import_unit link must be \"dynamic\" or \"static\", got {other:?}"
                            )))
                        }
                    };
                    (t.get::<String>("path")?, link, t.get::<Option<String>>("name")?)
                }
                _ => {
                    return Err(mlua::Error::external(
                        "import_unit takes a path string or { path =, link =, name = }",
                    ))
                }
            };
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ImportUnit { path, link, name }) }
        })?,
    )?;

    // #728: override one unit instance's parameter (omit `value` to clear back to the
    // unit's own). `bearcad.unit_override{ instance = 0, name = "width", value = "20" }`.
    // Registered under both names (#736 spells it `set_unit_parameter`; `unit_override`
    // is what session export writes).
    for hook in ["unit_override", "set_unit_parameter"] {
        api.set(
            hook,
            lua.create_function(|lua, opts: Table| {
                check_keys(&opts, "unit_override", &["instance", "name", "value", "expression"])?;
                let instance: usize = opts.get("instance")?;
                let name: String = opts.get("name")?;
                let expression: Option<String> = match opts.get::<Option<String>>("value")? {
                    Some(v) => Some(v),
                    None => opts.get("expression")?,
                };
                let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
                unsafe {
                    tick.exec(Instruction::SetUnitParameterOverride { instance, name, expression })
                }
            })?,
        )?;
    }

    // #734: switch a unit's link mode: `bearcad.unit_link(0, "static"|"dynamic")`.
    api.set(
        "unit_link",
        lua.create_function(|lua, (unit, mode): (usize, String)| {
            let link = match mode.as_str() {
                "static" => crate::model::LinkMode::Static,
                "dynamic" => crate::model::LinkMode::Dynamic,
                other => {
                    return Err(mlua::Error::external(format!(
                        "unit_link mode must be \"static\" or \"dynamic\", got {other:?}"
                    )))
                }
            };
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SetUnitLink { unit, link }) }
        })?,
    )?;

    // #736: another instance of an already-embedded unit.
    api.set(
        "add_unit_instance",
        lua.create_function(|lua, opts: Table| {
            check_keys(&opts, "add_unit_instance", &["unit", "name"])?;
            let unit: usize = opts.get("unit")?;
            let name: Option<String> = opts.get("name")?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::AddUnitInstance { unit, name }) }
        })?,
    )?;

    // #1404: clone an existing unit instance (same unit and parameter overrides).
    api.set(
        "clone_unit_instance",
        lua.create_function(|lua, opts: Table| {
            check_keys(&opts, "clone_unit_instance", &["instance"])?;
            let instance: usize = opts.get("instance")?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::CloneUnitInstance { instance }) }
        })?,
    )?;

    // #732: re-sync a unit's embedded copy from its source file. Takes the unit index,
    // or `{ unit = n }`.
    api.set(
        "sync_unit",
        lua.create_function(|lua, value: Value| {
            let unit = match value {
                Value::Integer(i) => i as usize,
                Value::Number(n) => n.round() as usize,
                Value::Table(t) => t.get::<usize>("unit")?,
                _ => {
                    return Err(mlua::Error::external(
                        "sync_unit takes a unit index or { unit = n }",
                    ))
                }
            };
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SyncUnit { unit }) }
        })?,
    )?;

    // #163/#169: import a PNG/JPEG as a tracing image. `import_image("p.png")` or
    // `import_image{ path = "p.png", plane = 0 }`.
    api.set(
        "import_image",
        lua.create_function(|lua, value: Value| {
            let (path, plane) = match value {
                Value::String(s) => (s.to_str()?.to_string(), None),
                Value::Table(t) => (t.get::<String>("path")?, t.get::<Option<usize>>("plane")?),
                _ => {
                    return Err(mlua::Error::external(
                        "import_image takes a path string or { path =, plane = }",
                    ))
                }
            };
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ImportImage { path, plane }) }
        })?,
    )?;

    // #171: calibrate a tracing image's scale from a plane-local reference segment.
    // Move / delete a calibration reference point (#424).
    api.set(
        "calibration_point",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(&opts, "calibration_point", &["image", "index", "x", "y"])?;
            let image: usize = opts.get("image")?;
            let index: usize = opts.get("index")?;
            let x: f32 = opts.get("x")?;
            let y: f32 = opts.get("y")?;
            unsafe { tick.exec(Instruction::SetCalibrationPoint { image, index, x, y }) }
        })?,
    )?;
    api.set(
        "remove_calibration_point",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(&opts, "remove_calibration_point", &["image", "index"])?;
            let image: usize = opts.get("image")?;
            let index: usize = opts.get("index")?;
            unsafe { tick.exec(Instruction::RemoveCalibrationPoint { image, index }) }
        })?,
    )?;

    api.set(
        "calibrate_image",
        lua.create_function(|lua, opts: Table| {
            let image: usize = opts.get("image")?;
            let parse_point = |t: Table| -> mlua::Result<(f32, f32)> {
                Ok((t.get(1)?, t.get(2)?))
            };
            let a = parse_point(opts.get::<Table>("from")?)?;
            let b = parse_point(opts.get::<Table>("to")?)?;
            let length: f32 = opts.get("length")?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::CalibrateImage { image, a, b, length }) }
        })?,
    )?;

    api.set(
        "import_step",
        lua.create_function(|lua, path: String| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ImportStep { path }) }
        })?,
    )?;

    // #1160: import a document Lua script (File → Export → Lua Script…).
    // `import_lua("part.lua")` or `import_lua{ path = "part.lua", force = true }`.
    // Refuses a non-blank document unless `force` is true.
    api.set(
        "import_lua",
        lua.create_function(|lua, value: Value| {
            let (path, force) = match value {
                Value::String(s) => (s.to_str()?.to_string(), false),
                Value::Table(t) => {
                    check_keys(&t, "import_lua", &["path", "force"])?;
                    let path: String = t.get("path")?;
                    let force: bool = t.get::<Option<bool>>("force")?.unwrap_or(false);
                    (path, force)
                }
                _ => {
                    return Err(mlua::Error::external(
                        "import_lua takes a path string or { path =, force = }",
                    ))
                }
            };
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ImportLua { path, force }) }
        })?,
    )?;

    api.set(
        "clear",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Clear) }
        })?,
    )?;

    api.set(
        "undo",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Undo) }
        })?,
    )?;

    // Copy / Paste (#1236): `bearcad.copy()` then `bearcad.paste{ x = 40 }` (or
    // `linked = true` for Paste Linked on bodies/components).
    api.set(
        "copy",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::CopySelection) }
        })?,
    )?;
    api.set(
        "paste",
        lua.create_function(|lua, opts: Option<Table>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let (linked, x, y, z) = if let Some(opts) = opts {
                let linked: bool = opts.get("linked").unwrap_or(false);
                let x: f32 = opts.get("x").unwrap_or(0.0);
                let y: f32 = opts.get("y").unwrap_or(0.0);
                let z: f32 = opts.get("z").unwrap_or(0.0);
                (linked, x, y, z)
            } else {
                (false, 0.0, 0.0, 0.0)
            };
            unsafe { tick.exec(Instruction::PasteAt { linked, x, y, z }) }
        })?,
    )?;

    api.set(
        "quit",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Quit) }
        })?,
    )?;

    // Gizmo introspection/control (#214): enumerate the viewport gizmos the current tool state
    // exposes, and drive their scalar the way a drag would — so gizmo tools are scriptable.
    api.set(
        "gizmos",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let state = unsafe { tick.state() };
            let arr = lua.create_table()?;
            for (i, g) in crate::actions::available_gizmos(state).into_iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("kind", g.kind)?;
                entry.set("name", g.name)?;
                entry.set("value", g.value)?;
                if let Some(p) = g.position {
                    let pos = lua.create_table()?;
                    pos.set("x", p.x)?;
                    pos.set("y", p.y)?;
                    pos.set("z", p.z)?;
                    entry.set("position", pos)?;
                }
                arr.set(i + 1, entry)?;
            }
            Ok(arr)
        })?,
    )?;
    api.set(
        "set_gizmo",
        lua.create_function(|lua, opts: Table| {
            let name: String = opts.get("name")?;
            let value: f32 = opts.get("value")?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SetGizmo { name, value, relative: false }) }
        })?,
    )?;
    api.set(
        "drag_gizmo",
        lua.create_function(|lua, opts: Table| {
            let name: String = opts.get("name")?;
            let by: f32 = opts.get("by")?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SetGizmo { name, value: by, relative: true }) }
        })?,
    )?;

    api.set(
        "tool",
        lua.create_function(|lua, name: String| {
            let tool = Tool::from_name(&name)
                .ok_or_else(|| mlua::Error::external(format!("unknown tool '{name}'")))?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Tool(tool)) }
        })?,
    )?;

    api.set(
        "begin_sketch",
        lua.create_function(|lua, args: MultiValue| {
            let args = args.into_vec();
            let face = if let Some(Value::Table(table)) = args.first() {
                parse_face_id_table(lua, table.clone())?
            } else {
                let kind = match args.first() {
                    Some(Value::String(s)) => s.to_str()?.to_string(),
                    _ => return Err(mlua::Error::external("begin_sketch requires face kind")),
                };
                let index = match args.get(1) {
                    Some(Value::Integer(i)) => *i as usize,
                    Some(Value::Number(n)) => n.round() as usize,
                    _ => return Err(mlua::Error::external("begin_sketch requires face index")),
                };
                let tick = lua
                    .app_data_ref::<ScriptTickData>()
                    .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
                let face = unsafe { FaceId::from_script(&tick.state().doc, &kind, index) };
                face.ok_or_else(|| {
                    mlua::Error::external(format!("unknown sketch face kind '{kind}'"))
                })?
            };
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::BeginSketch { face }) }
        })?,
    )?;

    api.set(
        "open_sketch",
        lua.create_function(|lua, sketch: usize| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::OpenSketch { sketch }) }
        })?,
    )?;

    api.set(
        "exit_sketch",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ExitSketch) }
        })?,
    )?;

    api.set(
        "element",
        lua.create_function(|lua, (kind, index): (String, usize)| {
            let element = {
                let tick = lua
                    .app_data_ref::<ScriptTickData>()
                    .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
                scene_element_from_kind(unsafe { &tick.state().doc }, &kind, index).ok_or_else(
                    || mlua::Error::external(format!("unknown element kind '{kind}'")),
                )?
            };
            make_element(lua, element)
        })?,
    )?;

    api.set(
        "find",
        lua.create_function(|lua, name: String| {
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let element = unsafe { find_element_by_name(&tick.state().doc, &name) };
            match element {
                Some(element) => Ok(Some(make_element(lua, element)?)),
                None => Ok(None),
            }
        })?,
    )?;

    api.set(
        "set_name",
        lua.create_function(|lua, (element, name): (Value, String)| {
            let element = resolve_element(lua, element)?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SetElementName { element, name }) }
        })?,
    )?;

    api.set(
        "focus_name",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::FocusElementName) }
        })?,
    )?;

    // #52: `bearcad.set_units{ length = "mm", angle = "deg" }` sets the document default
    // (unset fields keep their current document value). `bearcad.set_units{ sketch = N,
    // length = "in" }` sets a per-sketch override; a field left unset for a sketch call
    // means "follow the document default" (there's no way to distinguish an omitted Lua
    // table field from an explicit `nil`, so omission is treated as the inherit request).
    // NOTE: per #52's scope, this only stores/displays the choice — it doesn't (yet) drive
    // bare-number parsing defaults or dimension-label formatting.
    api.set(
        "set_units",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let length_name: Option<String> = opts.get("length")?;
            let length = length_name
                .map(|name| {
                    LengthUnit::from_name(&name)
                        .ok_or_else(|| mlua::Error::external(format!("unknown length unit '{name}'")))
                })
                .transpose()?;
            let angle_name: Option<String> = opts.get("angle")?;
            let angle = angle_name
                .map(|name| {
                    AngleUnit::from_name(&name)
                        .ok_or_else(|| mlua::Error::external(format!("unknown angle unit '{name}'")))
                })
                .transpose()?;
            if let Some(component) = opts.get::<Option<usize>>("component")? {
                unsafe {
                    tick.exec(Instruction::SetComponentUnits { component, length, angle })
                }
            } else if let Some(sketch) = opts.get::<Option<usize>>("sketch")? {
                unsafe { tick.exec(Instruction::SetSketchUnits { sketch, length, angle }) }
            } else {
                let doc = unsafe { &tick.state().doc };
                let length = length.unwrap_or(doc.default_length_unit);
                let angle = angle.unwrap_or(doc.default_angle_unit);
                unsafe { tick.exec(Instruction::SetDocumentUnits { length, angle }) }
            }
        })?,
    )?;

    // Components (#423): `bearcad.component{ name = "Frame", parent = 0 }` creates one and
    // returns its index; `bearcad.move_to_component{ kind = "body", index = 0,
    // component = 1 }` files an element into it (`component = false` moves it back out).
    // Derived (measured) parameters (#432/#647): `bearcad.derive_parameter{ kind =
    // "line_length"|"point_distance"|"line_distance"|"line_angle"|"body_edge_length"|
    // "body_vertex_distance", a =, b =, body =, body_b =, name = }`.
    // Point kinds take constraint-point tables for a/b; line kinds take line indices; the body
    // kinds take world points in **mm** on the body's mesh (quantized to the 0.01 mm selection
    // grid, so they need only land on the picked geometry, not match bit for bit).
    api.set(
        "derive_parameter",
        lua.create_function(|lua, opts: Table| {
            use crate::model::ParameterSource as PS;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(
                &opts,
                "derive_parameter",
                &["kind", "a", "b", "body", "body_b", "name", "instance", "face", "edge"],
            )?;
            let kind: String = opts.get("kind")?;
            let mm_point = |key: &str| -> mlua::Result<[i32; 3]> {
                let v: Vec<f32> = opts.get(key)?;
                if v.len() != 3 {
                    return Err(mlua::Error::external(format!(
                        "derive_parameter `{key}` must be {{x, y, z}} in mm"
                    )));
                }
                Ok(crate::hierarchy::quantize_body_point(glam::Vec3::new(
                    v[0], v[1], v[2],
                )))
            };
            let source = match kind.as_str() {
                "line_length" => PS::LineLength(line_key_from_ordinal(lua, opts.get("a")?)?),
                "point_distance" => PS::PointDistance(
                    parse_constraint_point_table(lua, opts.get("a")?)?,
                    parse_constraint_point_table(lua, opts.get("b")?)?,
                ),
                "line_distance" => PS::LineDistance(
                    line_key_from_ordinal(lua, opts.get("a")?)?,
                    line_key_from_ordinal(lua, opts.get("b")?)?,
                ),
                "line_angle" => PS::LineAngle(
                    line_key_from_ordinal(lua, opts.get("a")?)?,
                    line_key_from_ordinal(lua, opts.get("b")?)?,
                ),
                "body_edge_length" => PS::BodyEdgeLength {
                    body: body_key_from_ordinal(lua, opts.get("body")?)?,
                    a: mm_point("a")?,
                    b: mm_point("b")?,
                },
                "body_vertex_distance" => {
                    let ordinal_a: usize = opts.get("body")?;
                    let ordinal_b = opts.get::<Option<usize>>("body_b")?.unwrap_or(ordinal_a);
                    PS::BodyVertexDistance {
                        body_a: body_key_from_ordinal(lua, ordinal_a)?,
                        a: mm_point("a")?,
                        body_b: body_key_from_ordinal(lua, ordinal_b)?,
                        b: mm_point("b")?,
                    }
                }
                // Analytic unit edge (#724): `face` is the FaceId's JSON encoding (the
                // same spelling session export writes).
                "unit_edge_length" => {
                    let face_json: String = opts.get("face")?;
                    let face = serde_json::from_str(&face_json).map_err(|e| {
                        mlua::Error::external(format!("bad unit edge face: {e}"))
                    })?;
                    PS::UnitEdgeLength {
                        instance: unit_instance_key_from_ordinal(lua, opts.get("instance")?)?,
                        face,
                        edge: opts.get("edge")?,
                    }
                }
                other => {
                    return Err(mlua::Error::external(format!(
                        "unknown derive kind '{other}'"
                    )))
                }
            };
            let name: Option<String> = opts.get("name")?;
            unsafe { tick.exec(Instruction::CreateDerivedParameter { source, name }) }
        })?,
    )?;

    api.set(
        "component",
        lua.create_function(|lua, opts: Option<Table>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let (name, parent) = match &opts {
                Some(t) => {
                    check_keys(t, "component", &["name", "parent"])?;
                    (t.get::<Option<String>>("name")?, t.get::<Option<usize>>("parent")?)
                }
                None => (None, None),
            };
            unsafe { tick.exec(Instruction::CreateComponent { name, parent }) }?;
            Ok(unsafe { tick.state().doc.components.len().saturating_sub(1) })
        })?,
    )?;

    api.set(
        "move_to_component",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(&opts, "move_to_component", &["kind", "index", "component"])?;
            let kind: String = opts.get("kind")?;
            let index: usize = opts.get("index")?;
            let element = scene_element_from_kind(unsafe { &tick.state().doc }, &kind, index)
                .ok_or_else(|| {
                    mlua::Error::external(format!("unknown element kind '{kind}'"))
                })?;
            let component = match opts.get::<Value>("component")? {
                Value::Boolean(false) | Value::Nil => None,
                Value::Integer(i) => Some(i as usize),
                Value::Number(n) => Some(n as usize),
                other => {
                    return Err(mlua::Error::external(format!(
                        "component must be an index or false, got {other:?}"
                    )))
                }
            };
            unsafe { tick.exec(Instruction::MoveToComponent { element, component }) }
        })?,
    )?;

    api.set(
        "select",
        lua.create_function(|lua, args: MultiValue| {
            let mut args = args.into_vec();
            let additive = matches!(args.last(), Some(Value::Boolean(true)))
                || matches!(
                    args.last(),
                    Some(Value::Table(t)) if t.get::<bool>("additive").unwrap_or(false)
                );
            let element_value = args.remove(0);
            let element = resolve_element(lua, element_value)?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe {
                tick.exec(Instruction::SelectSceneElement { element, additive },
                )
            }
        })?,
    )?;

    api.set(
        "clear_selection",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ClearSceneSelection) }
        })?,
    )?;

    api.set(
        "set_visible",
        lua.create_function(|lua, (element, visible): (Value, Value)| {
            let element = resolve_element(lua, element)?;
            let visible = parse_visibility(visible)?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe {
                tick.exec(Instruction::SetElementVisible { element, visible },
                )
            }
        })?,
    )?;

    api.set(
        "set_construction",
        lua.create_function(|lua, (element, construction): (Value, Value)| {
            let element = resolve_element(lua, element)?;
            let construction = parse_bool(construction, "construction")?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe {
                tick.exec(Instruction::SetShapeConstruction {
                        element,
                        construction,
                    },
                )
            }
        })?,
    )?;

    api.set(
        "apply_construction",
        lua.create_function(|lua, construction: Value| {
            let construction = parse_bool(construction, "construction")?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ApplyConstruction { construction }) }
        })?,
    )?;

    api.set(
        "toggle_construction",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ToggleConstruction) }
        })?,
    )?;

    api.set(
        "apply_visibility",
        lua.create_function(|lua, visible: Value| {
            let visible = parse_bool(visible, "visible")?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ApplySelectionVisibility { visible }) }
        })?,
    )?;

    api.set(
        "toggle_visibility",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ToggleSelectionVisibility) }
        })?,
    )?;

    api.set(
        "set_dim",
        lua.create_function(|lua, (axis, value): (String, String)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            if let Some(axis) = RectAxis::from_name(&axis) {
                return unsafe { tick.exec(Instruction::SetDim { axis, value }) };
            }
            if axis.eq_ignore_ascii_case("length") || axis.eq_ignore_ascii_case("len") {
                return unsafe { tick.exec(Instruction::SetLineLength { value }) };
            }
            if axis.eq_ignore_ascii_case("diameter") || axis.eq_ignore_ascii_case("diam") {
                return unsafe { tick.exec(Instruction::SetCircleDiameter { value }) };
            }
            if axis.eq_ignore_ascii_case("offset") {
                return unsafe { tick.exec(Instruction::SetPlaneOffset { value }) };
            }
            if axis.eq_ignore_ascii_case("angle") {
                return unsafe { tick.exec(Instruction::SetPlaneAngle { value }) };
            }
            Err(mlua::Error::external(format!("unknown dimension '{axis}'")))
        })?,
    )?;

    api.set(
        "focus_dim",
        lua.create_function(|lua, axis: String| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            if let Some(axis) = RectAxis::from_name(&axis) {
                return unsafe { tick.exec(Instruction::FocusDim(axis)) };
            }
            if axis.eq_ignore_ascii_case("length") {
                return unsafe { tick.exec(Instruction::FocusLineLength) };
            }
            if axis.eq_ignore_ascii_case("diameter") {
                return unsafe { tick.exec(Instruction::FocusCircleDiameter) };
            }
            if let Some(dim) = PlaneDim::from_name(&axis) {
                return unsafe { tick.exec(Instruction::FocusPlaneDim(dim)) };
            }
            Err(mlua::Error::external(format!("unknown dimension '{axis}'")))
        })?,
    )?;

    api.set(
        "edit_dim",
        lua.create_function(|lua, axis: String| {
            let axis = DimLabelAxis::from_name(&axis)
                .ok_or_else(|| mlua::Error::external(format!("unknown dimension '{axis}'")))?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::BeginEditCommittedDim { axis }) }
        })?,
    )?;

    api.set(
        "commit_dim",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::CommitCommittedDim) }
        })?,
    )?;

    api.set(
        "set_dim_label_offset",
        lua.create_function(|lua, (axis, offset): (String, f32)| {
            let axis = DimLabelAxis::from_name(&axis)
                .ok_or_else(|| mlua::Error::external(format!("unknown dimension '{axis}'")))?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SetDimLabelOffset { axis, offset }) }
        })?,
    )?;

    api.set(
        "sketch_conflicts",
        lua.create_function(|lua, sketch: Option<usize>| {
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let state = unsafe { tick.state() };
            let sketch = match sketch {
                Some(ordinal) => state
                    .doc
                    .sketches
                    .keys()
                    .nth(ordinal)
                    .ok_or_else(|| mlua::Error::external(format!("no sketch {ordinal}")))?,
                None => state
                    .sketch_session
                    .map(|session| session.sketch)
                    .ok_or_else(|| mlua::Error::external("no active sketch"))?,
            };
            let conflicts =
                crate::constraints::sketch_conflicting_constraints(&state.doc, sketch)
                    .map_err(mlua::Error::external)?;
            let table = lua.create_table()?;
            // A script sees a constraint's ordinal among the live ones (#1055).
            for (i, index) in conflicts.iter().enumerate() {
                let ordinal = state.doc.constraints.keys().position(|k| k == *index);
                table.set(i + 1, ordinal)?;
            }
            Ok(table)
        })?,
    )?;

    api.set(
        "sketch_dof",
        lua.create_function(|lua, sketch: Option<usize>| {
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let state = unsafe { tick.state() };
            let sketch = match sketch {
                Some(ordinal) => state
                    .doc
                    .sketches
                    .keys()
                    .nth(ordinal)
                    .ok_or_else(|| mlua::Error::external(format!("no sketch {ordinal}")))?,
                None => state
                    .sketch_session
                    .map(|session| session.sketch)
                    .ok_or_else(|| mlua::Error::external("no active sketch"))?,
            };
            crate::constraints::sketch_degrees_of_freedom(&state.doc, sketch)
                .map_err(mlua::Error::external)
        })?,
    )?;

    // ----- Read-back / introspection getters (#107). Pure reads of the live state, like
    // `sketch_dof` above — not `Instruction`s, so they never appear in recorded scripts. -----

    api.set(
        "count",
        lua.create_function(|lua, kind: String| {
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let doc = unsafe { &tick.state().doc };
            let count = match kind.to_ascii_lowercase().as_str() {
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
                "component" => doc.components.len(),
                "image" => doc.tracing_images.len(),
                "joint" => doc.joints.len(),
                other => {
                    return Err(mlua::Error::external(format!(
                        "unknown count kind '{other}' (valid kinds: line, circle, sketch, \
                         constraint, construction_plane, extrusion, body, drawing, parameter, \
                         sketch_text, image, joint)"
                    )))
                }
            };
            Ok(count)
        })?,
    )?;

    api.set(
        "get",
        lua.create_function(|lua, opts: Table| {
            let kind: String = opts.get("kind")?;
            let index: usize = opts.get("index")?;
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let doc = unsafe { &tick.state().doc };
            let t = lua.create_table()?;
            match kind.to_ascii_lowercase().as_str() {
                "line" => {
                    // The script's `index` is the line's ordinal (#1055).
                    let Some(line) = doc.lines.keys().nth(index).map(|k| &doc.lines[k]) else {
                        return Ok(Value::Nil);
                    };
                    t.set("x0", line.x0)?;
                    t.set("y0", line.y0)?;
                    t.set("x1", line.x1)?;
                    t.set("y1", line.y1)?;
                    t.set("construction", line.construction)?;
                    t.set("curved", line.is_curved())?;
                    if let Some([c0, c1]) = line.bezier {
                        let handles = lua.create_table()?;
                        for (i, (hx, hy)) in [c0, c1].into_iter().enumerate() {
                            let h = lua.create_table()?;
                            h.set(1, hx)?;
                            h.set(2, hy)?;
                            handles.set(i + 1, h)?;
                        }
                        t.set("bezier", handles)?;
                    }
                    t.set("length", line.length())?;
                    t.set("projected", line.projection.is_some())?;
                    if let Some(name) = &line.name {
                        t.set("name", name.as_str())?;
                    }
                    t.set("sketch", doc.sketches.keys().position(|k| k == line.sketch))?;
                }
                "circle" => {
                    // The script's `index` is the circle's ordinal (#1055).
                    let Some(circle) = doc.circles.keys().nth(index).map(|k| &doc.circles[k])
                    else {
                        return Ok(Value::Nil);
                    };
                    t.set("x", circle.cx)?;
                    t.set("y", circle.cy)?;
                    t.set("r", circle.r)?;
                    t.set("diameter", circle.diameter())?;
                    t.set("construction", circle.construction)?;
                    if let Some(name) = &circle.name {
                        t.set("name", name.as_str())?;
                    }
                    t.set("sketch", doc.sketches.keys().position(|k| k == circle.sketch))?;
                }
                "sketch" => {
                    // The script's `index` is the sketch's ordinal (#1055).
                    let Some(sketch) = doc.sketches.keys().nth(index).map(|k| &doc.sketches[k])
                    else {
                        return Ok(Value::Nil);
                    };
                    t.set("face", face_kind_name(&sketch.face))?;
                    if let Some(name) = &sketch.name {
                        t.set("name", name.as_str())?;
                    }
                }
                "constraint" => {
                    // The script's `index` is the constraint's ordinal (#1055).
                    let Some(constraint) =
                        doc.constraints.keys().nth(index).map(|k| &doc.constraints[k])
                    else {
                        return Ok(Value::Nil);
                    };
                    t.set("kind", constraint_kind_name(&constraint.kind))?;
                    t.set("expression", constraint.expression.as_str())?;
                    if let Some(name) = &constraint.name {
                        t.set("name", name.as_str())?;
                    }
                    t.set(
                        "sketch",
                        doc.sketches.keys().position(|k| k == constraint.sketch),
                    )?;
                }
                "construction_plane" | "plane" => {
                    // The script's `index` is the plane's ordinal (#1055).
                    let Some(plane) = doc
                        .construction_planes
                        .keys()
                        .nth(index)
                        .map(|k| &doc.construction_planes[k])
                    else {
                        return Ok(Value::Nil);
                    };
                    t.set("origin", vec3_lua(lua, plane.origin)?)?;
                    t.set("normal", vec3_lua(lua, plane.normal)?)?;
                    // The drawn rectangle's size in the plane's own u/v axes (#833).
                    let extent = lua.create_table()?;
                    extent.set("u_min", plane.extent.u_min)?;
                    extent.set("u_max", plane.extent.u_max)?;
                    extent.set("v_min", plane.extent.v_min)?;
                    extent.set("v_max", plane.extent.v_max)?;
                    t.set("extent", extent)?;
                    if let Some(name) = &plane.name {
                        t.set("name", name.as_str())?;
                    }
                }
                "extrusion" => {
                    // The script's `index` is the extrusion's ordinal among the live
                    // ones (#1055).
                    let Some(extrusion) =
                        doc.extrusions.keys().nth(index).map(|k| &doc.extrusions[k])
                    else {
                        return Ok(Value::Nil);
                    };
                    t.set("distance", extrusion.distance)?;
                    t.set(
                        "sketch",
                        doc.sketches.keys().position(|k| k == extrusion.sketch),
                    )?;
                    t.set("faces", extrusion.faces.len())?;
                    if let Some(name) = &extrusion.name {
                        t.set("name", name.as_str())?;
                    }
                }
                "body" => {
                    // The script's `index` is the body's ordinal among the live ones (#1055).
                    let Some(body) = doc.bodies.keys().nth(index).map(|k| &doc.bodies[k]) else {
                        return Ok(Value::Nil);
                    };
                    if let Some(name) = &body.name {
                        t.set("name", name.as_str())?;
                    }
                    let add = lua.create_table()?;
                    for (i, ei) in body.source.extrusion_indices().iter().enumerate() {
                        add.set(i + 1, doc.extrusions.keys().position(|k| k == *ei))?;
                    }
                    t.set("add", add)?;
                    let cut = lua.create_table()?;
                    for (i, ei) in body.source.cut_extrusion_indices().iter().enumerate() {
                        cut.set(i + 1, doc.extrusions.keys().position(|k| k == *ei))?;
                    }
                    t.set("cut", cut)?;
                }
                "parameter" => {
                    // The script's `index` is the parameter's ordinal (#1055).
                    let Some(param) = doc.parameters.keys().nth(index).map(|k| &doc.parameters[k])
                    else {
                        return Ok(Value::Nil);
                    };
                    t.set("name", param.name.as_str())?;
                    t.set("expression", param.expression.as_str())?;
                }
                other => {
                    return Err(mlua::Error::external(format!(
                        "unknown get kind '{other}' (valid kinds: line, circle, sketch, \
                         constraint, construction_plane, extrusion, body, parameter)"
                    )))
                }
            }
            Ok(Value::Table(t))
        })?,
    )?;

    api.set(
        "body_stats",
        lua.create_function(|lua, index: usize| {
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let doc = unsafe { &tick.state().doc };
            let Some(index) = doc.bodies.keys().nth(index) else {
                return Ok(Value::Nil);
            };
            let Some(mesh) = crate::extrude::body_solid_mesh(doc, index) else {
                return Ok(Value::Nil);
            };
            let Some((min, max)) = mesh.bounds() else {
                return Ok(Value::Nil);
            };
            let t = lua.create_table()?;
            t.set("volume", crate::extrude::mesh_signed_volume(&mesh).abs())?;
            t.set("triangles", mesh.triangles.len())?;
            let bbox = lua.create_table()?;
            bbox.set("min", vec3_lua(lua, min)?)?;
            bbox.set("max", vec3_lua(lua, max)?)?;
            t.set("bbox", bbox)?;
            Ok(Value::Table(t))
        })?,
    )?;

    // A body's flat faces (#1020): `{ center = {x,y,z}, normal = {x,y,z} }` per face, in the
    // un-posed body's own coordinates — exactly what a mate's `face = {…}` argument takes.
    // Without this a script would have to guess a face's quantized key to name it at all.
    api.set(
        "body_faces",
        lua.create_function(|lua, index: usize| {
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let doc = unsafe { &tick.state().doc };
            let out = lua.create_table()?;
            let Some(key) = doc.bodies.keys().nth(index) else {
                return Ok(Value::Table(out));
            };
            let Some(mesh) = crate::extrude::body_solid_mesh_unposed(doc, key) else {
                return Ok(Value::Table(out));
            };
            // Flat faces only (#1013): a round wall is a cylinder, reported by
            // `body_cylinders`, and would give a mate a nonsense plane to land on.
            for (i, tris) in crate::gpu_viewport::solid_mesh_coplanar_faces(&mesh)
                .iter()
                .filter(|tris| crate::extrude::fit_cylinder(tris).is_none())
                .enumerate()
            {
                let face = lua.create_table()?;
                face.set("body", index)?;
                face.set(
                    "face",
                    vec3_lua(lua, crate::extrude::face_group_center(tris))?,
                )?;
                face.set(
                    "normal",
                    vec3_lua(
                        lua,
                        (tris[0][1] - tris[0][0])
                            .cross(tris[0][2] - tris[0][0])
                            .normalize_or_zero(),
                    )?,
                )?;
                out.set(i + 1, face)?;
            }
            Ok(Value::Table(out))
        })?,
    )?;

    // A body's cylindrical surfaces (#1013): each with its centre line, in the un-posed
    // body's own coordinates. `axis` is what a mate's `line_up` row takes to put a hole on a
    // shaft; `cylinder` names the round wall itself.
    api.set(
        "body_cylinders",
        lua.create_function(|lua, index: usize| {
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let doc = unsafe { &tick.state().doc };
            let out = lua.create_table()?;
            let Some(key) = doc.bodies.keys().nth(index) else {
                return Ok(Value::Table(out));
            };
            for (i, cyl) in crate::extrude::body_cylinders(doc, key).iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("body", index)?;
                entry.set("cylinder", vec3_lua(lua, cyl.origin)?)?;
                entry.set("direction", vec3_lua(lua, cyl.dir)?)?;
                entry.set("radius", cyl.radius)?;
                entry.set("length", cyl.half_length * 2.0)?;
                let axis = lua.create_table()?;
                axis.set("body", index)?;
                axis.set("hole_axis", vec3_lua(lua, cyl.origin)?)?;
                axis.set("direction", vec3_lua(lua, cyl.dir)?)?;
                entry.set("axis", axis)?;
                out.set(i + 1, entry)?;
            }
            Ok(Value::Table(out))
        })?,
    )?;

    // A body's feature edges (#1020): `{ edge = { {x,y,z}, {x,y,z} } }` per edge, in the
    // un-posed body's own coordinates — what a mate's `line_up` row takes.
    api.set(
        "body_edges",
        lua.create_function(|lua, index: usize| {
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let doc = unsafe { &tick.state().doc };
            let out = lua.create_table()?;
            let Some(key) = doc.bodies.keys().nth(index) else {
                return Ok(Value::Table(out));
            };
            let Some(mesh) = crate::extrude::body_solid_mesh_unposed(doc, key) else {
                return Ok(Value::Table(out));
            };
            for (i, chain) in crate::gpu_viewport::solid_mesh_edge_chains(&mesh)
                .iter()
                .enumerate()
            {
                let (a, b) = crate::gpu_viewport::chain_canonical_segment(chain);
                let edge = lua.create_table()?;
                edge.set("body", index)?;
                let ends = lua.create_table()?;
                ends.set(1, vec3_lua(lua, a)?)?;
                ends.set(2, vec3_lua(lua, b)?)?;
                edge.set("edge", ends)?;
                out.set(i + 1, edge)?;
            }
            Ok(Value::Table(out))
        })?,
    )?;

    api.set(
        "status",
        lua.create_function(|lua, ()| {
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            Ok(unsafe { tick.state().status.clone() })
        })?,
    )?;

    api.set(
        "selection",
        lua.create_function(|lua, ()| {
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let state = unsafe { tick.state() };
            let out = lua.create_table()?;
            for (i, element) in state.scene_selection.iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("kind", element_kind_name(element.clone()))?;
                // Point/FaceEdge selections have no flat (kind, index) mapping (they name a
                // vertex/edge of another element); report just their kind and leave `index` nil.
                if !matches!(
                    element,
                    SceneElement::Point(_)
                        | SceneElement::FaceEdge(_)
                        | SceneElement::BodyFace { .. }
        | SceneElement::BodyCylinder { .. }
        | SceneElement::BodyAxis { .. }
                ) {
                    entry.set("index", element_index(&state.doc, element))?;
                }
                out.set(i + 1, entry)?;
            }
            Ok(out)
        })?,
    )?;

    // The active tool's element pickers (#968): what each one is called, whether it has focus,
    // what kinds and how many it accepts, and what it currently holds. Without this a script
    // can't tell an accepted pick from a rejected one — a body-set tool consumes the click
    // either way, so `selection()` looks identical.
    // What the viewport is hover-highlighting (#968) — the pick a click would take, as
    // `{ kind, index }`, or nil when nothing is. Lets a script assert that the right thing
    // lights up, which is otherwise unobservable from outside the renderer.
    api.set(
        "hovered",
        lua.create_function(|lua, ()| {
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let state = unsafe { tick.state() };
            let Some(element) = state.hover_element.clone() else {
                return Ok(Value::Nil);
            };
            let entry = lua.create_table()?;
            entry.set("kind", element_kind_name(element.clone()))?;
            let doc = unsafe { &tick.state().doc };
            entry.set("index", element_index(doc, element.clone()))?;
            if let Some(label) = face_element_label(doc, &element) {
                entry.set("label", label)?;
            }
            Ok(Value::Table(entry))
        })?,
    )?;

    // The Selection Exploder's fan (#968): one `{ kind, index }` per leaf, or an empty table
    // when it's closed. The crowd should offer exactly what the focused picker can take.
    api.set(
        "exploder",
        lua.create_function(|lua, ()| {
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let state = unsafe { tick.state() };
            let out = lua.create_table()?;
            for (i, element) in state.exploder_leaves.iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("kind", element_kind_name(element.clone()))?;
                entry.set("index", element_index(&state.doc, element.clone()))?;
                // Where this leaf's loupe sits, in the viewport-local pixels `bearcad.ui.click`
                // takes (#986) — absent for a leaf the current level shows inside a group.
                if let Some(Some((x, y))) = state.exploder_loupe_positions.get(i) {
                    entry.set("x", *x)?;
                    entry.set("y", *y)?;
                }
                // Which face each leaf is (#988) — a fan over a solid offers several, and
                // `kind`/`index` are identical for all of them.
                if let Some(label) = face_element_label(&state.doc, element) {
                    entry.set("label", label)?;
                }
                out.set(i + 1, entry)?;
            }
            Ok(out)
        })?,
    )?;

    // Arm a picker by name (#963/#968), the scripted equivalent of clicking it in the pane.
    api.set(
        "picker_focus",
        lua.create_function(|lua, name: String| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::FocusPicker { name }) }
        })?,
    )?;

    api.set(
        "pickers",
        lua.create_function(|lua, ()| {
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let state = unsafe { tick.state() };
            let out = lua.create_table()?;
            for (i, view) in state.tool_pickers.iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("name", view.heading)?;
                entry.set("focused", view.picker.is_focused())?;
                match view.picker.limit() {
                    crate::element_picker::PickLimit::Finite(n) => entry.set("limit", n)?,
                    crate::element_picker::PickLimit::Infinite => {}
                }
                let kinds = lua.create_table()?;
                for (k, kind) in view.picker.filter().accepted_kinds().iter().enumerate() {
                    kinds.set(k + 1, kind.label())?;
                }
                entry.set("accepts", kinds)?;
                let items = lua.create_table()?;
                for (n, element) in view.picker.picked().iter().enumerate() {
                    let item = lua.create_table()?;
                    item.set("kind", element_kind_name(element.clone()))?;
                    item.set("index", element_index(&state.doc, element.clone()))?;
                    items.set(n + 1, item)?;
                }
                entry.set("items", items)?;
                out.set(i + 1, entry)?;
            }
            Ok(out)
        })?,
    )?;

    // Materials (#834): `bearcad.material{ name = "Steel", color = "#b0b6be", bodies = {0} }`
    // adds one and hands it to the listed bodies; `bearcad.set_material{ body = 0, material =
    // 0 }` (or `material = nil`) assigns/clears one.
    api.set(
        "material",
        lua.create_function(|lua, opts: Table| {
            let name: Option<String> = opts.get("name")?;
            let color = match opts.get::<Option<String>>("color")? {
                Some(text) => Some(parse_hex_color(&text)?),
                None => None,
            };
            let bodies: Vec<usize> = opts.get::<Option<Vec<usize>>>("bodies")?.unwrap_or_default();
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::AddMaterial { name, color, bodies }) }
        })?,
    )?;

    api.set(
        "set_material",
        lua.create_function(|lua, opts: Table| {
            let body: usize = opts.get("body")?;
            let material: Option<usize> = opts.get("material")?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SetBodyMaterial { body, material }) }
        })?,
    )?;

    // Shadow bodies (#1218): `bearcad.set_body_shadow{ body = 0, shadow = true }` hides a
    // body from the viewport (except hover/select) and from whole-document export —
    // the same flag operations set when they consume an input. `shadow = false` restores it.
    api.set(
        "set_body_shadow",
        lua.create_function(|lua, opts: Table| {
            let body: usize = opts.get("body")?;
            let shadow: bool = opts.get("shadow")?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SetBodyShadow { body, shadow }) }
        })?,
    )?;

    api.set(
        "add_constraint",
        lua.create_function(|lua, (target, expression): (Table, String)| {
            let target = parse_distance_target(lua, target)?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe {
                tick.exec(Instruction::AddDistanceConstraint { target, expression },
                )
            }
        })?,
    )?;

    // Angle dimension between two lines: `bearcad.add_angle_constraint{ a = 0, b = 5,
    // value = "120" }` (bare numbers are degrees; `rad` and parameters work; `sign`
    // picks which of the two wedges, like moving the cursor does interactively).
    // When `sign` is omitted, use the natural leg-pair sign so the expression applies
    // to the acute/obtuse wedge the lines currently form (#489).
    api.set(
        "add_angle_constraint",
        lua.create_function(|lua, opts: Table| {
            let line_a: usize = opts.get("a")?;
            let line_b: usize = opts.get("b")?;
            let expression: String = opts
                .get::<Option<String>>("value")?
                .or(opts.get::<Option<f64>>("angle")?.map(|a| a.to_string()))
                .ok_or_else(|| mlua::Error::external("add_angle_constraint requires `value`"))?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe {
                let rotation_sign: i8 = if let Some(s) = opts.get::<Option<i8>>("sign")? {
                    s
                } else {
                    crate::constraints::angle_constraint_natural_sign(
                        &tick.state().doc,
                        crate::model::ConstraintLine::Line(line_key_from_ordinal(lua, line_a)?),
                        crate::model::ConstraintLine::Line(line_key_from_ordinal(lua, line_b)?),
                    )
                    .unwrap_or(1)
                };
                tick.exec(Instruction::AddAngleConstraint {
                    line_a,
                    line_b,
                    rotation_sign,
                    expression,
                })
            }
        })?,
    )?;

    api.set(
        "add_geometric_constraint",
        lua.create_function(|lua, name: String| {
            let kind = parse_geometric_constraint(&name).ok_or_else(|| {
                mlua::Error::external(format!("unknown geometric constraint '{name}'"))
            })?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::AddGeometricConstraint(kind)) }
        })?,
    )?;

    api.set(
        "constraint_shortcut",
        lua.create_function(|lua, key: mlua::String| {
            let key = key.to_str()?;
            let key = key
                .chars()
                .next()
                .ok_or_else(|| mlua::Error::external("constraint_shortcut requires a key"))?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ApplyConstraintShortcut(key)) }
        })?,
    )?;

    // Two forms: positional `drag_vertex(point, u, v)` moves to an absolute sketch-local
    // spot, and the semantic-gizmo table form `drag_vertex{ point = ..., du = 1, dv = 0 }`
    // (#114) nudges by a delta from the vertex's current position. Both respect
    // constraints and raise (catchable via pcall) when the vertex is fully constrained.
    api.set(
        "drag_vertex",
        lua.create_function(|lua, (first, u, v): (Table, Option<f32>, Option<f32>)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let (point, u, v) = match (u, v) {
                (Some(u), Some(v)) => (parse_constraint_point_table(lua, first)?, u, v),
                _ => {
                    let point_table: Table = first.get("point")?;
                    let point = parse_constraint_point_table(lua, point_table)?;
                    let du: Option<f32> = first.get("du")?;
                    let dv: Option<f32> = first.get("dv")?;
                    if du.is_none() && dv.is_none() {
                        return Err(mlua::Error::external(
                            "drag_vertex table form requires `du` and/or `dv`",
                        ));
                    }
                    let (cur_u, cur_v) = unsafe {
                        let state = tick.state();
                        let sketch = state
                            .sketch_session
                            .map(|s| s.sketch)
                            .ok_or_else(|| mlua::Error::external("Not in sketch mode"))?;
                        crate::geometric_constraints::point_uv(&state.doc, sketch, point.clone())
                            .map_err(mlua::Error::external)?
                    };
                    (
                        point,
                        cur_u + du.unwrap_or(0.0),
                        cur_v + dv.unwrap_or(0.0),
                    )
                }
            };
            unsafe { tick.exec(Instruction::DragVertex { point, u, v }) }
        })?,
    )?;

    // Two forms: positional `drag_line(line, anchor_u, anchor_v, u, v)` replays a raw
    // grab-here-drop-there gesture, and the semantic-gizmo table form
    // `drag_line{ line = ..., du = 0, dv = 2 }` (#114) translates the line by a delta
    // (line drags are pure translations from the anchor, so the anchor is arbitrary).
    api.set(
        "drag_line",
        lua.create_function(
            |lua,
             (first, anchor_u, anchor_v, u, v): (
                Table,
                Option<f32>,
                Option<f32>,
                Option<f32>,
                Option<f32>,
            )| {
                let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
                let (target, anchor_u, anchor_v, u, v) = match (anchor_u, anchor_v, u, v) {
                    (Some(anchor_u), Some(anchor_v), Some(u), Some(v)) => {
                        (parse_constraint_line_table(lua, first)?, anchor_u, anchor_v, u, v)
                    }
                    _ => {
                        let line_table: Table = first.get("line")?;
                        let target = parse_constraint_line_table(lua, line_table)?;
                        let du: Option<f32> = first.get("du")?;
                        let dv: Option<f32> = first.get("dv")?;
                        if du.is_none() && dv.is_none() {
                            return Err(mlua::Error::external(
                                "drag_line table form requires `du` and/or `dv`",
                            ));
                        }
                        (target, 0.0, 0.0, du.unwrap_or(0.0), dv.unwrap_or(0.0))
                    }
                };
                unsafe {
                    tick.exec(Instruction::DragLineSegment {
                            target,
                            anchor_u,
                            anchor_v,
                            u,
                            v,
                        },
                    )
                }
            },
        )?,
    )?;

    api.set(
        "edit_plane",
        lua.create_function(|lua, index: usize| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::BeginEditConstructionPlane { index }) }
        })?,
    )?;

    api.set(
        "commit_plane",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::CommitConstructionPlane) }
        })?,
    )?;

    api.set(
        "orbit",
        lua.create_function(|lua, (dx, dy): (f32, f32)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Orbit { dx, dy }) }
        })?,
    )?;

    api.set(
        "pan",
        lua.create_function(|lua, (dx, dy): (f32, f32)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Pan { dx, dy }) }
        })?,
    )?;

    api.set(
        "wheel",
        lua.create_function(|lua, scroll: f32| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Zoom { scroll }) }
        })?,
    )?;

    // First-person mode (#91). `fps()` toggles (or `fps(true|false)` forces);
    // `fps_look(dx, dy)` turns the head in degrees (positive dx right, dy up);
    // `fps_move{ forward?, strafe? }` walks along the ground in mm;
    // `fps_jump()` presses the jump key; `fps_fly(on?)` toggles/sets flying;
    // `fps_advance(seconds)` runs physics with no keys held (lands a jump).
    api.set(
        "fps",
        lua.create_function(|lua, on: Option<bool>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::FpsMode { on }) }
        })?,
    )?;
    api.set(
        "fps_look",
        lua.create_function(|lua, (dx, dy): (f32, f32)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::FpsLook { dx, dy }) }
        })?,
    )?;
    api.set(
        "fps_move",
        lua.create_function(|lua, opts: Table| {
            let forward: f32 = opts.get::<Option<f32>>("forward")?.unwrap_or(0.0);
            let strafe: f32 = opts.get::<Option<f32>>("strafe")?.unwrap_or(0.0);
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::FpsMove { forward, strafe }) }
        })?,
    )?;
    api.set(
        "fps_jump",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::FpsJump) }
        })?,
    )?;
    api.set(
        "fps_fly",
        lua.create_function(|lua, on: Option<bool>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::FpsFly { on }) }
        })?,
    )?;
    api.set(
        "fps_advance",
        lua.create_function(|lua, seconds: f32| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::FpsAdvance { seconds }) }
        })?,
    )?;
    api.set(
        "fps_scale",
        lua.create_function(|lua, scale: f32| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::FpsScale { scale }) }
        })?,
    )?;

    api.set(
        "_view",
        lua.create_function(|lua, args: MultiValue| {
            let args = args.into_vec();
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let first = args
                .first()
                .ok_or_else(|| mlua::Error::external("view requires an argument"))?;
            match first {
                Value::String(s) => {
                    let name = s.to_str()?.to_string();
                    if let Some(mode) = ProjectionMode::from_name(&name) {
                        return unsafe { tick.exec(Instruction::ProjectionMode(mode)) };
                    }
                    if name.eq_ignore_ascii_case("edge") {
                        let edge_name = match args.get(1) {
                            Some(Value::String(s)) => s.to_str()?.as_ref().to_string(),
                            _ => return Err(mlua::Error::external("view edge requires edge id")),
                        };
                        let edge = CubeEdgeId::from_name(&edge_name).ok_or_else(|| {
                            mlua::Error::external(format!("unknown view edge '{edge_name}'"))
                        })?;
                        return unsafe { tick.exec(Instruction::ViewEdge(edge)) };
                    }
                    if name.eq_ignore_ascii_case("corner") {
                        let corner_name = match args.get(1) {
                            Some(Value::String(s)) => s.to_str()?.as_ref().to_string(),
                            _ => {
                                return Err(mlua::Error::external("view corner requires corner id"))
                            }
                        };
                        let corner = CubeCornerId::from_name(&corner_name).ok_or_else(|| {
                            mlua::Error::external(format!("unknown view corner '{corner_name}'"))
                        })?;
                        return unsafe { tick.exec(Instruction::ViewCorner(corner)) };
                    }
                    let view = StandardView::from_name(&name).ok_or_else(|| {
                        mlua::Error::external(format!("unknown standard view '{name}'"))
                    })?;
                    unsafe { tick.exec(Instruction::View(view)) }
                }
                other => Err(mlua::Error::external(format!(
                    "view expects a string, got {other:?}"
                ))),
            }
        })?,
    )?;

    api.set(
        "_view_home",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ViewHome) }
        })?,
    )?;

    api.set(
        "set_home_view",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SetHomeView) }
        })?,
    )?;

    api.set(
        "toggle_projection",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ToggleProjectionMode) }
        })?,
    )?;

    api.set(
        "shading",
        lua.create_function(|lua, name: String| {
            let mode = ShadingMode::from_name(&name)
                .ok_or_else(|| mlua::Error::external(format!("unknown shading mode '{name}'")))?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ShadingMode(mode)) }
        })?,
    )?;

    // #159: how the ground plane renders ("grid" | "solid").
    api.set(
        "ground",
        lua.create_function(|lua, name: String| {
            let mode = GroundDisplay::from_name(&name)
                .ok_or_else(|| mlua::Error::external(format!("unknown ground display '{name}'")))?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::GroundDisplay(mode)) }
        })?,
    )?;

    // #108: absolute camera control. `bearcad.ui.camera{}` (no args / no pose keys) is a pure
    // read of the live pose; passing any subset of `yaw`/`pitch`/`distance`/`target = {x, y, z}`
    // sets those fields instantly (no transition animation — deterministic for screenshots).
    api.set(
        "camera",
        lua.create_function(|lua, opts: Option<Table>| {
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let (yaw, pitch, distance, target) = match &opts {
                Some(t) => (
                    t.get::<Option<f32>>("yaw")?,
                    t.get::<Option<f32>>("pitch")?,
                    t.get::<Option<f32>>("distance")?,
                    match t.get::<Option<Table>>("target")? {
                        Some(v) => Some((v.get(1)?, v.get(2)?, v.get(3)?)),
                        None => None,
                    },
                ),
                None => (None, None, None, None),
            };
            if yaw.is_none() && pitch.is_none() && distance.is_none() && target.is_none() {
                let cam = unsafe { &tick.state().cam };
                let t = lua.create_table()?;
                t.set("yaw", cam.yaw)?;
                t.set("pitch", cam.pitch)?;
                t.set("distance", cam.distance)?;
                t.set("target", vec3_lua(lua, cam.target)?)?;
                t.set(
                    "projection",
                    match cam.projection_mode() {
                        ProjectionMode::Natural => "perspective",
                        ProjectionMode::Orthographic => "orthographic",
                    },
                )?;
                return Ok(Value::Table(t));
            }
            unsafe {
                tick.exec(Instruction::SetCamera {
                    yaw,
                    pitch,
                    distance,
                    target,
                })?;
            }
            Ok(Value::Nil)
        })?,
    )?;

    // #108/#1276/#1303: frame selection or document; half-Home glide unless animation is off.
    // Native name is `_zoom_fit`; the public `zoom_fit` yields until the transition ends.
    api.set(
        "_zoom_fit",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ZoomFit) }
        })?,
    )?;

    // #34/#94/#108: switch the Elements pane's layout ("list" | "tree" | "graph").
    api.set(
        "elements_view",
        lua.create_function(|lua, name: String| {
            let mode = crate::hierarchy::HierarchyViewMode::from_name(&name).ok_or_else(|| {
                mlua::Error::external(format!(
                    "unknown elements view '{name}' (expected 'list', 'tree', or 'graph')"
                ))
            })?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SetElementsView { mode }) }
        })?,
    )?;

    api.set(
        "pane",
        lua.create_function(|lua, (pane, visible): (String, Value)| {
            let pane = Pane::from_name(&pane)
                .ok_or_else(|| mlua::Error::external(format!("unknown pane '{pane}'")))?;
            let visible = parse_visibility(visible)?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SetPane { pane, visible }) }
        })?,
    )?;

    api.set(
        "parameter",
        lua.create_function(|lua, args: MultiValue| {
            let args = args.into_vec();
            let action = match args.first() {
                Some(Value::String(s)) => s.to_str()?.to_ascii_lowercase(),
                _ => return Err(mlua::Error::external("parameter requires action")),
            };
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            match action.as_str() {
                "add" => {
                    let name = match args.get(1) {
                        Some(Value::String(s)) => s.to_str()?.to_string(),
                        _ => return Err(mlua::Error::external("parameter add requires name")),
                    };
                    let expression = match args.get(2) {
                        Some(Value::String(s)) => s.to_str()?.to_string(),
                        _ => {
                            return Err(mlua::Error::external(
                                "parameter add requires expression",
                            ))
                        }
                    };
                    unsafe {
                        tick.exec(Instruction::AddParameter { name, expression })?;
                    }
                    Ok(Value::Nil)
                }
                // Pure reads (#107): `parameter("get", name)` evaluates the named parameter
                // to its canonical numeric value (mm for lengths, radians for angles) or nil;
                // `parameter("get_expression", name)` returns the raw expression string.
                "get" | "get_expression" => {
                    let name = match args.get(1) {
                        Some(Value::String(s)) => s.to_str()?.to_string(),
                        _ => {
                            return Err(mlua::Error::external(
                                "parameter get requires a parameter name",
                            ))
                        }
                    };
                    let doc = unsafe { &tick.state().doc };
                    let Some(param) = doc.parameters.values().find(|p| p.name == name) else {
                        return Ok(Value::Nil);
                    };
                    if action == "get_expression" {
                        return Ok(Value::String(lua.create_string(&param.expression)?));
                    }
                    match crate::value::eval_parameter_in_doc(&param.expression, doc) {
                        Some(crate::value::EvaluatedParameter::LengthMm(v))
                        | Some(crate::value::EvaluatedParameter::AngleRad(v)) => {
                            Ok(Value::Number(v as f64))
                        }
                        None => Ok(Value::Nil),
                    }
                }
                "from_line_length" => {
                    let line_index = match args.get(1) {
                        Some(Value::Integer(i)) => *i as usize,
                        Some(Value::Number(n)) => n.round() as usize,
                        _ => {
                            return Err(mlua::Error::external(
                                "parameter from_line_length requires line index",
                            ))
                        }
                    };
                    let name = match args.get(2) {
                        Some(Value::String(s)) => Some(s.to_str()?.to_string()),
                        None => None,
                        _ => {
                            return Err(mlua::Error::external(
                                "parameter from_line_length name must be a string",
                            ))
                        }
                    };
                    unsafe {
                        tick.exec(Instruction::CreateParameterFromLineLength {
                            line_index,
                            name,
                        })?;
                    }
                    Ok(Value::Nil)
                }
                "value" | "expression" => {
                    let index = match args.get(1) {
                        Some(Value::Integer(i)) => *i as usize,
                        Some(Value::Number(n)) => n.round() as usize,
                        _ => return Err(mlua::Error::external("parameter value requires index")),
                    };
                    let expression = match args.get(2) {
                        Some(Value::String(s)) => s.to_str()?.to_string(),
                        _ => {
                            return Err(mlua::Error::external(
                                "parameter value requires expression",
                            ))
                        }
                    };
                    unsafe {
                        tick.exec(Instruction::SetParameterExpression { index, expression })?;
                    }
                    Ok(Value::Nil)
                }
                "name" => {
                    let index = match args.get(1) {
                        Some(Value::Integer(i)) => *i as usize,
                        Some(Value::Number(n)) => n.round() as usize,
                        _ => return Err(mlua::Error::external("parameter name requires index")),
                    };
                    let name = match args.get(2) {
                        Some(Value::String(s)) => s.to_str()?.to_string(),
                        _ => return Err(mlua::Error::external("parameter name requires name")),
                    };
                    unsafe {
                        tick.exec(Instruction::SetParameterName { index, name })?;
                    }
                    Ok(Value::Nil)
                }
                "delete" => {
                    let index = match args.get(1) {
                        Some(Value::Integer(i)) => *i as usize,
                        Some(Value::Number(n)) => n.round() as usize,
                        _ => return Err(mlua::Error::external("parameter delete requires index")),
                    };
                    unsafe {
                        tick.exec(Instruction::DeleteParameter { index })?;
                    }
                    Ok(Value::Nil)
                }
                // #1180: Private is the inverse of primary (true = secondary/hidden).
                "private" => {
                    let index = match args.get(1) {
                        Some(Value::Integer(i)) => *i as usize,
                        Some(Value::Number(n)) => n.round() as usize,
                        _ => return Err(mlua::Error::external("parameter private requires index")),
                    };
                    let private = match args.get(2) {
                        Some(Value::Boolean(b)) => *b,
                        _ => {
                            return Err(mlua::Error::external(
                                "parameter private requires true/false",
                            ))
                        }
                    };
                    unsafe {
                        tick.exec(Instruction::SetParameterPrimary {
                            index,
                            primary: !private,
                        })?;
                    }
                    Ok(Value::Nil)
                }
                // #1176: min / max / step bounds — expression string sets, omit/empty clears.
                "min" | "minimum" | "max" | "maximum" | "step" => {
                    let which = match action.as_str() {
                        "min" | "minimum" => crate::parameters::ParameterBound::Minimum,
                        "max" | "maximum" => crate::parameters::ParameterBound::Maximum,
                        "step" => crate::parameters::ParameterBound::Step,
                        _ => unreachable!(),
                    };
                    let index = match args.get(1) {
                        Some(Value::Integer(i)) => *i as usize,
                        Some(Value::Number(n)) => n.round() as usize,
                        _ => {
                            return Err(mlua::Error::external(format!(
                                "parameter {} requires index",
                                which.label()
                            )))
                        }
                    };
                    let expression = match args.get(2) {
                        Some(Value::String(s)) => {
                            let s = s.to_str()?.to_string();
                            if s.trim().is_empty() {
                                None
                            } else {
                                Some(s)
                            }
                        }
                        Some(Value::Nil) | None => None,
                        _ => {
                            return Err(mlua::Error::external(format!(
                                "parameter {} expression must be a string",
                                which.label()
                            )))
                        }
                    };
                    unsafe {
                        tick.exec(Instruction::SetParameterBound {
                            index,
                            which,
                            expression,
                        })?;
                    }
                    Ok(Value::Nil)
                }
                other => Err(mlua::Error::external(format!(
                    "unknown parameter action '{other}'"
                ))),
            }
        })?,
    )?;

    api.set(
        "delete_selection",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::DeleteSelection) }
        })?,
    )?;

    // #737: drive the Settings window (docs captures need it open with help mode on).
    api.set(
        "settings",
        lua.create_function(|lua, verb: Option<String>| {
            let open = match verb.as_deref() {
                Some("show") | Some("open") => Some(true),
                Some("hide") | Some("close") => Some(false),
                None | Some("toggle") => None,
                Some(other) => {
                    return Err(mlua::Error::external(format!(
                        "settings expects \"show\"/\"hide\"/\"toggle\", got {other:?}"
                    )))
                }
            };
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SetSettingsWindow { open }) }
        })?,
    )?;

    // #1328: Help → Changelog. `text` returns the markdown baked into this build.
    api.set(
        "changelog",
        lua.create_function(|lua, verb: Option<String>| {
            match verb.as_deref() {
                Some("text") => Ok(mlua::Value::String(lua.create_string(crate::changelog::markdown())?)),
                other => {
                    let open = match other {
                        Some("show") | Some("open") => Some(true),
                        Some("hide") | Some("close") => Some(false),
                        None | Some("toggle") => None,
                        Some(got) => {
                            return Err(mlua::Error::external(format!(
                                "changelog expects \"show\"/\"hide\"/\"toggle\"/\"text\", got {got:?}"
                            )))
                        }
                    };
                    let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
                    unsafe { tick.exec(Instruction::SetChangelogWindow { open })? };
                    Ok(mlua::Value::Nil)
                }
            }
        })?,
    )?;

    // Tutorials pane (#1241): open/close/toggle, list registered walkthroughs with
    // completion flags, or start one by name (see `tutorial` below).
    api.set(
        "tutorial_pane",
        lua.create_function(|lua, verb: Option<String>| {
            let open = match verb.as_deref() {
                Some("show") | Some("open") => Some(true),
                Some("hide") | Some("close") => Some(false),
                None | Some("toggle") => None,
                Some(other) => {
                    return Err(mlua::Error::external(format!(
                        "tutorial_pane expects \"show\"/\"hide\"/\"toggle\", got {other:?}"
                    )))
                }
            };
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SetTutorialPane { open }) }
        })?,
    )?;
    api.set(
        "tutorials",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let state = unsafe { tick.state() };
            let list = lua.create_table()?;
            for (i, tut) in crate::tutorial::TUTORIALS.iter().enumerate() {
                let row = lua.create_table()?;
                row.set("name", tut.name)?;
                row.set("title", tut.title)?;
                row.set("completed", state.tutorial_completed(tut.name))?;
                list.set(i + 1, row)?;
            }
            Ok(list)
        })?,
    )?;
    // #1434: skip prompting, install age, button highlight, launch tooltip.
    api.set(
        "skip_all_tutorials",
        lua.create_function(|lua, skip: Option<bool>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            if let Some(skip) = skip {
                unsafe { tick.exec(Instruction::SkipAllTutorials { skip })? };
            }
            let state = unsafe { tick.state() };
            Ok(state.skip_all_tutorials)
        })?,
    )?;
    api.set(
        "install_age",
        lua.create_function(|lua, days: Option<Value>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let state = unsafe { tick.state() };
            match days {
                None => {}
                Some(Value::Nil) | Some(Value::Boolean(false)) => {
                    state.set_install_age_days(None);
                }
                Some(Value::Number(n)) => state.set_install_age_days(Some(n)),
                Some(Value::Integer(n)) => state.set_install_age_days(Some(n as f64)),
                Some(other) => {
                    return Err(mlua::Error::external(format!(
                        "install_age expects days or false, got {other:?}"
                    )));
                }
            }
            Ok(state.install_age_days())
        })?,
    )?;
    api.set(
        "tutorial_highlight",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let state = unsafe { tick.state() };
            Ok(state.tutorials_button_highlighted())
        })?,
    )?;
    api.set(
        "tutorial_prompt",
        lua.create_function(|lua, (verb, arg): (Option<String>, Option<f32>)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            if let Some(verb) = verb.as_deref() {
                let state = unsafe { tick.state() };
                match verb {
                    "launch" | "show" | "arm" => state.prepare_tutorial_prompt(),
                    "work" => state.note_document_work(),
                    "tick" => state.tick_tutorial_prompt(arg.unwrap_or(0.0)),
                    "dismiss" | "hide" => state.dismiss_tutorial_prompt(),
                    other => {
                        return Err(mlua::Error::external(format!(
                            "tutorial_prompt expects \"launch\"/\"work\"/\"tick\"/\"dismiss\", got {other:?}"
                        )));
                    }
                }
            }
            let state = unsafe { tick.state() };
            match (state.tutorial_prompt_text(), state.tutorial_prompt_alpha()) {
                (Some(text), Some(alpha)) => {
                    let t = lua.create_table()?;
                    t.set("text", text)?;
                    t.set("alpha", alpha)?;
                    Ok(Value::Table(t))
                }
                _ => Ok(Value::Nil),
            }
        })?,
    )?;
    api.set(
        "complete_tutorial",
        lua.create_function(|lua, name: String| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            if crate::tutorial::tutorial_index(&name).is_none() {
                return Err(mlua::Error::external(format!("unknown tutorial '{name}'")));
            }
            let state = unsafe { tick.state() };
            state.mark_tutorial_completed(&name);
            Ok(())
        })?,
    )?;

    // Document tabs: new / close / select / reorder / detach, plus a read-only strip snapshot.
    api.set(
        "new_tab",
        lua.create_function(|lua, opts: Option<Table>| {
            let same = opts
                .as_ref()
                .and_then(|t| t.get::<bool>("same").ok())
                .unwrap_or(false);
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            if same {
                unsafe { tick.exec(Instruction::NewTabSameDocument) }
            } else {
                unsafe { tick.exec(Instruction::NewTab) }
            }
        })?,
    )?;
    api.set(
        "close_tab",
        lua.create_function(|lua, index: Option<usize>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::CloseTab { index }) }
        })?,
    )?;
    api.set(
        "tab",
        lua.create_function(|lua, index: Option<usize>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe {
                if let Some(index) = index {
                    tick.exec(Instruction::SelectTab { index })?;
                }
                Ok(tick.state().script_active_tab)
            }
        })?,
    )?;
    api.set(
        "reorder_tab",
        lua.create_function(|lua, (from, to): (usize, usize)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ReorderTab { from, to }) }
        })?,
    )?;
    api.set(
        "detach_tab",
        lua.create_function(|lua, index: Option<usize>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::DetachTab { index }) }
        })?,
    )?;
    api.set(
        "tab_count",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { Ok(tick.state().script_tab_titles.len()) }
        })?,
    )?;
    api.set(
        "window_count",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            // Secondary windows are full application windows (#1133); count includes main.
            unsafe { Ok(tick.state().script_window_count) }
        })?,
    )?;
    api.set(
        "tabs",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe {
                let state = tick.state();
                let table = lua.create_table()?;
                for (i, title) in state.script_tab_titles.iter().enumerate() {
                    let row = lua.create_table()?;
                    row.set("title", title.as_str())?;
                    row.set(
                        "dirty",
                        state.script_tab_dirty.get(i).copied().unwrap_or(false),
                    )?;
                    row.set("active", i == state.script_active_tab)?;
                    table.set(i + 1, row)?; // 1-based Lua array
                }
                Ok(table)
            }
        })?,
    )?;

    // #1022: drive the McMaster-Carr catalog window, with an optional part number to open
    // it at — docs captures need it open, and a script needs to be able to say which part.
    api.set(
        "mcmaster",
        lua.create_function(|lua, (verb, part): (Option<String>, Option<String>)| {
            let open = match verb.as_deref() {
                Some("show") | Some("open") => Some(true),
                Some("hide") | Some("close") => Some(false),
                None | Some("toggle") => None,
                Some(other) => {
                    return Err(mlua::Error::external(format!(
                        "mcmaster expects \"show\"/\"hide\"/\"toggle\", got {other:?}"
                    )))
                }
            };
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SetMcMasterWindow { open, part }) }
        })?,
    )?;

    api.set(
        "palette",
        lua.create_function(|lua, args: MultiValue| {
            let args = args.into_vec();
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            if args.is_empty() {
                return unsafe { tick.exec(Instruction::SetCommandPalette { open: None }) };
            }
            match args.first() {
                Some(Value::String(s)) if s.to_str()? == "run" => {
                    let query = match args.get(1) {
                        Some(Value::String(s)) => s.to_str()?.to_string(),
                        _ => return Err(mlua::Error::external("palette run requires query")),
                    };
                    // A command that asks for an argument (#1022) takes it as the third
                    // value: `bearcad.ui.palette("run", "mcmaster", "socket head screw")`.
                    let argument = match args.get(2) {
                        Some(Value::String(s)) => Some(s.to_str()?.to_string()),
                        _ => None,
                    };
                    unsafe { tick.exec(Instruction::RunPaletteCommand { query, argument }) }
                }
                Some(Value::String(s)) => {
                    let verb = s.to_str()?.to_ascii_lowercase();
                    let open = match verb.as_str() {
                        "show" | "open" => Some(true),
                        "hide" | "close" => Some(false),
                        "toggle" => None,
                        other => {
                            return Err(mlua::Error::external(format!(
                                "unknown palette action '{other}'"
                            )))
                        }
                    };
                    unsafe { tick.exec(Instruction::SetCommandPalette { open }) }
                }
                _ => Err(mlua::Error::external("palette expects a string action")),
            }
        })?,
    )?;

    api.set(
        "auto_zoom",
        lua.create_function(|lua, on: Option<bool>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let on = on.unwrap_or(true);
            unsafe { tick.exec(Instruction::SetAutoZoom { on }) }
        })?,
    )?;

    // #913: snapping while drawing and placing, app-wide.
    api.set(
        "snapping",
        lua.create_function(|lua, on: Option<bool>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let on = on.unwrap_or(true);
            unsafe { tick.exec(Instruction::SetSnapping { on }) }
        })?,
    )?;

    // #917: how far apart the Move tool's rotation candidates sit, in degrees (0–90).
    api.set(
        "angle_snap",
        lua.create_function(|lua, degrees: f32| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SetMoveAngleSnap { degrees }) }
        })?,
    )?;

    // #906: the joint preview's sweep, app-wide.
    api.set(
        "animate_joints",
        lua.create_function(|lua, on: Option<bool>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let on = on.unwrap_or(true);
            unsafe { tick.exec(Instruction::SetJointAnimation { on }) }
        })?,
    )?;

    // #1276/#1303: Zoom to Fit glide (half Home duration). Off snaps instantly.
    api.set(
        "animate_zoom_to_fit",
        lua.create_function(|lua, on: Option<bool>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let on = on.unwrap_or(true);
            unsafe { tick.exec(Instruction::SetAnimateZoomToFit { on }) }
        })?,
    )?;

    // #1288: auto-update channel — "release" (default) or "pre_release".
    // No arg / nil returns the current channel string.
    api.set(
        "update_channel",
        lua.create_function(|lua, channel: Option<String>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            match channel {
                None => {
                    let state = unsafe { tick.state() };
                    Ok(Some(state.update_channel.as_str().to_string()))
                }
                Some(s) => {
                    let channel = crate::settings::UpdateChannel::parse(&s).ok_or_else(|| {
                        mlua::Error::external(format!(
                            "update_channel expects \"release\" or \"pre_release\", got {s:?}"
                        ))
                    })?;
                    unsafe { tick.exec(Instruction::SetUpdateChannel { channel })? };
                    Ok(None)
                }
            }
        })?,
    )?;

    // Read a line's current endpoints (sketch-local mm) — the assertion hook for
    // interaction regression tests.
    api.set(
        "line_endpoints",
        lua.create_function(|lua, index: usize| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let state = unsafe { tick.state() };
            // The script's `index` is the line's ordinal (#1055).
            let line = state
                .doc
                .lines
                .keys()
                .nth(index)
                .map(|k| &state.doc.lines[k])
                .ok_or_else(|| mlua::Error::external(format!("no line {index}")))?;
            Ok((line.x0, line.y0, line.x1, line.y1))
        })?,
    )?;

    // Touch mode: force on/off (otherwise auto-detected from the first real touch).
    api.set(
        "touch",
        lua.create_function(|lua, on: Option<bool>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SetTouchMode { on: on.unwrap_or(true) }) }
        })?,
    )?;

    // Interactive tutorials: start by registry name, advance a manual step, end, or
    // read the current step index (nil when none is running).
    api.set(
        "tutorial",
        lua.create_function(|lua, name: String| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let Some(index) = crate::tutorial::tutorial_index(&name) else {
                return Err(mlua::Error::external(format!("unknown tutorial '{name}'")));
            };
            unsafe { tick.exec(Instruction::StartTutorial { index }) }
        })?,
    )?;
    api.set(
        "tutorial_next",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::TutorialNext) }
        })?,
    )?;
    api.set(
        "tutorial_assist",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::TutorialAssist) }
        })?,
    )?;
    api.set(
        "tutorial_end",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::EndTutorial) }
        })?,
    )?;
    api.set(
        "tutorial_step",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let state = unsafe { tick.state() };
            // Some scripted edits mutate the document without routing through
            // `apply`; settle the tutorial before reporting, like the GUI frame does.
            state.advance_tutorial();
            Ok(state.tutorial.map(|r| r.step))
        })?,
    )?;
    // Animated guide-orb screen position (#1346). Nil when no ring is drawn.
    api.set(
        "tutorial_orb",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let state = unsafe { tick.state() };
            let Some(p) = state.tutorial_orb_screen else {
                return Ok(Value::Nil);
            };
            let t = lua.create_table()?;
            t.set("x", p.x)?;
            t.set("y", p.y)?;
            Ok(Value::Table(t))
        })?,
    )?;

    api.set(
        "move",
        lua.create_function(|lua, (x, y): (f32, f32)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Move { x, y }) }
        })?,
    )?;

    api.set(
        "click",
        lua.create_function(|lua, (x, y, opts): (f32, f32, Option<Table>)| {
            let mods = click_mods(opts)?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Click { x, y, mods }) }
        })?,
    )?;

    api.set(
        "help",
        lua.create_function(|lua, on: Option<bool>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::HelpMode { on }) }
        })?,
    )?;

    // #1319: shortcut badges shown on the toolbar while help mode is on.
    api.set(
        "toolbar_shortcuts",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let state = unsafe { tick.state() };
            let table = lua.create_table()?;
            for (name, label) in crate::shortcuts::toolbar_help_shortcuts(
                state.help_mode,
                state.editing_drawing.is_some(),
                state.sketch_session.is_some(),
            ) {
                table.set(name, label)?;
            }
            Ok(table)
        })?,
    )?;

    api.set(
        "tool_mode",
        lua.create_function(|lua, mode: String| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SetToolMode(mode)) }
        })?,
    )?;

    api.set(
        "move_ground",
        lua.create_function(|lua, (x, y): (f32, f32)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::MoveGround { x, y }) }
        })?,
    )?;

    api.set(
        "drag_ground",
        lua.create_function(|lua, (x0, y0, x1, y1): (f32, f32, f32, f32)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::DragGround { x0, y0, x1, y1 }) }
        })?,
    )?;
    api.set(
        "click_ground",
        lua.create_function(|lua, (x, y, opts): (f32, f32, Option<Table>)| {
            let mods = click_mods(opts)?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ClickGround { x, y, mods }) }
        })?,
    )?;

    api.set(
        "drag",
        lua.create_function(|lua, (x0, y0, x1, y1): (f32, f32, f32, f32)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Drag { x0, y0, x1, y1 }) }
        })?,
    )?;

    api.set(
        "right_drag",
        lua.create_function(|lua, (dx, dy): (f32, f32)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::RightDrag { dx, dy }) }
        })?,
    )?;

    api.set(
        "right_drag_pan",
        lua.create_function(|lua, (dx, dy): (f32, f32)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::RightDragShift { dx, dy }) }
        })?,
    )?;

    api.set(
        "key",
        // Optional `{ shift = true }` (etc.) holds modifiers for the key tap (#1198).
        lua.create_function(|lua, (name, opts): (String, Option<Table>)| {
            let key = parse_key(&name)
                .map_err(mlua::Error::external)?;
            let mods = click_mods(opts)?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Key { key, mods }) }
        })?,
    )?;

    api.set(
        "keydown",
        lua.create_function(|lua, name: String| {
            let key = parse_key(&name)
                .map_err(mlua::Error::external)?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::KeyDown(key)) }
        })?,
    )?;

    api.set(
        "keyup",
        lua.create_function(|lua, name: String| {
            let key = parse_key(&name)
                .map_err(mlua::Error::external)?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::KeyUp(key)) }
        })?,
    )?;

    api.set(
        "type",
        lua.create_function(|lua, text: String| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Type(text)) }
        })?,
    )?;

    api.set(
        "_wait",
        lua.create_function(|lua, frames: u32| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::WaitFrames(frames)) }
        })?,
    )?;

    api.set(
        "_wait_ms",
        lua.create_function(|lua, ms: u64| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::WaitMs(ms)) }
        })?,
    )?;

    api.set(
        "_screenshot",
        lua.create_function(|lua, (path, region): (Option<String>, Option<Value>)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let path = path
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| "screenshot-bearcad.png".to_string());
            // `true` still means the whole window, as it did before regions (#672).
            let region = match region {
                None | Some(Value::Nil) | Some(Value::Boolean(false)) => ScreenshotRegion::Viewport,
                Some(Value::Boolean(true)) => ScreenshotRegion::Window,
                Some(Value::String(name)) => {
                    let name = name.to_str()?;
                    ScreenshotRegion::from_name(&name).ok_or_else(|| {
                        mlua::Error::external(format!("unknown screenshot region '{name}'"))
                    })?
                }
                Some(other) => {
                    return Err(mlua::Error::external(format!(
                        "screenshot region must be a name or a boolean, got {}",
                        other.type_name()
                    )))
                }
            };
            unsafe { tick.exec(Instruction::Screenshot { path, region }) }
        })?,
    )?;

    api.set(
        "rect",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(&opts, "rect", &["x", "y", "width", "height", "name"])?;
            let (width, width_expr) = scalar_arg(lua, &opts, "width")?
                .ok_or_else(|| mlua::Error::external("rect requires a `width`"))?;
            let (height, height_expr) = scalar_arg(lua, &opts, "height")?
                .ok_or_else(|| mlua::Error::external("rect requires a `height`"))?;
            let x: f32 = opts.get("x").unwrap_or(0.0);
            let y: f32 = opts.get("y").unwrap_or(0.0);
            unsafe {
                // Make sure we're sketching; default to the ground (XY) construction plane.
                if tick.state().sketch_session.is_none() {
                    let ground = tick.state().doc.ground_plane().ok_or_else(|| {
                        mlua::Error::external("document has no construction plane")
                    })?;
                    tick.exec(Instruction::BeginSketch {
                        face: FaceId::ConstructionPlane(ground),
                    })?;
                }
                tick.exec(Instruction::CreateRect {
                    x,
                    y,
                    width,
                    height,
                    width_expr,
                    height_expr,
                })?;
            }
            // A rectangle is now four plain lines (#66 polygon); return a handle to its bottom
            // edge (the first of the four lines just created).
            // The rectangle's bottom edge: the first of the four lines just created (#1055).
            let element = {
                let keys: Vec<_> = unsafe { tick.state().doc.lines.keys().collect() };
                let Some(&first) = keys.iter().rev().nth(3) else {
                    return Ok(());
                };
                SceneElement::Line(first)
            };
            apply_optional_name(lua, element, Some(opts))
        })?,
    )?;

    api.set(
        "line",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(
                &opts,
                "line",
                &["x", "y", "x1", "y1", "length", "angle", "bezier", "dimension", "name"],
            )?;
            // Either give explicit endpoints (x,y)-(x1,y1), or origin + length + optional angle.
            let x0: f32 = opts.get("x").unwrap_or(0.0);
            let y0: f32 = opts.get("y").unwrap_or(0.0);
            let (x1, y1) = match (opts.get::<Option<f32>>("x1")?, opts.get::<Option<f32>>("y1")?) {
                (Some(x1), Some(y1)) => (x1, y1),
                _ => {
                    let length: f32 = opts.get("length")?;
                    let angle_deg: f32 = opts.get("angle").unwrap_or(0.0);
                    let a = angle_deg.to_radians();
                    (x0 + length * a.cos(), y0 + length * a.sin())
                }
            };
            // `bezier = { {cx0, cy0}, {cx1, cy1} }` makes this a curve (#54): tangent handles
            // near (x0,y0) and (x1,y1) respectively.
            let bezier: Option<[(f32, f32); 2]> = match opts.get::<Option<Table>>("bezier")? {
                Some(t) => {
                    let h0: Table = t.get(1)?;
                    let h1: Table = t.get(2)?;
                    Some([(h0.get(1)?, h0.get(2)?), (h1.get(1)?, h1.get(2)?)])
                }
                None => None,
            };
            // Like clicking, the line lands unconstrained. `dimension = "leg"` (or a number)
            // locks the length with that expression — the scripted equivalent of typing a
            // length while drawing; `dimension = true` locks it at the as-drawn length.
            let dimension: Option<String> = match opts.get::<Value>("dimension")? {
                Value::Nil => None,
                Value::Boolean(false) => None,
                Value::Boolean(true) => {
                    Some(((x1 - x0).hypot(y1 - y0)).to_string())
                }
                Value::String(s) => Some(s.to_str()?.to_string()),
                Value::Integer(i) => Some(i.to_string()),
                Value::Number(n) => Some(n.to_string()),
                _ => {
                    return Err(mlua::Error::external(
                        "line `dimension` must be an expression string, a number, or true",
                    ))
                }
            };
            unsafe {
                if tick.state().sketch_session.is_none() {
                    let ground = tick.state().doc.ground_plane().ok_or_else(|| {
                        mlua::Error::external("document has no construction plane")
                    })?;
                    tick.exec(Instruction::BeginSketch {
                        face: FaceId::ConstructionPlane(ground),
                    })?;
                }
                tick.exec(Instruction::CreateLine { x0, y0, x1, y1, bezier, dimension })?;
            }
            // The line just committed (#1055): the newest live one.
            let Some(key) = (unsafe { tick.state().doc.lines.keys().last() }) else {
                return Ok(());
            };
            apply_optional_name(lua, SceneElement::Line(key), Some(opts))
        })?,
    )?;

    api.set(
        "circle",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(&opts, "circle", &["x", "y", "r", "radius", "diameter", "name"])?;
            let cx: f32 = opts.get("x").unwrap_or(0.0);
            let cy: f32 = opts.get("y").unwrap_or(0.0);
            // Accept a radius (`r` or its `radius` alias, #108) or a `diameter`, in that
            // precedence order; none at all is a clear error rather than a nil-conversion one.
            // Each accepts a parameter expression too (#402); a radius expression doubles
            // into the diameter constraint the way the stored dimension expects.
            let (r, diameter_expr) = if let Some((r, e)) = scalar_arg(lua, &opts, "r")? {
                (r, e.map(|e| format!("({e}) * 2")))
            } else if let Some((radius, e)) = scalar_arg(lua, &opts, "radius")? {
                (radius, e.map(|e| format!("({e}) * 2")))
            } else if let Some((d, e)) = scalar_arg(lua, &opts, "diameter")? {
                (d * 0.5, e)
            } else {
                return Err(mlua::Error::external(
                    "circle requires a size: one of `r`, `radius`, or `diameter`",
                ));
            };
            unsafe {
                if tick.state().sketch_session.is_none() {
                    let ground = tick.state().doc.ground_plane().ok_or_else(|| {
                        mlua::Error::external("document has no construction plane")
                    })?;
                    tick.exec(Instruction::BeginSketch {
                        face: FaceId::ConstructionPlane(ground),
                    })?;
                }
                tick.exec(Instruction::CreateCircle { cx, cy, r, diameter_expr })?;
            }
            // The circle just committed (#1055): the newest live one.
            let Some(key) = (unsafe { tick.state().doc.circles.keys().last() }) else {
                return Ok(());
            };
            apply_optional_name(lua, SceneElement::Circle(key), Some(opts))
        })?,
    )?;

    // Sketch text (#282/#286): the scripted equivalent of the Text tool — glyph outlines are
    // baked from a system font and the font bytes embed in the document. `size` accepts an
    // expression (parameters work); `rotation` is degrees about the baseline origin.
    api.set(
        "text",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let text: String = opts.get("text")?;
            let x: f32 = opts.get("x").unwrap_or(0.0);
            let y: f32 = opts.get("y").unwrap_or(0.0);
            let size: String = match opts.get::<Value>("size")? {
                Value::Nil => "10".to_string(),
                Value::Integer(n) => n.to_string(),
                Value::Number(n) => n.to_string(),
                Value::String(s) => s.to_str()?.to_string(),
                other => {
                    return Err(mlua::Error::external(format!(
                        "text size must be a number or expression string, got {other:?}"
                    )))
                }
            };
            let font: Option<String> = opts.get("font")?;
            let bold: bool = opts.get::<Option<bool>>("bold")?.unwrap_or(false);
            let italic: bool = opts.get::<Option<bool>>("italic")?.unwrap_or(false);
            let underline: bool = opts.get::<Option<bool>>("underline")?.unwrap_or(false);
            let rotation_deg: f32 = opts.get::<Option<f32>>("rotation")?.unwrap_or(0.0);
            let wrap: Option<f32> = opts.get("wrap")?;
            unsafe {
                if tick.state().sketch_session.is_none() {
                    let ground = tick.state().doc.ground_plane().ok_or_else(|| {
                        mlua::Error::external("document has no construction plane")
                    })?;
                    tick.exec(Instruction::BeginSketch {
                        face: FaceId::ConstructionPlane(ground),
                    })?;
                }
                tick.exec(Instruction::CreateSketchText {
                    text,
                    font,
                    bold,
                    italic,
                    underline,
                    size,
                    x,
                    y,
                    rotation_deg,
                    wrap,
                })?;
            }
            // The text just committed (#1055): the newest live one.
            let Some(key) = (unsafe { tick.state().doc.sketch_texts.keys().last() }) else {
                return Ok(());
            };
            apply_optional_name(lua, SceneElement::SketchText(key), Some(opts))
        })?,
    )?;

    // #116: declaratively add a new construction plane offset from an existing one — the
    // scripted equivalent of picking a face/plane in the viewport and typing an offset.
    // `from` defaults to plane 0 (Ground). Alternatively `origin = {x,y,z}` and
    // `normal = {x,y,z}` together anchor the plane on an arbitrary face (#465), like
    // clicking a body face with the Plane tool. There is no scripted way yet to create
    // one anchored on an axis (which also takes an `angle`).
    api.set(
        "plane",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let offset: f32 = opts.get::<Option<f32>>("offset")?.unwrap_or(0.0);
            let from: usize = opts.get::<Option<usize>>("from")?.unwrap_or(0);
            let origin: Option<Table> = opts.get("origin")?;
            let normal: Option<Table> = opts.get("normal")?;
            unsafe {
                match (origin, normal) {
                    (Some(o), Some(n)) => {
                        let v = |t: &Table| -> mlua::Result<glam::Vec3> {
                            Ok(glam::Vec3::new(t.get(1)?, t.get(2)?, t.get(3)?))
                        };
                        tick.exec(Instruction::CreateFacePlane {
                            offset,
                            origin: v(&o)?,
                            normal: v(&n)?,
                        })?;
                    }
                    (None, None) => tick.exec(Instruction::CreatePlane { offset, from })?,
                    _ => {
                        return Err(mlua::Error::external(
                            "plane: origin and normal must be given together",
                        ))
                    }
                }
            }
            // The plane just committed (#1055): the newest live one.
            let Some(key) = (unsafe { tick.state().doc.construction_planes.keys().last() }) else {
                return Ok(());
            };
            apply_optional_name(lua, SceneElement::ConstructionPlane(key), Some(opts))
        })?,
    )?;

    api.set(
        "extrude",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(
                &opts,
                "extrude",
                &[
                    "distance",
                    "to",
                    "circle",
                    "circles",
                    "polygon",
                    "text",
                    "boolean",
                    "body",
                    "name",
                    "symmetric",
                    "taper",
                    "taper_mode",
                ],
            )?;
            // `to = { plane = i } | { face = <face spec> } | { vertex = <point> }` snaps the
            // extrusion to that object's extended plane (#114) — the scripted equivalent of
            // pulling the gizmo onto a surface. With a target, `distance` may be omitted.
            let target = match opts.get::<Option<Table>>("to")? {
                Some(t) => Some(parse_extrude_target_table(lua, &t)?),
                None => None,
            };
            // `distance` accepts a plain number or a parameter expression string (#402).
            let (distance, expression) = match scalar_arg(lua, &opts, "distance")? {
                Some(d) => d,
                None if target.is_some() => (0.0, None),
                None => return Err(mlua::Error::external("extrude requires a `distance` or `to`")),
            };
            // Faces: `circle` (single) and/or `circles` (array of indices), a `polygon` loop
            // (#66 — a rectangle is four lines forming such a loop), or a `boolean` region.
            let mut faces: Vec<crate::model::ExtrudeFace> = Vec::new();
            if let Some(i) = opts.get::<Option<usize>>("circle")? {
                faces.push(crate::model::ExtrudeFace::Circle(circle_key_from_ordinal(lua, i)?));
            }
            if let Some(list) = opts.get::<Option<Vec<usize>>>("circles")? {
                for i in list {
                    faces
                        .push(crate::model::ExtrudeFace::Circle(circle_key_from_ordinal(lua, i)?));
                }
            }
            // `polygon = {line0, line1, ...}`: a single closed-loop face (#66).
            if let Some(lines) = opts.get::<Option<Vec<usize>>>("polygon")? {
                faces.push(crate::model::ExtrudeFace::Polygon(line_keys_from_ordinals(lua, lines)?));
            }
            // `text = index`: extrude/engrave a whole sketch text — every glyph region of it,
            // counters (letter holes) preserved (#285/#355).
            if let Some(ti) = opts.get::<Option<usize>>("text")? {
                let text = sketch_text_key_from_ordinal(lua, ti)?;
                let glyphs = unsafe {
                    tick.state()
                        .doc
                        .sketch_texts
                        .get(text)
                        .map(|t| crate::text::group_glyphs(&t.contours).len())
                        .ok_or_else(|| mlua::Error::external(format!("no sketch text {ti}")))?
                };
                for glyph in 0..glyphs {
                    faces.push(crate::model::ExtrudeFace::TextGlyph { text, glyph });
                }
            }
            // `boolean = {op = "intersection"|"difference", a = <face spec>, b = <face
            // spec>}`: a boolean-combined region of two other (possibly nested) faces
            // (#16/#62) — the toggleable intersection/difference regions of two overlapping
            // shapes.
            if let Some(boolean) = opts.get::<Option<Table>>("boolean")? {
                faces.push(parse_boolean_face_table(lua, &boolean)?);
            }
            if faces.is_empty() {
                return Err(mlua::Error::external(
                    "extrude requires a `circle`/`polygon`/`boolean` or `circles` face list",
                ));
            }
            // `body = "merge"` joins the body of the face being extruded from (if any), and
            // `body = "cut"` subtracts the extrusion from that body (#32/#35); any other value
            // (including the default, omitted) creates a new body. A cut has no effect without
            // a candidate body, and in a non-kernel build renders the additive geometry only.
            let body = match opts.get::<Option<String>>("body")?.as_deref() {
                Some("merge") => crate::actions::ExtrudeBodyChoice::Merge,
                Some("cut") => crate::actions::ExtrudeBodyChoice::Cut,
                Some("join") => crate::actions::ExtrudeBodyChoice::JoinNew,
                _ => crate::actions::ExtrudeBodyChoice::New,
            };
            // Sketch from the first face's geometry (all faces should be coplanar).
            let sketch = unsafe {
                let doc = &tick.state().doc;
                crate::actions::extrude_face_sketch(doc, &faces[0])
            }
            .ok_or_else(|| mlua::Error::external("extrude face does not exist"))?;
            // The instruction names the sketch by its ordinal (#1055).
            let sketch = unsafe {
                tick.state().doc.sketches.keys().position(|k| k == sketch)
            }
            .ok_or_else(|| mlua::Error::external("extrude face does not exist"))?;
            let symmetric: bool = opts.get::<Option<bool>>("symmetric")?.unwrap_or(false);
            // Taper (#1243): `taper` is a number or expression; `taper_mode = "distance"|"angle"`.
            let taper_mode = match opts.get::<Option<String>>("taper_mode")? {
                None => crate::model::ExtrudeTaperMode::Distance,
                Some(s) => crate::model::ExtrudeTaperMode::from_name(&s).ok_or_else(|| {
                    mlua::Error::external(format!(
                        "unknown taper_mode '{s}' (distance|angle)"
                    ))
                })?,
            };
            let (taper, taper_expression) = match scalar_arg(lua, &opts, "taper")? {
                Some((v, e)) => {
                    // Angle mode: bare numbers are degrees (not radians).
                    let v = if taper_mode == crate::model::ExtrudeTaperMode::Angle {
                        // scalar_arg always treats numbers as lengths; for angles a bare
                        // number is already degrees when the user wrote `taper = -45`.
                        v
                    } else {
                        v
                    };
                    (v, e)
                }
                None => (0.0, None),
            };
            unsafe {
                tick.exec(Instruction::Extrude {
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
                })?;
            }
            // The extrusion just committed (#1055): the newest live one.
            let Some(key) = (unsafe { tick.state().doc.extrusions.keys().last() }) else {
                return Ok(());
            };
            apply_optional_name(lua, SceneElement::Extrusion(key), Some(opts))
        })?,
    )?;

    // Push/pull a bare 3D body face directly (#130/#122): `face = { kind = "extrude_cap" |
    // "extrude_side", ... }` picks the face, `distance` (or `to = { face|plane|vertex }` to
    // snap onto another surface) drives the depth, and `body = "merge"|"cut"` attaches it —
    // the declarative equivalent of clicking the face with the Extrude tool and pulling it.
    api.set(
        "extrude_face",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let face_table: Table = opts
                .get("face")
                .map_err(|_| mlua::Error::external("extrude_face requires a `face` table"))?;
            let face = parse_face_id_table(lua, face_table)?;
            let target = match opts.get::<Option<Table>>("to")? {
                Some(t) => Some(parse_extrude_target_table(lua, &t)?),
                None => None,
            };
            let distance: f32 = match opts.get::<Option<f32>>("distance")? {
                Some(d) => d,
                None if target.is_some() => 0.0,
                None => {
                    return Err(mlua::Error::external(
                        "extrude_face requires a `distance` or `to`",
                    ))
                }
            };
            let body = match opts.get::<Option<String>>("body")?.as_deref() {
                Some("merge") => crate::actions::ExtrudeBodyChoice::Merge,
                Some("cut") => crate::actions::ExtrudeBodyChoice::Cut,
                Some("join") => crate::actions::ExtrudeBodyChoice::JoinNew,
                _ => crate::actions::ExtrudeBodyChoice::New,
            };
            unsafe {
                tick.exec(Instruction::ExtrudeBodyFace { face, distance, body, target })?;
            }
            // The extrusion just committed (#1055): the newest live one.
            let Some(key) = (unsafe { tick.state().doc.extrusions.keys().last() }) else {
                return Ok(());
            };
            apply_optional_name(lua, SceneElement::Extrusion(key), Some(opts))
        })?,
    )?;

    // Revolve profiles around an axis (SPEC §3.5 Revolve): `axis = "x"|"y"|"z"` or
    // `{ line = i }` (construction/projected lines work); `angle` in degrees (default
    // 360); `symmetric` sweeps both ways; `body = "new"|"add"|"cut"` with `bodies`
    // for an explicit add/cut list ("add" with none auto-resolves touching bodies).
    api.set(
        "repeat_bodies",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(
                &opts,
                "repeat_bodies",
                &["bodies", "axis", "around", "flip", "mode", "count", "spacing", "gap", "length", "to", "name"],
            )?;
            let (targets, axis, around_axis, flip, mode, count, spacing, length, length_target) =
                parse_repeat_op_args(lua, &opts)?;
            unsafe {
                tick.exec(Instruction::CreateRepeatOp {
                    around_axis,
                    flip,
                    targets,
                    axis,
                    mode,
                    count,
                    spacing,
                    length,
                    length_target,
                })?;
            }
            let element = SceneElement::RepeatOp(unsafe {
                tick.state()
                    .doc
                    .repeat_ops
                    .keys()
                    .last()
                    .unwrap_or_else(|| crate::arena::Key::from_bits(u64::MAX))
            });
            drop(tick);
            apply_optional_name(lua, element, Some(opts))
        })?,
    )?;

    api.set(
        "edit_repeat",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(
                &opts,
                "edit_repeat",
                &["index", "bodies", "axis", "around", "flip", "mode", "count", "spacing", "gap", "length", "to"],
            )?;
            let op: usize = opts.get("index")?;
            let (targets, axis, around_axis, flip, mode, count, spacing, length, length_target) =
                parse_repeat_op_args(lua, &opts)?;
            unsafe {
                tick.exec(Instruction::EditRepeatOp {
                    around_axis,
                    flip,
                    op,
                    targets,
                    axis,
                    mode,
                    count,
                    spacing,
                    length,
                    length_target,
                })?;
            }
            Ok(())
        })?,
    )?;

    // 2D in-sketch linear repeat (#222): duplicate sketch lines/circles along an in-plane
    // direction. `sketch` selects the sketch; `lines`/`circles` are the operand index lists;
    // direction is `angle` (degrees, 0 = +u/x) or an explicit `dir = {du, dv}`; spacing uses the
    // same modes/expressions as `repeat_bodies`. Runs directly through the action (not the
    // command-log DSL), like the Move tool's plane/image targets.
    api.set(
        "repeat_sketch",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(
                &opts,
                "repeat_sketch",
                &["sketch", "lines", "circles", "angle", "dir", "mode", "count", "spacing", "gap", "length"],
            )?;
            let (sketch, lines, circles, dir_u, dir_v, mode, count, spacing, length) =
                parse_sketch_repeat_op_args(&opts)?;
            let sketch = sketch_key_from_ordinal(lua, sketch)?;
            let lines = line_keys_from_ordinals(lua, lines)?;
            let circles = circles
                .into_iter()
                .map(|o| circle_key_from_ordinal(lua, o))
                .collect::<mlua::Result<Vec<_>>>()?;
            let result = unsafe {
                tick.state().apply(crate::actions::Action::CreateSketchRepeatOperation {
                    sketch,
                    line_targets: lines,
                    circle_targets: circles,
                    dir_u,
                    dir_v,
                    mode,
                    count,
                    spacing,
                    length,
                })
            };
            if let crate::actions::ActionResult::Err(e) = result {
                return Err(mlua::Error::external(e));
            }
            Ok(())
        })?,
    )?;

    api.set(
        "edit_sketch_repeat",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(
                &opts,
                "edit_sketch_repeat",
                &["index", "sketch", "lines", "circles", "angle", "dir", "mode", "count", "spacing", "gap", "length"],
            )?;
            let op: usize = opts.get("index")?;
            let (_sketch, lines, circles, dir_u, dir_v, mode, count, spacing, length) =
                parse_sketch_repeat_op_args(&opts)?;
            let circles = circles
                .into_iter()
                .map(|o| circle_key_from_ordinal(lua, o))
                .collect::<mlua::Result<Vec<_>>>()?;
            let lines = line_keys_from_ordinals(lua, lines)?;
            let result = unsafe {
                tick.state().apply(crate::actions::Action::EditSketchRepeatOperation {
                    // A script names the op by its ordinal among the live ones (#1055).
                    op: tick
                        .state()
                        .doc
                        .sketch_repeat_ops
                        .keys()
                        .nth(op)
                        .ok_or_else(|| mlua::Error::external(format!("no operation {op}")))?,
                    line_targets: lines,
                    circle_targets: circles,
                    dir_u,
                    dir_v,
                    mode,
                    count,
                    spacing,
                    length,
                })
            };
            if let crate::actions::ActionResult::Err(e) = result {
                return Err(mlua::Error::external(e));
            }
            Ok(())
        })?,
    )?;

    // 2D in-sketch offset: parallel copies of sketch lines (mitered where they chain)
    // and concentric copies of circles at a signed distance. Positive grows a closed
    // loop / circle; negative shrinks (or flips an open chain's side). `construction`
    // emits the copies as construction geometry.
    api.set(
        "offset_sketch",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(
                &opts,
                "offset_sketch",
                &["sketch", "lines", "circles", "distance", "construction"],
            )?;
            let (sketch, lines, circles, distance, construction) =
                parse_sketch_offset_op_args(&opts)?;
            let sketch = sketch_key_from_ordinal(lua, sketch)?;
            let lines = line_keys_from_ordinals(lua, lines)?;
            let circles = circles
                .into_iter()
                .map(|o| circle_key_from_ordinal(lua, o))
                .collect::<mlua::Result<Vec<_>>>()?;
            let result = unsafe {
                tick.state().apply(crate::actions::Action::CreateSketchOffsetOperation {
                    sketch,
                    line_targets: lines,
                    circle_targets: circles,
                    distance,
                    construction,
                })
            };
            if let crate::actions::ActionResult::Err(e) = result {
                return Err(mlua::Error::external(e));
            }
            Ok(())
        })?,
    )?;

    api.set(
        "edit_sketch_offset",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(
                &opts,
                "edit_sketch_offset",
                &["index", "sketch", "lines", "circles", "distance", "construction"],
            )?;
            let op: usize = opts.get("index")?;
            let (_sketch, lines, circles, distance, construction) =
                parse_sketch_offset_op_args(&opts)?;
            let circles = circles
                .into_iter()
                .map(|o| circle_key_from_ordinal(lua, o))
                .collect::<mlua::Result<Vec<_>>>()?;
            let lines = line_keys_from_ordinals(lua, lines)?;
            let result = unsafe {
                tick.state().apply(crate::actions::Action::EditSketchOffsetOperation {
                    // A script names the op by its ordinal among the live ones (#1055).
                    op: tick
                        .state()
                        .doc
                        .sketch_offset_ops
                        .keys()
                        .nth(op)
                        .ok_or_else(|| mlua::Error::external(format!("no operation {op}")))?,
                    line_targets: lines,
                    circle_targets: circles,
                    distance,
                    construction,
                })
            };
            if let crate::actions::ActionResult::Err(e) = result {
                return Err(mlua::Error::external(e));
            }
            Ok(())
        })?,
    )?;

    // 2D in-sketch mirror (#523/#528): reflect sketch lines/circles across a straight `line`.
    api.set(
        "mirror_sketch",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(&opts, "mirror_sketch", &["sketch", "line", "lines", "circles"])?;
            let (sketch, line, lines, circles) = parse_sketch_mirror_op_args(&opts)?;
            let sketch = sketch_key_from_ordinal(lua, sketch)?;
            let lines = line_keys_from_ordinals(lua, lines)?;
            let circles = circles
                .into_iter()
                .map(|o| circle_key_from_ordinal(lua, o))
                .collect::<mlua::Result<Vec<_>>>()?;
            let result = unsafe {
                tick.state().apply(crate::actions::Action::CreateSketchMirrorOperation {
                    sketch,
                    line: line_key_from_ordinal(lua, line)?,
                    line_targets: lines,
                    circle_targets: circles,
                })
            };
            if let crate::actions::ActionResult::Err(e) = result {
                return Err(mlua::Error::external(e));
            }
            Ok(())
        })?,
    )?;

    api.set(
        "edit_sketch_mirror",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(
                &opts,
                "edit_sketch_mirror",
                &["index", "sketch", "line", "lines", "circles"],
            )?;
            let op: usize = opts.get("index")?;
            let (_sketch, line, lines, circles) = parse_sketch_mirror_op_args(&opts)?;
            let circles = circles
                .into_iter()
                .map(|o| circle_key_from_ordinal(lua, o))
                .collect::<mlua::Result<Vec<_>>>()?;
            let lines = line_keys_from_ordinals(lua, lines)?;
            let result = unsafe {
                tick.state().apply(crate::actions::Action::EditSketchMirrorOperation {
                    // A script names the op by its ordinal among the live ones (#1055).
                    op: tick
                        .state()
                        .doc
                        .sketch_mirror_ops
                        .keys()
                        .nth(op)
                        .ok_or_else(|| mlua::Error::external(format!("no operation {op}")))?,
                    line: line_key_from_ordinal(lua, line)?,
                    line_targets: lines,
                    circle_targets: circles,
                })
            };
            if let crate::actions::ActionResult::Err(e) = result {
                return Err(mlua::Error::external(e));
            }
            Ok(())
        })?,
    )?;

    // Repeat-operation replay (#220): replay a cut extrusion's effect along an axis, punching N
    // holes. `cuts` are the cut-extrusion indices; axis/mode/count/spacing/length as repeat_bodies.
    api.set(
        "repeat_cut",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let cuts = opts
                .get::<Option<Vec<usize>>>("cuts")?
                .unwrap_or_default()
                .into_iter()
                .map(|ordinal| extrusion_key_from_ordinal(lua, ordinal))
                .collect::<mlua::Result<Vec<_>>>()?;
            let (_targets, axis, around_axis, flip, mode, count, spacing, length, length_target) =
                parse_repeat_op_args(lua, &opts)?;
            let result = unsafe {
                tick.state().apply(crate::actions::Action::CreateRepeatOperation {
                    path_circle: None,
                    around_axis,
                    flip,
                    targets: Vec::new(),
                    plane_targets: Vec::new(),
                    extrusion_targets: cuts,
                    sketch_targets: Vec::new(),
                    axis,
                    mode,
                    count,
                    spacing,
                    length,
                    length_target,
                })
            };
            if let crate::actions::ActionResult::Err(e) = result {
                return Err(mlua::Error::external(e));
            }
            Ok(())
        })?,
    )?;

    // Repeat whole sketches along an axis (#226): `sketches` are construction-plane-hosted sketch
    // indices; each is copied at every offset onto a parallel offset plane. axis/mode/etc. as
    // repeat_bodies.
    api.set(
        "repeat_sketches",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let sketches = opts
                .get::<Option<Vec<usize>>>("sketches")?
                .unwrap_or_default()
                .into_iter()
                .map(|ordinal| sketch_key_from_ordinal(lua, ordinal))
                .collect::<mlua::Result<Vec<_>>>()?;
            let (_targets, axis, around_axis, flip, mode, count, spacing, length, length_target) =
                parse_repeat_op_args(lua, &opts)?;
            let result = unsafe {
                tick.state().apply(crate::actions::Action::CreateRepeatOperation {
                    path_circle: None,
                    around_axis,
                    flip,
                    targets: Vec::new(),
                    plane_targets: Vec::new(),
                    extrusion_targets: Vec::new(),
                    sketch_targets: sketches,
                    axis,
                    mode,
                    count,
                    spacing,
                    length,
                    length_target,
                })
            };
            if let crate::actions::ActionResult::Err(e) = result {
                return Err(mlua::Error::external(e));
            }
            Ok(())
        })?,
    )?;

    // 2D in-sketch slice (#224): split `lines` where `cutters` cross them. `sketch` selects the
    // sketch; both lists are line index lists. Runs directly through the action.
    api.set(
        "slice_sketch",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(
                &opts,
                "slice_sketch",
                &["sketch", "lines", "circles", "faces", "cutters"],
            )?;
            let sketch = sketch_key_from_ordinal(lua, opts.get::<Option<usize>>("sketch")?.unwrap_or(0))?;
            let line_targets =
                line_keys_from_ordinals(lua, opts.get::<Option<Vec<usize>>>("lines")?.unwrap_or_default())?;
            let circle_targets = opts
                .get::<Option<Vec<usize>>>("circles")?
                .unwrap_or_default()
                .into_iter()
                .map(|o| circle_key_from_ordinal(lua, o))
                .collect::<mlua::Result<Vec<_>>>()?;
            let face_targets = opts
                .get::<Option<Vec<Vec<usize>>>>("faces")?
                .unwrap_or_default()
                .into_iter()
                .map(|loop_lines| line_keys_from_ordinals(lua, loop_lines))
                .collect::<mlua::Result<Vec<_>>>()?;
            let cutter_lines = line_keys_from_ordinals(
                lua,
                opts.get::<Option<Vec<usize>>>("cutters")?.unwrap_or_default(),
            )?;
            let result = unsafe {
                tick.state().apply(crate::actions::Action::CreateSketchSliceOperation {
                    sketch,
                    line_targets,
                    circle_targets,
                    face_targets,
                    cutter_lines,
                })
            };
            if let crate::actions::ActionResult::Err(e) = result {
                return Err(mlua::Error::external(e));
            }
            Ok(())
        })?,
    )?;

    api.set(
        "edit_sketch_slice",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(
                &opts,
                "edit_sketch_slice",
                &["index", "lines", "circles", "faces", "cutters"],
            )?;
            let op: usize = opts.get("index")?;
            let line_targets =
                line_keys_from_ordinals(lua, opts.get::<Option<Vec<usize>>>("lines")?.unwrap_or_default())?;
            let circle_targets = opts
                .get::<Option<Vec<usize>>>("circles")?
                .unwrap_or_default()
                .into_iter()
                .map(|o| circle_key_from_ordinal(lua, o))
                .collect::<mlua::Result<Vec<_>>>()?;
            let face_targets = opts
                .get::<Option<Vec<Vec<usize>>>>("faces")?
                .unwrap_or_default()
                .into_iter()
                .map(|loop_lines| line_keys_from_ordinals(lua, loop_lines))
                .collect::<mlua::Result<Vec<_>>>()?;
            let cutter_lines = line_keys_from_ordinals(
                lua,
                opts.get::<Option<Vec<usize>>>("cutters")?.unwrap_or_default(),
            )?;
            let result = unsafe {
                tick.state().apply(crate::actions::Action::EditSketchSliceOperation {
                    // A script names the op by its ordinal among the live ones (#1055).
                    op: tick
                        .state()
                        .doc
                        .sketch_slice_ops
                        .keys()
                        .nth(op)
                        .ok_or_else(|| mlua::Error::external(format!("no operation {op}")))?,
                    line_targets,
                    circle_targets,
                    face_targets,
                    cutter_lines,
                })
            };
            if let crate::actions::ActionResult::Err(e) = result {
                return Err(mlua::Error::external(e));
            }
            Ok(())
        })?,
    )?;

    api.set(
        "move_bodies",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let (targets, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin,
                 face_offset, start_point_a, end_point_a, start_point_b, end_point_b,
                 start_point_c, end_point_c) =
                parse_move_op_args(lua, &opts)?;
            unsafe {
                tick.exec(Instruction::CreateMoveOp {
                    targets, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin,
                    face_offset, start_point_a, end_point_a, start_point_b, end_point_b,
                    start_point_c, end_point_c,
                })?;
            }
            let element = SceneElement::MoveOp(unsafe {
                tick.state()
                    .doc
                    .move_ops
                    .keys()
                    .last()
                    .unwrap_or_else(|| crate::arena::Key::from_bits(u64::MAX))
            });
            drop(tick);
            apply_optional_name(lua, element, Some(opts))
        })?,
    )?;

    // Arm the Move tool with a set of picks without committing them, so a script can show
    // the tool's live preview — the ghost, the A connector, the B and C paths.
    api.set(
        "begin_move",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let (targets, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin,
                 face_offset, start_point_a, end_point_a, start_point_b, end_point_b,
                 start_point_c, end_point_c) =
                parse_move_op_args(lua, &opts)?;
            unsafe {
                tick.exec(Instruction::BeginMoveOp {
                    targets, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin,
                    face_offset, start_point_a, end_point_a, start_point_b, end_point_b,
                    start_point_c, end_point_c,
                })?;
            }
            Ok(())
        })?,
    )?;

    api.set(
        "edit_move",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let op: usize = opts.get("index")?;
            let (targets, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin,
                 face_offset, start_point_a, end_point_a, start_point_b, end_point_b,
                 start_point_c, end_point_c) =
                parse_move_op_args(lua, &opts)?;
            unsafe {
                tick.exec(Instruction::EditMoveOp {
                    op, targets, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin,
                    face_offset, start_point_a, end_point_a, start_point_b, end_point_b,
                    start_point_c, end_point_c,
                })?;
            }
            Ok(())
        })?,
    )?;

    api.set(
        "joint",
        lua.create_function(|lua, opts: Table| {
            let (members, base, kind, placement, frame, position, position2, position3, limits) =
                parse_joint_op_args(lua, &opts, "joint")?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe {
                tick.exec(Instruction::CreateJointOp {
                    members, base, kind, placement, frame, position, position2, position3, limits,
                })?;
            }
            let element = SceneElement::Joint(unsafe {
                tick.state()
                    .doc
                    .joints
                    .keys()
                    .last()
                    .unwrap_or_else(|| crate::arena::Key::from_bits(u64::MAX))
            });
            drop(tick);
            apply_optional_name(lua, element, Some(opts))
        })?,
    )?;

    // Arm the Joint tool with a set of picks without committing them (#894), so a script
    // can shoot the tool's live preview — the counterpart begin_move gives the Move tool.
    api.set(
        "begin_joint",
        lua.create_function(|lua, opts: Table| {
            let (members, base, kind, placement, frame, position, position2, position3, limits) =
                parse_joint_op_args(lua, &opts, "begin_joint")?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe {
                tick.exec(Instruction::BeginJointOp {
                    members, base, kind, placement, frame, position, position2, position3, limits,
                })?;
            }
            Ok(())
        })?,
    )?;

    api.set(
        "edit_joint",
        lua.create_function(|lua, opts: Table| {
            let op: usize = opts.get("index")?;
            let (members, base, kind, placement, frame, position, position2, position3, limits) =
                parse_joint_op_args(lua, &opts, "edit_joint")?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe {
                tick.exec(Instruction::EditJointOp {
                    op, members, base, kind, placement, frame, position, position2, position3, limits,
                })?;
            }
            Ok(())
        })?,
    )?;

    api.set(
        "set_joint_rest",
        lua.create_function(|lua, op: usize| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::SetJointRest { op }) }
        })?,
    )?;

    api.set(
        "revert_joint",
        lua.create_function(|lua, op: usize| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::RevertJoint { op }) }
        })?,
    )?;

    api.set(
        "revert_joints",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::RevertAllJoints) }
        })?,
    )?;

    api.set(
        "mirror_bodies",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(&opts, "mirror_bodies", &["plane", "bodies", "output", "name"])?;
            let (plane, targets, mode) = parse_mirror_op_args(lua, &opts)?;
            unsafe {
                tick.exec(Instruction::CreateMirrorOp { plane, targets, mode })?;
            }
            let element = SceneElement::MirrorOp(unsafe {
                tick.state()
                    .doc
                    .mirror_ops
                    .keys()
                    .last()
                    .unwrap_or_else(|| crate::arena::Key::from_bits(u64::MAX))
            });
            drop(tick);
            apply_optional_name(lua, element, Some(opts))
        })?,
    )?;

    api.set(
        "edit_mirror",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(&opts, "edit_mirror", &["index", "plane", "bodies", "output"])?;
            let op: usize = opts.get("index")?;
            let (plane, targets, mode) = parse_mirror_op_args(lua, &opts)?;
            unsafe {
                tick.exec(Instruction::EditMirrorOp { op, plane, targets, mode })?;
            }
            Ok(())
        })?,
    )?;

    api.set(
        "combine",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(&opts, "combine", &["op", "a", "b", "keep_b", "name"])?;
            let (kind, a, b, keep_b) = parse_boolean_op_args(&opts)?;
            unsafe {
                tick.exec(Instruction::CreateBooleanOp { kind, a, b, keep_b })?;
            }
            let element = SceneElement::BooleanOp(unsafe {
                tick.state().doc.boolean_ops.keys().last().unwrap_or_else(|| crate::arena::Key::from_bits(u64::MAX))
            });
            drop(tick);
            apply_optional_name(lua, element, Some(opts))
        })?,
    )?;

    // Arm the Combine tool with picked sides without committing them, so a script can show
    // the tool's live result preview (#1033) — the counterpart begin_move gives Move.
    api.set(
        "begin_combine",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(&opts, "begin_combine", &["op", "a", "b", "keep_b"])?;
            let (kind, a, b, keep_b) = parse_boolean_op_args(&opts)?;
            unsafe {
                tick.exec(Instruction::BeginBooleanOp { kind, a, b, keep_b })?;
            }
            Ok(())
        })?,
    )?;

    api.set(
        "edit_boolean",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(&opts, "edit_boolean", &["index", "op", "a", "b", "keep_b"])?;
            let op: usize = opts.get("index")?;
            let (kind, a, b, keep_b) = parse_boolean_op_args(&opts)?;
            unsafe {
                tick.exec(Instruction::EditBooleanOp { op, kind, a, b, keep_b })?;
            }
            Ok(())
        })?,
    )?;

    api.set(
        "slice",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(&opts, "slice", &["bodies", "cutters", "extend", "name"])?;
            let (targets, cutters, extend_infinite) = parse_slice_op_args(lua, &opts)?;
            unsafe {
                tick.exec(Instruction::CreateSliceOp { targets, cutters, extend_infinite })?;
            }
            let element = SceneElement::SliceOp(unsafe {
                tick.state()
                    .doc
                    .slice_ops
                    .keys()
                    .last()
                    .unwrap_or_else(|| crate::arena::Key::from_bits(u64::MAX))
            });
            drop(tick);
            apply_optional_name(lua, element, Some(opts))
        })?,
    )?;

    api.set(
        "edit_slice",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(&opts, "edit_slice", &["index", "bodies", "cutters", "extend"])?;
            let op: usize = opts.get("index")?;
            let (targets, cutters, extend_infinite) = parse_slice_op_args(lua, &opts)?;
            unsafe {
                tick.exec(Instruction::EditSliceOp { op, targets, cutters, extend_infinite })?;
            }
            Ok(())
        })?,
    )?;

    api.set(
        "shell",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(&opts, "shell", &["bodies", "faces", "thickness", "name"])?;
            let (targets, open_faces, thickness) = parse_shell_op_args(lua, &opts)?;
            unsafe {
                tick.exec(Instruction::CreateShellOp {
                    targets,
                    open_faces,
                    thickness,
                })?;
            }
            let element = SceneElement::ShellOp(unsafe {
                tick.state()
                    .doc
                    .shell_ops
                    .keys()
                    .last()
                    .unwrap_or_else(|| crate::arena::Key::from_bits(u64::MAX))
            });
            drop(tick);
            apply_optional_name(lua, element, Some(opts))
        })?,
    )?;

    api.set(
        "edit_shell",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(&opts, "edit_shell", &["index", "bodies", "faces", "thickness"])?;
            let op: usize = opts.get("index")?;
            let (targets, open_faces, thickness) = parse_shell_op_args(lua, &opts)?;
            unsafe {
                tick.exec(Instruction::EditShellOp {
                    op,
                    targets,
                    open_faces,
                    thickness,
                })?;
            }
            Ok(())
        })?,
    )?;

    // Project outside 3D geometry into the active sketch (#1351): the declarative
    // equivalent of the Project tool + Enter. Empty / omitted sources use the current
    // selection (and un-project when that selection is only already-projected lines).
    api.set(
        "project",
        lua.create_function(|lua, opts: Option<Table>| {
            let elements = parse_project_elements(lua, opts)?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Project { elements }) }
        })?,
    )?;

    api.set(
        "revolve",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let mut faces: Vec<crate::model::ExtrudeFace> = Vec::new();
            if let Some(i) = opts.get::<Option<usize>>("circle")? {
                faces.push(crate::model::ExtrudeFace::Circle(circle_key_from_ordinal(lua, i)?));
            }
            if let Some(list) = opts.get::<Option<Vec<usize>>>("circles")? {
                for i in list {
                    faces
                        .push(crate::model::ExtrudeFace::Circle(circle_key_from_ordinal(lua, i)?));
                }
            }
            if let Some(lines) = opts.get::<Option<Vec<usize>>>("polygon")? {
                faces.push(crate::model::ExtrudeFace::Polygon(line_keys_from_ordinals(lua, lines)?));
            }
            if faces.is_empty() {
                return Err(mlua::Error::external(
                    "revolve requires a `circle`/`circles`/`polygon` face",
                ));
            }
            let axis = parse_revolve_axis(lua, opts.get::<mlua::Value>("axis")?, "revolve")?;
            // #1242: `revolutions` (turns) wins over `angle` (degrees) when both are set.
            let angle_deg: f32 = if let Some(turns) = opts.get::<Option<f32>>("revolutions")? {
                turns * 360.0
            } else {
                opts.get::<Option<f32>>("angle")?.unwrap_or(360.0)
            };
            // Helical pitch (mm per full turn): `pitch` / `offset` preferred; `gap` as alias.
            let pitch_mm: f32 = opts
                .get::<Option<f32>>("pitch")?
                .or(opts.get::<Option<f32>>("offset")?)
                .or(opts.get::<Option<f32>>("gap")?)
                .unwrap_or(0.0);
            let symmetric: bool = opts.get::<Option<bool>>("symmetric")?.unwrap_or(false);
            let bodies: Vec<usize> = opts.get::<Option<Vec<usize>>>("bodies")?.unwrap_or_default();
            let body = match opts.get::<Option<String>>("body")?.as_deref() {
                Some("add") => crate::actions::RevolveBodyChoice::AddTouching,
                Some("cut") => crate::actions::RevolveBodyChoice::Cut,
                _ => crate::actions::RevolveBodyChoice::NewBody,
            };
            unsafe {
                tick.exec(Instruction::Revolve {
                    faces,
                    axis,
                    angle_deg,
                    pitch_mm,
                    symmetric,
                    body,
                    bodies,
                })?;
            }
            let key = unsafe { tick.state().doc.bodies.keys().last() };
            let element =
                SceneElement::Body(key.ok_or_else(|| mlua::Error::external("no body was made"))?);
            apply_optional_name(lua, element, Some(opts))
        })?,
    )?;

    // Primitive shapes (#909): `bearcad.cuboid{ at = {x,y,z}?, normal = {..}?, u_axis = {..}?,
    // width =, depth =, height =, name = }`, `bearcad.cylinder{ radius =, height = }`,
    // `bearcad.sphere{ radius = }`. Every dimension takes a number or an expression string;
    // `at` defaults to the origin and `normal` to +Z (the ground), so the simplest call is
    // just the sizes. `bearcad.edit_shape{ index =, shape = "cuboid", ... }` re-points one.
    for (call, kind) in [
        ("cuboid", crate::model::PrimitiveKind::Cuboid),
        ("cylinder", crate::model::PrimitiveKind::Cylinder),
        ("sphere", crate::model::PrimitiveKind::Sphere),
    ] {
        api.set(
            call,
            lua.create_function(move |lua, opts: Option<Table>| {
                let opts = match opts {
                    Some(t) => t,
                    None => lua.create_table()?,
                };
                let shape = parse_shape_args(lua, &opts, kind, call)?;
                let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
                unsafe { tick.exec(Instruction::Shape { shape })? };
                let key = unsafe { tick.state().doc.primitives.keys().last() };
                let element = SceneElement::Shape(
                    key.ok_or_else(|| mlua::Error::external("shape was not created"))?,
                );
                drop(tick);
                apply_optional_name(lua, element, Some(opts))
            })?,
        )?;
    }

    api.set(
        "edit_shape",
        lua.create_function(|lua, opts: Table| {
            check_shape_keys(&opts, "edit_shape")?;
            // The script's `index` is the shape's ordinal among the live ones (#1055).
            let ordinal: usize = opts.get("index")?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let index = unsafe { tick.state().doc.primitives.keys().nth(ordinal) }
                .ok_or_else(|| mlua::Error::external(format!("no shape {ordinal}")))?;
            let existing = unsafe { tick.state().doc.primitives.get(index).cloned() };
            drop(tick);
            let kind = match opts.get::<Option<String>>("shape")? {
                Some(name) => crate::model::PrimitiveKind::from_name(&name).ok_or_else(|| {
                    mlua::Error::external(format!(
                        "unknown shape '{name}' (cuboid|cylinder|sphere)"
                    ))
                })?,
                None => existing
                    .as_ref()
                    .map(|s| s.kind)
                    .ok_or_else(|| mlua::Error::external(format!("no shape {ordinal}")))?,
            };
            let mut shape = parse_shape_args(lua, &opts, kind, "edit_shape")?;
            // Unmentioned dimensions keep what the shape already had.
            if let Some(old) = existing {
                if !opts.contains_key("width")? { shape.width = old.width.clone(); }
                if !opts.contains_key("depth")? { shape.depth = old.depth.clone(); }
                if !opts.contains_key("height")? { shape.height = old.height.clone(); }
                if !opts.contains_key("radius")? { shape.radius = old.radius.clone(); }
                if !opts.contains_key("at")? { shape.origin = old.origin; }
                if !opts.contains_key("normal")? { shape.normal = old.normal; }
                if !opts.contains_key("u_axis")? { shape.u_axis = old.u_axis; }
            }
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::EditShape { index: ordinal, shape })? };
            drop(tick);
            apply_optional_name(lua, SceneElement::Shape(index), Some(opts))
        })?,
    )?;

    // Sweep profiles along a path of sketch lines (SPEC §3.5 Sweep):
    // `bearcad.sweep{ circles = {i, ...} and/or polygon = {line, ...},
    // path = {line, ...}, body = "add"|"cut"?, bodies = {i, ...}? }`. Each face's sketch
    // is inferred like `extrude`'s; the path lines are chained tip-to-tail.
    api.set(
        "sweep",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let mut faces: Vec<crate::model::ExtrudeFace> = Vec::new();
            if let Some(i) = opts.get::<Option<usize>>("circle")? {
                faces.push(crate::model::ExtrudeFace::Circle(circle_key_from_ordinal(lua, i)?));
            }
            if let Some(list) = opts.get::<Option<Vec<usize>>>("circles")? {
                for i in list {
                    faces
                        .push(crate::model::ExtrudeFace::Circle(circle_key_from_ordinal(lua, i)?));
                }
            }
            if let Some(lines) = opts.get::<Option<Vec<usize>>>("polygon")? {
                faces.push(crate::model::ExtrudeFace::Polygon(line_keys_from_ordinals(lua, lines)?));
            }
            if faces.is_empty() {
                return Err(mlua::Error::external(
                    "sweep requires a `circle`/`circles`/`polygon` face",
                ));
            }
            let path =
                line_keys_from_ordinals(lua, opts.get::<Option<Vec<usize>>>("path")?.unwrap_or_default())?;
            if path.is_empty() {
                return Err(mlua::Error::external(
                    "sweep requires `path` (a list of line indices)",
                ));
            }
            let bodies: Vec<usize> = opts.get::<Option<Vec<usize>>>("bodies")?.unwrap_or_default();
            let body = match opts.get::<Option<String>>("body")?.as_deref() {
                Some("add") => crate::actions::RevolveBodyChoice::AddTouching,
                Some("cut") => crate::actions::RevolveBodyChoice::Cut,
                _ => crate::actions::RevolveBodyChoice::NewBody,
            };
            unsafe {
                tick.exec(Instruction::Sweep { faces, path, body, bodies })?;
            }
            let key = unsafe { tick.state().doc.bodies.keys().last() };
            let element =
                SceneElement::Body(key.ok_or_else(|| mlua::Error::external("no body was made"))?);
            apply_optional_name(lua, element, Some(opts))
        })?,
    )?;

    // Loft a solid through two or more closed cross-section profiles (SPEC §3.5).
    // `circles = {i, ...}` and/or `polygons = {{line, ...}, ...}` list the sections
    // (singular `circle`/`polygon` also accepted); each face's sketch is inferred like
    // `extrude`'s. Section order along the loft is recovered from the geometry.
    api.set(
        "loft",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let mut faces: Vec<crate::model::ExtrudeFace> = Vec::new();
            if let Some(i) = opts.get::<Option<usize>>("circle")? {
                faces.push(crate::model::ExtrudeFace::Circle(circle_key_from_ordinal(lua, i)?));
            }
            if let Some(list) = opts.get::<Option<Vec<usize>>>("circles")? {
                for i in list {
                    faces
                        .push(crate::model::ExtrudeFace::Circle(circle_key_from_ordinal(lua, i)?));
                }
            }
            if let Some(lines) = opts.get::<Option<Vec<usize>>>("polygon")? {
                faces.push(crate::model::ExtrudeFace::Polygon(line_keys_from_ordinals(lua, lines)?));
            }
            if let Some(loops) = opts.get::<Option<Vec<Vec<usize>>>>("polygons")? {
                for loop_lines in loops {
                    faces.push(crate::model::ExtrudeFace::Polygon(line_keys_from_ordinals(
                        lua, loop_lines,
                    )?));
                }
            }
            if faces.len() < 2 {
                return Err(mlua::Error::external(
                    "loft requires at least two sections (`circles`/`polygons`)",
                ));
            }
            let bodies: Vec<usize> = opts.get::<Option<Vec<usize>>>("bodies")?.unwrap_or_default();
            let body = match opts.get::<Option<String>>("body")?.as_deref() {
                Some("add") => crate::actions::RevolveBodyChoice::AddTouching,
                Some("cut") => crate::actions::RevolveBodyChoice::Cut,
                _ => crate::actions::RevolveBodyChoice::NewBody,
            };
            unsafe {
                tick.exec(Instruction::Loft { faces, body, bodies })?;
            }
            let key = unsafe { tick.state().doc.bodies.keys().last() };
            let element =
                SceneElement::Body(key.ok_or_else(|| mlua::Error::external("no body was made"))?);
            apply_optional_name(lua, element, Some(opts))
        })?,
    )?;

    // Technical drawings (#180): `bearcad.drawing{ name? }` creates a drawing (and opens its
    // pane), returning its index; `bearcad.drawing_view{ drawing, body|bodies|component|sketch,
    // orientation? }` adds a projection. Multi-body and whole-component views are #1190/#1191.
    api.set(
        "drawing",
        lua.create_function(|lua, opts: Option<Table>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let name: Option<String> = match &opts {
                Some(t) => t.get("name")?,
                None => None,
            };
            unsafe {
                tick.exec(Instruction::CreateDrawing { name })?;
            }
            Ok(unsafe { tick.state().doc.drawings.len().saturating_sub(1) })
        })?,
    )?;
    api.set(
        "drawing_view",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(
                &opts,
                "drawing_view",
                &["drawing", "body", "bodies", "component", "sketch", "orientation"],
            )?;
            let drawing: usize = opts.get("drawing")?;
            let orientation = match opts.get::<Option<String>>("orientation")? {
                Some(name) => crate::model::DrawingOrientation::from_name(&name).ok_or_else(|| {
                    mlua::Error::external(format!("unknown drawing orientation '{name}'"))
                })?,
                None => crate::model::DrawingOrientation::default(),
            };
            // A view projects a body, several bodies, a component, or a sketch (#278/#403/#1190/#1191).
            let body: Option<usize> = opts.get("body")?;
            let bodies: Option<Vec<usize>> = opts.get("bodies")?;
            let component: Option<usize> = opts.get("component")?;
            let sketch: Option<usize> = opts.get("sketch")?;
            let source_count = usize::from(body.is_some())
                + usize::from(bodies.is_some())
                + usize::from(component.is_some())
                + usize::from(sketch.is_some());
            if source_count != 1 {
                return Err(mlua::Error::external(
                    "drawing_view requires exactly one of `body`, `bodies`, `component`, or `sketch`",
                ));
            }
            unsafe {
                if let Some(sketch) = sketch {
                    return tick.exec(Instruction::AddDrawingSketchView {
                        drawing,
                        sketch,
                        orientation,
                    });
                }
                let bodies = if let Some(body) = body {
                    vec![body]
                } else if let Some(bodies) = bodies {
                    if bodies.is_empty() {
                        return Err(mlua::Error::external("`bodies` must not be empty"));
                    }
                    bodies
                } else {
                    let ci = component.expect("component set");
                    let state = tick.state();
                    let Some(ck) = state.doc.components.keys().nth(ci) else {
                        return Err(mlua::Error::external(format!("No component {ci}")));
                    };
                    let bodies: Vec<usize> = state
                        .component_body_indices(ck)
                        .into_iter()
                        .filter_map(|bk| state.doc.bodies.keys().position(|k| k == bk))
                        .collect();
                    if bodies.is_empty() {
                        return Err(mlua::Error::external(
                            "This component has no bodies to project",
                        ));
                    }
                    bodies
                };
                tick.exec(Instruction::AddDrawingView {
                    drawing,
                    bodies,
                    orientation,
                })
            }
        })?,
    )?;
    // Append bodies to an existing projection (#1191) — the scripted form of shift-click.
    api.set(
        "drawing_view_add",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(
                &opts,
                "drawing_view_add",
                &["drawing", "view", "body", "bodies", "component"],
            )?;
            let drawing: usize = opts.get("drawing")?;
            let view: usize = opts.get("view")?;
            let body: Option<usize> = opts.get("body")?;
            let bodies: Option<Vec<usize>> = opts.get("bodies")?;
            let component: Option<usize> = opts.get("component")?;
            let source_count = usize::from(body.is_some())
                + usize::from(bodies.is_some())
                + usize::from(component.is_some());
            if source_count != 1 {
                return Err(mlua::Error::external(
                    "drawing_view_add requires exactly one of `body`, `bodies`, or `component`",
                ));
            }
            unsafe {
                let bodies = if let Some(body) = body {
                    vec![body]
                } else if let Some(bodies) = bodies {
                    if bodies.is_empty() {
                        return Err(mlua::Error::external("`bodies` must not be empty"));
                    }
                    bodies
                } else {
                    let ci = component.expect("component set");
                    let state = tick.state();
                    let Some(ck) = state.doc.components.keys().nth(ci) else {
                        return Err(mlua::Error::external(format!("No component {ci}")));
                    };
                    let bodies: Vec<usize> = state
                        .component_body_indices(ck)
                        .into_iter()
                        .filter_map(|bk| state.doc.bodies.keys().position(|k| k == bk))
                        .collect();
                    if bodies.is_empty() {
                        return Err(mlua::Error::external(
                            "This component has no bodies to project",
                        ));
                    }
                    bodies
                };
                tick.exec(Instruction::AddBodiesToDrawingView {
                    drawing,
                    view,
                    bodies,
                })
            }
        })?,
    )?;

    // Set a drawing's page size and margin in millimetres (#406) — the scripted page-settings
    // editor. Omitted keys keep the drawing's current value, so partial updates work.
    api.set(
        "drawing_page",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(&opts, "drawing_page", &["drawing", "width", "height", "margin"])?;
            let drawing: usize = opts.get("drawing")?;
            unsafe {
                tick.exec(Instruction::SetDrawingPage {
                    drawing,
                    width_mm: opts.get::<Option<f32>>("width")?,
                    height_mm: opts.get::<Option<f32>>("height")?,
                    margin_mm: opts.get::<Option<f32>>("margin")?,
                })
            }
        })?,
    )?;

    // Export a technical drawing to a vector SVG file (#180) — prints to PDF via any print
    // dialog. `bearcad.export_drawing_svg{ drawing, path }`.
    api.set(
        "export_drawing_svg",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let drawing: usize = opts.get("drawing")?;
            let path: String = opts.get("path")?;
            unsafe { tick.exec(Instruction::ExportDrawingSvg { drawing, path }) }
        })?,
    )?;

    // Export a technical drawing to a single-page vector PDF file (#180).
    // `bearcad.export_drawing_pdf{ drawing, path }`.
    api.set(
        "export_drawing_pdf",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let drawing: usize = opts.get("drawing")?;
            let path: String = opts.get("path")?;
            unsafe { tick.exec(Instruction::ExportDrawingPdf { drawing, path }) }
        })?,
    )?;

    // Toggle a view's edge length dimension (#180): the edge is named by its two world
    // endpoints `a`/`b` (`{x, y, z}`), matched to the body's projected feature edge.
    api.set(
        "drawing_move_view",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let drawing: usize = opts.get("drawing")?;
            let view: usize = opts.get("view")?;
            let x: f32 = opts.get("x")?;
            let y: f32 = opts.get("y")?;
            unsafe { tick.exec(Instruction::MoveDrawingView { drawing, view, x, y }) }
        })?,
    )?;

    // Resize a projection card (page fractions) (#1207). Omitted width/height keep the
    // current value; linked aligned views share the matching axis.
    api.set(
        "drawing_view_size",
        lua.create_function(|lua, opts: Table| {
            check_keys(
                &opts,
                "drawing_view_size",
                &["drawing", "view", "width", "height", "size_x", "size_y"],
            )?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let drawing: usize = opts.get("drawing")?;
            let view: usize = opts.get("view")?;
            let (cur_x, cur_y) = {
                let state = unsafe { tick.state() };
                state
                    .doc
                    .drawings
                    .keys()
                    .nth(drawing)
                    .and_then(|dkey| state.doc.drawings.get(dkey))
                    .and_then(|d| d.views.get(view))
                    .map(|v| (v.size_x, v.size_y))
                    .unwrap_or((crate::drawing::CELL_FRAC, crate::drawing::CELL_FRAC))
            };
            let size_x: f32 = opts
                .get::<Option<f32>>("width")?
                .or(opts.get::<Option<f32>>("size_x")?)
                .unwrap_or(cur_x);
            let size_y: f32 = opts
                .get::<Option<f32>>("height")?
                .or(opts.get::<Option<f32>>("size_y")?)
                .unwrap_or(cur_y);
            unsafe {
                tick.exec(Instruction::SetDrawingViewSize {
                    drawing,
                    view,
                    size_x,
                    size_y,
                })
            }
        })?,
    )?;

    // Add a free text annotation to a drawing page (#312), positioned by page fraction.
    api.set(
        "drawing_text",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let drawing: usize = opts.get("drawing")?;
            let text: String = opts.get("text")?;
            let x: f32 = opts.get::<Option<f32>>("x")?.unwrap_or(0.1);
            let y: f32 = opts.get::<Option<f32>>("y")?.unwrap_or(0.1);
            let wrap: Option<f32> = opts.get("wrap")?;
            unsafe { tick.exec(Instruction::AddDrawingAnnotation { drawing, text, x, y, wrap }) }
        })?,
    )?;

    // Add an aligned child projection (#296): `dir` is "below"/"above"/"right"/"left".
    api.set(
        "drawing_align_view",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let drawing: usize = opts.get("drawing")?;
            let parent: usize = opts.get("parent")?;
            let name: String = opts.get("dir")?;
            let dir = match name.to_ascii_lowercase().as_str() {
                "below" | "down" | "bottom" => crate::model::AlignDir::Below,
                "above" | "up" | "top" => crate::model::AlignDir::Above,
                "right" => crate::model::AlignDir::Right,
                "left" => crate::model::AlignDir::Left,
                other => {
                    return Err(mlua::Error::external(format!(
                        "unknown align dir '{other}' (below/above/right/left)"
                    )))
                }
            };
            let pos: f32 = opts.get::<Option<f32>>("pos")?.unwrap_or(0.5);
            unsafe { tick.exec(Instruction::AddAlignedDrawingView { drawing, parent, dir, pos }) }
        })?,
    )?;

    // Toggle a view edge's length dimension by its two world endpoints (#180).
    api.set(
        "drawing_dimension",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let drawing: usize = opts.get("drawing")?;
            let view: usize = opts.get("view")?;
            let point = |key: &str| -> mlua::Result<(f32, f32, f32)> {
                let v: Vec<f32> = opts.get(key)?;
                if v.len() != 3 {
                    return Err(mlua::Error::external(format!(
                        "drawing_dimension `{key}` must be a {{x, y, z}} point"
                    )));
                }
                Ok((v[0], v[1], v[2]))
            };
            let a = point("a")?;
            let b = point("b")?;
            unsafe {
                tick.exec(Instruction::ToggleDrawingDimension {
                    drawing,
                    view,
                    a,
                    b,
                })
            }
        })?,
    )?;

    // Set (or clear) a drawing edge dim label's offset (#294/#1228).
    // `bearcad.drawing_dim_offset{ drawing, view, a, b, offset }` — omit/nil `offset` clears.
    api.set(
        "drawing_dim_offset",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let drawing: usize = opts.get("drawing")?;
            let view: usize = opts.get("view")?;
            let point = |key: &str| -> mlua::Result<(f32, f32, f32)> {
                let v: Vec<f32> = opts.get(key)?;
                if v.len() != 3 {
                    return Err(mlua::Error::external(format!(
                        "drawing_dim_offset `{key}` must be a {{x, y, z}} point"
                    )));
                }
                Ok((v[0], v[1], v[2]))
            };
            let a = point("a")?;
            let b = point("b")?;
            let offset: Option<f32> = opts.get("offset")?;
            unsafe {
                tick.exec(Instruction::SetDrawingDimensionOffset {
                    drawing,
                    view,
                    a,
                    b,
                    offset,
                })
            }
        })?,
    )?;

    // Set (or clear) a drawing circle Ø-label offset (#397/#1228).
    api.set(
        "drawing_circle_dim_offset",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let drawing: usize = opts.get("drawing")?;
            let view: usize = opts.get("view")?;
            let c: Vec<f32> = opts.get("center")?;
            if c.len() != 3 {
                return Err(mlua::Error::external(
                    "drawing_circle_dim_offset `center` must be a {x, y, z} point",
                ));
            }
            let offset: Option<f32> = opts.get("offset")?;
            unsafe {
                tick.exec(Instruction::SetDrawingCircleDimOffset {
                    drawing,
                    view,
                    center: (c[0], c[1], c[2]),
                    offset,
                })
            }
        })?,
    )?;

    // Show/hide an aligned child's dashed projection lines to its base view (#377):
    // `bearcad.drawing_view_align_lines{ drawing, view, show }`.
    api.set(
        "drawing_view_align_lines",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let drawing: usize = opts.get("drawing")?;
            let view: usize = opts.get("view")?;
            let show: bool = opts.get("show")?;
            unsafe {
                tick.exec(Instruction::SetDrawingViewAlignLines { drawing, view, show })
            }
        })?,
    )?;

    // Edit a view's caption label (#372): `bearcad.drawing_view_label{ drawing, view,
    // hidden?, pos?, text? }` — `pos` is "top-left"/"top-center"/…/"bottom-right"; an empty
    // `text` returns to the automatic caption ("Body 0 — Front (1:20)").
    api.set(
        "drawing_view_label",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let drawing: usize = opts.get("drawing")?;
            let view: usize = opts.get("view")?;
            let hidden: Option<bool> = opts.get("hidden")?;
            let pos: Option<String> = opts.get("pos")?;
            let text: Option<String> = opts.get("text")?;
            if hidden.is_none() && pos.is_none() && text.is_none() {
                return Err(mlua::Error::external(
                    "drawing_view_label needs at least one of `hidden`, `pos`, `text`",
                ));
            }
            unsafe {
                tick.exec(Instruction::SetDrawingViewLabel {
                    drawing,
                    view,
                    hidden,
                    pos,
                    text,
                })
            }
        })?,
    )?;

    // Toggle a detected circle's diameter dimension in a view (#373): keyed by the circle's
    // world centre. `bearcad.drawing_circle_dimension{ drawing, view, center = {x, y, z} }`.
    api.set(
        "drawing_circle_dimension",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let drawing: usize = opts.get("drawing")?;
            let view: usize = opts.get("view")?;
            let v: Vec<f32> = opts.get("center")?;
            if v.len() != 3 {
                return Err(mlua::Error::external(
                    "drawing_circle_dimension `center` must be a {x, y, z} point",
                ));
            }
            unsafe {
                tick.exec(Instruction::ToggleDrawingCircleDimension {
                    drawing,
                    view,
                    center: (v[0], v[1], v[2]),
                })
            }
        })?,
    )?;

    // Toggle a view's angle dimension between two edges (#180): `edge1`/`edge2` are each
    // `{ a = {x,y,z}, b = {x,y,z} }` (the edge's world endpoints).
    api.set(
        "drawing_angle",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let drawing: usize = opts.get("drawing")?;
            let view: usize = opts.get("view")?;
            let point = |t: &Table, key: &str| -> mlua::Result<(f32, f32, f32)> {
                let v: Vec<f32> = t.get(key)?;
                if v.len() != 3 {
                    return Err(mlua::Error::external(format!(
                        "drawing_angle edge `{key}` must be a {{x, y, z}} point"
                    )));
                }
                Ok((v[0], v[1], v[2]))
            };
            let edge = |key: &str| -> mlua::Result<((f32, f32, f32), (f32, f32, f32))> {
                let t: Table = opts.get(key)?;
                Ok((point(&t, "a")?, point(&t, "b")?))
            };
            let edge1 = edge("edge1")?;
            let edge2 = edge("edge2")?;
            unsafe {
                tick.exec(Instruction::ToggleDrawingAngle {
                    drawing,
                    view,
                    edge1,
                    edge2,
                })
            }
        })?,
    )?;

    // Semantic push/pull of an existing extrusion (#114) — the scripted extrusion gizmo.
    // `distance = d` sets an absolute depth (clearing any snap target), `by = d` pulls the
    // handle by a delta from the current effective depth, and `to = {...}` snaps to a
    // plane/face/vertex (same table shape as `extrude`'s `to`).
    api.set(
        "edit_extrusion",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            check_keys(&opts, "edit_extrusion", &["extrusion", "distance", "by", "to"])?;
            let extrusion: usize = opts.get("extrusion")?;
            // `distance` accepts a plain number or a parameter expression string (#402).
            let (mut distance, expression) = match scalar_arg(lua, &opts, "distance")? {
                Some((d, e @ Some(_))) => (Some(d), e),
                Some((d, None)) => (Some(d), None),
                None => (None, None),
            };
            let by: Option<f32> = opts.get("by")?;
            let target = match opts.get::<Option<Table>>("to")? {
                Some(t) => Some(parse_extrude_target_table(lua, &t)?),
                None => None,
            };
            if let Some(by) = by {
                if distance.is_some() {
                    return Err(mlua::Error::external(
                        "edit_extrusion takes `distance` or `by`, not both",
                    ));
                }
                let current = unsafe {
                    let doc = &tick.state().doc;
                    let key = doc
                        .extrusions
                        .keys()
                        .nth(extrusion)
                        .ok_or_else(|| mlua::Error::external(format!("no extrusion {extrusion}")))?;
                    let ext = &doc.extrusions[key];
                    crate::extrude::effective_distance(doc, ext)
                };
                distance = Some(current + by);
            }
            if distance.is_none() && target.is_none() {
                return Err(mlua::Error::external(
                    "edit_extrusion requires `distance`, `by`, or `to`",
                ));
            }
            unsafe {
                tick.exec(Instruction::UpdateExtrusion {
                    extrusion,
                    distance,
                    target,
                    expression,
                })
            }
        })?,
    )?;

    // Chamfer/fillet a sketch vertex where exactly two plain lines meet (#37/#38). `point`
    // resolves the same way as any other `ConstraintPoint` table arg, e.g.
    // `{ kind = "line", index = 0, end = "start" }` (see `parse_constraint_point_table`).
    api.set(
        "chamfer_vertex",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let point_table: Table = opts.get("point")?;
            let point = parse_constraint_point_table(lua, point_table)?;
            // Number or expression string, so `distance = "leg"` ties the chamfer to a parameter.
            let distance = lua_amount_expr(&opts, "distance")?;
            unsafe {
                tick.exec(Instruction::VertexTreatment {
                    point,
                    kind: VertexTreatmentKind::Chamfer,
                    amount: distance,
                })?;
            }
            Ok(())
        })?,
    )?;

    api.set(
        "fillet_vertex",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let point_table: Table = opts.get("point")?;
            let point = parse_constraint_point_table(lua, point_table)?;
            // Number or expression string, so `radius = "r"` ties the fillet to a parameter.
            let radius = lua_amount_expr(&opts, "radius")?;
            unsafe {
                tick.exec(Instruction::VertexTreatment {
                    point,
                    kind: VertexTreatmentKind::Fillet,
                    amount: radius,
                })?;
            }
            Ok(())
        })?,
    )?;

    // Chamfer/fillet an analytic edge of an extrusion's 3D solid (#77): `extrusion` is an
    // index into the document's extrusions, `edge` resolves via `parse_extrusion_edge_table`
    // (`{ kind = "vertical", face = 0, edge = 2 }` or `{ kind = "cap", face = 0, edge = 2,
    // top = true }`). Scoped to `Rect`/`Polygon`-profiled extrusions' vertical and side/cap
    // edges — see SPEC §3.4.
    api.set(
        "chamfer_edge",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let edges = parse_extrusion_edge_set(&opts)?;
            let distance: f32 = opts.get("distance")?;
            unsafe {
                tick.exec(Instruction::EdgeTreatment {
                    edges,
                    kind: VertexTreatmentKind::Chamfer,
                    amount: distance,
                })?;
            }
            Ok(())
        })?,
    )?;

    api.set(
        "fillet_edge",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let edges = parse_extrusion_edge_set(&opts)?;
            let radius: f32 = opts.get("radius")?;
            unsafe {
                tick.exec(Instruction::EdgeTreatment {
                    edges,
                    kind: VertexTreatmentKind::Fillet,
                    amount: radius,
                })?;
            }
            Ok(())
        })?,
    )?;

    api.set(
        "import",
        lua.create_function(|lua, ()| {
            let globals = lua.globals();
            let bearcad: Table = globals.get("bearcad")?;
            for pair in bearcad.pairs::<String, Value>() {
                let (name, value) = pair?;
                if name.starts_with('_') || name == "import" {
                    continue;
                }
                if let Value::Function(func) = value {
                    globals.set(name.as_str(), func)?;
                }
            }
            Ok(())
        })?,
    )?;

    lua.globals().set("bearcad", api)?;
    lua.load(
        r#"
        -- The primary API is declarative modeling (OpenSCAD-style). GUI/UI manipulation
        -- functions (camera, tool, panes, palette, mouse, keyboard, drags) live under the
        -- `bearcad.ui.*` sub-namespace so scripts can focus on modeling (#46).
        bearcad.ui = {}
        local ui_funcs = {
            "tool", "tool_mode", "help", "toolbar_shortcuts", "focus_name", "focus_dim", "pane", "palette", "settings",
            "changelog",
            "mcmaster",
            "new_tab", "close_tab", "tab", "tab_count", "window_count", "tabs", "reorder_tab", "detach_tab",
            "orbit", "pan", "wheel", "set_home_view", "toggle_projection", "shading", "ground",
            "fps", "fps_look", "fps_move", "fps_jump", "fps_fly", "fps_advance", "fps_scale",
            "camera", "elements_view", "auto_zoom", "animate_joints", "animate_zoom_to_fit",
            "update_channel",
            "snapping", "picker_focus", "angle_snap",
            "tutorial", "tutorial_next", "tutorial_assist", "tutorial_end", "tutorial_step",
            "tutorial_orb",
            "tutorial_pane", "tutorials",
            "skip_all_tutorials", "install_age", "tutorial_highlight", "tutorial_prompt",
            "complete_tutorial",
            "touch",
            "os_open",
            "move", "click", "move_ground", "click_ground",
            "drag", "drag_ground", "right_drag", "right_drag_pan",
            "key", "keydown", "keyup", "type",
            "_view", "_view_home", "_zoom_fit", "_wait", "_wait_ms", "_screenshot",
        }
        for _, name in ipairs(ui_funcs) do
            bearcad.ui[name] = bearcad[name]
            bearcad[name] = nil
        end
        -- Sketch-local (not viewport) manipulation, so it stays in the modeling namespace
        -- (#114); the ui aliases keep older scripts working.
        bearcad.ui.drag_vertex = bearcad.drag_vertex
        bearcad.ui.drag_line = bearcad.drag_line

        local function yielding(name, native_name)
            local native = bearcad.ui[native_name or name]
            bearcad.ui[name] = function(...)
                native(...)
                coroutine.yield()
            end
        end
        yielding("wait", "_wait")
        yielding("wait_ms", "_wait_ms")
        yielding("screenshot", "_screenshot")
        yielding("view", "_view")
        yielding("view_home", "_view_home")
        yielding("zoom_fit", "_zoom_fit")
    "#,
    )
    .exec()?;
    Ok(())
}

/// Load a `.lua` script file into a coroutine thread.
pub fn load_script(lua: &Lua, path: &Path) -> mlua::Result<mlua::Thread> {
    let source = std::fs::read_to_string(path).map_err(|e| {
        mlua::Error::external(format!("failed to read {}: {e}", path.display()))
    })?;
    register_api(lua)?;
    let func = lua.load(&source).set_name(path.to_string_lossy()).into_function()?;
    lua.create_thread(func)
}

#[cfg(test)]
mod tests {
    use crate::model::line_key_for_slot as lkey;
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::circle_key_for_slot as rkey;
    use crate::model::sketch_key_for_slot as skey;
    use crate::model::sketch_text_key_for_slot as tkey;
    use crate::model::extrusion_key_for_slot as xkey;
    use crate::model::drawing_key_for_slot as dkey;
    use crate::model::body_key_for_slot as bkey;
    use crate::model::joint_key_for_slot as jkey;
    use crate::model::sketch_op_key_for_slot as skop;
    use crate::model::edge_treatment_op_key_for_slot as etkey;
    use super::*;
    use crate::actions::AppState;
    use crate::model::FaceId;

    /// #1055: a script names an arena-backed element by its **ordinal** among the live ones,
    /// not by its slot. Deleting the first image used to renumber the rest; now the key moves
    /// and the ordinal is what has to be recomputed, in both directions.
    #[test]
    fn a_script_names_an_image_by_its_ordinal_among_the_live_ones() {
        let image = |name: &str| crate::model::TracingImage {
            bytes: Vec::new(),
            source_name: name.to_string(),
            plane: pkey(0),
            origin: (0.0, 0.0),
            base_origin: None,
            width_mm: 10.0,
            height_mm: 10.0,
            name: None,
            calibration: None,
        };
        let mut doc = crate::model::Document::default();
        let first = doc.tracing_images.insert(image("first"));
        let second = doc.tracing_images.insert(image("second"));

        assert_eq!(
            scene_element_from_kind(&doc, "image", 1),
            Some(SceneElement::Image(second))
        );
        assert_eq!(element_index(&doc, SceneElement::Image(second)), 1);

        // Remove the first: the second keeps its key and becomes ordinal 0.
        doc.tracing_images.remove(first);
        assert_eq!(
            scene_element_from_kind(&doc, "image", 0),
            Some(SceneElement::Image(second)),
            "the survivor is what index 0 names now"
        );
        assert_eq!(element_index(&doc, SceneElement::Image(second)), 0);
        assert_eq!(
            scene_element_from_kind(&doc, "image", 1),
            None,
            "and there is no second image to name"
        );
    }

    fn run_lua(source: &str) -> AppState {
        let mut runner = ScriptRunner::from_lua_source(source).unwrap();
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        let vp = egui::Rect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(960.0, 560.0));
        // No App frame loop here: advance view transitions so yielding camera ops
        // (view, view_home, zoom_fit) complete instead of spinning forever (#1276).
        let mut safety = 0u32;
        while !runner.done {
            let _ = state.cam.tick_transition(1.0 / 60.0);
            runner.tick(&mut state, &mut synthetic, Some(vp), &ctx);
            safety += 1;
            assert!(safety < 100_000, "run_lua spun too long; stuck waiting?");
        }
        // Failed modeling actions now raise Lua errors (#104/#109/#110/#112); tests that
        // exercise rejection paths catch them with `pcall`, so an uncaught error here is
        // always a test bug.
        assert!(runner.error.is_none(), "script error: {:?}", runner.error);
        state
    }

    fn run_lua_expect_ok(source: &str) {
        let mut runner = ScriptRunner::from_lua_source(source).unwrap();
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        let vp = egui::Rect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(960.0, 560.0));
        let mut safety = 0u32;
        while !runner.done {
            let _ = state.cam.tick_transition(1.0 / 60.0);
            runner.tick(&mut state, &mut synthetic, Some(vp), &ctx);
            safety += 1;
            assert!(safety < 100_000, "run_lua_expect_ok spun too long; stuck waiting?");
        }
        assert!(runner.error.is_none(), "script error: {:?}", runner.error);
    }

    /// #228: an in-sketch repeat op is a first-class pane element — it appears in the hierarchy
    /// with its duplicated entities nested under it (not double-listed under the sketch), and
    /// deleting the op removes the copies.
    #[test]
    fn sketch_repeat_op_groups_and_deletes_in_hierarchy() {
        use crate::hierarchy::{build_hierarchy, HierarchyNode, SceneElement};
        let mut state = run_lua(
            r#"
            bearcad.new()
            bearcad.circle{ x = 0, y = 0, r = 2 }
            bearcad.repeat_sketch{ sketch = 0, circles = {0}, angle = 0,
                                   mode = "count_gap", count = 3, spacing = 10 }
            "#,
        );
        let op = state.doc.sketch_repeat_ops.values().nth(0).unwrap().clone();
        assert_eq!(op.circle_outputs.len(), 2);
        // The op node exists with its two copies nested; count copy-circle nodes across the tree.
        let tree = build_hierarchy(&state.doc, None);
        fn count_nodes(entries: &[crate::hierarchy::HierarchyEntry], f: &dyn Fn(&HierarchyNode) -> bool) -> usize {
            entries.iter().map(|e| f(&e.node) as usize + count_nodes(&e.children, f)).sum()
        }
        assert_eq!(
            count_nodes(&tree, &|n| matches!(n, HierarchyNode::SketchRepeatOp(_))),
            1,
            "the op is a pane node"
        );
        // Each copy circle appears exactly once in the whole tree (under the op, not also the sketch).
        for &ci in &op.circle_outputs {
            assert_eq!(
                count_nodes(&tree, &|n| matches!(n, HierarchyNode::Circle(c) if *c == ci)),
                1,
                "copy circle {ci:?} is listed once"
            );
        }
        // Deleting the op removes the copies.
        crate::document_lifecycle::delete_element(&mut state.doc, SceneElement::SketchRepeatOp(skop(0)));
        assert!(state.doc.sketch_repeat_ops.is_empty(), "the op is gone");
        for &ci in &op.circle_outputs {
            assert!(!state.doc.circles.contains(ci), "copy circle {ci:?} removed with the op");
        }
    }

    /// #1346: scripts can read the guide orb's screen position (nil until the
    /// overlay draws it).
    #[test]
    fn tutorial_orb_lua_is_nil_without_a_drawn_overlay() {
        run_lua_expect_ok(
            r#"
            assert(bearcad.ui.tutorial_orb() == nil, "no tutorial, no orb")
            bearcad.ui.tutorial("cube")
            assert(bearcad.ui.tutorial_step() == 0)
            assert(bearcad.ui.tutorial_orb() == nil,
                   "headless apply does not draw the overlay")
            "#,
        );
    }

    /// #1334: the angle-bracket walkthrough is gone; scripting must not start it.
    #[test]
    fn bracket_tutorial_is_not_registered() {
        run_lua_expect_ok(
            r#"
            local names = {}
            for _, t in ipairs(bearcad.ui.tutorials()) do
              names[t.name] = t.title
            end
            assert(names.bracket == nil, "bracket tutorial should be gone")
            assert(names.cube, "other tutorials stay")
            local ok, err = pcall(function() bearcad.ui.tutorial("bracket") end)
            assert(not ok, "starting the removed tutorial should fail")
            assert(tostring(err):find("unknown tutorial", 1, true), tostring(err))
            "#,
        );
    }

    /// #1347: the parameters tutorial is scriptable by name and walks with assists.
    #[test]
    fn parameters_tutorial_lua_walks_and_builds_a_box() {
        run_lua_expect_ok(
            r#"
            local names = {}
            for _, t in ipairs(bearcad.ui.tutorials()) do
              names[t.name] = t.title
            end
            assert(names.parameters == "Parameters", "parameters tutorial is listed")
            bearcad.ui.tutorial("parameters")
            assert(bearcad.ui.tutorial_step() == 0)
            local guard = 0
            while bearcad.ui.tutorial_step() ~= nil do
              guard = guard + 1
              assert(guard < 50, "parameters tutorial should finish")
              bearcad.ui.tutorial_assist()
              if bearcad.ui.tutorial_step() ~= nil then
                bearcad.ui.tutorial_next()
              end
            end
            assert(bearcad.parameter("get", "width") == 30, "width changed to 30mm")
            assert(bearcad.parameter("get", "height") == 50, "height changed to 50mm")
            assert(bearcad.count("extrusion") == 1, "extruded")
            assert(bearcad.count("line") >= 4, "rectangle")
            "#,
        );
    }

    /// #1434: skip-all, install age, highlight, and the launch tooltip are scriptable.
    #[test]
    fn tutorial_prompt_lua_drives_skip_age_and_fade() {
        run_lua_expect_ok(
            r#"
            assert(bearcad.ui.tutorial_highlight(), "unfinished tutorials highlight")
            assert(bearcad.ui.skip_all_tutorials() == false)
            assert(bearcad.ui.install_age() == nil, "default is an upgrade, not a fresh install")
            assert(bearcad.ui.tutorial_prompt() == nil)

            bearcad.ui.install_age(5)
            local age = bearcad.ui.install_age()
            assert(age ~= nil and age >= 4.9 and age <= 5.1, "install age days, got " .. tostring(age))
            local p = bearcad.ui.tutorial_prompt("launch")
            assert(p ~= nil and p.text == "Want to try some tutorials?", "launch prompt")
            assert(p.alpha == 1)

            bearcad.ui.tutorial_prompt("tick", 10)
            p = bearcad.ui.tutorial_prompt()
            assert(p ~= nil and p.alpha == 1, "idle time does not fade")

            bearcad.ui.tutorial_prompt("work")
            bearcad.ui.tutorial_prompt("tick", 2.9)
            p = bearcad.ui.tutorial_prompt()
            assert(p ~= nil and p.alpha == 1, "hold before fade")
            bearcad.ui.tutorial_prompt("tick", 0.5)
            p = bearcad.ui.tutorial_prompt()
            assert(p ~= nil and p.alpha < 1 and p.alpha > 0, "fading, alpha=" .. tostring(p.alpha))
            bearcad.ui.tutorial_prompt("tick", 2)
            assert(bearcad.ui.tutorial_prompt() == nil, "faded away")

            bearcad.ui.tutorial_prompt("launch")
            bearcad.ui.skip_all_tutorials(true)
            assert(bearcad.ui.skip_all_tutorials())
            assert(not bearcad.ui.tutorial_highlight(), "skip-all kills the blue button")
            assert(bearcad.ui.tutorial_prompt() == nil, "skip-all kills the prompt")

            bearcad.ui.skip_all_tutorials(false)
            assert(bearcad.ui.tutorial_highlight(), "unskip restores the highlight")

            bearcad.ui.install_age(40)
            assert(bearcad.ui.tutorial_prompt("launch") == nil, "past 30 days, no prompt")
            bearcad.ui.install_age(false)
            assert(bearcad.ui.install_age() == nil)

            for _, t in ipairs(bearcad.ui.tutorials()) do
              bearcad.ui.complete_tutorial(t.name)
            end
            assert(not bearcad.ui.tutorial_highlight(), "all complete, no highlight")
            "#,
        );
    }

    /// #1306: navigate tutorial starts with cubes and no default datum planes.
    #[test]
    fn navigate_tutorial_lua_has_cubes_and_no_datum_planes() {
        run_lua_expect_ok(
            r#"
            bearcad.ui.tutorial("navigate")
            assert(bearcad.ui.tutorial_step() == 0)
            assert(bearcad.count("construction_plane") == 0,
                   "default planes should be gone, got " .. bearcad.count("construction_plane"))
            assert(bearcad.count("body") >= 2,
                   "seeded cubes, got " .. bearcad.count("body") .. " bodies")
            "#,
        );
    }

    /// An in-sketch offset op parallels a closed rectangle outward, nests the copies
    /// under the op in the pane, tracks source drags, honors the construction toggle,
    /// re-offsets on edit, and deletes with the op.
    #[test]
    fn sketch_offset_op_parallels_edits_and_deletes() {
        use crate::hierarchy::{build_hierarchy, HierarchyNode, SceneElement};
        let mut state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 20, height = 10 }
            bearcad.circle{ x = 40, y = 0, r = 5 }
            bearcad.offset_sketch{ sketch = 0, lines = {0, 1, 2, 3}, circles = {0},
                                   distance = 2 }
            "#,
        );
        let op = state.doc.sketch_offset_ops.values().nth(0).unwrap().clone();
        assert_eq!(op.line_outputs.len(), 4);
        assert_eq!(op.circle_outputs.len(), 1);
        // Closed loop grows outward: the offset rectangle spans 24 × 14.
        let xs: Vec<f32> = op
            .line_outputs
            .iter()
            .flat_map(|&li| [state.doc.lines[li].x0, state.doc.lines[li].x1])
            .collect();
        let w = xs.iter().cloned().fold(f32::MIN, f32::max)
            - xs.iter().cloned().fold(f32::MAX, f32::min);
        assert!((w - 24.0).abs() < 1e-3, "outward offset width, got {w}");
        assert!((state.doc.circles[op.circle_outputs[0]].r - 7.0).abs() < 1e-3);
        assert!(!state.doc.lines[op.line_outputs[0]].construction);

        // Pane: the op node exists, each output listed exactly once (under the op).
        let tree = build_hierarchy(&state.doc, None);
        fn count_nodes(
            entries: &[crate::hierarchy::HierarchyEntry],
            f: &dyn Fn(&HierarchyNode) -> bool,
        ) -> usize {
            entries.iter().map(|e| f(&e.node) as usize + count_nodes(&e.children, f)).sum()
        }
        assert_eq!(
            count_nodes(&tree, &|n| matches!(n, HierarchyNode::SketchOffsetOp(_))),
            1
        );
        for &li in &op.line_outputs {
            assert_eq!(
                count_nodes(&tree, &|n| matches!(n, HierarchyNode::Line(l) if *l == li)),
                1,
                "offset line {} listed once",
                li.index()
            );
        }

        // The outputs track source geometry through recompute (the circle's centre is
        // free — its radius is dimension-locked by the declarative call).
        state.doc.circles[rkey(0)].cx = 55.0;
        crate::parameters::recompute_document_geometry(&mut state.doc).unwrap();
        assert!(
            (state.doc.circles[op.circle_outputs[0]].cx - 55.0).abs() < 1e-3,
            "offset circle should follow its source, cx = {}",
            state.doc.circles[op.circle_outputs[0]].cx
        );

        // Edit: new distance and construction toggle re-offset in place.
        state.apply(crate::actions::Action::EditSketchOffsetOperation {
            op: crate::model::sketch_op_key_for_slot(0),
            line_targets: op.line_targets.clone(),
            circle_targets: op.circle_targets.clone(),
            distance: "-3".to_string(),
            construction: true,
        });
        let op = state.doc.sketch_offset_ops.values().nth(0).unwrap().clone();
        assert!((state.doc.circles[op.circle_outputs[0]].r - 2.0).abs() < 1e-3);
        assert!(state.doc.lines[op.line_outputs[0]].construction);
        let xs: Vec<f32> = op
            .line_outputs
            .iter()
            .flat_map(|&li| [state.doc.lines[li].x0, state.doc.lines[li].x1])
            .collect();
        let w = xs.iter().cloned().fold(f32::MIN, f32::max)
            - xs.iter().cloned().fold(f32::MAX, f32::min);
        assert!((w - 14.0).abs() < 1e-3, "negative offset shrinks, got {w}");

        // Deleting the op removes the outputs.
        crate::document_lifecycle::delete_element(&mut state.doc, SceneElement::SketchOffsetOp(skop(0)));
        assert!(state.doc.sketch_offset_ops.is_empty(), "the op is gone");
        for &li in &op.line_outputs {
            assert!(!state.doc.lines.contains(li));
        }
        assert!(!state.doc.circles.contains(op.circle_outputs[0]));
    }

    /// #495: a closed offset of a rectangle must form a pickable/extrudable inner face
    /// (mitered corners joined by Coincident constraints).
    #[test]
    fn sketch_offset_closed_loop_is_extrudable_face() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 40, height = 30 }
            bearcad.offset_sketch{ sketch = 0, lines = {0, 1, 2, 3}, distance = -5 }
            local op_lines = {}
            for i = 4, 7 do table.insert(op_lines, i) end
            bearcad.extrude{ polygon = op_lines, distance = 8 }
            assert(bearcad.get{ kind = "body", index = 0 } ~= nil, "inner face must extrude")
            "#,
        );
        assert!(!state.doc.bodies.is_empty(), "inner offset face extruded a body");
        let loops = crate::polygon::closed_line_loops(&state.doc, skey(0));
        assert!(
            loops.len() >= 2,
            "outer + inner loops expected, got {}",
            loops.len()
        );
    }

    /// #494: offsetting a cubic-bezier sketch line must produce a curved copy
    /// (bezier handles present), not a straight chamfer-style segment.
    #[test]
    fn sketch_offset_of_curve_stays_curved() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{
              x = 0, y = 0, x1 = 40, y1 = 0,
              bezier = {{10, 20}, {30, 20}},
            }
            bearcad.offset_sketch{ sketch = 0, lines = {0}, distance = 5 }
            "#,
        );
        let op = &state.doc.sketch_offset_ops.values().nth(0).unwrap();
        assert_eq!(op.line_outputs.len(), 1);
        let out = &state.doc.lines[op.line_outputs[0]];
        assert!(
            out.bezier.is_some(),
            "offset of a curved line must keep bezier handles"
        );
        let [c0, c1] = out.bezier.unwrap();
        let mid_handle_y = (c0.1 + c1.1) * 0.5;
        assert!(
            mid_handle_y.abs() > 1.0,
            "offset handles should leave the chord, mid_y={mid_handle_y}"
        );
    }

    /// A parameter expression drives the offset distance and re-syncs on parameter edits.
    #[test]
    fn sketch_offset_distance_follows_parameter() {
        let mut state = run_lua(
            r#"
            bearcad.new()
            bearcad.parameter("add", "gap", "3")
            bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0 }
            bearcad.offset_sketch{ sketch = 0, lines = {0}, distance = "gap" }
            "#,
        );
        let op = state.doc.sketch_offset_ops.values().nth(0).unwrap().clone();
        assert!((state.doc.lines[op.line_outputs[0]].y0 - 3.0).abs() < 1e-3);
        let param = state.doc.parameters.keys().next().expect("the parameter");
        state.apply(crate::actions::Action::CommitParameterExpression {
            index: param,
            expression: "5".to_string(),
        });
        assert!((state.doc.lines[op.line_outputs[0]].y0 - 5.0).abs() < 1e-3);
    }

    /// #226: repeating a whole sketch along an axis copies it onto parallel offset planes — the
    /// copies' entities keep their plane-local coords, so they step by the offset in world.
    #[test]
    fn repeat_sketch_along_axis_copies_onto_offset_planes() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.circle{ x = 1, y = 2, r = 3 }
            bearcad.repeat_sketches{ sketches = {0}, axis = "z",
                                     mode = "count_gap", count = 3, spacing = 10 }
            "#,
        );
        let op = &state.doc.repeat_ops.values().nth(0).unwrap();
        assert_eq!(op.sketch_outputs.len(), 2, "3 instances = original + 2 copies");
        assert_eq!(op.sketch_plane_outputs.len(), 2);
        // Host planes step +z by the gap (extent 0 → step = gap).
        let pz = |i: usize| state.doc.construction_planes[op.sketch_plane_outputs[i]].origin.z;
        assert!((pz(0) - 10.0).abs() < 1e-4);
        assert!((pz(1) - 20.0).abs() < 1e-4);
        // Each copy sketch carries a circle with the source's plane-local centre/radius.
        for &si in &op.sketch_outputs {
            let c = state.doc.circles.values().find(|c| c.sketch == si).unwrap();
            assert_eq!((c.cx, c.cy, c.r), (1.0, 2.0, 3.0));
        }

        // #231: the generated host planes nest under the repeat op, not at the top level.
        use crate::hierarchy::{build_hierarchy, HierarchyNode};
        let tree = build_hierarchy(&state.doc, None);
        let doc_root = &tree[0];
        for &pi in &op.sketch_plane_outputs {
            assert!(
                !doc_root
                    .children
                    .iter()
                    .any(|e| e.node == HierarchyNode::ConstructionPlane(pi)),
                "host plane {pi:?} should not be a top-level node"
            );
        }
        // The repeat-op node carries the host planes as children.
        let repeat_node = doc_root
            .children
            .iter()
            .find(|e| matches!(e.node, HierarchyNode::RepeatOp(_)))
            .expect("repeat op node");
        for &pi in &op.sketch_plane_outputs {
            assert!(
                repeat_node
                    .children
                    .iter()
                    .any(|e| e.node == HierarchyNode::ConstructionPlane(pi)),
                "host plane {pi:?} nests under the op"
            );
        }
    }

    /// #231: a sketch hosted on a body face (not a construction plane) can be repeated — the copy
    /// rides a plane synthesized from the face frame, offset along the axis.
    #[test]
    fn repeat_sketch_hosted_on_a_body_face() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 20, height = 20 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
            bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 0,
                                  profile = "polygon", profile_lines = {0, 1, 2, 3}, top = true }
            bearcad.circle{ x = 0, y = 0, r = 2 }
            bearcad.repeat_sketches{ sketches = {1}, axis = "z",
                                     mode = "count_gap", count = 2, spacing = 5 }
            "#,
        );
        let op = &state.doc.repeat_ops.values().nth(0).unwrap();
        assert_eq!(op.sketch_outputs.len(), 1, "2 instances = original + 1 copy");
        // The cap sits at z = 10; the copy's host plane is +5 above it.
        let pz = state.doc.construction_planes[op.sketch_plane_outputs[0]].origin.z;
        assert!((pz - 15.0).abs() < 1e-3, "host plane at cap (10) + gap (5), got {pz}");
        let si = op.sketch_outputs[0];
        assert!(state.doc.circles.values().any(|c| c.sketch == si));
    }

    /// #224: slicing a line by a crossing line shadows the original and emits two fragments that
    /// meet at the crossing point.
    #[test]
    fn sketch_slice_splits_a_line_at_a_crossing() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0 }
            bearcad.line{ x = 5, y = -5, x1 = 5, y1 = 5 }
            bearcad.slice_sketch{ sketch = 0, lines = {0}, cutters = {1} }
            "#,
        );
        // The original is shadowed (kept, not face-forming); its two fragments are real lines.
        assert!(state.doc.lines[lkey(0)].shadow, "sliced original becomes a shadow line");
        let op = &state.doc.sketch_slice_ops.values().nth(0).unwrap();
        assert_eq!(op.line_outputs.len(), 2, "one crossing → two fragments");
        let frag = |i: usize| {
            let l = &state.doc.lines[op.line_outputs[i]];
            (l.x0, l.y0, l.x1, l.y1)
        };
        assert_eq!(frag(0), (0.0, 0.0, 5.0, 0.0));
        assert_eq!(frag(1), (5.0, 0.0, 10.0, 0.0));
        assert!(!state.doc.lines[op.line_outputs[0]].shadow, "fragments are not shadow");
    }

    /// #229: an in-sketch slice op is a first-class pane element — its fragments nest under it,
    /// and deleting the op un-shadows the original and removes the fragments.
    #[test]
    fn sketch_slice_op_groups_and_deletes_in_hierarchy() {
        use crate::hierarchy::{build_hierarchy, HierarchyNode, SceneElement};
        let mut state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0 }
            bearcad.line{ x = 5, y = -5, x1 = 5, y1 = 5 }
            bearcad.slice_sketch{ sketch = 0, lines = {0}, cutters = {1} }
            "#,
        );
        let op = state.doc.sketch_slice_ops.values().nth(0).unwrap().clone();
        assert_eq!(op.line_outputs.len(), 2);
        assert!(state.doc.lines[lkey(0)].shadow);
        let tree = build_hierarchy(&state.doc, None);
        fn count_nodes(entries: &[crate::hierarchy::HierarchyEntry], f: &dyn Fn(&HierarchyNode) -> bool) -> usize {
            entries.iter().map(|e| f(&e.node) as usize + count_nodes(&e.children, f)).sum()
        }
        assert_eq!(count_nodes(&tree, &|n| matches!(n, HierarchyNode::SketchSliceOp(_))), 1);
        for &li in &op.line_outputs {
            assert_eq!(
                count_nodes(&tree, &|n| matches!(n, HierarchyNode::Line(l) if *l == li)),
                1,
                "fragment line {} listed once (under the op)",
                li.index()
            );
        }
        crate::document_lifecycle::delete_element(&mut state.doc, SceneElement::SketchSliceOp(skop(0)));
        assert!(!state.doc.lines[lkey(0)].shadow, "delete un-shadows the original");
        for &li in &op.line_outputs {
            assert!(!state.doc.lines.contains(li), "fragment {} removed", li.index());
        }
    }

    /// #224: a shadowed (sliced) line no longer forms a polygon face — its fragments do. Slicing
    /// one edge of a rectangle drops the original 4-line loop but the 5 pieces still close it.
    #[test]
    fn sketch_slice_shadow_line_is_excluded_from_faces() {
        let mut doc = crate::model::Document::default();
        doc.sketches.insert(crate::model::Sketch {
            face: crate::model::FaceId::ConstructionPlane(pkey(0)),
            name: None,
            length_unit: None,
            angle_unit: None,
        });
        // A closed square: 4 lines forming one loop.
        crate::construction::add_line_rectangle(&mut doc, skey(0), 0.0, 0.0, 10.0, 10.0, [false; 4]);
        assert_eq!(crate::polygon::closed_line_loops(&doc, skey(0)).len(), 1);
        // Shadow the bottom edge (line 0): the original loop is no longer detected.
        doc.lines[lkey(0)].shadow = true;
        assert_eq!(
            crate::polygon::closed_line_loops(&doc, skey(0)).len(),
            0,
            "a shadow edge breaks the loop until its fragments replace it"
        );
    }

    /// #222: a 2D in-sketch repeat duplicates a circle along +u at a fixed pitch — the copies'
    /// centres step by the pitch in sketch-local coords, grouped under the op.
    #[test]
    fn sketch_repeat_duplicates_a_circle_along_the_direction() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.circle{ x = 0, y = 0, r = 2 }
            bearcad.repeat_sketch{ sketch = 0, circles = {0}, angle = 0,
                                   mode = "count_gap", count = 4, spacing = 10 }
            "#,
        );
        let op = &state.doc.sketch_repeat_ops.values().nth(0).unwrap();
        // extent along +u is the circle's diameter (4); gap 10 → step 14.
        assert_eq!(op.circle_outputs.len(), 3, "4 instances = original + 3 copies");
        let cx = |i: usize| state.doc.circles[op.circle_outputs[i]].cx;
        assert!((cx(0) - 14.0).abs() < 1e-3, "first copy at x = extent + gap");
        assert!((cx(1) - 28.0).abs() < 1e-3);
        assert!((cx(2) - 42.0).abs() < 1e-3);
        // Copies keep the radius and stay on the same y.
        assert!((state.doc.circles[op.circle_outputs[0]].r - 2.0).abs() < 1e-6);
        assert!(state.doc.circles[op.circle_outputs[0]].cy.abs() < 1e-6);
    }

    /// #222: editing the op re-spaces and resizes the generated copies.
    #[test]
    fn sketch_repeat_edit_respaces_copies() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.circle{ x = 0, y = 0, r = 1 }
            bearcad.repeat_sketch{ sketch = 0, circles = {0}, angle = 90,
                                   mode = "count_gap", count = 2, spacing = 5 }
            bearcad.edit_sketch_repeat{ index = 0, circles = {0}, angle = 90,
                                        mode = "count_gap", count = 3, spacing = 5 }
            "#,
        );
        let op = &state.doc.sketch_repeat_ops.values().nth(0).unwrap();
        assert_eq!(op.circle_outputs.len(), 2, "3 instances = original + 2 copies");
        // angle 90 → +v; extent 2 (diameter), gap 5 → step 7 along y.
        let cy = |i: usize| state.doc.circles[op.circle_outputs[i]].cy;
        assert!((cy(0) - 7.0).abs() < 1e-3);
        assert!((cy(1) - 14.0).abs() < 1e-3);
    }

    /// #212: the scripting-doc examples that used the stale `"rect"` element/selection kind now
    /// address a rect as its four lines. Run the fixed snippets end to end so they can't rot
    /// back into a runtime error.
    #[test]
    fn docs_rect_examples_address_lines_not_a_rect_kind() {
        // declarative-modeling.md: name an edge of a rect after the fact.
        run_lua_expect_ok(
            r#"
            bearcad.new()
            bearcad.rect{ width = 80, height = 50, name = "Main box" }
            bearcad.set_name(bearcad.element("line", 0), "Front edge")
            "#,
        );
        // point-selection.md: select a rectangle corner as a line endpoint.
        run_lua_expect_ok(
            r#"
            bearcad.new()
            bearcad.rect{ width = 80, height = 50 }
            bearcad.select{ kind = "line", index = 2, ["end"] = "start" }
            "#,
        );
    }

    /// #33: `bearcad.ui.shading(...)` drives the HUD shading-mode popup's underlying state.
    #[test]
    fn lua_shading_sets_camera_shading_mode() {
        let state = run_lua(r#"bearcad.ui.shading("wireframe")"#);
        assert_eq!(state.cam.shading_mode(), ShadingMode::Wireframe);
    }

    #[test]
    fn lua_shading_accepts_all_mode_names() {
        for (name, expected) in [
            ("wireframe", ShadingMode::Wireframe),
            ("transparent", ShadingMode::TransparentSolid),
            ("solid", ShadingMode::Solid),
            ("solid_wireframe", ShadingMode::SolidWireframe),
            ("realistic", ShadingMode::Realistic),
        ] {
            let state = run_lua(&format!(r#"bearcad.ui.shading("{name}")"#));
            assert_eq!(state.cam.shading_mode(), expected, "shading({name})");
        }
    }

    #[test]
    fn lua_shading_rejects_unknown_mode() {
        let mut runner = ScriptRunner::from_lua_source(r#"bearcad.ui.shading("nonsense")"#)
            .unwrap();
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        let vp = egui::Rect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(960.0, 560.0));
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, Some(vp), &ctx);
        }
        assert!(runner.error.is_some(), "unknown shading mode should error");
    }

    /// Tab script API queues workspace ops on the runner (App applies them each frame).
    #[test]
    fn lua_tab_ops_queue_on_runner() {
        let mut runner = ScriptRunner::from_lua_source(
            r#"
            bearcad.ui.new_tab()
            bearcad.ui.new_tab{ same = true }
            bearcad.ui.tab(0)
            bearcad.ui.close_tab(1)
            bearcad.ui.reorder_tab(0, 1)
            bearcad.ui.detach_tab()
            assert(bearcad.ui.tab_count() == 1)
            local tabs = bearcad.ui.tabs()
            assert(#tabs == 1)
            assert(tabs[1].title == "Untitled")
            "#,
        )
        .unwrap();
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        let vp = egui::Rect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(960.0, 560.0));
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, Some(vp), &ctx);
        }
        assert!(runner.error.is_none(), "script error: {:?}", runner.error);
        use crate::script::TabOp;
        assert!(
            runner
                .pending_tab_ops
                .iter()
                .any(|op| matches!(op, TabOp::NewBlank)),
            "new_tab should queue NewBlank: {:?}",
            runner.pending_tab_ops
        );
        assert!(
            runner
                .pending_tab_ops
                .iter()
                .any(|op| matches!(op, TabOp::NewSameDocument)),
            "same-document new_tab should queue: {:?}",
            runner.pending_tab_ops
        );
        assert!(
            runner
                .pending_tab_ops
                .iter()
                .any(|op| matches!(op, TabOp::Select { index: 0 })),
            "tab(0) should queue Select: {:?}",
            runner.pending_tab_ops
        );
        assert!(
            runner
                .pending_tab_ops
                .iter()
                .any(|op| matches!(op, TabOp::Close { index: Some(1) })),
            "close_tab(1) should queue: {:?}",
            runner.pending_tab_ops
        );
        assert!(
            runner
                .pending_tab_ops
                .iter()
                .any(|op| matches!(op, TabOp::Reorder { from: 0, to: 1 })),
            "reorder_tab should queue: {:?}",
            runner.pending_tab_ops
        );
        assert!(
            runner
                .pending_tab_ops
                .iter()
                .any(|op| matches!(op, TabOp::Detach { index: None })),
            "detach_tab should queue: {:?}",
            runner.pending_tab_ops
        );
    }

    /// #1326: `bearcad.ui.os_open` rides the same pending-open queue as a Finder
    /// double-click, so the drain path is scriptable.
    #[test]
    fn lua_os_open_opens_via_the_finder_queue() {
        let path = std::env::temp_dir().join(format!(
            "bearcad_os_open_{}.bearcad",
            std::process::id()
        ));
        let path_s = path.to_string_lossy().replace('\\', "\\\\");
        let state = run_lua(&format!(
            r#"
            bearcad.new()
            bearcad.rect{{ width = 10, height = 10 }}
            bearcad.save("{path_s}")
            bearcad.new()
            assert(bearcad.count("line") == 0)
            bearcad.ui.os_open("{path_s}")
            bearcad.ui.wait(2)
            assert(bearcad.count("line") == 4, "os_open should load the saved document")
        "#
        ));
        let _ = std::fs::remove_file(&path);
        assert_eq!(state.doc.lines.len(), 4);
    }

    /// #1341: adding a body after save writes one body row; a second connection
    /// still sees the last commit until Save; existing lines are not rewritten.
    #[test]
    fn lua_incremental_body_write() {
        let path = std::env::temp_dir().join(format!(
            "bearcad_lua_incr_{}.bearcad",
            std::process::id()
        ));
        let path_s = path.to_string_lossy().replace('\\', "\\\\");
        let _ = std::fs::remove_file(&path);
        let state = run_lua(&format!(
            r#"
            bearcad.new()
            bearcad.rect{{ width = 20, height = 10 }}
            bearcad.cuboid{{ width = 8, depth = 8, height = 8 }}
            bearcad.save("{path_s}")
            local bodies0 = bearcad.sqlite_scalar("SELECT COUNT(*) FROM bodies")
            local lines0 = bearcad.sqlite_scalar("SELECT COUNT(*) FROM lines")
            assert(bodies0 == 1, "one body after first save")
            bearcad.cuboid{{ width = 4, depth = 4, height = 4, at = {{20, 0, 0}} }}
            local w = bearcad.session_writes()
            assert(w.bodies and w.bodies.inserts == 1, "bodies table grew by one insert")
            assert(not w.lines, "existing lines were not deleted/reinserted")
            assert(
                bearcad.sqlite_scalar("SELECT COUNT(*) FROM bodies") == bodies0,
                "another connection still sees the last save"
            )
            bearcad.save()
            assert(bearcad.sqlite_scalar("SELECT COUNT(*) FROM bodies") == bodies0 + 1)
            assert(bearcad.sqlite_scalar("SELECT COUNT(*) FROM lines") == lines0)
        "#
        ));
        let _ = std::fs::remove_file(&path);
        assert_eq!(state.doc.bodies.len(), 2);
    }

    /// #1343: open a boolean-heavy file; first-frame meshes come from geometry_cache.
    #[test]
    fn lua_open_boolean_meshes_come_from_cache() {
        let path = std::env::temp_dir().join(format!(
            "bearcad_lua_geom_cache_{}.bearcad",
            std::process::id()
        ));
        let path_s = path.to_string_lossy().replace('\\', "\\\\");
        let _ = std::fs::remove_file(&path);
        let state = run_lua(&format!(
            r#"
            bearcad.new()
            bearcad.cuboid{{ width = 20, depth = 20, height = 20 }}
            bearcad.cuboid{{ width = 8, depth = 8, height = 30, at = {{0, 0, 5}} }}
            bearcad.combine{{ op = "cut", a = {{0}}, b = {{1}} }}
            -- Force a committed mesh so Save writes geometry_cache.
            assert(bearcad.body_stats(2).triangles > 0, "boolean result must mesh")
            bearcad.save("{path_s}")
            assert(bearcad.sqlite_scalar("SELECT COUNT(*) FROM geometry_cache") >= 1,
                "saved file must hold a cache row")
            bearcad.new()
            bearcad.open("{path_s}")
            local s = bearcad.mesh_cache()
            assert(s.warmed >= 1, "open must warm meshes from geometry_cache")
            local misses = s.misses
            assert(bearcad.body_stats(2).triangles > 0)
            local s2 = bearcad.mesh_cache()
            assert(s2.misses == misses, "first-frame boolean mesh must come from cache")
            bearcad.rebuild_geometry()
            -- discard is in the open txn; another connection still sees the last save
            assert(bearcad.sqlite_scalar("SELECT COUNT(*) FROM geometry_cache") >= 1)
            bearcad.save()
            assert(bearcad.sqlite_scalar("SELECT COUNT(*) FROM geometry_cache") == 0,
                "Save after rebuild publishes the discarded table")
        "#
        ));
        let _ = std::fs::remove_file(&path);
        assert!(
            state.doc.bodies.len() >= 1,
            "boolean file should still have bodies"
        );
    }

    /// #1342: importing a unit persists `units.document` as a nested `.bearcad` blob.
    /// Syncing the source replaces that one blob; a second connection still sees the
    /// last save until ⌘S.
    #[test]
    fn lua_unit_document_is_a_nested_blob() {
        let pid = std::process::id();
        let source = std::env::temp_dir().join(format!("bearcad_lua_unit_src_{pid}.bearcad"));
        let host = std::env::temp_dir().join(format!("bearcad_lua_unit_host_{pid}.bearcad"));
        let source_s = source.to_string_lossy().replace('\\', "\\\\");
        let host_s = host.to_string_lossy().replace('\\', "\\\\");
        let _ = std::fs::remove_file(&source);
        let _ = std::fs::remove_file(&host);
        let state = run_lua(&format!(
            r#"
            bearcad.new()
            bearcad.parameter("add", "width", "10")
            bearcad.save("{source_s}")
            bearcad.new()
            bearcad.save("{host_s}")
            bearcad.import_unit{{ path = "{source_s}", link = "static", name = "part" }}
            bearcad.save()
            assert(bearcad.sqlite_scalar("SELECT typeof(document) FROM units") == "blob",
                "units.document must be a blob")
            assert(bearcad.sqlite_scalar("SELECT CAST(substr(document, 1, 15) AS TEXT) FROM units")
                == "SQLite format 3", "nested blob must be a .bearcad")
            bearcad.open("{source_s}")
            bearcad.parameter("value", 0, "99")
            bearcad.save()
            bearcad.open("{host_s}")
            bearcad.sync_unit(0)
            local w = bearcad.session_writes()
            assert(w.units and w.units.updates == 1, "sync replaces the one unit blob")
            assert(not w.unit_instances, "instances stay rows of their own")
            assert(
                bearcad.sqlite_scalar("SELECT CAST(substr(document, 1, 15) AS TEXT) FROM units")
                    == "SQLite format 3",
                "committed file still has the last save's blob"
            )
            bearcad.save()
            assert(bearcad.sqlite_scalar("SELECT typeof(document) FROM units") == "blob")
        "#
        ));
        let _ = std::fs::remove_file(&source);
        let _ = std::fs::remove_file(&host);
        assert_eq!(state.doc.units.len(), 1);
    }

    /// #46: GUI/UI manipulation lives under `bearcad.ui.*`; modeling stays top-level.
    #[test]
    fn lua_ui_functions_live_under_ui_namespace() {
        run_lua_expect_ok(
            r#"
            assert(bearcad.ui ~= nil, "bearcad.ui table missing")
            for _, name in ipairs({ "move", "click", "tool", "view", "orbit", "pan",
                                    "key", "type", "pane", "palette", "wait", "help",
                                    "toolbar_shortcuts", "changelog",
                                    "new_tab", "close_tab", "tab", "tabs", "tab_count",
                                    "window_count", "reorder_tab", "detach_tab" }) do
                assert(type(bearcad.ui[name]) == "function", "bearcad.ui." .. name .. " missing")
                assert(bearcad[name] == nil, "bearcad." .. name .. " should move to bearcad.ui")
            end
            -- drag_vertex/drag_line take sketch-local coordinates, so they live in the
            -- modeling namespace (#114) with back-compat aliases under bearcad.ui.
            for _, name in ipairs({ "drag_vertex", "drag_line" }) do
                assert(type(bearcad[name]) == "function", "bearcad." .. name .. " missing")
                assert(bearcad.ui[name] == bearcad[name], "bearcad.ui." .. name .. " alias missing")
            end
            -- declarative modeling stays at the top level
            for _, name in ipairs({ "rect", "line", "circle", "extrude", "new", "select",
                                    "add_constraint", "parameter", "export_stl", "export_3mf",
                                    "export_step", "export_preview",
                                    "import_stl", "import_step", "import_lua", "chamfer_vertex",
                                    "fillet_vertex", "chamfer_edge", "fillet_edge", "project" }) do
                assert(type(bearcad[name]) == "function", "bearcad." .. name .. " should stay top-level")
            end
        "#,
        );
    }

    /// #189: selecting a point and a sketch origin axis, then applying Coincident, pins the
    /// point onto that axis — the full select→constrain flow, no mouse simulation.
    #[test]
    fn lua_constrain_point_to_origin_axis() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 5, y = 5, x1 = 12, y1 = 8 }
            bearcad.select{ kind = "line", index = 0, ["end"] = "start" }
            bearcad.select({ kind = "axis", axis = "x" }, true)
            bearcad.add_geometric_constraint("coincident")
        "#,
        );
        assert!(
            state.doc.lines[lkey(0)].y0.abs() < 1e-3,
            "the start point should be pinned to the X axis (v = 0), got y0={}",
            state.doc.lines[lkey(0)].y0
        );
    }

    /// #839: `around = true` turns the copies about the axis; the copies land on the circle.
    #[test]
    fn lua_repeat_around_the_axis_turns_the_copies() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ x = 20, y = -3, width = 8, height = 6 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 4 }
            bearcad.exit_sketch()
            bearcad.repeat_bodies{ bodies = {0}, axis = "z", around = true,
                                   mode = "count_gap", count = 4, spacing = "90deg" }
        "#,
        );
        let op = &state.doc.repeat_ops.values().nth(0).unwrap();
        assert!(op.around_axis);
        assert_eq!(op.outputs.len(), 3, "4 instances = the original plus 3 copies");
        // The first copy is a quarter turn round: its bounds swap x for y.
        let source = crate::extrude::body_solid_mesh(&state.doc, bkey(0)).expect("source mesh");
        let copy = crate::extrude::body_solid_mesh(&state.doc, op.outputs[0]).expect("copy mesh");
        let (smin, smax) = source.bounds().unwrap();
        let (cmin, cmax) = copy.bounds().unwrap();
        assert!(smin.x > 0.0 && smax.x > 0.0, "the source sits out along +X");
        assert!(cmin.y > 0.0 && cmax.y > 0.0, "the quarter-turn copy sits out along +Y");
        assert!(cmax.x.abs() < 4.0, "and no longer along X, got {}", cmax.x);
    }

    /// #834: materials from scripts — created with a colour, handed to bodies, reassigned.
    #[test]
    fn lua_materials_are_scriptable() {
        let state = run_lua(
            r##"
            bearcad.new()
            bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.material{ name = "Brass", color = "#c88a4a", bodies = {0} }
            bearcad.material{ name = "Steel" }
        "##,
        );
        // The document starts with the default palette (#928), so the scripted pair lands
        // after it.
        let seeded = crate::model::Material::DEFAULTS.len();
        assert_eq!(state.doc.materials.len(), seeded + 2);
        let brass = state.doc.materials.keys().nth(seeded).expect("Brass");
        assert_eq!(state.doc.materials[brass].name, "Brass");
        assert_eq!(state.doc.materials[brass].color, [0xc8, 0x8a, 0x4a]);
        assert_eq!(state.doc.bodies[bkey(0)].material, Some(brass), "Brass was handed to it");

        let state = run_lua(&format!(
            r##"
            bearcad.new()
            bearcad.rect{{ x = 0, y = 0, width = 10, height = 10 }}
            bearcad.extrude{{ polygon = {{0, 1, 2, 3}}, distance = 5 }}
            bearcad.material{{ name = "Brass", color = "#c88a4a", bodies = {{0}} }}
            bearcad.material{{ name = "Steel" }}
            bearcad.set_material{{ body = 0, material = {} }}
        "##,
            seeded + 1
        ));
        // A script names a material by its ordinal among the live ones; the boundary
        // resolves that to the key the body actually holds (#1055).
        let steel = state.doc.materials.keys().nth(seeded + 1).expect("Steel");
        assert_eq!(state.doc.bodies[bkey(0)].material, Some(steel), "reassigned to Steel");
    }

    /// #1218: scripts can turn any body into a shadow body (and back).
    #[test]
    fn lua_set_body_shadow_is_scriptable() {
        let state = run_lua(
            r##"
            bearcad.new()
            bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.set_body_shadow{ body = 0, shadow = true }
        "##,
        );
        assert!(
            state.doc.bodies[bkey(0)].shadow,
            "set_body_shadow marks the body as a shadow"
        );

        let state = run_lua(
            r##"
            bearcad.new()
            bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.set_body_shadow{ body = 0, shadow = true }
            bearcad.set_body_shadow{ body = 0, shadow = false }
        "##,
        );
        assert!(
            !state.doc.bodies[bkey(0)].shadow,
            "set_body_shadow restores a live body"
        );
    }

    /// Modeling half of `docs-site/screenshots/materials.lua` (the capture/quit tail is
    /// stripped so this can run without a GPU frame).
    fn materials_screenshot_modeling_lua() -> String {
        let src = include_str!("../docs-site/screenshots/materials.lua");
        let cut = src
            .find("bearcad.clear_selection()")
            .expect("materials.lua should separate modeling from the capture tail");
        src[..cut].to_string()
    }

    /// The materials screenshot is eight coloured corner cubes plus a same-size centre
    /// cube that overlaps them all, with a through-hole, a sphere bite, a chamfer and a
    /// fillet each on a different corner cube.
    #[test]
    fn lua_materials_screenshot_scene_connects_cubes_and_features() {
        let state = run_lua(&materials_screenshot_modeling_lua());

        let cuboids: Vec<_> = state
            .doc
            .primitives
            .values()
            .filter(|p| p.kind == crate::model::PrimitiveKind::Cuboid)
            .collect();
        let spheres: Vec<_> = state
            .doc
            .primitives
            .values()
            .filter(|p| p.kind == crate::model::PrimitiveKind::Sphere)
            .collect();
        assert_eq!(cuboids.len(), 9, "eight corners plus a centre cube");
        assert_eq!(spheres.len(), 1, "one sphere to subtract from a side");
        assert_eq!(state.doc.extrusions.len(), 1, "circle extruded through one cube");
        assert_eq!(state.doc.boolean_ops.len(), 1, "sphere subtract is a Combine cut");
        assert_eq!(
            state.doc.boolean_ops.values().next().unwrap().kind,
            crate::model::BooleanOpKind::Cut
        );
        assert_eq!(
            state.doc.edge_treatment_ops.len(),
            2,
            "one chamfer and one fillet"
        );
        let kinds: Vec<_> = state
            .doc
            .edge_treatment_ops
            .values()
            .map(|op| op.kind)
            .collect();
        assert!(kinds.contains(&VertexTreatmentKind::Chamfer));
        assert!(kinds.contains(&VertexTreatmentKind::Fillet));

        let live: Vec<_> = state
            .doc
            .bodies
            .iter()
            .filter(|(_, b)| !b.shadow)
            .collect();
        assert_eq!(live.len(), 9, "nine live coloured bodies (sphere is a shadow)");

        let mats: std::collections::HashSet<_> =
            live.iter().filter_map(|(_, b)| b.material).collect();
        assert_eq!(mats.len(), 9, "each live body keeps a distinct material");

        let mut volumes: Vec<f32> = live
            .iter()
            .filter_map(|(k, _)| {
                crate::extrude::body_solid_mesh(&state.doc, *k)
                    .map(|m| crate::extrude::mesh_signed_volume(&m).abs())
            })
            .collect();
        volumes.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(volumes.len(), 9);
        let full = 20.0 * 20.0 * 20.0;
        let treated = volumes.iter().filter(|&&v| v < full - 50.0).count();
        assert!(
            treated >= 4,
            "hole, sphere bite, chamfer and fillet should each remove volume, got {volumes:?}"
        );

        let centre = cuboids
            .iter()
            .find(|p| (p.origin[0]).abs() < 1e-3 && (p.origin[1]).abs() < 1e-3)
            .expect("a same-size cube centred in the cluster");
        assert_eq!(centre.width, "20");
        assert_eq!(centre.depth, "20");
        assert_eq!(centre.height, "20");
    }

    #[test]
    fn lua_material_rejects_a_bad_colour() {
        let mut runner = ScriptRunner::from_lua_source(
            r##"
            bearcad.new()
            local ok, err = pcall(bearcad.material, { name = "Bad", color = "nope" })
            assert(not ok, "a bad colour should error")
            assert(tostring(err):find("#rrggbb"), "the error should say the form: " .. tostring(err))
        "##,
        )
        .unwrap();
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        let vp = egui::Rect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(960.0, 560.0));
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, Some(vp), &ctx);
        }
        assert!(runner.error.is_none(), "script error: {:?}", runner.error);
    }

    /// #837: a new-body extrude of profiles that don't touch makes one body each; `body =
    /// "join"` puts them all in one.
    #[test]
    fn lua_extrude_splits_unconnected_profiles_into_bodies() {
        let source = r#"
            bearcad.new()
            bearcad.circle{ x = 0, y = 0, r = 5 }
            bearcad.circle{ x = 30, y = 0, r = 5 }
            bearcad.exit_sketch()
            bearcad.extrude{ circles = {0, 1}, distance = 4BODY }
        "#;
        let split = run_lua(&source.replace("BODY", ""));
        assert_eq!(split.doc.extrusions.values().count(), 2);
        assert_eq!(split.doc.bodies.values().count(), 2);

        let joined = run_lua(&source.replace("BODY", r#", body = "join""#));
        assert_eq!(joined.doc.extrusions.values().count(), 1);
        assert_eq!(joined.doc.bodies.values().count(), 1);
        assert_eq!(joined.doc.extrusions[xkey(0)].faces.len(), 2, "both profiles in the one solid");
    }

    /// #797: a dimension value of `name = value` defines the parameter on the spot and
    /// dimensions with it — the scripted twin of typing it into the GUI's value field.
    #[test]
    fn lua_dimension_value_defines_a_parameter_inline() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 40, y1 = 0 }
            bearcad.line{ x = 0, y = 0, x1 = 0, y1 = 30 }
            bearcad.add_constraint({ kind = "line", index = 0 }, "leg = 40mm")
            bearcad.add_angle_constraint{ a = 0, b = 1, value = "corner = 90deg" }
        "#,
        );
        let param = |name: &str| {
            state
                .doc
                .parameters
                .values()
                .find(|p| p.name == name)
                .map(|p| p.expression.clone())
        };
        assert_eq!(param("leg").as_deref(), Some("40mm"));
        assert_eq!(param("corner").as_deref(), Some("90deg"));
        // The constraints reference the parameters, not the literals they were defined with.
        let expressions: Vec<String> = state
            .doc
            .constraints
            .values()
            .map(|c| c.expression.clone())
            .collect();
        assert!(expressions.iter().any(|e| e == "leg"), "got {expressions:?}");
        assert!(expressions.iter().any(|e| e == "corner"), "got {expressions:?}");
    }

    /// #797: an inline definition naming a live parameter redefines it, as in the GUI.
    #[test]
    fn lua_dimension_value_redefines_an_existing_parameter() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.parameter("add", "leg", "10mm")
            bearcad.line{ x = 0, y = 0, x1 = 40, y1 = 0 }
            bearcad.add_constraint({ kind = "line", index = 0 }, "leg = 40mm")
        "#,
        );
        let leg = state
            .doc
            .parameters
            .values()
            .find(|p| p.name == "leg")
            .expect("leg exists");
        assert_eq!(leg.expression, "40mm");
        assert_eq!(
            state.doc.parameters.values().filter(|p| p.name == "leg").count(),
            1,
            "redefining shouldn't add a second row"
        );
    }

    /// #1353: a declarative circle locks its diameter. `add_constraint` must update
    /// that existing dimension instead of erroring "Constraint already exists".
    #[test]
    fn lua_add_constraint_updates_declarative_circle_diameter() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.circle{ x = 5, y = 5, r = 4 }
            bearcad.add_constraint({ kind = "circle", index = 0 }, "d = 15mm")
            local c = bearcad.get{ kind = "circle", index = 0 }
            assert(math.abs(c.diameter - 15) < 1e-3, "diameter stayed " .. tostring(c.diameter))
        "#,
        );
        assert!((state.doc.circles[rkey(0)].diameter() - 15.0).abs() < 1e-3);
        let d = state
            .doc
            .parameters
            .values()
            .find(|p| p.name == "d")
            .expect("inline `d = 15mm` defines a parameter");
        assert_eq!(d.expression, "15mm");
    }

    /// #1353: a declarative rect locks each edge. `add_constraint` on a side
    /// updates the existing LineLength instead of erroring.
    #[test]
    fn lua_add_constraint_updates_declarative_rect_edge() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 40, height = 20 }
            bearcad.add_constraint({ kind = "line", index = 0 }, "50")
            local l = bearcad.get{ kind = "line", index = 0 }
            assert(math.abs(l.length - 50) < 1e-3, "width stayed " .. tostring(l.length))
        "#,
        );
        assert!((state.doc.lines[lkey(0)].length() - 50.0).abs() < 1e-3);
    }

    /// #1353: `edit_dim` / `set_dim` / `commit_dim` must reopen and change a
    /// committed circle diameter, matching the documented (and GUI) path.
    #[test]
    fn lua_edit_dim_updates_committed_circle_diameter() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.circle{ x = 0, y = 0, r = 4 }
            bearcad.edit_dim("diameter")
            bearcad.set_dim("diameter", "20")
            bearcad.commit_dim()
            local c = bearcad.get{ kind = "circle", index = 0 }
            assert(math.abs(c.diameter - 20) < 1e-3, "diameter stayed " .. tostring(c.diameter))
        "#,
        );
        assert!((state.doc.circles[rkey(0)].diameter() - 20.0).abs() < 1e-3);
    }

    /// #1353: same reopen path for a rect's committed width and height.
    #[test]
    fn lua_edit_dim_updates_committed_rect_size() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 40, height = 20 }
            bearcad.edit_dim("width")
            bearcad.set_dim("width", "80")
            bearcad.commit_dim()
            bearcad.edit_dim("height")
            bearcad.set_dim("height", "30")
            bearcad.commit_dim()
            local w = bearcad.get{ kind = "line", index = 0 }
            local h = bearcad.get{ kind = "line", index = 1 }
            assert(math.abs(w.length - 80) < 1e-3, "width stayed " .. tostring(w.length))
            assert(math.abs(h.length - 30) < 1e-3, "height stayed " .. tostring(h.length))
        "#,
        );
        assert!((state.doc.lines[lkey(0)].length() - 80.0).abs() < 1e-3);
        assert!((state.doc.lines[lkey(1)].length() - 30.0).abs() < 1e-3);
    }

    /// #1353: a committed line length (via `dimension=`) is also editable this way;
    /// the reopen must not silently no-op.
    #[test]
    fn lua_edit_dim_updates_committed_line_length() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 40, y1 = 0, dimension = 40 }
            bearcad.edit_dim("length")
            bearcad.set_dim("length", "50")
            bearcad.commit_dim()
            local l = bearcad.get{ kind = "line", index = 0 }
            assert(math.abs(l.length - 50) < 1e-3, "length stayed " .. tostring(l.length))
        "#,
        );
        assert!((state.doc.lines[lkey(0)].length() - 50.0).abs() < 1e-3);
    }

    /// #809: positioning dimensions from scripts — a point off an edge, two points apart,
    /// and the spacing between two parallel lines. The side each is measured on is captured
    /// from the geometry, as it is for an interactive pick.
    #[test]
    fn lua_positioning_dimensions_are_scriptable() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 40, y1 = 0 }
            bearcad.circle{ x = 12, y = 9, r = 3 }
            bearcad.add_constraint({ kind = "point_line",
                                     point = { kind = "circle", index = 0 },
                                     line  = { kind = "line", index = 0 } }, "15mm")
            bearcad.add_constraint({ kind = "point_line",
                                     point = { kind = "circle", index = 0 },
                                     line  = { kind = "axis", axis = "y" } }, "8mm")
        "#,
        );
        let circle = &state.doc.circles[rkey(0)];
        assert!((circle.cy - 15.0).abs() < 0.05, "off the edge: cy={}", circle.cy);
        assert!((circle.cx - 8.0).abs() < 0.05, "off the Y axis: cx={}", circle.cx);

        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 40, y1 = 0 }
            bearcad.circle{ x = 12, y = 9, r = 3 }
            bearcad.add_constraint({ kind = "point_point",
                                     anchor = { kind = "line", index = 0, ["end"] = "start" },
                                     mover  = { kind = "circle", index = 0 } }, "25mm")
        "#,
        );
        let circle = &state.doc.circles[rkey(0)];
        let dist = (circle.cx * circle.cx + circle.cy * circle.cy).sqrt();
        assert!((dist - 25.0).abs() < 0.1, "point-to-point: {dist}");

        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 40, y1 = 0 }
            bearcad.line{ x = 0, y = 20, x1 = 40, y1 = 20 }
            bearcad.add_geometric_constraint("parallel",
                { kind = "line", index = 0 }, { kind = "line", index = 1 })
            bearcad.add_constraint({ kind = "line_line",
                                     a = { kind = "line", index = 0 },
                                     b = { kind = "line", index = 1 } }, "12mm")
        "#,
        );
        let line = &state.doc.lines[lkey(1)];
        assert!((line.y0 - 12.0).abs() < 0.1, "line spacing: y0={}", line.y0);

        // #1436: origin + a circle centre is a scriptable point-to-point distance.
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.circle{ x = 8, y = 0, r = 3 }
            bearcad.add_constraint({ kind = "point_point",
                                     anchor = { kind = "origin" },
                                     mover  = { kind = "circle", index = 0 } }, "12mm")
        "#,
        );
        let circle = &state.doc.circles[rkey(0)];
        let dist = (circle.cx * circle.cx + circle.cy * circle.cy).sqrt();
        assert!(
            (dist - 12.0).abs() < 0.1,
            "origin-to-circle distance: {dist}"
        );
    }

    #[test]
    fn lua_unknown_constraint_target_names_the_valid_ones() {
        let mut runner = ScriptRunner::from_lua_source(
            r#"
            bearcad.new()
            local ok, err = pcall(bearcad.add_constraint, { kind = "widget", index = 0 }, "5mm")
            assert(not ok, "an unknown target should error")
            assert(tostring(err):find("point_line"), "the error should name the valid kinds: " .. tostring(err))
        "#,
        )
        .unwrap();
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        let vp = egui::Rect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(960.0, 560.0));
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, Some(vp), &ctx);
        }
        assert!(runner.error.is_none(), "script error: {:?}", runner.error);
    }

    #[test]
    fn lua_equal_constraint_is_scriptable() {
        // #47: the Equal constraint is reachable from scripting via
        // add_geometric_constraint("equal"); it records an Equal constraint between the
        // two selected edges. (The geometric effect on unlocked lines is covered by the
        // solver/geometric_constraints unit tests; lines drawn with the tool also carry
        // auto length locks, so this test only asserts the constraint is created.)
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0, name = "a" }
            bearcad.line{ x = 0, y = 5, x1 = 3, y1 = 5, name = "b" }
            bearcad.select("a")
            bearcad.select("b", true)
            bearcad.add_geometric_constraint("equal")
        "#,
        );
        assert!(
            state
                .doc
                .constraints
                .values()
                .any(|c| matches!(c.kind, crate::model::ConstraintKind::Equal { .. })),
            "an Equal constraint should have been created"
        );
    }

    #[test]
    fn lua_select_line_endpoint_makes_two_lines_coincident() {
        // #68: bearcad.select can now target an individual point (a line endpoint or rect
        // corner), not just a whole element — this closes a loop of plain lines purely from
        // Lua, the motivating case from the issue (needed to test #66 closed-loop detection
        // end-to-end without simulating mouse clicks).
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0, name = "a" }
            bearcad.line{ x = 20, y = 0, x1 = 30, y1 = 0, name = "b" }
            bearcad.select{ kind = "line", index = 0, ["end"] = "end" }
            bearcad.select({ kind = "line", index = 1, ["end"] = "start" }, true)
            bearcad.add_geometric_constraint("coincident")
        "#,
        );
        let end_point = crate::model::ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
            line: lkey(0),
            end: LineEnd::End,
        });
        let start_point = crate::model::ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
            line: lkey(1),
            end: LineEnd::Start,
        });
        assert!(
            state.doc.constraints.values().any(|c| {
                matches!(
                        &c.kind,
                        crate::model::ConstraintKind::Coincident { a, b }
                            if (*a == end_point && *b == start_point)
                                || (*a == start_point && *b == end_point)
                    )
            }),
            "expected a Coincident constraint between the two selected line endpoints, got: {:?}",
            state.doc.constraints
        );
    }

    #[test]
    fn lua_select_circle_center_with_explicit_point_flag() {
        // #68: kind="circle" alone still selects the whole circle (unchanged); `point = true`
        // is required to target just its center point.
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.circle{ x = 0, y = 0, r = 5, name = "hole" }
            bearcad.select{ kind = "circle", index = 0, point = true }
        "#,
        );
        assert_eq!(
            state.scene_selection.iter().next(),
            Some(SceneElement::Point(ConstraintPoint::CircleCenter(rkey(0))))
        );
    }

    #[test]
    fn lua_line_creates_line_on_ground_plane() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ length = 80, name = "Guide" }
        "#,
        );
        assert_eq!(state.doc.lines.len(), 1);
        assert!((state.doc.lines[lkey(0)].length() - 80.0).abs() < 1e-2);
        assert_eq!(
            find_element_by_name(&state.doc, "Guide"),
            Some(SceneElement::Line(lkey(0)))
        );
    }

    /// Builds a state with a corner (two lines coincident at (10,0), the second running to
    /// `b_far`) and runs `source` against it. Pre-builds the coincident vertex directly in Rust
    /// (rather than via `bearcad.select{..., end=...}` + `add_geometric_constraint("coincident")`,
    /// #68) for brevity, then lets the script call `bearcad.chamfer_vertex`/`fillet_vertex`
    /// against it. Returns the final state and any script error.
    fn run_lua_against_corner(source: &str, b_far: (f32, f32)) -> (AppState, Option<String>) {
        use crate::model::{Constraint, ConstraintEntity, ConstraintKind, Line, LineEnd, ShapeKind};

        let mut runner = ScriptRunner::from_lua_source(source).unwrap();
        runner.verbose = false;
        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        state.doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state
            .doc
            .lines
            .insert(Line::from_local_endpoints(sketch, 10.0, 0.0, b_far.0, b_far.1));
        state.doc.shape_order.extend([ShapeKind::Line, ShapeKind::Line]);
        state.doc.constraints.insert(Constraint {
            sketch,
            kind: ConstraintKind::Coincident {
                a: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: lkey(0),
                    end: LineEnd::End,
                }),
                b: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: lkey(1),
                    end: LineEnd::Start,
                }),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
        });
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, None, &ctx);
        }
        (state, runner.error)
    }

    /// [`run_lua_against_corner`] with a 90-degree corner and no script error expected.
    fn run_lua_against_a_right_angle_corner(source: &str) -> AppState {
        let (state, error) = run_lua_against_corner(source, (10.0, 10.0));
        assert!(error.is_none(), "script error: {error:?}");
        state
    }

    #[test]
    fn lua_chamfer_vertex_truncates_and_bridges_the_corner() {
        let state = run_lua_against_a_right_angle_corner(
            r#"
            bearcad.chamfer_vertex{
                point = { kind = "line", index = 0, ["end"] = "end" },
                distance = 3,
            }
        "#,
        );
        // #538: two sources shadowed + two trimmed copies + one bridge = 5 lines.
        assert_eq!(state.doc.lines.len(), 5, "trimmed copies + a bridge should be added");
        assert_eq!(state.doc.sketch_vertex_treatment_ops.len(), 1);
        let bridge = state.doc.lines.values().last().unwrap();
        assert!(!bridge.is_curved(), "chamfer bridges with a straight line");
    }

    #[test]
    fn lua_fillet_vertex_bridges_with_a_curve() {
        let state = run_lua_against_a_right_angle_corner(
            r#"
            bearcad.fillet_vertex{
                point = { kind = "line", index = 0, ["end"] = "end" },
                radius = 3,
            }
        "#,
        );
        // #538: two sources shadowed + two trimmed copies + one bridge = 5 lines.
        assert_eq!(state.doc.lines.len(), 5, "trimmed copies + a bridge should be added");
        let bridge = state.doc.lines.values().last().unwrap();
        assert!(bridge.is_curved(), "fillet bridges with a curved line");
    }

    /// #110: a corner within ~1° of straight (SPEC §3.1) must be *rejected at commit*, not
    /// silently accepted into a micro-bridge. The second line here leaves the shared vertex
    /// (10,0) toward (20, 0.01) — about 0.06° off dead-straight from the first line.
    #[test]
    fn lua_fillet_vertex_errors_on_a_near_straight_corner() {
        let (state, error) = run_lua_against_corner(
            r#"
            local ok, err = pcall(bearcad.fillet_vertex, {
                point = { kind = "line", index = 0, ["end"] = "end" },
                radius = 3,
            })
            assert(not ok, "near-straight corner fillet should error")
            assert(tostring(err):find("degenerate"), "unexpected error: " .. tostring(err))
        "#,
            (20.0, 0.01),
        );
        assert!(error.is_none(), "script error: {error:?}");
        assert_eq!(state.doc.lines.len(), 2, "no bridging line should be created");
    }

    /// #109: fillet/chamfer at a vertex that only one line touches must error (previously a
    /// silent no-op), and create nothing.
    #[test]
    fn lua_fillet_vertex_errors_on_a_one_line_vertex() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0 }
            local ok, err = pcall(bearcad.fillet_vertex, {
                point = { kind = "line", index = 0, ["end"] = "end" },
                radius = 3,
            })
            assert(not ok, "fillet at a one-line vertex should error")
            assert(tostring(err):find("exactly two lines"), "unexpected error: " .. tostring(err))
            assert(bearcad.count("line") == 1, "no bridging line should be created")
            local ok2, err2 = pcall(bearcad.chamfer_vertex, {
                point = { kind = "line", index = 0, ["end"] = "end" },
                distance = 3,
            })
            assert(not ok2, "chamfer at a one-line vertex should error")
            assert(bearcad.count("line") == 1, "no bridging line should be created")
        "#,
        );
        assert_eq!(state.doc.lines.len(), 1);
    }

    /// #109: a vertex where three lines join is just as invalid for chamfer/fillet as one.
    #[test]
    fn lua_fillet_vertex_errors_on_a_three_line_vertex() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0 }
            bearcad.line{ x = 10, y = 0, x1 = 20, y1 = 5 }
            bearcad.line{ x = 10, y = 0, x1 = 10, y1 = 10 }
            bearcad.select{ kind = "line", index = 0, ["end"] = "end" }
            bearcad.select({ kind = "line", index = 1, ["end"] = "start" }, true)
            bearcad.add_geometric_constraint("coincident")
            bearcad.select{ kind = "line", index = 0, ["end"] = "end" }
            bearcad.select({ kind = "line", index = 2, ["end"] = "start" }, true)
            bearcad.add_geometric_constraint("coincident")
            local ok, err = pcall(bearcad.fillet_vertex, {
                point = { kind = "line", index = 0, ["end"] = "end" },
                radius = 3,
            })
            assert(not ok, "fillet at a three-line vertex should error")
            assert(tostring(err):find("exactly two lines"), "unexpected error: " .. tostring(err))
            assert(bearcad.count("line") == 3, "no bridging line should be created")
        "#,
        );
        assert_eq!(state.doc.lines.len(), 3);
    }

    /// #104: degenerate (zero-size) rect/circle/line calls must raise Lua errors and create
    /// nothing, instead of silently succeeding.
    #[test]
    fn lua_zero_size_shapes_error_and_create_nothing() {
        let state = run_lua(
            r#"
            bearcad.new()
            local ok, err = pcall(bearcad.rect, { width = 0, height = 0 })
            assert(not ok, "zero-size rect should error")
            assert(tostring(err):find("width and height"), "unexpected error: " .. tostring(err))
            local ok2, err2 = pcall(bearcad.circle, { r = 0 })
            assert(not ok2, "zero-radius circle should error")
            assert(tostring(err2):find("radius"), "unexpected error: " .. tostring(err2))
            local ok3, err3 = pcall(bearcad.line, { x = 20, y = 0, x1 = 20, y1 = 0 })
            assert(not ok3, "zero-length line should error")
            assert(tostring(err3):find("too short"), "unexpected error: " .. tostring(err3))
            assert(bearcad.count("line") == 0, "no lines should be created")
            assert(bearcad.count("circle") == 0, "no circles should be created")
        "#,
        );
        assert_eq!(state.doc.lines.len(), 0);
        assert_eq!(state.doc.circles.len(), 0);
    }

    /// #104: a zero-distance extrude must error and create nothing (previously it created an
    /// invisible extrusion).
    #[test]
    fn lua_zero_distance_extrude_errors_and_creates_nothing() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
            local ok, err = pcall(bearcad.extrude, { polygon = {0, 1, 2, 3}, distance = 0 })
            assert(not ok, "zero-distance extrude should error")
            assert(tostring(err):find("non%-zero"), "unexpected error: " .. tostring(err))
            assert(bearcad.count("extrusion") == 0, "no extrusion should be created")
        "#,
        );
        assert_eq!(state.doc.extrusions.len(), 0);
        assert_eq!(state.doc.bodies.len(), 0);
    }

    /// #112: extruding a polygon face whose line indices don't exist (or don't form a closed
    /// loop) must error and create nothing, instead of creating a dead extrusion.
    #[test]
    fn lua_extrude_errors_on_a_missing_polygon_line() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
            local ok, err = pcall(bearcad.extrude, {
                polygon = {0, 1, 2, 99}, distance = 5, body = "merge",
            })
            assert(not ok, "extrude with a nonexistent line index should error")
            -- The ordinal is resolved to a key at the script boundary (#1055), so a line that
            -- isn't there is caught by name before the loop check ever runs.
            assert(tostring(err):find("no line 99"), "unexpected error: " .. tostring(err))
            assert(bearcad.count("extrusion") == 0, "extrusion count must be unchanged")
        "#,
        );
        assert_eq!(state.doc.extrusions.len(), 0);
    }

    /// #112: line indices that all exist but don't form a closed loop are rejected too.
    #[test]
    fn lua_extrude_errors_on_a_non_loop_polygon() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
            local ok, err = pcall(bearcad.extrude, { polygon = {0, 1, 2}, distance = 5 })
            assert(not ok, "extrude with an open line set should error")
            assert(tostring(err):find("closed loop"), "unexpected error: " .. tostring(err))
            assert(bearcad.count("extrusion") == 0, "extrusion count must be unchanged")
        "#,
        );
        assert_eq!(state.doc.extrusions.len(), 0);
    }

    /// #77: `bearcad.chamfer_edge`/`fillet_edge` chamfer/fillet an analytic edge of an
    /// extrusion's 3D solid — declared directly (extrusion index + structured edge reference),
    /// not via screen-space picking.
    #[test]
    fn lua_chamfer_edge_bevels_a_vertical_edge_and_visibly_changes_the_mesh() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.chamfer_edge{
                extrusion = 0,
                edge = { kind = "vertical", face = 0, edge = 0 },
                distance = 2,
            }
        "#,
        );
        assert_eq!(state.doc.edge_treatment_ops.len(), 1);
        assert_eq!(state.doc.edge_treatment_ops.values().nth(0).unwrap().kind, VertexTreatmentKind::Chamfer);
        // The chamfer's beveled output body has more than the 12 triangles of the plain box.
        let output = state.doc.edge_treatment_ops.values().nth(0).unwrap().outputs[0];
        let mesh = crate::extrude::body_solid_mesh(&state.doc, output).unwrap();
        assert_ne!(mesh.triangles.len(), 12);
    }

    /// #672: `edges = { ... }` treats the whole set in ONE operation (one undo, one amount).
    /// Sequential one-edge calls now chain onto the live body (#1323); a set is still the
    /// way to apply the same radius to several edges at once.
    #[test]
    fn lua_fillet_edge_treats_an_edge_set_as_one_operation() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 0, y = 0, width = 80, height = 50 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
            bearcad.fillet_edge{
                extrusion = 0,
                edges = {
                    { kind = "vertical", face = 0, edge = 0 },
                    { kind = "vertical", face = 0, edge = 1 },
                    { kind = "vertical", face = 0, edge = 2 },
                    { kind = "vertical", face = 0, edge = 3 },
                },
                radius = 8,
            }
        "#,
        );
        assert_eq!(state.doc.edge_treatment_ops.len(), 1, "one operation, not one per edge");
        assert_eq!(state.doc.edge_treatment_ops.values().nth(0).unwrap().edges.len(), 4);
        // One output body, and it carries all four rounds (far more than the box's 12 triangles).
        assert_eq!(state.doc.edge_treatment_ops.values().nth(0).unwrap().outputs.len(), 1);
        let output = state.doc.edge_treatment_ops.values().nth(0).unwrap().outputs[0];
        let mesh = crate::extrude::body_solid_mesh(&state.doc, output).unwrap();
        assert!(mesh.triangles.len() > 12, "{} triangles", mesh.triangles.len());
    }

    #[test]
    fn lua_fillet_edge_bevels_a_cap_edge_with_a_faceted_arc() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.fillet_edge{
                extrusion = 0,
                edge = { kind = "cap", face = 0, edge = 1, top = true },
                radius = 1.5,
            }
        "#,
        );
        assert_eq!(state.doc.edge_treatment_ops.len(), 1);
        assert_eq!(state.doc.edge_treatment_ops.values().nth(0).unwrap().kind, VertexTreatmentKind::Fillet);
        assert!(matches!(
            state.doc.edge_treatment_ops.values().nth(0).unwrap().edges[0].edge,
            ExtrusionEdgeRef::Cap { face: 0, edge: 1, top: true }
        ));
    }

    /// #1321: `fillet_edge` with radius 0 is a no-op — no error, no operation, no extra body.
    #[test]
    fn lua_fillet_edge_zero_radius_is_a_noop() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.fillet_edge{
                extrusion = 0,
                edge = { kind = "vertical", face = 0, edge = 0 },
                radius = 0,
            }
            assert(bearcad.count("body") == 1, "zero-radius fillet must not spawn a body")
        "#,
        );
        assert!(
            state.doc.edge_treatment_ops.is_empty(),
            "zero-radius fillet must not create an operation"
        );
        assert!(!state.doc.bodies.values().nth(0).unwrap().shadow);
    }

    /// #1323: two sequential `fillet_edge` calls on different edges of the same body chain —
    /// the second consumes the first's output — instead of forking two sibling bodies.
    #[test]
    fn lua_fillet_edge_stacks_a_second_fillet_on_the_same_body() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 0, y = 0, width = 20, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
            bearcad.fillet_edge{
                extrusion = 0,
                edge = { kind = "cap", face = 0, edge = 2, top = true },
                radius = 2.3,
            }
            local v1 = bearcad.body_stats(1).volume
            bearcad.fillet_edge{
                extrusion = 0,
                edge = { kind = "cap", face = 0, edge = 0, top = true },
                radius = 0.5,
            }
            -- Body 0 original (shadow), body 1 first fillet (shadow), body 2 both (live).
            local v2 = bearcad.body_stats(2).volume
            assert(v2 < v1 - 0.1, "stacked fillet must cut more than the first alone: " .. v2 .. " vs " .. v1)
        "#,
        );
        assert_eq!(state.doc.edge_treatment_ops.len(), 2);
        let ops: Vec<_> = state.doc.edge_treatment_ops.values().collect();
        assert_eq!(ops[1].targets, ops[0].outputs);
        let live: Vec<_> = state
            .doc
            .bodies
            .iter()
            .filter(|(_, b)| !b.shadow)
            .map(|(k, _)| k)
            .collect();
        assert_eq!(live, ops[1].outputs);
    }

    /// #192/#531: a fillet shows in the Elements pane as its own operation node (with its
    /// beveled output body nested under it), labelled by kind; re-opening it for edit and
    /// committing a new amount rebuilds the operation rather than stacking a second one.
    #[test]
    fn edge_treatment_is_an_editable_operation_element() {
        let mut state = run_lua(
            r#"
            bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.fillet_edge{
                extrusion = 0,
                edge = { kind = "vertical", face = 0, edge = 0 },
                radius = 1.5,
            }
        "#,
        );
        // It appears as a top-level operation node, labelled by its kind.
        let node = crate::hierarchy::HierarchyNode::EdgeTreatmentOp(etkey(0));
        let nodes = crate::hierarchy::build_element_list(&state.doc, state.sketch_session);
        assert!(nodes.contains(&node), "fillet op should show in the elements pane");
        assert!(crate::names::node_label(&state.doc, node).starts_with("Fillet"));
        // Its beveled output body nests under the operation node in the real tree.
        let tree = crate::hierarchy::build_hierarchy(&state.doc, state.sketch_session);
        let op_entry = crate::hierarchy::find_hierarchy_entry(&tree, node).expect("op entry");
        let output = state.doc.edge_treatment_ops.values().nth(0).unwrap().outputs[0];
        assert!(op_entry
            .children
            .iter()
            .any(|c| c.node == crate::hierarchy::HierarchyNode::Body(output)));

        // Re-opening the op for edit and committing a new amount leaves one live operation.
        assert_eq!(
            state.apply(crate::actions::Action::EditEdgeTreatmentOp {
                op: crate::model::edge_treatment_op_key_for_slot(0)
            }),
            crate::actions::ActionResult::Ok
        );
        let edge = crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 0 };
        assert_eq!(
            state.apply(crate::actions::Action::CommitEdgeTreatments {
                edges: vec![(crate::model::TreatableSolid::Extrusion(xkey(0)), edge)],
                kind: VertexTreatmentKind::Fillet,
                amount: 2.75,
            }),
            crate::actions::ActionResult::Ok
        );
        let live: Vec<_> = state
            .doc
            .edge_treatment_ops
            .values()
            .collect();
        assert_eq!(live.len(), 1);
        assert!((live[0].amount - 2.75).abs() < 1e-4);
    }

    /// #1329: `fillet_edge` / `chamfer_edge` treat a Shape-tool cuboid the same way they
    /// treat a rectangular extrusion — `primitive =` names the shape, `edge` is the same
    /// vertical/cap address.
    #[test]
    fn lua_fillet_edge_treats_a_cuboid_primitive() {
        let state = run_lua(
            r#"
            bearcad.cuboid{ width = 40, depth = 50, height = 22 }
            local v0 = bearcad.body_stats(0).volume
            bearcad.fillet_edge{
                primitive = 0,
                edge = { kind = "cap", face = 0, edge = 0, top = true },
                radius = 3,
            }
            local v1 = bearcad.body_stats(1).volume
            assert(v1 < v0 - 1, "fillet must cut the cuboid: " .. v1 .. " vs " .. v0)
        "#,
        );
        assert_eq!(state.doc.edge_treatment_ops.len(), 1);
        assert_eq!(
            state.doc.edge_treatment_ops.values().nth(0).unwrap().kind,
            VertexTreatmentKind::Fillet
        );
        let live: Vec<_> = state
            .doc
            .bodies
            .iter()
            .filter(|(_, b)| !b.shadow)
            .map(|(k, _)| k)
            .collect();
        assert_eq!(live.len(), 1, "one live filleted body");
    }

    /// #1324: fillets created through the scripted API must not draw a Document spoke
    /// once they have the treated body as an input.
    #[test]
    fn lua_fillet_has_no_document_graph_spoke() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 0, y = 0, width = 20, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
            bearcad.fillet_edge{
                extrusion = 0,
                edge = { kind = "cap", face = 0, edge = 0, top = true },
                radius = 2.3,
            }
            bearcad.fillet_edge{
                extrusion = 0,
                edge = { kind = "cap", face = 0, edge = 1, top = true },
                radius = 0.5,
            }
            bearcad.fillet_edge{
                extrusion = 0,
                edge = { kind = "cap", face = 0, edge = 2, top = true },
                radius = 7.0,
            }
            "#,
        );
        let tree = crate::hierarchy::build_hierarchy(&state.doc, state.sketch_session);
        let positions = crate::hierarchy::graph_node_positions(&tree);
        let parents = crate::hierarchy::graph_parent_edges(&positions, &state.doc);
        assert_eq!(state.doc.edge_treatment_ops.len(), 3);
        for (oi, _) in state.doc.edge_treatment_ops.iter() {
            let fillet = crate::hierarchy::HierarchyNode::EdgeTreatmentOp(oi);
            assert!(
                !parents.iter().any(|(p, c)| {
                    *p == crate::hierarchy::HierarchyNode::Document && *c == fillet
                }),
                "fillet {oi:?} must not connect to Document"
            );
        }
        assert!(
            parents.iter().any(|(p, c)| {
                *p == crate::hierarchy::HierarchyNode::Document
                    && matches!(c, crate::hierarchy::HierarchyNode::ConstructionPlane(_))
            }),
            "root planes still hang off Document"
        );
    }

    /// #1425: moving a cuboid shadows its body; with shadows hidden (the default graph
    /// filter) the Move must still dash to the cuboid Shape that produced that body.
    #[test]
    fn lua_hidden_shadow_move_skips_to_the_shape() {
        use crate::hierarchy::{
            build_hierarchy, filter_hierarchy, graph_node_positions, graph_shadow_skip_edges,
            prune_shadow_bodies, ElementFilter, HierarchyNode,
        };
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.cuboid{ width = 20, depth = 20, height = 20 }
            bearcad.cuboid{ width = 30, depth = 10, height = 40 }
            bearcad.move_bodies{ bodies = {0}, x = 10 }
            "#,
        );
        let filter = ElementFilter::default();
        assert!(!filter.shadow_bodies);
        let mut tree = filter_hierarchy(&build_hierarchy(&state.doc, None), &filter);
        prune_shadow_bodies(&mut tree, &state.doc);
        let present: std::collections::HashSet<HierarchyNode> =
            graph_node_positions(&tree).into_iter().map(|p| p.node).collect();

        let (shadow_bi, _) = state
            .doc
            .bodies
            .iter()
            .find(|(_, b)| b.shadow)
            .expect("move shadows the first cuboid");
        let shape = match &state.doc.bodies[shadow_bi].source {
            crate::model::BodySource::Primitive(pi) => *pi,
            other => panic!("shadowed body should be the cuboid primitive, got {other:?}"),
        };
        let (move_op, _) = state.doc.move_ops.iter().next().expect("one move");
        assert!(
            !present.contains(&HierarchyNode::Body(shadow_bi)),
            "default graph hides the shadow body"
        );
        let skips = graph_shadow_skip_edges(&state.doc, &present);
        assert!(
            skips.contains(&(HierarchyNode::Shape(shape), HierarchyNode::MoveOp(move_op))),
            "Move should dash to the cuboid that produced the hidden shadow: {skips:?}"
        );
    }

    #[test]
    fn lua_chamfer_edge_rejects_an_out_of_range_edge() {
        // `tick.exec` turns a failed declarative-modeling action into a Lua error
        // (#104/#109/#110/#112) — catchable with `pcall` — in addition to reporting it
        // through `AppState::status` like the interactive gizmo tool would see it.
        let state = run_lua(
            r#"
            bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            local ok, err = pcall(bearcad.chamfer_edge, {
                extrusion = 0,
                edge = { kind = "vertical", face = 0, edge = 99 },
                distance = 2,
            })
            assert(not ok, "an out-of-range edge should error")
            assert(tostring(err):lower():find("edge"), "unexpected error: " .. tostring(err))
        "#,
        );
        assert!(
            state.doc.extrusions[xkey(0)].edge_treatments.is_empty(),
            "an out-of-range edge shouldn't be stored"
        );
        assert!(
            state.status.to_ascii_lowercase().contains("edge"),
            "status should explain the rejection: {}",
            state.status
        );
    }

    #[test]
    fn lua_line_with_bezier_creates_a_curve() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0, bezier = { {3, 4}, {7, 4} }, name = "Curve" }
        "#,
        );
        assert_eq!(state.doc.lines.len(), 1);
        let line = &state.doc.lines[lkey(0)];
        assert!(line.is_curved());
        assert_eq!(line.bezier, Some([(3.0, 4.0), (7.0, 4.0)]));
        assert_eq!(
            find_element_by_name(&state.doc, "Curve"),
            Some(SceneElement::Line(lkey(0)))
        );
    }

    #[test]
    fn lua_get_line_length_reports_arc_length_for_curves() {
        run_lua_expect_ok(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0, bezier = { {3, 4}, {7, 4} } }
            local l = bearcad.get{ kind = "line", index = 0 }
            assert(l.curved == true)
            -- Arc length of the curve, not the 10 mm endpoint chord.
            assert(l.length > 10.5, "expected arc length > chord, got " .. l.length)
        "#,
        );
    }

    /// #105: one `undo()` reverts one whole user action — the entire rectangle
    /// gesture (4 lines + its coincident/H/V/dimension constraints), not a single
    /// shape_order entry at a time.
    #[test]
    fn lua_undo_reverts_a_whole_rectangle_gesture() {
        run_lua_expect_ok(
            r#"
            bearcad.new()
            bearcad.rect{ width = 40, height = 30 }
            bearcad.circle{ x = 60, y = 0, r = 8 }
            -- First undo removes only the circle gesture...
            bearcad.undo()
            assert(bearcad.count("circle") == 0, "circle should be undone first")
            assert(bearcad.count("line") == 4, "rect must survive the circle undo")
            -- ...second undo removes the whole rectangle in ONE step.
            bearcad.undo()
            assert(bearcad.count("line") == 0, "one undo must revert all 4 rect lines")
            assert(bearcad.count("constraint") == 0, "and every rect constraint")
        "#,
        );
    }

    /// #105: a cut extrusion undoes as one gesture — the cut extrusion disappears
    /// and the target body's volume is restored.
    #[test]
    fn lua_undo_reverts_a_cut_extrusion_gesture() {
        run_lua_expect_ok(
            r#"
            bearcad.new()
            bearcad.rect{ width = 40, height = 30 }
            bearcad.extrude{ polygon = {0,1,2,3}, distance = 20 }
            bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 0,
                                  profile = "polygon", profile_lines = {0,1,2,3}, top = true }
            bearcad.circle{ x = 10, y = 10, r = 5 }
            bearcad.extrude{ circle = 0, distance = -25, body = "cut" }
            assert(bearcad.body_stats(0).volume < 23999, "cut should remove volume")
            bearcad.undo()
            local v = bearcad.body_stats(0).volume
            assert(math.abs(v - 24000) < 1, "cut undo must restore the body, got " .. v)
            assert(bearcad.count("extrusion") == 1, "cut extrusion removed from the doc")
        "#,
        );
    }

    /// #106: file-I/O failures surface as catchable Lua errors instead of silent
    /// success (previously `import_step` on a missing file "succeeded" with an
    /// empty document).
    #[test]
    fn lua_import_step_missing_file_raises() {
        run_lua_expect_ok(
            r#"
            bearcad.new()
            local ok = pcall(function() bearcad.import_step("/nonexistent/nope.step") end)
            assert(not ok, "importing a missing STEP file must raise")
            assert(bearcad.count("body") == 0)
        "#,
        );
    }

    /// #1284: `bearcad.export_3mf` writes a ZIP package with a 3D model part.
    #[test]
    fn lua_export_3mf_writes_a_package() {
        let path = std::env::temp_dir().join("bearcad_lua_export.3mf");
        let named = std::env::temp_dir().join("bearcad_lua_export_named.3mf");
        let path_str = path.to_string_lossy().replace('\\', "\\\\");
        let named_str = named.to_string_lossy().replace('\\', "\\\\");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&named);
        run_lua_expect_ok(&format!(
            r#"
            bearcad.new()
            bearcad.rect{{ width = 40, height = 30 }}
            bearcad.extrude{{ polygon = {{0,1,2,3}}, distance = 10 }}
            assert(bearcad.count("body") == 1)
            bearcad.set_name(bearcad.element("body", 0), "Block")
            bearcad.export_3mf("{path_str}")
            bearcad.export_3mf("{named_str}", "Block")
        "#
        ));
        for p in [&path, &named] {
            let bytes = std::fs::read(p).expect("exported 3mf");
            let _ = std::fs::remove_file(p);
            assert!(bytes.len() > 100, "3mf too small: {}", bytes.len());
            assert_eq!(&bytes[0..4], b"PK\x03\x04", "3mf must be a ZIP package");
        }
    }

    /// #106: a single-body document exports real BREP STEP in kernel builds, and a
    /// curved fillet survives the export → import round-trip.
    #[test]
    fn lua_step_roundtrip_preserves_curved_brep() {
        let path = std::env::temp_dir().join("bearcad_lua_rt.step");
        let path_str = path.to_string_lossy().replace('\\', "\\\\");
        run_lua_expect_ok(&format!(
            r#"
            bearcad.new()
            bearcad.rect{{ width = 40, height = 30 }}
            bearcad.extrude{{ polygon = {{0,1,2,3}}, distance = 20 }}
            bearcad.fillet_edge{{ extrusion = 0, edge = {{ kind = "vertical", face = 0, edge = 1 }}, radius = 8 }}
            -- Body 0 is now the shadowed (unbeveled) input; body 1 is the filleted output (#531).
            local v0 = bearcad.body_stats(1).volume
            bearcad.export_step("{path_str}")
            bearcad.new()
            bearcad.import_step("{path_str}")
            assert(bearcad.count("body") == 1, "round-trip must import one body")
            local v1 = bearcad.body_stats(0).volume
            assert(math.abs(v1 - v0) < v0 * 0.005,
                   "curved fillet must survive: " .. v0 .. " -> " .. v1)
        "#
        ));
        let text = std::fs::read_to_string(&path).expect("exported file");
        let _ = std::fs::remove_file(&path);
        assert!(
            text.contains("ADVANCED_FACE"),
            "single-body export must be real BREP, not the faceted fallback"
        );
    }

    /// #105: legacy documents (no recorded boundaries) keep the old per-entry undo.
    #[test]
    fn undo_removes_the_whole_last_gesture() {
        // Checkpoint undo (#194) reverts a whole user gesture at once: a rectangle (its
        // sketch + four lines + constraints) undoes in a single step back to empty.
        let mut state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 40, height = 30 }
        "#,
        );
        assert_eq!(state.doc.lines.len(), 4, "the rectangle created four lines");
        state.apply(crate::actions::Action::UndoLast);
        assert!(
            state.doc.lines.is_empty(),
            "undo removes the entire rectangle gesture, not one line"
        );
    }

    #[test]
    fn lua_circle_creates_circle_on_ground_plane() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.circle{ x = 10, y = 5, r = 12, name = "Hole" }
        "#,
        );
        assert_eq!(state.doc.circles.len(), 1);
        let circle = &state.doc.circles[rkey(0)];
        assert!((circle.cx - 10.0).abs() < 1e-3 && (circle.cy - 5.0).abs() < 1e-3);
        assert!((circle.r - 12.0).abs() < 1e-3);
        assert_eq!(
            find_element_by_name(&state.doc, "Hole"),
            Some(SceneElement::Circle(rkey(0)))
        );
    }

    #[test]
    fn lua_circle_accepts_diameter() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.circle{ diameter = 30 }
        "#,
        );
        assert_eq!(state.doc.circles.len(), 1);
        assert!((state.doc.circles[rkey(0)].r - 15.0).abs() < 1e-3);
    }

    #[test]
    fn lua_import_stl_adds_a_body() {
        let path = std::env::temp_dir().join(format!("bearcad_lua_import_{}.stl", std::process::id()));
        std::fs::write(
            &path,
            "solid tri\n  facet normal 0 0 1\n    outer loop\n      vertex 0 0 0\n      vertex 1 0 0\n      vertex 0 1 0\n    endloop\n  endfacet\nendsolid tri\n",
        )
        .unwrap();
        let path_str = path.to_string_lossy().replace('\\', "\\\\");
        let state = run_lua(&format!(
            r#"
            bearcad.new()
            bearcad.import_stl("{path_str}")
        "#
        ));
        assert_eq!(state.doc.imported_meshes.len(), 1);
        assert_eq!(state.doc.bodies.len(), 1);
        assert_eq!(
            state.doc.bodies.values().nth(0).unwrap().source,
            crate::model::BodySource::Imported(crate::arena::Key::from_bits(0))
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn lua_import_step_adds_a_body() {
        let path = std::env::temp_dir().join(format!("bearcad_lua_import_{}.step", std::process::id()));
        let mesh = crate::extrude::SolidMesh {
            triangles: vec![[
                glam::Vec3::new(0.0, 0.0, 0.0),
                glam::Vec3::new(1.0, 0.0, 0.0),
                glam::Vec3::new(0.0, 1.0, 0.0),
            ]],
        };
        std::fs::write(&path, crate::step::write_step("part", &mesh)).unwrap();
        let path_str = path.to_string_lossy().replace('\\', "\\\\");
        let state = run_lua(&format!(
            r#"
            bearcad.new()
            bearcad.import_step("{path_str}")
        "#
        ));
        assert_eq!(state.doc.imported_meshes.len(), 1);
        assert_eq!(state.doc.bodies.len(), 1);
        assert_eq!(
            state.doc.bodies.values().nth(0).unwrap().source,
            crate::model::BodySource::Imported(crate::arena::Key::from_bits(0))
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn lua_extrude_creates_solid_in_hierarchy() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 80, height = 50 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20, name = "Boss" }
        "#,
        );
        assert_eq!(state.doc.extrusions.len(), 1);
        assert_eq!(state.doc.extrusions[xkey(0)].distance, 20.0);
        assert_eq!(
            find_element_by_name(&state.doc, "Boss"),
            Some(SceneElement::Extrusion(xkey(0)))
        );
        // The extrusion produces a body that depends on it.
        assert_eq!(state.doc.bodies.len(), 1);
        assert_eq!(
            state.doc.bodies.values().nth(0).unwrap().source,
            crate::model::BodySource::Extrusion(xkey(0))
        );
        // Both appear as elements; the body nests under its extrusion.
        let nodes = crate::hierarchy::build_element_list(&state.doc, state.sketch_session);
        assert!(nodes.contains(&crate::hierarchy::HierarchyNode::Extrusion(xkey(0))));
        assert!(nodes.contains(&crate::hierarchy::HierarchyNode::Body(bkey(0))));
        let mesh =
            crate::extrude::extrusion_mesh(&state.doc, &state.doc.extrusions[xkey(0)]).unwrap();
        assert_eq!(mesh.triangles.len(), 12);
    }

    /// #504: `symmetric = true` extrudes half the distance each side of the sketch plane.
    #[test]
    fn lua_extrude_symmetric_straddles_sketch_plane() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 40, height = 30 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20, symmetric = true }
        "#,
        );
        assert_eq!(state.doc.extrusions.len(), 1);
        assert!(state.doc.extrusions[xkey(0)].symmetric);
        let mesh =
            crate::extrude::extrusion_mesh(&state.doc, &state.doc.extrusions[xkey(0)]).unwrap();
        let (min, max) = mesh.bounds().unwrap();
        assert!(
            (min.z + 10.0).abs() < 0.5 && (max.z - 10.0).abs() < 0.5,
            "symmetric extrude should span z≈[-10,10], min={min:?} max={max:?}"
        );
    }

    /// #1243: `taper` + `taper_mode` on `bearcad.extrude` grow the end face / cut height.
    #[test]
    fn lua_extrude_taper_distance_and_angle() {
        // Distance taper +5 on a 10×10×10 box → 20×20 end face.
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10, taper = 5 }
        "#,
        );
        assert_eq!(state.doc.extrusions.len(), 1);
        let ext = &state.doc.extrusions[xkey(0)];
        assert!((ext.taper - 5.0).abs() < 1e-4);
        assert_eq!(ext.taper_mode, crate::model::ExtrudeTaperMode::Distance);
        let mesh = crate::extrude::extrusion_mesh(&state.doc, ext).unwrap();
        let (min, max) = mesh.bounds().unwrap();
        assert!(
            (max.x - min.x - 20.0).abs() < 0.5,
            "distance taper should make 20-wide end, got {}",
            max.x - min.x
        );

        // Angle taper −45° on 10×10×10 collapses at height 5.
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10, taper = -45, taper_mode = "angle" }
        "#,
        );
        let ext = &state.doc.extrusions[xkey(0)];
        assert_eq!(ext.taper_mode, crate::model::ExtrudeTaperMode::Angle);
        let mesh = crate::extrude::extrusion_mesh(&state.doc, ext).unwrap();
        let (min, max) = mesh.bounds().unwrap();
        assert!(
            (max.z - min.z - 5.0).abs() < 0.6,
            "−45° taper should cut height to 5, got {}",
            max.z - min.z
        );
    }

    /// #1352: an over-large angle taper clamps to 89° (not 89.999°) and warns; the
    /// solid stays a reasonable size instead of spanning kilometres.
    #[test]
    fn lua_extrude_taper_angle_180_clamps_to_89_and_warns() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 20, height = 20 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20, taper = 180, taper_mode = "angle" }
            local s = bearcad.body_stats(0)
            local span_x = s.bbox.max[1] - s.bbox.min[1]
            local span_y = s.bbox.max[2] - s.bbox.min[2]
            assert(span_x < 5000 and span_y < 5000,
                "bbox should stay under 5 m, got " .. span_x .. " x " .. span_y)
            local st = bearcad.status()
            assert(st:find("89") or st:find("limited") or st:find("[Tt]aper"),
                "status should warn about the clamp, got: " .. st)
        "#,
        );
        let ext = &state.doc.extrusions[xkey(0)];
        assert_eq!(ext.taper_mode, crate::model::ExtrudeTaperMode::Angle);
        assert!(
            (ext.taper - 89.0).abs() < 1e-3,
            "stored taper should be 89°, got {}",
            ext.taper
        );
        assert!(
            state.status.to_lowercase().contains("taper")
                || state.status.contains("89")
                || state.status.to_lowercase().contains("limited"),
            "status should carry the warning, got {:?}",
            state.status
        );
    }

    /// #1352: angle tapers ≤ −90° clamp to −90° with a warning.
    #[test]
    fn lua_extrude_taper_angle_below_minus_90_clamps() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 20, height = 20 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20, taper = -180, taper_mode = "angle" }
            local st = bearcad.status()
            assert(st:find("-90") or st:find("limited") or st:find("[Tt]aper"),
                "status should warn about the clamp, got: " .. st)
        "#,
        );
        let ext = &state.doc.extrusions[xkey(0)];
        assert!(
            (ext.taper - (-90.0)).abs() < 1e-3,
            "stored taper should be −90°, got {}",
            ext.taper
        );
    }

    /// #1352: 89° on a long extrude still makes a huge solid — further clamp + warn.
    #[test]
    fn lua_extrude_taper_long_89_deg_is_size_clamped() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 20, height = 20 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 1000, taper = 89, taper_mode = "angle" }
            local s = bearcad.body_stats(0)
            local span_x = s.bbox.max[1] - s.bbox.min[1]
            assert(span_x < 25000,
                "long 89° extrude should not flare past the size cap, span=" .. span_x)
            local st = bearcad.status()
            assert(st:find("limited") or st:find("[Tt]aper") or st:find("huge"),
                "status should warn about the size clamp, got: " .. st)
        "#,
        );
        let ext = &state.doc.extrusions[xkey(0)];
        assert!(
            ext.taper < 88.5,
            "89° at 1000 mm should drop below 89°, got {}",
            ext.taper
        );
        let offset = 1000.0 * ext.taper.to_radians().tan();
        assert!(
            offset <= crate::extrude::TAPER_MAX_OFFSET_MM + 1.0,
            "offset {offset} should be ≤ {}",
            crate::extrude::TAPER_MAX_OFFSET_MM
        );
    }

    #[test]
    fn lua_extrude_accepts_explicit_polygon_line_list() {
        // The triangle's corners must actually be joined (coincident constraints, #68) for
        // the line list to form a closed loop — since #112, extrude rejects a line set that
        // merely touches by coordinates (it would produce no geometry).
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0 }
            bearcad.line{ x = 10, y = 0, x1 = 5, y1 = 8 }
            bearcad.line{ x = 5, y = 8, x1 = 0, y1 = 0 }
            for _, pair in ipairs({ {0, 1}, {1, 2}, {2, 0} }) do
                bearcad.select{ kind = "line", index = pair[1], ["end"] = "end" }
                bearcad.select({ kind = "line", index = pair[2], ["end"] = "start" }, true)
                bearcad.add_geometric_constraint("coincident")
            end
            bearcad.extrude{ polygon = {0, 1, 2}, distance = 6 }
        "#,
        );
        assert_eq!(state.doc.extrusions.len(), 1);
        assert_eq!(
            state.doc.extrusions[xkey(0)].faces,
            vec![crate::model::ExtrudeFace::Polygon(vec![lkey(0), lkey(1), lkey(2)])]
        );
    }

    #[test]
    fn lua_extrude_with_body_merge_joins_the_existing_body() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 80, height = 50 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
            bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 0, profile = "polygon", profile_lines = {0, 1, 2, 3}, top = true }
            bearcad.rect{ x = 10, y = 10, width = 20, height = 10 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5, body = "merge" }
        "#,
        );
        assert_eq!(state.doc.extrusions.len(), 2);
        // #1106: host is shadowed; a new combined body is the fuse output.
        assert_eq!(state.doc.bodies.len(), 2, "merge shadows host and creates combined body");
        let shadow = state.doc.bodies.values().find(|b| b.shadow).expect("host shadow");
        assert_eq!(shadow.source.extrusion_indices(), [xkey(0)]);
        let live = state.doc.bodies.values().find(|b| !b.shadow).expect("combined live");
        assert_eq!(live.source.extrusion_indices(), [xkey(0), xkey(1)]);
    }

    /// #1170/#1171: `bearcad.shell{ thickness = "name=value" }` defines the parameter and
    /// stores the bare name — same as typing it into the Shell thickness field.
    #[test]
    fn lua_shell_thickness_defines_inline_parameter() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.cuboid{ width = 40, depth = 30, height = 20 }
            bearcad.shell{
                bodies = {0},
                thickness = "foo=2mm"
            }
            "#,
        );
        assert_eq!(state.doc.shell_ops.len(), 1);
        assert_eq!(
            state.doc.shell_ops.values().next().unwrap().thickness,
            "foo",
            "stored thickness should be the bare parameter name"
        );
        let foo = state
            .doc
            .parameters
            .values()
            .find(|p| p.name == "foo")
            .expect("foo parameter defined");
        assert_eq!(foo.expression, "2mm");
    }

    /// #1172: shelling a Shape-tool cuboid with open faces must actually open those faces.
    /// After commit, `body_index_for_face` prefers the live shell output over the shadowed
    /// input primitive — matching open faces against that live body used to drop every face
    /// and leave a closed (looks solid from outside) hollow.
    #[test]
    fn lua_shell_cuboid_open_top_actually_opens() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.cuboid{ width = 40, depth = 30, height = 20 }
            bearcad.shell{
                bodies = {0},
                faces = {{ kind = "primitive_face", primitive = 0, face = "top" }},
                thickness = "2"
            }
            "#,
        );
        assert_eq!(state.doc.shell_ops.len(), 1);
        let op = state.doc.shell_ops.values().next().unwrap();
        assert_eq!(op.open_faces.len(), 1, "open face must be recorded on the op");
        assert_eq!(op.outputs.len(), 1);
        let out = op.outputs[0];
        assert!(!state.doc.bodies[out].shadow);
        let shape = crate::extrude::occt_body_shape(&state.doc, out)
            .expect("shelled cuboid must build a kernel solid");
        let v = shape.volume().expect("volume");
        // Open top, 2 mm walls: outer 40×30×20 minus cavity 36×26×18 = 7152.
        // Closed shell would be outer − 36×26×16 = 9024 — the bug volume.
        let open_expected = 40.0 * 30.0 * 20.0 - 36.0 * 26.0 * 18.0;
        let closed_walls = 40.0 * 30.0 * 20.0 - 36.0 * 26.0 * 16.0;
        assert!(
            (v - open_expected).abs() < 2.0,
            "open-top shell volume {v}, expected ~{open_expected} (closed walls would be {closed_walls})"
        );
        assert!(
            (v - closed_walls).abs() > 100.0,
            "volume {v} must not be the closed-shell {closed_walls} — open faces were dropped"
        );
    }

    /// #1172: report repro — open top *and* a side on a cuboid must hollow with openings
    /// (closed walls look solid from outside; openings are what the user sees).
    #[test]
    fn lua_shell_cuboid_open_top_and_side_is_hollow_with_openings() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.cuboid{ width = 50, depth = 60, height = 90 }
            bearcad.shell{
                bodies = {0},
                faces = {
                    { kind = "primitive_face", primitive = 0, face = "top" },
                    { kind = "primitive_face", primitive = 0, face = "side", edge = 2 },
                },
                thickness = "5"
            }
            "#,
        );
        assert_eq!(state.doc.shell_ops.len(), 1);
        let op = state.doc.shell_ops.values().next().unwrap();
        assert_eq!(op.open_faces.len(), 2);
        let out = op.outputs[0];
        let shape = crate::extrude::occt_body_shape(&state.doc, out)
            .expect("shelled cuboid with two open faces");
        let v = shape.volume().expect("volume");
        let solid = 50.0 * 60.0 * 90.0;
        // Closed 5 mm walls: outer − 40×50×80 = 270000 − 160000 = 110000.
        let closed_walls = solid - 40.0 * 50.0 * 80.0;
        assert!(
            v > 1000.0 && v < solid,
            "volume {v} should be a hollow under solid {solid}"
        );
        assert!(
            v < closed_walls - 1000.0,
            "volume {v} must be well under closed-shell {closed_walls} (open faces applied)"
        );
        // Re-edit thickness still accepts the primitive open faces (#1172 validate path).
        let edited = run_lua(
            r#"
            bearcad.new()
            bearcad.cuboid{ width = 50, depth = 60, height = 90 }
            bearcad.shell{
                bodies = {0},
                faces = {{ kind = "primitive_face", primitive = 0, face = "top" }},
                thickness = "5"
            }
            bearcad.edit_shell{
                index = 0,
                bodies = {0},
                faces = {{ kind = "primitive_face", primitive = 0, face = "top" }},
                thickness = "4"
            }
            "#,
        );
        assert_eq!(edited.doc.shell_ops.values().next().unwrap().thickness, "4");
    }

    /// #1168: extruding off a face of a *shelled* body must merge into the hollow solid,
    /// not re-grow a solid cuboid from the shadow primitive (which fills the cavity and
    /// makes the shell look "gone").
    #[test]
    fn lua_extrude_merge_onto_shelled_cuboid_keeps_the_shell() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.cuboid{ width = 40, depth = 30, height = 20 }
            bearcad.shell{
                bodies = {0},
                faces = {{ kind = "primitive_face", primitive = 0, face = "top" }},
                thickness = "2"
            }
            bearcad.begin_sketch{ kind = "primitive_face", primitive = 0, face = "side", edge = 0 }
            bearcad.rect{ x = 5, y = 5, width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15, body = "merge" }
            "#,
        );
        assert_eq!(state.doc.shell_ops.len(), 1);
        assert_eq!(state.doc.extrusions.len(), 1);
        // Cuboid (shadowed by shell) + pure shelled (shadowed by fuse) + fused shelled+boss.
        assert_eq!(
            state.doc.bodies.len(),
            3,
            "primitive + shelled host + fuse-merge output"
        );
        let live: Vec<_> = state
            .doc
            .bodies
            .iter()
            .filter(|(_, b)| !b.shadow)
            .collect();
        assert_eq!(live.len(), 1, "exactly one live body, got {}", live.len());
        let (live_bi, live) = live[0];
        assert!(
            matches!(
                &live.source,
                crate::model::BodySource::Shelled { add, cut, .. }
                    if add.as_slice() == [xkey(0)] && cut.is_empty()
            ),
            "live body must be the shelled solid with the merged extrusion, got {:?}",
            live.source
        );
        // The bug was a Solid{base: cuboid, add: extrusion} that refilled the cavity.
        assert!(
            state
                .doc
                .bodies
                .values()
                .all(|b| b.source.primitive_base().is_none() || b.shadow),
            "no live Solid-with-primitive-base may steal the shell's place"
        );
        let shape = crate::extrude::occt_body_shape(&state.doc, live_bi)
            .expect("shelled+boss kernel solid");
        let v = shape.volume().expect("volume");
        // Solid cuboid + boss = 40*30*20 + 10*10*15 = 25500. Hollow walls + boss is well under.
        let solid_plus_boss = 40.0 * 30.0 * 20.0 + 10.0 * 10.0 * 15.0;
        assert!(
            v > 100.0 && v < solid_plus_boss - 1000.0,
            "volume {v} should stay hollow (well under solid+boss {solid_plus_boss})"
        );
        // Pure shell (no boss) is still less than solid cuboid.
        assert!(v < 40.0 * 30.0 * 20.0 + 10.0 * 10.0 * 15.0 * 0.5 + 500.0);
    }

    /// #1236: `bearcad.copy` + `bearcad.paste` create an independent body; `linked = true`
    /// creates a dependent copy without shadowing the source.
    #[test]
    fn lua_copy_paste_independent_and_linked() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.cuboid{ width = 20, depth = 20, height = 10 }
            bearcad.select{ kind = "body", index = 0 }
            bearcad.copy()
            bearcad.paste{ x = 50 }
            "#,
        );
        assert_eq!(state.doc.bodies.len(), 2, "independent paste adds a body");
        assert!(
            state
                .doc
                .bodies
                .values()
                .any(|b| matches!(b.source, crate::model::BodySource::Imported(_))),
            "independent paste is an imported mesh body"
        );
        assert!(!state.doc.bodies.values().next().unwrap().shadow);

        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.cuboid{ width = 20, depth = 20, height = 10 }
            bearcad.select{ kind = "body", index = 0 }
            bearcad.copy()
            bearcad.paste{ linked = true, z = 40 }
            "#,
        );
        assert_eq!(state.doc.bodies.len(), 2);
        assert!(!state.doc.bodies.values().next().unwrap().shadow, "source stays live");
        assert!(
            state.doc.move_ops.values().next().unwrap().keep_inputs,
            "Paste Linked keeps inputs"
        );
        assert!(
            state
                .doc
                .bodies
                .values()
                .any(|b| matches!(b.source, crate::model::BodySource::Moved { .. })),
            "linked paste is a Moved body"
        );
        assert_eq!(state.tool, crate::actions::Tool::Move);
        assert_eq!(
            state.move_translate_mode,
            crate::model::MoveTranslateMode::Free
        );
    }

    /// #1104/#1106: extruding from a Shape-tool cuboid face with `body = "merge"` shadows
    /// the pure cuboid body and produces a new combined Solid as the extrusion's output.
    #[test]
    fn lua_extrude_merge_into_shape_tool_cuboid() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.cuboid{ width = 40, depth = 30, height = 20, name = "Block" }
            bearcad.begin_sketch{ kind = "primitive_face", primitive = 0, face = "top" }
            bearcad.rect{ x = 5, y = 5, width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15, body = "merge" }
            "#,
        );
        assert_eq!(state.doc.primitives.len(), 1);
        assert_eq!(state.doc.extrusions.len(), 1);
        assert_eq!(
            state.doc.bodies.len(),
            2,
            "merge shadows the cuboid body and creates a combined output"
        );
        let pi = state.doc.primitives.keys().next().unwrap();
        let (shadow_bi, shadow) = state
            .doc
            .bodies
            .iter()
            .find(|(_, b)| b.shadow)
            .expect("host cuboid is shadowed");
        assert!(
            matches!(shadow.source, crate::model::BodySource::Primitive(p) if p == pi),
            "shadow is the pure cuboid body"
        );
        let (live_bi, live) = state
            .doc
            .bodies
            .iter()
            .find(|(_, b)| !b.shadow)
            .expect("combined solid is live");
        assert_ne!(shadow_bi, live_bi);
        assert_eq!(live.source.primitive_base(), Some(pi));
        assert_eq!(live.source.extrusion_indices(), [xkey(0)]);
        assert!(live.source.cut_extrusion_indices().is_empty());
        assert_eq!(
            crate::model::fuse_host_of(&state.doc, live_bi),
            Some(shadow_bi)
        );
    }

    /// #1358: fuse-merge onto a slider's moving body keeps the combined solid at the
    /// jointed location and attached to the host, rather than dropping the part back
    /// to its pre-joint pose as a disconnected lump.
    #[test]
    fn lua_extrude_merge_onto_a_slider_moving_body_stays_joined() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.cuboid{ width = 20, depth = 20, height = 20 }
            bearcad.cuboid{ at = {40, 0, 0}, width = 20, depth = 20, height = 20 }
            local function face_facing(body, n)
              for _, f in ipairs(bearcad.body_faces(body)) do
                if math.abs(f.normal[1] - n[1]) < 0.01
                   and math.abs(f.normal[2] - n[2]) < 0.01
                   and math.abs(f.normal[3] - n[3]) < 0.01 then
                  return f
                end
              end
            end
            bearcad.joint{
              a = 0, b = 1, kind = "slider",
              face = { moving = face_facing(1, {0, 0, -1}), fixed = face_facing(0, {0, 0, 1}) },
              position = 30,
            }
            local before = bearcad.body_stats(1).bbox
            bearcad.begin_sketch{ kind = "primitive_face", primitive = 1, face = "top" }
            bearcad.rect{ x = 5, y = 5, width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15, body = "merge" }
            local live = bearcad.count("body") - 1
            local after = bearcad.body_stats(live).bbox
            assert(after.min[3] > before.min[3] - 1,
              "combined body must not drop back to the pre-joint height")
            assert(after.max[3] > before.max[3] + 10,
              "boss must stay on the posed moving body")
            "#,
        );
        let live_bi = state
            .doc
            .bodies
            .iter()
            .find(|(_, b)| !b.shadow && b.source.producing_extrusion().is_some())
            .map(|(k, _)| k)
            .expect("combined live body");
        let host = crate::model::fuse_host_of(&state.doc, live_bi).expect("fuse host");
        assert!(
            crate::joints::body_joint_pose(&state.doc, live_bi).is_some(),
            "combined body inherits the slider pose"
        );
        assert_eq!(
            crate::joints::body_joint_pose(&state.doc, live_bi),
            crate::joints::body_joint_pose(&state.doc, host),
        );
        let unposed = crate::extrude::body_solid_mesh_unposed(&state.doc, live_bi)
            .expect("unposed combined mesh");
        let (umin, umax) = unposed.bounds().expect("unposed bounds");
        let span = umax - umin;
        assert!(
            span.x < 25.0 && span.y < 25.0 && span.z < 40.0,
            "fuse must attach the boss to the cuboid in modelling space, span={span:?}"
        );
        assert!(
            crate::extrude::mesh_signed_volume(&unposed).abs() > 20.0 * 20.0 * 20.0 + 10.0 * 10.0 * 10.0,
            "combined volume includes the boss"
        );
    }

    /// #1104: `body = "cut"` on a Shape-tool cuboid face still mutates that body into a
    /// Solid with the cut (cut does not use the merge shadow/new-body path).
    #[test]
    fn lua_extrude_cut_into_shape_tool_cuboid() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.cuboid{ width = 40, depth = 30, height = 20 }
            bearcad.begin_sketch{ kind = "primitive_face", primitive = 0, face = "top" }
            bearcad.rect{ x = 10, y = 5, width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = -10, body = "cut" }
            "#,
        );
        assert_eq!(state.doc.bodies.len(), 1);
        let body = state.doc.bodies.values().next().unwrap();
        assert_eq!(
            body.source.primitive_base(),
            Some(state.doc.primitives.keys().next().unwrap())
        );
        assert!(body.source.extrusion_indices().is_empty());
        assert_eq!(body.source.cut_extrusion_indices(), [xkey(0)]);
    }

    /// #1338: a tapered cut through a Combine result must actually subtract, and a second
    /// cut on the same combined body must still be allowed.
    #[test]
    fn lua_cut_extrude_into_a_combined_body_subtracts() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.cuboid{ width = 40, depth = 40, height = 20 }
            bearcad.cuboid{ at = {20, 0, 0}, width = 40, depth = 40, height = 20 }
            bearcad.combine{ op = "combine", a = {0, 1} }
            local live = bearcad.count("body") - 1
            local v0 = bearcad.body_stats(live).volume
            assert(v0 > 40000, "combined solid should be larger than one cuboid, got " .. v0)
            local faces = bearcad.body_faces(live)
            local top
            for i = 1, #faces do
                if faces[i].normal[3] > 0.9 then top = faces[i] break end
            end
            assert(top, "combined body has a +Z face")
            local function q(v)
                return {
                    math.floor(v[1] * 100 + 0.5),
                    math.floor(v[2] * 100 + 0.5),
                    math.floor(v[3] * 100 + 0.5),
                }
            end
            bearcad.begin_sketch{
                kind = "body_mesh_face",
                body = live,
                centroid = q(top.face),
                normal = q(top.normal),
            }
            bearcad.circle{ x = 0, y = 0, r = 6 }
            bearcad.extrude{ circle = 0, distance = -30, taper = 5, body = "cut" }
            local v1 = bearcad.body_stats(live).volume
            assert(v1 < v0 - 50, "first cut must remove material: " .. v1 .. " vs " .. v0)
            bearcad.begin_sketch{
                kind = "body_mesh_face",
                body = live,
                centroid = q(top.face),
                normal = q(top.normal),
            }
            bearcad.circle{ x = 12, y = 0, r = 4 }
            bearcad.extrude{ circle = 1, distance = -30, body = "cut" }
            local v2 = bearcad.body_stats(live).volume
            assert(v2 < v1 - 20, "second cut must also remove material: " .. v2 .. " vs " .. v1)
            "#,
        );
        let live = state
            .doc
            .bodies
            .iter()
            .find(|(_, b)| !b.shadow)
            .map(|(_, b)| b)
            .expect("live combined body");
        assert_eq!(
            live.source.cut_extrusion_indices().len(),
            2,
            "both cuts must stay on the combined body, not become orphan extrusions"
        );
    }

    /// Shared Lua: sketch a circle on the last body's most +Z face and cut-extrude it.
    fn cut_last_live_body_lua(label: &str) -> String {
        format!(
            r#"
            local live = bearcad.count("body") - 1
            local v0 = bearcad.body_stats(live).volume
            assert(v0 > 1000, "{label}: live body should have volume, got " .. v0)
            local faces = bearcad.body_faces(live)
            local top
            local best = -2
            for i = 1, #faces do
                local nz = faces[i].normal[3]
                if nz > best then best = nz; top = faces[i] end
            end
            assert(top, "{label}: live body has no faces")
            local function q(v)
                return {{
                    math.floor(v[1] * 100 + 0.5),
                    math.floor(v[2] * 100 + 0.5),
                    math.floor(v[3] * 100 + 0.5),
                }}
            end
            bearcad.begin_sketch{{
                kind = "body_mesh_face",
                body = live,
                centroid = q(top.face),
                normal = q(top.normal),
            }}
            bearcad.circle{{ x = 0, y = 0, r = 6 }}
            bearcad.extrude{{ circle = 0, distance = -30, body = "cut" }}
            local v1 = bearcad.body_stats(live).volume
            assert(v1 < v0 - 50, "{label}: cut must remove material: " .. v1 .. " vs " .. v0)
            "#
        )
    }

    /// #1345: a cut into a Move/Slice/Mirror/Repeat/fillet result must subtract, not
    /// create an orphan extrusion (same class as #1338).
    #[test]
    fn lua_cut_extrude_into_op_produced_bodies_subtracts() {
        let cases: &[(&str, &str, fn(&crate::model::BodySource) -> bool)] = &[
            (
                "moved",
                r#"
            bearcad.new()
            bearcad.cuboid{ width = 40, depth = 40, height = 20 }
            bearcad.move_bodies{ bodies = {0}, x = 30 }
            "#,
                |s| matches!(s, crate::model::BodySource::Moved { .. }),
            ),
            (
                "sliced",
                r#"
            bearcad.new()
            bearcad.cuboid{ width = 40, depth = 40, height = 20 }
            bearcad.plane{ offset = 10 }
            bearcad.slice{ bodies = {0}, cutters = {{ kind = "construction_plane", index = 3 }} }
            "#,
                |s| matches!(s, crate::model::BodySource::Sliced { .. }),
            ),
            (
                "mirrored",
                r#"
            bearcad.new()
            bearcad.cuboid{ width = 40, depth = 40, height = 20 }
            bearcad.mirror_bodies{
                plane = { kind = "construction_plane", index = 2 },
                bodies = {0},
            }
            "#,
                |s| matches!(s, crate::model::BodySource::Mirrored { .. }),
            ),
            (
                "repeated",
                r#"
            bearcad.new()
            bearcad.cuboid{ width = 40, depth = 40, height = 20 }
            bearcad.repeat_bodies{ bodies = {0}, axis = "x", count = 3, gap = 10 }
            "#,
                |s| matches!(s, crate::model::BodySource::Repeated { .. }),
            ),
            (
                "edge_treated",
                r#"
            bearcad.new()
            bearcad.cuboid{ width = 40, depth = 40, height = 20 }
            bearcad.fillet_edge{
                primitive = 0,
                edge = { kind = "cap", face = 0, edge = 0, top = true },
                radius = 3,
            }
            "#,
                |s| matches!(s, crate::model::BodySource::EdgeTreated { .. }),
            ),
        ];
        for (label, setup, is_kind) in cases {
            let source = format!("{setup}{}", cut_last_live_body_lua(label));
            let state = run_lua(&source);
            let live = state
                .doc
                .bodies
                .iter()
                .filter(|(_, b)| !b.shadow)
                .last()
                .map(|(_, b)| b)
                .unwrap_or_else(|| panic!("{label}: expected a live body"));
            assert!(
                is_kind(&live.source),
                "{label}: last live body should be the op output, got {:?}",
                live.source
            );
            assert_eq!(
                live.source.cut_extrusion_indices().len(),
                1,
                "{label}: cut must stay on the op-produced body, not become an orphan"
            );
        }
    }

    /// #1104/#1105/#1106: shape keeps its pure (shadow) body + face sketch; the combined
    /// body nests under the extrusion as its output.
    #[test]
    fn lua_shape_face_sketch_and_merged_body_appear_in_hierarchy() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.cuboid{ width = 40, depth = 30, height = 20 }
            bearcad.begin_sketch{ kind = "primitive_face", primitive = 0, face = "side", edge = 0 }
            bearcad.rect{ x = 2, y = 2, width = 8, height = 8 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 12, body = "merge" }
            "#,
        );
        let tree = crate::hierarchy::build_hierarchy(&state.doc, None);
        let root = &tree[0];
        let pi = state.doc.primitives.keys().next().unwrap();
        let shape_entry = root
            .children
            .iter()
            .find(|e| e.node == crate::hierarchy::HierarchyNode::Shape(pi))
            .expect("shape is a top-level element");
        let (shadow_bi, _) = state
            .doc
            .bodies
            .iter()
            .find(|(_, b)| b.shadow)
            .expect("shadow host");
        let (live_bi, _) = state
            .doc
            .bodies
            .iter()
            .find(|(_, b)| !b.shadow)
            .expect("live solid");
        assert!(
            shape_entry
                .children
                .iter()
                .any(|c| c.node == crate::hierarchy::HierarchyNode::Body(shadow_bi)),
            "pure cuboid body (shadow) nests under the shape, children: {:?}",
            shape_entry.children.iter().map(|c| &c.node).collect::<Vec<_>>()
        );
        let sketch = state.doc.sketches.keys().next().unwrap();
        assert!(
            shape_entry
                .children
                .iter()
                .any(|c| c.node == crate::hierarchy::HierarchyNode::Sketch(sketch)),
            "sketch on the shape face nests under the shape"
        );
        // Walk Sketch → Extrusion → combined body.
        let sketch_entry = shape_entry
            .children
            .iter()
            .find(|c| c.node == crate::hierarchy::HierarchyNode::Sketch(sketch))
            .unwrap();
        let ei = state.doc.extrusions.keys().next().unwrap();
        let extrude_entry = sketch_entry
            .children
            .iter()
            .find(|c| c.node == crate::hierarchy::HierarchyNode::Extrusion(ei))
            .expect("extrusion under sketch");
        assert!(
            extrude_entry
                .children
                .iter()
                .any(|c| c.node == crate::hierarchy::HierarchyNode::Body(live_bi)),
            "combined body is the extrusion's output, children: {:?}",
            extrude_entry.children.iter().map(|c| &c.node).collect::<Vec<_>>()
        );
        let deps = crate::hierarchy::graph_dependency_edges(&state.doc);
        assert!(
            deps.contains(&(
                crate::hierarchy::HierarchyNode::Body(shadow_bi),
                crate::hierarchy::HierarchyNode::Extrusion(ei)
            )),
            "shadow host feeds the extrusion in the graph"
        );
    }

    /// #1107: repeated fuse-merge extrudes each shadow the prior body and produce a new
    /// combined body under that extrusion (not a single mutating solid).
    #[test]
    fn lua_repeated_merge_extrudes_shadow_chain() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.cuboid{ width = 40, depth = 30, height = 20 }
            bearcad.begin_sketch{ kind = "primitive_face", primitive = 0, face = "side", edge = 0 }
            bearcad.rect{ x = 2, y = 2, width = 8, height = 8 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10, body = "merge" }
            bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 0, profile = "polygon", profile_lines = {0, 1, 2, 3}, top = true }
            bearcad.rect{ x = 1, y = 1, width = 6, height = 6 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 10, body = "merge" }
            bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 1, profile = "polygon", profile_lines = {4, 5, 6, 7}, top = true }
            bearcad.rect{ x = 1, y = 1, width = 4, height = 4 }
            bearcad.extrude{ polygon = {8, 9, 10, 11}, distance = 10, body = "merge" }
            "#,
        );
        assert_eq!(state.doc.extrusions.len(), 3);
        // Pure cuboid + three fused solids.
        assert_eq!(state.doc.bodies.len(), 4);
        let shadows: Vec<_> = state.doc.bodies.iter().filter(|(_, b)| b.shadow).collect();
        let lives: Vec<_> = state.doc.bodies.iter().filter(|(_, b)| !b.shadow).collect();
        assert_eq!(shadows.len(), 3, "each prior body is shadowed");
        assert_eq!(lives.len(), 1, "one live combined body");
        let live_bi = lives[0].0;
        assert_eq!(
            lives[0].1.source.extrusion_indices(),
            [xkey(0), xkey(1), xkey(2)]
        );
        // Each extrusion's output body nests under it.
        let tree = crate::hierarchy::build_hierarchy(&state.doc, None);
        for ei in state.doc.extrusions.keys() {
            let entry = crate::hierarchy::find_hierarchy_entry(
                &tree,
                crate::hierarchy::HierarchyNode::Extrusion(ei),
            )
            .unwrap_or_else(|| panic!("extrusion {ei:?} in tree"));
            let produced = state
                .doc
                .bodies
                .iter()
                .find(|(_, b)| b.source.producing_extrusion() == Some(ei))
                .map(|(k, _)| k)
                .expect("each extrusion produces a body");
            assert!(
                entry
                    .children
                    .iter()
                    .any(|c| c.node == crate::hierarchy::HierarchyNode::Body(produced)),
                "extrusion {ei:?} outputs body {produced:?}"
            );
        }
        // Final live body is produced by the last extrusion.
        assert_eq!(
            state.doc.bodies[live_bi].source.producing_extrusion(),
            Some(xkey(2))
        );
    }

    #[test]
    fn lua_extrude_with_body_cut_subtracts_from_the_existing_body() {
        // `body = "cut"` (#35) records the new extrusion as a subtraction of the extruded
        // face's body rather than fusing a new body (combine/merge is the shadow path).
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 80, height = 50 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
            bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 0, profile = "polygon", profile_lines = {0, 1, 2, 3}, top = true }
            bearcad.rect{ x = 10, y = 10, width = 20, height = 10 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5, body = "cut" }
        "#,
        );
        assert_eq!(state.doc.extrusions.len(), 2);
        assert_eq!(state.doc.bodies.len(), 1, "the cut should not create a new body");
        assert_eq!(state.doc.bodies.values().nth(0).unwrap().source.extrusion_indices(), [xkey(0)]);
        assert_eq!(state.doc.bodies.values().nth(0).unwrap().source.cut_extrusion_indices(), [xkey(1)]);
    }

    /// #178 part 1: `body = "cut"` (or `"merge"`) explicitly requested, but the sketch isn't
    /// on a body face, must error rather than silently degrading to a standalone new body
    /// (which produces no holes and raises nothing). Nothing is created.
    #[test]
    fn lua_extrude_cut_without_a_candidate_body_errors() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.circle{ x = 0, y = 0, r = 5 }
            local ok, err = pcall(bearcad.extrude, { circle = 0, distance = -3, body = "cut" })
            assert(not ok, "cut with no body to cut should error")
            assert(tostring(err):find("cut"), "unexpected error: " .. tostring(err))
            assert(bearcad.count("extrusion") == 0, "no extrusion should be created")
            assert(bearcad.count("body") == 0, "no body should be created")
        "#,
        );
        assert_eq!(state.doc.extrusions.len(), 0);
        assert_eq!(state.doc.bodies.len(), 0);
    }

    /// #178 part 2: a cut sketched on a *flat side wall* of a curved-profile (fillet-bridge)
    /// extrusion resolves the host body and subtracts from it — the side-face `edge` index is
    /// analytic (per profile line), so every flat wall is reachable regardless of how the
    /// curved bridge is faceted.
    #[test]
    fn lua_extrude_cut_on_a_curved_profile_side_wall_subtracts_from_the_host() {
        // Rect 0..3, fillet a corner (#538): edges 0,1 are shadowed, their trimmed copies land at
        // lines 4,5 and the curved bridge at line 6; the visible loop is [4,6,5,2,3].
        // edge 2 addresses line 5 (a straight wall), not a curve facet.
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ x = 0, y = 0, width = 30, height = 30 }
            bearcad.fillet_vertex{ point = { kind = "line", index = 0, ["end"] = "end" }, radius = 5 }
            bearcad.extrude{ polygon = {4, 6, 5, 2, 3}, distance = 10 }
            bearcad.begin_sketch{ kind = "extrude_side", extrusion = 0,
                profile = "polygon", profile_lines = {4, 6, 5, 2, 3}, edge = 2 }
            bearcad.circle{ x = 5, y = 5, r = 2 }
            bearcad.exit_sketch()
            bearcad.extrude{ circle = 0, distance = -3, body = "cut" }
        "#,
        );
        assert_eq!(state.doc.bodies.len(), 1, "the cut must not create a new body");
        assert_eq!(state.doc.bodies.values().nth(0).unwrap().source.extrusion_indices(), [xkey(0)]);
        assert_eq!(state.doc.bodies.values().nth(0).unwrap().source.cut_extrusion_indices(), [xkey(1)]);
    }

    /// #178 part 2: `side_quad_world`'s `edge` indexes the profile's lines analytically. The
    /// curved fillet bridge (a non-flat wall) resolves to `None`; each straight line resolves
    /// to a flat quad whose base edge is that line's actual world endpoints — not a curve
    /// facet. This is what makes every flat side wall addressable by a stable, script-visible
    /// index.
    #[test]
    fn side_quad_world_addresses_profile_lines_analytically() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ x = 0, y = 0, width = 30, height = 30 }
            bearcad.fillet_vertex{ point = { kind = "line", index = 0, ["end"] = "end" }, radius = 5 }
            bearcad.extrude{ polygon = {4, 6, 5, 2, 3}, distance = 10 }
        "#,
        );
        // #538: the shadowed sources' trimmed copies are lines 4,5; the curved bridge is line 6.
        let loop_lines = vec![lkey(4), lkey(6), lkey(5), lkey(2), lkey(3)];
        let profile = crate::model::ExtrudeFace::Polygon(loop_lines.clone());
        assert_eq!(crate::extrude::side_face_count(&profile), loop_lines.len());
        let frame = crate::face::sketch_geometry_frame(&state.doc, skey(0)).unwrap();
        for (edge, &li) in loop_lines.iter().enumerate() {
            let line = &state.doc.lines[li];
            let quad = crate::extrude::side_quad_world(&state.doc, xkey(0), &profile, edge);
            if line.is_curved() {
                assert!(
                    quad.is_none(),
                    "curved bridge (line {}) is not a flat wall",
                    li.index()
                );
                continue;
            }
            let quad =
                quad.unwrap_or_else(|| panic!("straight line {} has a flat wall", li.index()));
            // The wall's base edge is line `li`'s two world endpoints (in some order).
            let ws = crate::face::local_to_world(&frame, line.x0, line.y0);
            let we = crate::face::local_to_world(&frame, line.x1, line.y1);
            let base = [quad[0], quad[1]];
            let matches = (base[0].distance(ws) < 1e-3 && base[1].distance(we) < 1e-3)
                || (base[0].distance(we) < 1e-3 && base[1].distance(ws) < 1e-3);
            assert!(
                matches,
                "edge {edge} wall base {base:?} != line {} endpoints",
                li.index()
            );
        }
    }

    #[test]
    fn lua_extrude_without_body_merge_creates_a_new_body() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 80, height = 50 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
            bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 0, profile = "polygon", profile_lines = {0, 1, 2, 3}, top = true }
            bearcad.rect{ x = 10, y = 10, width = 20, height = 10 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
        "#,
        );
        assert_eq!(state.doc.extrusions.len(), 2);
        assert_eq!(state.doc.bodies.len(), 2, "default extrude always starts a new body");
    }

    #[test]
    fn deleting_extrusion_removes_its_body() {
        let mut state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 80, height = 50 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
        "#,
        );
        assert_eq!(state.doc.bodies.len(), 1);
        crate::document_lifecycle::delete_element(
            &mut state.doc,
            SceneElement::Extrusion(xkey(0)),
        );
        assert!(!state.doc.extrusions.contains(xkey(0)));
        assert!(!state.doc.bodies.contains(bkey(0)), "body should be removed with its extrusion");
    }

    #[test]
    fn lua_new_and_tool() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.begin_sketch("construction_plane", 0)
            bearcad.ui.tool("rectangle")
        "#,
        );
        assert_eq!(state.tool, Tool::Rectangle);
        assert!(state.sketch_session.is_some());
    }

    #[test]
    fn lua_find_and_set_name() {
        let mut runner = ScriptRunner::from_lua_source(
            r#"
            bearcad.set_name({ kind = "line", index = 0 }, "Main box")
            local found = bearcad.find("Main box")
            assert(found ~= nil)
        "#,
        )
        .unwrap();
        runner.verbose = false;
        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        state.doc.lines.insert(crate::model::Line::from_local_endpoints(
            sketch, 0.0, 0.0, 10.0, 0.0,
        ));
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, None, &ctx);
        }
        assert_eq!(
            find_element_by_name(&state.doc, "Main box"),
            Some(SceneElement::Line(lkey(0)))
        );
    }

    #[test]
    fn lua_set_units_sets_document_defaults() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.set_units{ length = "in", angle = "rad" }
        "#,
        );
        assert_eq!(state.doc.default_length_unit, LengthUnit::In);
        assert_eq!(state.doc.default_angle_unit, AngleUnit::Rad);
    }

    /// #1394: a bare number in an expression is interpreted in the document's default
    /// length unit (here inches), not millimetres.
    #[test]
    fn lua_parameter_bare_number_uses_document_default_unit() {
        run_lua_expect_ok(
            r#"
            bearcad.new()
            bearcad.set_units{ length = "in" }
            bearcad.parameter("add", "A", "1.5")
            local v = bearcad.parameter("get", "A")
            local want = 1.5 * 25.4
            assert(math.abs(v - want) < 1e-4, "bare 1.5 in inches doc should be " .. want .. " mm, got " .. tostring(v))
            -- Explicit mm still works as expected.
            bearcad.parameter("add", "B", "10mm")
            local b = bearcad.parameter("get", "B")
            assert(math.abs(b - 10) < 1e-4, "10mm should be 10 mm, got " .. tostring(b))
        "#);
    }

    #[test]
    fn lua_set_units_partial_document_call_keeps_other_axis() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.set_units{ length = "cm" }
        "#,
        );
        assert_eq!(state.doc.default_length_unit, LengthUnit::Cm);
        assert_eq!(state.doc.default_angle_unit, AngleUnit::Deg);
    }

    #[test]
    fn lua_set_units_sets_and_clears_sketch_override() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.begin_sketch("construction_plane", 0)
            bearcad.set_units{ sketch = 0, length = "ft" }
        "#,
        );
        assert_eq!(state.doc.sketches[skey(0)].length_unit, Some(LengthUnit::Ft));
        assert_eq!(state.doc.sketches[skey(0)].angle_unit, None);

        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.begin_sketch("construction_plane", 0)
            bearcad.set_units{ sketch = 0, length = "ft" }
            bearcad.set_units{ sketch = 0 }
        "#,
        );
        assert_eq!(
            state.doc.sketches[skey(0)].length_unit, None,
            "omitting length on a sketch call clears the override back to inherit"
        );
    }

    #[test]
    fn lua_set_units_rejects_unknown_unit_name() {
        let mut runner = ScriptRunner::from_lua_source(
            r#"
            bearcad.set_units{ length = "furlongs" }
        "#,
        )
        .unwrap();
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, None, &ctx);
        }
        assert!(runner.error.is_some(), "unknown unit name should error");
    }

    #[test]
    fn lua_sketch_dof_reports_remaining_degrees_of_freedom() {
        let mut runner = ScriptRunner::from_lua_source(
            r#"
            bearcad.begin_sketch("construction_plane", 0)
            bearcad.ui.tool("line")
            bearcad.ui.click(0, 0)
            bearcad.ui.click(100, 0)
            bearcad.commit()
            assert(bearcad.sketch_dof() > 0)
        "#,
        )
        .unwrap();
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, None, &ctx);
        }
    }

    #[test]
    fn lua_import_exposes_globals() {
        let mut runner = ScriptRunner::from_lua_source(
            r#"
            bearcad.import()
            new()
            tool("select")
        "#,
        )
        .unwrap();
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, None, &ctx);
        }
        assert_eq!(state.tool, Tool::Select);
    }

    /// #107: `bearcad.count(kind)` counts only non-deleted entities of that kind.
    #[test]
    fn lua_count_reports_non_deleted_entities() {
        run_lua_expect_ok(
            r#"
            bearcad.new()
            bearcad.rect{ x = 0, y = 0, width = 40, height = 30 }
            bearcad.circle{ x = 100, y = 0, r = 5 }
            assert(bearcad.count("line") == 4, "line count " .. bearcad.count("line"))
            assert(bearcad.count("circle") == 1)
            assert(bearcad.count("sketch") == 1)
            -- The three datum planes a new document opens with (#833).
            assert(bearcad.count("construction_plane") == 3)
            assert(bearcad.count("extrusion") == 0)
            assert(bearcad.count("body") == 0)
            assert(bearcad.count("parameter") == 0)
        "#,
        );
    }

    #[test]
    fn lua_count_rejects_unknown_kind_naming_valid_kinds() {
        run_lua_expect_ok(
            r#"
            local ok, err = pcall(bearcad.count, "widget")
            assert(not ok, "unknown kind should error")
            err = tostring(err)
            assert(err:find("construction_plane") and err:find("parameter"),
                   "error should name the valid kinds: " .. err)
        "#,
        );
    }

    /// #107: `bearcad.get{ kind, index }` returns a table of the entity's fields, or nil
    /// when the index is out of range (or the entity is deleted).
    #[test]
    fn lua_get_returns_entity_fields_and_nil_out_of_range() {
        run_lua_expect_ok(
            r#"
            bearcad.new()
            bearcad.line{ x = 1, y = 2, x1 = 11, y1 = 2, name = "Edge" }
            bearcad.circle{ x = 10, y = 5, r = 12 }
            local l = bearcad.get{ kind = "line", index = 0 }
            assert(math.abs(l.x0 - 1) < 1e-4 and math.abs(l.y0 - 2) < 1e-4)
            assert(math.abs(l.x1 - 11) < 1e-4 and math.abs(l.y1 - 2) < 1e-4)
            assert(l.curved == false and l.construction == false)
            assert(l.bezier == nil)
            assert(math.abs(l.length - 10) < 1e-3)
            assert(l.name == "Edge")
            assert(l.sketch == 0)
            local c = bearcad.get{ kind = "circle", index = 0 }
            assert(math.abs(c.x - 10) < 1e-4 and math.abs(c.y - 5) < 1e-4)
            assert(math.abs(c.r - 12) < 1e-4 and math.abs(c.diameter - 24) < 1e-4)
            assert(c.construction == false and c.name == nil)
            local s = bearcad.get{ kind = "sketch", index = 0 }
            assert(s.face == "construction_plane")
            local p = bearcad.get{ kind = "construction_plane", index = 0 }
            assert(p.origin[3] == 0 and p.normal[3] == 1)
            assert(bearcad.get{ kind = "line", index = 99 } == nil)
            assert(bearcad.get{ kind = "body", index = 0 } == nil)
        "#,
        );
    }

    /// #107: `bearcad.body_stats(index)` reports volume (divergence-theorem), triangle count,
    /// and world bbox for a body's solid mesh; nil for missing bodies.
    #[test]
    fn lua_body_stats_reports_volume_triangles_and_bbox() {
        run_lua_expect_ok(
            r#"
            bearcad.new()
            bearcad.rect{ x = 0, y = 0, width = 40, height = 30 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
            local s = bearcad.body_stats(0)
            assert(s ~= nil, "body_stats should return a table for body 0")
            assert(math.abs(s.volume - 12000) < 120, "volume " .. tostring(s.volume))
            assert(s.triangles > 0)
            assert(math.abs((s.bbox.max[1] - s.bbox.min[1]) - 40) < 0.1)
            assert(math.abs((s.bbox.max[2] - s.bbox.min[2]) - 30) < 0.1)
            assert(math.abs((s.bbox.max[3] - s.bbox.min[3]) - 10) < 0.1)
            assert(bearcad.body_stats(5) == nil)
        "#,
        );
    }

    /// #1319: `bearcad.ui.toolbar_shortcuts()` reports help-mode toolbar badges.
    #[test]
    fn lua_toolbar_shortcuts_follow_help_mode() {
        run_lua_expect_ok(
            r#"
            local empty = bearcad.ui.toolbar_shortcuts()
            assert(next(empty) == nil, "no badges while help mode is off")
            bearcad.ui.help(true)
            local s = bearcad.ui.toolbar_shortcuts()
            assert(s.shape == "B", "Shape should show B, got " .. tostring(s.shape))
            assert(s.sketch == "S")
            assert(s.select == nil, "Select has no shortcut")
            assert(s.project == nil, "Project is sketch-only")
            bearcad.begin_sketch("construction_plane", 0)
            s = bearcad.ui.toolbar_shortcuts()
            assert(s.project == "P", "Project should show P in a sketch")
            bearcad.ui.help(false)
            assert(next(bearcad.ui.toolbar_shortcuts()) == nil)
        "#,
        );
    }

    /// #107: `bearcad.status()` exposes the status-bar text.
    #[test]
    fn lua_status_returns_a_string() {
        run_lua_expect_ok(
            r#"
            bearcad.new()
            assert(type(bearcad.status()) == "string")
        "#,
        );
    }

    /// #107: `bearcad.selection()` lists the current scene selection as {kind, index} entries.
    #[test]
    fn lua_selection_reports_selected_elements() {
        run_lua_expect_ok(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0 }
            assert(#bearcad.selection() == 0)
            bearcad.select{ kind = "line", index = 0 }
            local sel = bearcad.selection()
            assert(#sel == 1)
            assert(sel[1].kind == "line")
            assert(sel[1].index == 0)
        "#,
        );
    }

    /// #402: sizes accept parameter-expression strings anywhere the GUI does — rect
    /// width/height, circle r/radius/diameter, and extrude distance — and store the
    /// expression so the model rebuilds when the parameter changes.
    #[test]
    fn lua_sizes_accept_parameter_expressions() {
        let state = run_lua(
            r#"
            bearcad.parameter("add", "w", "24")
            bearcad.rect{ width = "w", height = "w / 3" }
            bearcad.circle{ x = 40, y = 0, radius = "w / 4" }
            bearcad.circle{ x = 60, y = 0, diameter = "w" }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = "w / 2" }
            "#,
        );
        // Geometry evaluated against the parameter.
        let l = &state.doc.lines[lkey(0)];
        let width = ((l.x1 - l.x0).powi(2) + (l.y1 - l.y0).powi(2)).sqrt();
        assert!((width - 24.0).abs() < 1e-3, "rect width, got {width}");
        assert!((state.doc.circles[rkey(0)].r - 6.0).abs() < 1e-3, "radius expr");
        assert!((state.doc.circles[rkey(1)].r - 12.0).abs() < 1e-3, "diameter expr");
        assert!((state.doc.extrusions[xkey(0)].distance - 12.0).abs() < 1e-3);
        // Expressions stored, not baked numbers: the dims reference the parameter…
        assert_eq!(state.doc.extrusions[xkey(0)].expression, "w / 2");
        let exprs: Vec<&str> = state
            .doc
            .constraints
            .values()
            .map(|c| c.expression.as_str())
            .collect();
        assert!(exprs.contains(&"w"), "rect width constraint: {exprs:?}");
        assert!(exprs.contains(&"w / 3"), "rect height constraint: {exprs:?}");
        assert!(exprs.contains(&"(w / 4) * 2"), "radius constraint: {exprs:?}");

        // …so editing the parameter rebuilds the scripted model like a hand-built one.
        let state = run_lua(
            r#"
            bearcad.parameter("add", "w", "24")
            bearcad.rect{ width = "w", height = "w / 3" }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = "w / 2" }
            bearcad.parameter("value", 0, "30")
            "#,
        );
        let l = &state.doc.lines[lkey(0)];
        let width = ((l.x1 - l.x0).powi(2) + (l.y1 - l.y0).powi(2)).sqrt();
        assert!((width - 30.0).abs() < 1e-3, "rect follows the parameter, got {width}");
        assert!(
            (state.doc.extrusions[xkey(0)].distance - 15.0).abs() < 1e-3,
            "extrusion depth follows the parameter, got {}",
            state.doc.extrusions[xkey(0)].distance
        );
    }

    /// #403: unknown table keys are an error naming the accepted keys, `gap` works as
    /// the Repeat pane's alias for `spacing`, `count("image")` is a valid kind, and
    /// `drawing_view{ sketch = i }` projects a sketch.
    #[test]
    fn lua_api_polish_key_checks_aliases_and_sketch_views() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 20, height = 20 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }

            -- A typo'd key errors immediately, naming the accepted keys.
            local ok, err = pcall(function()
                bearcad.combine{ kind = "cut", a = {0}, b = {0} }
            end)
            assert(not ok, "combine{kind=} should error")
            assert(tostring(err):find("unknown key `kind`"), tostring(err))
            assert(tostring(err):find("op"), "error should list accepted keys: " .. tostring(err))
            local ok2, err2 = pcall(function()
                bearcad.rect{ width = 10, height = 10, witdh = 3 }
            end)
            assert(not ok2 and tostring(err2):find("witdh"), tostring(err2))

            -- `gap` = the Repeat pane's name for `spacing`.
            bearcad.repeat_bodies{ bodies = {0}, axis = "x", count = 3, gap = 5 }

            -- Images count (zero here, but the kind is valid).
            assert(bearcad.count("image") == 0)

            -- A drawing view of a sketch, not a body.
            local d = bearcad.drawing{}
            bearcad.drawing_view{ drawing = d, sketch = 0, orientation = "top" }
            local ok3 = pcall(function()
                bearcad.drawing_view{ drawing = d }
            end)
            assert(not ok3, "drawing_view without a source should error")
            "#,
        );
        assert_eq!(state.doc.repeat_ops.len(), 1);
        assert_eq!(state.doc.drawings[dkey(0)].views.len(), 1);
    }

    /// #648/#649/#650: naming both points makes a move a **snap** — the picked source corner
    /// lands exactly on the picked target corner, and x/y/z are ignored.
    #[test]
    fn lua_move_snaps_a_start_point_a_onto_a_end_point_a() {
        let state = run_lua(
            r#"
            -- Two 10x10x5 boxes: A at the origin, B 40mm along +X.
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.rect{ x = 40, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
            -- Snap A's origin corner onto B's near-bottom corner.
            bearcad.move_bodies{
                bodies = {0},
                from = { body = 0, vertex = {0, 0, 0} },
                to   = { body = 1, vertex = {40, 0, 0} },
                x = 999,  -- ignored: the points win
            }
            "#,
        );
        let op = &state.doc.move_ops.values().nth(0).unwrap();
        assert_eq!(op.translate_mode, crate::model::MoveTranslateMode::PointSnap);
        assert!(op.has_snap_translation());
        let t = crate::extrude::move_op_translation(&state.doc, op).expect("translation");
        assert!(
            (t - glam::Vec3::new(40.0, 0.0, 0.0)).length() < 1e-3,
            "snap offset should be +40 X, got {t:?}"
        );
        // The moved output really sits over body B.
        let out = op.outputs[0];
        let (min, _) = crate::extrude::body_solid_mesh(&state.doc, out)
            .and_then(|m| m.bounds())
            .expect("moved mesh");
        assert!((min.x - 40.0).abs() < 1e-2, "moved body starts at x=40, got {min:?}");
        // A plain x/y/z move stays free.
        let free = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.move_bodies{ bodies = {0}, x = 7 }
            "#,
        );
        assert_eq!(
            free.doc.move_ops.values().nth(0).unwrap().translate_mode,
            crate::model::MoveTranslateMode::Free
        );
    }

    /// #669: `from_b`/`to_b` add the rotation — the bodies turn about end point A so start
    /// point B lands on end point B.
    #[test]
    fn lua_move_b_pair_adds_the_rotation() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            -- No translation (A start = A end), then a quarter turn: the corner at
            -- (10, 0, 0) swings onto (0, 10, 0), both 10 from the pivot.
            bearcad.move_bodies{
                bodies = {0},
                from   = { body = 0, vertex = {0, 0, 0} },
                to     = { body = 0, vertex = {0, 0, 0} },
                from_b = { body = 0, vertex = {10, 0, 0} },
                to_b   = { body = 0, vertex = {0, 10, 0} },
            }
            "#,
        );
        let op = &state.doc.move_ops.values().nth(0).unwrap();
        assert!(op.has_snap_rotation(), "both B points make it rotate");
        let m = crate::extrude::move_op_transform(&state.doc, op).expect("transform");
        let landed = m.transform_point3(glam::Vec3::new(10.0, 0.0, 0.0));
        assert!(
            (landed - glam::Vec3::new(0.0, 10.0, 0.0)).length() < 1e-2,
            "start B lands on end B, got {landed:?}"
        );
        // Naming only the A pair leaves it a pure translation.
        let translate_only = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.move_bodies{ bodies = {0},
                from = { body = 0, vertex = {0, 0, 0} },
                to   = { body = 0, vertex = {0, 0, 0} } }
            "#,
        );
        assert!(!translate_only.doc.move_ops.values().nth(0).unwrap().has_snap_rotation());
    }

    /// `from_c`/`to_c` pin the spin about `end A → end B` that the B pair leaves free.
    #[test]
    fn lua_move_c_pair_pins_the_remaining_spin() {
        let source = |c: &str| {
            format!(
                r#"
            bearcad.rect{{ width = 10, height = 10 }}
            bearcad.extrude{{ polygon = {{0, 1, 2, 3}}, distance = 10 }}
            -- A holds the origin and B holds (10, 0, 0), so the box is still free to spin
            -- about the X axis; C is what decides that turn.
            bearcad.move_bodies{{
                bodies = {{0}},
                from   = {{ body = 0, vertex = {{0, 0, 0}} }},
                to     = {{ body = 0, vertex = {{0, 0, 0}} }},
                from_b = {{ body = 0, vertex = {{10, 0, 0}} }},
                to_b   = {{ body = 0, vertex = {{10, 0, 0}} }},
                {c}
            }}
            "#
            )
        };
        // Without C the spin is undecided, so nothing turns.
        let free = run_lua(&source(""));
        assert!(!free.doc.move_ops.values().nth(0).unwrap().has_snap_roll());
        let m = crate::extrude::move_op_transform(&free.doc, &free.doc.move_ops.values().nth(0).unwrap()).unwrap();
        let corner = glam::Vec3::new(0.0, 0.0, 10.0);
        assert!((m.transform_point3(corner) - corner).length() < 1e-2, "no C, no spin");

        // With C, the top corner swings a quarter turn onto +10 Y.
        let state = run_lua(&source(
            "from_c = { body = 0, vertex = {0, 0, 10} },
             to_c   = { body = 0, vertex = {0, 10, 0} },",
        ));
        let op = &state.doc.move_ops.values().nth(0).unwrap();
        assert!(op.has_snap_roll(), "both C points pin the spin");
        let m = crate::extrude::move_op_transform(&state.doc, op).expect("transform");
        let landed = m.transform_point3(corner);
        assert!(
            (landed - glam::Vec3::new(0.0, 10.0, 0.0)).length() < 1e-2,
            "start C lands on end C, got {landed:?}"
        );
    }

    /// `bearcad.begin_move` arms the tool with its picks instead of committing them, so a
    /// script can drive the live preview — the ghost and the pair marks — the way the
    /// documentation shots do.
    #[test]
    fn lua_begin_move_arms_the_tool_without_committing() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
            bearcad.rect{ x = 40, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 10 }
            bearcad.begin_move{
                bodies = {1},
                from   = { body = 1, vertex = {40, 0, 0} },
                to     = { body = 0, vertex = {0, 0, 10} },
                from_b = { body = 1, vertex = {50, 0, 0} },
                to_b   = { body = 0, on_edge = {10, 0, 10} },
                from_c = { body = 1, vertex = {40, 0, 10} },
                to_c   = { body = 0, on_edge = {0, 10, 10} },
            }
            "#,
        );
        assert_eq!(state.tool, crate::actions::Tool::Move, "the Move tool comes up armed");
        assert!(state.doc.move_ops.is_empty(), "nothing is committed");
        let cm = state.creating_move.as_ref().expect("a move in progress");
        assert_eq!(cm.targets, vec![bkey(1)]);
        assert_eq!(cm.translate_mode, crate::model::MoveTranslateMode::PointSnap);
        for (what, point) in [
            ("start A", cm.start_point_a),
            ("end A", cm.end_point_a),
            ("start B", cm.start_point_b),
            ("end B", cm.end_point_b),
            ("start C", cm.start_point_c),
            ("end C", cm.end_point_c),
        ] {
            assert!(point.is_some(), "{what} should be armed");
        }
    }

    /// #891/#894: `bearcad.joint` commits a joint whose revolute position turns the
    /// driven body about the mated axis, and the joint lands in the document.
    #[test]
    fn lua_joint_commits_a_revolute_that_poses_the_driven_body() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.rect{ x = 40, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
            -- Faces are named by their own middle and normal, which `body_faces` reports.
            local function face_at(body, nx, ny, nz)
              for _, f in ipairs(bearcad.body_faces(body)) do
                if math.abs(f.normal[1] - nx) < 0.01
                   and math.abs(f.normal[2] - ny) < 0.01
                   and math.abs(f.normal[3] - nz) < 0.01 then
                  return f
                end
              end
            end
            bearcad.joint{
                a = 0, b = 1, kind = "revolute",
                face = { moving = face_at(1, 0, 0, -1), fixed = face_at(0, 0, 0, 1) },
                position = 90,
                name = "Hinge",
            }
            "#,
        );
        assert_eq!(state.doc.joints.len(), 1);
        let joint = &state.doc.joints.values().nth(0).unwrap();
        assert_eq!(joint.members.len(), 2);
        assert_eq!(joint.name.as_deref(), Some("Hinge"));
        assert_eq!(joint.rest, "90", "rest pose captured from creation (#898)");
        let pose = crate::joints::body_joint_pose(&state.doc, bkey(1)).expect("driven body posed");
        // Face Snap lands B's underside middle **on** A's top middle (5, 5, 5) (#1079); the
        // 90° turn about that normal swings B's far corner (50, 0, 0) — 5 mm along +X and
        // 5 mm along -Y of the mate point — round to (10, 10, 5).
        let landed = pose.transform_point3(glam::Vec3::new(50.0, 0.0, 0.0));
        assert!(
            (landed - glam::Vec3::new(10.0, 10.0, 5.0)).length() < 1e-2,
            "swung corner lands at {landed:?}"
        );
        // The status names the joint and both parts.
        assert!(state.status.contains("Revolute"), "{}", state.status);
    }

    /// #1013: a hole and a shaft have centre lines you can pick, so "put this peg in that
    /// hole" is one face pair and one line-up row — no fudging with face centres.
    #[test]
    fn lua_joint_seats_a_peg_in_a_hole() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 40, height = 40 }
            bearcad.circle{ x = 20, y = 20, r = 5 }
            bearcad.extrude{
              boolean = { op = "difference", a = { polygon = {0,1,2,3} }, b = { circle = 0 } },
              distance = 6,
            }
            bearcad.circle{ x = 100, y = 0, r = 5 }
            bearcad.extrude{ circle = 1, distance = 20 }
            bearcad.exit_sketch()
            local function face_facing(body, n)
              for _, f in ipairs(bearcad.body_faces(body)) do
                if math.abs(f.normal[1]-n[1]) < 0.01 and math.abs(f.normal[2]-n[2]) < 0.01
                   and math.abs(f.normal[3]-n[3]) < 0.01 then return f end
              end
            end
            assert(#bearcad.body_cylinders(0) == 1, "the plate has one hole")
            assert(#bearcad.body_cylinders(1) == 1, "the peg has one round wall")
            bearcad.joint{
              a = 0, b = 1, kind = "cylindrical",
              face = { moving = face_facing(1, {0,0,-1}), fixed = face_facing(0, {0,0,1}) },
              -- The hole's own centre line is the axis the peg turns and slides about.
              frame_axis = bearcad.body_cylinders(0)[1].axis,
            }
            "#,
        );
        assert_eq!(state.doc.joints.len(), 1);
        let mesh = crate::extrude::body_solid_mesh(&state.doc, bkey(1)).expect("the peg meshes");
        let (min, max) = mesh.bounds().expect("bounds");
        // The peg stands on the plate's top face (z = 6), and Face Snap lands its underside's
        // middle on that face's middle — which, on a square plate with a central hole, is the
        // hole's own centre (20, 20). So the peg comes out concentric with it (#1079).
        assert!((min.z - 6.0).abs() < 0.05, "peg sits on the plate, got {min}");
        // Exact (#1080): the centre a mate lands on is the face's **area** centroid, which is
        // the same point whatever way the mesh happened to triangulate it. Averaging triangle
        // vertices instead put this a couple of tenths out.
        assert!(
            ((min.x + max.x) * 0.5 - 20.0).abs() < 0.01
                && ((min.y + max.y) * 0.5 - 20.0).abs() < 0.01,
            "peg is concentric with the hole, spans {min}..{max}"
        );
    }

    /// #894: `bearcad.begin_joint` arms the tool with its picks instead of committing.
    #[test]
    fn lua_begin_joint_arms_the_tool_without_committing() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.rect{ x = 40, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
            bearcad.begin_joint{
                a = 0, b = 1, kind = "slider",
                face = {
                  moving = bearcad.body_faces(1)[1],
                  fixed  = bearcad.body_faces(0)[1],
                },
            }
            "#,
        );
        assert_eq!(state.tool, crate::actions::Tool::Joint, "the Joint tool comes up armed");
        assert!(state.doc.joints.is_empty(), "nothing is committed");
        let cj = state.creating_joint.as_ref().expect("a joint in progress");
        assert_eq!(cj.members.len(), 2);
        assert!(cj.placement.start_point_a.is_some() && cj.placement.end_point_a.is_some());
        assert!(matches!(cj.kind, crate::model::JointKind::Slider));
    }

    /// #894: `bearcad.edit_joint` re-points a committed joint; a loop-closing edit is
    /// refused loudly.
    #[test]
    fn lua_edit_joint_repoints_and_refuses_loops() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.rect{ x = 40, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
            bearcad.joint{ a = 0, b = 1, kind = "rigid" }
            bearcad.edit_joint{ index = 0, a = 0, b = 1, kind = "slider", position = 3 }
            -- A second joint driving the already-driven body 1 is refused.
            local ok, err = pcall(function()
                bearcad.joint{ a = 0, b = 1, kind = "rigid" }
            end)
            assert(not ok, "a second joint on the same driven part must fail")
            assert(tostring(err):find("already driven"), tostring(err))
            "#,
        );
        assert_eq!(state.doc.joints.len(), 1);
        assert!(matches!(state.doc.joints.values().nth(0).unwrap().kind, crate::model::JointKind::Slider));
        assert_eq!(state.doc.joints.values().nth(0).unwrap().position, "3");
    }

    /// #898: a joint's rest pose — captured at creation, recapturable, and reverted to,
    /// singly or all at once.
    #[test]
    fn lua_joint_rest_set_and_revert() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.rect{ x = 40, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
            bearcad.joint{ a = 0, b = 1, kind = "slider", position = 5 }
            -- Drag the joint elsewhere, then revert: back to the rest captured at creation.
            bearcad.edit_joint{ index = 0, a = 0, b = 1, kind = "slider", position = 12 }
            bearcad.revert_joint(0)
            "#,
        );
        assert_eq!(state.doc.joints.values().nth(0).unwrap().position, "5", "reverted to the creation pose");
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.rect{ x = 40, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
            bearcad.joint{ a = 0, b = 1, kind = "slider", position = 5 }
            bearcad.edit_joint{ index = 0, a = 0, b = 1, kind = "slider", position = 12 }
            -- Recapture: 12 becomes the pose Revert-all returns to.
            bearcad.set_joint_rest(0)
            bearcad.edit_joint{ index = 0, a = 0, b = 1, kind = "slider", position = 3 }
            bearcad.revert_joints()
            "#,
        );
        assert_eq!(state.doc.joints.values().nth(0).unwrap().position, "12", "revert-all returns to the recaptured rest");
    }

    /// #936: shapes cut each other properly — the sphere's kernel solid is a real BREP
    /// sphere, so a boolean against one lands geometry instead of an empty body.
    #[test]
    fn lua_a_shape_cuts_another_shape() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.cuboid{ width = 40, depth = 40, height = 40 }
            bearcad.sphere{ at = {20, 20, 0}, radius = 20 }
            bearcad.combine{ op = "cut", a = {0}, b = {1}, keep_b = true }
            "#,
        );
        let volume = |i: crate::model::BodyKey| {
            crate::extrude::body_solid_mesh(&state.doc, i)
                .map(|m| crate::extrude::mesh_signed_volume(&m).abs())
                .unwrap_or(0.0)
        };
        let cut = state
            .doc
            .bodies
            .iter()
            .find_map(|(k, b)| matches!(b.source, crate::model::BodySource::Boolean { .. }).then_some(k))
            .expect("the cut lands an output body");
        let carved = volume(cut);
        assert!(carved > 0.0, "the cut body has geometry");
        assert!(
            carved < 64000.0 * 0.98,
            "and a bite out of it: {carved} vs the cuboid's 64000"
        );
    }

    /// #1355 Case A: cutting a cutter that floats fully inside the target must leave a
    /// cavity (20³ − 4³ ≈ 7936), not an unsliced copy of the target.
    #[test]
    fn lua_cut_of_a_fully_enclosed_solid_leaves_a_cavity() {
        run_lua_expect_ok(
            r#"
            bearcad.new()
            bearcad.cuboid{ width = 20, depth = 20, height = 20 }
            -- 4³ sitting at z=8: [-2,2]×[-2,2]×[8,12] inside the 20³ on the ground.
            bearcad.cuboid{ width = 4, depth = 4, height = 4, at = {0, 0, 8} }
            bearcad.combine{ op = "cut", a = {0}, b = {1} }
            local s = bearcad.body_stats(2)
            assert(s ~= nil, "enclosed cut must produce a real body")
            assert(math.abs(s.volume - 7936) < 5,
                   "cavity volume should be 20^3-4^3=7936, got " .. tostring(s.volume))
            assert(s.triangles > 0)
        "#,
        );
    }

    /// #1355 Case B: A−B is empty when the target sits wholly inside the cutter. The op
    /// must error (no phantom body) and leave the document exportable.
    #[test]
    fn lua_cut_of_a_body_wholly_inside_the_cutter_errors() {
        let path = std::env::temp_dir().join(format!(
            "bearcad_lua_empty_cut_{}.stl",
            std::process::id()
        ));
        let path_s = path.to_string_lossy().replace('\\', "\\\\");
        let _ = std::fs::remove_file(&path);
        run_lua_expect_ok(&format!(
            r#"
            bearcad.new()
            bearcad.cuboid{{ width = 4, depth = 4, height = 4 }}
            bearcad.cuboid{{ width = 50, depth = 50, height = 50 }}
            local n = bearcad.count("body")
            local ok, err = pcall(function()
                bearcad.combine{{ op = "cut", a = {{0}}, b = {{1}} }}
            end)
            assert(not ok, "empty cut must raise, got success")
            assert(tostring(err):lower():find("empty", 1, true),
                   "error should say the result is empty, got " .. tostring(err))
            assert(bearcad.count("body") == n, "must not leave a phantom body")
            assert(bearcad.body_stats(0) ~= nil)
            assert(bearcad.body_stats(n) == nil)
            bearcad.export_stl("{path_s}")
        "#
        ));
        assert!(
            path.is_file(),
            "export of the remaining real bodies must still work"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// #1356: intersect of disjoint bodies is empty — error, no phantom, export still works.
    #[test]
    fn lua_intersect_of_disjoint_bodies_errors() {
        let path = std::env::temp_dir().join(format!(
            "bearcad_lua_empty_intersect_{}.stl",
            std::process::id()
        ));
        let path_s = path.to_string_lossy().replace('\\', "\\\\");
        let _ = std::fs::remove_file(&path);
        run_lua_expect_ok(&format!(
            r#"
            bearcad.new()
            bearcad.cuboid{{ width = 10, depth = 10, height = 10, at = {{0, 0, 0}} }}
            bearcad.cuboid{{ width = 10, depth = 10, height = 10, at = {{50, 50, 50}} }}
            local n = bearcad.count("body")
            local ok, err = pcall(function()
                bearcad.combine{{ op = "intersect", a = {{0}}, b = {{1}} }}
            end)
            assert(not ok, "empty intersect must raise, got success")
            assert(tostring(err):lower():find("empty", 1, true),
                   "error should say the result is empty, got " .. tostring(err))
            assert(bearcad.count("body") == n, "must not leave a phantom body")
            assert(bearcad.body_stats(n) == nil)
            bearcad.export_stl("{path_s}")
        "#
        ));
        assert!(
            path.is_file(),
            "export of the remaining real bodies must still work"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// #926: a new body extruded off another body's face is made of the same material.
    #[test]
    fn lua_extrude_off_a_body_face_inherits_its_material() {
        let state = run_lua(
            r##"
            bearcad.new()
            bearcad.rect{ width = 20, height = 20 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
            bearcad.material{ name = "Brass", color = "#c88a4a", bodies = {0} }
            bearcad.exit_sketch()
            -- A sketch on the block's top cap, extruded into a body of its own.
            bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 0, profile = "polygon",
                                  profile_lines = {0, 1, 2, 3}, top = true }
            bearcad.circle{ x = 10, y = 10, r = 4 }
            bearcad.exit_sketch()
            bearcad.extrude{ circle = 0, distance = 6 }
        "##,
        );
        let brass = state
            .doc
            .materials
            .keys()
            .nth(crate::model::Material::DEFAULTS.len())
            .expect("Brass");
        assert_eq!(state.doc.bodies[bkey(0)].material, Some(brass));
        assert_eq!(
            state.doc.bodies.get(bkey(1)).and_then(|b| b.material),
            Some(brass),
            "the boss inherits the block's material"
        );
    }

    /// #917: the Move tool's rotation candidates sit this far apart, in degrees — 90 to
    /// begin with, and clamped to 0–90 however it's set.
    #[test]
    fn lua_move_angle_snap_clamps_to_the_range() {
        assert_eq!(
            AppState::default().move_angle_snap_deg,
            crate::actions::MAX_ANGLE_SNAP_DEG
        );
        let state = run_lua("bearcad.ui.angle_snap(45)");
        assert_eq!(state.move_angle_snap_deg, 45.0);
        let state = run_lua("bearcad.ui.angle_snap(120)");
        assert_eq!(state.move_angle_snap_deg, 90.0, "clamped to 90");
        let state = run_lua("bearcad.ui.angle_snap(-10)");
        assert_eq!(state.move_angle_snap_deg, 0.0, "and to 0");
    }

    /// #909: the shape calls place primitive solids straight into 3D — each its own body,
    /// sized by expressions, with no sketch behind it.
    #[test]
    fn lua_shapes_place_primitive_solids() {
        let state = run_lua(
            r#"
            bearcad.cuboid{ width = 40, depth = 20, height = 10, name = "Block" }
            bearcad.cylinder{ at = {100, 0, 0}, radius = 5, height = 20 }
            bearcad.sphere{ at = {200, 0, 0}, radius = 8 }
            "#,
        );
        assert_eq!(state.doc.primitives.len(), 3, "three shapes");
        assert_eq!(state.doc.bodies.len(), 3, "each shape owns a body");
        let first = state.doc.primitives.keys().next().expect("the first shape");
        assert_eq!(state.doc.primitives[first].name.as_deref(), Some("Block"));
        let volume = |i: crate::model::BodyKey| {
            crate::extrude::body_solid_mesh(&state.doc, i)
                .map(|m| crate::extrude::mesh_signed_volume(&m).abs())
                .unwrap_or(0.0)
        };
        assert!((volume(bkey(0)) - 8000.0).abs() < 1.0, "cuboid {}", volume(bkey(0)));
        let cylinder = std::f32::consts::PI * 25.0 * 20.0;
        assert!((volume(bkey(1)) - cylinder).abs() / cylinder < 0.02, "cylinder {}", volume(bkey(1)));
        let sphere = 4.0 / 3.0 * std::f32::consts::PI * 512.0;
        assert!((volume(bkey(2)) - sphere).abs() / sphere < 0.03, "sphere {}", volume(bkey(2)));
    }

    /// #909: a shape's dimensions are expressions, so it follows its parameters — and
    /// `edit_shape` re-points one in place, keeping its name and its body.
    #[test]
    fn lua_shapes_are_parametric_and_editable() {
        let state = run_lua(
            r#"
            bearcad.parameter("add", "side", "10")
            bearcad.cuboid{ width = "side", depth = "side", height = "side", name = "Cube" }
            bearcad.edit_shape{ index = 0, height = "side * 3" }
            "#,
        );
        let cube = state.doc.primitives.keys().next().expect("the cube");
        assert_eq!(state.doc.primitives[cube].width, "side");
        assert_eq!(state.doc.primitives[cube].height, "side * 3");
        assert_eq!(
            state.doc.primitives[cube].name.as_deref(),
            Some("Cube"),
            "the name survives"
        );
        assert_eq!(state.doc.bodies.len(), 1, "editing reuses the body");
        let stats = crate::extrude::body_solid_mesh(&state.doc, bkey(0))
            .and_then(|m| m.bounds())
            .expect("the cube meshes");
        assert!((stats.1.z - 30.0).abs() < 1e-3, "3 x side tall, got {}", stats.1.z);
    }

    /// #909: deleting a shape takes its body with it, and a shape missing a dimension is
    /// refused rather than landing an empty body.
    #[test]
    fn lua_shape_delete_and_refusal() {
        let state = run_lua(
            r#"
            bearcad.cuboid{ width = 10, depth = 10, height = 10 }
            bearcad.select{ kind = "shape", index = 0 }
            bearcad.delete_selection()
            "#,
        );
        assert!(state.doc.primitives.is_empty(), "the shape is gone for real (#1055)");
        assert!(!state.doc.bodies.contains(bkey(0)), "and so is its body");
        let mut runner =
            ScriptRunner::from_lua_source("bearcad.cylinder{ radius = 5 }").unwrap();
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, None, &ctx);
        }
        assert!(runner.error.is_some(), "a cylinder with no height is refused");
        assert!(state.doc.primitives.is_empty(), "and nothing lands");
    }

    /// #906: the joint preview's animation is one app-wide switch, on to begin with.
    #[test]
    fn lua_joint_animation_toggles_app_wide() {
        assert!(
            AppState::default().animate_joints,
            "the joint preview animates until it's turned off"
        );
        let state = run_lua("bearcad.ui.animate_joints(false)");
        assert!(!state.animate_joints, "the script turned the animation off");
        let state = run_lua("bearcad.ui.animate_joints(false) bearcad.ui.animate_joints(true)");
        assert!(state.animate_joints, "and back on");
    }

    /// #900: a rigid joint takes more than two parts — a rigid group — and the pane label
    /// says so. Tying things together never moves them, and selected parts walk straight
    /// into the tool.
    #[test]
    fn lua_rigid_group_ties_three_parts() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.rect{ x = 40, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
            bearcad.rect{ x = 80, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {8, 9, 10, 11}, distance = 5 }
            bearcad.joint{ parts = {0, 1, 2}, kind = "rigid" }
            -- A slider with three parts is refused: only rigid ties more than two.
            local ok, err = pcall(function()
                bearcad.joint{ parts = {0, 1, 2}, kind = "slider" }
            end)
            assert(not ok and tostring(err):find("rigid"), tostring(err))
            "#,
        );
        assert_eq!(state.doc.joints.values().nth(0).unwrap().members.len(), 3);
        assert_eq!(
            crate::names::node_label(&state.doc, crate::hierarchy::HierarchyNode::Joint(jkey(0))),
            "Rigid group 0"
        );
        // Tying in place moves nothing: every driven pose is identity.
        for n in 1..=2 {
            let bi = state.doc.body_at(n).unwrap();
            let pose = crate::joints::body_joint_pose(&state.doc, bi).unwrap();
            assert!(
                pose.abs_diff_eq(glam::Mat4::IDENTITY, 1e-5),
                "body {n} must stay put"
            );
        }
        // Selected parts walk straight into the tool (#900).
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.rect{ x = 40, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
            bearcad.exit_sketch()
            bearcad.select({ kind = "body", index = 0 })
            bearcad.select({ kind = "body", index = 1 }, true)
            bearcad.ui.tool("joint")
            "#,
        );
        let cj = state.creating_joint.as_ref().expect("tool armed");
        assert_eq!(cj.members.len(), 2, "the selection seeds the members");
    }

    /// #649/#650: an **edge midpoint** works as either point too.
    #[test]
    fn lua_move_snaps_from_an_edge_midpoint() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.rect{ x = 40, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
            -- A's bottom front edge midpoint (5, 0, 0) onto B's (45, 0, 0).
            bearcad.move_bodies{
                bodies = {0},
                from = { body = 0, edge = { {0, 0, 0}, {10, 0, 0} } },
                to   = { body = 1, edge = { {40, 0, 0}, {50, 0, 0} } },
            }
            "#,
        );
        let op = &state.doc.move_ops.values().nth(0).unwrap();
        let t = crate::extrude::move_op_translation(&state.doc, op).expect("translation");
        assert!(
            (t - glam::Vec3::new(40.0, 0.0, 0.0)).length() < 1e-3,
            "midpoint-to-midpoint offset should be +40 X, got {t:?}"
        );
    }

    /// #1079: a joint's frame is its own — scripted as `frame_axis`, and left to the mate to
    /// seed when it isn't given. A mate that names a fixed face seeds the axis from it, so the
    /// common case still needs nothing said.
    #[test]
    fn lua_joint_frame_is_set_or_seeded() {
        let script = |extra: &str| {
            format!(
                r#"
                bearcad.rect{{ width = 10, height = 10 }}
                bearcad.extrude{{ polygon = {{0, 1, 2, 3}}, distance = 5 }}
                bearcad.rect{{ x = 40, y = 0, width = 10, height = 10 }}
                bearcad.extrude{{ polygon = {{4, 5, 6, 7}}, distance = 5 }}
                bearcad.joint{{ a = 0, b = 1, kind = "revolute",
                  face = {{ moving = {{ body = 1, face = {{45, 5, 0}}, normal = {{0, 0, -1}} }},
                           fixed = {{ body = 0, face = {{5, 5, 5}}, normal = {{0, 0, 1}} }} }}{extra} }}
                "#
            )
        };
        // No frame given: the mate's fixed face seeds it.
        let seeded = run_lua(&script(""));
        let j = seeded.doc.joints.values().next().unwrap();
        assert_eq!(
            j.frame.primary,
            j.placement.end_point_a.as_ref().and_then(crate::model::move_point_host_mate_ref),
            "the fixed face is the axis"
        );
        assert!(j.frame.secondary.is_none(), "nothing else named, so no second axis");

        // Given outright, it is used as given rather than seeded over.
        let set = run_lua(&script(r#", frame_axis = { axis = "x" }"#));
        let j = set.doc.joints.values().next().unwrap();
        assert_eq!(
            j.frame.primary,
            Some(crate::model::MateRef::Axis(crate::construction::GlobalAxis::X))
        );
    }

    /// #1078: the third pair can be set as a `roll` **angle** instead of a target point —
    /// no third point needed, since the spin about the `endA → endB` axis is simply that many
    /// degrees. It round-trips through an exported session script.
    #[test]
    fn lua_move_third_pair_can_be_an_angle() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.rect{ x = 40, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
            bearcad.move_bodies{ bodies = {0},
              from = { body = 0, vertex = {0, 0, 0} },
              to   = { body = 1, vertex = {40, 0, 0} },
              from_b = { body = 0, vertex = {10, 0, 0} },
              to_b = { body = 1, vertex = {50, 0, 0} },
              roll = 90 }
            "#,
        );
        let op = state.doc.move_ops.values().next().unwrap();
        assert_eq!(op.roll_angle, "90");
        assert!(op.has_snap_roll_angle(), "no third point, and none needed");
        let (_, angle) =
            crate::extrude::move_snap_roll_axis_angle(&state.doc, op).expect("a spin");
        assert!((angle - std::f32::consts::FRAC_PI_2).abs() < 1e-4, "{angle}");
    }

    /// #1077: naming two faces and nothing else is asking for one to be put on the other, so
    /// a scripted move with two `on_face` points is a **Face Snap** — with `flip` for which
    /// side and `spin` for the turn. Naming a B pair says the turn comes from a second point
    /// pair instead, so a script written before Face Snap existed still means Point Snap.
    #[test]
    fn lua_two_face_points_make_a_face_snap() {
        let script = |extra: &str| {
            format!(
                r#"
                bearcad.rect{{ width = 10, height = 10 }}
                bearcad.extrude{{ polygon = {{0, 1, 2, 3}}, distance = 5 }}
                bearcad.rect{{ x = 40, y = 0, width = 10, height = 10 }}
                bearcad.extrude{{ polygon = {{4, 5, 6, 7}}, distance = 5 }}
                bearcad.move_bodies{{ bodies = {{0}},
                  from = {{ body = 0, on_face = {{5, 5, 5}}, normal = {{0, 0, 1}} }},
                  to   = {{ body = 1, on_face = {{40, 5, 2.5}}, normal = {{-1, 0, 0}} }}{extra} }}
                "#
            )
        };
        let state = run_lua(&script(", spin = 45, flip = true"));
        let op = state.doc.move_ops.values().next().unwrap();
        assert_eq!(op.translate_mode, crate::model::MoveTranslateMode::FaceSnap);
        assert!(op.face_flip);
        assert_eq!(op.face_spin, "45");
        assert!(crate::extrude::move_op_transform(&state.doc, op).is_some());

        // A B pair means the turn is coming from points, which is Point Snap.
        let with_b = run_lua(&script(
            r#",
                  from_b = { body = 0, vertex = {0, 0, 5} },
                  to_b = { body = 1, vertex = {40, 0, 5} }"#,
        ));
        assert_eq!(
            with_b.doc.move_ops.values().next().unwrap().translate_mode,
            crate::model::MoveTranslateMode::PointSnap
        );
    }

    /// #1234: Free Move's rotation rings are scriptable as `move_rx` / `move_ry` / `move_rz`
    /// (radians), matching the viewport drag gizmos.
    #[test]
    fn lua_free_move_rotation_gizmos() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.begin_move{ bodies = {0} }
            bearcad.ui.tool_mode("free")
            local names = {}
            for _, g in ipairs(bearcad.gizmos()) do names[g.name] = g.value end
            assert(names.move_x ~= nil, "translation gizmos present")
            assert(names.move_rx ~= nil and names.move_ry ~= nil and names.move_rz ~= nil,
                   "rotation gizmos present")
            bearcad.set_gizmo{ name = "move_rz", value = math.pi / 2 }
            local rz
            for _, g in ipairs(bearcad.gizmos()) do
                if g.name == "move_rz" then rz = g.value end
            end
            assert(math.abs(rz - math.pi / 2) < 1e-3)
            "#,
        );
        let cm = state.creating_move.as_ref().expect("move armed");
        assert_eq!(cm.translate_mode, crate::model::MoveTranslateMode::Free);
        assert_eq!(cm.rz, "90.0 deg");
    }

    /// #1320: Shape kinds are scriptable as `bearcad.ui.tool_mode`.
    #[test]
    fn lua_shape_tool_mode_cycles_kinds() {
        let state = run_lua(
            r#"
            bearcad.ui.tool("shape")
            bearcad.ui.tool_mode("cylinder")
            "#,
        );
        assert_eq!(state.tool, crate::actions::Tool::Shape);
        assert_eq!(state.shape_kind, crate::model::PrimitiveKind::Cylinder);
        assert_eq!(
            state.creating_shape.as_ref().unwrap().shape.kind,
            crate::model::PrimitiveKind::Cylinder
        );
        let state = run_lua(
            r#"
            bearcad.ui.tool("shape")
            bearcad.ui.tool_mode("sphere")
            bearcad.ui.tool_mode("cuboid")
            "#,
        );
        assert_eq!(state.shape_kind, crate::model::PrimitiveKind::Cuboid);
        let refused = run_lua(
            r#"
            bearcad.ui.tool("shape")
            local ok, err = pcall(bearcad.ui.tool_mode, "free")
            assert(not ok, "the Shape tool has no Free mode")
            assert(tostring(err):find("free"), "unexpected error: " .. tostring(err))
            "#,
        );
        assert_eq!(refused.tool, crate::actions::Tool::Shape);
    }

    /// #1076: Free mode's turns are scriptable alongside its amounts, and `set_mode` names all
    /// the modes the Move tool has — but not In place, which is the Joint tool's.
    #[test]
    fn lua_move_free_mode_turns_and_names_its_modes() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.move_bodies{ bodies = {0}, x = 10, rz = 90 }
            "#,
        );
        let op = state.doc.move_ops.values().next().unwrap();
        assert_eq!(op.rz, "90");
        let m = crate::extrude::move_op_transform(&state.doc, op).expect("transform");
        // Turned about the box's own centre (5, 5), then carried 10 along +X.
        let corner = m.transform_point3(glam::Vec3::ZERO);
        assert!((corner - glam::Vec3::new(20.0, 0.0, 0.0)).length() < 1e-3, "{corner:?}");

        // The mode names the tool answers to.
        for name in ["point_snap", "snap", "face_snap", "free", "xyz"] {
            let s = run_lua(&format!(
                r#"
                bearcad.rect{{ width = 10, height = 10 }}
                bearcad.extrude{{ polygon = {{0, 1, 2, 3}}, distance = 5 }}
                bearcad.begin_move{{ bodies = {{0}} }}
                bearcad.ui.tool_mode("{name}")
                "#
            ));
            assert!(s.creating_move.is_some(), "{name}");
        }
        // In place belongs to the Joint tool, so Move refuses it rather than silently
        // accepting a mode it has no rows for.
        let refused = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.begin_move{ bodies = {0} }
            local ok, err = pcall(bearcad.ui.tool_mode, "in_place")
            assert(not ok, "the Move tool has no In place mode")
            assert(tostring(err):find("in_place"), "unexpected error: " .. tostring(err))
            "#,
        );
        assert!(refused.creating_move.is_some());
    }

    /// #1074: `on_face` puts a point **within** a face — the face's selection key plus how far
    /// across it to sit, in the face's own axes. A move from a corner of one box's top cap
    /// onto the middle of another's lands where those two points meet.
    #[test]
    fn lua_move_snaps_from_a_point_on_a_face() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.rect{ x = 40, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
            -- A's top cap (centre 5,5,5) offset +5 along the face's u axis, onto the
            -- middle of B's top cap (45, 5, 5).
            bearcad.move_bodies{
                bodies = {0},
                from = { body = 0, on_face = {5, 5, 5}, normal = {0, 0, 1}, uv = {5, 0} },
                to   = { body = 1, face_center = {45, 5, 5}, normal = {0, 0, 1} },
            }
            "#,
        );
        let op = &state.doc.move_ops.values().next().unwrap();
        let from = op.start_point_a.as_ref().expect("the source point");
        assert!(
            matches!(from, crate::model::MovePointRef::OnFace { uv, .. } if *uv == [500, 0]),
            "the offset across the face is stored: {from:?}"
        );
        let a = crate::extrude::move_point_world(&state.doc, from).expect("source resolves");
        // The top cap's frame is `plane_basis(+Z)` = (u = +X, v = +Y), so +5u is x = 10.
        assert!((a - glam::Vec3::new(10.0, 5.0, 5.0)).length() < 1e-2, "{a:?}");
        let t = crate::extrude::move_op_translation(&state.doc, op).expect("translation");
        assert!(
            (t - glam::Vec3::new(35.0, 0.0, 0.0)).length() < 1e-2,
            "corner-to-centre offset should be +35 X, got {t:?}"
        );
    }

    /// #645: `repeat_bodies{ to = }` measures the fill length to a picked plane instead of a
    /// typed number, and the pattern follows that plane when it moves.
    #[test]
    fn lua_repeat_distance_to_a_plane() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            -- A wall 40mm out along +X, then fill up to it at a 10mm pitch.
            bearcad.plane{ origin = {40, 0, 0}, normal = {1, 0, 0} }
            bearcad.repeat_bodies{
                bodies = {0}, axis = "x", mode = "fill_pitch", spacing = 10,
                to = { plane = 3 },
            }
            "#,
        );
        let op = &state.doc.repeat_ops.values().nth(0).unwrap();
        assert!(op.length_target.is_some(), "the plane target is stored");
        let offsets = crate::extrude::repeat_offsets(&state.doc, op).expect("offsets");
        assert!(
            !offsets.is_empty(),
            "the target-derived length produced instances"
        );
        assert!(
            offsets.last().copied().unwrap_or(0.0) <= 40.0 + 1e-3,
            "the pattern stops at the wall, got {offsets:?}"
        );
    }

    /// #989: `repeat_bodies{ flip = true }` runs the pattern the other way along the path. A
    /// path has two directions and picking one says nothing about which you meant, so this is
    /// how you say it — and the copies really land on the other side.
    #[test]
    fn lua_repeat_flip_runs_the_other_way() {
        let bodies_at = |flip: &str| {
            let state = run_lua(&format!(
                r#"
                bearcad.rect{{ width = 10, height = 10 }}
                bearcad.extrude{{ polygon = {{0, 1, 2, 3}}, distance = 5 }}
                bearcad.repeat_bodies{{ bodies = {{0}}, axis = "x", count = 3, gap = 5{flip} }}
                "#
            ));
            let op = &state.doc.repeat_ops.values().nth(0).unwrap();
            assert!(op.outputs.len() >= 2, "the repeat made copies");
            op.outputs
                .iter()
                .filter_map(|&bi| crate::extrude::body_solid_mesh(&state.doc, bi))
                .map(|m| {
                    m.triangles
                        .iter()
                        .flatten()
                        .map(|p| p.x)
                        .fold(f32::NEG_INFINITY, f32::max)
                })
                .fold(f32::NEG_INFINITY, f32::max)
        };
        let plain = bodies_at("");
        let flipped = bodies_at(", flip = true");
        assert!(
            plain > 10.0,
            "unflipped, the copies march out along +X, got a far edge at {plain}"
        );
        assert!(
            flipped < 0.0,
            "flipped, they march the other way instead, got a far edge at {flipped}"
        );
    }

    /// #639: `mirror_bodies{ output = }` picks how the reflections land. `join` fuses each
    /// into its own source and consumes it; the default keeps the original alongside.
    #[test]
    fn lua_mirror_output_mode() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 20, height = 20 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 6 }
            bearcad.mirror_bodies{
                plane = { kind = "construction_plane", index = 0 },
                bodies = {0}, output = "join",
            }
            "#,
        );
        assert_eq!(state.doc.mirror_ops.values().nth(0).unwrap().mode, crate::model::MirrorMode::Join);
        assert!(state.doc.bodies.values().nth(0).unwrap().shadow, "a join consumes its source");
        let out = state.doc.mirror_ops.values().nth(0).unwrap().outputs[0];
        let (min, max) = crate::extrude::body_solid_mesh(&state.doc, out)
            .and_then(|m| m.bounds())
            .expect("joined mesh");
        assert!(min.z < -5.9 && max.z > 5.9, "spans both halves, got {min:?}..{max:?}");
        // An unknown output name is a clear error, named in the message.
        run_lua(
            r#"
            local ok, err = pcall(function()
                bearcad.mirror_bodies{ plane = { kind = "construction_plane", index = 0 },
                                       bodies = {0}, output = "sideways" }
            end)
            assert(not ok, "unknown output should error")
            assert(tostring(err):find("sideways"), tostring(err))
            "#,
        );
    }

    /// #1354: `plane = 0` is a construction-plane ordinal, same as
    /// `{ kind = "construction_plane", index = 0 }`. A non-spec value is a clear error,
    /// not "error converting Lua integer to table".
    #[test]
    fn lua_mirror_bodies_accepts_a_bare_plane_ordinal() {
        let state = run_lua(
            r#"
            bearcad.cuboid{ width = 20, depth = 20, height = 10 }
            bearcad.mirror_bodies{ plane = 0, bodies = {0} }
            bearcad.edit_mirror{ index = 0, plane = 1, bodies = {0} }
            "#,
        );
        let op = state.doc.mirror_ops.values().next().expect("mirror op");
        assert_eq!(op.plane, FaceId::ConstructionPlane(pkey(1)));
        assert_eq!(op.targets.len(), 1);
        // A string is a clear error listing accepted forms — not a type-conversion dump.
        run_lua(
            r#"
            bearcad.cuboid{ width = 20, depth = 20, height = 10 }
            local ok, err = pcall(function()
                bearcad.mirror_bodies{ plane = "xy", bodies = {0} }
            end)
            assert(not ok, "a string plane should error")
            err = tostring(err)
            assert(not err:find("converting Lua integer to table", 1, true), err)
            assert(not err:find("converting Lua string to table", 1, true), err)
            assert(err:find("plane", 1, true) and err:find("construction_plane", 1, true), err)
            "#,
        );
    }

    /// #647: body geometry is derivable — an edge's length and the distance between two mesh
    /// corners, given as plain mm points on the body.
    #[test]
    fn lua_derive_parameter_measures_body_edges_and_corners() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 30, height = 40 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
            bearcad.derive_parameter{
                kind = "body_edge_length", body = 0,
                a = {0, 0, 0}, b = {30, 0, 0}, name = "edge",
            }
            bearcad.derive_parameter{
                kind = "body_vertex_distance", body = 0,
                a = {0, 0, 0}, b = {30, 40, 0}, name = "diag",
            }
            "#,
        );
        let value = |name: &str| {
            let p = state.doc.parameters.values().find(|p| p.name == name).unwrap();
            crate::value::computed_length_in_doc(&p.expression, &state.doc).unwrap()
        };
        assert!((value("edge") - 30.0).abs() < 1e-2, "edge = {}", value("edge"));
        assert!((value("diag") - 50.0).abs() < 1e-2, "diag = {}", value("diag"));
        // Both are read-only, geometry-driven parameters.
        for name in ["edge", "diag"] {
            let p = state.doc.parameters.values().find(|p| p.name == name).unwrap();
            assert!(p.source.is_some(), "{name} is derived");
        }
    }

    /// #643: a repeat axis can be a **body edge**, given by the body it lives on plus the
    /// edge's world endpoints — the scripted form of picking an edge in the viewport.
    #[test]
    fn lua_repeat_axis_accepts_a_body_edge() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 20, height = 20 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            -- The body's bottom front edge runs along +X.
            bearcad.repeat_bodies{
                bodies = {0},
                axis = { body = 0, from = {0, 0, 0}, to = {20, 0, 0} },
                count = 3, gap = 5,
            }
            "#,
        );
        let op = &state.doc.repeat_ops.values().nth(0).unwrap();
        assert_eq!(
            op.axis,
            crate::model::RevolveAxis::BodyEdge {
                body: bkey(0),
                a: glam::Vec3::ZERO,
                b: glam::Vec3::new(20.0, 0.0, 0.0),
            }
        );
        // It resolves to a real direction, so the repeat actually produced copies.
        assert_eq!(
            crate::extrude::axis_world(&state.doc, op.axis).map(|(_, d)| d),
            Some(glam::Vec3::X)
        );
        assert!(!op.outputs.is_empty(), "the repeat produced instances");
        // …and the axis renders back out in the form the parser accepts.
        let rendered = crate::script::revolve_axis_lua(op.axis);
        assert_eq!(rendered, "{ body = 0, from = { 0, 0, 0 }, to = { 20, 0, 0 } }");
        let round_tripped = run_lua(&format!(
            r#"
            bearcad.rect{{ width = 20, height = 20 }}
            bearcad.extrude{{ polygon = {{0, 1, 2, 3}}, distance = 5 }}
            bearcad.repeat_bodies{{ bodies = {{0}}, axis = {rendered}, count = 3, gap = 5 }}
            "#
        ));
        assert_eq!(round_tripped.doc.repeat_ops.values().nth(0).unwrap().axis, op.axis);
    }

    /// #406: a boolean-profiled extrusion's cap hosts a scripted sketch, and a drawing's
    /// page size/margin are scriptable (omitted keys keep the current value).
    #[test]
    fn lua_boolean_cap_sketch_and_drawing_page() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 30, height = 30 }
            bearcad.circle{ x = 30, y = 15, r = 10 }
            bearcad.extrude{
                boolean = { op = "difference", a = { polygon = {0, 1, 2, 3} }, b = { circle = 0 } },
                distance = 8,
            }
            -- Sketch on the boolean profile's top cap, like clicking it in the GUI.
            bearcad.begin_sketch{
                kind = "extrude_cap", extrusion = 0, top = true,
                profile = "boolean",
                boolean = { op = "difference", a = { polygon = {0, 1, 2, 3} }, b = { circle = 0 } },
            }
            bearcad.circle{ x = 5, y = 5, r = 2 }
            bearcad.exit_sketch()

            local d = bearcad.drawing{}
            bearcad.drawing_page{ drawing = d, width = 297, height = 210, margin = 12 }
            bearcad.drawing_page{ drawing = d, margin = 8 } -- partial update keeps the size
            "#,
        );
        assert_eq!(state.doc.sketches.len(), 2, "cap sketch created: {}", state.status);
        assert_eq!(state.doc.circles.len(), 2);
        let d = &state.doc.drawings[dkey(0)];
        assert_eq!(
            (d.page_width_mm, d.page_height_mm, d.margin_mm),
            (297.0, 210.0, 8.0)
        );
    }

    /// #402: an expression that doesn't evaluate is a script error, not silence.
    #[test]
    fn lua_bad_size_expression_raises() {
        run_lua_expect_ok(
            r#"
            local ok, err = pcall(function()
                bearcad.rect{ width = "nope + 1", height = 10 }
            end)
            assert(not ok, "bad expression should fail the call")
            assert(tostring(err):find("nope"), "error should name the expression: " .. tostring(err))
            "#,
        );
    }

    /// #402: edit_extrusion can set a parametric distance expression.
    #[test]
    fn lua_edit_extrusion_accepts_expression() {
        let state = run_lua(
            r#"
            bearcad.parameter("add", "d", "9")
            bearcad.rect{ width = 20, height = 20 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.edit_extrusion{ extrusion = 0, distance = "d" }
            "#,
        );
        assert!((state.doc.extrusions[xkey(0)].distance - 9.0).abs() < 1e-3);
        assert_eq!(state.doc.extrusions[xkey(0)].expression, "d");
    }

    /// #1176/#1180: parameter min/max/step and private are scriptable.
    /// `private true` hides the knob (secondary); default plain value is public.
    #[test]
    fn lua_parameter_bounds_and_private() {
        let state = run_lua(
            r#"
            bearcad.parameter("add", "width", "10mm")
            bearcad.parameter("min", 0, "5mm")
            bearcad.parameter("max", 0, "50mm")
            bearcad.parameter("step", 0, "2.5mm")
            bearcad.parameter("private", 0, true)
            "#,
        );
        let p = state.doc.parameters.values().next().unwrap();
        assert_eq!(p.minimum.as_deref(), Some("5mm"));
        assert_eq!(p.maximum.as_deref(), Some("50mm"));
        assert_eq!(p.step.as_deref(), Some("2.5mm"));
        assert!(!p.primary, "private true ⇒ secondary");
        let state = run_lua(
            r#"
            bearcad.parameter("add", "width", "10mm")
            bearcad.parameter("min", 0, "5mm")
            bearcad.parameter("min", 0)  -- clear
            "#,
        );
        assert!(state.doc.parameters.values().next().unwrap().minimum.is_none());
    }

    /// #107: `bearcad.parameter("get"/"get_expression", name)` reads a parameter back.
    #[test]
    fn lua_parameter_get_returns_value_and_expression() {
        run_lua_expect_ok(
            r#"
            bearcad.new()
            bearcad.parameter("add", "A", "5mm")
            local v = bearcad.parameter("get", "A")
            assert(math.abs(v - 5) < 1e-4, "A should evaluate to 5mm, got " .. tostring(v))
            assert(bearcad.parameter("get_expression", "A") == "5mm")
            assert(bearcad.parameter("get", "missing") == nil)
        "#,
        );
    }

    /// #108: `circle{ radius = 12 }` is an alias of `r`; omitting all size keys is a clear
    /// error naming the accepted keys.
    #[test]
    fn lua_circle_accepts_radius_alias_and_errors_without_a_size() {
        run_lua_expect_ok(
            r#"
            bearcad.new()
            bearcad.circle{ radius = 12 }
            local c = bearcad.get{ kind = "circle", index = 0 }
            assert(math.abs(c.r - 12) < 1e-4)
            local ok, err = pcall(bearcad.circle, { x = 0, y = 0 })
            assert(not ok, "circle without a size should error")
            err = tostring(err)
            assert(err:find("radius") and err:find("diameter"),
                   "error should name the accepted keys: " .. err)
        "#,
        );
    }

    /// #108: `bearcad.ui.elements_view(...)` drives the Elements pane's layout mode.
    #[test]
    fn lua_elements_view_sets_hierarchy_view_mode() {
        let state = run_lua(r#"bearcad.ui.elements_view("graph")"#);
        assert_eq!(
            state.hierarchy_view_mode,
            crate::hierarchy::HierarchyViewMode::Graph
        );
    }

    #[test]
    fn lua_elements_view_rejects_unknown_mode() {
        run_lua_expect_ok(
            r#"
            local ok = pcall(bearcad.ui.elements_view, "spiral")
            assert(not ok, "unknown elements view should error")
        "#,
        );
    }

    /// #108: `bearcad.ui.camera{...}` sets the pose instantly and `bearcad.ui.camera{}`
    /// reads it back.
    #[test]
    fn lua_camera_set_and_get_round_trips() {
        run_lua_expect_ok(
            r#"
            bearcad.new()
            bearcad.ui.camera{ yaw = 1.0, distance = 200, target = {1, 2, 3} }
            local c = bearcad.ui.camera{}
            assert(math.abs(c.yaw - 1.0) < 1e-4, "yaw " .. c.yaw)
            assert(math.abs(c.distance - 200) < 1e-3, "distance " .. c.distance)
            assert(math.abs(c.target[1] - 1) < 1e-4)
            assert(math.abs(c.target[2] - 2) < 1e-4)
            assert(math.abs(c.target[3] - 3) < 1e-4)
            assert(type(c.pitch) == "number")
            assert(c.projection == "perspective")
            -- a partial set leaves the other fields alone
            bearcad.ui.camera{ pitch = 0.5 }
            local c2 = bearcad.ui.camera{}
            assert(math.abs(c2.pitch - 0.5) < 1e-4)
            assert(math.abs(c2.yaw - 1.0) < 1e-4)
            assert(math.abs(c2.distance - 200) < 1e-3)
        "#,
        );
    }

    /// #108/#1276: `bearcad.ui.zoom_fit()` frames the document — the camera target lands on
    /// the body's bbox center after the glide (or instantly when animation is off).
    #[test]
    fn lua_zoom_fit_targets_the_document_center() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ x = 0, y = 0, width = 40, height = 30 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
            bearcad.ui.zoom_fit()
        "#,
        );
        let expected = glam::Vec3::new(20.0, 15.0, 5.0);
        assert!(
            (state.cam.target - expected).length() < 0.5,
            "zoom_fit should center the target on the body, got {:?}",
            state.cam.target
        );
        assert!(
            !state.cam.is_transitioning(),
            "yielding zoom_fit waits out the transition"
        );
        assert!(state.cam.distance > 0.0 && state.cam.distance.is_finite());
    }

    /// #1276: `bearcad.ui.animate_zoom_to_fit(false)` makes zoom_fit snap.
    #[test]
    fn lua_animate_zoom_to_fit_toggles() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.ui.animate_zoom_to_fit(false)
            bearcad.rect{ x = 0, y = 0, width = 40, height = 30 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
            bearcad.ui.zoom_fit()
        "#,
        );
        assert!(!state.animate_zoom_to_fit);
        assert!(!state.cam.is_transitioning());
        let expected = glam::Vec3::new(20.0, 15.0, 5.0);
        assert!((state.cam.target - expected).length() < 0.5);
    }

    /// #1288: `bearcad.ui.update_channel` sets/gets the auto-update stream.
    #[test]
    fn lua_update_channel_sets_and_gets() {
        let state = run_lua(
            r#"
            bearcad.new()
            assert(bearcad.ui.update_channel() == "release")
            bearcad.ui.update_channel("pre_release")
            assert(bearcad.ui.update_channel() == "pre_release")
            bearcad.ui.update_channel("release")
            assert(bearcad.ui.update_channel() == "release")
        "#,
        );
        assert_eq!(
            state.update_channel,
            crate::settings::UpdateChannel::Release
        );
    }

    /// #108: an empty document leaves the camera alone.
    #[test]
    fn lua_zoom_fit_on_empty_document_is_a_no_op() {
        let default_cam = crate::camera::Camera::default();
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.ui.zoom_fit()
        "#,
        );
        assert_eq!(state.cam.target, default_cam.target);
        assert_eq!(state.cam.distance, default_cam.distance);
    }
    /// #114: the semantic-gizmo table form of `drag_vertex` nudges a vertex by a
    /// sketch-local delta from wherever it currently is.
    #[test]
    fn lua_drag_vertex_delta_moves_endpoint() {
        let state = run_lua(
            r#"
            bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0 }
            local p = { kind = "line", index = 0, ["end"] = "end" }
            bearcad.drag_vertex{ point = p, du = 5, dv = 3 }
            local l = bearcad.get{ kind = "line", index = 0 }
            assert(math.abs(l.x1 - 15) < 1e-3 and math.abs(l.y1 - 3) < 1e-3,
                   string.format("endpoint at (%g, %g), want (15, 3)", l.x1, l.y1))
        "#,
        );
        assert!((state.doc.lines[lkey(0)].x1 - 15.0).abs() < 1e-3);
        assert!((state.doc.lines[lkey(0)].y1 - 3.0).abs() < 1e-3);
    }

    /// #114: the table form of `drag_line` translates the whole line by a delta.
    #[test]
    fn lua_drag_line_delta_translates_line() {
        let state = run_lua(
            r#"
            bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0 }
            bearcad.drag_line{ line = { kind = "line", index = 0 }, dv = 4 }
            local l = bearcad.get{ kind = "line", index = 0 }
            assert(math.abs(l.y0 - 4) < 1e-3 and math.abs(l.y1 - 4) < 1e-3,
                   string.format("line at y %g..%g, want 4..4", l.y0, l.y1))
        "#,
        );
        assert!((state.doc.lines[lkey(0)].y0 - 4.0).abs() < 1e-3);
        assert!((state.doc.lines[lkey(0)].x1 - 10.0).abs() < 1e-3);
    }

    /// #114: attempting to drag a fully constrained vertex raises a catchable error and
    /// leaves the geometry untouched (a locked `rect` corner is fully constrained).
    #[test]
    fn lua_drag_vertex_fully_constrained_raises() {
        // #459: a dimensioned rect is only rigid once it's also *located* — pin a
        // corner to the origin, then dragging raises. (Unpinned dimensioned shapes
        // translate under drags instead of refusing.)
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.select{ kind = "line", index = 0, ["end"] = "start" }
            bearcad.select({ kind = "origin" }, true)
            bearcad.add_geometric_constraint("coincident")
            bearcad.clear_selection()
            local ok, err = pcall(function()
                bearcad.drag_vertex{
                    point = { kind = "line", index = 0, ["end"] = "end" },
                    du = 3,
                }
            end)
            assert(not ok, "dragging a locked rect corner should raise")
            assert(tostring(err):find("constrained"), "unexpected error: " .. tostring(err))
        "#,
        );
        assert!((state.doc.lines[lkey(0)].x1 - 10.0).abs() < 1e-3, "corner must not move");
    }

    /// #459 regression: a dimensioned-but-unpinned rect still drags — the whole
    /// shape translates, both dimensions intact. (The DOF analysis used to count
    /// the solver's weak gauge-hold pins as real constraints, so every dimensioned
    /// shape froze solid.)
    #[test]
    fn lua_drag_translates_dimensioned_unpinned_rect() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 15, y = 10, width = 40, height = 20 }
            bearcad.drag_vertex{
                point = { kind = "line", index = 0, ["end"] = "end" },
                du = 10, dv = 5,
            }
        "#,
        );
        let l0 = &state.doc.lines[lkey(0)];
        let w = (l0.x1 - l0.x0).abs();
        assert!((w - 40.0).abs() < 1e-2, "width preserved, got {w}");
        assert!(
            (l0.x1 - 55.0).abs() > 1.0 || (l0.y1 - 10.0).abs() > 1.0,
            "the corner actually moved: ({}, {})",
            l0.x1,
            l0.y1
        );
    }

    /// #114: `edit_extrusion` push/pulls an existing extrusion — `by` nudges from the
    /// current effective depth, `distance` sets an absolute one.
    #[test]
    fn lua_edit_extrusion_push_pull_updates_distance() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 8 }
            bearcad.edit_extrusion{ extrusion = 0, by = 2 }
            bearcad.edit_extrusion{ extrusion = 0, by = -4 }
            local ok = pcall(function()
                bearcad.edit_extrusion{ extrusion = 0, distance = 0 }
            end)
            assert(not ok, "zero distance should raise")
        "#,
        );
        assert!((state.doc.extrusions[xkey(0)].distance - 6.0).abs() < 1e-3);
    }

    /// #114: `extrude{ to = { vertex = ... } }` snaps the new extrusion to another
    /// body's surface, and the snap is parametric — resizing the target body moves the
    /// snapped extrusion with it. A plain `edit_extrusion` distance clears the target.
    #[test]
    fn lua_extrude_to_vertex_snaps_and_follows_target() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 8 }
            bearcad.exit_sketch()
            bearcad.begin_sketch("construction_plane", 0)
            bearcad.rect{ x = 20, y = 20, width = 5, height = 5 }
            local cap = {
                kind = "extrude_cap", extrusion = 0,
                profile = "polygon", lines = {0, 1, 2, 3}, top = true,
            }
            bearcad.extrude{
                polygon = {4, 5, 6, 7},
                to = { vertex = { kind = "face", face = cap, index = 0 } },
            }
            bearcad.edit_extrusion{ extrusion = 0, distance = 12 }
        "#,
        );
        let snapped = &state.doc.extrusions[xkey(1)];
        assert!(snapped.target.is_some(), "extrusion 1 should keep its snap target");
        let depth = crate::extrude::effective_distance(&state.doc, snapped);
        assert!(
            (depth - 12.0).abs() < 1e-3,
            "snapped extrusion should follow the resized target, got {depth}"
        );

        // A plain typed distance is a blind extrude again: it drops the snap target.
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 8 }
            bearcad.exit_sketch()
            bearcad.begin_sketch("construction_plane", 0)
            bearcad.rect{ x = 20, y = 20, width = 5, height = 5 }
            local cap = {
                kind = "extrude_cap", extrusion = 0,
                profile = "polygon", lines = {0, 1, 2, 3}, top = true,
            }
            bearcad.extrude{
                polygon = {4, 5, 6, 7},
                to = { vertex = { kind = "face", face = cap, index = 0 } },
            }
            bearcad.edit_extrusion{ extrusion = 1, distance = 3 }
        "#,
        );
        assert!(state.doc.extrusions[xkey(1)].target.is_none());
        assert!((state.doc.extrusions[xkey(1)].distance - 3.0).abs() < 1e-3);
    }

    /// #114: `extrude{ to = { plane = i } }` (no distance needed) reaches exactly the
    /// construction plane's offset.
    #[test]
    fn lua_extrude_to_plane_matches_plane_offset() {
        let state = run_lua(
            r#"
            bearcad.plane{ offset = 5 }
            bearcad.rect{ width = 10, height = 10 }
            -- Plane 3: the added one, after the three datum planes (#833).
            bearcad.extrude{ polygon = {0, 1, 2, 3}, to = { plane = 3 } }
        "#,
        );

        let ext = &state.doc.extrusions[xkey(0)];
        assert_eq!(ext.target, Some(crate::model::ExtrudeTarget::Plane(pkey(3))));
        let depth = crate::extrude::effective_distance(&state.doc, ext);
        assert!((depth - 5.0).abs() < 1e-3, "depth should match the plane offset, got {depth}");
    }

    /// #465: `plane{ origin, normal }` anchors a plane on an arbitrary face, offset
    /// along the normal — the scripted equivalent of clicking a body face.
    #[test]
    fn lua_plane_from_face_origin_and_normal() {
        let state = run_lua(
            r#"
            bearcad.plane{ offset = 5, origin = {0, 0, 10}, normal = {0, 0, 1} }
        "#,
        );
        let plane = state
            .doc
            .construction_planes
            .values()
            .last()
            .expect("plane should be created");
        assert!((plane.origin.z - 15.0).abs() < 1e-3, "origin {:?}", plane.origin);
        assert!((plane.normal.z - 1.0).abs() < 1e-4, "normal {:?}", plane.normal);
    }

    /// #126: `extrude{ to = { face = { kind = "extrude_cap", ... } } }` snaps an extrusion's
    /// depth to another (already-built) extrusion's cap face — not just a construction plane.
    #[test]
    fn lua_extrude_to_body_face_matches_that_faces_height() {
        let state = run_lua(
            r#"
            bearcad.rect{ width = 10, height = 10, name = "Base" }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 8, name = "Tall" }

            bearcad.rect{ width = 10, height = 10, x = 20, name = "Second base" }
            bearcad.extrude{
                polygon = {4, 5, 6, 7},
                to = { face = { kind = "extrude_cap", extrusion = 0, profile = "polygon",
                                profile_lines = {0, 1, 2, 3}, top = true } },
            }
        "#,
        );
        let ext = &state.doc.extrusions[xkey(1)];
        assert!(
            matches!(
                ext.target,
                Some(crate::model::ExtrudeTarget::BodyFace(
                    crate::model::FaceId::ExtrudeCap { extrusion, top: true, .. }
                )) if extrusion == xkey(0)
            ),
            "unexpected target: {:?}",
            ext.target
        );
        let depth = crate::extrude::effective_distance(&state.doc, ext);
        assert!((depth - 8.0).abs() < 1e-3, "should reach the first extrusion's 8mm cap, got {depth}");
    }

    /// #126: a body-face target must actually be a cap/side wall — a `kind` that resolves to
    /// some other `FaceId` (e.g. a plain circle) is rejected rather than silently misused.
    #[test]
    fn lua_extrude_to_body_face_rejects_non_cap_side_face_kinds() {
        let mut runner = ScriptRunner::from_lua_source(
            r#"
            bearcad.circle{ r = 5, name = "Hole" }
            bearcad.rect{ width = 10, height = 10, x = 20, name = "Base" }
            bearcad.extrude{
                polygon = {0, 1, 2, 3},
                to = { face = { kind = "circle", index = 0 } },
            }
        "#,
        )
        .unwrap();
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, None, &ctx);
        }
        let err = runner.error.expect("non-cap/side body face target should error");
        assert!(err.contains("cap or side wall"), "unexpected error: {err}");
    }

    /// SPEC §3.5 Revolve: a square revolved 360° around the global Y axis makes a
    /// ring-shaped body; 90° makes a quarter of it.
    #[test]
    fn lua_revolve_makes_a_ring_body() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 10, y = 0, width = 10, height = 10 }
            bearcad.exit_sketch()
            bearcad.revolve{ polygon = {0,1,2,3}, axis = "y", name = "Ring" }
        "#,
        );
        assert_eq!(state.doc.revolutions.len(), 1);
        let rev = state.doc.revolutions.keys().next().expect("the revolve");
        let bi = state.doc.bodies.keys().last().unwrap();
        assert_eq!(
            state.doc.bodies[bi].source,
            crate::model::BodySource::Revolve(rev)
        );
        assert_eq!(state.doc.bodies[bi].name.as_deref(), Some("Ring"));
        let mesh = crate::extrude::body_solid_mesh(&state.doc, bi).expect("mesh");
        let vol = crate::extrude::mesh_signed_volume(&mesh).abs();
        let expected = std::f32::consts::PI * (400.0 - 100.0) * 10.0;
        assert!(
            (vol - expected).abs() < expected * 0.02,
            "expected ~{expected}, got {vol}"
        );
    }

    /// #1242: `revolutions` and `pitch` wind a helical spring — multi-turn advances
    /// along the axis by pitch × turns.
    #[test]
    fn lua_revolve_pitch_and_revolutions_make_a_spring() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 10, y = 0, width = 5, height = 4 }
            bearcad.exit_sketch()
            bearcad.revolve{
                polygon = {0,1,2,3},
                axis = "y",
                revolutions = 3,
                pitch = 10,
                name = "Spring"
            }
        "#,
        );
        assert_eq!(state.doc.revolutions.len(), 1);
        let rev = state.doc.revolutions.keys().next().expect("the revolve");
        assert!((state.doc.revolutions[rev].angle_deg - 1080.0).abs() < 1e-3);
        assert!((state.doc.revolutions[rev].pitch_mm - 10.0).abs() < 1e-3);
        let bi = state.doc.bodies.keys().last().unwrap();
        let mesh = crate::extrude::body_solid_mesh(&state.doc, bi).expect("spring mesh");
        let (min, max) = mesh.bounds().expect("bounds");
        let span = max.y - min.y;
        // profile height 4 + 3 turns × pitch 10 = 34
        assert!(
            (span - 34.0).abs() < 3.0,
            "spring axial span ~34, got {span}"
        );
    }

    /// Combine tool scripting: `bearcad.combine{}` cuts one body out of another, shadows
    /// #130: a bare body face is push/pulled declaratively with `bearcad.extrude_face{}`,
    /// no simulated viewport click — the scripting path the user asked for.
    #[test]
    fn lua_extrude_face_pushes_a_body_side_wall() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 0, y = 0, width = 20, height = 20 }
            bearcad.exit_sketch()
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
            bearcad.extrude_face{
                face = { kind = "extrude_side", extrusion = 0, profile = "polygon", profile_lines = {0, 1, 2, 3}, edge = 0 },
                distance = 10,
                name = "Boss"
            }
        "#,
        );
        assert_eq!(state.doc.extrusions.len(), 2, "a second extrusion grew from the body face");
        assert_eq!(state.doc.extrusions[xkey(1)].name.as_deref(), Some("Boss"));
    }

    /// #130: `extrude_face{ to = { face = ... } }` snaps a pushed face onto another face —
    /// "simulate extruding and choose a face to snap to."
    #[test]
    fn lua_extrude_face_snaps_to_a_target_face() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 0, y = 0, width = 20, height = 20 }
            bearcad.exit_sketch()
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
            bearcad.extrude_face{
                face = { kind = "extrude_side", extrusion = 0, profile = "polygon", profile_lines = {0, 1, 2, 3}, edge = 0 },
                to = { plane = 0 }
            }
        "#,
        );
        assert_eq!(state.doc.extrusions.len(), 2);
        assert!(state.doc.extrusions[xkey(1)].target.is_some(), "the extrusion snapped to a target");
    }

    /// the inputs (except kept B), and names the operation.
    #[test]
    fn lua_combine_cut_creates_op_and_shadows() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
            bearcad.exit_sketch()
            bearcad.extrude{ polygon = {0,1,2,3}, distance = 5 }
            bearcad.begin_sketch{ kind = "plane", index = 0 }
            bearcad.rect{ x = 5, y = 0, width = 10, height = 10 }
            bearcad.exit_sketch()
            bearcad.extrude{ polygon = {4,5,6,7}, distance = 5 }
            bearcad.combine{ op = "cut", a = {0}, b = {1}, name = "Slot" }
        "#,
        );
        assert_eq!(state.doc.boolean_ops.len(), 1);
        let op = &state.doc.boolean_ops.values().nth(0).unwrap();
        assert_eq!(op.kind, crate::model::BooleanOpKind::Cut);
        assert_eq!(op.name.as_deref(), Some("Slot"));
        assert!(state.doc.bodies.values().nth(0).unwrap().shadow);
        assert!(state.doc.bodies.values().nth(1).unwrap().shadow);
        assert!(!op.outputs.is_empty());
    }

    /// Slice tool scripting: `bearcad.slice{}` cuts a box with an offset plane into two
    /// fragments and shadows the input.
    #[test]
    fn lua_slice_halves_a_box() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
            bearcad.exit_sketch()
            bearcad.extrude{ polygon = {0,1,2,3}, distance = 5 }
            bearcad.plane{ offset = 2.5 }
            bearcad.slice{ bodies = {0}, cutters = {{ kind = "construction_plane", index = 3 }}, name = "Halved" }
        "#,
        );
        assert_eq!(state.doc.slice_ops.len(), 1);
        let op = &state.doc.slice_ops.values().nth(0).unwrap();
        assert_eq!(op.name.as_deref(), Some("Halved"));
        assert_eq!(op.outputs.len(), 2, "a mid-plane cut yields two fragments");
        assert!(state.doc.bodies.values().nth(0).unwrap().shadow, "the sliced input becomes a shadow body");
    }

    /// #1351: `bearcad.project{ body = 0 }` projects a body's edges into the open sketch.
    #[test]
    fn lua_project_body_into_a_new_sketch() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            bearcad.begin_sketch{ kind = "construction_plane", index = 0 }
            bearcad.project{ body = 0 }
            "#,
        );
        let projected = state
            .doc
            .lines
            .values()
            .filter(|l| l.projection.is_some())
            .count();
        assert!(
            projected > 0,
            "projecting a body should create projected lines, status={}",
            state.status
        );
        assert!(
            state
                .doc
                .lines
                .values()
                .filter(|l| l.projection.is_some())
                .all(|l| l.construction),
            "projected lines are construction-style"
        );
    }

    /// #1351: `bearcad.project{ plane = 2 }` projects a construction plane as one reference line.
    #[test]
    fn lua_project_plane_into_sketch() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.begin_sketch{ kind = "construction_plane", index = 0 }
            bearcad.project{ plane = 2 }
            "#,
        );
        let projected: Vec<_> = state
            .doc
            .lines
            .values()
            .filter(|l| l.projection.is_some())
            .collect();
        assert_eq!(
            projected.len(),
            1,
            "YZ into ground is one line, status={}",
            state.status
        );
        assert!(projected[0].construction);
        assert!(matches!(
            projected[0].projection,
            Some(crate::model::ProjectionSource::Plane { .. })
        ));
        // YZ (normal X) meets the ground sketch along world Y: local u stays 0.
        assert!(
            projected[0].x0.abs() < 1e-3 && projected[0].x1.abs() < 1e-3,
            "{:?}",
            projected[0]
        );
    }

    /// #1351: `bearcad.project{ entities = { ... } }` accepts the same tables `select` takes.
    #[test]
    fn lua_project_entities_table() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.begin_sketch{ kind = "construction_plane", index = 0 }
            bearcad.project{ entities = { { kind = "construction_plane", index = 2 } } }
            "#,
        );
        assert_eq!(
            state.doc.lines.values().filter(|l| l.projection.is_some()).count(),
            1,
            "{}",
            state.status
        );
    }

    /// #1351: no-arg `bearcad.project()` projects the current selection.
    #[test]
    fn lua_project_uses_current_selection() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.begin_sketch{ kind = "construction_plane", index = 0 }
            bearcad.select{ kind = "construction_plane", index = 2 }
            bearcad.project()
            "#,
        );
        assert_eq!(
            state.doc.lines.values().filter(|l| l.projection.is_some()).count(),
            1,
            "{}",
            state.status
        );
    }

    /// #1351: project with only projected lines selected un-projects them.
    #[test]
    fn lua_project_unprojects_selected_projected_lines() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.begin_sketch{ kind = "construction_plane", index = 0 }
            bearcad.project{ plane = 2 }
            bearcad.select{ kind = "line", index = 0 }
            bearcad.project()
            "#,
        );
        assert_eq!(
            state.doc.lines.values().filter(|l| l.projection.is_some()).count(),
            0,
            "un-project should remove the reference, status={}",
            state.status
        );
    }

    /// #1351: project without an open sketch fails.
    #[test]
    fn lua_project_errors_without_a_sketch() {
        run_lua_expect_ok(
            r#"
            bearcad.new()
            local ok, err = pcall(function() bearcad.project{ plane = 2 } end)
            assert(not ok, "project outside a sketch must fail")
            assert(tostring(err):lower():find("sketch"), tostring(err))
            "#,
        );
    }

    /// #1351: project with nothing selected / nothing projectable fails.
    #[test]
    fn lua_project_errors_on_empty_selection() {
        run_lua_expect_ok(
            r#"
            bearcad.new()
            bearcad.begin_sketch{ kind = "construction_plane", index = 0 }
            local ok, err = pcall(function() bearcad.project() end)
            assert(not ok, "project with empty selection must fail")
            assert(tostring(err):lower():find("select") or tostring(err):lower():find("project"),
                   tostring(err))
            "#,
        );
    }

    /// #1351: unknown option keys are rejected like sibling tools.
    #[test]
    fn lua_project_rejects_unknown_keys() {
        run_lua_expect_ok(
            r#"
            bearcad.new()
            bearcad.begin_sketch{ kind = "construction_plane", index = 0 }
            local ok, err = pcall(function() bearcad.project{ widget = 1 } end)
            assert(not ok, "unknown key must fail")
            assert(tostring(err):find("widget"), tostring(err))
            assert(tostring(err):find("entities"), tostring(err))
            "#,
        );
    }

    /// #1126: a sketch line on a body face is a laser-style path cutter.
    #[test]
    fn lua_slice_with_a_line_cutter_halves_a_box() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
            bearcad.exit_sketch()
            bearcad.extrude{ polygon = {0,1,2,3}, distance = 5 }
            bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 0,
                                  profile = "polygon", profile_lines = {0,1,2,3}, top = true }
            bearcad.line{ x = 0, y = 5, x1 = 10, y1 = 5 }
            bearcad.exit_sketch()
            bearcad.slice{ bodies = {0}, cutters = {{ kind = "line", index = 4 }}, name = "Laser" }
        "#,
        );
        assert_eq!(state.doc.slice_ops.len(), 1, "{}", state.status);
        let op = &state.doc.slice_ops.values().nth(0).unwrap();
        assert_eq!(op.name.as_deref(), Some("Laser"));
        assert_eq!(op.outputs.len(), 2, "a midline laser cut yields two fragments");
        assert!(matches!(
            op.cutters.as_slice(),
            [crate::model::SliceCutter::Line { .. }]
        ));
        assert!(state.doc.bodies.values().nth(0).unwrap().shadow);
    }

    /// #1142: a zigzag of connected sketch lines is one laser path → two fragments.
    #[test]
    fn lua_slice_with_a_zigzag_path_halves_a_box() {
        let state = run_lua(
            r#"
            bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
            bearcad.exit_sketch()
            bearcad.extrude{ polygon = {0,1,2,3}, distance = 5 }
            bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 0,
                                  profile = "polygon", profile_lines = {0,1,2,3}, top = true }
            bearcad.line{ x = 3, y = 0, x1 = 7, y1 = 3.5 }
            bearcad.line{ x = 7, y = 3.5, x1 = 3, y1 = 6.5 }
            bearcad.line{ x = 3, y = 6.5, x1 = 7, y1 = 10 }
            bearcad.exit_sketch()
            bearcad.slice{ bodies = {0},
                           cutters = {
                             { kind = "line", index = 4 },
                             { kind = "line", index = 5 },
                             { kind = "line", index = 6 },
                           },
                           name = "Zigzag" }
        "#,
        );
        assert_eq!(state.doc.slice_ops.len(), 1, "{}", state.status);
        let op = &state.doc.slice_ops.values().nth(0).unwrap();
        assert_eq!(op.outputs.len(), 2, "zigzag laser path yields two fragments");
        assert_eq!(op.cutters.len(), 3);
    }

    /// SPEC §3.5 Loft: `bearcad.loft{ circles = {...} }` blends circle sections on two
    /// planes into a new loft-sourced body with a solid mesh.
    #[test]
    fn lua_loft_creates_body_from_two_circle_sections() {
        let state = run_lua(
            r#"
            bearcad.circle{ r = 5 }
            bearcad.plane{ offset = 10 }
            bearcad.begin_sketch{ kind = "plane", index = 1 }
            bearcad.circle{ r = 2 }
            bearcad.exit_sketch()
            bearcad.loft{ circles = {0, 1}, name = "Horn" }
        "#,
        );
        assert_eq!(state.doc.lofts.len(), 1);
        let loft_key = state.doc.lofts.keys().next().expect("one loft");
        assert_eq!(state.doc.lofts[loft_key].sections.len(), 2);
        let bi = state.doc.bodies.keys().last().unwrap();
        assert_eq!(
            state.doc.bodies[bi].source,
            crate::model::BodySource::Loft(loft_key)
        );
        assert_eq!(state.doc.bodies[bi].name.as_deref(), Some("Horn"));
        let mesh = crate::extrude::body_solid_mesh(&state.doc, bi).expect("loft mesh");
        assert!(!mesh.triangles.is_empty());
    }

    /// Lofting fewer than two sections is a scripting error, not a silent no-op.
    #[test]
    fn lua_loft_rejects_single_section() {
        run_lua_expect_ok(
            r#"
            bearcad.circle{ r = 5 }
            local ok, err = pcall(bearcad.loft, { circle = 0 })
            assert(not ok)
            assert(tostring(err):find("two sections"), tostring(err))
        "#,
        );
    }

    /// #180: `bearcad.drawing{}` creates a technical drawing (opening its pane) and
    /// `bearcad.drawing_view{}` adds body views in orientations. The drawing shows up in the
    /// Elements pane as a `Drawing` node with its name.
    #[test]
    fn lua_drawing_creates_a_drawing_with_views() {
        use crate::hierarchy::HierarchyNode;
        use crate::model::DrawingOrientation;
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 20, height = 20 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
            local d = bearcad.drawing{ name = "Plan" }
            bearcad.drawing_view{ drawing = d, body = 0, orientation = "top" }
            bearcad.drawing_view{ drawing = d, body = 0, orientation = "iso" }
            assert(bearcad.count("drawing") == 1, "one drawing")
        "#,
        );
        assert_eq!(state.doc.drawings.len(), 1);
        assert_eq!(state.doc.drawings[dkey(0)].name.as_deref(), Some("Plan"));
        assert_eq!(state.doc.drawings[dkey(0)].views.len(), 2);
        assert_eq!(state.doc.drawings[dkey(0)].views[0].orientation, DrawingOrientation::Top);
        assert_eq!(
            state.doc.drawings[dkey(0)].views[1].orientation,
            DrawingOrientation::Isometric
        );
        // Creating a drawing opens it in the drawing pane.
        assert_eq!(state.editing_drawing, Some(dkey(0)));
        // It appears in the Elements pane, labelled by its name.
        let list = crate::hierarchy::build_element_list(&state.doc, None);
        assert!(list.iter().any(|n| matches!(n, HierarchyNode::Drawing(_))));
        assert!(
            crate::names::node_label(&state.doc, HierarchyNode::Drawing(dkey(0))).starts_with("Plan")
        );
    }

    /// #180: adding a view of a body that doesn't exist errors instead of storing a dangling
    /// reference.
    #[test]
    fn lua_drawing_view_rejects_a_missing_body() {
        let state = run_lua(
            r#"
            bearcad.new()
            local d = bearcad.drawing{}
            local ok, err = pcall(bearcad.drawing_view, { drawing = d, body = 5, orientation = "front" })
            assert(not ok, "a view of a nonexistent body should error")
            assert(tostring(err):find("body"), "unexpected error: " .. tostring(err))
        "#,
        );
        assert_eq!(state.doc.drawings[dkey(0)].views.len(), 0);
    }

    /// #1191: `bodies = {…}` puts several bodies in one projection; `drawing_view_add` appends
    /// more (the scripted form of shift-click).
    #[test]
    fn lua_drawing_view_accepts_multiple_bodies_and_append() {
        use crate::model::body_key_for_slot as bkey;
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 20, height = 20 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
            bearcad.rect{ x = 30, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
            bearcad.rect{ x = 50, y = 0, width = 8, height = 8 }
            bearcad.extrude{ polygon = {8, 9, 10, 11}, distance = 3 }
            local d = bearcad.drawing{}
            bearcad.drawing_view{ drawing = d, bodies = {0, 1}, orientation = "front" }
            assert(bearcad.count("drawing") == 1)
            bearcad.drawing_view_add{ drawing = d, view = 0, body = 2 }
        "#,
        );
        assert_eq!(state.doc.drawings[dkey(0)].views.len(), 1, "one shared projection");
        assert_eq!(
            state.doc.drawings[dkey(0)].views[0].bodies,
            vec![bkey(0), bkey(1), bkey(2)],
            "all three bodies land in the same view"
        );
        // Multi-body edges are the union of each body's creases.
        let edges = crate::drawing::drawing_view_world_edges(
            &state.doc,
            &state.doc.drawings[dkey(0)].views[0],
        );
        assert!(
            edges.len() > 12,
            "two boxes contribute more creases than one alone, got {}",
            edges.len()
        );
    }

    /// #1190: `component = i` expands a component into one multi-body projection.
    #[test]
    fn lua_drawing_view_accepts_a_whole_component() {
        use crate::model::body_key_for_slot as bkey;
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 20, height = 20 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
            bearcad.rect{ x = 30, y = 0, width = 10, height = 10 }
            bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
            local c = bearcad.component{ name = "Frame" }
            bearcad.move_to_component{ kind = "body", index = 0, component = c }
            bearcad.move_to_component{ kind = "body", index = 1, component = c }
            local d = bearcad.drawing{ name = "Assembly" }
            bearcad.drawing_view{ drawing = d, component = c, orientation = "top" }
        "#,
        );
        assert_eq!(state.doc.drawings[dkey(0)].views.len(), 1);
        let view = &state.doc.drawings[dkey(0)].views[0];
        let mut bodies = view.bodies.clone();
        bodies.sort_unstable();
        assert_eq!(bodies, vec![bkey(0), bkey(1)], "component expands to both bodies");
        // Caption prefers the component name.
        let label = crate::drawing::drawing_view_source_label(&state.doc, view);
        assert!(
            label.contains("Frame"),
            "multi-body component view labels with the component name, got {label:?}"
        );
    }

    /// #180: `bearcad.drawing_dimension{}` toggles a view edge's length dimension, keyed by the
    /// edge's world endpoints; calling it again on the same edge hides it.
    #[test]
    fn lua_drawing_dimension_toggles_an_edge() {
        // Views start with no dimensions shown (#331), so the first toggle *shows* this edge and
        // a second toggle hides it again.
        let base_script = r#"
            bearcad.new()
            bearcad.rect{ x = 0, y = 0, width = 40, height = 25 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
            local d = bearcad.drawing{}
            bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
        "#;
        let script = |repeats: usize, a: glam::Vec3, b: glam::Vec3| {
            let toggle = format!(
                "bearcad.drawing_dimension{{ drawing = d, view = 0, a = {{{},{},{}}}, b = {{{},{},{}}} }}\n",
                a.x, a.y, a.z, b.x, b.y, b.z
            );
            format!("{base_script}\n{}", toggle.repeat(repeats))
        };
        let baseline = run_lua(base_script);
        assert!(
            baseline.doc.drawings[dkey(0)].views[0].dimensioned_edges.is_empty(),
            "a new projection starts with no dimensions shown (#331)"
        );
        // A bottom edge of the front view; toggling it adds then removes its dimension.
        let a = glam::Vec3::new(0.0, 0.0, 0.0);
        let b = glam::Vec3::new(40.0, 0.0, 0.0);
        let expected = crate::model::normalized_edge_key(
            crate::hierarchy::quantize_body_point(a),
            crate::hierarchy::quantize_body_point(b),
        );

        let shown = run_lua(&script(1, a, b));
        assert_eq!(
            shown.doc.drawings[dkey(0)].views[0].dimensioned_edges.len(),
            1,
            "one toggle shows the dimension"
        );
        assert!(shown.doc.drawings[dkey(0)].views[0].dimensioned_edges.contains(&expected));

        let hidden = run_lua(&script(2, a, b));
        assert!(
            hidden.doc.drawings[dkey(0)].views[0].dimensioned_edges.is_empty(),
            "toggling the same edge twice hides it again"
        );
    }

    /// #294/#1228: `bearcad.drawing_dim_offset{}` sets a drawing edge dim label's offset.
    #[test]
    fn lua_drawing_dim_offset_sets_and_clears() {
        let script = r#"
            bearcad.new()
            bearcad.rect{ x = 0, y = 0, width = 40, height = 25 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
            local d = bearcad.drawing{}
            bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
            bearcad.drawing_dimension{ drawing = d, view = 0, a = {0,0,0}, b = {40,0,0} }
            bearcad.drawing_dim_offset{ drawing = d, view = 0, a = {0,0,0}, b = {40,0,0}, offset = 6.5 }
        "#;
        let state = run_lua(script);
        let a = crate::hierarchy::quantize_body_point(glam::Vec3::new(0.0, 0.0, 0.0));
        let b = crate::hierarchy::quantize_body_point(glam::Vec3::new(40.0, 0.0, 0.0));
        let key = crate::model::normalized_edge_key(a, b);
        assert_eq!(
            state.doc.drawings[dkey(0)].views[0].dimension_offsets,
            vec![(key, 6.5)]
        );
        let cleared = run_lua(
            &(script.to_string()
                + "\nbearcad.drawing_dim_offset{ drawing = d, view = 0, a = {0,0,0}, b = {40,0,0} }\n"),
        );
        assert!(
            cleared.doc.drawings[dkey(0)].views[0]
                .dimension_offsets
                .is_empty(),
            "omitting offset clears the override"
        );
    }

    /// #373: `bearcad.drawing_circle_dimension{}` toggles a detected circle's diameter
    /// dimension, keyed by the circle's world centre; a second toggle hides it again.
    #[test]
    fn lua_drawing_circle_dimension_toggles_a_circle() {
        let base_script = r#"
            bearcad.new()
            bearcad.circle{ x = 10, y = 5, r = 8 }
            bearcad.exit_sketch()
            bearcad.extrude{ circle = 0, distance = 20 }
            local d = bearcad.drawing{}
            bearcad.drawing_view{ drawing = d, body = 0, orientation = "front-right" }
        "#;
        // The cylinder's base rim circle is centred at the sketch origin offset (10, 5, 0).
        let toggle = "bearcad.drawing_circle_dimension{ drawing = d, view = 0, center = {10, 5, 0} }\n";
        let baseline = run_lua(base_script);
        assert!(baseline.doc.drawings[dkey(0)].views[0].dimensioned_circles.is_empty());

        let shown = run_lua(&format!("{base_script}\n{toggle}"));
        assert_eq!(
            shown.doc.drawings[dkey(0)].views[0].dimensioned_circles,
            vec![crate::hierarchy::quantize_body_point(glam::Vec3::new(10.0, 5.0, 0.0))],
            "one toggle shows the circle's diameter dimension"
        );

        let hidden = run_lua(&format!("{base_script}\n{toggle}{toggle}"));
        assert!(
            hidden.doc.drawings[dkey(0)].views[0].dimensioned_circles.is_empty(),
            "toggling the same circle twice hides it again"
        );
    }

    /// #1207: `bearcad.drawing_view_size{}` resizes a projection card; aligned partners
    /// pick up the shared axis.
    #[test]
    fn lua_drawing_view_size_resizes_and_propagates() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 40, height = 20 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
            local d = bearcad.drawing{}
            bearcad.drawing_view{ drawing = d, body = 0, orientation = "top" }
            bearcad.drawing_align_view{ drawing = d, parent = 0, dir = "right", pos = 0.75 }
            bearcad.drawing_align_view{ drawing = d, parent = 0, dir = "below", pos = 0.75 }
            bearcad.drawing_view_size{ drawing = d, view = 0, width = 0.3, height = 0.5 }
            "#,
        );
        let views = &state.doc.drawings[dkey(0)].views;
        assert!((views[0].size_x - 0.3).abs() < 1e-4 && (views[0].size_y - 0.5).abs() < 1e-4);
        assert!(
            (views[1].size_x - crate::drawing::CELL_FRAC).abs() < 1e-4
                && (views[1].size_y - 0.5).abs() < 1e-4,
            "Right child shares height"
        );
        assert!(
            (views[2].size_x - 0.3).abs() < 1e-4
                && (views[2].size_y - crate::drawing::CELL_FRAC).abs() < 1e-4,
            "Below child shares width"
        );
    }

    /// #377: `bearcad.drawing_view_align_lines{}` toggles an aligned child's dashed
    /// projection lines; a non-aligned view rejects the toggle.
    #[test]
    fn lua_drawing_view_align_lines_toggles_on_aligned_children() {
        let base = r#"
            bearcad.new()
            bearcad.rect{ x = 0, y = 0, width = 40, height = 25 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
            local d = bearcad.drawing{}
            bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
            bearcad.drawing_align_view{ drawing = d, parent = 0, dir = "below", pos = 0.7 }
        "#;
        let on = run_lua(&format!(
            "{base}\nbearcad.drawing_view_align_lines{{ drawing = d, view = 1, show = true }}"
        ));
        assert!(on.doc.drawings[dkey(0)].views[1].align_lines);

        let off = run_lua(&format!(
            "{base}\nbearcad.drawing_view_align_lines{{ drawing = d, view = 1, show = true }}\n\
             bearcad.drawing_view_align_lines{{ drawing = d, view = 1, show = false }}"
        ));
        assert!(!off.doc.drawings[dkey(0)].views[1].align_lines);

        // The base view isn't aligned, so the toggle is rejected (raising a Lua error) and
        // the flag stays off.
        let rejected = run_lua(&format!(
            "{base}\nlocal ok = pcall(function()\n\
             bearcad.drawing_view_align_lines{{ drawing = d, view = 0, show = true }}\n\
             end)\nassert(not ok, \"toggling a non-aligned view must fail\")"
        ));
        assert!(!rejected.doc.drawings[dkey(0)].views[0].align_lines);
    }

    /// #372: `bearcad.drawing_view_label{}` edits a view's caption — visibility, position
    /// (grid name), and custom text; an empty text returns to the automatic caption.
    #[test]
    fn lua_drawing_view_label_edits_the_caption() {
        let base = r#"
            bearcad.new()
            bearcad.rect{ x = 0, y = 0, width = 40, height = 25 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
            local d = bearcad.drawing{}
            bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
        "#;
        let state = run_lua(&format!(
            "{base}\nbearcad.drawing_view_label{{ drawing = d, view = 0, hidden = true, \
             pos = \"bottom-center\", text = \"Plate {{w}}\" }}"
        ));
        let view = &state.doc.drawings[dkey(0)].views[0];
        assert!(view.label_hidden);
        assert_eq!(view.label_pos, crate::model::DrawingLabelPos::BottomCenter);
        assert_eq!(view.label_text.as_deref(), Some("Plate {w}"));

        let reset = run_lua(&format!(
            "{base}\nbearcad.drawing_view_label{{ drawing = d, view = 0, text = \"custom\" }}\n\
             bearcad.drawing_view_label{{ drawing = d, view = 0, text = \"\" }}"
        ));
        let view = &reset.doc.drawings[dkey(0)].views[0];
        assert_eq!(view.label_text, None, "empty text returns to the automatic caption");
        assert!(!view.label_hidden, "untouched aspects keep their values");
    }

    /// #334: a smooth extrusion (cylinder) has no crease edge down its side, so its **length**
    /// is only dimensionable via the view-dependent silhouette edges. `drawing_view_dimensionable_edges`
    /// adds them, so a side view exposes more edges than the crease-only set.
    #[test]
    fn cylinder_length_is_dimensionable_via_silhouette() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.circle{ x = 0, y = 0, r = 10 }
            bearcad.extrude{ circle = 0, distance = 30 }
            local d = bearcad.drawing{}
            bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
        "#,
        );
        let views = &state.doc.drawings[dkey(0)].views;
        let view = &views[0];
        let creases = crate::drawing::drawing_view_world_edges(&state.doc, view);
        let dimensionable = crate::drawing::drawing_view_dimensionable_edges(&state.doc, views, view);
        assert!(
            dimensionable.len() > creases.len(),
            "silhouette side edges join the dimensionable set (#334): creases={}, dimensionable={}",
            creases.len(),
            dimensionable.len()
        );
        // At least one added edge spans the 30mm extrusion length in projected space.
        let (right, up) = crate::drawing::view_axes(view.orientation);
        let has_length = dimensionable.iter().any(|(a, b)| {
            let pa = glam::Vec2::new(a.dot(right), a.dot(up));
            let pb = glam::Vec2::new(b.dot(right), b.dot(up));
            ((pb - pa).length() - 30.0).abs() < 0.5
        });
        assert!(has_length, "a side edge measures the 30mm length");
    }

    /// #342: Show all / Hide all also control a circle's diameter dimension (it's no longer
    /// always drawn), so Hide all clears `dimensioned_circles` and Show all repopulates it.
    #[test]
    fn show_and_hide_all_dimensions_controls_circle_diameters() {
        let mut state = run_lua(
            r#"
            bearcad.new()
            bearcad.circle{ x = 0, y = 0, r = 10 }
            bearcad.extrude{ circle = 0, distance = 30 }
            local d = bearcad.drawing{}
            bearcad.drawing_view{ drawing = d, body = 0, orientation = "top" }
        "#,
        );
        // A new view starts with no circle diameters shown (#331/#342).
        assert!(state.doc.drawings[dkey(0)].views[0].dimensioned_circles.is_empty());
        state.apply(crate::actions::Action::SetAllDrawingDimensions {
            drawing: dkey(0),
            view: 0,
            show: true,
        });
        assert!(
            !state.doc.drawings[dkey(0)].views[0].dimensioned_circles.is_empty(),
            "Show all reveals the circle's diameter dimension"
        );
        state.apply(crate::actions::Action::SetAllDrawingDimensions {
            drawing: dkey(0),
            view: 0,
            show: false,
        });
        assert!(
            state.doc.drawings[dkey(0)].views[0].dimensioned_circles.is_empty(),
            "Hide all clears the circle's diameter dimension (#342)"
        );
    }

    /// #408: a text's anchor point constrains coincident to a sketch point through the normal
    /// constraint tool flow — the text translates so the anchor sits on the point.
    #[test]
    fn lua_text_anchor_coincident_moves_the_text() {
        let family = ["Helvetica", "Arial", "DejaVu Sans", "Liberation Sans"]
            .into_iter()
            .find(|f| crate::text::font_bytes(f, false, false).is_some());
        if family.is_none() {
            eprintln!("no usable system font; skipping");
            return;
        }
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 30, y = 40, x1 = 60, y1 = 40 }
            bearcad.text{ text = "Hi", x = 0, y = 0, size = 10 }
            bearcad.select{ kind = "sketch_text", index = 0, anchor = "center" }
            bearcad.select({ kind = "line", index = 0, ["end"] = "start" }, true)
            bearcad.add_geometric_constraint("coincident")
        "#,
        );
        let t = &state.doc.sketch_texts[tkey(0)];
        let (cx, cy) = crate::text::sketch_text_anchor_uv(t, crate::model::TextAnchor::Center);
        assert!((cx - 30.0).abs() < 1e-2 && (cy - 40.0).abs() < 1e-2, "centre at ({cx}, {cy})");
        // The line stayed put — the text is the mover.
        assert_eq!(state.doc.lines[lkey(0)].x0, 30.0);
        assert_eq!(state.doc.lines[lkey(0)].y0, 40.0);
    }

    /// #355: `bearcad.extrude{ text = i }` extrudes a whole sketch text (all its glyphs), so a
    /// label can be engraved from a script.
    #[test]
    fn lua_extrude_text_engraves_all_glyphs() {
        let family = ["Helvetica", "Arial", "DejaVu Sans", "Liberation Sans"]
            .into_iter()
            .find(|f| crate::text::font_bytes(f, false, false).is_some());
        if family.is_none() {
            eprintln!("no usable system font; skipping");
            return;
        }
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.text{ text = "Bear", x = 0, y = 0, size = 10 }
            bearcad.exit_sketch()
            bearcad.extrude{ text = 0, distance = 2, name = "Label" }
        "#,
        );
        assert_eq!(
            state.doc.extrusions.values().count(),
            1,
            "the text extrudes into one extrusion"
        );
        // Its faces are the text's glyph regions.
        let ex = state.doc.extrusions.values().next().unwrap();
        assert!(
            ex.faces
                .iter()
                .all(|f| matches!(f, crate::model::ExtrudeFace::TextGlyph { .. })),
            "extruded faces are the text's glyphs"
        );
        assert!(!ex.faces.is_empty(), "a 4-letter word has glyph faces");

        // #386: the live drag preview of a text extrusion routes through the fast
        // tessellated mesher (cached, kernel-free) and still produces geometry — keeping the
        // gizmo drag responsive (the kernel per-glyph boolean chain ran every frame before).
        let preview =
            crate::extrude::preview_extrusion_mesh(&state.doc, ex).expect("text previews a mesh");
        assert!(!preview.triangles.is_empty());
        // Cached second call is effectively free; assert it stays far from the
        // seconds-per-frame territory the kernel path hit (250ms is generous for CI).
        let t = crate::time::Instant::now();
        let _ = crate::extrude::preview_extrusion_mesh(&state.doc, ex);
        assert!(
            t.elapsed() < std::time::Duration::from_millis(250),
            "cached text preview must be fast, took {:?}",
            t.elapsed()
        );
    }

    /// #331: "Show all dimensions" populates the deduped, staggered default set and "Hide all"
    /// clears it, both via `Action::SetAllDrawingDimensions`.
    #[test]
    fn show_and_hide_all_dimensions() {
        let script = r#"
            bearcad.new()
            bearcad.rect{ x = 0, y = 0, width = 40, height = 25 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
            local d = bearcad.drawing{}
            bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
        "#;
        let mut state = run_lua(script);
        assert!(state.doc.drawings[dkey(0)].views[0].dimensioned_edges.is_empty());
        assert_eq!(
            state.apply(crate::actions::Action::SetAllDrawingDimensions {
                drawing: dkey(0),
                view: 0,
                show: true,
            }),
            crate::actions::ActionResult::Ok
        );
        assert!(
            !state.doc.drawings[dkey(0)].views[0].dimensioned_edges.is_empty(),
            "Show all populates the default dimension set"
        );
        state.apply(crate::actions::Action::SetAllDrawingDimensions {
            drawing: dkey(0),
            view: 0,
            show: false,
        });
        assert!(
            state.doc.drawings[dkey(0)].views[0].dimensioned_edges.is_empty(),
            "Hide all clears the dimension set"
        );
    }

    /// #180: `bearcad.drawing_angle{}` toggles the angle dimension between two edges of a view,
    /// keyed by the edges' endpoints; a second call on the same pair hides it.
    #[test]
    fn lua_drawing_angle_toggles_between_two_edges() {
        let script = |repeats: usize| {
            let toggles = "bearcad.drawing_angle{ drawing = d, view = 0, edge1 = { a = {0,0,0}, b = {40,0,0} }, edge2 = { a = {0,0,0}, b = {0,0,15} } }\n".repeat(repeats);
            format!(
                r#"
                bearcad.new()
                bearcad.rect{{ x = 0, y = 0, width = 40, height = 25 }}
                bearcad.extrude{{ polygon = {{0, 1, 2, 3}}, distance = 15 }}
                local d = bearcad.drawing{{}}
                bearcad.drawing_view{{ drawing = d, body = 0, orientation = "front" }}
                {toggles}
            "#
            )
        };
        let shown = run_lua(&script(1));
        assert_eq!(shown.doc.drawings[dkey(0)].views[0].angle_dims.len(), 1);
        let hidden = run_lua(&script(2));
        assert_eq!(hidden.doc.drawings[dkey(0)].views[0].angle_dims.len(), 0);
    }

    /// #180: an angle needs two *different* edges.
    #[test]
    fn lua_drawing_angle_rejects_a_single_edge() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ x = 0, y = 0, width = 40, height = 25 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
            local d = bearcad.drawing{}
            bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
            local ok = pcall(bearcad.drawing_angle, { drawing = d, view = 0,
                edge1 = { a = {0,0,0}, b = {40,0,0} }, edge2 = { a = {0,0,0}, b = {40,0,0} } })
            assert(not ok, "same edge twice should error")
        "#,
        );
        assert_eq!(state.doc.drawings[dkey(0)].views[0].angle_dims.len(), 0);
    }

    /// #180: a drawing exports to a self-contained SVG with its title, view captions,
    /// projected edge lines, and shown dimensions.
    #[test]
    fn drawing_svg_export_has_lines_and_dimensions() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ x = 0, y = 0, width = 40, height = 25 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
            local d = bearcad.drawing{ name = "Plate" }
            bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
            bearcad.drawing_dimension{ drawing = d, view = 0, a = {0,0,0}, b = {40,0,0} }
        "#,
        );
        let svg = crate::drawing::drawing_to_svg(&state.doc, dkey(0)).expect("svg");
        assert!(svg.starts_with("<svg"), "is an svg document");
        assert!(svg.contains("<line"), "has projected edge lines");
        assert!(svg.contains("Plate"), "has the drawing title");
        assert!(svg.contains("Front"), "has the view caption");
        assert!(svg.contains("40"), "has the 40 mm length dimension");
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    /// #1350: exported dimension labels sit beside their dimension lines — the same
    /// visual-centre placement the editor uses — so a horizontal label never sits on
    /// its dimension stroke the way a baseline-aligned PDF used to.
    #[test]
    fn drawing_export_dimension_labels_do_not_overlap_their_lines() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ x = 0, y = 0, width = 80, height = 20 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
            local d = bearcad.drawing{}
            bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
            bearcad.drawing_dimension{ drawing = d, view = 0, a = {0,0,0}, b = {80,0,0} }
            bearcad.drawing_dimension{ drawing = d, view = 0, a = {0,0,0}, b = {0,0,15} }
        "#,
        );
        let svg = crate::drawing::drawing_to_svg(&state.doc, dkey(0)).expect("svg");
        let pdf = crate::drawing::drawing_to_pdf(&state.doc, dkey(0)).expect("pdf");
        let pdf_text = String::from_utf8_lossy(&pdf);

        let dim_labels: Vec<_> = svg
            .lines()
            .filter(|l| l.contains("dominant-baseline=\"central\"") && l.contains(" mm"))
            .collect();
        assert!(
            dim_labels.len() >= 2,
            "both length labels must be vertically centred in the SVG, got:\n{svg}"
        );

        // Each label's (x, y) is its visual centre. The nearest DIM_STROKE (0.6) line
        // that runs along the label must stay outside the glyph box.
        let dim_lines = parse_svg_dim_lines(&svg);
        assert!(
            !dim_lines.is_empty(),
            "export should stroke dimension lines, got:\n{svg}"
        );
        for line in &dim_labels {
            let (x, y, size, angle) = parse_svg_text_layout(line);
            let half = size * crate::drawing::DIM_LABEL_MID_EM;
            let nearest = dim_lines
                .iter()
                .map(|ln| perp_dist_to_seg(x, y, *ln))
                .fold(f32::MAX, f32::min);
            assert!(
                nearest > half + 0.4,
                "label at ({x:.1},{y:.1}) ang={angle:.2} overlaps a dim line (dist {nearest:.2}, need > {:.2}): {line}",
                half + 0.4
            );
        }

        // PDF: the same labels are emitted with a Tm whose baseline is 0.35em off the
        // layout point (so the glyphs centre on it, matching the SVG/editor).
        assert!(
            pdf_text.contains("80.0 mm") || pdf_text.contains("(80.0 mm)"),
            "PDF should contain the 80 mm label"
        );
    }

    fn parse_svg_text_layout(line: &str) -> (f32, f32, f32, f32) {
        let attr = |name: &str| -> f32 {
            let key = format!("{name}=\"");
            let rest = line.split_once(&key).expect(name).1;
            rest.split('"').next().unwrap().parse().unwrap()
        };
        let angle = line
            .split_once("rotate(")
            .map(|(_, rest)| rest.split([',', ' ']).next().unwrap().parse().unwrap_or(0.0))
            .unwrap_or(0.0);
        (attr("x"), attr("y"), attr("font-size"), angle)
    }

    fn parse_svg_dim_lines(svg: &str) -> Vec<(f32, f32, f32, f32)> {
        let mut out = Vec::new();
        for line in svg.lines() {
            if !line.contains("stroke-width=\"0.6\"") || !line.contains("<line") {
                continue;
            }
            let attr = |name: &str| -> Option<f32> {
                let key = format!("{name}=\"");
                let rest = line.split_once(&key)?.1;
                rest.split('"').next()?.parse().ok()
            };
            let Some(x1) = attr("x1") else { continue };
            let Some(y1) = attr("y1") else { continue };
            let Some(x2) = attr("x2") else { continue };
            let Some(y2) = attr("y2") else { continue };
            out.push((x1, y1, x2, y2));
        }
        out
    }

    fn perp_dist_to_seg(x: f32, y: f32, (x1, y1, x2, y2): (f32, f32, f32, f32)) -> f32 {
        let (dx, dy) = (x2 - x1, y2 - y1);
        let len2 = dx * dx + dy * dy;
        if len2 < 1e-6 {
            return (x - x1).hypot(y - y1);
        }
        let t = ((x - x1) * dx + (y - y1) * dy) / len2;
        if t < 0.0 {
            (x - x1).hypot(y - y1)
        } else if t > 1.0 {
            (x - x2).hypot(y - y2)
        } else {
            let (px, py) = (x1 + t * dx, y1 + t * dy);
            (x - px).hypot(y - py)
        }
    }

    /// #180: `bearcad.export_drawing_svg{}` writes the SVG to disk.
    #[test]
    fn lua_export_drawing_svg_writes_a_file() {
        let path = std::env::temp_dir()
            .join(format!("bearcad_drawing_{}.svg", std::process::id()));
        let p = path.to_string_lossy().replace('\\', "/");
        run_lua(&format!(
            r#"
            bearcad.new()
            bearcad.rect{{ width = 20, height = 20 }}
            bearcad.extrude{{ polygon = {{0, 1, 2, 3}}, distance = 10 }}
            local d = bearcad.drawing{{}}
            bearcad.drawing_view{{ drawing = d, body = 0, orientation = "iso" }}
            bearcad.export_drawing_svg{{ drawing = d, path = "{p}" }}
        "#
        ));
        let content = std::fs::read_to_string(&path).expect("svg file was written");
        assert!(content.contains("<svg"));
        let _ = std::fs::remove_file(&path);
    }

    /// #1206: projection-line endpoints land on the facing silhouette of the body at each
    /// shared-axis extreme — not on floating AABB corners. A short body on the left and a tall
    /// one on the right make the Front-Top AABB's top-left corner empty; the left line must
    /// touch the short body's top, not hover at the tall body's height.
    #[test]
    fn lua_align_lines_touch_body_edges_not_aabb_corners() {
        let mut state = run_lua(
            r#"
            bearcad.new()
            bearcad.cuboid{ at = {0, 0, 0}, width = 20, depth = 20, height = 10 }
            bearcad.cuboid{ at = {60, 0, 0}, width = 20, depth = 20, height = 50 }
            local d = bearcad.drawing{}
            bearcad.drawing_view{ drawing = d, bodies = {0, 1}, orientation = "top" }
            bearcad.drawing_align_view{ drawing = d, parent = 0, dir = "below", pos = 0.75 }
        "#,
        );
        // Roll the aligned child to Front-Top (same as the report: Top base → Front-Top below).
        state.doc.drawings[dkey(0)].views[1].orientation =
            crate::model::DrawingOrientation::Edge(crate::model::EdgeView::FrontTop);
        let views = &state.doc.drawings[dkey(0)].views;
        let lines = crate::drawing::aligned_projection_lines(&state.doc, views, 1)
            .expect("aligned child yields projection lines");
        let child = &views[1];
        let (right, up) = crate::drawing::resolved_view_axes(views, child);
        let edges =
            crate::drawing::drawing_view_dimensionable_edges(&state.doc, views, child);
        let pts: Vec<glam::Vec2> = edges
            .iter()
            .flat_map(|(a, b)| {
                [
                    glam::Vec2::new(a.dot(right), a.dot(up)),
                    glam::Vec2::new(b.dot(right), b.dot(up)),
                ]
            })
            .collect();
        assert!(!pts.is_empty(), "child has projected geometry");
        let cmax_y = pts.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        let cmin_x = pts.iter().map(|p| p.x).fold(f32::MAX, f32::min);
        // Each child endpoint must land on a projected vertex (the silhouette), not float at
        // an empty AABB corner.
        for (i, (_ppt, cpt)) in lines.iter().enumerate() {
            let near = pts.iter().any(|p| (*p - *cpt).length() < 0.75);
            assert!(
                near,
                "line {i} child endpoint {cpt:?} must sit on a silhouette vertex, got pts near x≈{}",
                cpt.x
            );
        }
        // Left line (index 0): at the short body's extreme, its facing top is well below the
        // overall AABB top (the tall body). Using (cmin.x, cmax.y) would fail this.
        let left = lines[0].1;
        assert!(
            (left.x - cmin_x).abs() < 1.0,
            "left line at the shared-axis min: left.x={}, cmin_x={cmin_x}",
            left.x
        );
        assert!(
            left.y < cmax_y - 5.0,
            "left line must touch the short body, not the tall AABB top: left.y={}, cmax_y={cmax_y}",
            left.y
        );
    }

    /// #377: toggled projection lines export as dashed strokes connecting the aligned pair.
    #[test]
    fn lua_align_lines_export_as_dashed_strokes() {
        let path = std::env::temp_dir()
            .join(format!("bearcad_align_lines_{}.svg", std::process::id()));
        let p = path.to_string_lossy().replace('\\', "/");
        run_lua(&format!(
            r#"
            bearcad.new()
            bearcad.rect{{ width = 20, height = 20 }}
            bearcad.extrude{{ polygon = {{0, 1, 2, 3}}, distance = 10 }}
            local d = bearcad.drawing{{}}
            bearcad.drawing_view{{ drawing = d, body = 0, orientation = "front" }}
            bearcad.drawing_align_view{{ drawing = d, parent = 0, dir = "below", pos = 0.75 }}
            bearcad.drawing_view_align_lines{{ drawing = d, view = 1, show = true }}
            bearcad.export_drawing_svg{{ drawing = d, path = "{p}" }}
        "#
        ));
        let content = std::fs::read_to_string(&path).expect("svg file was written");
        assert_eq!(
            content.matches("stroke-dasharray").count(),
            2,
            "two dashed projection lines"
        );
        // Both lines are vertical (the child is below): x1 == x2 on each.
        for line in content.lines().filter(|l| l.contains("stroke-dasharray")) {
            let attr = |k: &str| {
                let s = line.split(&format!("{k}=\"")).nth(1).unwrap();
                s.split('"').next().unwrap().parse::<f32>().unwrap()
            };
            assert!(
                (attr("x1") - attr("x2")).abs() < 0.2,
                "below-aligned projection lines are vertical: {line}"
            );
            assert!(attr("y2") > attr("y1"), "lines run from parent down to child: {line}");
        }
        let _ = std::fs::remove_file(&path);
    }

    /// #116: `bearcad.plane{}` declaratively adds a construction plane offset along the
    /// normal of an existing one (plane 0 / Ground by default) — the scripted equivalent of
    /// picking a plane in the viewport and typing an offset.
    #[test]
    fn lua_plane_adds_offset_construction_plane() {
        let state = run_lua("bearcad.plane{ offset = 5 }");
        // Three datum planes (#833) plus the one the script added.
        assert_eq!(state.doc.construction_planes.len(), 4);
        let plane = &state.doc.construction_planes[pkey(3)];
        assert!(
            (plane.origin.z - 5.0).abs() < 1e-3,
            "origin should sit 5mm above Ground along its normal, got {:?}",
            plane.origin
        );
        assert!((plane.normal - glam::Vec3::Z).length() < 1e-3);
    }

    #[test]
    fn lua_plane_offsets_from_an_explicit_from_index() {
        let state = run_lua(
            r#"
            bearcad.plane{ offset = 5 }
            bearcad.plane{ offset = 3, from = 3 }
        "#,
        );
        assert_eq!(state.doc.construction_planes.len(), 5);
        assert!(
            (state.doc.construction_planes[pkey(4)].origin.z - 8.0).abs() < 1e-3,
            "plane 4 should stack a further 3mm on top of plane 3's 5mm, got {:?}",
            state.doc.construction_planes[pkey(4)].origin
        );
    }

    #[test]
    fn lua_plane_rejects_unknown_from_index() {
        let mut runner = ScriptRunner::from_lua_source("bearcad.plane{ offset = 5, from = 9 }").unwrap();
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, None, &ctx);
        }
        let err = runner.error.expect("unknown plane index should error");
        assert!(err.contains("Unknown construction plane 9"), "unexpected error: {err}");
    }
    /// #91/#135: `bearcad.ui.fps()` toggles first-person mode; entering keeps the camera
    /// exactly where it was (the player's eye starts at the camera eye, so the view doesn't
    /// move), exiting leaves the mode.
    #[test]
    fn lua_fps_mode_toggles_and_keeps_the_camera_view() {
        let before = crate::camera::Camera::default();
        let state = run_lua("bearcad.ui.fps()");
        let player = state.fps.as_ref().expect("fps mode should be active");
        assert!(
            (player.eye - before.eye()).length() < 1e-2,
            "entering FPS must not move the eye: camera was {:?}, player at {:?}",
            before.eye(),
            player.eye
        );
        assert!(
            (state.cam.eye() - player.eye).length() < 1e-2,
            "camera eye should sit at the player eye"
        );
        let look_before = (before.target - before.eye()).normalize();
        let look_after = (state.cam.target - state.cam.eye()).normalize();
        assert!(
            (look_before - look_after).length() < 1e-3,
            "entering FPS must not change the look direction"
        );

        let state = run_lua("bearcad.ui.fps() bearcad.ui.fps()");
        assert!(state.fps.is_none(), "second toggle should leave FPS mode");
        let state = run_lua("bearcad.ui.fps(true) bearcad.ui.fps(true)");
        assert!(state.fps.is_some(), "fps(true) is idempotent");
    }

    /// #135: the default camera sits below standing eye height, so entering FPS there
    /// shrinks the player (#120) to keep the view in place instead of popping it up.
    #[test]
    fn lua_fps_enter_below_eye_height_shrinks_the_player() {
        let state = run_lua("bearcad.ui.fps()");
        let player = state.fps.as_ref().unwrap();
        assert!(player.scale < 1.0, "player should shrink, scale={}", player.scale);
        assert!(
            player.on_ground(),
            "shrunk entry at the camera height should be standing"
        );
    }

    /// #91: `fps_move` walks on the ground plane and `fps_look` turns the head; the
    /// orbit camera follows the player.
    #[test]
    fn lua_fps_move_and_look_drive_the_camera() {
        let state = run_lua(
            r#"
            bearcad.ui.fps()
            bearcad.ui.fps_scale(1)
            bearcad.ui.fps_look(90, 0)
            bearcad.ui.fps_move{ forward = 1000, strafe = 500 }
        "#,
        );
        let player = state.fps.as_ref().unwrap();
        assert!((player.eye.z - crate::fps::EYE_HEIGHT).abs() < 1e-3, "walking stays grounded");
        // Entering keeps the previous look heading (here the default isometric view),
        // so the look direction is not level — only the walking is.
        let look = player.look_dir();
        assert!((state.cam.target - player.eye).length() > 1.0, "target sits ahead of the eye");
        let cam_look = (state.cam.target - state.cam.eye()).normalize();
        assert!((cam_look - look).length() < 1e-3, "camera look matches the player");
    }

    /// #91: Space jumps (ballistic rise and land) and double-tap flying holds altitude —
    /// scripted via fps_jump/fps_fly/fps_advance.
    #[test]
    fn lua_fps_jump_and_fly_physics() {
        let state = run_lua(
            r#"
            bearcad.ui.fps()
            bearcad.ui.fps_scale(1)
            bearcad.ui.fps_jump()
            bearcad.ui.fps_advance(0.2)
        "#,
        );
        let z = state.fps.as_ref().unwrap().eye.z;
        assert!(z > crate::fps::EYE_HEIGHT + 100.0, "mid-jump should be airborne, z={z}");

        let state = run_lua(
            r#"
            bearcad.ui.fps()
            bearcad.ui.fps_scale(1)
            bearcad.ui.fps_jump()
            bearcad.ui.fps_advance(3)
        "#,
        );
        let z = state.fps.as_ref().unwrap().eye.z;
        assert!((z - crate::fps::EYE_HEIGHT).abs() < 1e-2, "gravity should land the jump, z={z}");

        let state = run_lua(
            r#"
            bearcad.ui.fps()
            bearcad.ui.fps_scale(1)
            bearcad.ui.fps_fly(true)
            bearcad.ui.fps_jump()
            bearcad.ui.fps_advance(3)
        "#,
        );
        let player = state.fps.as_ref().unwrap();
        assert!(player.flying, "fps_fly(true) should be flying");
        assert!(
            (player.eye.z - crate::fps::EYE_HEIGHT).abs() < 1e-2,
            "flying holds altitude (no gravity), z={}",
            player.eye.z
        );
    }

    /// #135: leaving FPS mode mid-flight and re-entering resumes flying at the same
    /// altitude, instead of dropping the player back to standing on the ground.
    #[test]
    fn lua_fps_reenter_resumes_flying_altitude() {
        let state = run_lua(
            r#"
            bearcad.ui.fps()
            bearcad.ui.fps_scale(1)
            bearcad.ui.fps_jump()
            bearcad.ui.fps_advance(0.2)
            bearcad.ui.fps_fly(true)
        "#,
        );
        let player = state.fps.as_ref().unwrap();
        assert!(player.flying);
        let z1 = player.eye.z;
        assert!(z1 > crate::fps::EYE_HEIGHT + 100.0, "should be well above ground, z={z1}");

        let state = run_lua(
            r#"
            bearcad.ui.fps()
            bearcad.ui.fps_scale(1)
            bearcad.ui.fps_jump()
            bearcad.ui.fps_advance(0.2)
            bearcad.ui.fps_fly(true)
            bearcad.ui.fps(false)
            bearcad.ui.fps(true)
        "#,
        );
        let player = state.fps.as_ref().expect("should be back in fps mode");
        assert!(player.flying, "re-entry should resume flying");
        assert!(
            (player.eye.z - z1).abs() < 1.0,
            "re-entry should resume the same altitude: expected ~{z1}, got {}",
            player.eye.z
        );
    }

    /// #91: FPS commands outside FPS mode raise catchable errors.
    #[test]
    fn lua_fps_commands_require_fps_mode() {
        run_lua_expect_ok(
            r#"
            for _, f in ipairs({
                function() bearcad.ui.fps_jump() end,
                function() bearcad.ui.fps_look(10, 0) end,
                function() bearcad.ui.fps_move{ forward = 100 } end,
                function() bearcad.ui.fps_fly() end,
                function() bearcad.ui.fps_advance(1) end,
                function() bearcad.ui.fps_scale(0.5) end,
            }) do
                local ok, err = pcall(f)
                assert(not ok, "fps command should raise outside FPS mode")
                assert(tostring(err):find("FPS"), "unexpected error: " .. tostring(err))
            end
        "#,
        );
    }

    /// #120: `bearcad.ui.fps_scale(value)` shrinks/grows the player, scaling eye height and
    /// movement/jump speed together so mm-detail and building-scale work are both usable.
    #[test]
    fn lua_fps_scale_resizes_the_player_and_their_movement() {
        let state = run_lua(
            r#"
            bearcad.ui.fps()
            bearcad.ui.fps_scale(0.1)
        "#,
        );
        let player = state.fps.as_ref().unwrap();
        assert!(
            (player.scale - 0.1).abs() < 1e-4,
            "scale should be set directly, got {}",
            player.scale
        );
        assert!(
            (player.eye.z - crate::fps::EYE_HEIGHT * 0.1).abs() < 1e-2,
            "eye height should scale down with the player, z={}",
            player.eye.z
        );

        let state = run_lua(
            r#"
            bearcad.ui.fps()
            bearcad.ui.fps_scale(0.1)
            bearcad.ui.fps_move{ forward = 100 }
        "#,
        );
        let small_x = state.fps.as_ref().unwrap().eye.x;

        let state = run_lua(
            r#"
            bearcad.ui.fps()
            bearcad.ui.fps_move{ forward = 100 }
        "#,
        );
        let normal_x = state.fps.as_ref().unwrap().eye.x;
        assert!(
            (small_x - normal_x).abs() < 1e-3,
            "fps_move is an absolute mm offset, unaffected by player scale: small={small_x} normal={normal_x}"
        );
    }

    /// #120: out-of-range scales are clamped, not rejected.
    #[test]
    fn lua_fps_scale_is_clamped_to_the_documented_range() {
        let state = run_lua(
            r#"
            bearcad.ui.fps()
            bearcad.ui.fps_scale(1e9)
        "#,
        );
        assert_eq!(state.fps.as_ref().unwrap().scale, crate::fps::MAX_SCALE);

        let state = run_lua(
            r#"
            bearcad.ui.fps()
            bearcad.ui.fps_scale(-5)
        "#,
        );
        assert_eq!(state.fps.as_ref().unwrap().scale, crate::fps::MIN_SCALE);
    }
}
