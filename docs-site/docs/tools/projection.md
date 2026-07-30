---
sidebar_position: 11
title: Projection
---

# Projection

Available inside a sketch, **Projection** pulls outside 3D geometry onto the sketch plane
as reference lines you can snap, dimension, and constrain against.

- Click an outside **body edge** to project that edge; click a **face** or **corner** to
  project the whole body's feature edges.
- Click a **construction plane** to project the line where it crosses the sketch plane.
- Click a **projected line** to remove it from the sketch.
- Or select edges, a body, or a plane with the Select tool before opening the sketch, then
  press **Y** inside it.
- Projected lines draw dashed in their own teal, and behave like construction geometry.

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
bearcad.ui.tool("project")   -- inside a sketch
-- or project the current selection, the Y shortcut's action:
bearcad.select{ kind = "body", index = 0 }
bearcad.ui.palette("run", "project selection")
```
