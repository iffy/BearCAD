---
sidebar_position: 2
title: Declarative modeling
---

# Declarative modeling

The top-level `bearcad.*` table is the primary API: OpenSCAD-style, describe geometry
directly. These examples come from the project's test suite and `examples/`, so the syntax
is exercised by CI.

## A rectangle, extruded and exported

`examples/export_step.lua` end to end:

```lua
-- Run: cargo run -- --script examples/export_step.lua --exit

bearcad.new()

local sides = bearcad.rect{ width = 80, height = 50, name = "Base" }
local box = bearcad.extrude{ profiles = sides, distance = 20, name = "Block" }

bearcad.export_step("block.step")

-- A single body can be exported on its own (handle, id, name, or ordinal):
-- bearcad.export_step("block.step", box)

bearcad.quit()
```

`bearcad.export_stl(path, [body])` and `bearcad.export_3mf(path, [body])` work the same way
for mesh export. Whole-document `export_3mf` keeps each body as its own colored object
(material → 3MF `m:colorgroup`, one filament slot per color in Bambu Studio).
`bearcad.export_preview(path)` writes a Home zoom-to-fit PNG (same image embedded on save).

## Handles: what a call hands back

Every creation call returns what it made — one element, or a list.

```lua
local sides = bearcad.rect{ width = 80, height = 50 }   -- four lines; a profile
local box   = bearcad.extrude{ profiles = sides, distance = 10 }  -- the new body
```

Ordinals shift when elements are deleted, and a solid operation consumes the body it acts
on. A handle doesn't: it names the same element until that element is gone.

```lua
box:kind()      -- "body"
box:index()     -- its ordinal right now; an error once it's gone
box:id()        -- "body#3v0" — unique in the document, never reused
box:name()      -- its name, or nil
box:exists()    -- false once deleted
box:delete()    -- or `bearcad.delete(box)` / `bearcad.delete{ box, other }`
```

`bearcad.delete` does not require or replace the scene selection. `bearcad.delete_selection()` is the GUI-equivalent (whatever is selected).

A line handle names its vertices: `line:start()` or `line:endpoint("end")`.

Anywhere an index is accepted — `bodies`, `profiles`, `extrusion`, `{ kind, index }` — a
handle, its `id` string, or a name works too. `bearcad.element` is the lookup — `element(kind, index)`, `element(id)`, or
`element(name)`. `bearcad.find(name)` is sugar for a name (`nil` if missing).
`bearcad.get(handle)` (or `get{ kind, index }` / `get(kind, index)`) reads properties.
`bearcad.id(el)` is the method spelled as a function.

## Sketch, draw, and name elements

```lua
bearcad.new()
local sides = bearcad.rect{ width = 80, height = 50, name = "Main box" }

-- Named lookup:
bearcad.select(bearcad.find("Main box"))

-- A rect is four lines; rename one from the list it returned:
bearcad.set_name(sides[1], "Front edge")
```

Geometry helpers enter a ground-plane sketch automatically if none is open:

```lua
bearcad.rect{ width = 80, height = 50, x = 0, y = 0, name = "Box" }
bearcad.line{ length = 80, angle = 45, name = "Diagonal" }
bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0 } -- explicit endpoints
bearcad.circle{ x = 10, y = 5, r = 12, name = "Hole" } -- `radius` and `diameter` also accepted
bearcad.text{ text = "Hello", x = 10, y = 10, size = 12, rotation = 30, flip = true, wrap = 40 }
```

A scripted line lands **unconstrained**. To lock its length, pass `dimension`:

```lua
bearcad.line{ x = 0, y = 0, x1 = 50, y1 = 0, dimension = "leg" } -- expression (parameters work)
bearcad.line{ x = 0, y = 0, x1 = 50, y1 = 0, dimension = 50 }    -- plain number
bearcad.line{ x = 0, y = 0, x1 = 50, y1 = 0, dimension = true }  -- lock at the as-drawn length
```

Every dimension is the same ValueInput the GUI's fields are — a number, or a string with
parameters, arithmetic and units (`"w / 3"`, `"5in"`, `"45deg"`). A string stays live, so
the model rebuilds when the parameter changes:

