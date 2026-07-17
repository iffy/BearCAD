---
sidebar_position: 12
title: Combine
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/combine.svg")} width="30" /> Combine

Combine performs boolean operations on whole bodies.

![Two overlapping bodies cut into a notched result](/img/screenshots/combine.png)

## The four operations

| Operation | Result |
|---|---|
| **Combine** | One body containing everything in the picked set (union). |
| **Cut** | Side A with side B carved away (A − B). |
| **Intersect** | Only the material common to A and B. |
| **Difference** | Only the material *not* common to A and B (symmetric difference). |

## How to use it

1. Pick the **Combine** tool and choose the operation.
2. Click bodies to add them. Two-sided operations have **Side A** and **Side B** pickers;
   click a picker to make it the active side. Re-clicking a body removes it.
3. **Keep B** leaves the B-side bodies as real bodies (by default all inputs become
   shadow bodies).
4. Press **Enter**.

A cut that severs a body into separate pieces gives one body per piece.

## Shadow bodies

Input bodies become **shadow bodies**: out of the 3D view and out of clicking's way until
you hover or select them in the Elements pane, where they ghost translucently.

## The operation element

The operation is an element with the new bodies nested under it. **Edit operation**
changes the kind, inputs, or **Keep B**; deleting it restores the inputs. Result bodies
are ordinary bodies, so operations chain.

## Scripting

```lua
bearcad.combine{ op = "cut", a = {0}, b = {1}, name = "Notched block" }
bearcad.combine{ op = "combine", a = {0, 1, 2} }
bearcad.combine{ op = "intersect", a = {0}, b = {1}, keep_b = true }
bearcad.edit_boolean{ index = 0, op = "difference", a = {0}, b = {1} }
```

## Good to know

- All four operations undo as a single step.
- Shadow bodies can't be picked into another operation — edit or delete the operation
  that owns them instead.
