---
sidebar_position: 18
title: Drawings
---

# <img src="/img/icons/drawing.svg" width="30" /> Technical drawings

A drawing is a black-on-white sheet of a part for printing. A document can hold as many as
you like, and each one collects **views** — a body shown from a chosen direction.

## How to use it

1. From the **CAD** menu, choose **New Drawing**. The drawing appears in the Elements tree with
   its own icon and opens in the drawing pane, which takes over the central area with a white
   sheet.
2. **Add view:** pick a body and an orientation — one of the six straight-on directions
   (Front, Back, Left, Right, Top, Bottom) or **Isometric** — and click **Add**. Each view
   draws the body as a black wireframe, projected and scaled to fit its cell. Repeat to place
   several views of the same or different bodies; they lay out in a grid.
3. **Dimensions:** a new view arrives with every edge's length dimension already shown. With
   the **Dimension tool** active, click an edge to hide its dimension (click again to bring it
   back). **Shift+click** two edges to show the angle between them (Shift+click either again
   to hide it).
4. Remove a view with the **×** beside it. Press **Esc** to return to the 3D view. Reopen a
   drawing any time by clicking its row (or right-click → **Edit drawing**).
5. **Export** with **Export PDF…** for a single-page vector PDF, or **Export SVG…** for a
   vector SVG you can open in any browser and print. Both are black-on-white and scale losslessly.

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
