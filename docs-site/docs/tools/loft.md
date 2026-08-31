---
sidebar_position: 15
title: Loft
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/loft.svg")} width="30" /> Loft

Loft blends a solid through two or more closed cross-section profiles on different planes —
horns, hulls, funnels, transitions between shapes.

<a
  href={useBaseUrl("/app/") + "?open=" + encodeURIComponent(useBaseUrl("/img/screenshots/loft.bearcad.json"))}
  target="_blank"
  rel="noopener noreferrer"
  title="Open this model in BearCAD"
>
  <img src={useBaseUrl("/img/screenshots/loft.png")} alt="Two circle sections on offset planes blended into a horn" />
</a>

## How to use it

1. Sketch a closed profile (a circle or a loop of lines) on each plane you want the solid
   to pass through — use [Construction Planes](/docs/tools/construction-plane) to stack
   section planes at the offsets you need.
2. Pick the **Loft** tool and click each profile — a click on any line of a loop picks
   the whole loop; clicking a picked section removes it.
3. Choose where the result lands in the context pane — **New body**, **Add to touching
   bodies**, or **Cut bodies** (click the bodies to carve in the viewport) — the same
   three buttons Revolve and Sweep have.
4. With two or more sections picked, press **Enter**. Sections blend in order along the
   loft's direction — pick order doesn't matter.

## Good to know

- Sections can be different shapes: a circle can blend into a rectangle.
- The loft is parametric — edit a section profile (dimensions, position) and the solid
  reshapes to match.
- A loft undoes as one step, and appears in the Elements pane with its body.

## Help

![The Loft tool's Context pane, each field explained](/img/screenshots/pane-loft.png)

## Scripting

```lua
bearcad.circle{ r = 5 }
bearcad.plane{ offset = 10 }
-- Plane 3: the new one, after the document's three starting planes.
bearcad.begin_sketch{ kind = "plane", index = 3 }
bearcad.circle{ r = 2 }
bearcad.exit_sketch()
bearcad.loft{ circles = {0, 1}, name = "Horn" }
```

`polygons = {{line, ...}, ...}` lofts line loops; each face's sketch is inferred, like
`bearcad.extrude`. `bearcad.edit_loft{ index, … }` re-points one.
