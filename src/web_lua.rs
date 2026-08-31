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

    // Name-first parameter verbs (#1867): mix of actions, reads, and pane state, so they
    // sit here rather than going through the generic instruction adaptor.
    if matches!(
        name,
        "add_parameter"
            | "set_parameter"
            | "parameter_value"
            | "parameter_expression"
            | "delete_parameter"
            | "edit_parameter"
            | "parameter_from_line_length"
            | "parameter_options"
            | "parameter_edit"
            | "parameter_editing"
            | "parameter_slider"
    ) {
        return run_parameter_verb(name, &args, runner, state, synthetic, viewport, ctx);
    }

    // Reads and actions that need `AppState` beyond the document (or take a positional
    // sketch arg), handled before the generic positional adaptor.
    match name {
        "status" => return Ok(json!(state.status)),
        "version" => return Ok(json!(crate::full_version())),
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

    // GUI input-simulation, semantic-gizmo drags, and path imports assume the native
    // frame-by-frame runner (or a host filesystem); web Load Script runs a whole script
    // synchronously in one frame, so these are deliberately unavailable here (#209).
    if is_native_only_verb(name) {
        return Err(format!(
            "'{name}' isn't available in browser scripting (GUI-simulation / import verbs run \
             only in the desktop app)"
        ));
    }

    // Positional calls arrive as `{ "__args": [...] }`; map them to named arguments.
    if let Some(arr) = args.get("__args").and_then(Value::as_array).cloned() {
        args = script_json::positional_to_named(name, &arr)?;
    }

    // Stable element ids (#1801) reach the browser as strings — there is no userdata across
    // the JSON bridge — so rewrite any operand spelled as an id into the ordinal the
    // instruction layer wants, before anything reads the arguments.
    resolve_id_operands(&mut args, &state.doc)?;

    // `bearcad.id(element)` reads an element's stable id back (#1801).
    if name == "id" {
        let element = resolve_element(
            args.get("element").ok_or("id requires an `element`")?,
            &state.doc,
        )?;
        return crate::hierarchy::element_id(&element)
            .map(Value::String)
            .ok_or_else(|| "that reference has no id of its own".to_string());
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

    // `visible` reads an element's effective visibility back (#1800). It lives here rather
    // than in `query_from_json` because the flags hang off the app state, not the document.
    if name == "visible" {
        let element = resolve_element(
            args.get("element").ok_or("visible requires an `element`")?,
            &state.doc,
        )?;
        return Ok(Value::Bool(
            state.element_visibility.effective_visible(&state.doc, element),
        ));
    }

    // `find` resolves a name to an element handle `{ kind, index }` (or null).
    if name == "find" {
        let query = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or("find requires a `name`")?;
        return Ok(match find_element_by_name(&state.doc, query) {
            Some(el) => match script_json::scene_element_kind_name(&state.doc, &el) {
                Some((kind, index)) => json!({ "kind": kind, "index": index }),
                None => Value::Null,
            },
            None => Value::Null,
        });
    }

    // Element-referencing verbs resolve their `element` argument against the live document
    // (by name or `{kind, index}`), which instruction_from_json can't do on its own.
    // `set_visible{ kind = … }` with no index sweeps every element of that kind (#1800) —
    // the same bulk form the desktop API takes.
    if name == "set_visible" {
        if let Some(kind) = args.get("element").and_then(kind_only_selector) {
            let visible = parse_visible(args.get("visible"));
            let mut elements = Vec::new();
            while let Some(element) =
                script_json::scene_element_from_kind(&state.doc, &kind, elements.len())
            {
                elements.push(element);
            }
            if elements.is_empty() {
                return Err(format!(
                    "set_visible: no '{kind}' elements — unknown kind, or none in the document"
                ));
            }
            for element in elements {
                exec(
                    runner,
                    Instruction::SetElementVisible { element, visible },
                    state,
                    synthetic,
                    viewport,
                    ctx,
                )?;
            }
            return Ok(Value::Null);
        }
    }
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
            Instruction::BeginSketch {
                face: FaceId::ConstructionPlane(
                    state.doc.ground_plane().ok_or("no ground plane")?,
                ),
            },
            state,
            synthetic,
            viewport,
            ctx,
        )?;
    }

    let instr = script_json::instruction_from_json(&state.doc, name, &args)?;
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

