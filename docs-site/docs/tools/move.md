---
sidebar_position: 19
title: Move
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/move.svg")} width="30" /> Move

**Shortcut:** `M`

Move slides whole bodies to a new place, producing moved copies.

<a
  href={useBaseUrl("/app/") + "?open=" + encodeURIComponent(useBaseUrl("/img/screenshots/move.bearcad.json"))}
  target="_blank"
  rel="noopener noreferrer"
  title="Open this model in BearCAD"
>
  <img src={useBaseUrl("/img/screenshots/move.png")} alt="A box moved into a second position" />
</a>

## How to use it

1. Pick the **Move** tool and click one or more bodies. Re-clicking removes one.
   Pressing **M** again steps through the Move modes.
2. Choose a **Move mode**:
   - **Point Snap** (the default) — pick a **Start point A** on a moving body, then an **End point
     A** on something that isn't moving, and the bodies slide so the first lands on the
     second. Destination picks ignore the moving bodies, so you can click through them. Either point can be a corner, the midpoint of an edge, or the middle of a flat
     face; an end point can also be the **origin**. Hovering marks the exact point a click
     would take — and while an end picker is
     armed, the preview glides over to show the move that point would make, before you
     click. A **yellow line** connects the two once both are picked.
     To **turn** the bodies as well, pick a second pair: **Start point B** on a moving body
     and **End point B** on something that isn't. The bodies then rotate about end point A
     until start B lands on end B. End point B has to be somewhere start B can actually reach
     — the same distance from end point A as start B is from start point A — so once you're
     picking it, every reachable spot is marked: where surrounding edges cross
     that distance, and mid-air spots straight out along any edge running through end point
     A, each with a dashed guide line from it. Each spot takes the colour of the axis its turn
     goes about — **red** for X, **green** for Y, **blue** for Z — so spots that turn the same
     way group together. The one under the cursor turns **gold** and
     the preview shows the move you'd get. With both B points picked, a **dashed blue curve** from
     start B to end B traces the point's path with the slide and the turn advancing
     together — half way along, it's half way through both.
     Lined up on end B, the bodies can still spin about the line from end point A to end
     point B. A third pair settles that: **Start point C** on a moving body and **End point
     C** wherever it should end up. The bodies spin about that line until start C points at
     end C, and the placement is then completely decided. End point C can be anywhere — only
     which way round it sits matters, since how far along and how far out are already set by
     the other two pairs. Because of that it can only ride a circle, so four spots a quarter
     turn apart on it are marked in blue: as it sits now, a quarter turn either way, and
     upside down.
     Or type a **Roll** angle instead of picking end point C: the part turns that far about
     the line from end point A to end point B, which is often quicker than finding a point
     that says the same thing.
   - **Face Snap** — for **Moving face** and again for **Fixed face**, click a face, then
     click the point on that face that should meet the other. A face offers its corners, the
     middle of each edge, and its centre — nine points on a rectangular one — and you can
     reach a corner from just outside the face. Once a face is picked, only points on it are
     on offer, and what highlights is always what a click takes. The part lands with the one
     point on the other and the two surfaces together. **Flip** puts it on the
     other side instead, and **Turn** spins it about the fixed face — type a value or drag the
     yellow ring at the mate point (0° sits on a world axis). A **yellow curve** leaves each
     face along its normal and meets the other, and the ghost shows where the part lands.
   - **Free** — optionally pick a **Reference Point** on a moving body, then type the
     **X / Y / Z** amounts, or drag the coloured arrows on each face of the selection's tight
     bounding cuboid (all six sides; each axis has a value box beside its +face handle).
     Under **Rotation**, type **X / Y / Z** turns or drag the matching coloured rings; they
     spin the part about its own centre. Everything is an expression, so the move stays
     parametric.
3. Press **Enter**.

### What each pair decides

The same slab landing on the same plate, with one more pair each time.

Each shows the tool mid-pick: the slab still where it started, a ghost where it's going,
start A green joined to end A red, and the B and C pairs in blue with the path each point
travels.

**A alone** — start A lands on end A. The slab slides; it faces exactly as it did.

<a
  href={useBaseUrl("/app/") + "?open=" + encodeURIComponent(useBaseUrl("/img/screenshots/snap-pairs-a.bearcad.json"))}
  target="_blank"
  rel="noopener noreferrer"
  title="Open this model in BearCAD"
>
  <img src={useBaseUrl("/img/screenshots/snap-pairs-a.png")} alt="The Move preview with only the A pair picked: a ghost slid onto the plate, facing as the slab does" />
</a>

**A and B** — it also turns about end A until start B points at end B. It can still roll
about the line between the two end points.

<a
  href={useBaseUrl("/app/") + "?open=" + encodeURIComponent(useBaseUrl("/img/screenshots/snap-pairs-ab.bearcad.json"))}
  target="_blank"
  rel="noopener noreferrer"
  title="Open this model in BearCAD"
