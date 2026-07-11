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
   change its orientation, set its scale, or **Remove view** there. The orientation is set with
   an interactive **navigation bear** (like the one in the 3D view): spin it by dragging, click
   a face to look from that side or a corner for the isometric view, or click the bear and
   press the numpad (4 left, 5 front, 6 right, 8 top, 2 bottom, 0 back). The view's current
   direction is shown as a **blue highlight** on the bear — a face, an edge, or a corner (for an
   isometric view) — so you can tell at a glance which way it's looking. The highlight stays
   visible even when that face has turned to the back of the bear. Tick **Free angle** to spin the
   view to any arbitrary angle instead of snapping to the standard faces/edges/corners — the
   widget then shows a small wireframe of the part itself, so you spin the actual body to the
   angle you want.
   - **Scale:** type a print scale like `1:20` (one page millimetre represents twenty model
     millimetres) and the view draws at exactly that size on the page and in exports — the
     caption shows it, e.g. `Body 0 — Front (1:20)`. Any positive ratio works (`2:3`,
     `10:1`); clear the field to return to auto-fit.
   - **Style:** choose how the projection draws — **Visible edges** (hidden lines removed),
     **Wireframe** (every edge, the default), or **Shaded** (grey-shaded faces under the
     visible edges). The editor and both exports render the same style.

## Text notes

A new drawing starts with its **title** already on the page as a text note in the top-left
(the drawing's name). It's an ordinary note — drag it, edit it, or delete it like any other,
and it looks the same in the editor as in the exported PDF/SVG.

Add free text to a page with the **Text** tool (or press **T** in the drawing workbench):
click where you want it for a box that grows to fit, or drag a rectangle for one that wraps
the text to that width. Switch to the **Select** tool to drag notes around the page, and edit
or remove a selected note from the context pane. All of the page's elements — projections, notes,
and dimensions — are also listed in the **Elements pane**, nested under the drawing just like a
sketch's geometry (each projection's dimensions sit under it). Hover a row to highlight that
element on the page, and click it to select it.

Notes can embed **variables** in curly braces: `Width: {plate_w}` shows the current value of the
`plate_w` parameter, and the note updates whenever the parameter changes. Any expression works
inside the braces (`{plate_w + 3in}`), the value prints in the document's unit, and an unknown
name shows `#NA`. To print a literal brace, double it: `{{` becomes `{`. While typing a name
inside braces, a completion list of your parameters pops up — press **Tab** to accept it.

## Aligned views

The **Aligned view** tool adds a projection that stays lined up with an existing one. Click a
placed view, then move the mouse — below, above, left, or right of it previews that neighbouring
view — and click to drop it. The child locks to the parent's shared edge: drag it any distance
away and it still lines up, and moving the parent carries its aligned views along. Each child
inherits the parent's scale and takes the orientation implied by its direction, so a few clicks
build a full orthographic layout around any base view.

![A Top view with a Right view aligned beside it and a Front view aligned below](/img/screenshots/aligned-views.png)

## Dimensions

A new view arrives with **no dimensions shown**. Select it and click **Show all dimensions** in
the context pane to add them all at once (**Hide all dimensions** clears them). They draw as proper
dimension lines — extension lines, an offset line with arrowheads, and the measurement centred on
it — in a lighter, thinner stroke than the model edges. Round features (holes, cylinders) get a
single diameter dimension (Ø); a cylinder can also be dimensioned along its **length** by clicking
its side wall.

With the **Dimension tool** active, the edge under the cursor highlights — click it to show or hide
that one dimension (hover either the model edge or the dimension line itself). **Shift+click** two
edges to show the angle between them. **Drag a dimension's label** (with the Select or Dimension
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
drawing's configured size (landscape 8.5×11 by default) with every view where you placed it. The
grey card outline in the editor is just a selection handle — it isn't printed.

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
accepts `front`/`back`/`left`/`right`/`top`/`bottom`/`iso`, or a diagonal edge view like
`front-right` or `back-left`. `bearcad.count("drawing")` returns
how many drawings the document has.
