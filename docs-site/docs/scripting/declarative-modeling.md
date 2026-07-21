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

`bearcad.export_stl(path, [body])` works the same way for STL.

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

Sizes accept **parameter expressions** anywhere the GUI's dimension fields do — pass a
string and the model rebuilds when the parameter changes:

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
-- error: combine: unknown key `kind` (accepted keys: op, a, b, keep_b, name)
```

To sketch on a specific plane:

```lua
bearcad.begin_sketch("construction_plane", 0)
bearcad.rect{ width = 80, height = 50, name = "Main box" }
```

`begin_sketch` also accepts a body face — an extrusion's cap or side wall:

```lua
bearcad.begin_sketch{
  kind = "extrude_cap", extrusion = 0,
  profile = "polygon", profile_lines = {0, 1, 2, 3}, top = true,
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
bearcad.plane{ offset = 12 }                                       -- 12 mm above Ground
bearcad.plane{ offset = 5, from = 1 }                              -- offset from plane 1
bearcad.plane{ offset = 5, origin = {0, 0, 20}, normal = {0, 0, 1} } -- on a body face
```

Re-open or leave a sketch without drawing:

```lua
bearcad.open_sketch(0)   -- re-enter sketch 0 to add more geometry to it
bearcad.exit_sketch()    -- leave the active sketch
```

## A closed polygon from plain lines, extruded

Any lines that connect end-to-end into a closed loop form an extrudable face — see
[point-level selection](./point-selection) for closing the loop from a script:

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

Both operate on a sketch vertex where exactly two plain lines meet:

```lua
local corner = { kind = "line", index = 0, ["end"] = "end" }
bearcad.chamfer_vertex{ point = corner, distance = 3 }
-- or:
bearcad.fillet_vertex{ point = corner, radius = 3 }
```

## Constraints and parameters

```lua
bearcad.select{ kind = "line", index = 0 }
bearcad.select({ kind = "line", index = 1 }, true)
bearcad.add_geometric_constraint("parallel")

bearcad.add_constraint({ kind = "line", index = 0 }, "25mm")

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

`edit_dim("length")` re-opens a committed line's length label:

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
-- kinds: line, circle, sketch, constraint, construction_plane, extrusion,
--        body, drawing, parameter, sketch_text, image
local l = bearcad.get{ kind = "line", index = 0 }
assert(l.x0 == 0 and math.abs(l.length - 40) < 1e-3)

local s = bearcad.body_stats(0)                -- volume / triangles / bbox of a body's mesh
assert(math.abs(s.volume - 40 * 30 * 10) < 120)
assert(s.bbox.max[3] - s.bbox.min[3] == 10)

bearcad.select{ kind = "line", index = 0 }
assert(bearcad.selection()[1].kind == "line")  -- current scene selection
print(bearcad.status())                        -- the status-bar text

bearcad.parameter("add", "A", "5mm")
assert(bearcad.parameter("get", "A") == 5)     -- evaluated value (mm / radians)
assert(bearcad.parameter("get_expression", "A") == "5mm")
```

`get` returns `nil` for an out-of-range or deleted index. See also
`bearcad.sketch_dof()` / `bearcad.sketch_conflicts()` for solver introspection, and
[`bearcad.ui.camera{}`](./ui-namespace#camera) for the camera pose.

## Visibility and construction geometry

```lua
bearcad.set_visible(box, "hide")       -- "show" | "hide" | "toggle"
bearcad.set_construction(box, true)
```

## Import

```lua
bearcad.new()
bearcad.import_stl("part.stl")
bearcad.import_step("part.step")

-- Tracing images (see the Tracing images tool page): PNG/JPEG onto a
-- construction plane (default: ground), centered, seeded at 1 px = 1 mm.
bearcad.import_image{ path = "drawing.png" }
bearcad.import_image{ path = "drawing.png", plane = 1 }

-- Scale calibration: mark a feature of known size (plane-local mm at the
-- image's current scale) and declare its real length; the image rescales
-- uniformly about the span's midpoint.
bearcad.calibrate_image{ image = 0, from = { -100, -120 }, to = { 100, -120 }, length = 50 }
```

STEP export writes real BREP and import reads it back, curved/NURBS surfaces
included.

## Document lifecycle

```lua
bearcad.new()
bearcad.open("path/to/file.bearcad")
bearcad.save()                 -- Save
bearcad.save("other.bearcad")  -- Save As
bearcad.clear()
bearcad.undo()
bearcad.quit()                 -- close the app when the script ends
```
