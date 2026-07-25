---
sidebar_position: 17
title: Move
---

import useBaseUrl from '@docusaurus/useBaseUrl';
import PaneCallouts from '@site/src/components/PaneCallouts';

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
     second. Either point can be a corner or the midpoint of an edge.
     To **turn** the bodies as well, pick a second pair: **Start point B** on a moving body
     and **End point B** on something that isn't. The bodies then rotate about end point A
     until start B lands on end B. End point B has to be somewhere start B can actually reach
     — the same distance from end point A as start B is from start point A — so once you're
     picking it, every reachable spot on the surrounding edges is marked in **blue**. The one
     under the cursor turns **gold** and the preview shows the move you'd get.
   - **Free** — type the **X / Y / Z** amounts, or drag the coloured arrows (each has a value
     box beside its handle). They're expressions, so the move stays parametric.
3. Press **Enter**.

The tool moves you along as you go: pick a body and it's ready for the start point, pick that
and it's ready for the end point. Click any picker to jump back to it.

## The Context pane

<PaneCallouts
  src="/img/screenshots/pane-move-snap.png"
  alt="The Move tool's Context pane in Snap mode"
  title="Translate: Snap"
  items={[
    {x: 37, y: 27, label: 'Bodies', children: <>The bodies that will move. Click one in the viewport to add it, click it again to drop it.</>},
    {x: 37, y: 39, label: 'Translate', children: <>Snap or Free — how you say where the bodies go. <strong>M</strong> switches between them.</>},
    {x: 37, y: 49, label: 'Start point A', children: <>The corner or edge midpoint on a moving body that you're aiming with.</>},
    {x: 37, y: 60, label: 'End point A', children: <>Where start point A lands — a corner or edge midpoint on something that isn't moving.</>},
    {x: 37, y: 70, label: 'Start point B', children: <>Optional. A second point on a moving body, to turn the bodies as well as slide them.</>},
    {x: 37, y: 81, label: 'End point B', children: <>Optional. Where start point B swings to, about end point A. Only the spots it can reach are offered.</>},
    {x: 37, y: 92, label: 'Commit', children: <>Applies the move. <strong>Enter</strong> does the same.</>},
  ]}
/>

<PaneCallouts
  src="/img/screenshots/pane-move-free.png"
  alt="The Move tool's Context pane in Free mode"
  title="Translate: Free"
  items={[
    {x: 37, y: 28, label: 'Bodies', children: <>The bodies that will move.</>},
    {x: 37, y: 41, label: 'Translate', children: <>Set to Free: you type the distance instead of picking points.</>},
    {x: 37, y: 50, label: 'Start point A', children: <>Where the drag arrows sit. Leave it and they sit on the selection.</>},
    {x: 37, y: 61, label: 'X', children: <>How far along X, as an expression — <code>25</code>, <code>gap * 2</code>, <code>10mm</code>.</>},
    {x: 37, y: 71, label: 'Y', children: <>How far along Y.</>},
    {x: 37, y: 81, label: 'Z', children: <>How far along Z.</>},
    {x: 37, y: 92, label: 'Commit', children: <>Applies the move.</>},
  ]}
/>

Once a start point is picked the moving bodies go translucent, so you can see the gizmos and
what you're aiming at through them. Start point A marks **green** and end point A **red** — go
and stop — with a line drawn between them, and a ghost shows where the bodies will land before
you commit.

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

-- A second pair turns it too: start B swings onto end B about end A.
bearcad.move_bodies{ bodies = {0},
  from   = { body = 0, vertex = {0, 0, 0} },
  to     = { body = 0, vertex = {0, 0, 0} },
  from_b = { body = 0, vertex = {10, 0, 0} },
  to_b   = { body = 0, vertex = {0, 10, 0} } }
```

Points are millimetre coordinates on the body's mesh — they only need to land on the corner or
edge you mean. `vertex` is a corner, `edge` takes an edge's midpoint, and `on_edge` is a
position along one.

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
