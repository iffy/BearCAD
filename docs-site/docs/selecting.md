---
sidebar_position: 5
title: Selecting things
---

# Selecting things

Every tool that needs you to point at something gathers it the same way: through an **element
picker** in the context pane. Learn it once and every tool works the way you expect.

## Pickers

A picker is the combo-box-shaped input beside a tool's label — **Bodies**, **Profile**,
**Cutters**, **Axis**. Each one takes a particular sort of thing and a particular number of
them. Empty, it shows what it's after: a count and the icons of the kinds it accepts. Filled, it
counts what it holds by kind — `2 ⟨line⟩ · 1 ⟨body⟩`. Click it to list what's in it, drop any
row with its ✕, or **Clear all**.

A single-pick input reads `0/1`, and `1/1` once it's filled.

## One picker is armed at a time

The armed picker wears a bright ring, and it's the one your next click feeds — in the viewport
or the Elements pane. Tools step through their pickers for you: pick the bodies to move, and the
tool arms the source point; pick that, and it arms the target. Click any picker to arm it
yourself.

A picker only takes what it's for. The Slice tool's **Cutters** wants planes and flat faces, so
a body under the cursor reads as its flat cap rather than as the whole solid. A Revolve axis
takes straight references only, so a circle is never offered as one. What the armed picker
turns down goes to the tool's first picker, so its main set stays reachable — pick a path for a
Repeat and you can still add more bodies to repeat.

## Picking

Click in the 3D viewport, or on a row in the Elements pane — either its name or its type icon.
Both feed the armed picker.

Hovering shows what a click would take — everything the armed picker accepts lights up, and
nothing it doesn't. Clicking a picked thing again removes it. Shift+click (or ⌘/Ctrl+click) adds
to a plain selection.

Where a picker wants **edges** and you click a **face**, you get all of that face's edges — so
chamfering the whole top of a box is one click, not four. The face lights up edge by edge on
hover, so you can see it coming.

## Edges come as whole runs

An edge picks together with every edge that runs smoothly out of it. A straight line that breaks
into a tangent curve and leaves again as a tangent line is one thing to click, not three. The run
stops at corners and wherever three edges meet. Hover shows the whole run before you commit to it.

Hold **Control** to take just the edge under the cursor.

When several things overlap under the cursor, press **Space** for the
[Selection Exploder](/docs/selection-exploder). It fans out the things the armed picker can
take, so a tool after bodies offers one handle per body and a tool after faces offers faces.

## What a picker holds is highlighted

Everything in a picker is styled as selected — in the viewport and in the Elements pane, so both
agree about what you've gathered. Things a tool will **consume** read **red** instead: a Combine
cut's second side, a Revolve's cut bodies.

## Switching tools keeps your picks

Gather three bodies with Combine, decide you meant Move, and the bodies come with you. The new
tool's first picker takes whatever it accepts and leaves the rest, so switching to a tool that
wants faces starts empty rather than carrying bodies it can't use.

## See also

- [Select](/docs/tools/select) — the tool for looking and picking.
- [Selection Exploder](/docs/selection-exploder) — reaching one thing in a crowd.
