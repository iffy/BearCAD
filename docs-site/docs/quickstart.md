---
sidebar_position: 2
title: Quickstart
---

# Quickstart: an angle bracket

Don't have BearCAD yet? [Download it here](https://github.com/iffy/BearCAD/releases/latest).

Let's design a real part: an L-shaped angle bracket with a rounded bend, rounded corners,
and two countersunk screw holes — driven by named parameters so you can resize it afterward
without redrawing anything.

![The finished angle bracket: rounded bend, rounded corners, two countersunk screw holes](/img/screenshots/quickstart.png)

You'll use most of BearCAD's core tools along the way: parameters, the Line tool, fillets,
extruding, sketching on a face, cutting holes, and rounding edges.

## 1. Set up parameters

Open the **Parameters** pane and click **+** to add each of these:

| Name | Value |
|---|---|
| `leg` | `50mm` |
| `width` | `40mm` |
| `thick` | `5mm` |
| `hole` | `5mm` |
| `bend` | `4mm` |

Any dimension field in BearCAD accepts these names — and arithmetic like `leg - thick` — in
place of a number. You can also invent a parameter on the spot by typing `name=value`
(e.g. `leg=50`) directly into any dimension field.

![The Parameters pane with the bracket's five parameters](/img/screenshots/quickstart-params.png)

## 2. Draw the L profile

Press **L** for the Line tool and click on the ground plane to start drawing. The Line tool
chains segments automatically — each click ends one segment and starts the next. Draw the
L cross-section clockwise, typing each segment's length and pressing **Enter**:

1. Right along the ground: type `leg` (50 mm).
2. Up: type `thick`.
3. Left: type `leg - thick`.
4. Up: type `leg - thick`.
5. Left: type `thick`.
6. Down: click the starting point — snapping will grab it and close the loop.

Keep each segment horizontal or vertical (the cursor snaps as you go). When the loop closes,
the L fills in — it's now a face you can extrude.

![The closed L cross-section, filled in as a face](/img/screenshots/quickstart-profile.png)

## 3. Round the bend

A real bracket bends, it doesn't fold. Press **F** for the Fillet tool:

1. Click the **inside corner** of the L, type `bend`, press **Enter**.
2. Click the **outside corner** of the L, type `bend + thick`, press **Enter**.

The two rounds are concentric, just like bent sheet metal. A live preview follows as you
drag the fillet handle, if you'd rather set it by eye.

![The L profile with both bend corners rounded](/img/screenshots/quickstart-bend.png)

## 4. Extrude it

Press **Esc** to leave the sketch, then **E** for the Extrude tool. Click the L face, type
`width`, and press **Enter**. You now have a solid bracket.

![The extruded bracket with its rounded bend](/img/screenshots/quickstart-extrude.png)

## 5. Cut the screw holes

Holes go through a flange face:

1. Press **S** for the Sketch tool and click the **outside face** of one flange. The camera
   turns to face it.
2. Press **O** for the Circle tool. Click to place a circle near the flange's tip, type
   `hole` for its diameter, press **Enter**. Place a second circle beside it.
3. To position them exactly, use the **Dimension** tool (**D**): click a circle's center and
   an edge of the face, then type the distance. Faces' own edges can be dimensioned against
   directly.
4. Press **Esc** to leave the sketch, then **E** to extrude. Click both circles, drag the
   handle **into** the bracket (or type `-thick - 1`), and pick **Cut** in the context pane.
   **Enter** — two clean holes.

Repeat on the other flange.

![Two screw holes cut through the base flange](/img/screenshots/quickstart-holes.png)

## 6. Countersink the holes

Flat-head screws want a cone-shaped seat. Press **K** for the Chamfer tool and click the
**rim** of one hole — the circle where the hole meets the face. Shift+click the other
hole's rim to add it, type `1.2`, press **Enter**. Both rims are cut into neat
countersinks.

![The screw holes with chamfered, countersunk rims](/img/screenshots/quickstart-countersink.png)

## 7. Round the corners

With no sketch open, press **F** (Fillet) and click one of the vertical edges at a flange
tip. Shift+click the other tip edges to add them to the set, type `2`, press **Enter**. The
flange corners are now rounded.

![The finished bracket with rounded flange corners](/img/screenshots/quickstart-corners.png)

## 8. Change your mind

This is the parametric payoff: open the **Parameters** pane and change `width` from `40mm`
to `60mm`. The bracket rebuilds wider — holes, fillets and all. Try `leg`, `hole`, or `bend`
too.

![The same bracket rebuilt 60 mm wide](/img/screenshots/quickstart-resize.png)

## 9. Export

**File → Export STL…** (for 3D printing) or **File → Export STEP…** (for other CAD apps).

> Every screenshot on this page is generated automatically from
> [`docs-site/screenshots/quickstart.lua`](https://github.com/iffy/BearCAD/tree/master/docs-site/screenshots/quickstart.lua),
> so they stay current as BearCAD changes.

## If something goes wrong

- **Esc** cancels whatever is in progress; pressing it again returns to the Select tool.
- **Undo** (⌘/Ctrl+Z) reverts whole steps — a fillet undoes as one unit, not line by line.
- The **status bar** (bottom of the window) explains what the active tool expects next.
