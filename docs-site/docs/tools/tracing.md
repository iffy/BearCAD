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
2. Click two points at either end of the known feature (**Esc** cancels).
3. Type the feature's real length, **Enter**.

The image rescales so the span measures exactly that length. Calibrate again any time.

![A calibrated tracing image on the ground plane with sketch lines traced over the plate outline](/img/screenshots/tracing.png)

## Tracing

Sketch on the image's plane as usual and trace what you need. Because the image is
calibrated, the traced geometry is in real units — dimension it, extrude it, print it.
