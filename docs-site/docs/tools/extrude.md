---
sidebar_position: 12
title: Extrude
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/extrude.svg")} width="30" /> Extrude

**Shortcut:** `E`

Extrude turns a flat sketch face into a solid — or carves into an existing one. Click one
or more faces in the same sketch plane, set a depth, and press **Enter** (or the **Extrude**
button in the panel).

![An 80 x 50 mm rectangle extruded 20 mm into a solid block](/img/screenshots/extrude.png)

Set the depth any of these ways:

- **Drag** the arrow handle in the 3D view.
- **Type** a distance (expressions work) in the **Distance** field — in the 3D view or the
  panel; the two mirror each other.
- **Extrude up to** a plane or face — see below.

Nothing commits until you press **Enter** or the **Extrude** button, so you can keep
adjusting the distance or target first.

- With two **concentric circles**, click between them to extrude just the **ring**.
- Double-click a finished extrusion (or right-click → **Edit**) to change its faces or
  depth later.
- Typing a digit jumps straight into the distance field.
- **Flip** extrudes to the other side of the sketch plane, keeping the same depth.

## Extrude up to something

Drag the handle onto a face, plane, or vertex and the depth snaps to it and stays tied to
it — if that target moves later, the extrusion follows. Works for slanted surfaces too. The
target shows in the panel's **Up to** picker; you can also focus that picker and click the
plane or face directly, or clear it there to go back to a plain distance. Setting a target
clears the Distance field, since the depth then comes from the target.

## Adding to or cutting a body

Extruding from a face of an existing body offers three choices:

- **New body** — the extrusion stands alone.
- **Add** — it fuses into the existing body.
- **Cut** — it's subtracted. Extruding *backward* through a body switches to Cut
  automatically. This is how holes are drilled: sketch a circle on a face, extrude it
  through, Cut.

## Overlapping shapes

Clicking inside an overlap picks just that region — the intersection, or one shape minus
the other — so overlapping outlines build shapes neither makes alone.
