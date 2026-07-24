---
sidebar_position: 8
title: Components
---

# Components

Components group top-level elements — planes, features, bodies, drawings — into named,
nestable folders in the Elements pane. Grouping is organizational: it never changes
geometry.

- The pane header's **+** button adds a component; right-click a component for **New
  component inside**.
- A new component is selected and becomes **active** (marked with an accent-colored dot): elements you
  create land inside it. Selecting a component activates it; click the **Document** row
  to go back to creating at the root.
- Contents indent under the component; the **triangle** collapses/expands them.
- **Drag** any top-level row onto a component to move it there (a name tag follows the
  cursor; or use right-click → **Move to**). Drop on the **Document** row to move it back
  out. Components drag into each other to nest.
- **Hiding** a component hides everything inside it, nested components included.
- Deleting a component keeps its contents — they move to its parent.
- **Export** a component straight to **STL** or **STEP** from its right-click menu: every
  body inside it (and its nested components) is written to one file, named after the
  component.

## Units

Each component can override the **length and angle units** (select it and use the
Component units pickers). Contents inherit through the chain: sketch override → component
→ parent components → document default.

## Graph view

In the Elements pane's graph view, components draw as smooth, lightly shaded areas
encompassing their member nodes rather than as nodes themselves.

The nodes lay themselves out with a force simulation that repels overlaps and spaces
things apart. A **force-layout toggle** (the button beside the List/Graph buttons, shown
in graph mode) turns that on or off: leave it on to let a busy graph untangle itself, or
turn it off to freeze the layout so it holds still while you read it or drag nodes into
place. The **type filter** at the bottom of the pane applies to both the list and the
graph, so hiding a category (say, sketches) thins out either view the same way.

## Rolling back

Right-click any element in the Elements pane and open the **Rollback** submenu:

- **Rollback to here** — see the model as it was just *after* that element: everything that
  **depends on** it (the operations built on it and their results) is hidden, but the element
  itself stays.
- **Rollback to just before here** — hide that element too, so you see the model as it was
  just *before* it was added.

Hidden elements are suppressed in the viewport and faded in the pane, without touching your
own show/hide toggles. Independent branches stay put, so this follows the element graph
rather than the order things were created. While rolled back, a status line at the top of
the pane shows where you are; click **Done** to roll forward again.

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
