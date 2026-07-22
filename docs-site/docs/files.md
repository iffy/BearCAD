---
sidebar_position: 9
title: Files, import & export
---

# Files, import & export

## Documents

**File → Save / Open** work with `.bearcad` files. A document is self-contained: embedded
fonts ([Text](/docs/tools/text)) and [tracing images](/docs/tools/tracing) travel inside it,
so it opens identically on any machine. In the browser app, saving downloads the file and
opening picks one from disk.

**Undo** (⌘/Ctrl+Z) reverts whole steps — a fillet or a boolean undoes as one unit.

The window title shows the current file name and a leading **`*`** whenever there are
unsaved changes; the `*` clears once you save (or if you undo all the way back to the last
saved state). Quitting with unsaved changes asks whether to **Save**, **Don't Save**, or
**Cancel** so a stray ⌘Q doesn't lose work.

## Import

- **File → Import → STL…** — a triangulated mesh becomes a body.
- **File → Import → STEP…** — BREP from other CAD tools, curved surfaces included,
  tessellated into a body.
- **File → Import → Image…** — a PNG/JPEG to trace over; see
  [Tracing images](/docs/tools/tracing) for scale calibration.

## Export

- **File → Export → STL…** — for 3D printing. Right-click a body row in the Elements pane to
  export just that body.
- **File → Export → STEP…** — real BREP (planar and curved surfaces) for other CAD apps.
- **Technical drawings** export as vector **PDF** or **SVG** from the drawing workbench —
  see [Drawings](/docs/tools/drawing#exporting).

## Turning a session into a script

**Help → Export Session Commands…** writes everything you've done this session as a
replayable `.lua` script — the same calls the [scripting API](/docs/scripting) uses. Running
the app with `--show-commands` echoes each GUI action as its `bearcad.*` call live.
