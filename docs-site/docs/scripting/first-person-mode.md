---
sidebar_position: 6
title: First-person mode
---

# First-person mode

FPS mode walks around (and inside) a model like a first-person game. Also reachable from
the command palette ("Toggle FPS Mode") and the **View** menu.

```lua
bearcad.ui.fps()          -- toggle; bearcad.ui.fps(true) / bearcad.ui.fps(false) forces it
```

## Looking and moving

```lua
bearcad.ui.fps_look(20, 5)              -- turn the head: degrees right, degrees up
bearcad.ui.fps_move{ forward = 1000 }   -- walk along the ground, millimetres
bearcad.ui.fps_move{ strafe = -200 }    -- forward/strafe combine in one call
bearcad.ui.fps_jump()                   -- press the jump key once
```

`fps_move` is an instant, absolute offset along the current heading — not integrated
physics, so it positions the player precisely without frame timing.

## Flying

Double-tapping Space toggles flying interactively; from a script:

```lua
bearcad.ui.fps_fly(true)    -- start flying: no gravity, Space/Shift ascend/descend interactively
bearcad.ui.fps_fly()        -- toggle
bearcad.ui.fps_fly(false)   -- stop flying (resumes gravity from rest)
```

`fps_move` only offsets along the ground; ascending/descending while flying is
interactive-only.

## Advancing physics

`fps_advance(seconds)` integrates gravity/jump physics with no keys held:

```lua
bearcad.ui.fps()
bearcad.ui.fps_jump()
bearcad.ui.fps_advance(3)   -- enough time for gravity to bring the jump back down
```

## Scale

`[`/`]` shrink/grow the player interactively. Eye height, speeds, jump, and gravity scale
together — a smaller/larger person, not a world zoom. Look sensitivity and `fps_move`'s
millimetre offsets are unaffected.

```lua
bearcad.ui.fps_scale(0.1)   -- 1/10th human scale: eye height 170mm
bearcad.ui.fps_scale(10)    -- 10x human scale: eye height 17m
```

Scale clamps to 1/100×–100× human scale.

## Weapon-style tool switching

In FPS mode, number keys **1–9** pick tool slots and the wheel cycles tools (the wheel
doesn't zoom and right-drag doesn't orbit). Interactive-only; scripts call
[`bearcad.ui.tool(...)`](./ui-namespace) directly.

## Errors outside FPS mode

Every `fps_*` function except `fps()` raises a catchable error if FPS mode isn't active:

```lua
local ok, err = pcall(function() bearcad.ui.fps_jump() end)
assert(not ok, "fps_jump should require FPS mode")
```

## Reading state back

The player's eye/look writes the ordinary orbit camera every frame
(`target = eye + look`), so assert via [`bearcad.ui.camera{}`](./ui-namespace#camera):

```lua
bearcad.ui.fps()
local before = bearcad.ui.camera{}
bearcad.ui.fps_move{ forward = 500 }
local after = bearcad.ui.camera{}
assert(after.target[1] ~= before.target[1] or after.target[2] ~= before.target[2])
```
