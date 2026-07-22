---
sidebar_position: 18
title: Mirror
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/mirror.svg")} width="30" /> Mirror

Mirror reflects whole bodies across a plane, adding a mirrored copy of each while keeping
the originals.

## How to use it

1. Pick the **Mirror** tool.
2. Click a **construction plane** or a **flat face** to set the mirror plane. (Use the
   **✕** next to the plane in the context pane to pick a different one.)
3. Click one or more **bodies** to reflect. Re-clicking removes one. A translucent ghost
   previews each reflection as you go.
4. Press **Enter**.

Each picked body gains a reflected copy — a real body you can use in further operations.
The originals stay exactly where they were: a mirror *adds* geometry rather than moving it.

**Edit mirror** (double-click the operation, or the button in the context pane) re-opens
the tool with its plane and bodies loaded, so you can change either later. Deleting the
mirror removes just the reflected copies.

Because the reflection goes through the geometry kernel, mirrored bodies combine into
[booleans](/docs/tools/combine) and export as real STEP surfaces just like any other body.

## Scripting

```lua
bearcad.mirror_bodies{ plane = { kind = "construction_plane", index = 0 }, bodies = { 0, 1 } }
bearcad.edit_mirror{ index = 0, plane = { kind = "construction_plane", index = 0 }, bodies = { 0 } }
```
