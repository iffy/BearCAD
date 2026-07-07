//! Web (wasm32) Lua scripting (todoer #179/#207).
//!
//! The browser can't compile mlua's bundled Lua C for `wasm32-unknown-unknown`, so the Lua
//! interpreter ships as a *second* Emscripten module (mirroring the OCCT kernel — see
//! `cpp/bearcad_lua.cpp` and `scripts/build-lua-wasm.sh`) reached through the JS bridge
//! `web/lua-bridge.js`. This module is the app side: it runs a script with [`run_script`] and
//! services each `bearcad.*` call the script makes, re-entrantly, against the live
//! [`AppState`].
//!
//! Flow: `run_script` installs a dispatch callback on `globalThis.bearcadDispatch` and stashes
//! raw pointers to the live app state (the same re-entrancy trick as the native
//! `ScriptTickData`), then calls `lua_run`. The Lua module executes the whole script
//! synchronously; for every command it calls back into [`dispatch`], which routes the name +
//! JSON args through [`crate::script_json`] onto the shared Instruction/Action layer and
//! returns a JSON result (`{ok, value?}` or `{error}`).

use crate::actions::AppState;
use crate::hierarchy::SceneElement;
use crate::model::{Document, FaceId};
use crate::names::find_element_by_name;
use crate::script::{Instruction, ScriptRunner, SyntheticInput};
use crate::script_json;
use eframe::egui;
use serde_json::{json, Value};
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/web/lua-bridge.js")]
extern "C" {
    fn lua_available() -> bool;
    /// Runs a script; returns the Lua error message, or `None` on success.
    fn lua_run(src: &str) -> Option<String>;
}

/// Live pointers to the app, valid only for the duration of a synchronous [`lua_run`].
struct ScriptCtx {
    runner: *mut ScriptRunner,
    state: *mut AppState,
    synthetic: *mut SyntheticInput,
    viewport: Option<egui::Rect>,
    ctx: *const egui::Context,
}

thread_local! {
    static CTX: RefCell<Option<ScriptCtx>> = const { RefCell::new(None) };
    // Kept alive so `globalThis.bearcadDispatch` stays callable across runs.
    static DISPATCH: RefCell<Option<Closure<dyn Fn(String, String) -> String>>> =
        const { RefCell::new(None) };
}

/// Whether the Lua interpreter module loaded (mirrors `kernel::available` for scripting).
pub fn available() -> bool {
    lua_available()
}

