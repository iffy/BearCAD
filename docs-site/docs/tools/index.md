---
slug: /tools
sidebar_position: 3
title: Tools & Navigation
---

# Tools & Navigation

The viewport always has one active **tool**. **Select** is the default — it only looks
around and picks things, so moving the camera never accidentally creates geometry. Switch to
a drawing tool when you want to draw.

## Tool reference

| | Tool | Shortcut | What it does |
|---|---|---|---|
| <img src="/img/icons/select.svg" width="22" /> | [Select](/docs/tools/select) | — | Look around and pick geometry; the default tool. |
| <img src="/img/icons/sketch.svg" width="22" /> | [Sketch](/docs/tools/sketch) | `S` | Pick a face (or the ground plane) to draw on. |
| <img src="/img/icons/rectangle.svg" width="22" /> | [Rectangle](/docs/tools/rectangle) | `R` | Draw a rectangle by two corners. |
| <img src="/img/icons/line.svg" width="22" /> | [Line](/docs/tools/line) | `L` | Draw connected lines and curves. |
| <img src="/img/icons/circle.svg" width="22" /> | [Circle](/docs/tools/circle) | `O` | Draw a circle by center and diameter. |
| <img src="/img/icons/plane.svg" width="22" /> | [Construction Plane](/docs/tools/construction-plane) | `P` | Add a flat reference plane to sketch on. |
| <img src="/img/icons/dimension.svg" width="22" /> | [Dimension](/docs/tools/dimension) | `D` | Set exact lengths, distances, and angles. |
| <img src="/img/icons/constraint.svg" width="22" /> | [Constraint](/docs/tools/constraint) | `C` | Relate geometry: parallel, equal, coincident, … |
| <img src="/img/icons/extrude.svg" width="22" /> | [Extrude](/docs/tools/extrude) | `E` | Pull a sketch face into a solid — or cut into one. |
| <img src="/img/icons/revolve.svg" width="22" /> | [Revolve](/docs/tools/revolve) | — | Spin a profile around an axis into a solid. |
| <img src="/img/icons/combine.svg" width="22" /> | [Combine](/docs/tools/combine) | — | Boolean operations on bodies: merge, cut, intersect, difference. |
| <img src="/img/icons/move.svg" width="22" /> | [Move](/docs/tools/move) | — | Translate or rotate bodies into moved copies. |
| <img src="/img/icons/repeat.svg" width="22" /> | [Repeat](/docs/tools/repeat) | — | Copies of bodies spaced along an axis. |
| <img src="/img/icons/slice.svg" width="22" /> | [Slice](/docs/tools/slice) | — | Cut bodies into fragments with planes or faces. |
| <img src="/img/icons/chamfer.svg" width="22" /> | [Chamfer](/docs/tools/chamfer) | `K` | Cut a corner or edge flat. |
| <img src="/img/icons/fillet.svg" width="22" /> | [Fillet](/docs/tools/fillet) | `F` | Round a corner or edge. |
| <img src="/img/icons/loft.svg" width="22" /> | [Loft](/docs/tools/loft) | — | Blend a solid through two or more cross-section profiles. |

Reference images for tracing over (import, scale calibration) are covered in
[Tracing images](/docs/tools/tracing).

## Habits that apply everywhere

- **Click to start, move to preview, click or type to finish.** Rectangle, Line, and Circle
  all work this way. While drawing, just type a number (or a
  [parameter](/docs/quickstart#1-set-up-parameters) name) to make that dimension exact —
  **Tab** switches between fields, **Enter** commits.
- **Esc backs out.** It cancels whatever is in progress; pressed again, it returns to
  Select.
- **X toggles construction geometry** — dashed reference shapes that guide your sketch but
  never become part of the solid.
- **The Context pane follows you.** Whatever tool or selection is active, its options appear
  in the pane on the right.

See [Navigation](/docs/tools/navigation) for camera controls and the view bear.
