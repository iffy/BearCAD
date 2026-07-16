---
sidebar_position: 13
title: Constraint
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/constraint.svg")} width="30" /> Constraint

**Shortcut:** `C`

Constraints state facts about your sketch that BearCAD then keeps true: these two lines
stay parallel, these endpoints stay attached, this line stays horizontal. Where the
[Dimension](./dimension.md) tool pins *sizes*, constraints pin *relationships*.

Select the geometry first, then click a constraint button in the Context pane — or press
its number key (active while the Constraint tool is selected):

| Constraint | Select first | Key |
|---|---|---|
| Parallel | two lines | `1` |
| Perpendicular | two lines | `2` |
| Equal | two lines | `3` |
| Coincident | two points; a point + a line, circle, or the origin; or two lines (made collinear) | `4` |
| Midpoint | a point + a line | `5` |
| Vertical | a line | `6` |
| Horizontal | a line | `7` |

The pane always lists every constraint; ones the current selection can't satisfy are shown
faded, with a hint about what they need. Many constraints also happen automatically while
you draw — snapping a line's end onto a point keeps them attached.

Below the buttons, the context pane shows the **element picker** for the geometry you're
constraining — the same combo-box control the [Select](./select.md) tool uses, but limited to
what's constrainable (points, lines, circles, and body/face edges). It reads out what you've
picked as a summary by kind; click it to review each item and remove any you didn't mean to
include.

When a sketch is drawn on a body's face, the face's own corners and edges can be
constrained against too — pin a point to the face's corner, or keep a line on its edge.

## The origin and its axes

The **origin** and the two **origin axes** are selectable just like any other geometry. Click
the origin (the marker where the axes cross) and a point to pin that point to the origin, or
click an axis and a point to hold the point on that axis (which fixes one of its coordinates).
A selected origin brightens and a selected axis highlights along its full length, so you can
see exactly what you picked.
