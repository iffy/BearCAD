---
sidebar_position: 6.5
slug: /view-styles
title: View styles
---

import useBaseUrl from "@docusaurus/useBaseUrl";

# View styles

How the model is *drawn*, never what it is. Pick one from the **gear icon** under the
[view bear](/docs/tools/navigation#the-view-bear).

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

Second row of the popup. Strokes that overshoot their corners and wobble along their length,
and a hatched contact shadow. The same edge is drawn the same way from every angle, so the
drawing doesn't crawl as you orbit.

<div className="row">
<div className="col col--6">

**Loose pencil** — one pencil, one color, on white paper.

<img src={useBaseUrl("/img/screenshots/view-styles-loose_pencil.png")} alt="Nine cubes drawn in graphite on white paper with hand-drawn strokes" />

</div>
<div className="col col--6">

**Dark pencil** — the same hand with the lights out: a white pencil on the dark background.

<img src={useBaseUrl("/img/screenshots/view-styles-dark_pencil.png")} alt="The same hand-drawn cubes in white pencil on the app's dark background" />

</div>
</div>

<div className="row">
<div className="col col--6">

**Colored pencil** — each body's color laid in with the side of the lead, past the lines;
solids shadow each other.

<img src={useBaseUrl("/img/screenshots/view-styles-color_pencil.png")} alt="The same hand-drawn cubes, each in its own color" />

</div>
<div className="col col--6">

**Watercolor** — the same drawing, painted: each color a wash that pools and dries darker at
the edges.

<img src={useBaseUrl("/img/screenshots/view-styles-watercolor.png")} alt="The same hand-drawn cubes, their colors laid on as watercolor washes" />

</div>
</div>

## Projections on a drawing

A [projection](/docs/drawing-tools/projection) on a drawing page has its own **Style**, set
per view rather than per viewport. Every picture below is the same plate, from the same angle.

<div className="row">
<div className="col col--6">

**Visible edges** — hidden lines removed.

<img src={useBaseUrl("/img/screenshots/view-styles-drawing-visible.png")} alt="A projected plate and block with only the edges facing the viewer" />

</div>
<div className="col col--6">

**Wireframe** — every feature edge, including the back ones. The default.

<img src={useBaseUrl("/img/screenshots/view-styles-drawing-wireframe.png")} alt="The same projection with the hidden edges drawn as well" />

</div>
</div>

<div className="row">
<div className="col col--6">

**Shaded** — grey-shaded faces under the visible edges.

<img src={useBaseUrl("/img/screenshots/view-styles-drawing-shaded.png")} alt="The same projection with grey-shaded faces" />

</div>
<div className="col col--6">

**Colorful** — shaded, keeping each body's material color.

<img src={useBaseUrl("/img/screenshots/view-styles-drawing-colorful.png")} alt="The same projection shaded in each body's own color" />

</div>
</div>

<div className="row">
<div className="col col--6">

**Loose pencil** — the visible edges drawn by hand.

<img src={useBaseUrl("/img/screenshots/view-styles-drawing-loose_pencil.png")} alt="The same projection with hand-drawn graphite edges" />

</div>
<div className="col col--6">

**Colored pencil** — the same hand, with each body's color scribbled in and the solids' shadows on each other.

<img src={useBaseUrl("/img/screenshots/view-styles-drawing-color_pencil.png")} alt="The same hand-drawn projection with each body's color scribbled in" />

</div>
</div>

<div className="row">
<div className="col col--6">

**Watercolor** — the same hand, with each color washed on instead — pooling, and darker where it dried at the edges.

<img src={useBaseUrl("/img/screenshots/view-styles-drawing-watercolor.png")} alt="The same hand-drawn projection with its colors laid on as watercolor washes" />

</div>
<div className="col col--6">
</div>
</div>

Both pencil styles letter their captions and dimensions by hand, in Klee One. SVG exports name
the font; PDF exports keep the drawing's usual sans.

With nothing selected, the pane's **Drawing → New views** sets what the *next* projection on
the page starts as. Views already placed keep theirs.
