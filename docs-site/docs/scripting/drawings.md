---
sidebar_position: 2.5
title: Drawings
---

# Drawings

A [technical drawing](/docs/tools/drawing) is a printable sheet of projections. `drawing{}`
returns a **handle** (not an index). Anywhere these verbs take `drawing` or `body`, a
handle, id, name, or ordinal works.

```lua
local sides = bearcad.rect{ width = 40, height = 20 }
local box = bearcad.extrude{ profiles = sides, distance = 10 }

local d = bearcad.drawing{ name = "Plate" }
bearcad.drawing_view{ drawing = d, body = box, orientation = "top" }
bearcad.drawing_view{ drawing = d, body = box, orientation = "iso" }
bearcad.drawing_view{ drawing = d, sketch = 0 }  -- a sketch projects too
-- Several bodies (or a whole component) share one projection card.
bearcad.drawing_view{ drawing = d, bodies = {0, 1}, orientation = "front" }
bearcad.drawing_view{ drawing = d, component = 0 }
bearcad.drawing_view_add{ drawing = d, view = 0, body = 1 }  -- shift-click
```

`drawing_view` takes exactly one of `body`, `bodies`, `component`, `sketch`, or
`cross_section` (a cut of the whole model — see [cross-section views](./declarative-modeling#cross-section-views)).
`orientation` defaults to `"front"`; accepts `front`/`back`/`left`/`right`/`top`/`bottom`/`iso`
or a diagonal like `front-right`. `bearcad.count("drawing")` is the number of drawings.

## Dimensions, style, page

```lua
-- Dimension an edge of view 0 by its two world endpoints.
bearcad.drawing_dimension{ drawing = d, view = 0, a = {0, 0, 0}, b = {40, 0, 0} }
-- Offset (projected mm past the default gap) and optional snap angle (radians).
bearcad.drawing_dim_offset{ drawing = d, view = 0, a = {0, 0, 0}, b = {40, 0, 0},
  offset = 8, angle = 1.5708 }
local dim = bearcad.get{ kind = "edge_dimension", drawing = d, view = 0, index = 0 }
bearcad.drawing_circle_dimension{ drawing = d, view = 0, center = {20, 10, 10} }
-- A smooth curve (a cut edge) toggles as one length dimension.
bearcad.drawing_curve_dimension{ drawing = d, view = 0,
  points = { {0, 0, 10}, {5, 0, 14}, {12, 0, 15} } }
bearcad.drawing_angle{ drawing = d, view = 0,
  edge1 = { a = {0, 0, 0}, b = {40, 0, 0} },
  edge2 = { a = {0, 0, 0}, b = {0, 0, 15} } }

-- visible | wireframe | shaded | colorful | loose_pencil | color_pencil | watercolor
bearcad.drawing_view_style{ drawing = d, view = 0, style = "shaded" }
bearcad.drawing_paper{ drawing = d, paper = "white" }   -- "white" | "dark"
bearcad.drawing_view_orientation{ drawing = d, view = 0, orientation = "front-right-top" }
bearcad.drawing_view_label{ drawing = d, view = 0, pos = "bottom-center", text = "Plate {w}" }

-- Page size and margin, in millimetres; omitted keys keep their current value.
bearcad.drawing_page{ drawing = d, width = 297, height = 210, margin = 12 }
-- Resize a projection card (page fractions 0..1). Aligned views share the matching
-- axis: Above/Below share width, Left/Right share height.
bearcad.drawing_view_size{ drawing = d, view = 0, width = 0.3, height = 0.4 }
-- Print scale (`page:model`); omit/`nil` auto-fits. Arrows and dim-line thickness
-- stay the same page size when the scale changes.
bearcad.drawing_view_scale{ drawing = d, view = 0, scale = "2:1" }

bearcad.export_drawing_pdf{ drawing = d, path = "plate.pdf" }
bearcad.export_drawing_svg{ drawing = d, path = "plate.svg" }
```

## Zoom loupes

Ring a detail and redraw it larger ([Zoom loupe](/docs/drawing-tools/zoom-loupe)). Centres
and radii are in the view's projected millimetres.

```lua
bearcad.drawing_loupe{ drawing = d, view = 0, at = {0, 0}, radius = 8,
                       to = {10, -50}, to_radius = 20 }
bearcad.edit_drawing_loupe{ drawing = d, view = 0, index = 0, to_radius = 30 }
-- `style = "shaded"` draws the detail in that style; `"view"` follows the projection.
bearcad.edit_drawing_loupe{ drawing = d, view = 0, index = 0, style = "shaded" }
local ls = bearcad.drawing_loupes{ drawing = d, view = 0 }
-- at, radius, to, to_radius, zoom, style; each dimension has a, b, open_a, open_b
bearcad.drawing_loupe_dimension{ drawing = d, view = 0, index = 0, a = {0,0,0}, b = {40,0,0} }
bearcad.delete_drawing_loupe{ drawing = d, view = 0, index = 0 }
```

## Selecting page items

`bearcad.select` names page items — selecting one opens the drawing, like clicking it in
the Elements pane:

```lua
bearcad.select{ kind = "projection", drawing = d, view = 0 }
bearcad.select{ kind = "annotation", drawing = d, index = 0 }
-- An edge dimension is named by its two world endpoints; a point dimension by its place
-- in the view's point-dimension list.
bearcad.select{ kind = "dimension", drawing = d, view = 0, a = {0, 0, 0}, b = {40, 0, 0} }
bearcad.select{ kind = "dimension", drawing = d, view = 0, index = 0 }
bearcad.select{ kind = "drawing_loupe", drawing = d, view = 0, index = 0 }
```

`bearcad.selection()` reports page items too, each with the `drawing` it is on.