```lua
bearcad.add_parameter("w", "24")
local sides = bearcad.rect{ width = "w", height = "w / 3" }
bearcad.circle{ x = 40, y = 0, diameter = "w" }        -- `r`/`radius` take expressions too
local box = bearcad.extrude{ profiles = sides, distance = "w / 2" }
bearcad.edit_extrusion{ extrusion = 0, distance = "w" }
bearcad.set_parameter("w", "30")                       -- everything above re-sizes
```

An expression that doesn't evaluate fails the call with an error naming it. Options tables
reject unrecognized keys — a typo fails immediately (catchable with `pcall`) with the
accepted keys listed:

```lua
bearcad.combine{ kind = "cut", a = {0}, b = {1} }
-- error: combine: unknown key `kind` (accepted keys: op, a, b, keep_b, bake, name)
```

To sketch on a specific plane — `0`, `1` and `2` are the default ground/front/side datum
planes; planes you create start at `3`:

```lua
bearcad.begin_sketch{ kind = "plane", index = 3 }
bearcad.rect{ width = 80, height = 50, name = "Main box" }
```

`begin_sketch` also accepts a body face — an extrusion's cap or side wall, or the flat
cap/washer faces of a revolve (a full-turn revolve has no end caps; sketch on its
`revolve_side` faces):

```lua
bearcad.begin_sketch{
  kind = "extrude_cap", extrusion = 0,
  profile = "polygon", profile_lines = sides, top = true,
}
bearcad.begin_sketch{
  kind = "revolve_side", revolution = 0, edge = 2,
  profile = "polygon", profile_lines = sides,
}
```

`profile` is `"circle"` (with `profile_index`), `"polygon"` (with `profile_lines`), or
`"boolean"` with the same descriptor `extrude`'s `boolean =` takes:

```lua
bearcad.begin_sketch{
  kind = "extrude_cap", extrusion = 0, top = true,
  profile = "boolean",
  boolean = { op = "difference", a = { polygon = sides }, b = { circle = hole } },
}
```

Create construction planes — offset from an existing plane, on a face, or pivoted
around an axis (same as the Plane tool):

```lua
bearcad.plane{ offset = 12 }                                       -- 12 mm above Ground ("0.5in" works too)
bearcad.plane{ offset = 5, from = 1 }                              -- offset from plane 1
bearcad.plane{ offset = 5, origin = {0, 0, 20}, normal = {0, 0, 1} } -- on a body face
bearcad.plane{ axis = "x", angle = 45 }                            -- around the world X axis
bearcad.plane{ axis = { line = 0 }, angle = 30, offset = 5 }        -- around a sketch line
```

Project outside 3D geometry into the open sketch as associative reference lines
(the [Projection](/docs/tools/projection) tool):

```lua
bearcad.project{ body = 0 }     -- every edge of body 0
bearcad.project{ plane = 2 }    -- where plane 2 crosses the sketch
bearcad.project()               -- current selection; un-projects if only projected lines
```

Re-open or leave a sketch without drawing:

```lua
bearcad.open_sketch(0)   -- re-enter sketch 0 to add more geometry to it
bearcad.exit_sketch()    -- leave the active sketch
```

## A closed polygon from plain lines, extruded

Any lines whose endpoints coincide exactly (no constraints needed) form an
extrudable face:

```lua
bearcad.new()
local a = bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0 }
local b = bearcad.line{ x = 10, y = 0, x1 = 5, y1 = 8 }
local c = bearcad.line{ x = 5, y = 8, x1 = 0, y1 = 0 }
bearcad.extrude{ profiles = {a, b, c}, distance = 6 }
```

`bearcad.sketch_faces(sketch?)` lists closed loops, circles, text glyphs, and plane regions
on a sketch (the open one, or every sketch if none is open). Each value is a `profiles`
operand:

```lua
local faces = bearcad.sketch_faces()
bearcad.extrude{ profiles = faces[1], distance = 6 }
bearcad.extrude{ profiles = faces, distance = 4, body = "join" }
```

`profiles` takes one circle handle, one line list (a `rect` return), a text handle,
a spec (`{circle=i}`, `{polygon={…}}`, `{text=i}`, `{text_glyph={text, glyph}}`,
`{region={sketch, u, v}}`, `{boolean={…}}`), or a list of those. `body` is still the
add/cut/join mode; `bodies` is still the target list.

