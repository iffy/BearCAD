---
sidebar_position: 18
title: Repeat
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/repeat.svg")} width="30" /> Repeat

Repeat lays out copies of bodies along a straight line — bolt patterns, studs along a
wall, teeth on a rack.

![A block repeated four times along the X axis](/img/screenshots/repeat.png)

## How to use it

1. Pick the **Repeat** tool and click one or more bodies (a body selected beforehand is
   picked automatically).
2. Pick the **axis**: click any body edge, sketch line, or origin axis in the viewport — each
   lights up under the cursor while the axis is what you're picking.
3. Choose a spacing mode and fill in the values — every value is an expression, so
   parameters work. Ghost previews show the pattern.
4. Press **Enter**.

A **distance handle** sits on the pattern along the axis: click it to grab, move the mouse to
drag the distance out, click again to drop it.

Count, Offset and Distance are interlinked: two are yours to set and the third is computed.
A **green lock** marks the computed one; click a grey lock to move it there. Clicking the
**Offset** or **Distance** label (or its icon) switches how that value is measured.

The originals stay put as the first instance; every copy is a real body nested under the
repeat element. **Edit repeat** on the element changes anything later.

## Spacing modes

| Mode | You give | Meaning |
|---|---|---|
| **Count × gap** | N, D | N instances with a clear gap D between them. |
| **Count fit (to end)** | N, L | N instances spread evenly; the last one *ends* at L. |
| **Count fit (start-to-start)** | N, L | N instances; the last one *starts* at L. |
| **Fill length, gap** | L, D | As many instances as fit in L with gap D. |
| **Fill length, pitch** | L, D | As many instances as fit in L at start-to-start pitch D. |
| **Fill length, max pitch** | L, D | An instance lands exactly at the end of L, spaced evenly, never farther apart than D — stud spacing. |

## Repeating construction planes

With the Repeat tool active, click a construction plane in the Elements pane. Copies step
along the axis, nest under the repeat element, and follow the original plane if it moves.
Bodies and planes can repeat in the same operation.

## Repeating sketch geometry in 2D

Repeat lines and circles inside a sketch along an in-plane direction, with the same
spacing modes. From scripts:

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

Direction is an `angle` in degrees (0 = the sketch's +X/u) or an explicit
`dir = {du, dv}`. `gap` is accepted as an alias for `spacing` in every repeat call.

## Repeating a cut (drilling a row of holes)

A repeat can replay a **cut** along the axis — one hole becomes a row of holes. Spacing is
centre-to-centre. From scripts:

```lua
-- extrusion 1 is a hole cut through a plate; drill it 4 times, 12mm apart along X:
bearcad.repeat_cut{ cuts = {1}, axis = "x", mode = "count_gap", count = 4, spacing = 12 }
```

## Repeating a whole sketch along an axis

Copy an entire sketch along an axis — each copy lands on its own parallel construction
plane with copies of the sketch's lines and circles. With the Repeat tool active, click a
sketch, set the axis and spacing, and commit. Delete the repeat element and the copies go
away. From scripts:

```lua
-- Copy sketch 0 three times, 10mm apart up the Z axis:
bearcad.repeat_sketches{ sketches = {0}, axis = "z", mode = "count_gap", count = 3, spacing = 10 }
```

## Scripting

```lua
bearcad.repeat_bodies{ bodies = {0}, axis = "x", mode = "count_gap", count = 4, spacing = 8 }
bearcad.repeat_bodies{ bodies = {0}, axis = "x", mode = "fill_max_pitch",
                       length = "wall", spacing = "16in", name = "Studs" }
bearcad.edit_repeat{ index = 0, bodies = {0}, axis = "x", mode = "count_gap", count = 6, spacing = 8 }
```

`axis` is `"x"`/`"y"`/`"z"`, a sketch line (`{ line = 0 }`), or a body edge given by its world
endpoints (`{ body = 0, from = {0, 0, 0}, to = {20, 0, 0} }`) — the same three the picker takes.
Revolve and Move accept all three too.
