---
sidebar_position: 3
title: Solids and operations
---

# Solids and operations

Shapes, booleans, patterns, and mates. Sketch and extrude live on
[Declarative modeling](./declarative-modeling).

## Shapes

No sketch. `at` is the base centre; the solid grows along `normal` (default +Z).

```lua
local block = bearcad.cuboid{ width = 40, depth = 20, height = 10, name = "Block" }
bearcad.cylinder{ at = {100, 0, 0}, radius = 5, height = 20 }
bearcad.sphere{ at = {200, 0, 0}, radius = 8 }
bearcad.edit_shape{ index = block, height = "side * 3" }
```

## Combine

`op` is `"union"` | `"cut"` | `"intersect"` | `"xor"`. `keep_b` keeps what the op would discard.

```lua
bearcad.combine{ op = "cut", a = {a}, b = {b}, name = "Notched" }
bearcad.combine{ op = "union", a = {a, b, c} }
bearcad.edit_combine{ index = 0, op = "xor", a = {a}, b = {b} }
```

## Sweep and loft

`profiles` is a circle handle, a line loop, or a list of those. Sweep `path` is lines chained tip-to-tail. `body = "add"` | `"cut"`; omit for a new body.

```lua
bearcad.sweep{ profiles = profile, path = {up, over}, body = "cut", bodies = {box} }
bearcad.loft{ profiles = {base, tip}, name = "Horn" }
bearcad.edit_sweep{ index = 0, path = {up} }
bearcad.edit_loft{ index = 0 }
```

## Shell

```lua
bearcad.shell{ bodies = {box}, faces = {box:face("top")}, thickness = 1 }
bearcad.shell{ bodies = {box}, thickness = 2 }   -- closed hollow
bearcad.edit_shell{ index = 0, faces = {}, thickness = 1.5 }
```

## Slice

A planar cutter is a face-spec (`kind = "plane"`, or a body face). A laser cutter is `{ kind = "line", index = path }`. `extend = false` keeps the cut finite.

```lua
bearcad.slice{ bodies = {box}, cutters = {{ kind = "plane", index = plane }} }
bearcad.slice_sketch{ sketch = 0, lines = {target}, cutters = {cutter} }
bearcad.edit_slice{ index = 0, cutters = {{ kind = "plane", index = plane }} }
bearcad.edit_sketch_slice{ index = 0, lines = {target}, cutters = {cutter} }
```

## Offset and mirror

```lua
bearcad.offset_sketch{ sketch = 0, lines = sides, distance = 4 }
bearcad.offset_sketch{ sketch = 0, circles = {hole}, distance = -2, construction = true }
bearcad.edit_sketch_offset{ index = 0, distance = 6 }

bearcad.mirror_bodies{ plane = 0, bodies = {box} }                 -- output = "new"|"add"|"cut"
bearcad.mirror_sketch{ sketch = 0, line = "x", lines = {sides[1]} } -- or a line handle; `"y"` / `"gx"`
bearcad.edit_mirror{ index = 0, plane = 0, bodies = {box} }
bearcad.edit_sketch_mirror{ index = 0, sketch = 0, line = 0, lines = {sides[1]} }
```

## Repeat

`axis` is `"x"`/`"y"`/`"z"`, `{ line = i }`, a body edge, or `{ circle_normal = hole }` (turns `around` on). `to` is an extrude-style target instead of `length`. `gap` aliases `spacing`.

```lua
bearcad.repeat_bodies{ bodies = {box}, axis = "x", mode = "count_gap", count = 4, spacing = 8, flip = true }
bearcad.repeat_bodies{ bodies = {box}, axis = "z", around = true, mode = "count_gap", count = 6, spacing = "60deg" }
bearcad.repeat_bodies{ bodies = {box}, axis = "x", mode = "fill_pitch", spacing = 10, to = { plane = 1 } }
bearcad.repeat_sketch{ sketch = 0, circles = {hole}, angle = 0, mode = "count_gap", count = 4, spacing = 10 }
bearcad.repeat_cut{ cuts = {hole_cut}, axis = "x", mode = "count_gap", count = 4, spacing = 12 }
bearcad.repeat_sketches{ sketches = {sk}, axis = "z", mode = "count_gap", count = 3, spacing = 10 }
bearcad.edit_repeat{ index = 0, count = 6 }
bearcad.edit_sketch_repeat{ index = 0, count = 6 }
```

Sketch direction is `angle` in degrees (0 = sketch +X) or `dir = {du, dv}`.

## Move

Free: `x`/`y`/`z` and `rx`/`ry`/`rz` (or `rotate = { z = 90 }`). Point Snap: `from`/`to` as a point or a list of pairs. Face Snap: `on_face` + `normal`, then `flip` / `spin`. `roll` is the third pair as an angle.

```lua
bearcad.move_bodies{ bodies = {box}, x = 40, rz = 90 }
bearcad.move_bodies{ images = {img}, x = 25, rz = 90 }
bearcad.move_bodies{
  bodies = {box},
  from = { body = box, vertex = {0, 0, 0} },
  to   = { origin = true },
}
bearcad.edit_move{ index = 0, x = 30 }
local corners = bearcad.image_corners(img)   -- world quad, live preview included
```

Arm without committing: [`bearcad.ui.begin_move`](./ui-namespace).

## Joints

`body_faces` / `body_cylinders` spell the mate the same way the GUI pickers do.

```lua
bearcad.joint{
  a = base, b = moving, kind = "revolute",
  face = { moving = bearcad.body_faces(moving)[1], fixed = bearcad.body_faces(base)[1], offset = 2 },
  frame_axis = { axis = "z" },
  position = 90, turn_min = 0, turn_max = 110,
}
bearcad.joint{ parts = {a, b, c}, kind = "rigid" }
bearcad.joint{ a = base, b = moving, kind = "screw", lead = 2, position = 720 }
bearcad.edit_joint{ index = 0, position = 45 }
bearcad.set_joint_rest(0)
bearcad.revert_joint(0)
bearcad.revert_joints()
```

[`bearcad.ui.begin_joint`](./ui-namespace) arms the tool. [`bearcad.ui.animate_joints`](./ui-namespace) toggles the preview sweep.
