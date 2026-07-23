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
bearcad.ui.click_ground(0, 0)           -- click on the active sketch plane, in millimetres
bearcad.ui.move_ground(80, 50)
bearcad.ui.click(x, y)                  -- viewport pixel coordinates instead
bearcad.ui.move(x, y)
bearcad.ui.key("enter")
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
bearcad.ui.shading("solid_wireframe")   -- "wireframe" | "transparent" | "solid" | "solid_wireframe"
bearcad.ui.ground("off")                -- ground plane: "grid" | "solid" | "off"
```

Absolute camera control sets the pose **instantly** (no transition animation), which keeps
scripted screenshots deterministic; with no pose fields, `camera{}` is a pure read:

```lua
local c = bearcad.ui.camera{}           -- { yaw, pitch, distance, target = {x, y, z},
                                        --   projection = "perspective" | "orthographic" }
bearcad.ui.camera{ yaw = 1.0, distance = 200 }        -- set any subset of the pose
bearcad.ui.camera{ target = {20, 15, 5}, pitch = 0.6 }
bearcad.ui.zoom_fit()                   -- frame the whole document (bodies + sketch geometry)
```

See [Navigation](/docs/tools/navigation) for what these correspond to in the GUI, including the
view bear's gear/shading-modes popup.

## Panes and the command palette

```lua
bearcad.ui.pane("hierarchy", "hide")    -- show / hide / toggle a pane
bearcad.ui.pane("view_bear", "show")    -- panes: hierarchy, context, parameters, view_bear
bearcad.ui.palette("run", "view top")   -- run a command palette entry by name
bearcad.ui.elements_view("graph")       -- Elements-pane layout: "list" | "tree" | "graph"
```

## Dragging constrained geometry

```lua
bearcad.ui.drag_vertex({ kind = "line", index = 0, ["end"] = "end" }, u, v)
bearcad.ui.drag_line({ kind = "line", index = 0 }, au, av, u, v)
bearcad.ui.focus_dim("length")          -- focus a dimension input field
```

## Waiting

Because scripts run in a coroutine, these calls yield until the condition is met rather than
blocking the interpreter:

```lua
bearcad.ui.wait(5)        -- wait 5 UI frames
bearcad.ui.wait_ms(100)   -- wait 100 milliseconds
```

## Screenshots

```lua
bearcad.ui.screenshot()                       -- writes screenshot-bearcad.png
bearcad.ui.screenshot("out.png")
bearcad.ui.screenshot("out.png", true)        -- whole_window = true: capture the entire window
```

By default, `screenshot` captures the 3D viewport only (the view bear is suppressed for
that frame). This is the mechanism behind BearCAD's visual regression testing: a script
drives an exact interactive flow and emits a screenshot to compare against a golden image
in CI.

Whether an action arrives declaratively or through `bearcad.ui.*`, it lands as the same
committed document change.
