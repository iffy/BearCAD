//! The BearCAD Lua API, as one plaintext page (#1623/#1635).
//!
//! An agent that only reads prose invents calls that do not exist.
//! This catalog is the real API: signatures for the modeling verbs, then **every** registered
//! function, so a name that is not here is not a function.
//!
//! **One source, three consumers.** `bearcad api` prints it; the checked-in copy at
//! `docs-site/static/bearcad-api.md` is what the website serves at `/bearcad-api.md`, which
//! is where `bearcad-skill.md` sends an agent that needs the whole surface; and a test keeps
//! the copy honest. The Every-function list is harvested from `register_api` (accepted keys
//! + positional args) so it cannot drift from the live Lua table (#1884). Regenerate it with:
//!
//! ```sh
//! cargo run -- api > docs-site/static/bearcad-api.md
//! ```

use std::sync::OnceLock;

/// The whole reference, guide first and the exhaustive function list last.
pub fn document() -> &'static str {
    DOCUMENT.get_or_init(build).as_str()
}

static DOCUMENT: OnceLock<String> = OnceLock::new();

/// Modeling reference: names, keys, and the few rules that stop a script from failing.
/// Compact on purpose — an agent reads all of it before writing a line.
const GUIDE: &str = r#"# BearCAD Lua API

Use only the functions listed here. Unknown option keys fail the call and name the
accepted ones. Prefer `bearcad.*` (declarative modeling) over `bearcad.ui.*`.

Every dimension is a ValueInput, the same field the app gives you: pass a number, or an
expression string that may name parameters, units and arithmetic — `"leg"`, `"leg / 2 + 3mm"`,
`"45deg"`, `"1.2rad"`, `"5in"`. An expression stays live, so editing the parameter moves the
geometry. A bare number is millimetres (degrees for an angle) — the canonical units every
read-back reports; a bare number *inside* a string follows the document's default unit, so
`"1.5"` is 1.5 in when `bearcad.set_units{ length = "in" }` is in force.

Indices are creation-order ordinals and shift when things are deleted — prefer names
(`bearcad.find`) for anything you will refer to twice. One operation per call
(especially fillets, chamfers, booleans).

A rectangle is four lines (bottom, right, top, left); `rect` returns those lines as a
profile. Drawing verbs open a ground-plane sketch when none is active. An operation that
consumes a body produces a new one: chain off the handle the call returned, or use names.

## Shapes (no sketch)

A cube/box/block is `cuboid` (`cube`/`box` alias it). `cube{ size = 10 }` is equal
sides. It sits on the plane at `at` (the base centre, default origin) and grows along
`normal` (default +Z, so on the ground).

```lua
bearcad.cube{ size = 10 }
bearcad.cuboid{ width = 10, depth = 10, height = 10 }
```

`r` / `radius` / `diameter` are accepted on every radial size (circle, cylinder,
sphere, fillet). Circle `get` returns `r`, `radius`, and `diameter`.

```
bearcad.cuboid{ width, depth, height, at = {x,y,z}?, normal?, u_axis?, name? }
bearcad.cube{ size, width?, depth?, height?, at?, normal?, u_axis?, name? }
bearcad.box{ width, depth, height, at?, normal?, u_axis?, name? }
bearcad.cylinder{ r | radius | diameter, height, at?, normal?, name? }
bearcad.sphere{ r | radius | diameter, at?, name? }
bearcad.edit_shape{ index, shape = "cuboid"|"cylinder"|"sphere"?, width?, depth?, height?, size?, radius?, at?, normal?, u_axis?, name? }
```

Every dimension takes a number or an expression string.

## Sketching

```
bearcad.rect{ width, height, x = 0?, y = 0?, name? }
bearcad.line{ x, y, x1, y1, name?, dimension? }          -- or length + angle (degrees)
bearcad.circle{ x, y, r | radius | diameter, name? }
bearcad.edit_circle{ index, r | radius | diameter, name? }
bearcad.text{ text, x, y, size?, font?, bold?, italic?, underline?, rotation?, wrap?, flip?, name? }
bearcad.begin_sketch{ kind = "plane", index = i }
bearcad.begin_sketch(box:face("top"))            -- or a body_faces entry
bearcad.begin_sketch{ kind = "extrude_cap"|"extrude_side"|…, … }
bearcad.open_sketch(i)
bearcad.exit_sketch()
bearcad.plane{ offset?, from = 0?, origin = {x,y,z}?, normal = {x,y,z}?, axis = "x"|"y"|"z"|line?, angle?, name? }
bearcad.project{ body?, bodies?, plane?, planes?, entities? }
```

`dimension` on a line locks its length (number, expression, or `true` for as-drawn).

## Solids

```
bearcad.extrude{ profiles = circle | {line, …} | {…}, distance?, to?, body = "new"|"add"|"cut"|"join"?, name?, symmetric?, taper?, taper_mode = "distance"|"angle"? }
bearcad.sketch_faces(sketch?)            -- closed loops/circles/text/regions for `profiles`
bearcad.edit_extrusion{ index | extrusion, distance? | by? | to? }
bearcad.extrude_face{ face = {…}, distance, body? }
bearcad.revolve{ profiles = circle | {line, …} | {…}, axis = "x"|"y"|"z"|{ line = i }, angle? | revolutions?, pitch?, body = "new"|"add"|"cut"?, bodies?, symmetric?, name? }
bearcad.edit_revolve{ index, angle? | revolutions?, pitch?, axis?, … }
bearcad.sweep{ profiles = circle | {line, …} | {…}, path = {line, …}, body = "add"|"cut"?, bodies? }
bearcad.edit_sweep{ index, path?, … }
bearcad.loft{ profiles = { circle | {line, …}, … }, body? }
bearcad.edit_loft{ index, … }
bearcad.combine{ op = "union"|"cut"|"intersect"|"xor", a = {i, …}, b = {i, …}, keep_b?, bake?, name? }   -- `difference` means cut; bake = true consumes the inputs and leaves one standalone body
bearcad.edit_combine{ index, op, a, b, keep_b? }
bearcad.slice{ bodies = {i, …}, cutters = {…}, extend?, name? }
bearcad.shell{ bodies = {i, …}, faces = {…}?, thickness, name? }
bearcad.move_bodies{ bodies = {i, …}, x?, y?, z?, rx?, ry?, rz?, name? }
bearcad.mirror_bodies{ plane = i, bodies = {i, …}, output = "new"|"add"|"cut"?, name? }
bearcad.repeat_bodies{ bodies = {i, …}, axis = "x"|"y"|"z", mode?, count?, spacing?, length?, around?, flip?, to?, name? }
```

