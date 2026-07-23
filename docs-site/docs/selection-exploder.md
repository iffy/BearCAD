---
sidebar_position: 7
title: Selection Exploder
---

# Selection Exploder

When several pickable things pile up under your cursor — overlapping vertices, an edge
running through a face, a circle centre sitting on a corner — clicking the exact one you
want is a guessing game. The **Selection Exploder** fans that crowd out into spaced-apart
handles so you can pick the right one with a single, unambiguous click.

Park your cursor over the cluster and press **Space**:

![The Selection Exploder fanned open over a crowded corner: each stacked thing gets its own round loupe — a 2× magnified view of the pick region with that one element highlighted — joined by a thin leader line back to where it really is](/img/screenshots/exploder.png)

Everything inside your cursor's pick radius pops out to its own handle, arranged in a ring
and joined by a thin leader line back to where it really lives. Each handle is a little round
**loupe**: a magnified view of the pick region, with the one element that handle stands for
drawn on top (in **blue**) and the rest of the crowd dimmed grey behind it. Coincident line
ends are told apart by a short stub pointing the way each line runs; faces are shaded, not
just outlined. So even when several things overlap almost perfectly, you can see exactly which
one each handle will pick.

## Picking from the fan

- **Hover** a handle and both it and its real thing in the scene light up **yellow** — the
  whole line, edge, or face — so you can confirm you've got the right one before committing.
- **Click** the handle to select it. A selected thing's loupe stays yellow.
- Hold **Shift** while you click to keep the fan open and gather several things in a row.
- **Scroll** the mouse wheel to zoom the loupes in for a closer look; they grow to fill the
  space and stop once they reach the edges.
- Press **Space** again, press **Esc**, or click empty space to dismiss the fan.

While the fan is open the **camera holds still**, so the handles stay put under your cursor
instead of drifting as you reach for one.

## Big crowds fan out in groups

When a lot of things stack up, the fan keeps itself readable by **grouping** related handles.
A group shows as one loupe with a count badge:

- **Click a group** to drill into it — its members spring out into their own loupes while the
  other groups gather into a small **cluster loupe** off to the side.
- **Click the cluster loupe** to go back up a level; the members you were looking at gather
  back into their group as the cluster fans its siblings back out.

The hand-off between levels is animated, so you can always see where a group came from and
where it went — drill in and back out as many levels as you need.

## Any time, any tool, anything

Press **Space** whenever you like — you don't have to wait for a crowd:

- Over a cluster it fans out several handles.
- Over a single thing it gives you just one handle.
- Over empty space it simply parks the hitbox circle there.

A faint circle appears under the cursor whenever two or more things are stacked, hinting
that exploding will help.

The exploder works with **every tool**, not only Select, and with **every kind** of pickable
thing — sketch vertices, lines, and circles, body vertices, edges, and faces, and even the
**constraint badges** stacked over your geometry. Whatever handle you pick is fed straight to
the tool you're using, so you can extrude a buried face, fillet a hidden edge, or combine an
overlapping body without fighting the pick.

A constraint badge's loupe shows that constraint's icon, and hovering it lights the real badge
up in the drawing — so when several constraint icons pile onto the same corner, you can fan
them out and click the exact one you mean to select.

## See also

- [Select](/docs/tools/select) — the default tool for looking and picking.
- [Navigation](/docs/tools/navigation) — camera, views, and the command palette.