```lua
bearcad.extrude{ profiles = { text = 0 }, distance = 1, body = "cut" }  -- engrave the whole word
```

## Push or pull a body face

`extrude_face` extrudes a bare face of an existing body — the scripted equivalent of
pulling it with the Extrude tool. Give the face the same way `begin_sketch` names one,
plus a `distance` (or a `to` target to snap onto another surface). `body = "cut"`
subtracts; `body = "add"` joins — both error if there's no body to cut or add into.
Profiles that don't touch make one body each; `body = "join"` puts them in a single body.
Positive `distance` extrudes along the face's outward normal; a cut that would miss the
body is flipped inward. A side wall's `edge` is the profile **line index**, stable even
when filleted edges sit between walls.

```lua
local sides = bearcad.rect{ x = 0, y = 0, width = 20, height = 20 }
bearcad.exit_sketch()
bearcad.extrude{ profiles = sides, distance = 20 }

-- Pull a side wall outward by 10 mm into a boss.
bearcad.extrude_face{
  face = { kind = "extrude_side", extrusion = 0, profile = "polygon", profile_lines = sides, edge = 0 },
  distance = 10, name = "Boss",
}

-- Or snap the pushed face onto another surface instead of a fixed distance.
bearcad.extrude_face{
  face = { kind = "extrude_cap", extrusion = 0, profile = "polygon", profile_lines = sides, top = true },
  to = { plane = 1 },
}
```

## Bezier curves

```lua
bearcad.line{
  x = 0, y = 0, x1 = 10, y1 = 0,
  bezier = { {3, 4}, {7, 4} },
  name = "Curve",
}
```

## Chamfer and fillet

In a sketch, both operate on a vertex where exactly two plain lines meet. `point` is one
corner; `points` treats several in **one** operation — the same rule as `edges` on
the solid verbs:

```lua
local corner = sides[1]:endpoint("end")
bearcad.chamfer_vertex{ point = corner, distance = 3 }
bearcad.fillet_vertex{ point = corner, radius = 3 }

bearcad.fillet_vertex{ points = {
  sides[1]:endpoint("end"),
  sides[2]:endpoint("end"),
}, radius = 3 }
```

On a solid, `fillet`/`chamfer` take the body and its edges — `body_edges` entries, a
handle, or the older analytic `{ kind = "vertical"|"top"|"bottom", face, edge }` form.
`fillet_edge`/`chamfer_edge` remain as aliases. Sketch on a face the same way:
`begin_sketch(box:face("top"))` or `begin_sketch(bearcad.body_faces(box)[1])`.

```lua
bearcad.fillet{ body = box, edges = bearcad.body_edges(box), radius = 8 }
bearcad.chamfer{
  body = box,
  edges = { { kind = "cap", face = 0, edge = 1, top = true } },
  distance = 3,
}
```

One call is one operation. Four separate one-edge calls stack four bodies; keep a set in
a single `edges` list. Analytic `extrusion=` / `primitive=` is still accepted.

## Constraints and parameters

