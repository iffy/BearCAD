---
sidebar_position: 14
title: Move
---

# Move

Move translates and/or rotates whole bodies, producing moved copies — position an imported
part, space out duplicates, or angle a bracket into place.

![A box moved and rotated into a second position](/img/screenshots/move.png)

## How to use it

1. Pick the **Move** tool and click one or more bodies. Re-clicking removes one; the
   picked set is listed in the context pane.
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

## Scripting

```lua
bearcad.move_bodies{ bodies = {0}, x = "25", name = "Shifted" }
bearcad.move_bodies{ bodies = {0, 1}, x = "gap * 2", axis = "z", angle = "45" }
bearcad.edit_move{ index = 0, bodies = {0}, x = "30" }
```
