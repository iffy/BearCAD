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
   sketch in the Elements pane — or **drag its row** straight onto the page to place it where
   you drop it. The projection lands already selected; with the **Select** tool, drag its card
   anywhere on the page, and click any placed view later to reopen the same editor (which also
   holds **Remove view**). The editor's options:
   - **Direction:** set with an interactive **navigation bear** (like the 3D view's) — spin it
     by dragging, click a face for a straight-on view (Front, Back, Left, Right, Top, Bottom)
     or a corner for **Isometric**, or click the bear and use the numpad (4 left, 5 front,
     6 right, 8 top, 2 bottom, 0 back). The view's current direction shows as a **blue
     highlight** on the bear — a face, edge, or corner — even when it has turned to the bear's
     back. For an arbitrary angle, orbit the 3D model to the view you want, then click **Use
     this view** just below the bear.
   - **Label:** each view carries a caption ("Body 0 — Front"). The **Label** checkbox shows or
     hides it, the position grid places it in any corner or center of the card's top or bottom,
     and the text field replaces the automatic caption — `{parameter}` fields update with the
     model, and clearing the field restores the automatic caption.
   - **Scale:** type a print scale like `1:20` (one page millimetre represents twenty model
     millimetres) and the view draws at exactly that size on the page and in exports; the
     caption shows it, e.g. `Body 0 — Front (1:20)`. Any positive ratio works (`2:3`, `10:1`);
     clear the field to return to auto-fit.
   - **Style:** **Visible edges** (hidden lines removed), **Wireframe** (every edge, the
     default), or **Shaded** (grey-shaded faces under the visible edges). The editor and both
     exports render the same style.

## Text notes

A new drawing starts with its **title** already on the page as a text note in the top-left
(the drawing's name). It's an ordinary note — drag it, edit it, or delete it like any other,
and it looks the same in the editor as in the exported PDF/SVG.

Add free text to a page with the **Text** tool (or press **T** in the drawing workbench):
click where you want it for a box that grows to fit, or drag a rectangle for one that wraps
the text to that width. Switch to the **Select** tool to drag notes around the page, and edit
or remove a selected note from the context pane. **Double-click** a note to jump straight to
that editor with its text selected — just start typing to replace it. All of the page's elements — projections, notes,
and dimensions — are also listed in the **Elements pane**, nested under the drawing just like a
sketch's geometry (each projection's dimensions sit under it). Hover a row to highlight that
element on the page, and click it to select it.

Notes can embed **variables** in curly braces: `Width: {plate_w}` shows the current value of the
`plate_w` parameter, and the note updates whenever the parameter changes. Any expression works
inside the braces (`{plate_w + 3in}`), the value prints in the document's unit, and an unknown
name shows `#NA`. To print a literal brace, double it: `{{` becomes `{`. While typing a name
inside braces, a completion list of your parameters pops up — press **Tab** to accept it.

## Aligned views

The **Aligned view** tool adds a projection that stays lined up with an existing one. Pick the
**base view** to align to — if a projection is already selected when you choose the tool it's used
automatically, otherwise pick one from the tool's **Base view** picker in the context pane (or just
click a projection on the page). Then move the mouse — below, above, left, or right of the base
previews that neighbouring view — and click to drop it. The child locks to the base's shared edge:
drag it any distance away and it still lines up, and moving the base carries its aligned views
along. Each child inherits the base's scale and takes the orientation implied by its direction, so
a few clicks build a full orthographic layout around any base view. You can also **adjust an aligned
view's angle** from its context pane: the same navigation bear as any view, limited to the
orientations that keep the shared edge (for a right-of-front view: right, back, left, and the
diagonal edge views between them — never top or bottom), so you can spin the projection while it
stays lined up. Selecting an aligned view
also offers a **Projection lines** checkbox: two dashed, lightweight lines connect the outer
silhouette edges of the pair across the gap — at the far left and right for a view below or above
its base, at the top and bottom for one beside it — in the editor and in the exported PDF/SVG.

![A Top view with a Right view aligned beside it and a Front view aligned below](/img/screenshots/aligned-views.png)

## Dimensions

A new view arrives with **no dimensions shown**. Select it and click **Show all dimensions** in
the context pane to add them all at once (**Hide all dimensions** clears them). They draw as proper
dimension lines — extension lines, an offset line with arrowheads, and the measurement centred on
it — in a lighter, thinner stroke than the model edges. Round features (holes, cylinders) get a
single diameter dimension (Ø), drawn horizontally across the circle — drag its label up or down
to reposition it; a cylinder can also be dimensioned along its **length** by clicking
its side wall.

With the **Dimension tool** active, the edge under the cursor highlights — click it to show or hide
that one dimension (hover either the model edge or the dimension line itself). Circles work the
same way: hover a circle's outline — round face-on, or the line it appears as when viewed from the
side — and click to toggle its diameter. **Shift+click** two
edges to show the angle between them. Existing dimensions highlight on hover too, so it's clear
which one a click or drag will affect. **Drag a dimension's label** (with the Select or Dimension
tool) to push its line nearer or further from the edge. Coincident edges share one dimension, and
parallel dimensions that would overlap are stacked onto separate lines so no label covers another.

## Removing views and elements

Remove a view with the **×** on its card, or select any element — a projection, a text note, or a
dimension — and press **Delete** or **Backspace**. Click **Back** in the toolbar to return to the
3D model; reopen a drawing any time from its **Elements pane** row (or right-click → **Edit
drawing**), where its projections, text notes, and dimensions are listed nested underneath.

## Exporting

Use the **Export** button in the toolbar to save a single-page vector **PDF** or an **SVG** you can
open in any browser. Both are black-on-white, scale losslessly, and are WYSIWYG: the page is the
drawing's configured size with every view where you placed it. The grey card outline in the editor
is just a selection handle — it isn't printed.

**Page size and margins:** right-click the sheet background to open the page editor — set the
size in inches (Landscape/Portrait Letter presets included) and the margin. The default is a
landscape 11 × 8.5 in sheet with 0.5 in margins. Scripts set it in millimetres with
`bearcad.drawing_page{}` (below).

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

`bearcad.drawing{}` returns the drawing's index; `orientation` defaults to `"front"` and
accepts `front`/`back`/`left`/`right`/`top`/`bottom`/`iso`, or a diagonal edge view like
`front-right` or `back-left`. `bearcad.count("drawing")` returns
how many drawings the document has.
