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

![A profile squared up by parallel, perpendicular, and horizontal constraints, with the constraint pane open](/img/screenshots/constraint.png)

| Constraint | Select first | Key |
|---|---|---|
| Parallel | two lines | `1` |
| Perpendicular | two lines | `2` |
| Equal | two lines | `3` |
| Coincident | two points; a point + a line, circle, or the origin; or two lines (made collinear) | `4` |
| Midpoint | a point + a line | `5` |
| Vertical | a line | `6` |
| Horizontal | a line | `7` |

Many constraints also happen automatically while drawing — snapping a line's end onto a
point keeps them attached.

When a sketch is drawn on a body's face, the face's own corners and edges can be
constrained against too.

## The origin and its axes

The **origin** and the two **origin axes** are selectable like any other geometry: pin a
point to the origin, or hold a point on an axis (which fixes one of its coordinates).
