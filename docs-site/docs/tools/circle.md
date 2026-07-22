---
sidebar_position: 6
title: Circle
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/circle.svg")} width="30" /> Circle

**Shortcut:** `O`

Click to place the center, move the mouse to size it, then click — or type a **diameter**
and press **Enter**.

**Anchor mode.** The context pane has a two-icon toggle for how the two clicks define the
circle: **centre + radius** (the default — first click is the centre, drag out the radius) or
**edge to opposite edge** (first click pins one point on the rim, then click the
diametrically opposite point — the two clicks span a diameter). Press **O** again while the
tool is already active to toggle between the two. You can still type a **diameter** to
constrain it in either mode.

![A plate with a construction bolt circle and a dimensioned bolt hole](/img/screenshots/circle.png)

A circle is a face: extrude it to get a cylinder, or extrude it **into** a body as a
[cut](./extrude.md#adding-to-or-cutting-a-body) to drill a round hole.

- **Esc** cancels the in-progress circle.
- **X** makes it construction (reference) geometry — handy for bolt-circle layouts.
