---
slug: /tools
sidebar_position: 1
title: Modeling Tools
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# Modeling Tools

The viewport always has one active **tool**; **Select** is the default. These make up the
toolbar in the **3D modeling** workbench (the default one — opening a drawing switches to
the [Drawing Tools](/docs/drawing-tools)).

| | Tool | Shortcut | What it does |
|---|---|---|---|
| <img src={useBaseUrl("/img/icons/select.svg")} width="22" /> | [Select](/docs/tools/select) | — | Look around and pick geometry; the default tool. |
| <img src={useBaseUrl("/img/icons/sketch.svg")} width="22" /> | [Sketch](/docs/tools/sketch) | `S` | Pick a face (or the ground plane) to draw on. |
| <img src={useBaseUrl("/img/icons/rectangle.svg")} width="22" /> | [Rectangle](/docs/tools/rectangle) | `R` | Draw a rectangle by two corners. |
| <img src={useBaseUrl("/img/icons/line.svg")} width="22" /> | [Line](/docs/tools/line) | `L` | Draw connected lines and curves. |
| <img src={useBaseUrl("/img/icons/circle.svg")} width="22" /> | [Circle](/docs/tools/circle) | `O` | Draw a circle by center and diameter. |
| <img src={useBaseUrl("/img/icons/text.svg")} width="22" /> | [Text](/docs/tools/text) | `T` | Place editable lettering in a sketch. |
| <img src={useBaseUrl("/img/icons/plane.svg")} width="22" /> | [Construction Plane](/docs/tools/construction-plane) | `P` | Add a flat reference plane to sketch on. |
| <img src={useBaseUrl("/img/icons/dimension.svg")} width="22" /> | [Dimension](/docs/tools/dimension) | `D` | Set exact lengths, distances, and angles. |
| <img src={useBaseUrl("/img/icons/constraint.svg")} width="22" /> | [Constraint](/docs/tools/constraint) | `C` | Relate geometry: parallel, equal, coincident, … |
| <img src={useBaseUrl("/img/icons/extrude.svg")} width="22" /> | [Extrude](/docs/tools/extrude) | `E` | Pull a sketch face into a solid — or cut into one. |
| <img src={useBaseUrl("/img/icons/revolve.svg")} width="22" /> | [Revolve](/docs/tools/revolve) | — | Spin a profile around an axis into a solid. |
| <img src={useBaseUrl("/img/icons/combine.svg")} width="22" /> | [Combine](/docs/tools/combine) | — | Boolean operations on bodies: merge, cut, intersect, difference. |
| <img src={useBaseUrl("/img/icons/move.svg")} width="22" /> | [Move](/docs/tools/move) | — | Translate or rotate bodies into moved copies. |
| <img src={useBaseUrl("/img/icons/repeat.svg")} width="22" /> | [Repeat](/docs/tools/repeat) | — | Copies of bodies spaced along an axis. |
| <img src={useBaseUrl("/img/icons/slice.svg")} width="22" /> | [Slice](/docs/tools/slice) | — | Cut bodies into fragments with planes or faces. |
| <img src={useBaseUrl("/img/icons/offset.svg")} width="22" /> | [Offset](/docs/tools/offset) | — | Parallel copies of sketch edges a constant distance away. |
| <img src={useBaseUrl("/img/icons/chamfer.svg")} width="22" /> | [Chamfer](/docs/tools/chamfer) | `K` | Cut a corner or edge flat. |
| <img src={useBaseUrl("/img/icons/fillet.svg")} width="22" /> | [Fillet](/docs/tools/fillet) | `F` | Round a corner or edge. |
| <img src={useBaseUrl("/img/icons/loft.svg")} width="22" /> | [Loft](/docs/tools/loft) | — | Blend a solid through two or more cross-section profiles. |

Reference images for tracing over (import, scale calibration) are covered in
[Tracing images](/docs/tools/tracing).

## Habits that apply everywhere

- **Click to start, move to preview, click or type to finish.** While drawing, type a
  number or [parameter](/docs/parameters) name to make that dimension exact — **Tab**
  switches fields, **Enter** commits.
- **Esc backs out**; pressed again, it returns to Select.
- **X toggles construction geometry** — dashed reference shapes that never become part of
  the solid.
- **The Context pane** (right) shows the active tool's or selection's options.

See [Navigation](/docs/tools/navigation) for camera controls and the view bear.
