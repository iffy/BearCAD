---
sidebar_position: 15
title: Repeat
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/repeat.svg")} width="30" /> Repeat

Repeat lays out copies of bodies along a straight line — bolt patterns, studs along a
wall, teeth on a rack.

![A block repeated four times along the X axis](/img/screenshots/repeat.png)

## How to use it

1. Pick the **Repeat** tool and click one or more bodies. The picked set shows in the context
   pane's **Bodies** element picker — the same combo-box control the other tools use — where
   you can review and remove them.
2. Pick the **axis**: the X/Y/Z buttons in the context pane, or click any line in the
   viewport.
3. Choose how to space the copies (see the modes below) and fill in the values — every
   value is an expression, so parameters work. The context pane shows the live instance
   count as you type, and the viewport shows translucent ghosts of the copies so you can see
   the pattern before committing.
4. Press **Enter** (or the **Repeat** button).

## Spacing modes

| Mode | You give | Meaning |
|---|---|---|
| **Count × gap** | N, D | N instances with a clear gap D between them. |
| **Count fit (to end)** | N, L | N instances spread evenly; the last one *ends* at L. |
| **Count fit (start-to-start)** | N, L | N instances; the last one *starts* at L. |
| **Fill length, gap** | L, D | As many instances as fit in L with gap D. |
| **Fill length, pitch** | L, D | As many instances as fit in L at start-to-start pitch D. |
| **Fill length, max pitch** | L, D | An instance lands exactly at the end of L, spaced evenly, never farther apart than D — stud spacing. |

## What you get

The original bodies stay put as the first instance. Every further instance is a real body,
nested under the repeat element in the pane. Select the element and choose **Edit repeat**
to change anything — the copies re-space, and the set grows or shrinks with the count.
Because the values are expressions, editing a parameter re-spaces the whole pattern.

## Repeating construction planes

Repeat also copies **construction planes** along the axis. With the Repeat tool active, click a
construction plane in the Elements pane (or select it) — the context pane shows how many planes
are picked, and each copy steps along the axis by the gap or pitch you set. The copies nest under
the repeat element, and a copy follows its source: move the original plane and its repeats move
with it, each keeping its own offset on top. You can repeat bodies and planes in the same
operation.

## Repeating sketch geometry in 2D

You can also repeat geometry **inside a sketch**: pick lines and circles and duplicate them along
an in-plane direction, using the same spacing modes. The copies are new entities in the same
sketch, spaced by the count/gap (or fill) you choose — a bolt-hole row, gear teeth, a comb of
slots. This is available from scripts today:

```lua
-- Four circles in a row, 10mm gap, along the sketch's +X:
bearcad.repeat_sketch{ sketch = 0, circles = {0}, angle = 0,
                       mode = "count_gap", count = 4, spacing = 10 }

-- Duplicate two lines up the sketch's +Y at a fixed pitch:
bearcad.repeat_sketch{ sketch = 0, lines = {0, 1}, angle = 90,
                       mode = "fill_pitch", length = 60, spacing = 12 }

bearcad.edit_sketch_repeat{ index = 0, circles = {0}, angle = 0,
                            mode = "count_gap", count = 6, spacing = 10 }
```

The direction is an `angle` in degrees (0 = the sketch's +X/u), or an explicit `dir = {du, dv}`.
`mode`, `count`, `spacing`, and `length` work exactly like the 3D repeat above. `gap` is
accepted as an alias for `spacing` in every repeat call — it's the name the Repeat pane uses.

## Repeating a cut (drilling a row of holes)

Instead of copying a solid, a repeat can replay a **cut** along the axis — punch the same hole
through a body N times. Point it at the cutting extrusion and it subtracts that tool at every
instance position, so one hole becomes a row of holes. Available from scripts:

```lua
-- extrusion 1 is a hole cut through a plate; drill it 4 times, 12mm apart along X:
bearcad.repeat_cut{ cuts = {1}, axis = "x", mode = "count_gap", count = 4, spacing = 12 }
```

Spacing is centre-to-centre. The same `mode`/`count`/`spacing`/`length` options apply.

## Repeating a whole sketch along an axis

You can copy an entire sketch along an axis — each copy lands on its own construction plane,
parallel to the original and offset down the axis, carrying copies of all the sketch's lines and
circles. Handy for stacking a profile at intervals. With the Repeat tool active, click a sketch
(in the Elements pane or the viewport) to add it — the context pane shows how many sketches are
picked — then set the axis and spacing and commit. The source sketch can sit on a construction
plane or a body face. From scripts:

```lua
-- Copy sketch 0 three times, 10mm apart up the Z axis:
bearcad.repeat_sketches{ sketches = {0}, axis = "z", mode = "count_gap", count = 3, spacing = 10 }
```

The copies (and their planes) are tied to the repeat element — delete it and they go away.

## Scripting

```lua
bearcad.repeat_bodies{ bodies = {0}, axis = "x", mode = "count_gap", count = 4, spacing = 8 }
bearcad.repeat_bodies{ bodies = {0}, axis = "x", mode = "fill_max_pitch",
                       length = "wall", spacing = "16in", name = "Studs" }
bearcad.edit_repeat{ index = 0, bodies = {0}, axis = "x", mode = "count_gap", count = 6, spacing = 8 }
```
