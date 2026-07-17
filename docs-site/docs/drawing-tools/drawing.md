---
sidebar_position: 2
slug: /tools/drawing
title: Drawings
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/drawing.svg")} width="30" /> Technical drawings

A drawing is a black-on-white sheet for printing. A document can hold any number, each
collecting **views** — a body shown from a chosen direction.

![A drawing page with front and top views of a plate, dimensions shown](/img/screenshots/drawing.png)

**CAD → New Drawing** creates a drawing and opens the drawing pane. Its toolbar holds the
[drawing tools](/docs/drawing-tools); **Back** returns to the 3D model.

## Removing views and elements

Remove a view with the **×** on its card, or select any element and press **Delete**.
Reopen a drawing from its Elements pane row.

## Exporting

**Export** saves a vector **PDF** or **SVG**. Both are WYSIWYG at the page's configured
size.

Right-click the sheet background to set page size and margins (default landscape
11 × 8.5 in, 0.5 in margins).

## Scripting

```lua
bearcad.rect{ width = 40, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }

local d = bearcad.drawing{ name = "Plate" }
bearcad.drawing_view{ drawing = d, body = 0, orientation = "top" }
bearcad.drawing_view{ drawing = d, body = 0, orientation = "iso" }
bearcad.drawing_view{ drawing = d, sketch = 0 }  -- a sketch projects too

-- Dimension an edge of view 0 by its two world endpoints.
bearcad.drawing_dimension{ drawing = d, view = 0, a = {0, 0, 0}, b = {40, 0, 0} }

-- Toggle a circle's diameter dimension by its world centre.
bearcad.drawing_circle_dimension{ drawing = d, view = 0, center = {20, 10, 10} }

-- Edit a view's caption label: hide it, move it, or set custom text ("" = automatic).
bearcad.drawing_view_label{ drawing = d, view = 0, pos = "bottom-center", text = "Plate {w}" }

-- Show the angle between two edges of a view.
bearcad.drawing_angle{ drawing = d, view = 0,
  edge1 = { a = {0, 0, 0}, b = {40, 0, 0} },
  edge2 = { a = {0, 0, 0}, b = {0, 0, 15} } }

-- Page size and margin, in millimetres; omitted keys keep their current value.
bearcad.drawing_page{ drawing = d, width = 297, height = 210, margin = 12 }

-- Export the drawing as a vector PDF, or as an SVG.
bearcad.export_drawing_pdf{ drawing = d, path = "plate.pdf" }
bearcad.export_drawing_svg{ drawing = d, path = "plate.svg" }
```

`bearcad.drawing{}` returns the drawing's index. `orientation` defaults to `"front"`;
accepts `front`/`back`/`left`/`right`/`top`/`bottom`/`iso` or a diagonal like
`front-right`. `bearcad.count("drawing")` returns the number of drawings.
