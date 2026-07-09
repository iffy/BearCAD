---
sidebar_position: 11
title: Revolve
---

# Revolve

Revolve spins a flat profile around an axis into a solid — rings, shafts, vases, grooves.

![A rectangular profile revolved 270 degrees into a partial ring](/img/screenshots/revolve.png)

## How to use it

1. Pick the **Revolve** tool and click one or more profile faces (they must share a
   sketch plane). Each picked face shows in the **Profile** element picker in the Context
   pane — expand it to remove one with its ✕.
2. Click the **axis** to revolve around: any line in the sketch — construction and
   projected lines work — or one of the origin's X/Y/Z axes. It shows in the **Axis**
   element picker; clear it with its ✕ to pick a different one.
3. Set the **sweep angle**: drag the handle, or type into the floating field. It defaults
   to `360` for a full solid of revolution; degrees are the default and `rad` works
   (`90`, `1.57rad`, or a parameter).
4. In the Context pane, choose **Symmetric** to sweep half the angle to each side of the
   profile plane, and choose where the result lands with the segmented icon buttons:
   - **New body** — the revolve stands alone.
   - **Add to touching bodies** — it fuses into whatever bodies it touches.
   - **Cut bodies** — it's subtracted from bodies you pick: click bodies in the viewport
     to add them to the **Cut bodies** element picker in the pane. It's the same combo-box
     picker every tool uses, here accepting only bodies and highlighting them in **red** to
     signal they'll be cut away; expand it to review the list and remove any body.
5. **Enter** commits; **Esc** cancels.

## Notes

- The profile can't cross its axis; keep it entirely on one side.
- A translucent preview of the swept solid follows the angle live.
- Cutting shows its true result live.
