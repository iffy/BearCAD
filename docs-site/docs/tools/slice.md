---
sidebar_position: 16
title: Slice
---

# Slice

Slice cuts whole bodies apart with flat cutters — halving a part, carving a slot line, or
splitting a model into printable pieces.

![A box sliced by a plane into two fragments](/img/screenshots/slice.png)

## How to use it

1. Pick the **Slice** tool. The context pane starts on the **Bodies** picker — click one or
   more bodies to slice.
2. Switch the *Picking* control to **Cutters**, then click the planes or planar faces to cut
   with: any construction plane, or a flat face of a body. Each side you pick lists in the
   context pane with a remove button.
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
