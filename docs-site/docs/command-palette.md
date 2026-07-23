---
sidebar_position: 9
title: Command palette
---

# Command palette

**⌘/Ctrl+P** opens the command palette: a searchable list of every command available
right now — tools, views, document actions, pane toggles. Anything without a visible
button is reachable here.

![The command palette open over the viewport](/img/screenshots/command-palette.png)

- Matching is **fuzzy**: type any subsequence, like `zf` for **Zoom to Fit**.
- **↑/↓** move the highlight, **Enter** runs it, **Esc** closes.
- The list is **context-pertinent**: e.g. **Exit Sketch** appears only inside a sketch,
  **Delete Selection** only with something selected.
- **Every tool** is here — the sketch tools and the 3D ones (Extrude, Chamfer, Fillet, Revolve,
  Sweep, Combine, Mirror, Slice, and the rest) — so you can switch tool without hunting the
  toolbar.
- **Explode Selection Under Cursor** opens the [Selection Exploder](/docs/selection-exploder)
  right where your pointer is, the same as pressing **Space**.

## Scripting

```lua
bearcad.ui.palette("show")            -- show / hide / toggle
bearcad.ui.palette("run", "view top") -- run the best-matching entry for a query
```