```lua
bearcad.constrain("parallel", sides[1], sides[2])

bearcad.dimension{ kind = "line", index = sides[1], value = "25mm" }
-- `name = value` defines the parameter and dimensions with it, as in any value field:
bearcad.dimension{ kind = "line", index = sides[2], value = "leg = 40mm" }
-- Repeating `dimension` on an already-dimensioned line or circle updates the value.
bearcad.dimension{ kind = "angle", a = sides[1], b = sides[2], value = "90" }

local p = bearcad.add_parameter("A", "5mm")
bearcad.set_parameter("A", "A + 5in")        -- name or handle
bearcad.edit_parameter{ name = p, rename = "Len", private = true,
  min = "1mm", max = "100mm", step = "0.5mm" }  -- min+max ⇒ slider
bearcad.parameter_options("Len", true)       -- open the row's gear-options
bearcad.parameter_edit("Len", "min")         -- focus a bound field (Tab → max → step)
bearcad.parameter_editing()                  -- {name=, field="min"|"max"|"step"} or nil
bearcad.parameter_slider("Len")              -- {min=, max=, value=, step?} or nil
bearcad.parameter_slider("Len", 15)          -- set via the slider (mm / degrees, snapped)
bearcad.edit_parameter{ name = "Len", min = false }  -- clear a bound
bearcad.delete_parameter("Len")

bearcad.derive_parameter{ kind = "line_length", a = 0, name = "leg" }
bearcad.derive_parameter{ kind = "line_distance", a = 0, b = 1 }
bearcad.derive_parameter{ kind = "line_angle", a = 0, b = 2 }
bearcad.derive_parameter{ kind = "point_distance",
  a = { kind = "line", index = 0, endpoint = "start" },
  b = { kind = "line", index = 0, endpoint = "end" } }
-- Body geometry: a/b are mm points on the picked edge's ends or the corners.
bearcad.derive_parameter{ kind = "body_edge_length", body = 0, a = {0, 0, 0}, b = {30, 0, 0} }
bearcad.derive_parameter{ kind = "body_vertex_distance", body = 0,
  a = {0, 0, 0}, b = {30, 40, 0} }

bearcad.set_units{ length = "in", angle = "deg" }          -- document defaults
bearcad.set_units{ sketch = 0, length = "mm" }             -- per-sketch override
```

## Editing dimensions while drawing

`bearcad.ui.set_dim(axis, value)` sets a dimension field while a shape is being drawn —
`axis` is `"width"`/`"height"` (rect), `"length"` (line), `"diameter"` (circle), or
`"offset"`/`"angle"` (construction plane):

```lua
bearcad.ui.tool("rectangle")
bearcad.ui.click_ground(0, 0)
bearcad.ui.set_dim("width", "80")
bearcad.ui.set_dim("height", "50")
bearcad.ui.key("enter")
```

`edit_dim` re-opens a committed dimension: `"length"` for a line, `"width"`/`"height"`
for a rectangle's sides, `"diameter"` for a circle. Then `set_dim` + `commit_dim`:

```lua
bearcad.ui.edit_dim("length")
bearcad.ui.set_dim("length", "100")
bearcad.ui.commit_dim()
```

## Reading state back

Pure read-back getters let a script assert what it built. Reads never appear in recorded
scripts.

```lua
bearcad.new()
local sides = bearcad.rect{ width = 40, height = 30 }
local box = bearcad.extrude{ profiles = sides, distance = 10 }

assert(bearcad.count("line") == 4)             -- non-deleted entities per kind
-- kinds (`count` and `get` take the same set): line, circle, sketch, constraint,
--        plane, extrusion, shape, body, drawing, cross_section,
--        section_plane, parameter, sketch_text, component, image, joint
local l = bearcad.get{ kind = "line", index = sides[1] }
assert(l.x0 == 0 and math.abs(l.length - 40) < 1e-3)

-- A construction plane reports its drawn rectangle in its own u/v axes.
local e = bearcad.get{ kind = "plane", index = 0 }.extent
assert(e.u_min == 5 and e.u_max == 105)

local s = bearcad.body_stats(box)              -- volume / triangles / bbox of a body's mesh
assert(math.abs(s.volume - 40 * 30 * 10) < 120)
assert(s.bbox.max.z - s.bbox.min.z == 10)

-- A body's faces and edges, spelled the way a joint's mate takes them.
local f = bearcad.body_faces(box)[1]           -- { body, face = {x,y,z}, normal = {x,y,z} }
local v = bearcad.drawing_views(0)[1]          -- { orientation, style, bodies, pos_x, pos_y, … }
local e2 = bearcad.body_edges(box)[1]          -- { body, edge = { {x,y,z}, {x,y,z} } }
local c = bearcad.body_cylinders(box)[1]       -- a hole/boss: radius, length, and its axis

bearcad.select(sides[1])
assert(bearcad.selection()[1].kind == "line")  -- current scene selection
print(bearcad.status())                        -- the status-bar text
print(bearcad.session_log())                   -- what this run has done, timestamped
print(bearcad.version())                       -- Help → About identity

bearcad.add_parameter("A", "5mm")
assert(bearcad.parameter_value("A") == 5)     -- evaluated value (mm / degrees)
assert(bearcad.parameter_expression("A") == "5mm")
```

