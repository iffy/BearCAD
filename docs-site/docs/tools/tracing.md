---
sidebar_position: 25
title: Tracing images
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# Tracing images

Import a photo, scan, or datasheet drawing, tell BearCAD its real-world scale, and trace
over it with the normal sketch tools. The image is saved inside the document and behaves
like any other element.

## Importing

**File → Import Image…** places a PNG or JPEG on the ground plane. For a different plane,
select it and run **Import image on this plane…** from the command palette, or
right-click it in the Elements pane. Import selects the new image so you can
calibrate it right away.

Images draw slightly translucent (0.9 by default): bodies in front hide them, but sketch
lines stay visible on top. Select an image to change its opacity with the slider.

## Setting the scale

While the image is selected, a line appears from the top-middle of the image
to the bottom-middle, with a dimension that cannot be removed.

Drag the endpoints onto a feature whose real size you know. Double-click the dimension
(or type the length in the context pane) and enter the real length — any expression.
The image rescales so those two points stay on the same spots of the image and the
line measures that length.

<a
  href={useBaseUrl("/app/") + "?open=" + encodeURIComponent(useBaseUrl("/img/screenshots/tracing.bearcad.json"))}
  target="_blank"
  rel="noopener noreferrer"
  title="Open this model in BearCAD"
>
  <img src={useBaseUrl("/img/screenshots/tracing.png")} alt="A calibrated tracing image on the ground plane with sketch lines traced over the plate outline" />
</a>

## Moving and constraining

The [Move](/docs/tools/move) tool moves an image: click its quad in the viewport (or its
Elements pane row) to pick it, then set the translation.

A calibrated image's two reference points are regular sketch points: with the
[Constraint](/docs/tools/constraint) tool in a sketch on the image's plane, hold one
coincident to a vertex, a line, or the origin/axes — the whole image translates to
follow (scale never changes). From scripts:

```lua
bearcad.select{ kind = "image", index = 0, point = 0 }   -- calibration point 0 or 1
bearcad.select({ kind = "line", index = 2, ["end"] = "start" }, true)
bearcad.add_geometric_constraint("coincident")
```

## Tracing

Click the image with Sketch, Line, Rectangle, or Circle to start a sketch on its plane
and trace what you need. Because the image is calibrated, the traced geometry is in
real units — dimension it, extrude it, print it.

```lua
bearcad.import_image("plate.png")
bearcad.begin_sketch{ kind = "image", index = 0 }
bearcad.line{ x1 = -10, y1 = 0, x2 = 10, y2 = 0 }
```

## Help

![A tracing image's Context pane, each field explained](/img/screenshots/pane-tracing.png)
