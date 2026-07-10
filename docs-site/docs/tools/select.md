---
sidebar_position: 2
title: Select
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/select.svg")} width="30" /> Select

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

The **context pane** shows your selection in the **element picker** — a combo-box-style input
that looks and focuses like any other field. Empty, it reads "Nothing selected"; once you've
picked things it shows a compact summary of counts by kind (for example `2 ⟨line⟩ · 1 ⟨body⟩`).
Click it to expand a popup that lists each picked element by name, with a remove button on
every row and a **Clear all** at the bottom.

It's the same control every tool that gathers elements uses (Loft, Fillet, and the rest), so
reviewing and pruning a selection works the same way everywhere. Each tool configures its own
picker — which kinds of element it will accept and how many — but the look and the interaction
are always the same.
