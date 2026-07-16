---
sidebar_position: 6
title: First-person mode
---

# First-person mode

FPS mode walks around (and inside) a model like a first-person game instead of orbiting a
camera around it. The same `bearcad.ui.fps_*` functions drive a scripted walkthrough, a
physics-timing test, or a quick toggle from the command palette.

```lua
bearcad.ui.fps()          -- toggle; bearcad.ui.fps(true) / bearcad.ui.fps(false) forces it
```

In the GUI, FPS mode is also reachable from the command palette ("Toggle FPS Mode") and the
**View** menu ("FPS Mode", checked while active).

## Looking and moving

```lua
bearcad.ui.fps_look(20, 5)              -- turn the head: degrees right, degrees up
bearcad.ui.fps_move{ forward = 1000 }   -- walk along the ground, millimetres
bearcad.ui.fps_move{ strafe = -200 }    -- forward/strafe combine in one call
bearcad.ui.fps_jump()                   -- press the jump key once
```

`fps_move` is an instant, absolute offset along the current heading — unlike the interactive
WASD keys, it isn't integrated physics, so it's a precise way to position the player from a
script without worrying about frame timing.

## Flying

Double-tapping Space toggles Minecraft-style flying interactively; from a script, set it
directly:

```lua
bearcad.ui.fps_fly(true)    -- start flying: no gravity, Space/Shift ascend/descend interactively
bearcad.ui.fps_fly()        -- toggle
bearcad.ui.fps_fly(false)   -- stop flying (resumes gravity from rest)
```

There's no scripted vertical move — `fps_move` only offsets along the ground (`forward`/
`strafe`); ascending/descending while flying is interactive-only (held Space/Shift).

## Advancing physics

`fps_advance(seconds)` integrates gravity/jump physics with no keys held — useful for landing a
jump or settling the player onto the ground without a real-time wait:

```lua
bearcad.ui.fps()
bearcad.ui.fps_jump()
bearcad.ui.fps_advance(3)   -- enough time for gravity to bring the jump back down
```

## Scale

`[`/`]` shrink/grow the player interactively, for working comfortably at mm-detail scale
or covering a building-sized model quickly. Eye height, walk/fly speed, jump speed, and gravity
all scale together — an intentionally smaller/larger person, not a world zoom. Look sensitivity
and `fps_move`'s explicit millimetre offsets are unaffected.

```lua
bearcad.ui.fps_scale(0.1)   -- 1/10th human scale: eye height 170mm
bearcad.ui.fps_scale(10)    -- 10x human scale: eye height 17m
```

Scale is clamped to 1/100×–100× human scale (eye height 17&nbsp;mm–170&nbsp;m); out-of-range
values are clamped rather than rejected.

## Weapon-style tool switching

While in FPS mode, number keys **1–9** pick tool slots and the mouse wheel cycles through the
full tool list — the wheel doesn't zoom and right-drag doesn't orbit while FPS mode is active,
since the mouse is busy looking around. This is interactive-only; there's no scripted equivalent
of "press 3" (call [`bearcad.ui.tool(...)`](./ui-namespace) directly instead).

## Errors outside FPS mode

Every `fps_*` function except `fps()` itself raises a catchable error if FPS mode isn't active:

```lua
local ok, err = pcall(function() bearcad.ui.fps_jump() end)
assert(not ok, "fps_jump should require FPS mode")
```

## Reading state back

There's no dedicated FPS getter — the player's eye/look continuously *writes* the ordinary orbit
camera every frame (`target = eye + look`), so assert on the resulting pose via
[`bearcad.ui.camera{}`](./ui-namespace#camera) instead:

```lua
bearcad.ui.fps()
local before = bearcad.ui.camera{}
bearcad.ui.fps_move{ forward = 500 }
local after = bearcad.ui.camera{}
assert(after.target[1] ~= before.target[1] or after.target[2] ~= before.target[2])
```
