---
sidebar_position: 3
title: Viewport styles
---

# Viewport styles

![A selected body — a cube with a cylinder boss — outlined by the blue selection aura](/img/screenshots/styles-scene.png)

How each kind of geometry is styled in the 3D viewport, in every state. These swatches are
generated straight from the renderer's color constants (`cargo test
generate_style_swatches -- --ignored`), so they cannot drift from the app.

## Lines

Solid sketch lines change color with their constraint state; construction and projected
geometry draw dashed in their own colors.

| Kind | Normal | Hovered | Selected |
|---|---|---|---|
| **Unconstrained** — still has freedom | ![](/img/screenshots/styles/line-normal.png) | ![](/img/screenshots/styles/line-normal-hovered.png) | ![](/img/screenshots/styles/line-normal-selected.png) |
| **Fully constrained** — dimensioned and immobile (the same signal that blocks dragging) | ![](/img/screenshots/styles/line-constrained.png) | ![](/img/screenshots/styles/line-constrained-hovered.png) | ![](/img/screenshots/styles/line-constrained-selected.png) |
| **Construction** — reference geometry, never part of the solid model | ![](/img/screenshots/styles/line-construction.png) | ![](/img/screenshots/styles/line-construction-hovered.png) | ![](/img/screenshots/styles/line-construction-selected.png) |
| **Projected** — an associative projection of external 3D geometry (press `Y`); follows its source and is not draggable | ![](/img/screenshots/styles/line-projected.png) | ![](/img/screenshots/styles/line-projected-hovered.png) | ![](/img/screenshots/styles/line-projected-selected.png) |

Hovering draws the pick highlight (a thicker stroke in the hover color with endpoint dots)
over the line; selecting redraws it in the selection-highlight gold.

## Points

Line endpoints and circle centers.

| Normal | Hovered | Selected |
|---|---|---|
| ![](/img/screenshots/styles/point-normal.png) | ![](/img/screenshots/styles/point-hovered.png) | ![](/img/screenshots/styles/point-selected.png) |

## Faces

Faces highlight on hover (a translucent tint plus a border in the hover color) — for
sketching-on-face, extruding, and 3D face picking. Faces are not persistently selectable
yet, so there is no selected state.

| Normal | Hovered |
|---|---|
| ![](/img/screenshots/styles/face-normal.png) | ![](/img/screenshots/styles/face-hovered.png) |

## Bodies

A selected body (or one hovered in the Elements pane) gets an **aura**: a solid outline
offset a few pixels *outside* its screen-space silhouette. The aura is blue for selection
and uses the hover color for pane hover; bodies in front of the silhouette occlude it, and
the auras of nearby selected bodies join.

| Normal | Hovered (Elements pane) | Selected |
|---|---|---|
| ![](/img/screenshots/styles/body-normal.png) | ![](/img/screenshots/styles/body-hovered.png) | ![](/img/screenshots/styles/body-selected.png) |
