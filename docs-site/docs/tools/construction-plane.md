---
sidebar_position: 9
title: Construction Plane
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/plane.svg")} width="30" /> Construction Plane

Construction planes are invisible flat surfaces to sketch on — for building at an offset
from a face, or at an angle. Click a reference, position the plane, press **Enter**:

![A construction plane offset above a block, holding a circle sketch](/img/screenshots/construction-plane.png)

- **Click a face** (the ground, another plane, or a body's face): the new plane sits
  parallel to it. Drag the arrow handle or type an offset.
- **Click an edge or axis** (a sketch line, a body edge, or one of the origin's X/Y/Z
  axes): the plane pivots around it. Set an offset *and* an angle — the angle handle on the
  ring rotates it.

Planes never render as solid and never appear in exports; they exist only to hold sketches.
Their handles stay visible and grabbable even when a body is in front of them.
