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

bearcad.rect{ width = 80, height = 50, name = "Base" }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20, name = "Block" }

bearcad.export_step("block.step")

-- A single named body can be exported on its own:
-- bearcad.export_step("block.step", "Block")

bearcad.quit()
```

`bearcad.export_stl(path, [body])` and `bearcad.export_3mf(path, [body])` work the same way
for mesh export. Whole-document `export_3mf` keeps each body as its own coloured object
(material → 3MF `m:colorgroup`, one filament slot per colour in Bambu Studio).
`bearcad.export_preview(path)` writes a Home zoom-to-fit PNG (same image embedded on save).

## Sketch, draw, and name elements

```lua
bearcad.new()
bearcad.rect{ width = 80, height = 50, name = "Main box" }

-- Named lookup:
local box = bearcad.find("Main box")
bearcad.select(box)

-- Rename anything. A rect is four lines, so its edges are addressable individually:
bearcad.set_name(bearcad.element("line", 0), "Front edge")
```

Geometry helpers enter a ground-plane sketch automatically if none is open:

```lua
bearcad.rect{ width = 80, height = 50, x = 0, y = 0, name = "Box" }
bearcad.line{ length = 80, angle = 45, name = "Diagonal" }
bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0 } -- explicit endpoints
bearcad.circle{ x = 10, y = 5, r = 12, name = "Hole" } -- `radius` and `diameter` also accepted
bearcad.text{ text = "Hello", x = 10, y = 10, size = 12 } -- see the Text tool page
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
bearcad.parameter("add", "w", "24")
bearcad.rect{ width = "w", height = "w / 3" }
bearcad.circle{ x = 40, y = 0, diameter = "w" }        -- `r`/`radius` take expressions too
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = "w / 2" }
bearcad.edit_extrusion{ extrusion = 0, distance = "w" }
bearcad.parameter("value", 0, "30")                    -- everything above re-sizes
```

An expression that doesn't evaluate fails the call with an error naming it. Options tables
reject unrecognized keys — a typo fails immediately (catchable with `pcall`) with the
accepted keys listed:

```lua
bearcad.combine{ kind = "cut", a = {0}, b = {1} }
-- error: combine: unknown key `kind` (accepted keys: op, a, b, keep_b, keep_leftovers, name)
```

To sketch on a specific plane — `0`, `1` and `2` are the default ground/front/side datum
planes; planes you create start at `3`:

```lua
bearcad.begin_sketch("construction_plane", 3)
bearcad.rect{ width = 80, height = 50, name = "Main box" }
```

`begin_sketch` also accepts a body face — an extrusion's cap or side wall, or the flat
cap/washer faces of a revolve (a full-turn revolve has no end caps; sketch on its
`revolve_side` faces):

```lua
bearcad.begin_sketch{
  kind = "extrude_cap", extrusion = 0,
  profile = "polygon", profile_lines = {0, 1, 2, 3}, top = true,
}
bearcad.begin_sketch{
  kind = "revolve_side", revolution = 0, edge = 2,
  profile = "polygon", profile_lines = {0, 1, 2, 3},
}
```

`profile` is `"circle"` (with `profile_index`), `"polygon"` (with `profile_lines`), or
`"boolean"` with the same descriptor `extrude`'s `boolean =` takes:

```lua
bearcad.begin_sketch{
  kind = "extrude_cap", extrusion = 0, top = true,
  profile = "boolean",
  boolean = { op = "difference", a = { polygon = {0, 1, 2, 3} }, b = { circle = 0 } },
}
```

Create construction planes — offset from an existing plane, or anchored on any face
by its origin and normal:

```lua
bearcad.plane{ offset = 12 }                                       -- 12 mm above Ground ("0.5in" works too)
bearcad.plane{ offset = 5, from = 1 }                              -- offset from plane 1
bearcad.plane{ offset = 5, origin = {0, 0, 20}, normal = {0, 0, 1} } -- on a body face
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
bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0 }
bearcad.line{ x = 10, y = 0, x1 = 5, y1 = 8 }
bearcad.line{ x = 5, y = 8, x1 = 0, y1 = 0 }
bearcad.extrude{ polygon = {0, 1, 2}, distance = 6 }
```

## Push or pull a body face

`extrude_face` extrudes a bare face of an existing body — the scripted equivalent of
pulling it with the Extrude tool. Give the face the same way `begin_sketch` names one,
plus a `distance` (or a `to` target to snap onto another surface). `body = "cut"`
subtracts; `body = "merge"` joins — both error if there's no body to cut or merge into.
Profiles that don't touch make one body each; `body = "join"` puts them in a single body.
Positive `distance` extrudes along the face's outward normal; a cut that would miss the
body is flipped inward. A side wall's `edge` is the profile **line index**, stable even
when filleted edges sit between walls.

```lua
bearcad.rect{ x = 0, y = 0, width = 20, height = 20 }
bearcad.exit_sketch()
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }

