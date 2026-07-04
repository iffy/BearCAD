---
sidebar_position: 13
title: Navigation
---

# Navigation

## Camera

| Input | Action |
|---|---|
| Right-drag | Orbit around the part |
| Shift + right-drag | Pan |
| Mouse wheel | Zoom |
| **Esc** | Cancel what's in progress; again to return to Select |

Only the Select tool treats plain clicks as picking — drawing tools use them to draw,
which is why the camera bindings live on the *right* mouse button.

**Zoom to fit** (in the command palette and the View menu) frames your selection — or the
whole model — in one step.

## The view cube

The cube in the corner snaps to standard views: click a face, edge, or corner. The house
icon returns to the **Home** view (right-click it to save the current view as Home).

The **gear icon** under the cube opens display settings:

- **Projection** — orthographic (flat, technical) or perspective (natural).
- **Shading** — wireframe, transparent, solid, solid + visible edges, or realistic
  lighting.
- **Ground** — grid lines or a solid ground plane.

These change how you *see* the model, never the model itself.

## Sketch mode

While a sketch is open the viewport has an **orange border** as a reminder. The camera
still works normally; press **Esc** to leave the sketch.

## Hover feedback

Anything you can click highlights as the cursor approaches — lines and points within a
comfortable distance, not just pixel-perfect hits. What highlights is exactly what a click
will do.
