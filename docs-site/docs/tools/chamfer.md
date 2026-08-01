---
sidebar_position: 9
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
  The Context pane mirrors the **Distance**, with a ✓ to commit.

![A box with its two long top edges chamfered](/img/screenshots/chamfer.png)

## Help

![The Chamfer tool's Context pane inside a sketch, each field explained](/img/screenshots/pane-chamfer-sketch.png)

![The Chamfer tool's Context pane on a solid, each field explained](/img/screenshots/pane-chamfer-body.png)