/// Run a Lua `src` against the live app state. Returns the Lua error message on failure.
pub fn run_script(
    state: &mut AppState,
    synthetic: &mut SyntheticInput,
    viewport: Option<egui::Rect>,
    ctx: &egui::Context,
    src: &str,
) -> Result<(), String> {
    if !lua_available() {
        return Err("Lua interpreter module not loaded".to_string());
    }
    install_dispatch();

    // A bare runner is the execution engine (it applies instructions to `state`); no native
    // Lua runtime is involved on the web — the interpreter is the separate module.
    let mut runner = ScriptRunner::from_instructions(Vec::new());
    let cx = ScriptCtx {
        runner: &mut runner,
        state,
        synthetic,
        viewport,
        ctx,
    };
    CTX.with(|c| *c.borrow_mut() = Some(cx));
    let result = lua_run(src);
    CTX.with(|c| *c.borrow_mut() = None);

    match result {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// Install `globalThis.bearcadDispatch` once. The Lua module's C shim calls it (via EM_JS)
/// for every `bearcad.*` call.
fn install_dispatch() {
    DISPATCH.with(|d| {
        if d.borrow().is_some() {
            return;
        }
        let closure = Closure::wrap(
            Box::new(|name: String, args: String| dispatch(name, args))
                as Box<dyn Fn(String, String) -> String>,
        );
        let global = js_sys::global();
        let _ = js_sys::Reflect::set(
            &global,
            &JsValue::from_str("bearcadDispatch"),
            closure.as_ref().unchecked_ref(),
        );
        *d.borrow_mut() = Some(closure);
    });
}

/// Service one `bearcad.<name>(<args>)` call. Returns a JSON string: `{"ok":true[,"value":…]}`
/// or `{"error":"…"}`.
fn dispatch(name: String, args_json: String) -> String {
    CTX.with(|c| {
        let borrow = c.borrow();
        let Some(cx) = borrow.as_ref() else {
            return error_json("script context not active");
        };
        // SAFETY: the pointers are valid for the duration of `run_script`'s `lua_run`, which
        // is the only time `dispatch` is reachable (it's re-entered synchronously from it).
        let runner = unsafe { &mut *cx.runner };
        let state = unsafe { &mut *cx.state };
        let synthetic = unsafe { &mut *cx.synthetic };
        let egui_ctx = unsafe { &*cx.ctx };
        match run_command(&name, &args_json, runner, state, synthetic, cx.viewport, egui_ctx) {
            Ok(Value::Null) => json!({ "ok": true }).to_string(),
            Ok(value) => json!({ "ok": true, "value": value }).to_string(),
            Err(e) => error_json(&e),
        }
    })
}

fn run_command(
    name: &str,
    args_json: &str,
    runner: &mut ScriptRunner,
    state: &mut AppState,
    synthetic: &mut SyntheticInput,
    viewport: Option<egui::Rect>,
    ctx: &egui::Context,
) -> Result<Value, String> {
    let mut args: Value = if args_json.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(args_json).map_err(|e| format!("bad arguments: {e}"))?
    };

    // `parameter(action, ...)` has an action-dependent positional shape, so it's handled
    // before the generic positional adaptor. `add`/`from_line_length` are actions; `get`/
    // `get_expression` are reads.
    if name == "parameter" {
        return run_parameter(&args, runner, state, synthetic, viewport, ctx);
    }

    // Reads and actions that need `AppState` beyond the document (or take a positional
    // sketch arg), handled before the generic positional adaptor.
    match name {
        "status" => return Ok(json!(state.status)),
        "selection" => return Ok(selection_json(state)),
        "sketch_dof" => {
            let sketch = arg_sketch(&args, state)?;
            return crate::constraints::sketch_degrees_of_freedom(&state.doc, sketch).map(|d| json!(d));
        }
        "sketch_conflicts" => {
            let sketch = arg_sketch(&args, state)?;
            return crate::constraints::sketch_conflicting_constraints(&state.doc, sketch)
                .map(|v| json!(v));
        }
        "set_units" => {
            let instr = set_units_instruction(&args, &state.doc)?;
            exec(runner, instr, state, synthetic, viewport, ctx)?;
            return Ok(Value::Null);
        }
        _ => {}
    }

    // Positional calls arrive as `{ "__args": [...] }`; map them to named arguments.
    if let Some(arr) = args.get("__args").and_then(Value::as_array).cloned() {
        args = script_json::positional_to_named(name, &arr)?;
    }

    // Read-back queries return data instead of an instruction.
    if matches!(name, "count" | "get" | "body_stats") {
        return script_json::query_from_json(name, &args, &state.doc);
    }

    // The extrude verbs read the live document (sketch inference, current depth), so they
    // build their instruction from the doc rather than through instruction_from_json.
    if matches!(name, "extrude" | "extrude_face" | "edit_extrusion") {
        let instr = script_json::extrude_instruction(name, &args, &state.doc)?;
        exec(runner, instr, state, synthetic, viewport, ctx)?;
        return Ok(Value::Null);
    }

    // `find` resolves a name to an element handle `{ kind, index }` (or null).
    if name == "find" {
        let query = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or("find requires a `name`")?;
        return Ok(match find_element_by_name(&state.doc, query) {
            Some(el) => match script_json::scene_element_kind_name(&el) {
                Some((kind, index)) => json!({ "kind": kind, "index": index }),
                None => Value::Null,
            },
            None => Value::Null,
        });
    }

    // Element-referencing verbs resolve their `element` argument against the live document
    // (by name or `{kind, index}`), which instruction_from_json can't do on its own.
    if matches!(name, "select" | "set_name" | "set_visible" | "set_construction") {
        let element = resolve_element(
            args.get("element").ok_or_else(|| format!("{name} requires an `element`"))?,
            &state.doc,
        )?;
        let instr = match name {
            "select" => Instruction::SelectSceneElement {
                element,
                additive: args.get("additive").and_then(Value::as_bool).unwrap_or(false),
            },
            "set_name" => Instruction::SetElementName {
                element,
                name: args
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("set_name requires a `name`")?
                    .to_string(),
            },
            "set_visible" => Instruction::SetElementVisible {
                element,
                visible: parse_visible(args.get("visible")),
            },
            "set_construction" => Instruction::SetShapeConstruction {
                element,
                construction: args
                    .get("construction")
                    .and_then(Value::as_bool)
                    .ok_or("set_construction requires a boolean `construction`")?,
            },
            _ => unreachable!(),
        };
        exec(runner, instr, state, synthetic, viewport, ctx)?;
        return Ok(Value::Null);
    }

    // Sketch primitives auto-open a sketch on the ground plane when none is active, exactly
    // as the desktop `rect`/`line`/`circle` closures do.
    if script_json::opens_sketch_when_none_active(name) && state.sketch_session.is_none() {
        exec(
            runner,
            Instruction::BeginSketch { face: FaceId::ConstructionPlane(0) },
            state,
            synthetic,
            viewport,
            ctx,
        )?;
    }

    let instr = script_json::instruction_from_json(name, &args)?;
    exec(runner, instr, state, synthetic, viewport, ctx)?;
    Ok(Value::Null)
}

/// Execute one instruction, surfacing an action rejection as an error (the web analogue of
/// `ScriptTickData::exec` raising `last_action_error` as a Lua error).
fn exec(
    runner: &mut ScriptRunner,
    instr: Instruction,
    state: &mut AppState,
    synthetic: &mut SyntheticInput,
    viewport: Option<egui::Rect>,
    ctx: &egui::Context,
) -> Result<(), String> {
    runner.last_action_error = None;
    let _ = runner.execute_instruction(instr, state, synthetic, viewport, ctx);
    match runner.last_action_error.take() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// `bearcad.parameter(action, ...)`: `add`/`from_line_length` are actions; `get`/
/// `get_expression` read the document. Mirrors the desktop `parameter` closure.
fn run_parameter(
    args: &Value,
    runner: &mut ScriptRunner,
    state: &mut AppState,
    synthetic: &mut SyntheticInput,
    viewport: Option<egui::Rect>,
    ctx: &egui::Context,
) -> Result<Value, String> {
    let a = args.get("__args").and_then(Value::as_array).cloned().unwrap_or_default();
    let action = a.first().and_then(Value::as_str).ok_or("parameter requires an action")?;
    match action {
        "add" => {
            let name = a
                .get(1)
                .and_then(Value::as_str)
                .ok_or("parameter add requires a name")?
                .to_string();
            let expression = a
                .get(2)
                .and_then(value_to_string)
                .ok_or("parameter add requires an expression")?;
            exec(
                runner,
                Instruction::AddParameter { name, expression },
                state,
                synthetic,
                viewport,
                ctx,
            )?;
            Ok(Value::Null)
        }
        "from_line_length" => {
            let line_index = a
                .get(1)
                .and_then(Value::as_u64)
                .ok_or("parameter from_line_length requires a line index")? as usize;
            let name = a.get(2).and_then(Value::as_str).map(str::to_string);
            exec(
                runner,
                Instruction::CreateParameterFromLineLength { line_index, name },
                state,
                synthetic,
                viewport,
                ctx,
            )?;
            Ok(Value::Null)
        }
        "get" | "get_expression" => {
            let name = a
                .get(1)
                .and_then(Value::as_str)
                .ok_or("parameter get requires a parameter name")?;
            let Some(param) = state.doc.parameters.iter().find(|p| !p.deleted && p.name == name)
            else {
                return Ok(Value::Null);
            };
            if action == "get_expression" {
                return Ok(json!(param.expression));
            }
            Ok(
                match crate::value::eval_parameter_in_doc(&param.expression, &state.doc) {
                    Some(crate::value::EvaluatedParameter::LengthMm(v))
                    | Some(crate::value::EvaluatedParameter::AngleRad(v)) => json!(v),
                    None => Value::Null,
                },
            )
        }
        other => Err(format!("unknown parameter action '{other}'")),
    }
}

/// The `selection` query: the live scene selection as an array of `{ kind, index? }`.
fn selection_json(state: &AppState) -> Value {
    let items: Vec<Value> = state
        .scene_selection
        .iter()
        .map(|el| {
            let mut m = serde_json::Map::new();
            m.insert("kind".into(), json!(script_json::scene_element_full_kind_name(&el)));
            if let Some(index) = script_json::scene_element_selection_index(&el) {
                m.insert("index".into(), json!(index));
            }
            Value::Object(m)
        })
        .collect();
    json!(items)
}

/// The sketch a `sketch_dof`/`sketch_conflicts` call targets: an explicit index (positional
/// `__args[0]` or a `sketch` field) or the active sketch session.
fn arg_sketch(args: &Value, state: &AppState) -> Result<usize, String> {
    let explicit = args
        .get("__args")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_u64)
        .or_else(|| args.get("sketch").and_then(Value::as_u64));
    match explicit {
        Some(n) => Ok(n as usize),
        None => state
            .sketch_session
            .map(|s| s.sketch)
            .ok_or_else(|| "no active sketch".to_string()),
    }
}

/// Build a `set_units` instruction: a per-sketch override when `sketch` is given, else the
/// document default (unset fields keep the current document value).
fn set_units_instruction(args: &Value, doc: &Document) -> Result<Instruction, String> {
    let o = args.as_object().ok_or("set_units expects a table")?;
    let length = match o.get("length").and_then(Value::as_str) {
        Some(n) => Some(
            crate::value::LengthUnit::from_name(n)
                .ok_or_else(|| format!("unknown length unit '{n}'"))?,
        ),
        None => None,
    };
    let angle = match o.get("angle").and_then(Value::as_str) {
        Some(n) => Some(
            crate::value::AngleUnit::from_name(n)
                .ok_or_else(|| format!("unknown angle unit '{n}'"))?,
        ),
        None => None,
    };
    if let Some(sketch) = o.get("sketch").and_then(Value::as_u64) {
        Ok(Instruction::SetSketchUnits { sketch: sketch as usize, length, angle })
    } else {
        Ok(Instruction::SetDocumentUnits {
            length: length.unwrap_or(doc.default_length_unit),
            angle: angle.unwrap_or(doc.default_angle_unit),
        })
    }
}

/// An expression `Value` (string or number) as a string, for a parameter's expression.
fn value_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Resolve an element argument (a name string, `{ name }`, or `{ kind, index }`) to a
/// `SceneElement` against the live document — the web analogue of `lua_script::resolve_element`
/// for whole elements.
fn resolve_element(v: &Value, doc: &Document) -> Result<SceneElement, String> {
    match v {
        Value::String(name) => {
            find_element_by_name(doc, name).ok_or_else(|| format!("no element named '{name}'"))
        }
        Value::Object(o) => {
            if let Some(name) = o.get("name").and_then(Value::as_str) {
                return find_element_by_name(doc, name)
                    .ok_or_else(|| format!("no element named '{name}'"));
            }
            let kind = o
                .get("kind")
                .and_then(Value::as_str)
                .ok_or("element requires a `kind` or `name`")?;
            let index = o
                .get("index")
                .and_then(Value::as_u64)
                .ok_or("element requires an `index`")? as usize;
            script_json::scene_element_from_kind(kind, index)
                .ok_or_else(|| format!("unknown element kind '{kind}'"))
        }
        _ => Err("expected an element (name string or {kind, index})".to_string()),
    }
}

/// A `visible` argument → `Some(true|false)` (show/hide) or `None` (toggle).
fn parse_visible(v: Option<&Value>) -> Option<bool> {
    match v {
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::String(s)) => match s.to_ascii_lowercase().as_str() {
            "show" | "on" | "true" | "yes" | "1" => Some(true),
            "hide" | "off" | "false" | "no" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn error_json(msg: &str) -> String {
    json!({ "error": msg }).to_string()
}
