---
sidebar_position: 2
title: Declarative modeling
---

# Declarative modeling

The top-level `bearcad.*` table is the primary API: OpenSCAD-style, describe geometry directly.
These examples are adapted from the project's own Lua test suite (`src/lua_script.rs`) and the
example scripts under `examples/`, so the syntax shown here is exercised by CI.

## A rectangle, extruded and exported

This is `examples/export_step.lua` end to end:

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

`bearcad.export_stl(path, [body])` works the same way for STL. Both mirror the GUI's
**File → Export STL…** / STEP export, and export just one body if a name is given (matching what
right-clicking a body row in the Elements pane does).

## Sketch, draw, and name elements

```lua
bearcad.new()
bearcad.rect{ width = 80, height = 50, name = "Main box" }

-- Named lookup:
local box = bearcad.find("Main box")
bearcad.select(box)

-- Rename after the fact, or name a piece created without a `name` field. A rect is four
-- lines, so its edges are addressable individually:
bearcad.set_name(bearcad.element("line", 0), "Front edge")
```

Geometry-creation helpers are single calls that enter a ground-plane sketch automatically if none
is open — no simulated mouse/keyboard required:

```lua
bearcad.rect{ width = 80, height = 50, x = 0, y = 0, name = "Box" }
bearcad.line{ length = 80, angle = 45, name = "Diagonal" }
bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0 } -- explicit endpoints
bearcad.circle{ x = 10, y = 5, r = 12, name = "Hole" } -- `radius` and `diameter` also accepted
bearcad.text{ text = "Hello", x = 10, y = 10, size = 12 } -- see the Text tool page
```

A scripted line lands **unconstrained**, exactly like clicking with the Line tool. To lock its
length — the scripted equivalent of typing a length while drawing — pass `dimension`:

```lua
bearcad.line{ x = 0, y = 0, x1 = 50, y1 = 0, dimension = "leg" } -- expression (parameters work)
bearcad.line{ x = 0, y = 0, x1 = 50, y1 = 0, dimension = 50 }    -- plain number
bearcad.line{ x = 0, y = 0, x1 = 50, y1 = 0, dimension = true }  -- lock at the as-drawn length
```

Sizes accept **parameter expressions** anywhere the GUI's dimension fields do: pass a string
instead of a number and the expression is stored, so the model rebuilds when the parameter
changes — exactly like typing it into the field:

```lua
bearcad.parameter("add", "w", "24")
bearcad.rect{ width = "w", height = "w / 3" }
bearcad.circle{ x = 40, y = 0, diameter = "w" }        -- `r`/`radius` take expressions too
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = "w / 2" }
bearcad.edit_extrusion{ extrusion = 0, distance = "w" }
bearcad.parameter("value", 0, "30")                    -- everything above re-sizes
```

An expression that doesn't evaluate (unknown parameter, bad syntax) fails the call with an
error naming it.

Calls that take an options table also **reject unrecognized keys**, so a typo fails
immediately with the accepted keys listed (catchable with `pcall`) instead of being silently
ignored and confusing a later step:

```lua
bearcad.combine{ kind = "cut", a = {0}, b = {1} }
-- error: combine: unknown key `kind` (accepted keys: op, a, b, keep_b, name)
```

To sketch on a specific plane instead of the default ground plane:

```lua
bearcad.begin_sketch("construction_plane", 0)
bearcad.rect{ width = 80, height = 50, name = "Main box" }
```

`begin_sketch` also accepts a 3D body face — an extrusion's own cap or side wall — so sketching
on an existing solid is scriptable too:

```lua
bearcad.begin_sketch{
  kind = "extrude_cap", extrusion = 0,
  profile = "polygon", profile_lines = {0, 1, 2, 3}, top = true,
}
```

`profile` is `"circle"` (with `profile_index`), `"polygon"` (with `profile_lines`), or
`"boolean"` with the same descriptor `extrude`'s `boolean =` takes — so the cap of a
boolean-combined extrusion hosts a sketch too:

```lua
bearcad.begin_sketch{
  kind = "extrude_cap", extrusion = 0, top = true,
  profile = "boolean",
  boolean = { op = "difference", a = { polygon = {0, 1, 2, 3} }, b = { circle = 0 } },
}
```

Re-open an existing sketch, or leave the active one, without drawing anything:

```lua
bearcad.open_sketch(0)   -- re-enter sketch 0 to add more geometry to it
bearcad.exit_sketch()    -- leave the active sketch
```

## A closed polygon from plain lines, extruded

Any set of plain lines that connects end-to-end into a closed loop is a usable, extrudable face —
see [point-level selection](./point-selection) for how to close the loop purely from a script:

```lua
bearcad.new()
bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0 }
bearcad.line{ x = 10, y = 0, x1 = 5, y1 = 8 }
bearcad.line{ x = 5, y = 8, x1 = 0, y1 = 0 }
bearcad.extrude{ polygon = {0, 1, 2}, distance = 6 }
```

## Push or pull a body face

Extrude a bare face of an existing body directly — the scripted equivalent of clicking the
face with the Extrude tool and pulling it. Give the face the same way `begin_sketch` names a
body face, then a `distance` (or a `to` target to snap onto another surface). `body = "cut"`
subtracts instead of adding; `body = "merge"` joins the face's body. Both require the sketch
to be on a body face — if there's no body to cut or merge into, the call errors instead of
quietly making a separate new body. A positive `distance` extrudes **along the face's
outward normal** (away from the body); a cut whose tool would miss the body entirely is
automatically flipped inward, and a cut that can't remove material in either direction
commits with a status warning. A side wall's `edge` is the profile **line index** (line
`0` of the loop is `edge = 0`), so every flat wall is reachable by a stable number even when
the profile has curved (filleted) edges between walls.

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

`bearcad.set_dim(axis, value)` sets a dimension field while a rectangle/line/circle/plane is
being drawn — `axis` is `"width"`/`"height"` (rect), `"length"` (line), `"diameter"` (circle), or
`"offset"`/`"angle"` (construction plane):

```lua
bearcad.ui.tool("rectangle")
bearcad.ui.click_ground(0, 0)
bearcad.set_dim("width", "80")
bearcad.set_dim("height", "50")
bearcad.ui.key("enter")
```

`edit_dim("length")` re-opens a committed plain line's length label; set it and commit:

```lua
bearcad.edit_dim("length")
bearcad.set_dim("length", "100")
bearcad.commit_dim()
```

## Reading state back

The API isn't write-only: a set of pure read-back getters lets a script assert what it built —
handy for regression tests written entirely in Lua. Reads never appear in recorded scripts.

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

`get` returns `nil` for an index that is out of range or deleted. See also
`bearcad.sketch_dof()` / `bearcad.sketch_conflicts()` for constraint-solver introspection, and
[`bearcad.ui.camera{}`](./ui-namespace#camera) for reading the camera pose.

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

With the OCCT kernel compiled in (the standard build), STEP export writes **real BREP** (planar
and curved surfaces) and STEP import reads it back, curved/NURBS surfaces included — files from
other CAD tools round-trip. A `--no-default-features` build uses a faceted STEP path instead:
triangulated export, and import of that same planar subset (curved `ADVANCED_FACE` files are
rejected with a clear error rather than approximated).

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
