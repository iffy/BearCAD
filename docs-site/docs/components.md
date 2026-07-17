---
sidebar_position: 7
title: Components
---

# Components

Components group top-level elements — planes, features, bodies, drawings — into named,
nestable folders in the Elements pane. Grouping is organizational: it never changes
geometry.

- The pane header's **+** button adds a component; right-click a component for **New
  component inside**.
- Contents indent under the component; the **triangle** collapses/expands them.
- **Drag** any top-level row onto a component to move it there (or use right-click →
  **Move to**). Drop on the **Document** row to move it back out. Components drag into
  each other to nest.
- **Hiding** a component hides everything inside it, nested components included.
- Deleting a component keeps its contents — they move to its parent.

## Units

Each component can override the **length and angle units** (select it and use the
Component units pickers). Contents inherit through the chain: sketch override → component
→ parent components → document default.

## Graph view

In the Elements pane's graph view, components draw as smooth, lightly shaded areas
encompassing their member nodes rather than as nodes themselves.

## Scripting

```lua
local frame = bearcad.component{ name = "Frame" }          -- returns the index
local legs  = bearcad.component{ name = "Legs", parent = frame }
bearcad.move_to_component{ kind = "extrusion", index = 0, component = frame }
bearcad.move_to_component{ kind = "body", index = 0, component = false }  -- back to root
bearcad.set_units{ component = frame, length = "in" }
bearcad.select{ kind = "component", index = frame }
bearcad.count("component")
```
