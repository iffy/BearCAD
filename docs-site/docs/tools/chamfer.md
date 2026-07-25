---
sidebar_position: 8
title: Chamfer
---

import useBaseUrl from '@docusaurus/useBaseUrl';
import PaneCallouts from '@site/src/components/PaneCallouts';

# <img src={useBaseUrl("/img/icons/chamfer.svg")} width="30" /> Chamfer

**Shortcut:** `K`

Chamfer is [Fillet](./fillet.md)'s angular sibling: instead of rounding a corner, it cuts
it off flat. Everything else works the same way —

- **In a sketch:** click a corner where two lines meet, drag the handle or type a cut
  distance, **Enter**.

![A rectangle profile with its top-right corner chamfered flat](/img/screenshots/chamfer-sketch.png)

- **On a solid:** click an edge (Shift+click for several), set the distance, **Enter**.

![A box with its two long top edges chamfered](/img/screenshots/chamfer.png)

## The Context pane

The pane collects what to cut; the cut distance itself is typed (or dragged) in the 3D view
once something is picked.

<PaneCallouts
  src="/img/screenshots/pane-chamfer-sketch.png"
  alt="The Chamfer tool's Context pane inside a sketch"
  title="In a sketch"
  items={[
    {x: 37, y: 73, label: 'Selection', children: <>The sketch corners to cut. Click a corner where two lines meet to add it, click it again to drop it.</>},
  ]}
/>

<PaneCallouts
  src="/img/screenshots/pane-chamfer-body.png"
  alt="The Chamfer tool's Context pane on a solid"
  title="On a solid"
  items={[
    {x: 37, y: 47, label: 'Edges', children: <>The body edges to cut, one row per edge. Click an edge to add it, Shift+click for several.</>},
    {x: 37, y: 73, label: 'Length', children: <>The length unit the distance you type is read in, when you don't write one.</>},
    {x: 37, y: 86, label: 'Angle', children: <>The angle unit, likewise. Both are the document's defaults, shown whenever nothing is selected.</>},
  ]}
/>

**Countersinking screw holes** is a chamfer too: click the rim of a drilled hole, set the
distance, **Enter** — the rim is cut into a cone, ready for a flat-head screw. The
[Quickstart bracket](/docs/quickstart#7-countersink-the-holes) does exactly this.

See [Fillet](./fillet.md) for the shared details: live preview, the Context-pane edge list,
expression input, and current limitations.
