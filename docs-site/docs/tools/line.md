---
sidebar_position: 5
title: Line
---

# Line

**Shortcut:** `L`

Click to start, move the mouse, click to finish a segment — or type a length and press
**Enter**. The tool then chains: the next segment starts where the last one ended, so you
draw an outline with successive clicks. Closing back onto the starting point (snapping
grabs it) finishes the shape — and any closed outline becomes a face you can extrude.

![A capital letter "B", traced as a closed outline and extruded into a solid](/img/screenshots/letter-b.png)

*An extruded letter "B": outline drawn with the Line tool, extruded, and its two holes cut
through.*

Press **Esc** to stop chaining and keep what you've drawn.

## Snapping

While drawing, the cursor snaps to nearby endpoints, midpoints, and lines — a ring marks
the active snap, and finishing a point on a snap keeps that relationship (the sketch stays
attached there even as things move). Touching a corner also arms dashed **guide lines**
along its edges, so you can line a new point up with existing geometry at a distance.
Snapping can be toggled in the Context pane.

## Curves

Lines can bend. While drawing, tick **Curve mode** in the Context pane (or press
**⌘/Ctrl+B**) and the next point becomes a smooth curve point instead of a corner. On a
finished sketch, a curved line shows two round **handles** — drag them to reshape the
curve, or right-click a corner where two lines meet and choose **Convert to bezier curve**
to smooth it (and **Straighten curve** to undo that). Deleting a curve's handle straightens
it.

Curved lines behave like straight ones everywhere else: they close loops, extrude, and take
dimensions (a length dimension controls the straight-line distance between the endpoints).
