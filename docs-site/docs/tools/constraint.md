---
sidebar_position: 21
title: Constraint
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/constraint.svg")} width="30" /> Constraint

**Shortcut:** `C`

Constraints state facts about your sketch that BearCAD keeps true: these lines stay
parallel, these endpoints stay attached. Where [Dimension](./dimension.md) pins *sizes*,
constraints pin *relationships*.

Select the geometry, then click a constraint button or press its number key:

![A profile squared up by parallel and perpendicular constraints, with the constraint pane open](/img/screenshots/constraint.png)

| Constraint | Select first | Key |
|---|---|---|
| Parallel | two lines | `1` |
| Perpendicular | two lines | `2` |
| Equal | two lines | `3` |
| Coincident | two points; a point + a line, circle, or the origin; or two lines (made collinear) | `4` |
| Midpoint | a point + a line | `5` |

Many constraints also happen automatically while drawing — snapping a line's end onto a
point keeps them attached.

When a sketch is drawn on a body's face, the face's own corners and edges can be
constrained against too.

## The origin and its axes

Every sketch shows a **floating origin** with two axes — X (red) and Y (green) — drawn through
the origin so you always know which way the sketch runs. Both are selectable like any other
geometry:

- Pin a point **to the origin**, or hold a point **on an axis** (which fixes one of its
  coordinates).
- Select a line **and an axis**, then press **Parallel** (`1`) or **Perpendicular** (`2`), to
  make the line run along — or square to — that axis. This is how you make a line "horizontal"
  or "vertical": parallel to the X or Y axis. It works the same on any sketch plane, at any
  angle, because it refers to the sketch's own axes rather than the screen.
