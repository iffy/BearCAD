---
sidebar_position: 13
title: Loft
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/loft.svg")} width="30" /> Loft

Loft blends a solid through two or more closed cross-section profiles on different planes —
horns, hulls, funnels, transitions between shapes.

![Two circle sections on offset planes blended into a horn](/img/screenshots/loft.png)

## How to use it

1. Sketch a closed profile (a circle or a loop of lines) on each plane you want the solid
   to pass through — use [Construction Planes](/docs/tools/construction-plane) to stack
   section planes at the offsets you need.
2. Pick the **Loft** tool and click each profile — a click on any line of a loop picks
   the whole loop; clicking a picked section removes it.
3. With two or more sections picked, press **Enter**. Sections blend in order along the
   loft's direction — pick order doesn't matter.

## Good to know

- Sections can be different shapes: a circle can blend into a rectangle.
- The loft is parametric — edit a section profile (dimensions, position) and the solid
  reshapes to match.
- A loft undoes as one step, and appears in the Elements pane with its body.

## Scripting

```lua
bearcad.circle{ r = 5 }
bearcad.plane{ offset = 10 }
bearcad.begin_sketch{ kind = "plane", index = 1 }
bearcad.circle{ r = 2 }
bearcad.exit_sketch()
bearcad.loft{ circles = {0, 1}, name = "Horn" }
```

`polygons = {{line, ...}, ...}` lofts line loops; each face's sketch is inferred, like
`bearcad.extrude`.
