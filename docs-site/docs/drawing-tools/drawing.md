---
sidebar_position: 2
slug: /tools/drawing
title: Drawings
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/drawing.svg")} width="30" /> Technical drawings

A drawing is a black-on-white sheet for printing. A document can hold any number, each
collecting **views** — a body, several bodies, or a whole component shown from a chosen
direction.

![The PDF export of a technical drawing: four views of a cut cylinder, the cut faces hatched, one length dimension](/img/drawing-pdf.png)

**CAD → New Drawing** creates a drawing and opens the drawing pane. Right-click a **body**
in the Elements pane → **Create drawing** makes a drawing of that body in one step. The
toolbar holds the [drawing tools](/docs/drawing-tools); **Back** returns to the 3D model.

## Removing views and elements

Remove a view with the **×** on its card, right-click → **Remove**, or select any element
and press **Delete**. Right-click an orthographic card → **Create aligned view** arms
the Aligned-view tool with that card as the base. Reopen a drawing from its Elements
pane row.

## Resizing cards

With **Select**, a selected projection shows corner grips. Drag a corner to resize the
card (centre stays put). Aligned partners share the matching axis — Above/Below share
width, Left/Right share height — so a resize on any of them updates the linked dimension.

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
-- Several bodies (or a whole component) share one projection card.
bearcad.drawing_view{ drawing = d, bodies = {0, 1}, orientation = "front" }
bearcad.drawing_view{ drawing = d, component = 0 }
bearcad.drawing_view_add{ drawing = d, view = 0, body = 1 }  -- shift-click

-- Dimension an edge of view 0 by its two world endpoints.
bearcad.drawing_dimension{ drawing = d, view = 0, a = {0, 0, 0}, b = {40, 0, 0} }

-- Toggle a circle's diameter dimension by its world centre.
bearcad.drawing_circle_dimension{ drawing = d, view = 0, center = {20, 10, 10} }

-- Display style and which way a placed view faces.
bearcad.drawing_view_style{ drawing = d, view = 0, style = "shaded" }  -- visible/wireframe/shaded
bearcad.drawing_view_orientation{ drawing = d, view = 0, orientation = "front-right-top" }

-- Edit a view's caption label: hide it, move it, or set custom text ("" = automatic).
bearcad.drawing_view_label{ drawing = d, view = 0, pos = "bottom-center", text = "Plate {w}" }

-- Show the angle between two edges of a view.
bearcad.drawing_angle{ drawing = d, view = 0,
  edge1 = { a = {0, 0, 0}, b = {40, 0, 0} },
  edge2 = { a = {0, 0, 0}, b = {0, 0, 15} } }

-- Page size and margin, in millimetres; omitted keys keep their current value.
bearcad.drawing_page{ drawing = d, width = 297, height = 210, margin = 12 }

-- Resize a projection card (page fractions 0..1). Aligned views share the matching
-- axis: Above/Below share width, Left/Right share height. Omitted keys keep the value.
bearcad.drawing_view_size{ drawing = d, view = 0, width = 0.3, height = 0.4 }

-- Export the drawing as a vector PDF, or as an SVG.
bearcad.export_drawing_pdf{ drawing = d, path = "plate.pdf" }
bearcad.export_drawing_svg{ drawing = d, path = "plate.svg" }
```

`bearcad.drawing{}` returns the drawing's index. `drawing_view` takes exactly one of
`body`, `bodies`, `component`, or `sketch`. `orientation` defaults to `"front"`;
accepts `front`/`back`/`left`/`right`/`top`/`bottom`/`iso` or a diagonal like
`front-right`. `bearcad.count("drawing")` returns the number of drawings.

`bearcad.select` also names page items — selecting one opens the drawing and puts it in
the page selection, like clicking it in the Elements pane:

```lua
bearcad.select{ kind = "projection", drawing = d, view = 0 }
bearcad.select{ kind = "annotation", drawing = d, index = 0 }
-- An edge dimension is named by its two world endpoints; a point dimension by its place
-- in the view's point-dimension list.
bearcad.select{ kind = "dimension", drawing = d, view = 0, a = {0, 0, 0}, b = {40, 0, 0} }
bearcad.select{ kind = "dimension", drawing = d, view = 0, index = 0 }
```