/// Name-first parameter verbs (#1867). Mirrors the desktop closures.
fn run_parameter_verb(
    name: &str,
    args: &Value,
    runner: &mut ScriptRunner,
    state: &mut AppState,
    synthetic: &mut SyntheticInput,
    viewport: Option<egui::Rect>,
    ctx: &egui::Context,
) -> Result<Value, String> {
    let a = args.get("__args").and_then(Value::as_array);
    match name {
        "add_parameter" => {
            let name = a
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                .or_else(|| args.get("name").and_then(Value::as_str))
                .ok_or("add_parameter requires a name")?
                .to_string();
            let expression = a
                .and_then(|a| a.get(1))
                .or_else(|| args.get("expression"))
                .and_then(value_to_string)
                .ok_or("add_parameter requires an expression")?;
            exec(
                runner,
                Instruction::AddParameter {
                    name: name.clone(),
                    expression,
                },
                state,
                synthetic,
                viewport,
                ctx,
            )?;
            let index = state
                .doc
                .parameters
                .values()
                .position(|p| p.name == name)
                .unwrap_or(state.doc.parameters.len().saturating_sub(1));
            Ok(json!({ "kind": "parameter", "index": index }))
        }
        "set_parameter" => {
            let target = a.and_then(|a| a.first()).or_else(|| args.get("name"));
            let index = json_parameter_index(target, &state.doc, "set_parameter")?
                .ok_or_else(|| "set_parameter: no such parameter".to_string())?;
            let expression = a
                .and_then(|a| a.get(1))
                .or_else(|| args.get("expression"))
                .and_then(value_to_string)
                .ok_or("set_parameter requires an expression")?;
            exec(
                runner,
                Instruction::SetParameterExpression { index, expression },
                state,
                synthetic,
                viewport,
                ctx,
            )?;
            Ok(Value::Null)
        }
        "parameter_value" | "parameter_expression" => {
            let target = a.and_then(|a| a.first()).or_else(|| args.get("name"));
            let Some(index) = json_parameter_index(target, &state.doc, name)? else {
                return Ok(Value::Null);
            };
            let Some(param) = state.doc.parameters.values().nth(index) else {
                return Ok(Value::Null);
            };
            if name == "parameter_expression" {
                return Ok(json!(param.expression));
            }
            Ok(
                match crate::value::eval_parameter_in_doc(&param.expression, &state.doc) {
                    Some(crate::value::EvaluatedParameter::LengthMm(v)) => json!(v),
                    Some(crate::value::EvaluatedParameter::AngleRad(v)) => json!(v.to_degrees()),
                    None => Value::Null,
                },
            )
        }
        "delete_parameter" => {
            let target = a.and_then(|a| a.first()).or_else(|| args.get("name"));
            let index = json_parameter_index(target, &state.doc, "delete_parameter")?
                .ok_or_else(|| "delete_parameter: no such parameter".to_string())?;
            exec(
                runner,
                Instruction::DeleteParameter { index },
                state,
                synthetic,
                viewport,
                ctx,
            )?;
            Ok(Value::Null)
        }
        "edit_parameter" => {
            let o = args.as_object().ok_or("edit_parameter requires a table")?;
            let target = o.get("name").ok_or("edit_parameter requires a `name`")?;
            let index = json_parameter_index(Some(target), &state.doc, "edit_parameter")?
                .ok_or_else(|| "edit_parameter: no such parameter".to_string())?;
            if let Some(rename) = o.get("rename").and_then(Value::as_str) {
                exec(
                    runner,
                    Instruction::SetParameterName {
                        index,
                        name: rename.to_string(),
                    },
                    state,
                    synthetic,
                    viewport,
                    ctx,
                )?;
            }
            if let Some(private) = o.get("private").and_then(Value::as_bool) {
                exec(
                    runner,
                    Instruction::SetParameterPrimary {
                        index,
                        primary: !private,
                    },
                    state,
                    synthetic,
                    viewport,
                    ctx,
                )?;
            }
            for (key, which) in [
                ("min", crate::parameters::ParameterBound::Minimum),
                ("max", crate::parameters::ParameterBound::Maximum),
                ("step", crate::parameters::ParameterBound::Step),
            ] {
                if let Some(v) = o.get(key) {
                    let expression = json_bound_expression(v, key)?;
                    exec(
                        runner,
                        Instruction::SetParameterBound {
                            index,
                            which,
                            expression,
                        },
                        state,
                        synthetic,
                        viewport,
                        ctx,
                    )?;
                }
            }
            Ok(Value::Null)
        }
        "parameter_from_line_length" => {
            let line_index = a
                .and_then(|a| a.first())
                .and_then(Value::as_u64)
                .or_else(|| args.get("line").and_then(Value::as_u64))
                .ok_or("parameter_from_line_length requires a line")? as usize;
            let pname = a
                .and_then(|a| a.get(1))
                .and_then(Value::as_str)
                .or_else(|| args.get("name").and_then(Value::as_str))
                .map(str::to_string);
            exec(
                runner,
                Instruction::CreateParameterFromLineLength {
                    line_index,
                    name: pname.clone(),
                },
                state,
                synthetic,
                viewport,
                ctx,
            )?;
            let index = pname
                .and_then(|n| state.doc.parameters.values().position(|p| p.name == n))
                .unwrap_or(state.doc.parameters.len().saturating_sub(1));
            Ok(json!({ "kind": "parameter", "index": index }))
        }
        "parameter_options" => {
            let target = a.and_then(|a| a.first()).or_else(|| args.get("name"));
            let index = json_parameter_index(target, &state.doc, "parameter_options")?
                .ok_or_else(|| "parameter_options: no such parameter".to_string())?;
            let Some(key) = state.doc.parameters.keys().nth(index) else {
                return Err(format!("Parameter {index} not found"));
            };
            match a.and_then(|a| a.get(1)).or_else(|| args.get("open")) {
                None | Some(Value::Null) => {
                    Ok(json!(state.parameters_pane.options_open.contains(&key)))
                }
                Some(Value::Bool(open)) => {
                    if *open {
                        state.parameters_pane.options_open.insert(key);
                    } else {
                        state.parameters_pane.options_open.remove(&key);
                        if state
                            .parameters_pane
                            .options_editing
                            .is_some_and(|(k, _)| k == key)
                        {
                            state.parameters_pane.options_editing = None;
                            state.parameters_pane.options_draft.clear();
                        }
                    }
                    Ok(Value::Null)
                }
                _ => Err("parameter_options open flag must be true/false".into()),
            }
        }
        "parameter_edit" => {
            let target = a.and_then(|a| a.first()).or_else(|| args.get("name"));
            let index = json_parameter_index(target, &state.doc, "parameter_edit")?
                .ok_or_else(|| "parameter_edit: no such parameter".to_string())?;
            let field = a
                .and_then(|a| a.get(1))
                .and_then(Value::as_str)
                .or_else(|| args.get("field").and_then(Value::as_str))
                .ok_or("parameter_edit requires \"min\", \"max\", or \"step\"")?;
            let which = crate::parameters::ParameterBound::from_name(field)
                .ok_or("parameter_edit field must be \"min\", \"max\", or \"step\"")?;
            let Some(key) = state.doc.parameters.keys().nth(index) else {
                return Err(format!("Parameter {index} not found"));
            };
            let current = crate::parameters::bound_expression(&state.doc.parameters[key], which)
                .unwrap_or("")
                .to_string();
            state
                .parameters_pane
                .begin_options_edit(key, which, &current);
            Ok(Value::Null)
        }
        "parameter_editing" => {
            let Some((key, which)) = state.parameters_pane.options_editing else {
                return Ok(Value::Null);
            };
            let Some(index) = state.doc.parameters.keys().position(|k| k == key) else {
                return Ok(Value::Null);
            };
            Ok(json!({
                "index": index,
                "name": state.doc.parameters[key].name,
                "field": which.script_name(),
            }))
        }
        "parameter_slider" => {
            let target = a.and_then(|a| a.first()).or_else(|| args.get("name"));
            let Some(index) = json_parameter_index(target, &state.doc, "parameter_slider")? else {
                return Ok(Value::Null);
            };
            let spec = state.doc.parameters.keys().nth(index).and_then(|key| {
                crate::parameters::parameter_slider_spec(&state.doc, &state.doc.parameters[key])
            });
            let Some(spec) = spec else {
                return Ok(Value::Null);
            };
            match a.and_then(|a| a.get(1)).or_else(|| args.get("value")) {
                None | Some(Value::Null) => {
                    let mut t = serde_json::Map::new();
                    t.insert("min".into(), json!(spec.min));
                    t.insert("max".into(), json!(spec.max));
                    t.insert("value".into(), json!(spec.current));
                    if let Some(step) = spec.limits.step {
                        t.insert("step".into(), json!(step));
                    }
                    Ok(Value::Object(t))
                }
                Some(v) => {
                    let slider_v = v
                        .as_f64()
                        .ok_or("parameter_slider value must be a number (mm / degrees)")?
                        as f32;
                    let key = state.doc.parameters.keys().nth(index).unwrap();
                    let expression = crate::parameters::parameter_slider_expression(
                        &state.doc,
                        &state.doc.parameters[key],
                        slider_v,
                    )
                    .ok_or("parameter has no slider")?;
                    exec(
                        runner,
                        Instruction::SetParameterExpression { index, expression },
                        state,
                        synthetic,
                        viewport,
                        ctx,
                    )?;
                    Ok(Value::Null)
                }
            }
        }
        other => Err(format!("unknown parameter call '{other}'")),
    }
}