>
  <img src={useBaseUrl("/img/screenshots/snap-pairs-ab.png")} alt="The same preview with the B pair added, the ghost turned so its far corner points at end B" />
</a>

**A, B and C** — it also spins about that line until start C points at end C, standing it
on its long edge. Nothing is left to choose.

<a
  href={useBaseUrl("/app/") + "?open=" + encodeURIComponent(useBaseUrl("/img/screenshots/snap-pairs-abc.bearcad.json"))}
  target="_blank"
  rel="noopener noreferrer"
  title="Open this model in BearCAD"
>
  <img src={useBaseUrl("/img/screenshots/snap-pairs-abc.png")} alt="The same preview with the C pair added, the ghost spun up onto its long edge" />
</a>

The tool moves you along as you go: pick a body and it's ready for the start point, then the
end point, then straight on to **Start point B** in case you want the turn too, and then to
**Start point C** for the spin. Click any picker to jump back to it. The pane labels the
B and C rows **Rotation** — those four points turn the part, the A pair moves it — and
**Angle snap** at the top of that section sets how far apart the candidate dots sit:
90° offers the six axis directions, 45° offers the diagonals too. Drag the slider or type
the angle. Hovering a dot draws the turn that reaches it, in degrees — two arcs for end
point B, one for end point C. Under 30° for end point B — 5° for end point C — there are no
dots: the sphere (or, for end point C, its circle) is drawn instead and you pick anywhere on
it, with the angle reading out as you move.

## Help

![The Move tool's Context pane in Snap mode, each field explained](/img/screenshots/pane-move-snap.png)

![The Move tool's Context pane in Free mode, each field explained](/img/screenshots/pane-move-free.png)

## Scripting

```lua
-- Free: explicit components, and turns about the part's own centre.
bearcad.move_bodies{ bodies = {0}, x = 40, z = "plate_thickness" }
bearcad.move_bodies{ bodies = {0}, rz = 90 }

-- Face Snap: put a face on a face. `flip` picks the side; `spin` turns it.
bearcad.move_bodies{ bodies = {0},
  from = { body = 0, on_face = {5, 5, 5}, normal = {0, 0, 1} },
  to   = { body = 1, on_face = {40, 5, 2.5}, normal = {-1, 0, 0} },
  spin = 45 }

-- The third pair as an angle: turn 90 degrees about the end A -> end B line.
bearcad.move_bodies{ bodies = {0},
  from = { body = 0, vertex = {0, 0, 0} }, to = { body = 1, vertex = {40, 0, 0} },
  from_b = { body = 0, vertex = {10, 0, 0} }, to_b = { body = 1, vertex = {50, 0, 0} },
  roll = 90 }

-- Point Snap: land one point on another. `vertex` is a corner; `edge` takes a midpoint.
bearcad.move_bodies{ bodies = {0},
  from = { body = 0, vertex = {0, 0, 0} },
  to   = { body = 1, vertex = {40, 0, 0} } }
bearcad.move_bodies{ bodies = {0},
  from = { body = 0, edge = { {0, 0, 0}, {10, 0, 0} } },
  to   = { body = 1, edge = { {40, 0, 0}, {50, 0, 0} } } }

-- An end point can be the world origin.
bearcad.move_bodies{ bodies = {0},
  from = { body = 0, vertex = {40, 40, 0} },
  to   = { origin = true } }

-- A second pair turns it too: start B swings onto end B about end A.
bearcad.move_bodies{ bodies = {0},
  from   = { body = 0, vertex = {0, 0, 0} },
  to     = { body = 0, vertex = {0, 0, 0} },
  from_b = { body = 0, vertex = {10, 0, 0} },
  to_b   = { body = 0, vertex = {0, 10, 0} } }

-- A third pair spins it about end A → end B, so the placement is fully decided.
bearcad.move_bodies{ bodies = {0},
  from   = { body = 0, vertex = {0, 0, 0} },
  to     = { body = 0, vertex = {0, 0, 0} },
  from_b = { body = 0, vertex = {10, 0, 0} },
  to_b   = { body = 0, vertex = {10, 0, 0} },
  from_c = { body = 0, vertex = {0, 0, 10} },
  to_c   = { body = 0, vertex = {0, 10, 0} } }

-- `begin_move` takes the same arguments but leaves the tool armed rather than committing,
-- so the preview is on screen: the ghost, the A connector, and the B and C paths.
bearcad.begin_move{ bodies = {0},
  from = { body = 0, vertex = {0, 0, 0} },
  to   = { body = 1, vertex = {40, 0, 0} } }
```

Points are millimetre coordinates on the body's mesh — they only need to land on the corner or
edge you mean. `vertex` is a corner, `edge` takes an edge's midpoint, `on_edge` is a position
along one, and `on_face = {x,y,z}, normal = {x,y,z}` is a point on a flat face — the middle of
it unless you add `uv = {du, dv}`, which steps that far across the face in its own axes.
`face_center =` is the same thing as `on_face =` with no `uv`.

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
