---
sidebar_position: 2
title: Quickstart
---

# Quickstart: an angle bracket

Don't have BearCAD yet? [Run it in your browser](https://www.iffycan.com/BearCAD/app/)
or [download it here](https://github.com/iffy/BearCAD/releases/latest).

Let's design a real part: a 120-degree angle bracket with a rounded bend, rounded corners,
and two countersunk screw holes. You'll draw it *sloppily* on purpose, then let the
constraint solver square it up — and because everything is driven by named parameters,
you can change the bend angle (or anything else) afterward without redrawing a thing.

![The finished 120-degree bracket: rounded bend, rounded corners, two countersunk screw holes](/img/screenshots/quickstart.png)

You'll use most of BearCAD's core tools along the way: parameters, the Line tool,
geometric constraints, dimensions (including an angle dimension), extruding, fillets,
sketching on a face, and cutting holes.

As you work, move around freely: **right-drag** orbits the view, **Shift+right-drag** pans,
and the mouse wheel zooms. **Zoom to fit** (in the View menu) frames the whole part. More in
[Navigation](/docs/tools/navigation).

## 1. Set up parameters

Open the **Parameters** pane and click **+** to add each of these:

| Name | Value |
|---|---|
| `leg` | `50mm` |
| `width` | `40mm` |
| `thick` | `5mm` |
| `hole` | `5mm` |
| `bend` | `4mm` |
| `bend_angle` | `120deg` |

Any dimension field in BearCAD accepts these names — and arithmetic like `leg - thick` — in
place of a number. Angle fields understand `deg` and `rad` suffixes (a bare number means
degrees). You can also invent a parameter on the spot by typing `name=value` (e.g.
`bend_angle=120`) directly into any dimension field. More in
[Parameters & units](/docs/parameters).

![The Parameters pane with the bracket's six parameters](/img/screenshots/quickstart-params.png)

## 2. Draw the profile — roughly

Press **L** for the Line tool and click on the ground plane to start drawing. The Line tool
chains segments automatically — each click ends one segment and starts the next — and
snapping grabs the starting point when you come back around to close the loop.

Draw the bracket's cross-section as a closed loop of six lines: a long base leg, a short
end cap, back along the inside of the base, up the inside of the tilted leg (aim for
roughly 120° from the base — steeper than a right angle), a short end cap at the top, and
back down the outside to the start.

**Don't try to be precise.** Wobbly lengths and crooked angles are fine — squaring it up is
the next step. When the loop closes, the profile fills in: it's now a face you can extrude.

![Six sloppy chained lines forming a rough 120-degree bracket profile](/img/screenshots/quickstart-sloppy.png)

## 3. Square it up with constraints

Now tell BearCAD what you *meant*. Press **C** for the Constraint tool, then for each
constraint select the line(s) and press the constraint's number key (or click its button in
the pane — the number is shown beside each one):

First, pin the profile down: select the **bend corner** (where the base meets the leg) and the
**origin**, then press **4** — **Coincident**. That anchors one corner to `(0, 0)` so the
sketch is located, not free to slide around. Then square up the rest:

1. Select the bottom base line, press **7** — **Horizontal**.
2. Select the bottom base line and the inner base line, press **1** — **Parallel**.
3. Select the two tilted leg lines, press **1** — **Parallel**.
4. Select the base's end cap and the bottom base line, press **2** — **Perpendicular**.
5. Select the leg's end cap and the outer leg line, press **2** — **Perpendicular**.

The sketch straightens as each constraint lands. Then give it exact sizes with the
**Dimension** tool (**D**): click a line, type the value, press **Enter**:

- both outer legs: `leg`
- both end caps: `thick`

Finally, the bend angle: select the bottom base line **and** the inner leg line, press
**D**, type `bend_angle`, and press **Enter**. The wedge marker at the corner shows which
angle you're dimensioning. The whole profile snaps to a crisp 120° bracket:

![The same profile squared up: parallel, perpendicular, exact lengths, and a 120-degree angle dimension](/img/screenshots/quickstart-squared.png)

## 4. Extrude it

Press **Esc** to leave the sketch, then **E** for the Extrude tool. Click the profile face,
type `width`, and press **Enter**. You now have a solid bracket — with a sharp bend.

![The extruded bracket, still with a sharp bend](/img/screenshots/quickstart-extrude.png)

## 5. Round the bend

A real bracket bends, it doesn't fold. With no sketch open, press **F** for the Fillet tool:

1. Click the **inside edge** of the bend (the long edge running across the part), type
   `bend`, press **Enter**.
2. Click the **outside edge** of the bend, type `bend + thick`, press **Enter**.

The two rounds are concentric, just like bent sheet metal. A live preview follows as you
drag the fillet handle, if you'd rather set it by eye.

![The bracket with inner and outer bend edges filleted](/img/screenshots/quickstart-bend.png)

## 6. Cut the screw holes

Holes go through the base flange, drilled from the **inside** face — that's where the screw
heads will sit:

1. Press **S** for the Sketch tool and click the **inside face** of the base flange. The
   camera turns to face it.
2. Press **O** for the Circle tool. Click to place a circle near the flange's tip, type
   `hole` for its diameter, press **Enter**. Place a second circle beside it.
3. To position them exactly, use the **Dimension** tool (**D**): click a circle's center and
   an edge of the face, then type the distance. Faces' own edges can be dimensioned against
   directly.
4. Press **Esc** to leave the sketch, then **E** to extrude. Click both circles, drag the
   handle **into** the bracket (or type `thick + 1`), and pick **Cut** in the context pane.
   **Enter** — two clean holes.

![Two screw holes cut through the base flange](/img/screenshots/quickstart-holes.png)

## 7. Countersink the holes

Flat-head screws want a cone-shaped seat. Press **K** for the Chamfer tool and click the
**rim** of one hole — the circle where the hole meets the inside face. Shift+click the
other hole's rim to add it, type `1.2`, press **Enter**. Both rims are cut into neat
countersinks.

![The screw holes with chamfered, countersunk rims](/img/screenshots/quickstart-countersink.png)

## 8. Round the corners

Press **F** (Fillet) and click one of the vertical edges at a flange tip. Shift+click the
other tip edges to add them to the set, type `2`, press **Enter**. The flange corners are
now rounded.

![The finished bracket with rounded flange corners](/img/screenshots/quickstart-corners.png)

## 9. Engrave a label

Let's brand it. Press **T** for the Text tool and click the **outer face** of the base flange —
the flat wall on the back of the two screw holes. Type `BearCAD`, and position it so it's centered
on the face (drag it with the Select tool, or nudge the size until it fits). Then press **E** for
Extrude, click the text (the whole word selects as one), and push the handle **into** the face
(type `1` for a 1 mm depth) and pick **Cut** — the letters are engraved into the bracket, letter
counters and all. Orbit around to the front of that face and you can read it:

![The bracket turned around to show "BearCAD" engraved on the outer face of the base](/img/screenshots/quickstart-engrave.png)

## 10. Change your mind

This is the parametric payoff: open the **Parameters** pane and change `bend_angle` from
`120deg` to `150deg`. The whole part rebuilds at the new angle — bend, holes, countersinks,
corner rounds and all. Try `leg`, `width`, or `hole` too.

![The same bracket rebuilt with a 150-degree bend](/img/screenshots/quickstart-angle.png)

## 11. Export

**File → Export → STL…** (for 3D printing) or **File → Export → STEP…** (for other CAD apps) —
or use the **Export** button in the toolbar.

> Every screenshot on this page is generated automatically from
> [`docs-site/screenshots/quickstart.lua`](https://github.com/iffy/BearCAD/tree/master/docs-site/screenshots/quickstart.lua),
> so they stay current as BearCAD changes.

## If something goes wrong

- **Esc** cancels whatever is in progress; pressing it again returns to the Select tool.
- **Undo** (⌘/Ctrl+Z) reverts whole steps — a fillet undoes as one unit, not line by line.
- The **status bar** (bottom of the window) explains what the active tool expects next.
- If the solver flags a **conflict** (red), you've asked for contradictory things — delete
  the most recent constraint or dimension and try a different one.
