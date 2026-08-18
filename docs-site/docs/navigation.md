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

### On a touch screen

| Gesture | Action |
|---|---|
| Two-finger drag | Pan |
| Pinch | Zoom |
| Three-finger drag | Orbit |

Touch mode switches on at the first touch: pick targets grow to finger size, and on
phone-width screens the side panes become floating windows, toggled from the bottom bar.
While a finger drags a shape or vertex, a **magnifier** floats beside it showing what the
finger covers, so snap points stay aimable.
A trackpad pinch zooms the same way. Focusing any value field floats an **on-screen
keypad** — digits, units, operators, and your parameter names as one-tap chips — and a
**long press** opens the same menus a right-click does.

**Zoom to fit** (in the command palette and the View menu) frames your selection — or the
whole model — with a short glide (faster than Home). Settings → **Animate zoom to fit** turns
the glide off if you prefer an instant snap.

**View → Tool Hints** hides the usage line at the bottom of the viewport. Scripts:
`bearcad.ui.tool_hints(false)`.

**Auto-zoom** (the toggle next to Zoom to fit in the toolbar) keeps your geometry
framed: type rectangle dimensions bigger than the view, or drag an extrusion past the
edge, and the camera glides out to fit — shrink it back and the camera glides in.
Committed results count too: confirm an extrusion taller than the view and the camera
glides out to show the whole body; undo it and the view glides back in. Selections too:
pick a face that's half out of view and the camera glides out until all of it is on
screen — only ever outward, so selecting something small never dives the camera in.
Scripts: `bearcad.ui.auto_zoom(true)`.

## Keyboard shortcuts

**View → Keyboard Shortcuts** (also under Help) lists every binding in the app, grouped
by where it applies.

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
  lighting. Curved surfaces shade smoothly, while real edges — box corners, chamfers,
  extrusion caps — stay crisp.
- **Ground** — grid lines or a solid ground plane. Grid lines follow the document's
  units — millimetre powers of ten, or inches and feet — and finer subdivisions fade
  in between the heavier lines as you zoom.

These change how you *see* the model, never the model itself.

## Sketch mode

While a sketch is open the viewport has an **orange border**. The camera still works
normally; **Esc** leaves the sketch.