To cut a hole: sketch on a face, then `extrude{ …, body = "cut" }`. A cut pointing away
from the body is flipped inward.

Rounding is one call per operation — a set of edges in a single call, never one call per
edge (four calls would make four bodies):

```
bearcad.fillet{ body = h, edges = bearcad.body_edges(h) | { { kind = "vertical"|"top"|"bottom", face = i, edge = i }, … }, r | radius | diameter }
bearcad.chamfer{ body = h, edges = …, distance }
bearcad.edit_fillet{ index, radius? }
bearcad.edit_chamfer{ index, distance? }
bearcad.fillet_edge / chamfer_edge     -- aliases; `extrusion=` / `shape=` still accepted
bearcad.extrude_edges(i)               -- analytic edge refs on extrusion i
bearcad.fillet_vertex{ point = { kind = "line", index = i, endpoint = "start"|"end" }, r | radius | diameter }
bearcad.chamfer_vertex{ point = { kind = "line", index = i, endpoint = "start"|"end" }, distance }
```

## Parameters and constraints

```
bearcad.add_parameter("w", "24")
bearcad.set_parameter("w", "30")
bearcad.select{ kind, index, endpoint? }               -- second arg true = add
                                                       -- line vertex: endpoint = "start"|"end"
                                                       -- or line:start() / line:endpoint("end")
                                                       -- drawing page items: kind "projection"|"annotation"|"dimension"
                                                       --   + drawing (+ view / a,b / index); selecting opens the drawing
bearcad.constrain("parallel"|"perpendicular"|"equal"|"coincident"|"midpoint"|"horizontal"|"vertical"|"tangent", a, b, …)
bearcad.dimension{ kind = "line"|"circle"|"point_point"|"point_line"|"line_line"|"angle", value, … }
bearcad.ui.add_geometric_constraint(name)              -- current selection; UI tests only
```

Anywhere a size is accepted, an expression string is too.

## Inspect

```
bearcad.count(kind)                -- canonical: line, circle, sketch, constraint,
bearcad.get(handle|{ kind, index }|kind, index)
                                   --   plane, extrusion, revolution, sweep,
                                   --   loft, combine, move, mirror, repeat, slice, shell,
                                   --   edge_treatment, sketch_offset, sketch_mirror,
                                   --   sketch_repeat, sketch_slice, sketch_chamfer, shape,
                                   --   body, drawing, cross_section, section_plane, parameter,
                                   --   sketch_text, component, image, joint, unit_instance.
                                   --   aliases: construction_plane, revolve, boolean, primitive, text,
                                   --   tracing_image, sketch_fillet, unit, offset.
                                   --   not chamfer/fillet (use edge_treatment or sketch_chamfer).
                                   --   `count` and `get` take the same set.
                                   --   get returns create/edit keys plus evaluated numbers;
                                   --   missing identity is nil (unknown kind errors).
bearcad.find("name")               -- sugar for element-by-name; nil if missing
bearcad.set_name(el, "name")
bearcad.element("line", i)         -- the lookup; also element(id) / element(name)
bearcad.id(el)                     -- el:id(): a stable id, unique and never reused
bearcad.line_endpoints(i)          -- x0, y0, x1, y1; missing → nil
bearcad.image_corners(i)           -- tracing image quad in world mm, live Move included
bearcad.body_stats(i)              -- volume, triangles, bbox = { min = {x,y,z}, max = {x,y,z} }
                                   -- missing body → nil; body with no mesh → error
bearcad.body_faces(i)              -- pass an entry to begin_sketch / extrude_face / fillet
bearcad.drawing_views(i)           -- a drawing's page: orientation, style, bodies, dimensions
bearcad.body_edges(i)              -- pass entries to fillet{ body, edges } / chamfer{ body, edges }
bearcad.body_cylinders(i)
bearcad.selection()                -- { kind, index, … } tables that work as handles;
                                   -- point selections include index + endpoint
bearcad.visible(el)                -- effective visibility, component chain included
bearcad.set_visible(el, false)     -- handle, list, or { kind = "plane" }; boolean only
bearcad.set_construction(el, true) -- same targets; selection forms are bearcad.ui.*
bearcad.sketch_dof()
bearcad.sketch_conflicts()
bearcad.sketch_faces()

bearcad.status()
```

Never assume a call did what you meant: read it back and assert.

## Handles

A creation call hands back what it made: one element, or a list of them.

```
local sides = bearcad.rect{ x = 0, y = 0, width = 20, height = 10 }   -- four lines
local box   = bearcad.extrude{ profiles = sides, distance = 5 }        -- the new body
box:kind()  box:index()  box:id()  box:name()  box:exists()  box:delete()
box:get()   box:stats()  box:select()  tostring(box)
bearcad.delete(box)                -- or a list; does not replace the scene selection
bearcad.delete_selection()         -- whatever is selected (the GUI Delete)
```

