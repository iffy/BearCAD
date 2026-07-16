---
sidebar_position: 4
title: Point-level selection
---

# Point-level selection

`bearcad.select` normally targets a whole element (a line, a circle, a body). Point-level
selection targets an individual **vertex** — a `ConstraintPoint` — instead: a line endpoint (a
rectangle corner is one of these, since a rect is four lines), or (with an explicit flag) a
circle's center. This uses the same point numbering the interactive
[Constraint](/docs/tools/constraint) tool uses, so a script can drive exactly the same constraint
flows a user would with the mouse.

## Selecting a line endpoint

```lua
bearcad.select{ kind = "line", index = 0, ["end"] = "start" }  -- or "end"
```

A line's two points are `start`/`end`, i.e. `(x0, y0)`/`(x1, y1)`.

## Selecting a rectangle corner

`bearcad.rect` builds a rectangle as **four separate lines** (one per edge), so a corner is just
the shared endpoint of two of them — address it as a line endpoint. With the rect's lines at
indices 0–3, each corner is the `start` of the line with the same number:

```lua
bearcad.select{ kind = "line", index = 2, ["end"] = "start" }  -- the third corner (top-right)
```

The lines (and their start corners) run **counterclockwise starting at the `(x, y)` origin
corner**: line 0 is the bottom edge (its start is the origin corner), 1 the right, 2 the top, 3
the left — the same numbering shown when the interactive Constraint tool highlights a rect's
points.

## Selecting a circle's center

`kind = "circle"` alone still selects the whole circle. Pass `point = true` to target just its
center point:

```lua
bearcad.select{ kind = "circle", index = 0, point = true }
```

`point = true` is the general escape hatch for targeting a point that has no `end`/`corner` field
of its own — a table with neither still resolves to the whole element, as before.

## Selecting a face's own vertex or edge

While a sketch is open directly on one of a body's own faces (an extrusion cap or side wall —
not a construction plane), that face's own boundary loop is selectable too, so a sketch can be
constrained against the face it's drawn on (e.g. "30mm from the top edge"):

```lua
bearcad.select{
    kind = "face",
    face = { kind = "extrude_cap", extrusion = 0, profile = "polygon", profile_lines = { 0, 1, 2, 3 }, top = true },
    index = 2,
}
```

`face` takes the same table shape [`bearcad.begin_sketch`](./declarative-modeling) does for a 3D
body face. `index` numbers the face's boundary loop the same way `cap_polygon_world`/
`side_quad_world` order it. This selects the vertex (a `ConstraintPoint::FaceVertex`); add
`edge = true` to select the edge running from that corner to the next instead
(`ConstraintLine::FaceEdge`):

```lua
bearcad.select{
    kind = "face",
    face = { kind = "extrude_side", extrusion = 0, profile = "polygon", profile_lines = { 0, 1, 2, 3 }, edge = 0 },
    index = 0,
    edge = true,
}
```

Both are fixed by the body's own geometry — not draggable or settable — but otherwise plug into
`Coincident`, `Midpoint`, and distance constraints exactly like any other sketch point/line.
Picking (interactive or scripted) is scoped to the *sketch's own face* only, not arbitrary other
faces in the scene; imported STL/STEP bodies have no analytic boundary to reference here.

## Selecting the origin and its axes

The origin and the two in-plane axes through it — the X axis (`v = 0`) and the Y axis
(`u = 0`) — are selectable. Select an axis to constrain a point onto it (which pins that
coordinate to 0), or select the origin to pin a point directly to it:

```lua
bearcad.line{ x = 5, y = 5, x1 = 12, y1 = 8 }
bearcad.select{ kind = "line", index = 0, ["end"] = "start" }
bearcad.select({ kind = "axis", axis = "x" }, true)   -- add the X axis
bearcad.add_geometric_constraint("coincident")         -- start point now sits on the X axis

bearcad.select{ kind = "line", index = 0, ["end"] = "end" }
bearcad.select({ kind = "origin" }, true)              -- add the origin
bearcad.add_geometric_constraint("coincident")         -- end point now sits on the origin
```

Interactively, dragging a point near an axis or the origin snaps it on; leaving it there adds
the same constraint. In the constraint tool you can also click the origin marker or an axis
directly — both highlight when selected.

## Making two lines collinear

Select two lines and apply `Coincident` to make them collinear (each line's endpoints are held
on the other's carrier):

```lua
bearcad.select{ kind = "line", index = 0 }
bearcad.select({ kind = "line", index = 1 }, true)
bearcad.add_geometric_constraint("coincident")         -- the two lines are now collinear
```

## Additive selection

Pass `true` as the second argument to `bearcad.select` to add to the current selection instead of
replacing it — this is how you build up a two-point (or two-line) selection for a constraint:

```lua
bearcad.select({ kind = "line", index = 1 }, true)
```

## Worked example: closing a polygon loop purely from a script

Joining two line endpoints with a `Coincident` constraint — enough to close a polygon loop
without simulating any mouse clicks:

```lua
bearcad.new()
bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0, name = "a" }
bearcad.line{ x = 20, y = 0, x1 = 30, y1 = 0, name = "b" }

bearcad.select{ kind = "line", index = 0, ["end"] = "end" }
bearcad.select({ kind = "line", index = 1, ["end"] = "start" }, true)
bearcad.add_geometric_constraint("coincident")
```

After this, line `0`'s end and line `1`'s start are coincident — exactly as if you'd clicked both
endpoints in the viewport with the Constraint tool active and pressed `4`. Combine this with
[`bearcad.extrude{ polygon = {...} }`](./declarative-modeling#a-closed-polygon-from-plain-lines-extruded)
to build and extrude an arbitrary closed profile without any GUI interaction at all.
