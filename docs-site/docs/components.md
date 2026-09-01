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
- **Drag** any top-level row onto a component to move it there (list or graph; a name tag
  follows the cursor; or use right-click → **Move to**). Drop on the **Document** row to
  move it back out. Components drag into each other to nest.
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

The Elements pane's graph view is one element per line, with the relationships drawn
beside them as vertical lanes — the way `gitk` draws commits. A component's contents
string down the lane it opens, so its extent reads straight off the graph.

The **type filter** at the bottom of the pane applies to both the list and the graph, so
hiding a category (say, sketches) thins out either view the same way.

**Right-click any row** for the same context menu its list row offers — edit the
element, add it to a drawing, export a body or component, nest a component, move it to a
component, roll back to it, or delete it.

**Drag** a row onto a component to file it there, the same as in the list. An element's
inputs stay **above** it and its outputs stay **below** it.

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
