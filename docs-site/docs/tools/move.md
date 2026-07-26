---
sidebar_position: 17
title: Move
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/move.svg")} width="30" /> Move

**Shortcut:** `M`

Move slides whole bodies to a new place, producing moved copies.

![A box moved into a second position](/img/screenshots/move.png)

## How to use it

1. Pick the **Move** tool and click one or more bodies. Re-clicking removes one.
   Pressing **M** again switches between the two Translate modes.
2. Choose how to **Translate**:
   - **Snap** (the default) — pick a **Start point A** on a moving body, then an **End point
     A** on something that isn't moving, and the bodies slide so the first lands on the
     second. Either point can be a corner, the midpoint of an edge, or the middle of a flat
     face; hovering marks the exact point a click would take, and a **yellow line** connects
     the two once both are picked.
     To **turn** the bodies as well, pick a second pair: **Start point B** on a moving body
     and **End point B** on something that isn't. The bodies then rotate about end point A
     until start B lands on end B. End point B has to be somewhere start B can actually reach
     — the same distance from end point A as start B is from start point A — so once you're
     picking it, every reachable spot is marked in **blue**: where surrounding edges cross
     that distance, and mid-air spots straight out along any edge running through end point
     A, each with a dashed guide line from it. The one under the cursor turns **gold** and
     the preview shows the move you'd get.
   - **Free** — type the **X / Y / Z** amounts, or drag the coloured arrows (each has a value
     box beside its handle). They're expressions, so the move stays parametric.
3. Press **Enter**.

The tool moves you along as you go: pick a body and it's ready for the start point, then the
end point, then straight on to **Start point B** in case you want the turn too. Click any
picker to jump back to it.

## Help

![The Move tool's Context pane in Snap mode, each field explained](/img/screenshots/pane-move-snap.png)

![The Move tool's Context pane in Free mode, each field explained](/img/screenshots/pane-move-free.png)

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

-- A second pair turns it too: start B swings onto end B about end A.
bearcad.move_bodies{ bodies = {0},
  from   = { body = 0, vertex = {0, 0, 0} },
  to     = { body = 0, vertex = {0, 0, 0} },
  from_b = { body = 0, vertex = {10, 0, 0} },
  to_b   = { body = 0, vertex = {0, 10, 0} } }
```

Points are millimetre coordinates on the body's mesh — they only need to land on the corner or
edge you mean. `vertex` is a corner, `edge` takes an edge's midpoint, `on_edge` is a position
along one, and `face_center = {x,y,z}, normal = {x,y,z}` is the middle of a flat face.

## Moving geometry inside a sketch

Inside a sketch, Move moves sketch geometry. Select lines, circles, or text, then switch
to Move: a gizmo appears at the selection's centre. Drag the centre disc to slide freely,
or an arrow to move along one axis only.

Constraints keep holding as you drag; a move that would force an edge to stretch is
refused (lengths never change).

## Moving construction planes and tracing images

Pick a construction plane or tracing image from the Elements pane with the Move tool
active, then set the translation like a body.

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
bearcad.move_bodies{ bodies = {0, 1}, x = "gap * 2", z = "10mm" }
bearcad.edit_move{ index = 0, bodies = {0}, x = "30" }
```