fn json_parameter_index(
    value: Option<&Value>,
    doc: &Document,
    what: &str,
) -> Result<Option<usize>, String> {
    let Some(value) = value else {
        return Err(format!("{what} requires a parameter name or handle"));
    };
    match value {
        Value::Number(_) => Err(format!(
            "{what} takes a parameter name or handle, not an ordinal"
        )),
        Value::String(name) => {
            if let Some(element) = crate::hierarchy::element_from_id(doc, name) {
                return match element {
                    SceneElement::Parameter(_) => Ok(crate::hierarchy::element_live_index(
                        doc, &element,
                    )),
                    other => Err(format!(
                        "{what} expected a parameter, got {}",
                        script_json::scene_element_full_kind_name(&other)
                    )),
                };
            }
            Ok(doc.parameters.values().position(|p| p.name == name))
        }
        Value::Object(_) => {
            let element = resolve_element(value, doc)?;
            match element {
                SceneElement::Parameter(_) => Ok(crate::hierarchy::element_live_index(doc, &element)),
                other => Err(format!(
                    "{what} expected a parameter, got {}",
                    script_json::scene_element_full_kind_name(&other)
                )),
            }
        }
        _ => Err(format!("{what} takes a parameter name or handle")),
    }
}

fn json_bound_expression(v: &Value, which: &str) -> Result<Option<String>, String> {
    match v {
        Value::Null | Value::Bool(false) => Ok(None),
        Value::String(s) if s.trim().is_empty() => Ok(None),
        Value::String(s) => Ok(Some(s.clone())),
        Value::Number(n) => Ok(Some(n.to_string())),
        _ => Err(format!(
            "edit_parameter `{which}` must be an expression string, or false to clear"
        )),
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
            if let Some(index) = script_json::scene_element_selection_index(&state.doc, &el) {
                m.insert("index".into(), json!(index));
            }
            Value::Object(m)
        })
        .collect();
    json!(items)
}

