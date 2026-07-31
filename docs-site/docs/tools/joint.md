---
sidebar_position: 25
title: Joint
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/joint.svg")} width="30" /> Joint

**Shortcut:** `J` — pressing it again cycles the joint type.

Joint connects two parts — bodies, components, or imported units — with a kinematic
relationship: a hinge swings, a slider slides, a screw turns and advances. The joint
poses the moving part; it never changes any shape.

![A revolute joint mid-swing](/img/screenshots/joint-kinds-revolute.png)

## How to use it

1. Pick the **Joint** tool. Parts already selected come along as the joint's parts.
2. Choose the **Type**: rigid, slider, revolute, cylindrical, planar, ball, pin-slot, or
   screw — each with its own icon in the dropdown, or press `J` again to step through them.
3. Click the part that **moves**, then the part that **holds** it. They fill the **Mobile**
   and **Fixed** slots in that order. (Rigid takes any number of parts in one list instead —
   nothing moves.)
4. Snap the mating points: **Start point A** on the moving part, **End point A** on the
   held one. The **B** pair aims the axis, the **C** pair pins the spin.
5. **Enter** (or the blue button) commits.

Skip the points to join parts right where they sit — the joint records the existing
relationship and nothing moves.

While the joint is being created or edited, the two parts are coloured by their role —
**green** for the one that moves, **blue** for the one holding it — so you can see which is
which without reading the pane. They go back to their own colours once it's committed.

The moving part also sweeps back and forth through its range, showing the motion before you
commit. **Animate** turns that sweep off — one switch for every joint.

## The kinds

<a
  href={useBaseUrl("/app/") + "?open=" + encodeURIComponent(useBaseUrl("/img/screenshots/joint-kinds-all.bearcad.json"))}
  target="_blank"
  rel="noopener noreferrer"
  title="Open this model in BearCAD"
>
  <img
    src={useBaseUrl("/img/screenshots/joint-kinds-all.png")}
    alt="All eight joint kinds, each pair posed through its joint — click to open the model in BearCAD"
  />
</a>

*Click the picture to open this model in BearCAD.*

| | Kind | Motion |
|---|---|---|
| <img src={useBaseUrl("/img/icons/joint_rigid.svg")} width="22" /> | Rigid | None — the parts move as one. Takes any number of parts. |
| <img src={useBaseUrl("/img/icons/joint_slider.svg")} width="22" /> | Slider | Slides along the axis. |
| <img src={useBaseUrl("/img/icons/joint_revolute.svg")} width="22" /> | Revolute | Turns about the axis. |
| <img src={useBaseUrl("/img/icons/joint_cylindrical.svg")} width="22" /> | Cylindrical | Slides and turns about the same axis. |
| <img src={useBaseUrl("/img/icons/joint_planar.svg")} width="22" /> | Planar | Glides across a plane and spins about its normal. |
| <img src={useBaseUrl("/img/icons/joint_ball.svg")} width="22" /> | Ball | Turns freely about all three axes. |
| <img src={useBaseUrl("/img/icons/joint_pin_slot.svg")} width="22" /> | Pin-slot | Slides along one axis while turning about another. |
| <img src={useBaseUrl("/img/icons/joint_screw.svg")} width="22" /> | Screw | Turns, advancing by the lead per full turn. |

## Base and driven

One side is the **base** — it stays put; the other moves through the joint. The Base row
swaps sides. Joints chain: a part driven by one joint can be the base of the next, and a
rigid joint with three or more parts ties them into one group that moves together.

## Positions and limits

The joint's position — slide in millimetres, turn in degrees — is an expression, so a
pose can be driven by parameters. Limits bound the travel: numbers, expressions, or a
**stop** picked as a face or plane, where the slide ends. A hinge that opens 110° one
way and not at all the other is `Turn min 0`, `Turn max 110`.

## Dragging

With the **Select** tool, drag a jointed part by any of it — a face, an edge, a corner —
and it moves through its joint, stopping at the limits. Nothing needs selecting first.
The number lands in the joint's position.

## Rest position

A joint remembers the pose it was made in. **Set** captures the current position as the
new rest; **Revert** goes back to it. Right-click a joint in the Elements pane to revert
one joint or every joint at once.

## In the Elements pane and the 3D view

Each joint shows its kind's icon — on its row, and in the 3D view at the joint's frame.
Click the 3D icon to select the joint; hover it to light up the parts it joins.
Double-click the row to edit the joint.

## Help

![The Joint tool's Context pane, each field explained](/img/screenshots/pane-joint.png)

## Scripting

```lua
-- Join two bodies with a hinge, mated corner-to-corner, swung 90°.
bearcad.joint{
  a = 0, b = 1, kind = "revolute",
  from   = { body = 1, vertex = {40, 0, 0} },  -- on the moving part
  to     = { body = 0, vertex = {0, 0, 0} },   -- on the held part
  from_b = { body = 1, vertex = {40, 0, 5} },  -- aims the axis
  to_b   = { body = 0, vertex = {0, 0, 5} },
  position = 90,
  turn_min = 0, turn_max = 110,
  name = "Hinge",
}

-- Drive the pose, capture a rest, and go back to it.
bearcad.edit_joint{ index = 0, a = 0, b = 1, kind = "revolute", position = 45 }
bearcad.set_joint_rest(0)
bearcad.revert_joint(0)     -- one joint
bearcad.revert_joints()     -- every joint

-- A rigid group, and a screw with a 2 mm lead.
bearcad.joint{ parts = {0, 1, 2}, kind = "rigid" }
bearcad.joint{ a = 0, b = 3, kind = "screw", lead = 2, position = 720 }

-- Arm the tool with picks, without committing (for live-preview shots).
bearcad.begin_joint{ a = 0, b = 1, kind = "slider" }

bearcad.ui.animate_joints(false)   -- the preview sweep, for every joint
```
