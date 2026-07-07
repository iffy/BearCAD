---
sidebar_position: 17
title: Drawings
---

# Technical drawings

A drawing is a black-on-white sheet of a part for printing. A document can hold as many as
you like, and each one collects **views** — a body shown from a chosen direction.

## How to use it

1. In the **Elements** pane, click **＋ New Drawing**. The drawing appears in the tree with
   its own icon and opens in the drawing pane, which takes over the central area with a white
   sheet.
2. **Add view:** pick a body and an orientation — one of the six straight-on directions
   (Front, Back, Left, Right, Top, Bottom) or **Isometric** — and click **Add**. Each view
   draws the body as a black wireframe, projected and scaled to fit its cell. Repeat to place
   several views of the same or different bodies; they lay out in a grid.
3. **Dimensions:** click an edge in any view to show its length; click it again to hide it.
   The measured length is drawn beside the edge. **Shift+click** two edges to show the angle
   between them (Shift+click either again to hide it).
4. Remove a view with the **×** beside it. Click **← Back to model** to return to the 3D
   view. Reopen a drawing any time by clicking its row (or right-click → **Edit drawing**).
5. **Export** with **Export SVG…** — a vector SVG you can open in any browser and **print to
   PDF**.

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

-- Export the drawing as a vector SVG (print it to PDF from your browser).
bearcad.export_drawing_svg{ drawing = d, path = "plate.svg" }
```

`bearcad.drawing{}` returns the drawing's index; `orientation` defaults to `"front"` and
accepts `front`/`back`/`left`/`right`/`top`/`bottom`/`iso`. `bearcad.count("drawing")` returns
how many drawings the document has.
