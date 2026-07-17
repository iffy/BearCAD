---
sidebar_position: 8
title: Chamfer
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/chamfer.svg")} width="30" /> Chamfer

**Shortcut:** `K`

Chamfer is [Fillet](./fillet.md)'s angular sibling: instead of rounding a corner, it cuts
it off flat. Everything else works the same way —

- **In a sketch:** click a corner where two lines meet, drag the handle or type a cut
  distance, **Enter**.

![A rectangle profile with its top-right corner chamfered flat](/img/screenshots/chamfer-sketch.png)

- **On a solid:** click an edge (Shift+click for several), set the distance, **Enter**.

![A box with its two long top edges chamfered](/img/screenshots/chamfer.png)

**Countersinking screw holes** is a chamfer too: click the rim of a drilled hole, set the
distance, **Enter** — the rim is cut into a cone, ready for a flat-head screw. The
[Quickstart bracket](/docs/quickstart#7-countersink-the-holes) does exactly this.

See [Fillet](./fillet.md) for the shared details: live preview, the Context-pane edge list,
expression input, and current limitations.
