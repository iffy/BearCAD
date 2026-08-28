---
sidebar_position: 3
title: The bearcad.ui.* namespace
---

# The `bearcad.ui.*` namespace

Everything under `bearcad.ui` simulates a real user driving the GUI — mouse, keyboard,
camera, tools, panes, palette. Use it when the UI interaction itself is what you're
testing; for ordinary modeling use the [declarative API](./declarative-modeling).

## Tools and synthetic input

```lua
bearcad.ui.tool("rectangle")            -- select, line, circle, sketch, rectangle, ...
bearcad.ui.tool()                       -- no name: the tool that is armed
bearcad.ui.click_ground(0, 0)           -- click on the active sketch plane, in millimetres
bearcad.ui.move_ground(80, 50)
bearcad.ui.click(x, y)                  -- viewport pixel coordinates instead
bearcad.ui.double_click(x, y)           -- opens a sketch / plane / dimension for editing
bearcad.ui.repeat_tool{ axis = "x", count = 5, gap = 40 }   -- the Repeat tool's fields
bearcad.ui.repeat_tool{ offset = true, to_end = false, computed = "gap" }  -- and its toggles
bearcad.ui.click_world(20, 10, 50)      -- a point in world space: a body's side wall, say
bearcad.ui.move_world(20, 10, 50)
bearcad.ui.viewport()                   -- { width, height, x, y } of the area clicks address
bearcad.ui.click_ground(20, -10, { shift = true })   -- Shift+click
bearcad.ui.click_ground(20, -10, { ctrl = true })    -- Ctrl+click: one edge, not its run
bearcad.ui.move(x, y)
bearcad.ui.right_click_ground(55, 55)   -- right-click: opens a context menu
bearcad.ui.right_click(x, y)
local menu = bearcad.ui.context_menu()  -- { kind, index } of the open menu's target, or nil
bearcad.ui.key("enter")
bearcad.ui.key("space", { shift = true })   -- Shift+Space (e.g. one-shot additive exploder)
bearcad.ui.type("12.5")
```

## Camera

```lua
bearcad.ui.orbit(dx, dy)
bearcad.ui.pan(dx, dy)
bearcad.ui.wheel(scroll)
bearcad.ui.view("front")                -- standard view; waits for the camera animation
bearcad.ui.view("edge", "front_top")    -- a view-bear edge
bearcad.ui.view_home()
bearcad.ui.toggle_projection()
bearcad.ui.shading("solid_wireframe")   -- "wireframe" | "transparent" | "solid"
                                        -- "solid_wireframe" | "realistic" | "loose_pencil"
bearcad.ui.ground("off")                -- ground plane: "grid" | "solid" | "off"
```

Absolute camera control sets the pose **instantly** (no transition animation), which keeps
scripted screenshots deterministic; with no pose fields, `camera{}` is a pure read:

```lua
local c = bearcad.ui.camera{}           -- { yaw, pitch, distance, target = {x, y, z},
                                        --   projection = "perspective" | "orthographic",
                                        --   shading, ground }
bearcad.ui.camera{ yaw = 45, distance = 200 }         -- angles are degrees
bearcad.ui.camera{ target = {20, 15, 5}, pitch = -35 }
bearcad.ui.zoom_fit()                   -- frame selection or document (short glide)
bearcad.ui.animate_zoom_to_fit(false)   -- off = snap zoom_fit instantly
bearcad.ui.snapping(false)              -- snapping while drawing and placing shapes
```

See [Navigation](/docs/tools/navigation) for what these correspond to in the GUI, including the
view bear's gear/shading-modes popup.

## Panes and the command palette

```lua
bearcad.ui.pane("hierarchy", "hide")    -- show / hide / toggle a pane
bearcad.ui.pane("view_bear", "show")    -- panes: hierarchy, context, parameters, view_bear, ai
local r = bearcad.ui.pane_rect("elements")  -- {x, y, w, h} last frame, or nil if hidden
local v = bearcad.ui.pane_scroll("ai")      -- {offset, content, viewport}, or nil if hidden
bearcad.ui.scroll_pane("ai", 200)           -- wheel over a pane; positive scrolls down
bearcad.ui.ai_sections("open")              -- AI pane sections: "open" | "close"
bearcad.ui.palette("run", "view top")   -- run a command palette entry by name
-- With a construction plane selected, the third value is the image path:
bearcad.ui.palette("run", "import image on this plane", "drawing.png")
bearcad.ui.elements_view("graph")       -- Elements-pane layout: "list" | "tree" | "graph"
bearcad.ui.workbench()                  -- "model" | "sketch" | "drawing" | "view"
bearcad.ui.workbench("view")            -- switch, opening the most recent view/drawing/sketch
local g = bearcad.ui.elements_graph()   -- the graph view's one-node-per-line layout
bearcad.ui.elements_graph{ shadow_bodies = true }  -- include shadow bodies
-- g.lanes                              -- how many lanes wide it is
-- g.rows[i]  = { name=, kind=, lane=, x=, y=, w=, h= }   -- x/y/w/h: where the row was drawn
--                                                          last frame, in click coordinates
-- g.edges[i] = { from=, to=, lane=, kind = "parent" | "dependency" | "related" }
--                                                          from/to are row numbers
```

## Tabs

