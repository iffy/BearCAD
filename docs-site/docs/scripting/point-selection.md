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
bearcad.select{ kind = "line", index = 0, ["end"] = "start" }  -- or "end"
```

## Selecting a rectangle corner

A rect is four separate lines, so a corner is a line endpoint. Lines run
counterclockwise from the `(x, y)` origin corner — 0 bottom, 1 right, 2 top, 3 left —
and each corner is the `start` of the line with the same number:

```lua
bearcad.select{ kind = "line", index = 2, ["end"] = "start" }  -- the third corner (top-right)
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

## Selecting a face's own vertex or edge

While a sketch is open directly on a body's face (an extrusion cap or side wall), that
face's boundary loop is selectable, so the sketch can be constrained against the face
it's drawn on:

```lua
bearcad.select{
    kind = "face",
    face = { kind = "extrude_cap", extrusion = 0, profile = "polygon", profile_lines = { 0, 1, 2, 3 }, top = true },
    index = 2,
}
```

`face` takes the same table shape as [`bearcad.begin_sketch`](./declarative-modeling);
`index` numbers the boundary loop. This selects the vertex; add `edge = true` for the
edge from that corner to the next:

```lua
bearcad.select{
    kind = "face",
    face = { kind = "extrude_side", extrusion = 0, profile = "polygon", profile_lines = { 0, 1, 2, 3 }, edge = 0 },
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
bearcad.line{ x = 5, y = 5, x1 = 12, y1 = 8 }
bearcad.select{ kind = "line", index = 0, ["end"] = "start" }
bearcad.select({ kind = "axis", axis = "x" }, true)   -- add the X axis
bearcad.add_geometric_constraint("coincident")         -- start point now sits on the X axis

bearcad.select{ kind = "line", index = 0, ["end"] = "end" }
bearcad.select({ kind = "origin" }, true)              -- add the origin
bearcad.add_geometric_constraint("coincident")         -- end point now sits on the origin
```

Interactively, dragging a point near an axis or the origin snaps it on and adds the same
constraint.

## Making two lines collinear

Select two lines and apply `Coincident`:

```lua
bearcad.select{ kind = "line", index = 0 }
bearcad.select({ kind = "line", index = 1 }, true)
bearcad.add_geometric_constraint("coincident")         -- the two lines are now collinear
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
bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0, name = "a" }
bearcad.line{ x = 20, y = 0, x1 = 30, y1 = 0, name = "b" }

bearcad.select{ kind = "line", index = 0, ["end"] = "end" }
bearcad.select({ kind = "line", index = 1, ["end"] = "start" }, true)
bearcad.add_geometric_constraint("coincident")
```

Combine with
[`bearcad.extrude{ polygon = {...} }`](./declarative-modeling#a-closed-polygon-from-plain-lines-extruded)
to build and extrude an arbitrary closed profile without any GUI interaction.
