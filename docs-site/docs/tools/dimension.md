---
sidebar_position: 22
title: Dimension
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/dimension.svg")} width="30" /> Dimension

**Shortcut:** `D`

Dimensions make geometry exact — and keep it that way. Click a line (or a rectangle edge,
or a circle): the dimension appears straight away and **follows your cursor**. Click again
to drop it where you want it, then type the value and press **Enter**.

![Two lines with length dimensions and the angle between them dimensioned](/img/screenshots/dimension.png)

- **Shift+click** a second line for the **angle** between them — the preview switches to the
  angle and follows the cursor the same way.
- A plain click always moves on to dimensioning whatever you clicked, so a change of mind
  costs one click. Something that's **already dimensioned** still selects — clicking it
  doesn't reopen its value; **double-click** to change that.
- While you're typing, the dimension stays drawn where you placed it — just without its
  number, which is in the field under your cursor. The **Context pane** mirrors the same
  value (**Span** for a length, **Angle** for an angle) with a blue **✓** to commit.
- Value fields accept **expressions**: `25`, `2.5in`, `leg/2 + 5`, or a new parameter
  created on the spot by typing `name=value` — see [Parameters & units](/docs/parameters).
- Dimension labels are draggable, and double-clicking one reopens it for editing.
- For angles, two crossing lines enclose two different angles — move the cursor into the
  one you mean before clicking.
- When a sketch is drawn on a body's face, you can dimension against that face's own edges
  — e.g. "this hole's center is 10 mm from the top edge."

A fully dimensioned shape draws in the
[fully-constrained color](/docs/styles#lines) and can no longer be dragged out of shape —
that's the goal: a sketch that only changes when you change a number. Until it's also
**located** (say, a corner pinned to the origin), the whole shape still drags around as
one piece, dimensions intact.

## In 3D mode

Outside a sketch, the Dimension tool **measures**. Pick what to measure — one line or body
edge for its length, two parallel lines for the distance between them, two non-parallel lines
for the angle, or two vertices (sketch points or body corners) for their distance — and
the Context pane shows the live **Value**
with a **Parameter name** box prefilled with a sensible name, ready to edit. Press the
blue **Derive parameter** button (or **Enter**): the measurement is recorded as a
[derived parameter](/docs/parameters#derived-parameters). It lands in the Parameters pane,
re-measures as the geometry changes, and works in any expression.

## Help

![The Dimension tool's Context pane, each field explained](/img/screenshots/pane-dimension.png)
