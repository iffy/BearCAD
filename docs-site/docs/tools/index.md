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

| Tool | Shortcut | What it does |
|---|---|---|
| [Select](/docs/tools/select) | — | Look around and pick geometry; the default tool. |
| [Sketch](/docs/tools/sketch) | `S` | Pick a face (or the ground plane) to draw on. |
| [Rectangle](/docs/tools/rectangle) | `R` | Draw a rectangle by two corners. |
| [Line](/docs/tools/line) | `L` | Draw connected lines and curves. |
| [Circle](/docs/tools/circle) | `O` | Draw a circle by center and diameter. |
| [Construction Plane](/docs/tools/construction-plane) | `P` | Add a flat reference plane to sketch on. |
| [Dimension](/docs/tools/dimension) | `D` | Set exact lengths, distances, and angles. |
| [Constraint](/docs/tools/constraint) | `C` | Relate geometry: parallel, equal, coincident, … |
| [Extrude](/docs/tools/extrude) | `E` | Pull a sketch face into a solid — or cut into one. |
| [Revolve](/docs/tools/revolve) | — | Spin a profile around an axis into a solid. |
| [Combine](/docs/tools/combine) | — | Boolean operations on bodies: merge, cut, intersect, difference. |
| [Move](/docs/tools/move) | — | Translate or rotate bodies into moved copies. |
| [Repeat](/docs/tools/repeat) | — | Copies of bodies spaced along an axis. |
| [Chamfer](/docs/tools/chamfer) | `K` | Cut a corner or edge flat. |
| [Fillet](/docs/tools/fillet) | `F` | Round a corner or edge. |
| [Loft](/docs/tools/loft) | — | Blend a solid through two or more cross-section profiles. |

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
