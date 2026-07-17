---
sidebar_position: 6
slug: /tools/navigation
title: Navigation
---

# Navigation

## Camera

| Input | Action |
|---|---|
| Right-drag | Orbit around the part |
| Middle-drag, or Shift + right-drag | Pan |
| Mouse wheel | Zoom |
| **Esc** | Cancel what's in progress; again to return to Select |

Only the Select tool treats plain clicks as picking — drawing tools use them to draw,
which is why the camera bindings live on the *right* mouse button.

**Zoom to fit** (in the command palette and the View menu) frames your selection — or the
whole model — in one step.

## Command palette

**⌘/Ctrl+P** opens the command palette: a searchable list of context-pertinent commands —
tools, views, document actions — filtered as you type. Arrow keys move the highlight and
**Enter** runs it. Any action without a visible button is reachable here.

## The view bear

The bear-shaped cube in the corner — the **view bear** — snaps to standard views: click a
face, edge, or corner. The house icon returns to the **Home** view (right-click it to save
the current view as Home).

The **gear icon** under the view bear opens display settings:

- **Projection** — orthographic (flat, technical) or perspective (natural).
- **Shading** — wireframe, transparent, solid, solid + visible edges, or realistic
  lighting.
- **Ground** — grid lines or a solid ground plane.

These change how you *see* the model, never the model itself.

## Sketch mode

While a sketch is open the viewport has an **orange border**. The camera still works
normally; **Esc** leaves the sketch.
