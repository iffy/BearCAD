---
sidebar_position: 12
title: Constraint
---

# Constraint

**Shortcut:** `C`

Constraints state facts about your sketch that BearCAD then keeps true: these two lines
stay parallel, these endpoints stay attached, this line stays horizontal. Where the
[Dimension](./dimension.md) tool pins *sizes*, constraints pin *relationships*.

Select the geometry first, then click a constraint button in the Context pane — or press
its shortcut letter:

| Constraint | Select first | Key |
|---|---|---|
| Parallel | two lines | `A` |
| Perpendicular | two lines | `T` |
| Equal | two lines | `Q` |
| Coincident | two points, a point + a line, or a point + a circle | `I` |
| Midpoint | a point + a line | `M` |
| Vertical | a line | `V` |
| Horizontal | a line | `H` |

The pane always lists every constraint; ones the current selection can't satisfy are shown
faded, with a hint about what they need. Many constraints also happen automatically while
you draw — snapping a line's end onto a point keeps them attached.

When a sketch is drawn on a body's face, the face's own corners and edges can be
constrained against too — pin a point to the face's corner, or keep a line on its edge.
