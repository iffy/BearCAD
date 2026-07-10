---
sidebar_position: 18
title: Drawings
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/drawing.svg")} width="30" /> Technical drawings

A drawing is a black-on-white sheet of a part for printing. A document can hold as many as
you like, and each one collects **views** — a body shown from a chosen direction.

![A drawing page with front and top views of a plate, dimensions shown](/img/screenshots/drawing.png)

## How to use it

1. From the **CAD** menu, choose **New Drawing**. The drawing appears in the Elements tree with
   its own icon and opens in the drawing pane, which takes over the central area with a white
   sheet.
2. **Add view:** pick the **Add view** tool (the ＋ in the toolbar), then click a body or
   sketch in the Elements pane — a projection of it lands on the page, already selected. You
   can also **drag a body or sketch row** from the Elements pane straight onto the page to
   place it exactly where you drop it. Set
   its direction in the context pane: one of the six straight-on directions (Front, Back,
   Left, Right, Top, Bottom) or **Isometric**. Each view draws the source as a black
   wireframe, scaled to fit its card. Drag the card wherever it should sit on the page, and
   repeat for more views. Clicking any placed view selects it and reopens the same editor —
   change its orientation, set its scale, or **Remove view** there.
   - **Scale:** type a print scale like `1:20` (one page millimetre represents twenty model
     millimetres) and the view draws at exactly that size on the page and in exports — the
     caption shows it, e.g. `Body 0 — Front (1:20)`. Any positive ratio works (`2:3`,
     `10:1`); clear the field to return to auto-fit.
   - **Style:** choose how the projection draws — **Visible edges** (hidden lines removed),
     **Wireframe** (every edge, the default), or **Shaded** (grey-shaded faces under the
     visible edges). The editor and both exports render the same style.

## Text notes

Add free text to a page with the **Text** tool (or press **T** in the drawing workbench):
click where you want it for a box that grows to fit, or drag a rectangle for one that wraps
the text to that width. Switch to the **Select** tool to drag notes around the page, and edit
or remove a selected note from the context pane.

## Aligned views

To place a second view of a body that stays lined up with the first, use the **Aligned view**
tool. Click an existing projection, then move the mouse: below it previews the bottom view,
above it the top, to the sides the left/right views — all constrained to line up with the
parent's edges. Click to drop it. You can drag an aligned view any distance away, but it
always stays aligned along the shared edge; move the parent and its aligned views follow.
Aligned views inherit the parent's scale and orientation, so a whole group reads as one set.
3. **Dimensions:** a new view arrives with every edge's length dimension already shown, drawn
   as proper dimension lines — extension lines, an offset line with arrowheads, and the
   measurement centred on it — the label runs along the dimension line (or sits just past its
   end when the line is too short to fit). Round features (holes, cylinders) render as a single
   smooth circle with one diameter dimension (Ø), not a ring of little segments. With the **Dimension tool** active, the edge under the cursor
   highlights; click it to hide its dimension (click again to bring it back). **Shift+click**
   two edges to show the angle between them (Shift+click either again to hide it). **Drag a
   dimension's label** (with the Select or Dimension tool) to push its line further from or
   closer to the edge.
4. Remove a view with the **×** beside it. Press **Esc** to return to the 3D view. Reopen a
   drawing any time by clicking its row (or right-click → **Edit drawing**).
5. **Export** with **Export PDF…** for a single-page vector PDF, or **Export SVG…** for a
   vector SVG you can open in any browser and print. Both are black-on-white, scale
   losslessly, and are WYSIWYG: the page is the drawing's configured page size (landscape
   8.5×11 by default) with every view exactly where you placed it.

## Scripting

```lua
bearcad.rect{ width = 40, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }

local d = bearcad.drawing{ name = "Plate" }
bearcad.drawing_view{ drawing = d, body = 0, orientation = "top" }
bearcad.drawing_view{ drawing = d, body = 0, orientation = "iso" }

-- Dimension an edge of view 0 by its two world endpoints.
bearcad.drawing_dimension{ drawing = d, view = 0, a = {0, 0, 0}, b = {40, 0, 0} }

-- Show the angle between two edges of a view.
bearcad.drawing_angle{ drawing = d, view = 0,
  edge1 = { a = {0, 0, 0}, b = {40, 0, 0} },
  edge2 = { a = {0, 0, 0}, b = {0, 0, 15} } }

-- Export the drawing as a vector PDF, or as an SVG.
bearcad.export_drawing_pdf{ drawing = d, path = "plate.pdf" }
bearcad.export_drawing_svg{ drawing = d, path = "plate.svg" }
```

`bearcad.drawing{}` returns the drawing's index; `orientation` defaults to `"front"` and
accepts `front`/`back`/`left`/`right`/`top`/`bottom`/`iso`. `bearcad.count("drawing")` returns
how many drawings the document has.