`get` returns `nil` for an out-of-range or deleted index. See also
`bearcad.sketch_dof()` / `bearcad.sketch_conflicts()` for solver introspection, and
[`bearcad.ui.camera{}`](./ui-namespace#camera) for the camera pose.

## Cross-section views

A cross-section view is a saved way of *looking* at the model — cutting planes that hide
what's in front of them — not a change to it. Views live under **Views** in the Elements
pane; **View → Create Cross Section** makes one. Each cutting plane is its own row under
the view.

```lua
local v = bearcad.cross_section{ name = "Front half" }   -- returns a handle; opens the view
assert(bearcad.ui.workbench() == "view")
bearcad.ui.workbench("model")                            -- back to the model
```

Creating a view (or double-clicking its row) opens the **View workbench**; the toolbar's
leftmost control names the workbench you're on and switches between them.

In that workbench the **Cutting plane** tool's Anchor picker hangs a plane on a face, plane,
edge, or axis — the same picks as a construction plane. A face gets an offset gizmo and two
in-plane tilts; an edge rotates around the edge. After a pick, Offset takes the keyboard.
**Enter** or the blue button hangs it on the open view. Double-click a hanging plane to edit
it. Visible views cut the model even in the modeling workbench; hide a view to lift its cut.

```lua
bearcad.section_plane{ plane = 1, offset = 5 }     -- on a construction plane's frame
bearcad.section_plane{ origin = {0, 0, 10}, normal = {0, 0, 1}, flip = true }
bearcad.ui.begin_edit_section_plane{ cut = 0 }        -- the live edit draft (Esc/Enter ends it)
bearcad.edit_section_plane{ cut = 0, offset = -2, roll = 30 }   -- slide and turn it
bearcad.delete_section_plane{ cut = 1 }
local cuts = bearcad.section_planes()              -- { origin, normal, offset, depth, roll,
                                                   --   flip, bodies ("all"/indices), excludes }
```

**Cut depth** bounds how far a plane reaches. Blank cuts all the way through; a length pairs
the plane with a second one that far behind it, facing back, so only the slab between the two
is hidden — a chunk out of the middle rather than a whole half.

```lua
bearcad.section_plane{ origin = {0, 0, 16}, normal = {0, 0, 1}, depth = 6 }
bearcad.edit_section_plane{ cut = 0, depth = false }   -- back to cutting through
local left = bearcad.section_stats(0)                  -- body 0 as the open view shows it:
                                                       -- { volume, triangles, bbox }
```

A plane takes every body unless you scope it (#1769): the tool's **Cut bodies** picker
turns All into an explicit list, and its **Exclude** picker spares bodies even so — each
picker has an **All bodies** switch. Scripts use `bodies` and `exclude_bodies`:

```lua
bearcad.edit_section_plane{ cut = 0, bodies = {1} }          -- cut only body 1
bearcad.edit_section_plane{ cut = 0, exclude_bodies = {1} }  -- or: everything but body 1
bearcad.edit_section_plane{ cut = 0, bodies = "all", exclude_bodies = false }  -- back to all
```

A technical drawing can import a view — the whole model cut, or just some bodies.
See [Drawings](./drawings) for the rest of the page API:

```lua
bearcad.drawing_view{ drawing = 0, cross_section = 0 }            -- the whole view
bearcad.drawing_view{ drawing = 0, body = 0, cross_section = 0 }  -- one body, cut by it
bearcad.drawing_view_section{ drawing = 0, view = 1, cross_section = false }  -- un-section
bearcad.drawing_style{ drawing = 0, style = "colorful" }          -- next projection on the page
bearcad.drawing_view_style{ drawing = 0, view = 0, style = "colorful" }  -- this view
-- visible | wireframe | shaded | colorful | loose_pencil | color_pencil | watercolor
```

## Materials

See [Materials](/docs/materials).

```lua
bearcad.material{ name = "Brass", color = "#c88a4a", bodies = {0} }
bearcad.material{ name = "Blue", bodies = {1} }   -- no color: applies the one already there
bearcad.set_material{ body = 1, material = 0 }
bearcad.set_material{ body = 1 }        -- back to the default material
```

`set_material` names a material by its order in the document. Unobtainium is `0`.

## Visibility, construction, and shadow bodies

```lua
bearcad.set_visible(box, false)        -- handle, list, or { kind = "plane" }; boolean only
bearcad.set_visible({ kind = "plane" }, false)
bearcad.visible(box)                   -- read it back
bearcad.set_construction(box, true)
bearcad.ui.toggle_visibility()         -- current selection
-- Shadow body: hidden in the viewport (except hover/select) and omitted from export.
bearcad.set_body_shadow{ body = 0, shadow = true }
bearcad.set_body_shadow{ body = 0, shadow = false }  -- back to a live body
bearcad.get("body", 0).shadow          -- true for a consumed operation input
bearcad.visible(box)                   -- false for a shadow body
```

An operation leaves its inputs behind as shadow bodies, so `count("body")` — which has to
match the ordinal space `element`/`get` index — counts them too. `live_body` is the same
arena with the shadows skipped:

```lua
bearcad.count("live_body")             -- bodies that are really in the scene
bearcad.element("live_body", 0)
bearcad.get("live_body", 0)
```

## Components

```lua
local frame = bearcad.component{ name = "Frame" }          -- returns a handle
local legs  = bearcad.component{ name = "Legs", parent = frame }
bearcad.move_to_component{ kind = "extrusion", index = 0, component = frame }
bearcad.move_to_component{ kind = "body", index = 0, component = false }  -- back to root
bearcad.set_units{ component = frame, length = "in" }
bearcad.select{ kind = "component", index = frame }
bearcad.count("component")
```

## Import

```lua
bearcad.new()
bearcad.import_stl("part.stl")
bearcad.import_step("part.step")

-- Another BearCAD document as a unit (see Files → Importing BearCAD files):
-- embeds one copy, adds an instance named after the file stem.
bearcad.import_unit("bracket.bearcad")
bearcad.import_unit{ path = "bracket.bearcad", link = "static", name = "left_bracket" }
bearcad.add_unit_instance{ unit = 0, name = "right_bracket" }
bearcad.clone_unit_instance{ instance = 0 }                 -- another instance, same overrides
bearcad.set_unit_parameter{ instance = 1, name = "width", expression = "20" }
bearcad.set_unit_parameter{ instance = 1, name = "width" }   -- back to the part's value
bearcad.unit_link(0, "dynamic")                              -- or "static"
bearcad.sync_unit(0)                                         -- update from the source now
bearcad.select{ kind = "unit_instance", index = 1 }          -- or by instance name

-- Tracing images (see the Tracing images tool page): PNG/JPEG onto a
-- construction plane (default: ground), centered, seeded at 1 px = 1 mm.
bearcad.import_image{ path = "drawing.png" }
bearcad.import_image{ path = "drawing.png", plane = 1 }
bearcad.begin_sketch{ kind = "image", index = 0 }

-- Scale calibration: a selected image already has a top-middle → bottom-middle
-- line. Drag its endpoints, or set them here, then assign a real length
-- (any expression). The image rescales about the span midpoint.
bearcad.calibrate_image{ image = 0, from = { -100, -120 }, to = { 100, -120 }, length = 50 }
bearcad.calibrate_image{ image = 0, length = "2 * scale" }   -- current line
local img = bearcad.get{ kind = "image", index = 0 }        -- plane/from/to/length/expression/opacity
bearcad.image_opacity{ image = 0, opacity = 0.5 }          -- 0..1; expressions ok
```

STEP export writes real BREP and import reads it back, curved/NURBS surfaces
included.

## Document lifecycle

```lua
bearcad.new()
bearcad.open("path/to/file.bearcad")
bearcad.save()                 -- Save
bearcad.save("other.bearcad")  -- Save As
bearcad.count("body")          -- live
bearcad.count_saved("body")    -- last committed file
bearcad.clear()
bearcad.undo()
bearcad.rebuild_geometry()     -- File → Rebuild Geometry: drop tessellation cache
bearcad.export_preview("preview.png")  -- Home zoom-to-fit PNG (also embedded on save)
bearcad.quit()                 -- close the app when the script ends
```
