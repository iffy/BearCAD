---
sidebar_position: 12.5
title: Shape
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/shape_cuboid.svg")} width="30" /> Shape

**Shortcut:** `B` — pressing it again cycles cuboid → cylinder → sphere.

Shape places a solid straight into 3D: a cuboid, a cylinder, or a sphere. No sketch, no
profile. The toolbar button shows the shape you used last.

## The shapes

| | Shape | Dimensions |
|---|---|---|
| <img src={useBaseUrl("/img/icons/shape_cuboid.svg")} width="22" /> | Cuboid | Width, Depth, Height |
| <img src={useBaseUrl("/img/icons/shape_cylinder.svg")} width="22" /> | Cylinder | Radius, Height |
| <img src={useBaseUrl("/img/icons/shape_sphere.svg")} width="22" /> | Sphere | Radius |

Every dimension is an expression, so a shape follows [parameters](/docs/parameters) like
anything else. The shape sits **on** the plane it's placed on and grows along that plane's
normal: a cuboid and a cylinder from the middle of their base, a sphere from the point it
rests on.

## Placing one

The shape follows your cursor until you click. The **first click** puts it down — on a face
or plane under the cursor, otherwise on the ground — and it grows away from there.

| Shape | Click 1 | Click 2 | Click 3 |
|---|---|---|---|
| Cuboid | One base corner | The opposite corner | The height |
| Cylinder | The centre | The radius | The height |
| Sphere | Where it rests | The radius | — |

Each step offers its own field, ready for typing: type a size instead of clicking and that
dimension stops following the cursor. **Enter** creates it; **Esc** clears what's in
progress.

**Snapping** — the context pane's toggle, shared with the drawing tools — pulls a
placement click onto the nearest body corner or edge midpoint.

## Editing

A shape is its own row in the Elements pane, named by kind. Double-click it to reopen the
tool with its dimensions loaded; **Apply changes** re-points it, keeping its body.

## Scripting

```lua
bearcad.cuboid{ width = 40, depth = 20, height = 10, name = "Block" }
bearcad.cylinder{ at = {100, 0, 0}, radius = 5, height = 20 }
bearcad.sphere{ at = {200, 0, 0}, radius = 8 }

-- On another plane: `normal` is the direction it grows, `u_axis` the width direction.
bearcad.cuboid{ at = {0, 0, 10}, normal = {0, 0, 1}, width = "side", depth = "side", height = 4 }

-- Re-point one; unmentioned fields keep their value.
bearcad.edit_shape{ index = 0, height = "side * 3" }
```
