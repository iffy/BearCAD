---
sidebar_position: 11
title: Viewport styles
---

# Viewport styles

![A selected body — a cube with a cylinder boss — outlined by the blue selection aura](/img/screenshots/styles-scene.png)

What the colors in the 3D viewport mean, for every kind of geometry and state.

## Lines

Solid sketch lines change color with their constraint state; construction and projected
geometry draw dashed in their own colors.

| Kind | Normal | Hovered | Selected |
|---|---|---|---|
| **Unconstrained** — can still move | ![](/img/screenshots/styles/line-normal.png) | ![](/img/screenshots/styles/line-normal-hovered.png) | ![](/img/screenshots/styles/line-normal-selected.png) |
| **Fully constrained** — dimensioned and immobile | ![](/img/screenshots/styles/line-constrained.png) | ![](/img/screenshots/styles/line-constrained-hovered.png) | ![](/img/screenshots/styles/line-constrained-selected.png) |
| **Construction** — reference geometry, never part of the solid | ![](/img/screenshots/styles/line-construction.png) | ![](/img/screenshots/styles/line-construction-hovered.png) | ![](/img/screenshots/styles/line-construction-selected.png) |
| **Projected** — traced from outside the sketch (press `Y`); follows its source | ![](/img/screenshots/styles/line-projected.png) | ![](/img/screenshots/styles/line-projected-hovered.png) | ![](/img/screenshots/styles/line-projected-selected.png) |

## Points

Line endpoints and circle centers.

| Normal | Hovered | Selected |
|---|---|---|
| ![](/img/screenshots/styles/point-normal.png) | ![](/img/screenshots/styles/point-hovered.png) | ![](/img/screenshots/styles/point-selected.png) |

## Faces

Faces highlight when hovered — for picking a face to sketch on or extrude.

| Normal | Hovered |
|---|---|
| ![](/img/screenshots/styles/face-normal.png) | ![](/img/screenshots/styles/face-hovered.png) |

## Dimensions

Committed dimensions draw in grey; hovering one (to drag its label or double-click to edit)
recolors it with the edit accent.

| Kind | Normal | Hovered |
|---|---|---|
| **Linear** | ![](/img/screenshots/styles/dim-linear.png) | ![](/img/screenshots/styles/dim-linear-hovered.png) |
| **Angle** | ![](/img/screenshots/styles/dim-angle.png) | ![](/img/screenshots/styles/dim-angle-hovered.png) |

## Bodies

A selected body fills in a more saturated blue; a hovered one in a warm gold-grey. In
wireframe mode the lines recolor instead of the fill.

| Normal | Hovered (Elements pane) | Selected |
|---|---|---|
| ![](/img/screenshots/styles/body-normal.png) | ![](/img/screenshots/styles/body-hovered.png) | ![](/img/screenshots/styles/body-selected.png) |
