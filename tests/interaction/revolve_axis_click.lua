-- #970: the Revolve tool matched on the pick target to decide whether a click was the axis or
-- a body to cut, with its own copy of the pick→axis mapping. The axis is an ordinary element
-- pick now, so the armed picker decides and the mapping is shared with the Repeat tool's path.
bearcad.new()
bearcad.rect{ width = 20, height = 10 }
-- A separate line off to the side, to revolve the profile about.
bearcad.line{ x = 0, y = -15, x1 = 20, y1 = -15 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
-- View transitions are animated (~0.35s); wait enough frames for true top-view
-- pitch (#1183) before zoom_fit / profile pick.
bearcad.ui.wait(30)
bearcad.ui.zoom_fit()
bearcad.ui.wait(10)
bearcad.ui.tool("revolve")
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

assert(picker("Profile").focused, "Profile is the primary, armed first")
-- Geometric center of the 20×10 rectangle. Under true top view this is on the
-- fan-triangulation diagonal; point_in_tri must include edges (see face.rs).
bearcad.ui.click_ground(10, 5)
bearcad.ui.wait(5)
assert(#picker("Profile").items == 1,
  "the click should take the profile, got " .. #picker("Profile").items)
assert(picker("Axis").focused, "with a profile picked, the Axis takes over")

-- The separate line is the axis. It's an ordinary line pick.
bearcad.ui.click_ground(10, -15)
bearcad.ui.wait(5)
local axis = picker("Axis")
assert(#axis.items == 1, "the line should become the axis, got " .. #axis.items)
assert(axis.items[1].kind == "line", "and it should be the line, got " .. axis.items[1].kind)
-- With both settled the primary is armed again, so a click still edits the profile.
assert(picker("Profile").focused, "Profile takes the ring back once the axis is set")

print("ok: the Revolve axis is an ordinary element pick")
bearcad.quit()