```lua
bearcad.ui.os_open("part.bearcad")      -- same path as a Finder / file-manager double-click
bearcad.ui.new_tab()                    -- blank document tab
bearcad.ui.new_tab{ same = true }       -- same document, fresh view
bearcad.ui.tab(1)                       -- activate tab (0-based); bare call returns active index
bearcad.ui.close_tab()                  -- active tab; or close_tab(i)
bearcad.ui.reorder_tab(from, to)
bearcad.ui.detach_tab()                 -- move tab to its own full application window
local n = bearcad.ui.tab_count()
local w = bearcad.ui.window_count()     -- OS windows (main + detached)
local tabs = bearcad.ui.tabs()          -- { { title=, dirty=, active= }, ... } (1-based)
local wins = bearcad.ui.windows()       -- cycle order: main, detached, report_issue, mcmaster
local focus = bearcad.ui.focused_window()
bearcad.ui.report_issue("show")         -- DEV Report issue window
bearcad.ui.key("`", { cmd = true })     -- next window (⌘` / Ctrl+`)
```

## Dragging constrained geometry

```lua
bearcad.ui.drag_vertex({ kind = "line", index = 0, ["end"] = "end" }, u, v)
bearcad.ui.drag_line({ kind = "line", index = 0 }, au, av, u, v)
bearcad.ui.focus_dim("length")          -- focus a dimension input field
bearcad.ui.focus_calibrate()            -- focus a selected image's Real length field
```

## Waiting

Because scripts run in a coroutine, these calls yield until the condition is met rather than
blocking the interpreter:

```lua
bearcad.ui.wait(5)        -- wait 5 UI frames
bearcad.ui.wait_ms(100)   -- wait 100 milliseconds
```

## Changelog

**Help → Changelog** shows the changelog baked into this build.

```lua
bearcad.ui.changelog("show")   -- "hide" / "toggle"
local md = bearcad.ui.changelog("text")
```

## Help mode

**Help → Help Mode** in the OS menu (⌘/Ctrl+/) toggles it in the app; from scripts:

```lua
bearcad.ui.help(true)    -- explain every Context-pane control
bearcad.ui.help(false)
bearcad.ui.help()        -- toggle
local keys = bearcad.ui.toolbar_shortcuts()  -- { shape = "B", sketch = "S", ... }
                                             -- empty while help mode is off
local bar = bearcad.ui.toolbar_tools()       -- current workbench tools, left to right
```

With help mode on, each row of the Context pane gets a floating note beside it saying what
it wants, and each toolbar tool that has a shortcut grows a small badge with that key.
A pane screenshot widens to include the notes, which is how the annotated pane
pictures in the tool pages are made.

## Viewport tool hints

The usage line at the bottom of the 3D view. **View → Tool Hints** toggles it; from scripts:

```lua
bearcad.ui.tool_hints(false)   -- hide
bearcad.ui.tool_hints(true)    -- show
bearcad.ui.tool_hints()        -- toggle
```

Docs screenshot scripts turn it off so the line does not cover the model.

## Tutorials

```lua
bearcad.ui.tutorial_pane("show")   -- list every walkthrough (status-bar Tutorials button)
local list = bearcad.ui.tutorials() -- { {name=, title=, number=, completed=}, ... }
bearcad.ui.tutorial("cube")        -- start by registry name; fresh document
bearcad.ui.tutorial_next()
bearcad.ui.tutorial_assist()
bearcad.ui.tutorial_end()
local step = bearcad.ui.tutorial_step()  -- nil when none running
local text = bearcad.ui.tutorial_narration()  -- current step text, or nil
local orb = bearcad.ui.tutorial_orb()    -- {x=, y=} window px, or nil
                                         -- subtract viewport().x/y to click it
local bubble = bearcad.ui.tutorial_bubble() -- {x=, y=, w=, h=} screen px, or nil
bearcad.ui.complete_all_tutorials()     -- mark every walkthrough finished
bearcad.ui.unstart_all_tutorials()      -- clear every completion check
bearcad.ui.install_age(5)               -- pretend this fresh install is 5 days old
local prompt = bearcad.ui.tutorial_prompt("launch")  -- {text=, alpha=}, or nil
bearcad.ui.tutorial_prompt("work")
bearcad.ui.tutorial_prompt("tick", 3.5)
assert(bearcad.ui.tutorial_highlight())
```

```lua
bearcad.ui.tool_mode("free")   -- put the active tool in one of its modes
assert(bearcad.ui.tool_mode() == "free")
```

`tool_mode` names the mode a tool's Context pane offers — `"snap"`/`"free"` for Move,
`"combine"`/`"cut"`/`"intersect"`/`"difference"` for Combine, `"cuboid"`/`"cylinder"`/
`"sphere"` for Shape, `"new"`/`"merge"`/`"cut"` for Extrude/Revolve/Sweep/Loft/Mirror
Output — reaching mode rows that a scripted click cannot. Called with no argument it
returns the armed mode (or `nil`).

## Screenshots

```lua
bearcad.ui.screenshot()                       -- writes screenshot-bearcad.png
bearcad.ui.screenshot("out.png")
bearcad.ui.screenshot("out.png", true)        -- the entire window
bearcad.ui.screenshot("out.png", "window")    -- the same, named
bearcad.ui.screenshot("out.png", "context")   -- just the Context pane
bearcad.ui.screenshot("out.png", "elements")  -- just the Elements pane
```

By default, `screenshot` captures the 3D viewport only (the view bear is suppressed for
that frame). This is the mechanism behind BearCAD's visual regression testing: a script
drives an exact interactive flow and emits a screenshot to compare against a golden image
in CI.

The second argument picks the region: `"viewport"` (the default), `"window"`, or a pane
name — `"context"`, `"elements"`, `"parameters"`. A pane shot is cropped to the pane and
stops below its last control, so it is the controls rather than a tall empty column. The
docs' annotated pane pictures are made this way.

Whether an action arrives declaratively or through `bearcad.ui.*`, it lands as the same
committed document change.
