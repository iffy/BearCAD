---
sidebar_position: 10
title: Extrude
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/extrude.svg")} width="30" /> Extrude

**Shortcut:** `E`

Extrude turns a flat sketch face into a solid — or carves into an existing solid. Click one
or more faces in the same sketch plane to include them, then drag the arrow handle or type
a distance (numbers, parameters, and expressions all work) and press **Enter**. A
translucent preview shows the result the whole time.

![An 80 x 50 mm rectangle extruded 20 mm into a solid block](/img/screenshots/extrude.png)

- The picked faces show in a **Faces** element picker in the Context pane — expand it to
  drop one with its ✕ (faces are still added by clicking them in the viewport).
- With two **concentric circles** you can extrude just the **ring** between them: click in
  the area between the circles and the tool selects the outer face minus the inner. Clicking
  the inner disc selects that; the whole outer disc still works too.
- Double-click a finished extrusion (or right-click → **Edit**) to change its faces or
  depth later.
- Typing a digit while the tool is active jumps straight into the distance field.
- A **Flip** button next to the distance field extrudes to the **other side** of the sketch
  plane, keeping the same depth — handy for pulling a profile down instead of up without
  dragging the handle back through the plane.

## Extrude up to something

While dragging the handle, hover a face, plane, or vertex: the depth snaps to it, and stays
tied to it — if that target moves later, the extrusion follows. This is how you make a part
exactly meet another surface, even a slanted one (the preview shows the true resulting
shape).

## Adding to or cutting a body

When you extrude from a face of an existing body, the Context pane offers three choices:

- **New body** — the extrusion stands alone.
- **Add** — it fuses into the existing body.
- **Cut** — it's subtracted, carving a pocket or hole. Extruding *backward* through a body
  switches to Cut automatically. This is how holes are drilled: sketch a circle on a face,
  extrude it through, Cut.

## Overlapping shapes

If two shapes in a sketch overlap — say a circle overlapping a rectangle — clicking inside
the overlap picks just that region: the intersection, or one shape minus the other.
Extruding regions lets you build shapes that neither outline makes alone, with no separate
"boolean operations" step to learn.
