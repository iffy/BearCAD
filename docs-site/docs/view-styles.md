---
sidebar_position: 6.5
slug: /view-styles
title: View styles
---

import useBaseUrl from "@docusaurus/useBaseUrl";

# View styles

How the model is *drawn*, never what it is. Pick one from the **gear icon** under the
[view bear](/docs/tools/navigation#the-view-bear), or from a script:

```lua
bearcad.ui.shading("realistic")
print(bearcad.ui.camera{}.shading)   -- reads it back
```

Every picture below is the same nine cubes, from the same camera.

## Technical

<div className="row">
<div className="col col--6">

**Wireframe** — every feature edge, nothing filled. Shows the back of the part through the
front, which is what you want when you are checking that a feature went all the way through.

<img src={useBaseUrl("/img/screenshots/view-styles-wireframe.png")} alt="Nine cubes drawn as edges only, with the far edges showing through" />

</div>
<div className="col col--6">

**Transparent solid** — filled, but see-through. Where a part sits inside another, without
losing the shape of either.

<img src={useBaseUrl("/img/screenshots/view-styles-transparent.png")} alt="Nine cubes filled translucently, overlaps showing through" />

</div>
</div>

<div className="row">
<div className="col col--6">

**Solid** — opaque, flat-shaded, no edge overlay. The default: the fewest lines that still
read as a solid.

<img src={useBaseUrl("/img/screenshots/view-styles-solid.png")} alt="Nine cubes as opaque flat-shaded solids" />

</div>
<div className="col col--6">

**Solid + wireframe** — opaque, with every edge drawn on top and through. A drawing of the
part's construction over the part.

<img src={useBaseUrl("/img/screenshots/view-styles-solid_wireframe.png")} alt="Nine opaque cubes with all their edges overlaid, including hidden ones" />

</div>
</div>

<div className="row">
<div className="col col--6">

**Realistic** — lit rather than flat-shaded, with a contact shadow on the build plane. The
shadow is the only cue for whether a part rests on the ground or floats above it.

<img src={useBaseUrl("/img/screenshots/view-styles-realistic.png")} alt="Nine cubes lit with highlights and a soft contact shadow" />

</div>
<div className="col col--6">
</div>
</div>

## Pencil

Second row of the popup. White paper, strokes that overshoot their corners and wobble along
their length, and a hatched contact shadow. The same edge is drawn the same way from every
angle, so the drawing doesn't crawl as you orbit.

<div className="row">
<div className="col col--6">

**Loose pencil** — one pencil, one colour.

<img src={useBaseUrl("/img/screenshots/view-styles-loose_pencil.png")} alt="Nine cubes drawn in graphite on white paper with hand-drawn strokes" />

</div>
<div className="col col--6">

**Coloured pencil** — each body in its own colour, shaded with strokes; solids shadow each other.

<img src={useBaseUrl("/img/screenshots/view-styles-colour_pencil.png")} alt="The same hand-drawn cubes, each in its own colour" />

</div>
</div>

## Projections on a drawing

A [projection](/docs/drawing-tools/projection) on a drawing page has its own **Style**, set
per view rather than per viewport:

| Style | |
|---|---|
| Visible edges | Hidden lines removed. |
| Wireframe | Every feature edge, including the back ones. The default. |
| Shaded | Grey-shaded faces under the visible edges. |
| Colorful | Shaded, keeping each body's material colour. |
| Loose pencil | The visible edges drawn by hand. |
| Coloured pencil | The same hand, in each body's colour, shaded with strokes and with the solids' shadows on each other. |

```lua
bearcad.drawing_view_style{ drawing = 0, view = 0, style = "colorful" }
```
