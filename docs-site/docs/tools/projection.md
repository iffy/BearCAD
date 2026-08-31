---
sidebar_position: 12
title: Projection
---

# Projection

**Projection** (**P**) pulls outside 3D geometry onto the sketch plane as reference lines
you can snap, dimension, and constrain against. Outside a sketch it clicks a face to start
one, like Offset.

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
bearcad.begin_sketch{ kind = "plane", index = 0 }
bearcad.project{ body = 0 }          -- all edges of body 0
bearcad.project{ plane = 2 }         -- construction plane 2 (YZ)
bearcad.project{ entities = { { kind = "body", index = 0 } } }

-- Current selection (same as Enter on the tool), including un-project:
bearcad.select{ kind = "plane", index = 2 }
bearcad.project()
```