Ordinals shift when elements are deleted, and a solid op consumes the body it acts on.
A handle does not: it names the same element until that element is gone, and then says so.
Anywhere an index is accepted — `bodies`, `profiles`, `extrusion`, `{ kind, index }`, … —
a handle, its `id` string, or a name works too.

## Files

```
bearcad.new()
bearcad.open("part.bearcad")
bearcad.save()                     -- or save("other.bearcad")
bearcad.undo()
bearcad.import_step("part.step")
bearcad.import_stl("part.stl")
bearcad.export_step("out.step")            -- or (path, body) with a handle/id/name/ordinal
bearcad.export_stl("out.stl")
bearcad.export_3mf("out.3mf")
```

## Drawings

`bearcad.drawing_text{ drawing, text, x, y }` — x/y are page fractions (0–1), not millimetres.

## GUI (`bearcad.ui.*`)

Reach for this only when the interaction itself is the point.

```
bearcad.ui.tool("select"|"rectangle"|…)
bearcad.ui.view("front"|"top"|"iso"|…)
bearcad.ui.zoom_fit()
bearcad.ui.screenshot("shot.png")            -- viewport; "window" / a pane name for others
bearcad.ui.camera{ yaw?, pitch?, distance?, target?, projection?, shading?, ground? }
bearcad.ui.camera{}                          -- read it back, shading and ground included
bearcad.ui.shading("loose_pencil")           -- …|realistic|loose_pencil|dark_pencil|color_pencil|watercolor
bearcad.ui.pane("ai"|"hierarchy"|"context"|"parameters"|…, "show"|"hide"|"toggle")
bearcad.ui.click_ground(x, y)               -- sketch-plane millimetres
bearcad.ui.click_world(x, y, z)             -- any world point: a body's side wall, say
bearcad.ui.click(x, y) / click(rect)        -- viewport px, or a window-space rect/orb
bearcad.ui.double_click(x, y) / (rect)      -- waits out egui's click counter
bearcad.ui.viewport()                       -- { width, height, x, y } of the area clicks address
bearcad.ui.right_click_ground(x, y)         -- opens a context menu
bearcad.ui.context_menu()                   -- { kind, index } of the open menu, or nil
bearcad.ui.key("enter")
bearcad.ui.palette("Export STEP")
bearcad.ui.begin_move{ … } / begin_combine / begin_joint / begin_edit_shape   -- arm a tool; do not commit
bearcad.ui.pickers() / picker("Targets")                  -- armed tool pickers
bearcad.ui.gizmos() / gizmo("move_rz")                    -- live gizmo rows
bearcad.ui.hovered() / exploder()                         -- viewport hover / Selection Exploder
bearcad.ui.set_dim / edit_dim / commit_dim                -- dimension widget
```
"#;

fn build() -> String {
    // The function list is built by registering the API into a throwaway Lua state, which
    // the web build does not do — there `out` is the guide alone.
    #[allow(unused_mut)]
    let mut out = String::from(GUIDE);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let names = registered_names();
        if !names.is_empty() {
            out.push_str(
                "\n## Every function\n\nEvery function BearCAD exposes, with the arguments it \
                 takes — a name that is\nnot in this list is not a function. Built from the \
                 live Lua table:\naccepted option keys plus positional args. `{ … }` is one \
                 options table; `?` marks an\noptional argument or table. The sections above \
                 carry the detail for the calls they\ncover.\n\n```\n",
            );
            for (_, sig) in live_signatures() {
                out.push_str(sig);
                out.push('\n');
            }
            out.push_str("```\n");
        }
    }
    out
}

/// The call shape published for `name` (`bearcad.line`, `bearcad.ui.camera`, …).
#[cfg(test)]
pub fn signature(name: &str) -> Option<&'static str> {
    live_signatures()
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, sig)| sig.as_str())
}

/// One line per public function, in the same order as [`registered_names`].
/// Built by walking the live Lua table and the `create_function` / `check_keys`
/// shapes `register_api` actually installed (#1884).
#[cfg(not(target_arch = "wasm32"))]
pub fn live_signatures() -> &'static [(String, String)] {
    SIGNATURES.get_or_init(harvest_signatures).as_slice()
}

#[cfg(target_arch = "wasm32")]
pub fn live_signatures() -> &'static [(String, String)] {
    &[]
}

#[cfg(not(target_arch = "wasm32"))]
static SIGNATURES: OnceLock<Vec<(String, String)>> = OnceLock::new();

#[derive(Clone, Debug)]
struct RustParams {
    table: bool,
    table_optional: bool,
    /// Formatted `(path, body?)` / `()` / `(…)` when the call is not a pure options table.
    positional: Option<String>,
}

const LUA_SCRIPT_SRC: &str = include_str!("../lua_script.rs");

#[cfg(not(target_arch = "wasm32"))]
fn harvest_signatures() -> Vec<(String, String)> {
    let names = registered_names();
    let rust = parse_api_set_params(LUA_SCRIPT_SRC);
    let from_src = parse_check_keys_map(LUA_SCRIPT_SRC);
    let probed = probe_accepted_keys(&names, &rust);
    names
        .into_iter()
        .map(|name| {
            let short = name.rsplit('.').next().unwrap_or(name.as_str());
            let keys = probed
                .get(short)
                .or_else(|| from_src.get(short))
                .map(|k| k.as_slice());
            let sig = format_signature(&name, rust.get(short), keys);
            (name, sig)
        })
        .collect()
}

