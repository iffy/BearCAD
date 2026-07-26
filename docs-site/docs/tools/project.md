---
title: Project
---

# Project

Available inside a sketch, **Project** pulls outside 3D geometry onto the sketch plane as
reference lines you can snap, dimension, and constrain against.

- Click an outside **body edge** to project that edge; click a **face** or **corner** to
  project the whole body's feature edges.
- Or select edges (or a body) with the Select tool before opening the sketch, then press
  **Y** inside it.
- Projected lines draw dashed in their own teal, and behave like construction geometry.

Projections are **associative**: when the source body changes, the projected lines follow
it. Imported units project too — a [unit's face outline](/docs/files#importing-bearcad-files)
lands in a sketch opened on it automatically.

## Help

![The Project tool's Context pane, each field explained](/img/screenshots/pane-project.png)

## Scripting

```lua
bearcad.ui.tool("project")   -- inside a sketch
-- or project the current selection, the Y shortcut's action:
bearcad.select{ kind = "body", index = 0 }
bearcad.ui.palette("run", "project selection")
```
