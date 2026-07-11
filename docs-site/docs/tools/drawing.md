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
   visible even when that face has turned to the back of the bear.
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

To place a second view of a body that stays lined up with the first, use the **Aligned view**
tool. Click an existing projection, then move the mouse: below it previews the bottom view,
above it the top, to the sides the left/right views — all constrained to line up with the
parent's edges. Click to drop it. You can drag an aligned view any distance away, but it
always stays aligned along the shared edge; move the parent and its aligned views follow.
Aligned views inherit the parent's scale, so a whole group reads as one set. A placed aligned
view's orientation can still be changed from its context pane — the chooser offers just the views
that stay lined up with the base (for a side view off a front view, that's front, back, left, or
right, plus the diagonal edge views in between — front-right, back-left, and so on).
3. **Dimensions:** a new view arrives with **no dimensions shown**. Select the projection and
   use the **Show all dimensions** button in the context pane to add them all at once (or **Hide
   all dimensions** to clear them). They're drawn
   as proper dimension lines — extension lines, an offset line with arrowheads, and the
   measurement centred on it, all drawn with a lighter, thinner stroke than the model edges so
   the part outline stays the eye's focus — the label runs along the dimension line (or sits just
   past its end when the line is too short to fit). Round features (holes, cylinders) render as a single
   smooth circle with one diameter dimension (Ø), not a ring of little segments. Round and other
   smooth extrusions can still be dimensioned along their **length** — click their straight side
   (the cylinder wall) with the Dimension tool, or use Show all dimensions. The set stays
   readable: coincident edges (a box's bottom edge, front and back, land on
   the same line) share one dimension, and parallel dimensions that would otherwise sit on top of
   each other are stacked onto separate lines so no measurement label overlaps another. With the **Dimension tool** active, the edge under the cursor
   highlights; click it to show or hide that one dimension. To turn a dimension off you can hover
   either the model edge **or the dimension line itself**. **Shift+click**
   two edges to show the angle between them (Shift+click either again to hide it). **Drag a
   dimension's label** (with the Select or Dimension tool) to push its line further from or
   closer to the edge — with the Select tool, hovering a dimension highlights its line and label
   so you can see which one you're about to move.
4. Remove a view with the **×** beside it, or select any element — a projection, a text note, or
   a dimension (click it with the Select tool) — and press **Delete** or **Backspace** to remove
   it. Click **Back** in the toolbar to return to the 3D
   model. Reopen a
   drawing any time by clicking its row (or right-click → **Edit drawing**). While a drawing is
   open, the **Elements pane** lists it with its projections, text notes, and dimensions nested
   underneath (each projection's dimensions under it), so you can see everything that's on the page
   — click any of those rows to jump into the drawing and select that element.
5. **Export** with **Export PDF…** for a single-page vector PDF, or **Export SVG…** for a
   vector SVG you can open in any browser and print. Both are black-on-white, scale
   losslessly, and are WYSIWYG: the page is the drawing's configured page size (landscape
   8.5×11 by default) with every view exactly where you placed it. The grey card outline you
   see around each view in the editor is just a selection handle — it isn't printed.

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
