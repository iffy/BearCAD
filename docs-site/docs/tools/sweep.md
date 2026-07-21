---
sidebar_position: 11.5
title: Sweep
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/sweep.svg")} width="30" /> Sweep

Sweep pulls a flat profile along a path of sketch lines into a solid — pipes,
rails, handles, curved channels.

![A circular profile swept along a curved path into a tube](/img/screenshots/sweep.png)

## How to use it

1. Pick the **Sweep** tool and click one or more profile faces (same sketch plane).
2. Click the **path**: one or more lines — straight or curved, in any sketch — that
   connect end-to-end and cross the profile's plane. Click a picked line again to remove
   it; pick order doesn't matter, the segments chain tip-to-tail.
3. A translucent preview of the swept solid follows every pick. Choose where the result
   lands:
   - **New body** — the sweep stands alone.
   - **Add to touching bodies** — it fuses into whatever it touches.
   - **Cut bodies** — it's carved out of bodies you click into the **Cut bodies** picker;
     the preview shows the finished cut.
4. **Enter** commits; **Esc** cancels.

The context pane lists the picked profile faces and path lines as element pickers — each
row has a ✕ to remove it. In the Elements pane's graph view, the profile's sketch and
every path line feed the **Sweep** operation, and the swept body hangs off it as
its output. Select a committed sweep and press **Edit sweep** in the context pane
to re-open it with its faces, path, and body mode loaded.

## Scripting

```lua
bearcad.sweep{
  circles = { 0 },          -- and/or polygon = { line indices of a closed loop }
  path = { 4, 5 },          -- line indices, chained tip-to-tail
  body = "cut",             -- "add" | "cut"; omit for a new body
  bodies = { 0 },           -- the Add/Cut body list
  name = "Handle",
}
```

Interactive sweeps replay to the command log as the same call.
