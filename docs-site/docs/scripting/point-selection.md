---
sidebar_position: 4
title: Point-level selection
---

# Point-level selection

`bearcad.select` normally targets a whole element. Point-level selection targets an
individual **vertex** instead — a line endpoint or a circle's center — using the same
point numbering the interactive [Constraint](/docs/tools/constraint) tool uses.

## Selecting a line endpoint

```lua
bearcad.select{ kind = "line", index = 0, endpoint = "start" }  -- or "end"
```

A line handle names the same points: `line:start()` or `line:endpoint("end")`.

## Selecting a rectangle corner

A rect is four separate lines, so a corner is a line endpoint. Lines run
counterclockwise from the `(x, y)` origin corner — bottom, right, top, left —
and each corner is the `start` of that side:

```lua
local sides = bearcad.rect{ width = 80, height = 50 }
bearcad.select(sides[3]:start())  -- the third corner (top-right)
```

## Selecting a circle's center

`kind = "circle"` alone selects the whole circle. Pass `point = true` for its center:

```lua
bearcad.select{ kind = "circle", index = 0, point = true }
```

## Selecting a text's anchor

A sketch text has nine anchor points — box corners, edge midpoints, center. Pass `anchor`
to select one (without it, `sketch_text` selects the whole text):

```lua
bearcad.select{ kind = "sketch_text", index = 0, anchor = "center" }  -- or "top_left", …
```

Constraining an anchor translates the whole text to satisfy it; rotation and size never
change.

## Selecting an image's box point, edge, or calibration point

A tracing image on the sketch's plane has nine box points (corners, edge midpoints,
centre), four edges, and two calibration points. Constraining a box or calibration
point translates the whole image (scale never changes); an edge is a fixed reference
that sketch geometry constrains onto:

```lua
bearcad.select{ kind = "image", index = 0, anchor = "center" }   -- or "bottom_left", …
bearcad.select{ kind = "image", index = 0, edge = "left" }       -- or "right"/"top"/"bottom"
bearcad.select{ kind = "image", index = 0, point = 0 }           -- calibration 0 or 1
```

## Selecting a face's own vertex or edge

While a sketch is open directly on a body's face (an extrusion cap or side wall), that
face's boundary loop is selectable, so the sketch can be constrained against the face
it's drawn on:

```lua
bearcad.select{
    kind = "face",
    face = { kind = "extrude_cap", extrusion = 0, profile = "polygon", profile_lines = sides, top = true },
    index = 2,
}
```

`face` takes the same table shape as [`bearcad.begin_sketch`](./declarative-modeling);
`index` numbers the boundary loop. This selects the vertex; add `edge = true` for the
edge from that corner to the next:

```lua
bearcad.select{
    kind = "face",
    face = { kind = "extrude_side", extrusion = 0, profile = "polygon", profile_lines = sides, edge = 0 },
    index = 0,
    edge = true,
}
```

Both are fixed by the body's geometry — not draggable — but plug into `Coincident`,
`Midpoint`, and distance constraints like any sketch point/line. Only the sketch's own
face is pickable; imported STL/STEP bodies have no analytic boundary to reference.

## Selecting the origin and its axes

Constrain a point onto an axis (pins that coordinate to 0) or onto the origin:

```lua
local a = bearcad.line{ x = 5, y = 5, x1 = 12, y1 = 8 }
bearcad.constrain("coincident", a:start(), { kind = "axis", axis = "x" })
bearcad.constrain("coincident", a:endpoint("end"), { kind = "origin" })
```

A dimension from the origin to a circle's centre (how far a hole sits from a circular
face's centre) is the same `{ kind = "origin" }` table:

```lua
bearcad.dimension{ kind = "point_point",
                   anchor = { kind = "origin" },
                   mover  = { kind = "circle", index = 0 },
                   value = "12mm" }
```

Interactively, dragging a point near an axis or the origin snaps it on and adds the same
constraint.

## Making two lines collinear

```lua
bearcad.constrain("coincident",
  { kind = "line", index = 0 }, { kind = "line", index = 1 })
```

## Additive selection

Pass `true` as the second argument to add to the current selection instead of replacing
it:

```lua
bearcad.select({ kind = "line", index = 1 }, true)
```

## Worked example: closing a polygon loop purely from a script

```lua
bearcad.new()
local a = bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0, name = "a" }
local b = bearcad.line{ x = 20, y = 0, x1 = 30, y1 = 0, name = "b" }
bearcad.constrain("coincident", a:endpoint("end"), b:start())
```

Combine with
[`bearcad.extrude{ profiles = {a, b, c} }`](./declarative-modeling#a-closed-polygon-from-plain-lines-extruded)
to build and extrude an arbitrary closed profile without any GUI interaction.
