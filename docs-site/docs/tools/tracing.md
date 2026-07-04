---
sidebar_position: 14
title: Tracing images
---

# Tracing images

Import a reference image — a photo, a scanned drawing, a datasheet figure — onto a
construction plane, set its real-world scale from a feature of known size, and trace over it
with the normal sketch tools. The image is embedded in the document (saved files stay
self-contained) and lives in the Elements pane like any other element: it can be renamed,
hidden, or deleted, and it nests under its host plane.

## Importing an image

**File → Import Image…** opens a file dialog for a PNG or JPEG. The image lands on the
**ground plane**, centered on the plane origin, seeded at **1 px = 1 mm** — almost never the
real scale, which is what calibration is for.

Images render as a translucent textured quad on their host plane: bodies in front occlude
them, but sketch geometry always reads on top, so traced lines stay visible over the picture.

## Calibrating the scale

Pick any feature of the image whose real size you know — a printed scale bar, a ruler in the
photo, a dimension label on the drawing:

1. **Select the image** (click it in the viewport or in the Elements pane). The context pane
   shows a **Calibrate scale** button.
2. **Click the button**, then **click two points** on the image, at either end of the
   known feature. The placed points and the span between them are previewed live, with a
   rubber band following the cursor to the second point. **Esc** cancels.
3. With both points placed, the context pane shows the length field: **type the feature's
   real length** (any length expression works — `50`, `2.5in`, `width/2`) and press
   **Enter** or click **Apply**.

The image rescales uniformly about the marked span's midpoint so that span measures exactly
the typed length — the feature you clicked stays put while the rest of the image grows or
shrinks around it. The calibration is stored on the image, and running **Calibrate scale**
again replaces it.

As an alternative, you can draw a **line** over the known feature with the Line tool, then
select the image *and* that line together — the same length field appears, using the line as
the reference span.

![A calibrated tracing image on the ground plane with sketch lines traced over the plate outline](/img/screenshots/tracing.png)

> This image is auto-generated from
> [`docs-site/screenshots/tracing.lua`](https://github.com/iffy/BearCAD/tree/master/docs-site/screenshots/tracing.lua).
> See [Auto-generated screenshots](/docs/scripting/screenshots).

## Tracing

Once calibrated, sketch on the image's plane as usual ([Sketch](./sketch.md), then
[Line](./line.md)/[Rectangle](./rectangle.md)/[Circle](./circle.md)) and trace the shapes you
need; measurements taken off the traced geometry are then in real units. Extrude the traced
profiles like any other sketch geometry.

## Scripting

```lua
bearcad.import_image{ path = "drawing.png" }          -- ground plane
bearcad.import_image{ path = "drawing.png", plane = 1 }

-- Reference span in plane-local mm (at the image's current scale) + its real length:
bearcad.calibrate_image{ image = 0, from = { -100, -120 }, to = { 100, -120 }, length = 50 }
```

See [Declarative modeling](/docs/scripting/declarative-modeling#import) for the rest of the
scripting API.

## Known limitation

Calibration mutates the image's placement in place and is not yet individually undoable —
re-run **Calibrate scale** with corrected inputs to fix a wrong calibration.
