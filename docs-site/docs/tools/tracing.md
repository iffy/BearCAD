---
sidebar_position: 14
title: Tracing images
---

# Tracing images

Have a photo, scan, or datasheet drawing of the thing you're modeling? Import it, tell
BearCAD its real-world scale, and trace over it with the normal sketch tools. The image is
saved inside your document and behaves like any other element — rename, hide, or delete it
from the Elements pane.

## Importing

**File → Import Image…** places a PNG or JPEG on the ground plane. It arrives at an
arbitrary scale — fixing that is the next step.

Images draw slightly translucent: solid bodies in front hide them, but your sketch lines
always stay visible on top.

## Setting the scale

Find a feature in the image whose real size you know — a scale bar, a ruler in the photo, a
printed dimension:

1. **Select the image.** The Context pane shows a **Calibrate scale** button.
2. **Click it, then click two points** on the image at either end of the known feature. A
   preview line follows your clicks; **Esc** cancels.
3. **Type the feature's real length** in the field that appears and press **Enter**.

The image rescales so the span you marked measures exactly that length — the feature you
clicked stays put while the rest of the image resizes around it. Calibrate again any time
to redo it.

![A calibrated tracing image on the ground plane with sketch lines traced over the plate outline](/img/screenshots/tracing.png)

## Tracing

Sketch on the image's plane as usual ([Sketch](./sketch.md), then
[Line](./line.md)/[Rectangle](./rectangle.md)/[Circle](./circle.md)) and trace what you
need. Because the image is calibrated, the traced geometry is in real units — dimension it,
extrude it, print it.

## Limitation

Calibration can't be undone with Undo yet — run **Calibrate scale** again with corrected
inputs instead.
