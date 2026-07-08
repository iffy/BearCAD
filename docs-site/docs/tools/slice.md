---
sidebar_position: 16
title: Slice
---

# Slice

Slice cuts whole bodies apart with flat cutters — halving a part, carving a slot line, or
splitting a model into printable pieces.

![A box sliced by a plane into two fragments](/img/screenshots/slice.png)

## How to use it

1. Pick the **Slice** tool. The context pane shows two element pickers, **Bodies** and
   **Cutters**. The **Bodies** picker is focused to start — click one or more bodies to slice.
2. Click the **Cutters** picker to make it active (a focus ring marks which picker your next
   click feeds), then click the planes or planar faces to cut with: any construction plane, or
   a flat face of a body. Expand either picker to review its set and remove items.
3. Press **Enter** (or the **Slice** button).

Each target is cut independently. With several cutters, each one divides whatever pieces the
previous cuts produced, so two crossing planes through a block give you four fragments.

## Extend to infinity

The **Extend cutters to infinity** toggle (on by default) treats every cutter as an endless
plane, so it divides the whole body. Turn it off and a cutter only separates material within
its own face footprint — useful when you want a finite face to carve just the region it
covers. A construction plane is always infinite.

## What you get

Each fragment is a new body, nested under the slice element in the pane. The body you cut
lives on as a **shadow body** — listed in the Elements pane with a dashed-outline icon,
hidden from the 3D view until you hover or select it, where it appears as a translucent
ghost. A cutter that misses a body leaves it whole.

Select the slice element and choose **Edit slice** to re-open the pickers — add or remove
targets and cutters, or flip the extend toggle — then **Apply changes**. Deleting the slice
removes its fragments and restores the input to a real body. The fragments are ordinary
bodies, so they chain into further operations.

## Scripting

```lua
bearcad.slice{ bodies = {0}, cutters = {{ kind = "construction_plane", index = 1 }} }
bearcad.slice{ bodies = {0, 1},
               cutters = {{ kind = "construction_plane", index = 1 }},
               extend = false, name = "Split" }
bearcad.edit_slice{ index = 0, bodies = {0},
                    cutters = {{ kind = "construction_plane", index = 2 }} }
```

A cutter is a face-spec table, the same shape `bearcad.begin_sketch` accepts — a construction
plane or a planar body cap face.

## Slicing sketch geometry in 2D

You can also slice **inside a sketch**: split lines where other lines cross them. The sliced
line is kept as a *shadow* — no longer part of any solid face, but still there to edit or restore
— and each crossing produces a new fragment line. This is available from scripts today:

```lua
-- Split line 0 wherever line 1 crosses it, in sketch 0:
bearcad.slice_sketch{ sketch = 0, lines = {0}, cutters = {1} }

-- Slice several targets with several cutters at once, then re-point:
bearcad.slice_sketch{ sketch = 0, lines = {0, 2}, cutters = {1, 3} }
bearcad.edit_sketch_slice{ index = 0, lines = {0}, cutters = {1} }
```

`lines` are the targets to cut; `cutters` are the lines that divide them. A shadowed original no
longer forms a face — its fragments do — so you can carve a profile into pieces that extrude
independently.
