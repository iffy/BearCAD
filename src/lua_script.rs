//! Lua scripting API (`bearcad` global) for driving the live application.

use crate::actions::{DimLabelAxis, Pane, RectAxis, Tool};
use crate::camera::{ProjectionMode, ShadingMode, StandardView};
use crate::construction::PlaneDim;
use crate::geometric_constraints::GeometricConstraintType;
use crate::hierarchy::SceneElement;
use crate::model::{
    ConstraintKind, ConstraintLine, ConstraintPoint, DistanceTarget, ExtrusionEdgeRef, FaceId,
    LineEnd, SketchId, VertexTreatmentKind,
};
use crate::names::find_element_by_name;
use crate::script::{parse_key, Instruction, ScriptRunner, SyntheticInput};
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
}

impl UserData for LuaElement {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("kind", |_, this, ()| Ok(element_kind_name(this.element.clone())));
        methods.add_method("index", |_, this, ()| Ok(element_index(this.element.clone())));
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
    }
}

fn element_index(element: SceneElement) -> usize {
    match element {
        SceneElement::ConstructionPlane(i)
        | SceneElement::Sketch(i)
        | SceneElement::Line(i)
        | SceneElement::Circle(i)
        | SceneElement::Constraint(i)
        | SceneElement::Extrusion(i)
        | SceneElement::Body(i) => i,
        SceneElement::Point(_) | SceneElement::FaceEdge(_) => 0,
    }
}

pub fn scene_element_from_kind(kind: &str, index: usize) -> Option<SceneElement> {
    match kind.to_ascii_lowercase().as_str() {
        "plane" | "construction_plane" | "constructionplane" => {
            Some(SceneElement::ConstructionPlane(index))
        }
        "sketch" => Some(SceneElement::Sketch(index)),
        "line" => Some(SceneElement::Line(index)),
        "circle" => Some(SceneElement::Circle(index)),
        "constraint" => Some(SceneElement::Constraint(index)),
        "extrusion" => Some(SceneElement::Extrusion(index)),
        "body" => Some(SceneElement::Body(index)),
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
    Ok(Value::UserData(lua.create_userdata(LuaElement { element })?))
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
            return Ok(SceneElement::FaceEdge(parse_constraint_line_table(table)?));
        }
        return Ok(SceneElement::Point(parse_constraint_point_table(table)?));
    }
    let index: usize = table.get("index")?;
    // Point-level selector (#68): a line endpoint (`end = "start"|"end"`), or an explicit
    // `point = true` (e.g. a circle's center) — otherwise
    // `kind`/`index` alone resolve to the whole element as before.
    if table.contains_key("end")?
        || table.contains_key("corner")?
        || table.get::<Option<bool>>("point")?.unwrap_or(false)
    {
        return Ok(SceneElement::Point(parse_constraint_point_table(table)?));
    }
    scene_element_from_kind(&kind, index)
        .ok_or_else(|| mlua::Error::external(format!("unknown element kind '{kind}'")))
}