-- Pull a side wall outward by 10 mm into a boss.
bearcad.extrude_face{
  face = { kind = "extrude_side", extrusion = 0, profile = "polygon", profile_lines = {0, 1, 2, 3}, edge = 0 },
  distance = 10, name = "Boss",
}

-- Or snap the pushed face onto another surface instead of a fixed distance.
bearcad.extrude_face{
  face = { kind = "extrude_cap", extrusion = 0, profile = "polygon", profile_lines = {0, 1, 2, 3}, top = true },
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

Both operate on a sketch vertex where exactly two plain lines meet. `point` is one
corner; `points` treats several in **one** operation — the same rule as `edges` on
the solid verbs:

```lua
local corner = { kind = "line", index = 0, ["end"] = "end" }
bearcad.chamfer_vertex{ point = corner, distance = 3 }
bearcad.fillet_vertex{ point = corner, radius = 3 }

bearcad.fillet_vertex{ points = {
  { kind = "line", index = 0, ["end"] = "end" },
  { kind = "line", index = 1, ["end"] = "end" },
}, radius = 3 }
```

On a solid, `chamfer_edge`/`fillet_edge` take an analytic edge of an extrusion or a
Shape-tool cuboid (`primitive = 0` instead of `extrusion = 0`) — a vertical
edge between two side walls, or a cap edge where a side wall meets the top or base:

```lua
bearcad.fillet_edge{
  extrusion = 0,
  edge = { kind = "vertical", face = 0, edge = 2 },
  radius = 8,
}
bearcad.chamfer_edge{
  extrusion = 0,
  edge = { kind = "cap", face = 0, edge = 1, top = true },
  distance = 3,
}
```

Rounding several edges at once takes **one call** with `edges`, matching what a Shift+click
multi-edge commit does in the app:

```lua
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
```

One call is one operation, and an operation bevels the body its extrusion built. Four
separate one-edge calls would each round that same sharp box and leave four bodies sitting
on top of each other, so keep a set in a single call. An entry may name its own
`extrusion = i, edge = {...}` to span several extrusions in the one operation.

## Constraints and parameters

```lua
bearcad.select{ kind = "line", index = 0 }
bearcad.select({ kind = "line", index = 1 }, true)
bearcad.add_geometric_constraint("parallel")

bearcad.add_constraint({ kind = "line", index = 0 }, "25mm")
-- `name = value` defines the parameter and dimensions with it, as in any value field:
bearcad.add_constraint({ kind = "line", index = 1 }, "leg = 40mm")
-- Repeating `add_constraint` on an already-dimensioned line or circle updates the value.

bearcad.parameter("add", "A", "5mm")
bearcad.parameter("value", 0, "A + 5in")
bearcad.parameter("name", 0, "Len")     -- rename parameter 0
bearcad.parameter("delete", 0)
```

## Editing dimensions while drawing

`bearcad.set_dim(axis, value)` sets a dimension field while a shape is being drawn —
`axis` is `"width"`/`"height"` (rect), `"length"` (line), `"diameter"` (circle), or
`"offset"`/`"angle"` (construction plane):

```lua
bearcad.ui.tool("rectangle")
bearcad.ui.click_ground(0, 0)
bearcad.set_dim("width", "80")
bearcad.set_dim("height", "50")
bearcad.ui.key("enter")
```

`edit_dim` re-opens a committed dimension: `"length"` for a line, `"width"`/`"height"`
for a rectangle's sides, `"diameter"` for a circle. Then `set_dim` + `commit_dim`:

```lua
bearcad.edit_dim("length")
bearcad.set_dim("length", "100")
bearcad.commit_dim()
```

## Reading state back

Pure read-back getters let a script assert what it built. Reads never appear in recorded
scripts.

```lua
bearcad.new()
bearcad.rect{ width = 40, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }

assert(bearcad.count("line") == 4)             -- non-deleted entities per kind
-- kinds (`count` and `get` take the same set): line, circle, sketch, constraint,
--        construction_plane, extrusion, shape, body, drawing, cross_section,
--        section_plane, parameter, sketch_text, component, image, joint
local l = bearcad.get{ kind = "line", index = 0 }
assert(l.x0 == 0 and math.abs(l.length - 40) < 1e-3)

-- A construction plane reports its drawn rectangle in its own u/v axes.
local e = bearcad.get{ kind = "construction_plane", index = 0 }.extent
assert(e.u_min == 5 and e.u_max == 105)

local s = bearcad.body_stats(0)                -- volume / triangles / bbox of a body's mesh
assert(math.abs(s.volume - 40 * 30 * 10) < 120)
assert(s.bbox.max[3] - s.bbox.min[3] == 10)

-- A body's faces and edges, spelled the way a joint's mate takes them.
local f = bearcad.body_faces(0)[1]             -- { body, face = {x,y,z}, normal = {x,y,z} }
local v = bearcad.drawing_views(0)[1]          -- { orientation, style, dimensions, pos_x, pos_y, … }
local e2 = bearcad.body_edges(0)[1]            -- { body, edge = { {x,y,z}, {x,y,z} } }
local c = bearcad.body_cylinders(0)[1]         -- a hole/boss: radius, length, and its axis

bearcad.select{ kind = "line", index = 0 }
assert(bearcad.selection()[1].kind == "line")  -- current scene selection
print(bearcad.status())                        -- the status-bar text
print(bearcad.session_log())                   -- what this run has done, timestamped
print(bearcad.version())                       -- Help → About identity

bearcad.parameter("add", "A", "5mm")
assert(bearcad.parameter("get", "A") == 5)     -- evaluated value (mm / degrees)
assert(bearcad.parameter("get_expression", "A") == "5mm")
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
local v = bearcad.cross_section{ name = "Front half" }   -- returns its index; opens the view
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
bearcad.begin_edit_section_plane{ cut = 0 }        -- the live edit draft (Esc/Enter ends it)
bearcad.edit_section_plane{ cut = 0, offset = -2, roll = 30 }   -- slide and turn it
bearcad.delete_section_plane{ cut = 1 }
local cuts = bearcad.section_planes()              -- { origin, normal, offset, roll, flip,
                                                   --   bodies ("all" or indices), excludes }
```

A plane takes every body unless you scope it (#1769): the tool's **Cut bodies** picker
turns All into an explicit list, and its **Exclude** picker spares bodies even so — each
picker has an **All bodies** switch. Scripts use `bodies` and `exclude_bodies`:

```lua
bearcad.edit_section_plane{ cut = 0, bodies = {1} }          -- cut only body 1
bearcad.edit_section_plane{ cut = 0, exclude_bodies = {1} }  -- or: everything but body 1
bearcad.edit_section_plane{ cut = 0, bodies = "all", exclude_bodies = false }  -- back to all
```

A technical drawing can import a view — the whole model cut, or just some bodies:

```lua
bearcad.drawing_view{ drawing = 0, cross_section = 0 }            -- the whole view
bearcad.drawing_view{ drawing = 0, body = 0, cross_section = 0 }  -- one body, cut by it
bearcad.drawing_view_section{ drawing = 0, view = 1, cross_section = false }  -- un-section
```

## Materials

See [Materials](/docs/materials).

```lua
bearcad.material{ name = "Brass", color = "#c88a4a", bodies = {0} }
bearcad.set_material{ body = 1, material = 0 }
bearcad.set_material{ body = 1 }        -- back to the default material
```

## Visibility, construction, and shadow bodies

```lua
bearcad.set_visible(box, "hide")       -- "show" | "hide" | "toggle"
bearcad.set_construction(box, true)
-- Shadow body: hidden in the viewport (except hover/select) and omitted from export.
bearcad.set_body_shadow{ body = 0, shadow = true }
bearcad.set_body_shadow{ body = 0, shadow = false }  -- back to a live body
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
bearcad.sqlite_scalar("SELECT COUNT(*) FROM bodies")  -- last committed file
bearcad.session_writes()       -- last incremental flush: { bodies = { inserts = 1, ... } }
bearcad.clear()
bearcad.undo()
bearcad.quit()                 -- close the app when the script ends
```
