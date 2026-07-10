---
sidebar_position: 17
title: Text
---

# <img src="/img/icons/text.svg" width="30" /> Text

Text places real, engraving-ready lettering in a sketch — part numbers, labels, logos — as
glyph outlines you can edit, rotate, and extrude or cut like any other sketch profile.

## How to use it

1. Open a sketch and pick the **Text** tool.
2. Click where the text should start. A text element appears at that point and is selected,
   so its editor opens in the context pane immediately.
3. In the context pane, type your text (multi-line works — press Enter for a new line),
   choose a **font**, toggle **B**/**I**/**U**, and set the **size** and **rotation**.

Every change re-bakes the letter outlines right away, so the sketch always shows exactly
what you'll get.

## The text editor

Selecting a single text element (click it in the Elements pane, or place a new one) shows
its editor in the context pane:

- **Text** — a multi-line box; newlines stack lines below each other with the font's
  natural line spacing.
- **Font** — a chooser listing every font family installed on your computer.
- **B / I / U** — bold, italic, and underline toggles. Bold and italic select the matching
  face of the family.
- **Size** — the font size in mm. This is an expression field: numbers, units (`1cm`),
  parameters, and arithmetic (`w / 2`) all work, so lettering scales with your model.
- **Rotation°** — turns the text about its start point, in degrees.

## Rotating with the Move tool

With the **Move** tool active and a text selected, a rotation ring appears around the text.
Drag the ring to turn the text in place — the context pane's **Rotation°** field follows
live, and typing in the field turns the text the same way.

## Fonts travel with the file

Like a PDF, the document embeds the font data and the baked letter outlines. Open the file
on a machine that doesn't have the font and it still renders exactly as you made it.

## Extruding and cutting text

The [Extrude](/docs/tools/extrude) tool treats text as a face set: click the text and the
whole string highlights as one selection, then pull it out (or push it in to **cut** —
engraving). Letter counters — the holes in `o`, `a`, `e` — stay holes in the solid.

## Scripting

```lua
bearcad.text{ text = "Hello", x = 10, y = 10, size = 12 }
bearcad.text{ text = "Label", size = "w / 2", font = "Helvetica",
              bold = true, rotation = 30, name = "Lid label" }
bearcad.select{ kind = "sketch_text", index = 0 }
bearcad.count("sketch_text")
```

Like `rect` and `circle`, `text` begins a ground sketch when none is open. `size` accepts an
expression; `rotation` is degrees about the text's start point at `(x, y)`; optional `wrap`
sets a wrap width in mm. `font` defaults to a standard system font.
