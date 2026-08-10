-- #1154: Slice 3D — after the first target lands in Targets, focus moves to Cutters so the
-- next click can pick a plane/face/line. Adding further targets (user re-armed Targets)
-- keeps focus on Targets so multi-body slices don't bounce away.
bearcad.new()
bearcad.rect{ width = 30, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.begin_sketch{ kind = "plane", index = 0 }
bearcad.rect{ x = 40, y = 0, width = 30, height = 20 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 10 }
bearcad.exit_sketch()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

bearcad.ui.tool("slice")
bearcad.ui.wait(5)
assert(picker("Targets").focused, "Targets starts focused")

-- First body → Cutters arms.
bearcad.select{ kind = "body", index = 0 }
bearcad.ui.wait(5)
assert(#picker("Targets").items == 1, "first body in Targets")
assert(picker("Cutters").focused, "first target arms Cutters")

-- Re-arm Targets and add a second body — stay on Targets.
bearcad.ui.picker_focus("Targets")
bearcad.ui.wait(5)
assert(picker("Targets").focused, "Targets re-armed")
bearcad.select{ kind = "body", index = 1 }
bearcad.ui.wait(5)
assert(#picker("Targets").items == 2, "second body in Targets")
assert(picker("Targets").focused,
  "a non-first target must keep focus on Targets")
assert(not picker("Cutters").focused, "Cutters stays unarmed")

print("ok: first Slice target arms Cutters; further targets keep Targets")
bearcad.quit()
