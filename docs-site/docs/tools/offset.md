---
sidebar_position: 10
title: Offset
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/offset.svg")} width="30" /> Offset

Offset makes a parallel copy of sketch edges a constant distance away — wall
thicknesses, clearances, insets.

<a
  href={useBaseUrl("/app/") + "?open=" + encodeURIComponent(useBaseUrl("/img/screenshots/offset.bearcad.json"))}
  target="_blank"
  rel="noopener noreferrer"
  title="Open this model in BearCAD"
>
  <img src={useBaseUrl("/img/screenshots/offset.png")} alt="A rectangle offset outward and a circle offset inward as construction" />
</a>

## How to use it

1. Pick the **Offset** tool. Outside a sketch, click a face to sketch on it first.
2. Click lines and circles to add them to the offset (click again to remove). Click a
   **face** — a closed sketch profile, or a body face — to take all of its edges at once.
3. Click the **push-pull handle** to grab it (click again to drop), or type a
   distance. A typed value sticks. **Positive grows** a closed loop or circle;
   negative shrinks — or flips an open chain's side.
4. Press **Enter**.

Lines that connect end-to-end offset as one chain with mitered corners; a circle's
radius grows or shrinks. The copies land under an **Offset** element beneath the sketch
in the Elements pane and follow their sources as the sketch changes — the distance is an
expression, so parameters work. **Edit offset** on the element changes anything later.

Toggle **Construction** (or press **X**) to emit the copies as construction geometry — e.g.
a guide line a fixed clearance from a wall. The preview goes dashed to match.

## Help

![The Offset tool's Context pane, each field explained](/img/screenshots/pane-offset.png)

See [Scripting](/docs/scripting).
