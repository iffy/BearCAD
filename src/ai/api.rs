//! The BearCAD Lua API, as one plaintext page (#1623/#1635).
//!
//! An agent that only reads prose invents calls that do not exist.
//! This catalog is the real API: signatures for the modeling verbs, then **every** registered
//! function, so a name that is not here is not a function.
//!
//! **One source, three consumers.** `bearcad api` prints it; the checked-in copy at
//! `docs-site/static/bearcad-api.md` is what the website serves at `/bearcad-api.md`, which
//! is where `bearcad-skill.md` sends an agent that needs the whole surface; and a test keeps
//! the copy honest. Regenerate it with:
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

A rectangle is four lines (bottom, right, top, left). Drawing verbs open a ground-plane
sketch when none is active. An operation that consumes a body produces a new one, so
the index moves: chain off `bearcad.count("body") - 1` or use names.

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
bearcad.fillet_edge{ extrusion = i | shape = i, edges = { { kind = "vertical"|"top"|"bottom", face = i, edge = i }, … }, r | radius | diameter }
bearcad.edit_fillet{ index, radius? }
bearcad.chamfer_edge{ extrusion = i | shape = i, edges = { … }, distance }
bearcad.edit_chamfer{ index, distance? }
bearcad.extrude_edges(i)               -- the edge refs fillet/chamfer accept on extrusion i
bearcad.fillet_vertex{ point = { kind = "line", index = i, endpoint = "start"|"end" }, r | radius | diameter }
bearcad.chamfer_vertex{ point = { kind = "line", index = i, endpoint = "start"|"end" }, distance }
```

Shape-tool cuboids use the same edge calls with `shape = i` (`primitive` still accepted).

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
bearcad.get{ kind, index }         --   plane, extrusion, revolution, sweep,
                                   --   loft, combine, move, mirror, repeat, slice, shell,
                                   --   edge_treatment, sketch_offset, sketch_mirror,
                                   --   sketch_repeat, sketch_slice, sketch_chamfer, shape,
                                   --   body, drawing, cross_section, section_plane, parameter,
                                   --   sketch_text, component, image, joint, unit_instance.
                                   --   aliases: construction_plane, revolve, boolean, primitive, text,
                                   --   tracing_image, sketch_fillet, unit, offset.
                                   --   not chamfer/fillet (use edge_treatment or sketch_chamfer).
                                   --   `count` and `get` take the same set.
bearcad.find("name")
bearcad.set_name(el, "name")
bearcad.element("line", i)         -- or bearcad.element(id) / bearcad.element(name)
bearcad.id(el)                     -- el:id(): a stable id, unique and never reused
bearcad.line_endpoints(i)          -- x0, y0, x1, y1
bearcad.image_corners(i)           -- tracing image quad in world mm, live Move included
bearcad.body_stats(i)              -- volume, triangles, bbox = { min = {x,y,z}, max = {x,y,z} }
bearcad.body_faces(i)
bearcad.drawing_views(i)           -- a drawing's page: orientation, style, dimensions
bearcad.body_edges(i)
bearcad.body_cylinders(i)
bearcad.selection()
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
box:kind()  box:index()  box:id()  box:name()  box:exists()  box:delete()  tostring(box)
bearcad.delete(box)                -- or a list; does not replace the scene selection
bearcad.delete_selection()         -- whatever is selected (the GUI Delete)
```

Ordinals shift when elements are deleted, and a solid op consumes the body it acts on.
A handle does not: it names the same element until that element is gone, and then says so.
Anywhere an index is accepted — `bodies`, `polygon`, `extrusion`, `{ kind, index }`, … —
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
bearcad.ui.double_click(x, y)               -- opens a sketch / plane / dimension for editing
bearcad.ui.viewport()                       -- { width, height, x, y } of the area clicks address
bearcad.ui.right_click_ground(x, y)         -- opens a context menu
bearcad.ui.context_menu()                   -- { kind, index } of the open menu, or nil
bearcad.ui.key("enter")
bearcad.ui.palette("Export STEP")
bearcad.ui.begin_move{ … } / begin_combine / begin_joint   -- arm a tool; do not commit
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
                 takes — a name that is\nnot in this list is not a function. `{ … }` is one \
                 options table; `?` marks an\noptional argument or key. The sections above \
                 carry the detail for the calls they\ncover.\n\n```\n",
            );
            for name in names {
                // Every name must have a signature — the test below fails the build if one
                // is added without (#1664).
                match signature(&name) {
                    Some(sig) => out.push_str(sig),
                    None => out.push_str(&name),
                }
                out.push('\n');
            }
            out.push_str("```\n");
        }
    }
    out
}

/// The call shape published for `name` (`bearcad.line`, `bearcad.ui.camera`, …).
pub fn signature(name: &str) -> Option<&'static str> {
    crate::ai::signatures::SIGNATURES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, sig)| *sig)
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
            "no signature for {missing:?} — add them to src/ai/signatures.rs"
        );
        let stale: Vec<&str> = crate::ai::signatures::SIGNATURES
            .iter()
            .map(|(n, _)| *n)
            .filter(|n| !names.iter().any(|r| r == n))
            .collect();
        assert!(
            stale.is_empty(),
            "src/ai/signatures.rs lists {stale:?}, which is not a function"
        );
    }

    /// #1664: a signature must be a call of the name it belongs to — a copy/paste that
    /// keeps the wrong name would send an agent to a function that does not exist.
    #[test]
    fn every_signature_starts_with_its_own_name() {
        for (name, sig) in crate::ai::signatures::SIGNATURES {
            let next = sig[name.len()..].chars().next();
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
        for (name, sig) in crate::ai::signatures::SIGNATURES {
            // A trailing `-- note` may itself mention a nested table; the call's own keys
            // are what comes before it.
            let sig = sig.split("  --").next().unwrap_or(sig).trim_end();
            let Some(open) = sig.find("{ ") else { continue };
            let Some(close) = sig.rfind(" }") else { continue };
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
            assert!(doc.contains(call), "the page never shows how to call {call}…");
        }
        assert!(
            doc.contains("page fractions"),
            "drawing_text's x/y are not millimetres; the page must say so"
        );
    }

    #[test]
    fn cuboid_is_how_you_make_a_box() {
        let doc = document();
        assert!(doc.contains("bearcad.cuboid{"), "got: {}", &doc[..doc.len().min(400)]);
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
        assert!(checked > 20, "expected the guide to show real calls, saw {checked}");
    }
}
