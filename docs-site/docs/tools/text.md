---
sidebar_position: 17
title: Text
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/text.svg")} width="30" /> Text

Text places engraving-ready lettering in a sketch as glyph outlines you can edit, rotate,
and extrude or cut like any other profile.

## How to use it

With the **Text** tool (**T**) in a sketch, **click** where the text should start for a
box that grows to fit, or **drag a rectangle** for one that wraps text to that width. Type
in the editor that opens; the outlines re-bake as you type. With no sketch open, click any
face or plane to start a sketch there.

## The text editor

- **Text** — multi-line. Embed parameters in curly braces: `Bore {d}` re-bakes whenever
  `d` changes. Any expression works (`{d / 2}`); `{{` prints a literal brace; **Tab**
  accepts the completion popup.
- **Font** — any installed font family. **B / I / U** toggle bold, italic, underline.
- **Size** — font size in mm; an expression field, so lettering scales with the model.
- **Rotation°** — turns the text about its start point.
- **Wrap width** — empty grows to fit; a width in mm word-wraps.

## Pinning text to a point

A selected text shows nine anchor dots. In the **Pin** row, choose an anchor, click
**to point…**, then click a sketch vertex — the anchor stays on that vertex as the model
changes. **Unpin** releases it.

## Rotating with the Move tool

With the **Move** tool, drag the rotation ring around a selected text to turn it in place.

## Fonts travel with the file

The document embeds the font data and baked outlines, so the file renders identically on
machines without the font.

## Extruding and cutting text

The [Extrude](/docs/tools/extrude) tool treats a text as one face set: click it, then pull
it out or push it in to **cut** (engraving). Letter counters — the holes in `o`, `a` —
stay holes.

## Scripting

```lua
bearcad.text{ text = "Hello", x = 10, y = 10, size = 12 }
bearcad.text{ text = "Label", size = "w / 2", font = "Helvetica",
              bold = true, rotation = 30, name = "Lid label" }
bearcad.select{ kind = "sketch_text", index = 0 }
bearcad.count("sketch_text")

-- Engrave a text: extrude/cut the whole word (all its glyphs) in one call.
bearcad.extrude{ text = 0, distance = 1, body = "cut" }

-- Pin a text's anchor to a sketch point so it follows it as the model changes.
bearcad.pin_text{ text = 0, anchor = "center", line = 2, endpoint = "start" }
```

Like `rect` and `circle`, `text` begins a ground sketch when none is open. `size` accepts
an expression; `rotation` is degrees about `(x, y)`; optional `wrap` sets a wrap width in
mm; `font` defaults to a standard system font.
