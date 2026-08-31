---
sidebar_position: 6
title: First-person mode (experimental)
---

# First-person mode (experimental)

Walk around (and inside) a model like a first-person game. Experimental.
Also reachable from the command palette ("Toggle FPS Mode (experimental)") and the
**View** menu.

```lua
bearcad.ui.first_person()          -- toggle; first_person(true) / first_person(false) forces it
```

## Looking and moving

```lua
bearcad.ui.first_person_look(20, 5)              -- turn the head: degrees right, degrees up
bearcad.ui.first_person_move{ forward = 1000 }   -- walk along the ground, millimetres
bearcad.ui.first_person_move{ strafe = -200 }    -- forward/strafe combine in one call
bearcad.ui.first_person_jump()                   -- press the jump key once
```

`first_person_move` is an instant, absolute offset along the current heading — not
integrated physics, so it positions the player precisely without frame timing.

## Flying

Double-tapping Space toggles flying interactively; from a script:

```lua
bearcad.ui.first_person_fly(true)    -- start flying: no gravity, Space/Shift ascend/descend interactively
bearcad.ui.first_person_fly()        -- toggle
bearcad.ui.first_person_fly(false)   -- stop flying (resumes gravity from rest)
```

`first_person_move` only offsets along the ground; ascending/descending while flying is
interactive-only.

## Advancing physics

`first_person_advance(seconds)` integrates gravity/jump physics with no keys held:

```lua
bearcad.ui.first_person()
bearcad.ui.first_person_jump()
bearcad.ui.first_person_advance(3)   -- enough time for gravity to bring the jump back down
```

## Scale

`[`/`]` shrink/grow the player interactively. Eye height, speeds, jump, and gravity scale
together — a smaller/larger person, not a world zoom. Look sensitivity and
`first_person_move`'s millimetre offsets are unaffected.

```lua
bearcad.ui.first_person_scale(0.1)   -- 1/10th human scale: eye height 170mm
bearcad.ui.first_person_scale(10)    -- 10x human scale: eye height 17m
```

Scale clamps to 1/100×–100× human scale.

## Weapon-style tool switching

In first-person mode, number keys **1–9** pick tool slots and the wheel cycles tools (the
wheel doesn't zoom and right-drag doesn't orbit). Interactive-only; scripts call
[`bearcad.ui.tool(...)`](./ui-namespace) directly.

## Errors outside first-person mode

Every `first_person_*` function except `first_person()` raises a catchable error if
first-person mode isn't active:

```lua
local ok, err = pcall(function() bearcad.ui.first_person_jump() end)
assert(not ok, "first_person_jump should require first-person mode")
```

## Reading state back

The player's eye/look writes the ordinary orbit camera every frame
(`target = eye + look`), so assert via [`bearcad.ui.camera{}`](./ui-namespace#camera):

```lua
bearcad.ui.first_person()
local before = bearcad.ui.camera{}
bearcad.ui.first_person_move{ forward = 500 }
local after = bearcad.ui.camera{}
assert(after.target[1] ~= before.target[1] or after.target[2] ~= before.target[2])
```
