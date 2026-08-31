---
name: bearcad
description: Drive BearCAD, a parametric CAD app, from its Lua scripting API — build and edit 3D models, read geometry back, export STEP/STL/3MF, and take screenshots. Use when the user asks to model, measure, modify, or inspect a BearCAD document, or mentions .bearcad files.
---

# BearCAD

BearCAD is a parametric CAD app with two equal front ends: the GUI, and a Lua API. Anything
the GUI can do, a script can do. You drive the Lua API.

Model in millimetres, angles in **degrees** — in the GUI's fields and in the API alike.
An expression string may name its own unit (`"45deg"`, `"1.2rad"`, `"5in"` work anywhere
an expression is accepted).

## The complete API

This page is the working subset. **Every** function BearCAD exposes, with signatures, is one
plaintext page: <https://bearcad.com/bearcad-api.md> — or `bearcad api` locally. Fetch it
before reaching for a call that is not shown here; a name that is not on that page is not a
function.

## Running a script

```sh
bearcad --script build.lua            # run headless (no window), then exit
bearcad --script build.lua --exit     # same; --exit is implied for scripts
bearcad drawing.bearcad --headless    # open a document, no window
bearcad --script edit.lua drawing.bearcad
```

- Scripts run **headless** by default — offscreen rendering, no window, works over SSH
  and in CI. Add `--no-headless` to watch a script in a real window.
- `--timeout <seconds>` force-exits non-zero if it hangs. Use it in unattended runs.
- A failed `assert` or a Lua error exits non-zero and prints the traceback — that is how
  you find out something did not work.
- `bearcad --repl` reads Lua from stdin against a live window, so
  `echo 'print(bearcad.count("body"))' | bearcad --repl --exit` answers one question.
- If `bearcad` is not on PATH, run `bearcad install-cli` once (the app's Help menu has the
  same item).

A script runs in a coroutine. Calls that wait for a frame — `bearcad.ui.wait`,
`bearcad.ui.screenshot`, `bearcad.ui.view` — yield rather than block.

## The two namespaces

- **`bearcad.*` — declarative modeling. Prefer this.** Describe geometry directly.
- **`bearcad.ui.*` — simulated GUI use.** Mouse, keyboard, camera, panes, tools. Reach for
  it only when the interaction itself is the point (testing a drag, taking a screenshot of
  a tool in use).

```lua
bearcad.new()
bearcad.rect{ width = 80, height = 50, name = "Base" }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20, name = "Block" }
bearcad.export_step("block.step")
bearcad.quit()
```

## Sketching

Drawing verbs open a ground-plane sketch automatically when none is active.

```lua
bearcad.rect{ x = 0, y = 0, width = 80, height = 50, name = "Box" }
bearcad.circle{ x = 10, y = 5, r = 12, name = "Hole" }   -- radius / diameter also accepted
bearcad.line{ x = 0, y = 0, x1 = 50, y1 = 0 }            -- explicit endpoints
bearcad.line{ length = 80, angle = 45 }                  -- length + angle
bearcad.text{ text = "Hello", x = 10, y = 10, size = 12 }

bearcad.begin_sketch("construction_plane", 0)   -- sketch on a specific plane
bearcad.open_sketch(0)                          -- re-enter sketch 0
bearcad.exit_sketch()
```

A rectangle is **four lines**, indexed in creation order (bottom, right, top, left). A
scripted line lands unconstrained; `dimension = 50` or `dimension = "leg"` locks its length.

Every creation call hands back what it made — one element, or a list — and those handles go
anywhere an index does:

```lua
local sides = bearcad.rect{ width = 80, height = 50 }            -- four line handles
local box   = bearcad.extrude{ polygon = sides, distance = 10 }  -- the new body
box:id()    -- "body#3v0": unique in the document, never reused
box:index() -- its ordinal right now; an error once it's gone
```

Construction planes:

```lua
bearcad.plane{ offset = 12 }                                          -- above Ground
bearcad.plane{ offset = 5, origin = {0, 0, 20}, normal = {0, 0, 1} }  -- on a face
```

## Solids

```lua
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20, name = "Block" }
bearcad.revolve{ polygon = {0, 1, 2, 3}, axis = "y", angle = 180 }   -- axis is required
bearcad.combine{ op = "cut", a = {0}, b = {1} }  -- union | cut | intersect | xor  (`difference` means cut)
bearcad.shell{ bodies = {0}, thickness = 2 }
bearcad.move_bodies{ bodies = {0}, x = 40 }
bearcad.mirror_bodies{ bodies = {0}, plane = 0 }
```

An operation **consumes** the body it acts on and produces a new one, so the index moves:
after `shell{ bodies = {0} }`, the shelled result is the *last* body, and `{0}` is spent.
Chain operations off `bearcad.count("body") - 1`, or give bodies names and use
`bearcad.find`.

To cut into a body, sketch **on one of its faces**, then extrude with `body = "cut"`. A cut
pointing away from the body is flipped inward for you.

```lua
bearcad.begin_sketch{
  kind = "extrude_cap", extrusion = 0,
  profile = "polygon", profile_lines = {0, 1, 2, 3}, top = true,
}
bearcad.circle{ x = 40, y = 25, r = 5 }            -- r is a radius; (0,0) is the rect corner
bearcad.extrude{ circle = 0, distance = 20, body = "cut" }
```

