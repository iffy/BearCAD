---
sidebar_position: 10
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

- **File → Import → BearCAD File…** — another BearCAD document becomes a reusable **unit**;
  see [Importing BearCAD files](#importing-bearcad-files).
- **File → Import → STL…** — a triangulated mesh becomes a body.
- **File → Import → STEP…** — BREP from other CAD tools, curved surfaces included,
  tessellated into a body.
- **File → Import → Image…** — a PNG/JPEG to trace over; see
  [Tracing images](/docs/tools/tracing) for scale calibration.

## Export

- **File → Export → STL…** — for 3D printing. Right-click a body row in the Elements pane to
  export just that body, or right-click a **component** to export everything inside it (and its
  nested components) as one file.
- **File → Export → STEP…** — real BREP (planar and curved surfaces) for other CAD apps.
  Right-click a body or a component row to export just that body or the whole component.
- **Technical drawings** export as vector **PDF** or **SVG** from the drawing workbench —
  see [Drawings](/docs/tools/drawing#exporting).

## Importing BearCAD files

**File → Import → BearCAD File…** brings another `.bearcad` document in as a **unit**: a
reusable part with its own parameters.

- The importing document embeds a copy, so it opens and rebuilds even with the source
  file absent.
- Importing the same file again adds a second **instance** sharing the one embedded copy.
- Each instance gets a name from the file stem (`bracket`, `bracket2`, …).
- A file under the [library directory](/docs/settings#library-directory) is remembered by
  its library path, so it resolves on any machine with the same library. Any other file is
  stored relative to the importing document — save the document once before importing.
- Importing a file that imports the current document is refused; imports can't cycle.

Each instance is **one row** in the Elements pane — rename, hide, or delete it like any
element (deleting removes just that instance). The row's triangle expands a read-only look
inside; everything in a unit is edited in its source file, not here. The node graph shows
an instance as a single node — enable **Unit contents** in the pane filter to see inside.

```lua
bearcad.import_unit("bracket.bearcad")
bearcad.import_unit{ path = "bracket.bearcad", link = "static", name = "left_bracket" }
```

`link = "dynamic"` (the default) follows later changes to the source file; `"static"`
freezes the imported copy.

## Turning a session into a script

**Help → Export Session Commands…** writes everything you've done this session as a
replayable `.lua` script — the same calls the [scripting API](/docs/scripting) uses. Running
the app with `--show-commands` echoes each GUI action as its `bearcad.*` call live.
