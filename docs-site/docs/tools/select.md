---
sidebar_position: 2
title: Select
---

# Select

**Shortcut:** none — it's the default. **Esc** (with nothing in progress) always returns
to Select.

Select is for looking and picking, never for creating. Right-drag orbits, Shift+right-drag
pans, the wheel zooms, and clicking picks whatever is under the cursor.

## What you can pick

- **Sketch points** — line endpoints, corners, circle centers. Points win over the edges
  they sit on when you click near a corner.
- **Lines and edges** — in sketches and on solid bodies.
- **Faces** — of bodies and construction planes.
- **Whole bodies** — in the viewport or in the Elements pane.

Anything pickable highlights as you hover it, and what highlights is exactly what a click
will select. You can't select things hidden behind a body.

## Selection feeds the other tools

Most tools act on what you've selected: select two lines and the
[Constraint](./constraint.md) tool's Parallel button lights up; select a line and press
**D** to dimension it; select edges and press **F** to fillet them. Shift+click (or
⌘/Ctrl+click) adds to a selection.

The **Elements pane** mirrors your selection and offers three views — a flat list, an
indented tree, and a graph of what depends on what. Hovering a row highlights that element
in the viewport.

## Reviewing the selection

Once you've picked something, the **context pane** lists the selection as an element picker —
one row per selected element, each with a remove button, plus a **Clear all**. It's the same
picker the tools that gather elements (like Loft or Fillet) use, so pruning a selection works
the same way everywhere.
