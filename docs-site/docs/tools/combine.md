---
sidebar_position: 18
title: Combine
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/combine.svg")} width="30" /> Combine

Combine performs boolean operations on whole bodies. Before a **Cut** — the block and
the overlapping cutting body:

<a
  href={useBaseUrl("/app/") + "?open=" + encodeURIComponent(useBaseUrl("/img/screenshots/combine-before.bearcad.json"))}
  target="_blank"
  rel="noopener noreferrer"
  title="Open this model in BearCAD"
>
  <img src={useBaseUrl("/img/screenshots/combine-before.png")} alt="Two overlapping bodies before the cut" />
</a>

After — the cutting body is carved away:

<a
  href={useBaseUrl("/app/") + "?open=" + encodeURIComponent(useBaseUrl("/img/screenshots/combine.bearcad.json"))}
  target="_blank"
  rel="noopener noreferrer"
  title="Open this model in BearCAD"
>
  <img src={useBaseUrl("/img/screenshots/combine.png")} alt="The notched result after the cut" />
</a>

## The four operations

| Operation | Result |
|---|---|
| **Combine** | One body containing everything in the picked set (union). |
| **Cut** | Side A with side B carved away (A − B). |
| **Intersect** | Only the material common to A and B. |
| **Difference** | Only the material *not* common to A and B (symmetric difference). |

## How to use it

1. Pick the **Combine** tool and choose the operation. Bodies already selected are
   side A.
2. Click bodies to add them. Two-sided operations have **Side A** and **Side B** pickers;
   click a picker to make it the active side. Re-clicking a body removes it.
3. **Keep cutting shape** / **Keep trimmed parts** / **Keep hole** keeps what that
   operation would discard. Intersect and Difference with it on both split A and B
   into three parts.
4. Press **Enter**.

Once every side the operation needs is picked, the result previews in the viewport —
the side-A bodies give way to a translucent ghost of what committing would build.

A cut that severs a body into separate pieces gives one body per piece.

## Help

![The Combine tool's Context pane in Combine mode, each field explained](/img/screenshots/pane-combine-combine.png)

![The Combine tool's Context pane in Cut mode, each field explained](/img/screenshots/pane-combine-cut.png)

## Shadow bodies

Input bodies become **shadow bodies**: out of the 3D view and out of clicking's way until
you hover or select them in the Elements pane, where they ghost translucently.

## The operation element

The operation is an element with the new bodies nested under it. **Edit operation**
changes the kind, inputs, or leftovers; deleting it restores the inputs. Result bodies
are ordinary bodies, so operations chain.

## Scripting

```lua
bearcad.combine{ op = "cut", a = {0}, b = {1}, name = "Notched block" }
bearcad.combine{ op = "combine", a = {0, 1, 2} }
bearcad.combine{ op = "intersect", a = {0}, b = {1}, keep_leftovers = true }
bearcad.edit_boolean{ index = 0, op = "difference", a = {0}, b = {1} }
-- `begin_combine` takes the same arguments but leaves the tool armed rather than
-- committing, so the result preview is on screen for a screenshot.
bearcad.begin_combine{ op = "cut", a = {0}, b = {1} }
```

## Good to know

- Switching from Combine to Cut, Intersect, or Difference after Side A is filled focuses Side B.
- The first Side A pick in Cut, Intersect, or Difference also focuses Side B. Further Side A picks stay on Side A.
- All four operations undo as a single step.
- An empty result (a cut that leaves nothing, or an intersect with no overlap) is
  refused — the inputs stay as they were.
- Shadow bodies can't be picked into another operation — edit or delete the operation
  that owns them instead.
