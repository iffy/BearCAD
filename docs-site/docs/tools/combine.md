---
sidebar_position: 12
title: Combine
---

# Combine

Combine performs boolean operations on whole bodies: merge them into one, cut one out of
another, keep only their overlap, or keep everything *except* the overlap.

![Two overlapping bodies cut into a notched result](/img/screenshots/combine.png)

## The four operations

| Operation | Result |
|---|---|
| **Combine** | One body containing everything in the picked set (union). |
| **Cut** | Side A with side B carved away (A − B). |
| **Intersect** | Only the material common to A and B. |
| **Difference** | Only the material *not* common to A and B (symmetric difference). |

## How to use it

1. Pick the **Combine** tool and choose the operation in the context pane.
2. Click bodies in the viewport to add them. **Combine** has a single picked set; the
   other operations have **A** and **B** sides — the *Picking* switch in the context pane
   chooses where the next click lands, and each side can hold several bodies. Re-clicking
   a body removes it.
3. For the two-sided operations, **Keep B** leaves the B-side bodies as real bodies after
   the operation (by default all inputs become shadow bodies).
4. Press **Enter** (or the **Create** button) to commit.

The result is one or more new bodies — a cut that severs a body into separate pieces gives
you one body per piece.

## Shadow bodies

The input bodies aren't gone: they become **shadow bodies**, listed in the Elements pane
with a dashed-outline icon. A shadow body stays out of the 3D view (and out of clicking's
way) until you hover or select it in the pane, where it appears as a translucent ghost —
hovering the operation row ghosts all of its inputs at once.

## The operation element

The operation itself is an element in the pane, with the new bodies nested under it.
Select it and choose **Edit operation** to re-open the pickers — change the operation
kind, add or remove inputs, or flip **Keep B** — then **Apply changes**. Deleting the
operation removes its result bodies and restores the inputs to real bodies.

Operations chain: the result bodies are ordinary bodies, so they can be picked as inputs
to further boolean operations.

## Scripting

```lua
bearcad.combine{ op = "cut", a = {0}, b = {1}, name = "Notched block" }
bearcad.combine{ op = "combine", a = {0, 1, 2} }
bearcad.combine{ op = "intersect", a = {0}, b = {1}, keep_b = true }
bearcad.edit_boolean{ index = 0, op = "difference", a = {0}, b = {1} }
```

## Good to know

- All four operations undo as a single step, restoring the inputs.
- Shadow bodies can't be picked into another operation — edit or delete the operation
  that owns them instead. Result bodies chain freely.
