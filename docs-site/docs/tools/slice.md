---
sidebar_position: 19
title: Slice
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/slice.svg")} width="30" /> Slice

Slice cuts whole bodies apart with flat cutters — halving a part, or splitting a model
into printable pieces.

![A box sliced by a plane into two fragments](/img/screenshots/slice.png)

## How to use it

1. Pick the **Slice** tool and click one or more bodies (the **Bodies** picker).
2. Click the **Cutters** picker, then click the planes or planar faces to cut with: any
   construction plane, or a flat face of a body.
3. Press **Enter**.

Each target is cut independently. Each cutter divides whatever pieces the previous cuts
produced — two crossing planes through a block give four fragments.

**Extend cutters to infinity** (on by default) treats every cutter as an endless plane.
Off, a finite face carves only its own footprint. A construction plane is always infinite.

## What you get

Each fragment is a new body nested under the slice element. The input body lives on as a
**shadow body** — hidden until you hover or select it. A cutter that misses a body leaves
it whole.

**Edit slice** re-opens the pickers; deleting the slice restores the input body.

## Scripting

```lua
bearcad.slice{ bodies = {0}, cutters = {{ kind = "construction_plane", index = 1 }} }
bearcad.slice{ bodies = {0, 1},
               cutters = {{ kind = "construction_plane", index = 1 }},
               extend = false, name = "Split" }
bearcad.edit_slice{ index = 0, bodies = {0},
                    cutters = {{ kind = "construction_plane", index = 2 }} }
```

A cutter is a face-spec table, the same shape `bearcad.begin_sketch` accepts.

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
