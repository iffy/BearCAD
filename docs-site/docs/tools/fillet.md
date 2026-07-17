---
sidebar_position: 7
title: Fillet
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/fillet.svg")} width="30" /> Fillet

**Shortcut:** `F`

Fillet rounds corners. It works in two places:

**In a sketch:** click a corner where two lines meet, then drag the handle or type a
radius; **Enter** commits. A live preview shows the rounded corner as you adjust it. This
is how you round a profile *before* extruding.

![A rectangle profile with its top-right corner rounded](/img/screenshots/fillet-sketch.png)

**On a solid:** with no sketch open, click an edge of a body — a vertical corner edge, an
edge where a wall meets the top or bottom face, or the **round rim** of a cylinder or a
drilled hole. Shift+click more edges to round several with one radius, then drag or type
and press **Enter** — like the rounded bend in the
[Quickstart bracket](/docs/quickstart#5-round-the-bend).
The picked edges are listed in the Context pane, where individual ones
can be removed before committing.

![A box with its four vertical edges rounded](/img/screenshots/fillet.png)

## Good to know

- The radius field takes expressions.
- A radius that can't physically fit is rejected at commit rather than producing broken
  geometry.
- Rounding solid edges works on bodies made from sketch profiles.
- **Edit the amount later:** double-click the fillet's Elements pane row (or right-click →
  **Edit fillet**) to bring back its gizmo and amount input.
