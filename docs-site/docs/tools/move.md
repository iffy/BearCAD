---
sidebar_position: 17
title: Move
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/move.svg")} width="30" /> Move

Move translates and/or rotates whole bodies, producing moved copies.

![A box moved and rotated into a second position](/img/screenshots/move.png)

## How to use it

1. Pick the **Move** tool and click one or more bodies. Re-clicking removes one.
2. Choose how to **Translate**:
   - **Snap** (the default) — pick a **Source point** on a moving body, then a **Target
     point** on something that isn't moving, and the bodies slide so the first lands on the
     second. Either point can be a corner or the midpoint of an edge.
   - **Free** — type the **X / Y / Z** amounts, or drag the coloured arrows (each has a value
     box beside its handle). They're expressions, so the move stays parametric.
3. To rotate, choose **Rotate → Free** and fill in up to three **Axis** pickers, each with its
   own **Angle** below it (degrees by default; `rad` and parameters work). They start on the
   X, Y and Z origin axes, and any of them can be re-pointed at a sketch line or a body edge.
   The turns apply in order, axis 1 first. The **Rotation point** picker sets what they turn
   about — a corner or edge midpoint on any body; left alone it follows the source point.
4. Press **Enter**.

Once a source point is picked the moving bodies go translucent, so you can see the gizmos and
the target you're aiming at through them.

The inputs become [shadow bodies](/docs/tools/combine#shadow-bodies) and each gains a
moved copy — a real body for further operations. **Edit move** changes anything later;
deleting the move restores the originals. Editing a parameter re-places every body moved
by it.

## Scripting

```lua
-- Free: explicit components.
bearcad.move_bodies{ bodies = {0}, x = 40, z = "plate_thickness" }

-- Snap: land one point on another. `vertex` is a corner; `edge` takes a midpoint.
bearcad.move_bodies{ bodies = {0},
  from = { body = 0, vertex = {0, 0, 0} },
  to   = { body = 1, vertex = {40, 0, 0} } }
bearcad.move_bodies{ bodies = {0},
  from = { body = 0, edge = { {0, 0, 0}, {10, 0, 0} } },
  to   = { body = 1, edge = { {40, 0, 0}, {50, 0, 0} } } }

-- Turn about a picked corner instead of the axis itself.
bearcad.move_bodies{ bodies = {0}, axis = "z", angle = 90,
                     pivot = { body = 0, vertex = {20, 0, 0} } }

-- Up to three turns in one move, applied in order.
bearcad.move_bodies{ bodies = {0}, axis = "x", angle = 90, axis2 = "y", angle2 = 90 }
```

Points are millimetre coordinates on the body's mesh — they only need to land on the corner or
edge you mean.

## Moving geometry inside a sketch

Inside a sketch, Move moves sketch geometry. Select lines, circles, or text, then switch
to Move: a gizmo appears at the selection's centre. Drag the centre disc to slide freely,
or an arrow to move along one axis only.

Constraints keep holding as you drag; a move that would force an edge to stretch is
refused (lengths never change).

## Moving construction planes and tracing images

Pick a construction plane or tracing image from the Elements pane with the Move tool
active, then set translation/rotation like a body.

- A **construction plane** moves in place, carrying everything anchored to it — sketches,
  images, extrusions grown from them.
- A **tracing image** slides on its host plane (and follows the plane if the plane moves).

Editing the move back to zero returns it home.

## Rotating sketch text

With a single [sketch text](/docs/tools/text) selected, drag the rotation ring to turn it
about its start point.

## Scripting

```lua
bearcad.move_bodies{ bodies = {0}, x = "25", name = "Shifted" }
bearcad.move_bodies{ bodies = {0, 1}, x = "gap * 2", axis = "z", angle = "45" }
bearcad.edit_move{ index = 0, bodies = {0}, x = "30" }
```