Rounding takes **one call per operation** — a set of edges in a single call, never one call
per edge (four calls would make four bodies stacked on each other):

```lua
bearcad.fillet_edge{
  extrusion = 0,
  edges = {
    { kind = "vertical", face = 0, edge = 0 },
    { kind = "vertical", face = 0, edge = 1 },
  },
  radius = 8,
}
bearcad.chamfer_vertex{ point = { kind = "line", index = 0, endpoint = "end" }, distance = 3 }
```

## Parameters and constraints

Parameters make a model parametric; anywhere a size is accepted, an expression string is
too, and the model rebuilds when the parameter changes.

```lua
bearcad.add_parameter("w", "24")
bearcad.rect{ width = "w", height = "w / 3" }
bearcad.set_parameter("w", "30")             -- everything sized by w re-sizes

bearcad.constrain("parallel",                         -- horizontal, vertical, equal,
  { kind = "line", index = 0 },                       -- perpendicular, coincident, tangent…
  { kind = "line", index = 1 })
bearcad.dimension{ kind = "line", index = 0, value = "25mm" }
bearcad.dimension{ kind = "line", index = 1, value = "leg = 40mm" }  -- names a parameter
```

## Reading state back — verify your own work

Never assume a call did what you meant. Read it back and assert.

```lua
assert(bearcad.count("body") == 1)          -- line, circle, sketch, constraint, body,
                                            -- shape, extrusion, parameter, drawing,
                                            -- image, joint… (`get` takes the same set)
local l = bearcad.get{ kind = "line", index = 0 }         -- x0,y0,x1,y1,length…
local x0, y0, x1, y1 = bearcad.line_endpoints(0)
local s = bearcad.body_stats(0)                           -- volume, triangles, bbox
assert(math.abs(s.volume - 80 * 50 * 20) < 200)           -- tessellated, so allow a tolerance

bearcad.body_faces(0)      -- { body, face = {x,y,z}, normal = {x,y,z} }
bearcad.body_edges(0)
bearcad.body_cylinders(0)  -- holes and bosses: radius, length, axis — the reliable way to
                           -- check a hole is really there and really the right size
bearcad.selection()        -- what is selected
bearcad.sketch_dof()       -- remaining degrees of freedom
bearcad.sketch_conflicts()
print(bearcad.status())    -- the status bar: what the last action said
```

`bearcad.find("Main box")` looks an element up by name; `bearcad.set_name(el, "…")` renames
one. Options tables reject unknown keys and list the accepted ones, so a typo fails
immediately — wrap in `pcall` if you want to handle that yourself.

## Files

```lua
bearcad.open("part.bearcad")
bearcad.save()                  -- or save("other.bearcad")
bearcad.import_step("part.step")
bearcad.import_stl("part.stl")
bearcad.import_unit("bracket.bearcad")     -- another document as a reusable unit
bearcad.export_step("out.step")            -- real BREP; second arg is a handle/id/name/ordinal
bearcad.export_stl("out.stl")
bearcad.export_3mf("out.3mf")              -- one colored object per body
bearcad.undo()
```

**File → Export → Lua Script…** (and `bearcad.import_lua(path)`) round-trips a whole
document through this API — the fastest way to see how an existing document is built.

## Looking at the model

```lua
bearcad.ui.view("front")                       -- top, bottom, left, right, back, iso
bearcad.ui.camera{ yaw = 45, distance = 200 }  -- degrees; instant, so screenshots are deterministic
bearcad.ui.zoom_fit()
bearcad.ui.screenshot("shot.png")              -- the 3D viewport
bearcad.ui.screenshot("shot.png", "window")    -- the whole window
bearcad.ui.screenshot("shot.png", "elements")  -- one pane
```

Screenshots render offscreen: they work headless (the default for scripts) and in a
window alike, with no virtual display needed.

## Driving the GUI directly

```lua
bearcad.ui.tool("rectangle")
bearcad.ui.click_ground(0, 0)      -- sketch-plane millimetres
bearcad.ui.move_ground(80, 50)
bearcad.ui.key("enter")
bearcad.ui.click(x, y)             -- viewport pixels
bearcad.ui.click_ground(20, -10, { shift = true })
bearcad.ui.pane("ai", "show")      -- hierarchy, context, parameters, tutorials, ai
bearcad.ui.palette("Export STEP")  -- the command palette
```

## Connecting over MCP instead

If BearCAD is already open, it can host a local MCP server — the **MCP Server** section of
its AI pane — so you act on the document the user is looking at rather than one of your
own. `bearcad mcp-install` prints the client
configuration; `bearcad mcp` bridges stdio to the running app. Its tools are
`document_summary`, `document_lua`, `run_lua`, `undo` and `screenshot`.

## Rules of thumb

1. **Start from the document, not from scratch.** If one is open, read it (`bearcad.count`,
   `bearcad.get`, the Lua export) before changing anything.
2. **One operation per call.** Especially fillets, chamfers and booleans — batching edges
   into one call is the difference between one body and several.
3. **Assert what you built.** Geometry that silently failed looks identical to geometry
   that was never asked for.
4. **Indices are ordinals in creation order** and shift when things are deleted. Hold the
   handle a creation call returned (or a name) for anything you will refer to twice.
5. **Prefer the declarative API.** Reach for `bearcad.ui.*` only to test an interaction or
   to take a picture.
