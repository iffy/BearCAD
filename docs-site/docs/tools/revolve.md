---
sidebar_position: 11
title: Revolve
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/revolve.svg")} width="30" /> Revolve

Revolve spins a flat profile around an axis into a solid — rings, shafts, vases, grooves.

![A rectangular profile revolved 270 degrees into a partial ring](/img/screenshots/revolve.png)

## How to use it

1. Pick the **Revolve** tool and click one or more profile faces (same sketch plane).
2. Click the **axis**: any line in the sketch — construction and projected lines work —
   or one of the origin's X/Y/Z axes.
3. Set the **sweep angle**: drag the round handle around the arc, or type into the field.
   Defaults to `360`; degrees by default, `rad` and parameters work.
4. **Symmetric** sweeps half the angle to each side of the profile plane. Choose where the
   result lands:
   - **New body** — the revolve stands alone.
   - **Add to touching bodies** — it fuses into whatever it touches.
   - **Cut bodies** — it's subtracted from bodies you click into the **Cut bodies**
     picker.
5. **Enter** commits; **Esc** cancels.

The profile can't cross its axis; keep it entirely on one side.