/// The sketch a `sketch_dof`/`sketch_conflicts` call targets: an explicit index (positional
/// `__args[0]` or a `sketch` field) or the active sketch session.
fn arg_sketch(args: &Value, state: &AppState) -> Result<crate::model::SketchId, String> {
    let explicit = args
        .get("__args")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_u64)
        .or_else(|| args.get("sketch").and_then(Value::as_u64));
    match explicit {
        // A script names a sketch by its ordinal among the live ones (#1055).
        Some(n) => state
            .doc
            .sketches
            .keys()
            .nth(n as usize)
            .ok_or_else(|| format!("no sketch {n}")),
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
/// Operand keys that name an element by ordinal — the ones a stable id may stand in for
/// (#1801). Mirrors the desktop `Operands` reads; a `name` or a `path` is left alone.
const ID_OPERAND_KEYS: &[&str] = &[
    "index", "drawing", "revolution", "primitive", "extrusion", "body", "body_b", "sketch",
    "sketches", "plane", "line", "lines", "circle", "circles", "polygon", "text", "image",
    "component", "cross_section", "joint", "bodies", "images", "cutters", "path", "cuts",
    "from", "parent", "unit", "profile_lines", "view", "a", "b", "repeat_op",
];

/// Rewrite every id-spelled operand in a call's arguments into the element's current ordinal
/// (#1801). Recurses into nested objects and arrays, since operands like `cutters` hold
/// element tables of their own.
fn resolve_id_operands(args: &mut Value, doc: &Document) -> Result<(), String> {
    match args {
        Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                if ID_OPERAND_KEYS.contains(&key.as_str()) {
                    resolve_ids_in_place(value, doc)?;
                }
                resolve_id_operands(value, doc)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                resolve_id_operands(item, doc)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Turn an id string — or a list of them — into the ordinal(s) it names. Anything that isn't
/// an id is left exactly as it was, so names and numbers keep working.
fn resolve_ids_in_place(value: &mut Value, doc: &Document) -> Result<(), String> {
    match value {
        Value::String(id) => {
            let Some(element) = crate::hierarchy::element_from_id(doc, id) else {
                return Ok(());
            };
            let ordinal = crate::hierarchy::element_live_index(doc, &element)
                .ok_or_else(|| format!("{id} no longer exists"))?;
            *value = Value::from(ordinal);
        }
        Value::Array(items) => {
            for item in items {
                resolve_ids_in_place(item, doc)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// The kind a bare `{ kind = … }` element selector sweeps (#1800) — the web mirror of
/// `lua_script::kind_only_selector`. `None` when the object names one specific element.
fn kind_only_selector(v: &Value) -> Option<String> {
    let o = v.as_object()?;
    let kind = o.get("kind").or_else(|| o.get("type"))?.as_str()?;
    let names_one = [
        "index", "name", "endpoint", "end", "corner", "anchor", "point", "edge", "face", "axis",
        "drawing", "view",
    ]
    .iter()
    .any(|key| o.contains_key(*key));
    if names_one || kind.eq_ignore_ascii_case("origin") {
        return None;
    }
    Some(kind.to_string())
}

/// A whole `SceneElement` from a JSON element argument, resolved against the live document —
/// the web analogue of `lua_script::resolve_element`.
fn resolve_element(v: &Value, doc: &Document) -> Result<SceneElement, String> {
    match v {
        // An id first (#1801), then a name — the two can never collide.
        Value::String(name) => crate::hierarchy::element_from_id(doc, name)
            .or_else(|| find_element_by_name(doc, name))
            .ok_or_else(|| format!("no element named '{name}'")),
        Value::Object(o) => {
            if let Some(name) = o.get("name").and_then(Value::as_str) {
                return find_element_by_name(doc, name)
                    .ok_or_else(|| format!("no element named '{name}'"));
            }
            let kind = o
                .get("kind")
                .and_then(Value::as_str)
                .ok_or("element requires a `kind` or `name`")?;
            // A drawing's page item (#1747): a projection, a text annotation, or a shown
            // dimension, keyed by the drawing ordinal the other drawing verbs take.
            match kind.to_ascii_lowercase().as_str() {
                "projection" | "annotation" | "dimension" | "drawing_dimension" => {
                    return drawing_element_from_json(o, kind, doc)
                }
                _ => {}
            }            let index = o
                .get("index")
                .and_then(Value::as_u64)
                .ok_or("element requires an `index`")? as usize;
            script_json::scene_element_from_kind(doc, kind, index)
                .ok_or_else(|| format!("unknown element kind '{kind}'"))
        }
        _ => Err("expected an element (name string or {kind, index})".to_string()),
    }
}

/// A drawing's page item from a `{ kind, drawing, … }` object (#1747) — the web mirror of
/// `lua_script::parse_drawing_element_table`.
fn drawing_element_from_json(
    o: &serde_json::Map<String, Value>,
    kind: &str,
    doc: &Document,
) -> Result<SceneElement, String> {
    use crate::context::DrawingElementRef as D;
    let drawing_ordinal = o
        .get("drawing")
        .and_then(Value::as_u64)
        .ok_or("a drawing element requires a `drawing`")? as usize;
    let key = doc.drawings.keys().nth(drawing_ordinal)
        .ok_or_else(|| format!("no drawing {drawing_ordinal}"))?;
    let element = match kind.to_ascii_lowercase().as_str() {
        "projection" => D::Projection(
            o.get("view")
                .or_else(|| o.get("index"))
                .and_then(Value::as_u64)
                .ok_or("a projection requires a `view`")? as usize,
        ),
        "annotation" => {
            let ordinal =
                o.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            D::Text(
                doc.drawings
                    .get(key)
                    .and_then(|d| d.annotations.keys().nth(ordinal))
                    .ok_or_else(|| format!("no annotation {ordinal} in drawing {drawing_ordinal}"))?,
            )
        }
        _ => {
            if o.contains_key("a") || o.contains_key("b") {
                let q = |k: &str| -> Result<[i32; 3], String> {
                    let p = o
                        .get(k)
                        .and_then(|v| v.as_array())
                        .ok_or_else(|| format!("dimension `{k}` must be a {{x, y, z}} point"))?;
                    let n = |i: usize| p.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                    Ok(crate::hierarchy::quantize_body_point(glam::Vec3::new(
                        n(0), n(1), n(2),
                    )))
                };
                D::Dimension {
                    view: o.get("view").and_then(Value::as_u64).unwrap_or(0) as usize,
                    a: q("a")?,
                    b: q("b")?,
                }
            } else {
                D::PointDim {
                    view: o.get("view").and_then(Value::as_u64).unwrap_or(0) as usize,
                    index: o.get("index").and_then(Value::as_u64).unwrap_or(0) as usize,
                }
            }
        }
    };
    Ok(SceneElement::DrawingElement { drawing: key, element })
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

/// Verbs that only make sense in the desktop app: synthetic GUI input (which needs the native
/// frame-by-frame runner to sequence across frames), the frame-pump/screenshot verbs, the
/// semantic-gizmo drags, and path-based import (#209).
fn is_native_only_verb(name: &str) -> bool {
    matches!(
        name,
        "move"
            | "click"
            | "move_ground"
            | "click_ground"
            | "drag_world"
            | "drag"
            | "right_drag"
            | "right_drag_pan"
            | "key"
            | "keydown"
            | "keyup"
            | "type"
            | "wait"
            | "wait_ms"
            | "screenshot"
            | "drag_vertex"
            | "drag_line"
            | "import"
    )
}

fn error_json(msg: &str) -> String {
    json!({ "error": msg }).to_string()
}
