---
sidebar_position: 15
title: Tracing images
---

# Tracing images

Import a photo, scan, or datasheet drawing, tell BearCAD its real-world scale, and trace
over it with the normal sketch tools. The image is saved inside the document and behaves
like any other element.

## Importing

**File → Import Image…** places a PNG or JPEG on the ground plane. For a different plane,
right-click that construction plane in the Elements pane → **Import image on this
plane…**.

Images draw slightly translucent: bodies in front hide them, but sketch lines stay visible
on top.

## Setting the scale

Find a feature whose real size you know — a scale bar, a ruler, a printed dimension:

1. Select the image and click **Calibrate scale**.
2. Click two points at either end of the known feature — a dot under the cursor previews
   each click, and a line connects the points (**Esc** cancels).
3. The length field pre-fills with the span's current measured length; type the real
   length, **Enter**.

The image rescales so the span measures exactly that length.

The marker line stays visible whenever the image is selected, and its length stays
editable in the context pane. **Drag** either point to move it (the field re-syncs to the
measured span — apply the real length again to rescale), or **click** a point and press
**Delete** to remove it, then click to re-place it.

![A calibrated tracing image on the ground plane with sketch lines traced over the plate outline](/img/screenshots/tracing.png)

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

Sketch on the image's plane as usual and trace what you need. Because the image is
calibrated, the traced geometry is in real units — dimension it, extrude it, print it.
