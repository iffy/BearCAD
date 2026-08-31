-- #963: a click in the Elements pane feeds the **focused** picker, not a hardcoded set per
-- tool. With the Slice tool's Cutters picker armed, picking a construction plane must land in
-- Cutters — before, only bodies were routed at all, and always to the tool's one fixed set.
-- #1154: the first target auto-arms Cutters, so a plane can land there without a manual re-arm.
bearcad.new()
bearcad.rect{ width = 40, height = 40 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

bearcad.ui.tool("slice")
bearcad.ui.wait(5)

-- Targets is focused first, so a body lands there.
assert(picker("Targets").focused, "Targets should start focused")
bearcad.select{ kind = "body", index = 0 }
bearcad.ui.wait(5)
assert(#picker("Targets").items == 1,
  "the body should enter Targets, got " .. #picker("Targets").items)
assert(#picker("Cutters").items == 0, "and not Cutters")
-- #1154: first target hands focus to Cutters.
assert(picker("Cutters").focused,
  "after the first target, Cutters should be focused")

-- Cutters is now armed, so a construction plane is exactly what it wants (#963).
bearcad.select{ kind = "plane", index = 0 }
bearcad.ui.wait(5)
local cutters = picker("Cutters").items
assert(#cutters == 1 and cutters[1].kind == "plane",
  "the plane should enter Cutters, got " .. #cutters)
assert(#picker("Targets").items == 1, "and Targets is untouched")

-- Re-arm Targets: a plane is not a valid target, so it must not land there.
bearcad.ui.picker_focus("Targets")
bearcad.ui.wait(5)
assert(picker("Targets").focused, "Targets re-armed")
bearcad.select{ kind = "plane", index = 1 }
bearcad.ui.wait(5)
assert(#picker("Targets").items == 1,
  "a plane is not a body target, got " .. #picker("Targets").items)
assert(#picker("Cutters").items == 1,
  "the second plane must not join Cutters while Targets is focused")

print("ok: a pane click lands in whichever picker is focused")
bearcad.quit()
