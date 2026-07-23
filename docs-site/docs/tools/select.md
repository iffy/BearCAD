---
sidebar_position: 2
title: Select
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/select.svg")} width="30" /> Select

**Shortcut:** none — it's the default. **Esc** (with nothing in progress) always returns
to Select.

Select is for looking and picking, never for creating. Right-drag orbits, Shift+right-drag
pans, the wheel zooms, clicking picks.

## What you can pick

- **Sketch points** — endpoints, corners, circle centers. Points win over the edges they
  sit on.
- **Lines and edges** — in sketches and on bodies.
- **Faces** — of bodies and construction planes.
- **Whole bodies** — in the viewport or the Elements pane.

You can't select things hidden behind a body.

## Picking from a crowd — the Selection Exploder

When a tiny vertex or edge is buried in a cluster of overlapping things, it's hard to click
exactly the one you want. Press **Space** and everything inside your cursor's pick radius pops
out to its own spaced-apart handle, joined by a thin line back to where it really is (with a
little icon of its kind when the crowd is mixed). Now every handle is far enough apart that
there's no guessing — hover one and its real thing lights up, click it to select. Hold
**Shift** while clicking to keep the fan open and pick several. Press **Space** again, click
empty space, or switch tools to dismiss it.

You can hit **Space** any time: over a crowd it fans out several handles, over a single thing
just one, and over empty space it simply parks the hitbox circle there. A faint circle appears
under the cursor whenever two or more things are stacked, hinting that exploding will help.

## Selection feeds the other tools

Most tools act on the selection: select two lines and Constraint's Parallel lights up;
select a line and press **D** to dimension it; select edges and press **F** to fillet.
Shift+click (or ⌘/Ctrl+click) adds to a selection.

The **Elements pane** mirrors the selection and offers three views — list, tree, and a
dependency graph.

## Reviewing the selection

The context pane's **element picker** summarizes the selection by kind (e.g.
`2 ⟨line⟩ · 1 ⟨body⟩`); click it to list each element, remove any, or **Clear all**. Every
tool that gathers elements uses this same control.

With nothing selected, the context pane holds the document's
[**Default units**](/docs/parameters#display-units). **Delete** removes the selection;
**N** jumps to the name field for renaming.