/// `check_keys(opts, "rect", &["x", "y"])` and `const FOO: &[&str] = &["a"]`.
fn parse_check_keys_map(src: &str) -> std::collections::HashMap<String, Vec<String>> {
    let consts = parse_str_slice_consts(src);
    let mut out = std::collections::HashMap::new();
    let mut via_call: Option<Vec<String>> = None;
    let mut search = 0;
    while let Some(rel) = src[search..].find("check_keys(") {
        let abs = search + rel + "check_keys(".len();
        let Some(end) = matching_close_paren(&src[abs..]) else {
            search = abs;
            continue;
        };
        let args = split_call_args(&src[abs..abs + end]);
        search = abs;
        if args.len() < 3 {
            continue;
        }
        let keys = expand_keys_expr(&args[2], &consts);
        if keys.is_empty() {
            continue;
        }
        match string_lit(&args[1]) {
            Some(name) => {
                out.insert(name, keys);
            }
            None => via_call = Some(keys),
        }
    }
    if let Some(keys) = via_call {
        for name in string_lits_named(src, "parse_joint_op_args") {
            out.entry(name).or_insert_with(|| keys.clone());
        }
    }
    out
}

fn parse_str_slice_consts(src: &str) -> std::collections::HashMap<String, Vec<String>> {
    let mut out = std::collections::HashMap::new();
    let mut search = 0;
    while let Some(rel) = src[search..].find("const ") {
        let abs = search + rel + 6;
        let rest = src[abs..].trim_start();
        let Some(name_end) = rest.find(':') else {
            search = abs;
            continue;
        };
        let name = rest[..name_end].trim();
        if !name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
            search = abs;
            continue;
        }
        let after = rest[name_end..].trim_start();
        let Some(after) = after.strip_prefix(": &[&str]") else {
            search = abs;
            continue;
        };
        let after = after.trim_start().trim_start_matches('=').trim_start();
        if let Some(inner) = bracket_inner(after.trim_start_matches('&').trim()) {
            out.insert(name.to_string(), string_lits(inner));
        }
        search = abs;
    }
    out
}

fn expand_keys_expr(
    expr: &str,
    consts: &std::collections::HashMap<String, Vec<String>>,
) -> Vec<String> {
    let expr = expr.trim().trim_end_matches(".concat()").trim();
    let expr = expr.trim_start_matches('&').trim();
    if let Some(keys) = consts.get(expr) {
        return keys.clone();
    }
    let Some(inner) = bracket_inner(expr) else {
        return Vec::new();
    };
    let parts = split_call_args(inner);
    if parts.iter().any(|p| string_lit(p).is_none()) {
        let mut out = Vec::new();
        for part in parts {
            out.extend(expand_keys_expr(&part, consts));
        }
        return out;
    }
    string_lits(inner)
}

