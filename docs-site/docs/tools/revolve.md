---
sidebar_position: 16
title: Revolve
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/revolve.svg")} width="30" /> Revolve

Revolve spins a flat profile around an axis into a solid — rings, shafts, vases, grooves.

<a
  href={useBaseUrl("/app/") + "?open=" + encodeURIComponent(useBaseUrl("/img/screenshots/revolve.bearcad.json"))}
  target="_blank"
  rel="noopener noreferrer"
  title="Open this model in BearCAD"
>
  <img src={useBaseUrl("/img/screenshots/revolve.png")} alt="A rectangular profile revolved 270 degrees into a partial ring" />
</a>

## How to use it

1. Pick the **Revolve** tool and click one or more profile faces (same sketch plane).
2. Click the **axis**: any line in the sketch — construction and projected lines work —
   or one of the origin's X/Y/Z axes.
3. Set the **Angle** (or click the label for **Revolutions** — decimals and negatives
   allowed). Drag the round handle around the arc, or type. Defaults to `360°` / `1`.
4. Optional **Offset** (coil pitch): how far the start face advances along the axis after
   one full turn — use it to wind a spring. The icon toggles to **Gap** (clear space
   between coils). Both accept negatives.
5. **Symmetric** sweeps half the angle to each side of the profile plane. Choose where the
   result lands:
   - **New body** — the revolve stands alone.
   - **Add to touching bodies** — it fuses into whatever it touches.
   - **Cut bodies** — it's subtracted from bodies you click into the **Cut bodies**
     picker.
6. **Enter** commits; **Esc** cancels.

The profile can't cross its axis; keep it entirely on one side.

## Help

![The Revolve tool's Context pane, each field explained](/img/screenshots/pane-revolve.png)

## Sketching on the result

Every **flat** face of a revolved body accepts new sketches, just like an extrusion's caps
and side walls: a partial sweep's two flat profile ends, and the washer-shaped faces swept
by any profile edge that runs perpendicular to the axis (a full 360° ring keeps those flat
ends too). Hover them with the [Sketch tool](sketch.md) and click to start drawing there.