/// Parses a `begin_sketch`/`face = { ... }` table into a `FaceId`. 3D body faces
/// (`extrude_cap`/`extrude_side`) need extra descriptors (extrusion + profile + which face), so
/// they can't go through the plain `(kind, index)` `FaceId::from_script` path; everything else
/// does. Shared by `begin_sketch` and the `face` arms of `parse_constraint_point_table`/
/// `parse_constraint_line_table` below (#26/#27's `FaceVertex`/`FaceEdge` from a script).
fn parse_face_id_table(table: Table) -> mlua::Result<FaceId> {
    let kind: String = table.get("kind").or_else(|_| table.get("type"))?;
    match kind.to_ascii_lowercase().as_str() {
        "extrude_cap" | "extrude_side" => {
            let extrusion: usize = table.get("extrusion")?;
            let profile_kind: String =
                table.get("profile").or_else(|_| table.get("profile_kind"))?;
            let profile_index: usize = table
                .get("profile_index")
                .or_else(|_| table.get("index"))
                .unwrap_or(0);
            let profile = match profile_kind.to_ascii_lowercase().as_str() {
                "circle" => crate::model::ExtrudeFace::Circle(profile_index),
                // A rectangle is now a `Polygon` loop (#66); give its four line indices as
                // `profile_lines = {..}`.
                "polygon" => {
                    let lines: Vec<usize> = table
                        .get("profile_lines")
                        .or_else(|_| table.get("lines"))?;
                    crate::model::ExtrudeFace::Polygon(lines)
                }
                other => {
                    return Err(mlua::Error::external(format!(
                        "unknown extrude profile kind '{other}'"
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
        _ => {
            let index: usize = table.get("index")?;
            FaceId::from_script(&kind, index).ok_or_else(|| {
                mlua::Error::external(format!("unknown sketch face kind '{kind}'"))
            })
        }
    }
}

/// An `ExtrudeFace` from a face-spec table: `{rect = i}`, `{circle = i}`, `{polygon = {..}}`,
/// or a nested `{boolean = {op = "intersection"|"difference", a = <face spec>, b = <face
/// spec>}}` (#16/#62). Mirrors `extrude_face_spec_table`/`boolean_face_lua_table` in
/// src/script.rs, which render this same shape back out for the recorded-script export.
fn parse_extrude_face_table(table: &Table) -> mlua::Result<crate::model::ExtrudeFace> {
    if let Some(i) = table.get::<Option<usize>>("circle")? {
        return Ok(crate::model::ExtrudeFace::Circle(i));
    }
    if let Some(lines) = table.get::<Option<Vec<usize>>>("polygon")? {
        return Ok(crate::model::ExtrudeFace::Polygon(lines));
    }
    if let Some(boolean) = table.get::<Option<Table>>("boolean")? {
        return parse_boolean_face_table(&boolean);
    }
    Err(mlua::Error::external(
        "face spec requires one of circle/polygon/boolean",
    ))
}

fn parse_boolean_face_table(table: &Table) -> mlua::Result<crate::model::ExtrudeFace> {
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
        a: Box::new(parse_extrude_face_table(&a)?),
        b: Box::new(parse_extrude_face_table(&b)?),
    })
}

fn parse_constraint_line_table(table: Table) -> mlua::Result<ConstraintLine> {
    let kind: String = table.get("kind").or_else(|_| table.get("type"))?;
    if kind.eq_ignore_ascii_case("face") {
        // { kind = "face", face = { kind = "extrude_cap", extrusion = 0, profile = "rect",
        //   profile_index = 0, top = true }, index = 2 } — edge `index` of that face's own
        // boundary loop (#26/#27's `FaceEdge`).
        let face_table: Table = table.get("face")?;
        let face = parse_face_id_table(face_table)?;
        let index: usize = table.get("index")?;
        return Ok(ConstraintLine::FaceEdge { face, index });
    }
    let index: usize = table.get("index")?;
    match kind.to_ascii_lowercase().as_str() {
        "line" => Ok(ConstraintLine::Line(index)),
        other => Err(mlua::Error::external(format!(
            "drag_line target must be line, not '{other}'"
        ))),
    }
}

fn parse_constraint_point_table(table: Table) -> mlua::Result<ConstraintPoint> {
    let kind: String = table.get("kind").or_else(|_| table.get("type"))?;
    if kind.eq_ignore_ascii_case("face") {
        // { kind = "face", face = { ... }, index = 0 } — vertex `index` of that face's own
        // boundary loop (#26/#27's `FaceVertex`).
        let face_table: Table = table.get("face")?;
        let face = parse_face_id_table(face_table)?;
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
            Ok(ConstraintPoint::LineEndpoint { line: index, end })
        }
        "circle" => Ok(ConstraintPoint::CircleCenter(index)),
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
    let face: usize = table.get("face")?;
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

fn parse_geometric_constraint(name: &str) -> Option<GeometricConstraintType> {
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

fn parse_distance_target(table: Table) -> mlua::Result<DistanceTarget> {
    let kind: String = table.get("kind").or_else(|_| table.get("type"))?;
    let index: usize = table.get("index")?;
    match kind.to_ascii_lowercase().as_str() {
        "line" => Ok(DistanceTarget::LineLength(index)),
        "circle" => Ok(DistanceTarget::CircleDiameter(index)),
        other => Err(mlua::Error::external(format!(
            "unknown constraint target '{other}'"
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
        ConstraintKind::Horizontal { .. } => "horizontal",
        ConstraintKind::Vertical { .. } => "vertical",
        ConstraintKind::Angle { .. } => "angle",
    }
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
    unsafe { tick.exec(Instruction::SetElementName { element, name }) }
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

    api.set(
        "save",
        lua.create_function(|lua, path: Option<String>| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Save(path)) }
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
        "export_step",
        lua.create_function(|lua, (path, body): (String, Option<String>)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ExportStep { path, body }) }
        })?,
    )?;

    api.set(
        "import_stl",
        lua.create_function(|lua, path: String| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ImportStl { path }) }
        })?,
    )?;

    api.set(
        "import_step",
        lua.create_function(|lua, path: String| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ImportStep { path }) }
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

    api.set(
        "quit",
        lua.create_function(|lua, ()| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Quit) }
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
                parse_face_id_table(table.clone())?
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
                FaceId::from_script(&kind, index).ok_or_else(|| {
                    mlua::Error::external(format!("unknown sketch face kind '{kind}'"))
                })?
            };
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::BeginSketch { face }) }
        })?,
    )?;

    api.set(
        "open_sketch",
        lua.create_function(|lua, sketch: SketchId| {
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
            let element = scene_element_from_kind(&kind, index).ok_or_else(|| {
                mlua::Error::external(format!("unknown element kind '{kind}'"))
            })?;
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
            if let Some(sketch) = opts.get::<Option<SketchId>>("sketch")? {
                unsafe { tick.exec(Instruction::SetSketchUnits { sketch, length, angle }) }
            } else {
                let doc = unsafe { &tick.state().doc };
                let length = length.unwrap_or(doc.default_length_unit);
                let angle = angle.unwrap_or(doc.default_angle_unit);
                unsafe { tick.exec(Instruction::SetDocumentUnits { length, angle }) }
            }
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
        lua.create_function(|lua, sketch: Option<SketchId>| {
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let state = unsafe { tick.state() };
            let sketch = sketch
                .or_else(|| state.sketch_session.map(|session| session.sketch))
                .ok_or_else(|| mlua::Error::external("no active sketch"))?;
            let conflicts =
                crate::constraints::sketch_conflicting_constraints(&state.doc, sketch)
                    .map_err(mlua::Error::external)?;
            let table = lua.create_table()?;
            for (i, index) in conflicts.iter().enumerate() {
                table.set(i + 1, *index)?;
            }
            Ok(table)
        })?,
    )?;

    api.set(
        "sketch_dof",
        lua.create_function(|lua, sketch: Option<SketchId>| {
            let tick = lua
                .app_data_ref::<ScriptTickData>()
                .ok_or_else(|| mlua::Error::external("script tick context missing"))?;
            let state = unsafe { tick.state() };
            let sketch = sketch
                .or_else(|| state.sketch_session.map(|session| session.sketch))
                .ok_or_else(|| mlua::Error::external("no active sketch"))?;
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
                "line" => doc.lines.iter().filter(|e| !e.deleted).count(),
                "circle" => doc.circles.iter().filter(|e| !e.deleted).count(),
                "sketch" => doc.sketches.iter().filter(|e| !e.deleted).count(),
                "constraint" => doc.constraints.iter().filter(|e| !e.deleted).count(),
                "construction_plane" | "plane" => {
                    doc.construction_planes.iter().filter(|e| !e.deleted).count()
                }
                "extrusion" => doc.extrusions.iter().filter(|e| !e.deleted).count(),
                "body" => doc.bodies.iter().filter(|e| !e.deleted).count(),
                "parameter" => doc.parameters.iter().filter(|e| !e.deleted).count(),
                other => {
                    return Err(mlua::Error::external(format!(
                        "unknown count kind '{other}' (valid kinds: line, circle, sketch, \
                         constraint, construction_plane, extrusion, body, parameter)"
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
                    let Some(line) = doc.lines.get(index).filter(|e| !e.deleted) else {
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
                    if let Some(name) = &line.name {
                        t.set("name", name.as_str())?;
                    }
                    t.set("sketch", line.sketch)?;
                }
                "circle" => {
                    let Some(circle) = doc.circles.get(index).filter(|e| !e.deleted) else {
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
                    t.set("sketch", circle.sketch)?;
                }
                "sketch" => {
                    let Some(sketch) = doc.sketches.get(index).filter(|e| !e.deleted) else {
                        return Ok(Value::Nil);
                    };
                    t.set("face", face_kind_name(&sketch.face))?;
                    if let Some(name) = &sketch.name {
                        t.set("name", name.as_str())?;
                    }
                }
                "constraint" => {
                    let Some(constraint) = doc.constraints.get(index).filter(|e| !e.deleted)
                    else {
                        return Ok(Value::Nil);
                    };
                    t.set("kind", constraint_kind_name(&constraint.kind))?;
                    t.set("expression", constraint.expression.as_str())?;
                    if let Some(name) = &constraint.name {
                        t.set("name", name.as_str())?;
                    }
                    t.set("sketch", constraint.sketch)?;
                }
                "construction_plane" | "plane" => {
                    let Some(plane) =
                        doc.construction_planes.get(index).filter(|e| !e.deleted)
                    else {
                        return Ok(Value::Nil);
                    };
                    t.set("origin", vec3_lua(lua, plane.origin)?)?;
                    t.set("normal", vec3_lua(lua, plane.normal)?)?;
                    if let Some(name) = &plane.name {
                        t.set("name", name.as_str())?;
                    }
                }
                "extrusion" => {
                    let Some(extrusion) = doc.extrusions.get(index).filter(|e| !e.deleted)
                    else {
                        return Ok(Value::Nil);
                    };
                    t.set("distance", extrusion.distance)?;
                    t.set("sketch", extrusion.sketch)?;
                    t.set("faces", extrusion.faces.len())?;
                    if let Some(name) = &extrusion.name {
                        t.set("name", name.as_str())?;
                    }
                }
                "body" => {
                    let Some(body) = doc.bodies.get(index).filter(|e| !e.deleted) else {
                        return Ok(Value::Nil);
                    };
                    if let Some(name) = &body.name {
                        t.set("name", name.as_str())?;
                    }
                    let add = lua.create_table()?;
                    for (i, ei) in body.source.extrusion_indices().iter().enumerate() {
                        add.set(i + 1, *ei)?;
                    }
                    t.set("add", add)?;
                    let cut = lua.create_table()?;
                    for (i, ei) in body.source.cut_extrusion_indices().iter().enumerate() {
                        cut.set(i + 1, *ei)?;
                    }
                    t.set("cut", cut)?;
                }
                "parameter" => {
                    let Some(param) = doc.parameters.get(index).filter(|e| !e.deleted) else {
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
            if !doc.bodies.get(index).is_some_and(|b| !b.deleted) {
                return Ok(Value::Nil);
            }
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
                if !matches!(element, SceneElement::Point(_) | SceneElement::FaceEdge(_)) {
                    entry.set("index", element_index(element))?;
                }
                out.set(i + 1, entry)?;
            }
            Ok(out)
        })?,
    )?;

    api.set(
        "add_constraint",
        lua.create_function(|lua, (target, expression): (Table, String)| {
            let target = parse_distance_target(target)?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe {
                tick.exec(Instruction::AddDistanceConstraint { target, expression },
                )
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

    api.set(
        "drag_vertex",
        lua.create_function(|lua, (point, u, v): (Table, f32, f32)| {
            let point = parse_constraint_point_table(point)?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::DragVertex { point, u, v }) }
        })?,
    )?;

    api.set(
        "drag_line",
        lua.create_function(
            |lua, (target, anchor_u, anchor_v, u, v): (Table, f32, f32, f32, f32)| {
                let target = parse_constraint_line_table(target)?;
                let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
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

    // #108: frame the whole document (bodies + sketch geometry) instantly.
    api.set(
        "zoom_fit",
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
                    let Some(param) =
                        doc.parameters.iter().find(|p| !p.deleted && p.name == name)
                    else {
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
                    unsafe { tick.exec(Instruction::RunPaletteCommand { query }) }
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
        "move",
        lua.create_function(|lua, (x, y): (f32, f32)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Move { x, y }) }
        })?,
    )?;

    api.set(
        "click",
        lua.create_function(|lua, (x, y): (f32, f32)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Click { x, y }) }
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
        "click_ground",
        lua.create_function(|lua, (x, y): (f32, f32)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::ClickGround { x, y }) }
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
        lua.create_function(|lua, name: String| {
            let key = parse_key(&name)
                .map_err(|e| mlua::Error::external(e))?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::Key(key)) }
        })?,
    )?;

    api.set(
        "keydown",
        lua.create_function(|lua, name: String| {
            let key = parse_key(&name)
                .map_err(|e| mlua::Error::external(e))?;
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            unsafe { tick.exec(Instruction::KeyDown(key)) }
        })?,
    )?;

    api.set(
        "keyup",
        lua.create_function(|lua, name: String| {
            let key = parse_key(&name)
                .map_err(|e| mlua::Error::external(e))?;
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
        lua.create_function(|lua, (path, whole_window): (Option<String>, Option<bool>)| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let path = path
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| "screenshot-bearcad.png".to_string());
            unsafe {
                tick.exec(Instruction::Screenshot {
                    path,
                    whole_window: whole_window.unwrap_or(false),
                })
            }
        })?,
    )?;

    api.set(
        "rect",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let width: f32 = opts.get("width")?;
            let height: f32 = opts.get("height")?;
            let x: f32 = opts.get("x").unwrap_or(0.0);
            let y: f32 = opts.get("y").unwrap_or(0.0);
            unsafe {
                // Make sure we're sketching; default to the ground (XY) construction plane.
                if tick.state().sketch_session.is_none() {
                    tick.exec(Instruction::BeginSketch {
                        face: FaceId::ConstructionPlane(0),
                    })?;
                }
                tick.exec(Instruction::CreateRect {
                    x,
                    y,
                    width,
                    height,
                })?;
            }
            // A rectangle is now four plain lines (#66 polygon); return a handle to its bottom
            // edge (the first of the four lines just created).
            let element = {
                let n = unsafe { tick.state().doc.lines.len() };
                SceneElement::Line(n.saturating_sub(4))
            };
            apply_optional_name(lua, element, Some(opts))
        })?,
    )?;

    api.set(
        "line",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
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
            unsafe {
                if tick.state().sketch_session.is_none() {
                    tick.exec(Instruction::BeginSketch {
                        face: FaceId::ConstructionPlane(0),
                    })?;
                }
                tick.exec(Instruction::CreateLine { x0, y0, x1, y1, bezier })?;
            }
            let element =
                SceneElement::Line(unsafe { tick.state().doc.lines.len().saturating_sub(1) });
            apply_optional_name(lua, element, Some(opts))
        })?,
    )?;

    api.set(
        "circle",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let cx: f32 = opts.get("x").unwrap_or(0.0);
            let cy: f32 = opts.get("y").unwrap_or(0.0);
            // Accept a radius (`r` or its `radius` alias, #108) or a `diameter`, in that
            // precedence order; none at all is a clear error rather than a nil-conversion one.
            let r: f32 = if let Some(r) = opts.get::<Option<f32>>("r")? {
                r
            } else if let Some(radius) = opts.get::<Option<f32>>("radius")? {
                radius
            } else if let Some(diameter) = opts.get::<Option<f32>>("diameter")? {
                diameter * 0.5
            } else {
                return Err(mlua::Error::external(
                    "circle requires a size: one of `r`, `radius`, or `diameter`",
                ));
            };
            unsafe {
                if tick.state().sketch_session.is_none() {
                    tick.exec(Instruction::BeginSketch {
                        face: FaceId::ConstructionPlane(0),
                    })?;
                }
                tick.exec(Instruction::CreateCircle { cx, cy, r })?;
            }
            let element =
                SceneElement::Circle(unsafe { tick.state().doc.circles.len().saturating_sub(1) });
            apply_optional_name(lua, element, Some(opts))
        })?,
    )?;

    api.set(
        "extrude",
        lua.create_function(|lua, opts: Table| {
            let tick = lua.app_data_ref::<ScriptTickData>().unwrap();
            let distance: f32 = opts.get("distance")?;
            // Faces: `circle` (single) and/or `circles` (array of indices), a `polygon` loop
            // (#66 — a rectangle is four lines forming such a loop), or a `boolean` region.
            let mut faces: Vec<crate::model::ExtrudeFace> = Vec::new();
            if let Some(i) = opts.get::<Option<usize>>("circle")? {
                faces.push(crate::model::ExtrudeFace::Circle(i));
            }
            if let Some(list) = opts.get::<Option<Vec<usize>>>("circles")? {
                faces.extend(list.into_iter().map(crate::model::ExtrudeFace::Circle));
            }
            // `polygon = {line0, line1, ...}`: a single closed-loop face (#66).
            if let Some(lines) = opts.get::<Option<Vec<usize>>>("polygon")? {
                faces.push(crate::model::ExtrudeFace::Polygon(lines));
            }
            // `boolean = {op = "intersection"|"difference", a = <face spec>, b = <face
            // spec>}`: a boolean-combined region of two other (possibly nested) faces
            // (#16/#62) — the toggleable intersection/difference regions of two overlapping
            // shapes.
            if let Some(boolean) = opts.get::<Option<Table>>("boolean")? {
                faces.push(parse_boolean_face_table(&boolean)?);
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
                _ => crate::actions::ExtrudeBodyChoice::New,
            };
            // Sketch from the first face's geometry (all faces should be coplanar).
            let sketch = unsafe {
                let doc = &tick.state().doc;
                crate::actions::extrude_face_sketch(doc, &faces[0])
            }
            .ok_or_else(|| mlua::Error::external("extrude face does not exist"))?;
            unsafe {
                tick.exec(Instruction::Extrude {
                    sketch,
                    faces,
                    distance,
                    body,
                })?;
            }
            let element = SceneElement::Extrusion(unsafe {
                tick.state().doc.extrusions.len().saturating_sub(1)
            });
            apply_optional_name(lua, element, Some(opts))
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
            let point = parse_constraint_point_table(point_table)?;
            let distance: f32 = opts.get("distance")?;
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
            let point = parse_constraint_point_table(point_table)?;
            let radius: f32 = opts.get("radius")?;
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
            let extrusion: usize = opts.get("extrusion")?;
            let edge_table: Table = opts.get("edge")?;
            let edge = parse_extrusion_edge_table(edge_table)?;
            let distance: f32 = opts.get("distance")?;
            unsafe {
                tick.exec(Instruction::EdgeTreatment {
                    extrusion,
                    edge,
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
            let extrusion: usize = opts.get("extrusion")?;
            let edge_table: Table = opts.get("edge")?;
            let edge = parse_extrusion_edge_table(edge_table)?;
            let radius: f32 = opts.get("radius")?;
            unsafe {
                tick.exec(Instruction::EdgeTreatment {
                    extrusion,
                    edge,
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
            "tool", "focus_name", "focus_dim", "pane", "palette",
            "orbit", "pan", "wheel", "set_home_view", "toggle_projection", "shading",
            "camera", "zoom_fit", "elements_view",
            "move", "click", "move_ground", "click_ground",
            "drag", "right_drag", "right_drag_pan", "drag_vertex", "drag_line",
            "key", "keydown", "keyup", "type",
            "_view", "_view_home", "_wait", "_wait_ms", "_screenshot",
        }
        for _, name in ipairs(ui_funcs) do
            bearcad.ui[name] = bearcad[name]
            bearcad[name] = nil
        end

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
    use super::*;
    use crate::actions::AppState;
    use crate::model::FaceId;

    fn run_lua(source: &str) -> AppState {
        let mut runner = ScriptRunner::from_lua_source(source).unwrap();
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        let vp = egui::Rect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(960.0, 560.0));
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, Some(vp), &ctx);
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
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, Some(vp), &ctx);
        }
        assert!(runner.error.is_none(), "script error: {:?}", runner.error);
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

    /// #46: GUI/UI manipulation lives under `bearcad.ui.*`; modeling stays top-level.
    #[test]
    fn lua_ui_functions_live_under_ui_namespace() {
        run_lua_expect_ok(
            r#"
            assert(bearcad.ui ~= nil, "bearcad.ui table missing")
            for _, name in ipairs({ "move", "click", "tool", "view", "orbit", "pan",
                                    "key", "type", "pane", "palette", "drag_vertex", "wait" }) do
                assert(type(bearcad.ui[name]) == "function", "bearcad.ui." .. name .. " missing")
                assert(bearcad[name] == nil, "bearcad." .. name .. " should move to bearcad.ui")
            end
            -- declarative modeling stays at the top level
            for _, name in ipairs({ "rect", "line", "circle", "extrude", "new", "select",
                                    "add_constraint", "parameter", "export_stl", "export_step",
                                    "import_stl", "import_step", "chamfer_vertex",
                                    "fillet_vertex", "chamfer_edge", "fillet_edge" }) do
                assert(type(bearcad[name]) == "function", "bearcad." .. name .. " should stay top-level")
            end
        "#,
        );
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
                .iter()
                .any(|c| !c.deleted && matches!(c.kind, crate::model::ConstraintKind::Equal { .. })),
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
            line: 0,
            end: LineEnd::End,
        });
        let start_point = crate::model::ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
            line: 1,
            end: LineEnd::Start,
        });
        assert!(
            state.doc.constraints.iter().any(|c| {
                !c.deleted
                    && matches!(
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
            Some(SceneElement::Point(ConstraintPoint::CircleCenter(0)))
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
        assert!((state.doc.lines[0].length() - 80.0).abs() < 1e-2);
        assert_eq!(
            find_element_by_name(&state.doc, "Guide"),
            Some(SceneElement::Line(0))
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
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state
            .doc
            .lines
            .push(Line::from_local_endpoints(sketch, 10.0, 0.0, b_far.0, b_far.1));
        state.doc.shape_order.extend([ShapeKind::Line, ShapeKind::Line]);
        state.doc.constraints.push(Constraint {
            sketch,
            kind: ConstraintKind::Coincident {
                a: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: 0,
                    end: LineEnd::End,
                }),
                b: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: 1,
                    end: LineEnd::Start,
                }),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
            deleted: false,
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
        assert_eq!(state.doc.lines.len(), 3, "a bridging line should be added");
        assert!(!state.doc.lines[2].is_curved(), "chamfer bridges with a straight line");
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
        assert_eq!(state.doc.lines.len(), 3, "a bridging line should be added");
        assert!(state.doc.lines[2].is_curved(), "fillet bridges with a curved line");
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
            assert(tostring(err):find("closed loop"), "unexpected error: " .. tostring(err))
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
        assert_eq!(state.doc.extrusions[0].edge_treatments.len(), 1);
        assert_eq!(
            state.doc.extrusions[0].edge_treatments[0].kind,
            VertexTreatmentKind::Chamfer
        );
        let mesh = crate::extrude::extrusion_mesh(&state.doc, &state.doc.extrusions[0]).unwrap();
        // An untreated 10x10x5 box extrusion is 12 triangles; the chamfer adds geometry.
        assert_ne!(mesh.triangles.len(), 12);
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
        assert_eq!(state.doc.extrusions[0].edge_treatments.len(), 1);
        assert_eq!(
            state.doc.extrusions[0].edge_treatments[0].kind,
            VertexTreatmentKind::Fillet
        );
        assert!(matches!(
            state.doc.extrusions[0].edge_treatments[0].edge,
            ExtrusionEdgeRef::Cap { face: 0, edge: 1, top: true }
        ));
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
            state.doc.extrusions[0].edge_treatments.is_empty(),
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
        let line = &state.doc.lines[0];
        assert!(line.is_curved());
        assert_eq!(line.bezier, Some([(3.0, 4.0), (7.0, 4.0)]));
        assert_eq!(
            find_element_by_name(&state.doc, "Curve"),
            Some(SceneElement::Line(0))
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
        let circle = &state.doc.circles[0];
        assert!((circle.cx - 10.0).abs() < 1e-3 && (circle.cy - 5.0).abs() < 1e-3);
        assert!((circle.r - 12.0).abs() < 1e-3);
        assert_eq!(
            find_element_by_name(&state.doc, "Hole"),
            Some(SceneElement::Circle(0))
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
        assert!((state.doc.circles[0].r - 15.0).abs() < 1e-3);
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
            state.doc.bodies[0].source,
            crate::model::BodySource::Imported(0)
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
            state.doc.bodies[0].source,
            crate::model::BodySource::Imported(0)
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
        assert_eq!(state.doc.extrusions[0].distance, 20.0);
        assert_eq!(
            find_element_by_name(&state.doc, "Boss"),
            Some(SceneElement::Extrusion(0))
        );
        // The extrusion produces a body that depends on it.
        assert_eq!(state.doc.bodies.len(), 1);
        assert_eq!(
            state.doc.bodies[0].source,
            crate::model::BodySource::Extrusion(0)
        );
        // Both appear as elements; the body nests under its extrusion.
        let nodes = crate::hierarchy::build_element_list(&state.doc, state.sketch_session);
        assert!(nodes.contains(&crate::hierarchy::HierarchyNode::Extrusion(0)));
        assert!(nodes.contains(&crate::hierarchy::HierarchyNode::Body(0)));
        let mesh =
            crate::extrude::extrusion_mesh(&state.doc, &state.doc.extrusions[0]).unwrap();
        assert_eq!(mesh.triangles.len(), 12);
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
            state.doc.extrusions[0].faces,
            vec![crate::model::ExtrudeFace::Polygon(vec![0, 1, 2])]
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
        assert_eq!(state.doc.bodies.len(), 1, "the second extrusion should join body 0");
        assert_eq!(state.doc.bodies[0].source.extrusion_indices(), [0, 1]);
    }

    #[test]
    fn lua_extrude_with_body_cut_subtracts_from_the_existing_body() {
        // `body = "cut"` (#35) records the new extrusion as a subtraction of the extruded
        // face's body rather than fusing it. The model records the cut in every build; the
        // geometry only performs it under `--features occt`.
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
        assert_eq!(state.doc.bodies[0].source.extrusion_indices(), [0]);
        assert_eq!(state.doc.bodies[0].source.cut_extrusion_indices(), [1]);
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
        crate::document_lifecycle::tombstone_element(
            &mut state.doc,
            SceneElement::Extrusion(0),
        );
        assert!(state.doc.extrusions[0].deleted);
        assert!(state.doc.bodies[0].deleted, "body should be removed with its extrusion");
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
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        state.doc.lines.push(crate::model::Line::from_local_endpoints(
            sketch, 0.0, 0.0, 10.0, 0.0,
        ));
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, None, &ctx);
        }
        assert_eq!(
            find_element_by_name(&state.doc, "Main box"),
            Some(SceneElement::Line(0))
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
        assert_eq!(state.doc.sketches[0].length_unit, Some(LengthUnit::Ft));
        assert_eq!(state.doc.sketches[0].angle_unit, None);

        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.begin_sketch("construction_plane", 0)
            bearcad.set_units{ sketch = 0, length = "ft" }
            bearcad.set_units{ sketch = 0 }
        "#,
        );
        assert_eq!(
            state.doc.sketches[0].length_unit, None,
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
            assert(bearcad.count("construction_plane") == 1)
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

    /// #108: `bearcad.ui.zoom_fit()` frames the document — the camera target lands on the
    /// body's bbox center, instantly (no transition).
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
        assert!(!state.cam.is_transitioning(), "zoom_fit applies instantly");
        assert!(state.cam.distance > 0.0 && state.cam.distance.is_finite());
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
}