---
sidebar_position: 12
title: Projection
---

# Projection

Available inside a sketch (**P**), **Projection** pulls outside 3D geometry onto the sketch
plane as reference lines you can snap, dimension, and constrain against.

- Select outside **body edges** (a **face** or **corner** takes the whole body), or a
  **construction plane**, then press **Enter** (or the blue commit button) to project.
- Select only **projected lines** and press **Enter** to un-project them.
- Or select edges/a body/a plane before opening the sketch, then **P** → **Enter**.
- Projected lines draw solid cyan, behave like construction geometry, and show the
  projector icon in the Elements pane.

The tool picks outside geometry only; the sketch's own drawn geometry belongs to the other
sketch tools.

Projections are **associative**: when the source body changes — or the source plane moves
or is resized — the projected lines follow it. Imported units project too — a
[unit's face outline](/docs/files#importing-bearcad-files)
lands in a sketch opened on it automatically.

## Help

![The Projection tool's Context pane, each field explained](/img/screenshots/pane-projection.png)

## Scripting

```lua
bearcad.ui.tool("project")   -- inside a sketch (same as P)
bearcad.select{ kind = "body", index = 0 }
bearcad.ui.palette("run", "project selection")  -- Enter on the tool
```