fn bracket_inner(s: &str) -> Option<&str> {
    let rest = s.trim().strip_prefix('[')?;
    let mut depth: i32 = 1;
    for (i, ch) in rest.char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&rest[..i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn string_lits(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut search = 0;
    let bytes = s.as_bytes();
    while search < bytes.len() {
        if bytes[search] == b'"' {
            search += 1;
            let start = search;
            while search < bytes.len() && bytes[search] != b'"' {
                search += 1;
            }
            if search <= bytes.len() {
                out.push(s[start..search.min(s.len())].to_string());
            }
            search += 1;
        } else {
            search += 1;
        }
    }
    out
}

fn string_lit(s: &str) -> Option<String> {
    let s = s.trim();
    let inner = s.strip_prefix('"')?.strip_suffix('"')?;
    Some(inner.to_string())
}

fn string_lits_named(src: &str, fn_name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut search = 0;
    let needle = format!("{fn_name}(");
    while let Some(rel) = src[search..].find(&needle) {
        let abs = search + rel + needle.len();
        let Some(end) = matching_close_paren(&src[abs..]) else {
            search = abs;
            continue;
        };
        for arg in split_call_args(&src[abs..abs + end]) {
            if let Some(s) = string_lit(&arg) {
                out.push(s);
            }
        }
        search = abs;
    }
    out
}

fn matching_close_paren(s: &str) -> Option<usize> {
    let mut depth: i32 = 1;
    let mut brackets: i32 = 0;
    for (i, ch) in s.char_indices() {
        match ch {
            '[' => brackets += 1,
            ']' => brackets = brackets.saturating_sub(1),
            '(' => depth += 1,
            ')' if brackets == 0 => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_call_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth: i32 = 0;
    for ch in s.chars() {
        match ch {
            '(' | '[' => {
                depth += 1;
                cur.push(ch);
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                cur.push(ch);
            }
            ',' if depth == 0 => {
                let t = cur.trim().to_string();
                if !t.is_empty() {
                    out.push(t);
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    let t = cur.trim().to_string();
    if !t.is_empty() {
        out.push(t);
    }
    out
}

/// `api.set("name", lua.create_function(|lua, ARGS|` → short name → [`RustParams`].
fn parse_api_set_params(src: &str) -> std::collections::HashMap<String, RustParams> {
    let mut out = std::collections::HashMap::new();
    let mut search = 0;
    while let Some(rel) = src[search..].find("api.set") {
        let abs = search + rel;
        let after = src[abs + 7..]
            .trim_start()
            .trim_start_matches('(')
            .trim_start();
        let name = after
            .strip_prefix('"')
            .and_then(|rest| rest.find('"').map(|end| rest[..end].to_string()));
        let Some(cf) = after.find("create_function") else {
            search = abs + 8;
            continue;
        };
        let after_cf = &after[cf + "create_function".len()..];
        let Some(bar) = after_cf.find('|') else {
            search = abs + 8;
            continue;
        };
        let after_bar = &after_cf[bar + 1..];
        let Some(end) = after_bar.find('|') else {
            search = abs + 8;
            continue;
        };
        if let Some(name) = name {
            let params = after_bar[..end].trim();
            out.insert(name, classify_params(&strip_lua_receiver(params)));
        }
        search = abs + 8;
    }
    out
}

fn strip_lua_receiver(params: &str) -> String {
    let params = params.trim();
    for prefix in ["_lua,", "lua,", "_lua", "lua"] {
        if let Some(rest) = params.strip_prefix(prefix) {
            return rest.trim().to_string();
        }
    }
    params.to_string()
}

fn classify_params(args: &str) -> RustParams {
    let args = collapse_ws(args);
    if args.is_empty() || args == "()" {
        return RustParams {
            table: false,
            table_optional: false,
            positional: Some("()".into()),
        };
    }
    if args.contains("MultiValue") && !args.contains('(') {
        return RustParams {
            table: false,
            table_optional: false,
            positional: Some("(…)".into()),
        };
    }
    let is_single = !args.trim_start().starts_with('(');
    if is_single && args.contains("Option<Table>") {
        return RustParams {
            table: true,
            table_optional: true,
            positional: None,
        };
    }
    if is_single && args.contains(": Table") {
        return RustParams {
            table: true,
            table_optional: false,
            positional: None,
        };
    }
    RustParams {
        table: false,
        table_optional: false,
        positional: Some(format_positional(&args)),
    }
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            space = true;
        } else {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(ch);
        }
    }
    out
}

fn format_positional(args: &str) -> String {
    let Some((names_part, types_part)) = args.split_once(':') else {
        return "(…)".into();
    };
    let names_part = names_part.trim();
    let names: Vec<&str> = if let Some(inner) = names_part
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
    {
        inner
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        vec![names_part]
    };
    let types_inner = types_part
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();
    let types = split_type_list(types_inner);
    let mut parts = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let ty = types.get(i).map(String::as_str).unwrap_or("");
        if ty.contains("MultiValue") {
            parts.push("…".to_string());
        } else if ty.contains("Option<") {
            parts.push(format!("{name}?"));
        } else {
            parts.push((*name).to_string());
        }
    }
    format!("({})", parts.join(", "))
}

fn split_type_list(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth: i32 = 0;
    for ch in s.chars() {
        match ch {
            '<' | '(' => {
                depth += 1;
                cur.push(ch);
            }
            '>' | ')' => {
                depth = depth.saturating_sub(1);
                cur.push(ch);
            }
            ',' if depth == 0 => {
                let t = cur.trim().to_string();
                if !t.is_empty() {
                    out.push(t);
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    let t = cur.trim().to_string();
    if !t.is_empty() {
        out.push(t);
    }
    out
}

fn format_signature(full_name: &str, rust: Option<&RustParams>, keys: Option<&[String]>) -> String {
    if let Some(keys) = keys {
        let table = if keys.is_empty() {
            "{}".to_string()
        } else {
            format!("{{ {} }}", keys.join(", "))
        };
        if rust.map(|r| r.table).unwrap_or(true) {
            let suffix = if rust.map(|r| r.table_optional).unwrap_or(false) {
                "?"
            } else {
                ""
            };
            return format!("{full_name}{table}{suffix}");
        }
        if let Some(pos) = rust.and_then(|r| r.positional.as_deref()) {
            let inner = pos.trim_start_matches('(').trim_end_matches(')');
            return format!("{full_name}({inner} | {table})");
        }
        return format!("{full_name}{table}");
    }
    if let Some(pos) = rust.and_then(|r| r.positional.as_deref()) {
        if pos == "()" {
            return format!("{full_name}()");
        }
        return format!("{full_name}{pos}");
    }
    if rust.map(|r| r.table).unwrap_or(false) {
        let suffix = if rust.map(|r| r.table_optional).unwrap_or(false) {
            "?"
        } else {
            ""
        };
        return format!("{full_name}{{ … }}{suffix}");
    }
    format!("{full_name}(…)")
}

/// Call each registered function with a unique unknown key and parse `accepted keys`.
#[cfg(not(target_arch = "wasm32"))]
fn probe_accepted_keys(
    names: &[String],
    rust: &std::collections::HashMap<String, RustParams>,
) -> std::collections::HashMap<String, Vec<String>> {
    let mut out = std::collections::HashMap::new();
    let lua = mlua::Lua::new();
    if crate::lua_script::register_api(&lua).is_err() {
        return out;
    }
    let mut runner = crate::script::ScriptRunner::from_instructions(vec![]);
    runner.verbose = false;
    let mut state = crate::actions::AppState::default();
    let mut synthetic = crate::script::SyntheticInput::default();
    let mut ctx = egui::Context::default();
    let viewport = Some(egui::Rect::from_min_size(
        egui::pos2(0.0, 40.0),
        egui::vec2(960.0, 560.0),
    ));
    lua.set_app_data(crate::lua_script::ScriptTickData {
        runner: &mut runner,
        state: &mut state,
        synthetic: &mut synthetic,
        viewport,
        ctx: &mut ctx,
    });
    for name in names {
        let short = name.rsplit('.').next().unwrap_or(name.as_str());
        // Positional no-arg calls ignore extra args; probing them would run copy/quit/…
        // UI verbs are registered as `_zoom_fit` then moved to `ui.zoom_fit`.
        let r = rust.get(short).or_else(|| rust.get(&format!("_{short}")));
        if let Some(r) = r {
            if !r.table && r.positional.as_deref() != Some("(value)") {
                continue;
            }
        }
        let Some(func) = lua_function(&lua, name) else {
            continue;
        };
        let Ok(table) = lua.create_table() else {
            continue;
        };
        let _ = table.set("__bearcad_api_probe__", true);
        match func.call::<()>(table) {
            Ok(()) => {}
            Err(e) => {
                if let Some(keys) = keys_from_error(&e.to_string()) {
                    let short = name.rsplit('.').next().unwrap_or(name).to_string();
                    out.insert(short, keys);
                }
            }
        }
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
fn lua_function(lua: &mlua::Lua, path: &str) -> Option<mlua::Function> {
    let mut cur: mlua::Value = lua.globals().get("bearcad").ok()?;
    let parts: Vec<&str> = path.split('.').skip(1).collect();
    for (i, part) in parts.iter().enumerate() {
        let table = match cur {
            mlua::Value::Table(t) => t,
            _ => return None,
        };
        let next: mlua::Value = table.get(*part).ok()?;
        if i + 1 == parts.len() {
            return match next {
                mlua::Value::Function(f) => Some(f),
                _ => None,
            };
        }
        cur = next;
    }
    None
}

fn keys_from_error(err: &str) -> Option<Vec<String>> {
    for prefix in ["accepted keys: ", "(try "] {
        if let Some(i) = err.find(prefix) {
            let rest = &err[i + prefix.len()..];
            let rest = rest.split(')').next().unwrap_or(rest);
            let rest = rest.split('\n').next().unwrap_or(rest).trim();
            if rest.is_empty() {
                return Some(Vec::new());
            }
            let keys: Vec<String> = rest
                .split(", ")
                .map(|s| s.trim().trim_matches('`').to_string())
                .filter(|s| {
                    !s.is_empty() && s.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
                })
                .collect();
            if keys.is_empty() {
                return None;
            }
            return Some(keys);
        }
    }
    None
}

/// Every public `bearcad.*` / `bearcad.ui.*` function, by walking the
/// live Lua table after [`crate::lua_script::register_api`]. Underscore-prefixed names
/// are internals (the yielding wrappers sit next to them without the prefix).
#[cfg(not(target_arch = "wasm32"))]
pub fn registered_names() -> Vec<String> {
    let lua = mlua::Lua::new();
    if crate::lua_script::register_api(&lua).is_err() {
        return Vec::new();
    }
    let Ok(bearcad) = lua.globals().get::<mlua::Table>("bearcad") else {
        return Vec::new();
    };
    let mut names = Vec::new();
    collect_functions(&bearcad, "bearcad", &mut names);
    names.sort();
    names.dedup();
    names
}

#[cfg(not(target_arch = "wasm32"))]
fn collect_functions(table: &mlua::Table, prefix: &str, out: &mut Vec<String>) {
    let mut nested = Vec::new();
    for pair in table.clone().pairs::<String, mlua::Value>() {
        let Ok((key, value)) = pair else { continue };
        if key.starts_with('_') {
            continue;
        }
        match value {
            mlua::Value::Function(_) => out.push(format!("{prefix}.{key}")),
            mlua::Value::Table(child) => nested.push((format!("{prefix}.{key}"), child)),
            _ => {}
        }
    }
    for (child_prefix, child) in nested {
        collect_functions(&child, &child_prefix, out);
    }
}

/// The fenced Every-function list from [`document`] — one line per registered call.
#[cfg(all(test, not(target_arch = "wasm32")))]
fn every_function_dump() -> String {
    let doc = document();
    let Some(rest) = doc.split("## Every function").nth(1) else {
        return String::new();
    };
    let Some(fence) = rest.find("```\n") else {
        return String::new();
    };
    let body = &rest[fence + 4..];
    match body.find("```") {
        Some(end) => body[..end].to_string(),
        None => body.to_string(),
    }
}

/// The copy the website serves at `/bearcad-api.md` (#1635) — regenerated with
/// `cargo run -- api > docs-site/static/bearcad-api.md`, and kept honest by
/// [`tests::the_published_page_matches_what_bearcad_api_prints`].
#[cfg(test)]
const PUBLISHED: &str = include_str!("../../docs-site/static/bearcad-api.md");

#[cfg(test)]
mod tests {
    use super::*;

    /// #1635: the page an agent lands on is the API the app actually has.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_published_page_matches_what_bearcad_api_prints() {
        assert_eq!(
            PUBLISHED.trim_end(),
            document().trim_end(),
            "docs-site/static/bearcad-api.md is stale — regenerate it with \
             `cargo run -- api > docs-site/static/bearcad-api.md`"
        );
    }

    /// #1664: the page is only "the complete API" if every name on it says how to call
    /// it. A function added without a signature fails here, not silently in an agent's
    /// hands.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn every_registered_function_has_a_signature() {
        let names = registered_names();
        assert!(!names.is_empty(), "the API registered nothing");
        let missing: Vec<&String> = names.iter().filter(|n| signature(n).is_none()).collect();
        assert!(
            missing.is_empty(),
            "no signature for {missing:?} — harvest from register_api failed"
        );
        let stale: Vec<&str> = live_signatures()
            .iter()
            .map(|(n, _)| n.as_str())
            .filter(|n| !names.iter().any(|r| r == n))
            .collect();
        assert!(
            stale.is_empty(),
            "harvested signatures list {stale:?}, which is not a function"
        );
    }

    /// #1664: a signature must be a call of the name it belongs to — a copy/paste that
    /// keeps the wrong name would send an agent to a function that does not exist.
    #[test]
    fn every_signature_starts_with_its_own_name() {
        for (name, sig) in live_signatures() {
            let next = sig.get(name.len()..).and_then(|s| s.chars().next());
            assert!(
                sig.starts_with(name) && matches!(next, Some('(') | Some('{')),
                "{name}'s signature is {sig:?}"
            );
        }
    }

    /// #1664: a published key must be one the call actually accepts. Every key of every
    /// documented options table is offered to its own function; a call that validates its
    /// keys must not answer "unknown key" for one the page told an agent to pass.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn every_published_option_key_is_accepted_by_its_call() {
        let mut checks = String::from("local bad = {}\n");
        checks.push_str(
            "local function check(name, key)\n\
             local f = _G\n\
             for part in string.gmatch(name, \"[^.]+\") do f = f[part] end\n\
             if type(f) ~= \"function\" then return end\n\
             local ok, err = pcall(function() f({ [key] = 0 }) end)\n\
             if not ok and tostring(err):find(\"unknown key `\" .. key .. \"`\", 1, true) then\n\
             bad[#bad+1] = name .. \" : \" .. key\n\
             end\n\
             end\n",
        );
        let mut keys = 0;
        for (name, sig) in live_signatures() {
            // A trailing `-- note` may itself mention a nested table; the call's own keys
            // are what comes before it.
            let sig = sig.split("  --").next().unwrap_or(sig).trim_end();
            let Some(open) = sig.find("{ ") else { continue };
            let Some(close) = sig.rfind(" }") else {
                continue;
            };
            for key in sig[open + 2..close].split(',') {
                let key = key.trim().trim_end_matches('?');
                if key.is_empty() || key == "…" {
                    continue;
                }
                checks.push_str(&format!("check({name:?}, {key:?})\n"));
                keys += 1;
            }
        }
        assert!(keys > 300, "only {keys} option keys published?");
        checks.push_str("assert(#bad == 0, table.concat(bad, \"; \"))\n");
        crate::lua_script::tests::run_lua(&checks);
    }

    /// #1664: the traps that made the page unusable — a call whose table shape the prose
    /// never showed, and one whose x/y are page fractions rather than millimetres.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_page_shows_the_calls_agents_could_not_reach() {
        let doc = document();
        for call in [
            "bearcad.drawing_text{",
            "bearcad.drawing_view_add{",
            "bearcad.drawing_dimension{",
            "bearcad.export_drawing_svg{",
            "bearcad.joint{",
            "bearcad.material{",
            "bearcad.component{",
            "bearcad.set_units{",
            "bearcad.offset_sketch{",
            "bearcad.ui.hovered(",
            "bearcad.ui.pickers(",
        ] {
            assert!(
                doc.contains(call),
                "the page never shows how to call {call}…"
            );
        }
        assert!(
            doc.contains("page fractions"),
            "drawing_text's x/y are not millimetres; the page must say so"
        );
    }

    #[test]
    fn cuboid_is_how_you_make_a_box() {
        let doc = document();
        assert!(
            doc.contains("bearcad.cuboid{"),
            "got: {}",
            &doc[..doc.len().min(400)]
        );
        assert!(doc.contains("bearcad.cube{"), "cube aliases cuboid");
        assert!(doc.contains("bearcad.box{"), "box aliases cuboid");
        assert!(doc.contains("size = 10"));
        assert!(doc.contains("width = 10, depth = 10, height = 10"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn every_registered_function_is_listed() {
        let doc = document();
        let names = registered_names();
        assert!(
            names.iter().any(|n| n == "bearcad.cuboid"),
            "cuboid is registered in a loop; the walker must still find it: {names:?}"
        );
        assert!(names.iter().any(|n| n == "bearcad.rect"));
        assert!(names.iter().any(|n| n == "bearcad.ui.tool"));
        let mut missing = Vec::new();
        for name in &names {
            if !doc.contains(name) {
                missing.push(name.clone());
            }
        }
        assert!(
            missing.is_empty(),
            "the catalog omitted {} function(s), including {:?}",
            missing.len(),
            &missing[..missing.len().min(8)]
        );
    }

    /// #1884: the Every-function dump is harvested from registration (`check_keys` +
    /// `create_function` params), not a hand-maintained table that can lie.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_every_function_list_matches_what_is_registered() {
        let dump = every_function_dump();
        let names = registered_names();
        assert!(!names.is_empty(), "the API registered nothing");
        let mut listed = Vec::new();
        for line in dump.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('`') {
                continue;
            }
            let name_end = line
                .find(|c: char| c == '(' || c == '{')
                .unwrap_or(line.len());
            listed.push(line[..name_end].trim().to_string());
        }
        assert_eq!(
            listed, names,
            "Every-function dump must be exactly the registered names, in order"
        );

        fn line_for<'a>(dump: &'a str, name: &str) -> &'a str {
            dump.lines()
                .find(|l| l.starts_with(name) && l[name.len()..].starts_with(['(', '{']))
                .unwrap_or("")
        }
        let combine = line_for(&dump, "bearcad.combine");
        assert!(
            combine.contains("bake"),
            "combine accepts `bake`; the dump must say so: {combine}"
        );
        let cut = line_for(&dump, "bearcad.repeat_cut");
        assert!(cut.contains("cuts"), "repeat_cut takes `cuts`: {cut}");
        assert!(
            !cut.contains("bodies"),
            "repeat_cut must not list `bodies`: {cut}"
        );
        let sketches = line_for(&dump, "bearcad.repeat_sketches");
        assert!(
            sketches.contains("sketches"),
            "repeat_sketches takes `sketches`: {sketches}"
        );
        assert!(
            !sketches.contains("bodies"),
            "repeat_sketches must not list `bodies`: {sketches}"
        );
        let param = line_for(&dump, "bearcad.add_parameter");
        assert!(
            param.starts_with("bearcad.add_parameter("),
            "add_parameter is positional: {param}"
        );
        let export = line_for(&dump, "bearcad.export_step");
        assert!(
            export.starts_with("bearcad.export_step("),
            "export path is positional: {export}"
        );
        let constrain = line_for(&dump, "bearcad.constrain");
        assert!(
            constrain.contains('(') && constrain.contains('…'),
            "constrain takes explicit refs: {constrain}"
        );
        let angle = line_for(&dump, "bearcad.dimension");
        assert!(
            angle.contains("value"),
            "dimension's amount key is `value`: {angle}"
        );
        let first_person = line_for(&dump, "bearcad.ui.first_person");
        assert!(
            !first_person.is_empty(),
            "first_person is registered and must be listed"
        );
        let globals = line_for(&dump, "bearcad.globals");
        assert_eq!(globals, "bearcad.globals()");
        let fillet = line_for(&dump, "bearcad.fillet");
        assert!(fillet.contains("body"), "fillet takes `body`: {fillet}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn every_call_the_guide_shows_actually_exists() {
        let names: std::collections::HashSet<_> = registered_names().into_iter().collect();
        let mut checked = 0;
        for (index, _) in GUIDE.match_indices("bearcad.") {
            let rest = &GUIDE[index..];
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
                .unwrap_or(rest.len());
            let path = rest[..end].trim_end_matches('.');
            let next = rest[end..].chars().next();
            if !matches!(next, Some('(') | Some('{')) {
                continue;
            }
            assert!(
                names.contains(path),
                "the API guide calls {path}, which the Lua API does not have"
            );
            checked += 1;
        }
        assert!(
            checked > 20,
            "expected the guide to show real calls, saw {checked}"
        );
    }

    #[test]
    fn parse_check_keys_map_reads_literal_and_const_lists() {
        let src = r#"
            const MOVE_OP_KEYS: &[&str] = &["bodies", "x", "from", "to"];
            check_keys(&opts, "rect", &["x", "y", "width", "height", "name"])?;
            check_keys(&opts, "begin_move", MOVE_OP_KEYS)?;
            check_keys(&opts, "move_bodies", &[MOVE_OP_KEYS, &["name"]].concat())?;
            fn parse_joint_op_args(lua: &Lua, opts: &Table, call: &str) {
                check_keys(opts, call, &["index", "a", "b", "name"])?;
            }
            parse_joint_op_args(lua, &opts, "joint")?;
            parse_joint_op_args(lua, &opts, "edit_joint")?;
        "#;
        let map = parse_check_keys_map(src);
        assert_eq!(map["rect"], ["x", "y", "width", "height", "name"]);
        assert_eq!(map["begin_move"], ["bodies", "x", "from", "to"]);
        assert_eq!(map["move_bodies"], ["bodies", "x", "from", "to", "name"]);
        assert_eq!(map["joint"], ["index", "a", "b", "name"]);
        assert_eq!(map["edit_joint"], ["index", "a", "b", "name"]);
    }

    #[test]
    fn parse_api_set_params_reads_create_function_shapes() {
        let src = r#"
            api.set("new", lua.create_function(|lua, ()| { Ok(()) })?);
            api.set(
                "open",
                lua.create_function(|lua, path: String| { Ok(()) })?,
            );
            api.set("save", lua.create_function(|lua, path: Option<String>| Ok(()))?);
            api.set("rect", lua.create_function(|lua, opts: Table| Ok(()))?);
            api.set("project", lua.create_function(|lua, opts: Option<Table>| Ok(()))?);
            api.set(
                "export_step",
                lua.create_function(|lua, (path, body): (String, Option<Value>)| Ok(()))?,
            );
            api.set("select", lua.create_function(|lua, args: MultiValue| Ok(()))?);
            api.set(
                "drag_world",
                lua.create_function(
                    |lua, (x0, y0, z0, x1, y1, z1): (f32, f32, f32, f32, f32, f32)| Ok(()),
                )?,
            );
        "#;
        let map = parse_api_set_params(src);
        assert_eq!(map["new"].positional.as_deref(), Some("()"));
        assert_eq!(map["open"].positional.as_deref(), Some("(path)"));
        assert_eq!(map["save"].positional.as_deref(), Some("(path?)"));
        assert!(map["rect"].table && !map["rect"].table_optional);
        assert!(map["project"].table && map["project"].table_optional);
        assert_eq!(
            map["export_step"].positional.as_deref(),
            Some("(path, body?)")
        );
        assert_eq!(map["select"].positional.as_deref(), Some("(…)"));
        assert_eq!(
            map["drag_world"].positional.as_deref(),
            Some("(x0, y0, z0, x1, y1, z1)")
        );
    }

    #[test]
    fn keys_from_error_stops_before_the_lua_traceback() {
        let err = "combine: unknown key `__bearcad_api_probe__` (accepted keys: op, a, b, keep_b, bake, name)\nstack traceback:\n\t[C]: in ?";
        assert_eq!(
            keys_from_error(err),
            Some(
                ["op", "a", "b", "keep_b", "bake", "name"]
                    .into_iter()
                    .map(str::to_string)
                    .collect()
            )
        );
    }

    #[test]
    fn format_signature_combines_keys_and_params() {
        let table = RustParams {
            table: true,
            table_optional: false,
            positional: None,
        };
        let opt = RustParams {
            table: true,
            table_optional: true,
            positional: None,
        };
        let dual = RustParams {
            table: false,
            table_optional: false,
            positional: Some("(value)".into()),
        };
        let keys = ["op".into(), "a".into(), "bake".into()];
        assert_eq!(
            format_signature("bearcad.combine", Some(&table), Some(&keys)),
            "bearcad.combine{ op, a, bake }"
        );
        assert_eq!(
            format_signature("bearcad.project", Some(&opt), Some(&["entities".into()])),
            "bearcad.project{ entities }?"
        );
        assert_eq!(
            format_signature(
                "bearcad.import_unit",
                Some(&dual),
                Some(&["path".into(), "link".into()])
            ),
            "bearcad.import_unit(value | { path, link })"
        );
        assert_eq!(
            format_signature(
                "bearcad.new",
                Some(&RustParams {
                    table: false,
                    table_optional: false,
                    positional: Some("()".into()),
                }),
                None
            ),
            "bearcad.new()"
        );
    }
}
