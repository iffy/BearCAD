---
sidebar_position: 14
title: Move
---

# <img src="/img/icons/move.svg" width="30" /> Move

Move translates and/or rotates whole bodies, producing moved copies — position an imported
part, space out duplicates, or angle a bracket into place.

![A box moved and rotated into a second position](/img/screenshots/move.png)

## How to use it

1. Pick the **Move** tool and click one or more bodies. Re-clicking removes one; the
   picked set shows in the context pane's **Bodies** element picker (the same combo-box
   control the other tools use), where you can review and remove them.
2. Type the translation **X / Y / Z** amounts. These are expressions — numbers,
   parameters, arithmetic — so the move stays parametric.
3. To rotate, pick an **axis** (the X/Y/Z buttons, or click any line in the viewport) and
   type the **Angle** (degrees by default; `rad` works; parameters work).
4. Press **Enter** (or the **Move** button) to commit.

## What you get

The inputs become [shadow bodies](/docs/tools/combine#shadow-bodies) and each one gains a
moved copy — a real body you can extrude against, cut, combine, or move again. The move
itself is an element in the pane with the copies nested under it: select it and choose
**Edit move** to change the amounts, axis, or picked set; delete it to restore the
originals.

Because the amounts are expressions, editing a parameter re-places every body moved by it.

## Moving construction planes and tracing images

The Move tool also moves **construction planes** and **tracing images** — pick one from the
Elements pane (or select it) with the Move tool active, then set the translation and rotation
just like a body.

- A **construction plane** moves in place: its frame shifts, and everything anchored to it —
  sketches built on the plane, images hosted on it, extrusions grown from those sketches —
  moves with it. Move a plane and the whole feature tree that lives on it follows.
- A **tracing image** slides in place on its host plane. In-plane translation repositions the
  image over your model; it stays flush on its plane. (An image sitting on a plane you move
  follows the plane, and can then be nudged on its own on top of that.)

Editing the move back to zero, or removing the plane/image from the move, returns it home.

## Scripting

```lua
bearcad.move_bodies{ bodies = {0}, x = "25", name = "Shifted" }
bearcad.move_bodies{ bodies = {0, 1}, x = "gap * 2", axis = "z", angle = "45" }
bearcad.edit_move{ index = 0, bodies = {0}, x = "30" }
```
