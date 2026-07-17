---
slug: /scripting
sidebar_position: 1
title: Scripting
---

# Scripting

BearCAD's Lua API is a first-class front end: everything achievable in the GUI is
achievable by scripting, and vice versa — one model, two front ends.

The interpreter is **sandboxed**: no filesystem/network access beyond the explicit
document/import/export/screenshot operations the API exposes.

## Namespace split

- The **primary API is declarative modeling**, OpenSCAD-style, at the top level:
  `bearcad.new`, `bearcad.rect`, `bearcad.extrude`, `bearcad.add_constraint`,
  `bearcad.parameter`, `bearcad.select`, ….
- **All GUI manipulation** — simulated mouse/keyboard, camera, tools, panes, the
  palette — lives under **`bearcad.ui.*`**: `bearcad.ui.click`, `bearcad.ui.key`,
  `bearcad.ui.orbit`, `bearcad.ui.tool`, `bearcad.ui.screenshot`, ….

Prefer the declarative API; reach for `bearcad.ui.*` only when the UI interaction itself
is the point.

```lua
-- Declarative (preferred): describe the geometry directly.
bearcad.new()
bearcad.rect{ width = 80, height = 50, name = "Main box" }

-- Simulated interaction (bearcad.ui.*): only when the interaction matters.
bearcad.ui.tool("rectangle")
bearcad.ui.click_ground(0, 0)
bearcad.ui.move_ground(80, 50)
bearcad.ui.key("enter")
```

## Running a script

`--script` (or a bare `.lua` path) runs a script; `--exit` closes the app when it
finishes:

```sh
cargo run -- --script examples/rectangle.lua --exit
# equivalent:
cargo run -- examples/rectangle.lua --exit
```

Once installed as `bearcad` on your `PATH` (**Help → Install "bearcad" Command in PATH**,
or `bearcad install-cli`):

```sh
bearcad --script examples/rectangle.lua --exit
```

Both the desktop and browser apps run a script interactively through **File → Load
Script…**. The browser runs the full modeling API; the `bearcad.ui.*` simulation verbs run
in the desktop app.

Other flags:

- `--timeout <seconds>` — force-exit (non-zero) if the app hasn't closed in time.
- `--show-commands` — echo GUI actions as `bearcad.*` calls on stdout. **Help → Export
  Session Commands…** does the same into a replayable `.lua` file.

## Interactive REPL

`bearcad --repl` runs the same Lua API on stdin against the live app — the GUI stays
usable while you type:

```
$ bearcad --repl
bearcad> x = 15
bearcad> bearcad.rect{ width = x * 2, height = x }
bearcad> 1 + 2
3
bearcad> bearcad.save("drawing.bearcad")
```

Semantics match the standalone `lua` interpreter: globals persist between entries, bare
expressions echo their value, errors print and the session continues, multi-line
constructs buffer under a `...>` prompt, and yielding calls (`bearcad.ui.wait`,
screenshots) work. **Ctrl-D** ends the session; with `--exit` it also closes the app.

`--repl` and `--script` are mutually exclusive. Piping works:
`echo '...' | bearcad --repl --exit`.

## Import shorthand

`bearcad.import()` copies the top-level modeling functions into the global namespace
(`bearcad.ui.*` stays namespaced):

```lua
bearcad.import()
new()
rect{ width = 80, height = 50 }
```

## Coroutines and waiting

Scripts run in a coroutine. Calls that wait for a frame or animation —
`bearcad.ui.wait`, `bearcad.ui.wait_ms`, `bearcad.ui.screenshot`, `bearcad.ui.view` —
yield until the next frame rather than blocking.

## Gizmos

Viewport drag handles are scriptable — each gizmo is one scalar:

```lua
-- What gizmos does the current tool state expose?
for _, g in ipairs(bearcad.gizmos()) do
  print(g.kind, g.name, g.value)   -- e.g. "push_pull"  "extrude"  7.0
end

bearcad.set_gizmo{ name = "extrude", value = 15 }   -- set the depth outright
bearcad.drag_gizmo{ name = "extrude", by = 5 }      -- nudge it (mirrors a drag delta)
```

Lengths are in millimetres, angles in radians. Gizmos today: `"extrude"`,
`"chamfer"`/`"fillet"`, `"revolve"`, `"offset"` (construction plane), and the Move tool's
`"move_x"`/`"move_y"`/`"move_z"` and `"move_angle"`.

## Where to go next

- **[Declarative modeling](/docs/scripting/declarative-modeling)** — worked examples:
  sketch, draw, extrude, export.
- **[The `bearcad.ui.*` namespace](/docs/scripting/ui-namespace)** — camera, panes, the
  palette, synthetic input.
- **[Point-level selection](/docs/scripting/point-selection)** — selecting a single
  vertex, for scripted constraint authoring.
- **[First-person mode](/docs/scripting/first-person-mode)** — walking, flying, and
  scale, from a script.
