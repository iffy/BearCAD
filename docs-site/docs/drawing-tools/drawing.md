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

## Views

**CAD → New Drawing** creates a drawing and opens the drawing pane.

Add a view with the **Add view** tool: click a body or sketch in the Elements pane, or drag
its row onto the page. Click a placed view to reopen its editor. Options:

- **Direction:** drag the navigation bear, click a face or corner, or use the numpad
  (4 left, 5 front, 6 right, 8 top, 2 bottom, 0 back). For an arbitrary angle, orbit the
  3D model and click **Use this view**.
- **Label:** show/hide the caption, position it, or replace its text — `{parameter}`
  fields update with the model; clear the field to restore the automatic caption.
- **Scale:** type a print scale like `1:20` (page mm : model mm). Clear to auto-fit.
- **Style:** Visible edges, Wireframe (default), or Shaded.

## Text notes

A new drawing starts with its title as a text note.

With the **Text** tool (**T**), click for a box that grows to fit, or drag a rectangle for
one that wraps to that width. **Double-click** a note to edit its text.

Notes embed parameters in curly braces: `Width: {plate_w}`. Any expression works
(`{plate_w + 3in}`); `{{` prints a literal brace. Press **Tab** to accept the completion
popup.

## Aligned views

The **Aligned view** tool adds a projection that stays lined up with a base view. Pick the
base (the selected projection is used automatically), move the mouse to a side, and click
to drop. Moving the base carries its aligned views along; each child inherits the base's
scale.

The child's context pane offers the navigation bear (limited to orientations that keep the
shared edge) and a **Projection lines** checkbox connecting the pair with dashed lines.

![A Top view with a Right view aligned beside it and a Front view aligned below](/img/screenshots/aligned-views.png)

## Dimensions

New views have no dimensions. The context pane has buttons to show/hide all dimensions.

With the **Dimension tool**:

- Click edges to show/hide dimensions for a line. Circles get a diameter (Ø) dimension;
  a cylinder's side wall gets its length.
- Shift+click two lines to show the angle between them.
- Click and drag dimensions to reposition them.

## Removing views and elements

Remove a view with the **×** on its card, or select any element and press **Delete**.
**Back** returns to the 3D model; reopen a drawing from its Elements pane row.

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
