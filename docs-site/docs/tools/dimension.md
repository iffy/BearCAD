---
sidebar_position: 12
title: Dimension
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/dimension.svg")} width="30" /> Dimension

**Shortcut:** `D`

Dimensions make geometry exact — and keep it that way. Click a line (or a rectangle edge,
or a circle) to set its length or diameter; select two lines that cross and press **D** for
the angle between them. Type the value, press **Enter**.

![Two lines with length dimensions and the angle between them dimensioned](/img/screenshots/dimension.png)

- Value fields accept **expressions**: `25`, `2.5in`, `leg/2 + 5`, or a new parameter
  created on the spot by typing `name=value` — see [Parameters & units](/docs/parameters).
- Dimension labels are draggable, and double-clicking one reopens it for editing.
- For angles, two crossing lines enclose two different angles — move the cursor into the
  one you mean before clicking.
- When a sketch is drawn on a body's face, you can dimension against that face's own edges
  — e.g. "this hole's center is 10 mm from the top edge."

A fully dimensioned shape draws in the
[fully-constrained color](/docs/styles#lines) and can no longer be dragged out of shape —
that's the goal: a sketch that only changes when you change a number.

## In 3D mode

Outside a sketch, the Dimension tool **measures**: click a line to capture its length as a
[derived parameter](/docs/parameters#derived-parameters); Shift+click two points or two
lines to capture the distance (or angle) between them. The parameter lands in the
Parameters pane, re-measures as the geometry changes, and works in any expression.
