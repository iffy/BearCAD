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

use crate::model::FaceId;
use crate::script::Instruction;
use serde_json::{Map, Value};

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
/// Coverage is the sketch-building core (document/tool actions and 2D primitives); the
/// heavier modeling verbs (extrude/revolve/loft/booleans/move/repeat/slice, drawings) are
/// tracked separately and land on top of this same mechanism.
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
        other => Err(format!("unknown command '{other}'")),
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
}
