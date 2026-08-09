---
sidebar_position: 22
title: Slice
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/slice.svg")} width="30" /> Slice

Slice cuts whole bodies apart — with flat planes/faces, or with sketch lines that laser-cut
through a body along their path.

<a
  href={useBaseUrl("/app/") + "?open=" + encodeURIComponent(useBaseUrl("/img/screenshots/slice.bearcad.json"))}
  target="_blank"
  rel="noopener noreferrer"
  title="Open this model in BearCAD"
>
  <img src={useBaseUrl("/img/screenshots/slice.png")} alt="A box sliced by a plane into two fragments" />
</a>

## How to use it

1. Pick the **Slice** tool and click one or more bodies (the **Bodies** picker).
2. Click the **Cutters** picker, then pick:
   - a construction plane or flat body face, or
   - a sketch line (straight or curved) on a face — the laser points into the face and
     travels the path. Endpoint-connected lines form one continuous path (a zigzag is
     one cut → two pieces). Disjoint lines each cut.
3. Press **Enter**. A laser cut is only allowed when it splits a body into at least two
   pieces.

Each target is cut independently. Each cutter divides whatever pieces the previous cuts
produced — two crossing planes (or separate lines) through a block give four fragments.

While you pick, target bodies go semi-transparent and laser paths preview as cutting
surfaces through the solid (extended past the ends when Infinite cut is on).

**Infinite cut** (on by default) extends every plane endlessly and expands every line past
its endpoints (straight: same direction; curve: end tangent) so a short path still severs
the solid. Off, a finite face carves only its own footprint and a line only cuts within its
span. Construction planes are always infinite.

## What you get

Each fragment is a new body nested under the slice element. The input body lives on as a
**shadow body** — hidden until you hover or select it. A cutter that misses a body leaves
it whole.

**Edit slice** re-opens the pickers; deleting the slice restores the input body.

## Help

![The Slice tool's Context pane, each field explained](/img/screenshots/pane-slice.png)

## Scripting

```lua
bearcad.slice{ bodies = {0}, cutters = {{ kind = "construction_plane", index = 1 }} }
bearcad.slice{ bodies = {0}, cutters = {{ kind = "line", index = 4 }} }
bearcad.slice{ bodies = {0, 1},
               cutters = {{ kind = "construction_plane", index = 1 }},
               extend = false, name = "Split" }
bearcad.edit_slice{ index = 0, bodies = {0},
                    cutters = {{ kind = "construction_plane", index = 2 }} }
```

A planar cutter is a face-spec table (same shape `bearcad.begin_sketch` accepts). A line
cutter is `{ kind = "line", index = i }`.

## Slicing sketch geometry in 2D

Split lines where other lines cross them. The sliced line becomes a *shadow* — no longer
part of any face, but still editable — and each crossing produces a fragment line. From
scripts:

```lua
-- Split line 0 wherever line 1 crosses it, in sketch 0:
bearcad.slice_sketch{ sketch = 0, lines = {0}, cutters = {1} }

-- Slice several targets with several cutters at once, then re-point:
bearcad.slice_sketch{ sketch = 0, lines = {0, 2}, cutters = {1, 3} }
bearcad.edit_sketch_slice{ index = 0, lines = {0}, cutters = {1} }

-- Circles and curves slice too — a line through circle 0 splits it into arcs:
bearcad.slice_sketch{ sketch = 0, circles = {0}, cutters = {1} }
```

Bezier targets stay curved when split; a sliced circle becomes arcs. A shadowed original
no longer forms a face — its fragments do, so they extrude independently.
