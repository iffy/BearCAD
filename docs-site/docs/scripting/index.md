---
slug: /scripting
sidebar_position: 1
title: Scripting
---

# Scripting

BearCAD's Lua API is a first-class front end: everything achievable in the GUI is achievable by
scripting, and vice versa. Scripted actions create the same underlying document changes as GUI
actions — there is one model, two front ends.

The interpreter is **sandboxed**: no arbitrary filesystem/network access beyond the explicit
document/import/export/screenshot operations the API exposes.

## Namespace split

This is the single most important thing to know about the API's shape:

- The **primary API is declarative modeling**, in the spirit of OpenSCAD: geometry and document
  operations live at the top level — `bearcad.new`, `bearcad.rect`, `bearcad.extrude`,
  `bearcad.add_constraint`, `bearcad.parameter`, `bearcad.select`, and so on. You describe
  geometry directly instead of simulating clicks.
- **All GUI/UI manipulation** — simulated mouse/keyboard, camera motion, tool selection, panes,
  the command palette, and viewport drags — lives under the **`bearcad.ui.*`** sub-namespace:
  `bearcad.ui.move`, `bearcad.ui.click`, `bearcad.ui.key`, `bearcad.ui.type`, `bearcad.ui.orbit`,
  `bearcad.ui.pan`, `bearcad.ui.wheel`, `bearcad.ui.view`, `bearcad.ui.tool`, `bearcad.ui.pane`,
  `bearcad.ui.palette`, `bearcad.ui.drag_vertex`, `bearcad.ui.wait`, `bearcad.ui.screenshot`, and
  more.

**Prefer the declarative top-level API**, and reach for `bearcad.ui.*` only when the UI
interaction itself is the point — for example, testing that a click-drag on the Line tool
produces a curve, or capturing a screenshot of an in-progress draw. Most modeling scripts never
touch `bearcad.ui.*` at all.

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

The current CLI runs a script with the `--script` flag (or a bare `.lua` path) and, for headless
runs, `--exit` to close the app once the script finishes:

```sh
cargo run -- --script examples/rectangle.lua --exit
# equivalent:
cargo run -- examples/rectangle.lua --exit
```

Once installed as `bearcad` on your `PATH` (**Help → Install "bearcad" Command in PATH**, or
`bearcad install-cli`):

```sh
bearcad --script examples/rectangle.lua --exit
```

Both the desktop and browser apps also run a script interactively through **File → Load
Script…** — pick a `.lua` file and it executes against the current document, reporting completion
or the error in the status line. Browser Load Script runs the full modeling API (geometry,
constraints, drawings, queries, camera); the GUI-simulation verbs under `bearcad.ui.*`
(`move`/`click`/`key`/`type`/`wait`/`screenshot`, the semantic drags) run in the desktop app,
where a script plays out across frames.

Other useful flags:

- `--timeout <seconds>` — force-exit (non-zero) if the app hasn't closed on its own within the
  given duration, so an unattended/CI launch can't hang forever.
- `--show-commands` — echo GUI actions as `bearcad.*` calls on stdout as you interact with the
  app, useful for turning an interactive session into a script. The GUI's **Help → Export Session
  Commands…** does the same thing into a timestamped, replayable `.lua` file.

## Interactive REPL

`bearcad --repl` runs the same Lua API as an interactive session on stdin, against the live app —
the GUI stays fully usable while you type, so you can mix commands with mouse work and watch each
entry take effect in the viewport:

```
$ bearcad --repl
bearcad> x = 15
bearcad> bearcad.rect{ width = x * 2, height = x }
bearcad> 1 + 2
3
bearcad> bearcad.save("drawing.bearcad")
```

REPL semantics match the standalone `lua` interpreter's:

- **Globals persist between entries** (one Lua state for the whole session; `local`s are
  entry-scoped as usual).
- **Bare expressions echo their value** (rendered with `tostring`).
- **Errors print and the session continues** — a typo doesn't end the REPL.
- **Multi-line constructs** (an unclosed `function`, `do`, `if`…) buffer under a `...>`
  continuation prompt until the entry is syntactically complete.
- **Yielding calls work**: `bearcad.ui.wait`, camera transitions, and `bearcad.ui.screenshot`
  behave exactly as in scripts.
- **Ctrl-D** (EOF) ends the session; with `--exit` it also closes the app.

`--repl` and `--script` are mutually exclusive. Piping works too — `echo 'bearcad.rect{ width =
30, height = 20 }' | bearcad --repl --exit` behaves like a one-off script.

:::note CLI scope
`SPEC.md` §9 describes a longer-term `bearcad run script.lua`-style subcommand surface (`export`,
`run`, `render`, `set`, `import`/`convert`) as the CLI grows toward full GUI parity. As of this
writing the implemented CLI is the flag-based form shown above (`--script`, `--exit`,
`--show-commands`, `--timeout`, plus `install-cli`/`uninstall-cli`) — these docs describe what's
actually implemented today, per `src/script.rs`'s argument parser.
:::

## Import shorthand

Call `bearcad.import()` once at the top of a script to copy the top-level modeling functions into
the global namespace, so you can write `rect{}` instead of `bearcad.rect{}` (the `bearcad.ui.*`
functions stay namespaced under `bearcad.ui`):

```lua
bearcad.import()
new()
rect{ width = 80, height = 50 }
```

You can also bind individual functions locally: `local new, rect = bearcad.new, bearcad.rect`.

## Coroutines and waiting

Scripts run in a coroutine. Calls that need to wait for a frame or an animation — `bearcad.ui.wait`,
`bearcad.ui.wait_ms`, `bearcad.ui.screenshot`, and the `bearcad.ui.view(...)` camera commands —
yield until the next frame rather than blocking.

## Where to go next

- **[Declarative modeling](/docs/scripting/declarative-modeling)** — worked examples: sketch, draw,
  extrude, export.
- **[The `bearcad.ui.*` namespace](/docs/scripting/ui-namespace)** — camera, panes, the palette, and
  synthetic input.
- **[Point-level selection](/docs/scripting/point-selection)** — selecting a single vertex instead of a
  whole element, for scripted constraint authoring.
- **[First-person mode](/docs/scripting/first-person-mode)** — walking, flying, and scale, from a
  script.
