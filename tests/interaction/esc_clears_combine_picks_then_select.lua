-- #1484: Esc on Combine used to throw the tool away. First press now empties the picks
-- and stays on Combine; the second returns to Select — the same rule as Move.
bearcad.new()
bearcad.rect{ width = 30, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
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

bearcad.ui.tool("combine")
bearcad.ui.wait(5)
bearcad.select{ kind = "body", index = 0 }
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

local bodies = picker("Bodies") or picker("Side A")
assert(bodies and #bodies.items >= 1, "Combine should hold the body")

bearcad.ui.key("escape")
bearcad.ui.wait(5)
assert(bearcad.debug.tool_row().tool == "combine", "first Esc keeps Combine")
bodies = picker("Bodies") or picker("Side A")
assert(bodies and #bodies.items == 0, "first Esc empties the picks")

bearcad.ui.key("escape")
bearcad.ui.wait(5)
assert(bearcad.debug.tool_row().tool == "select", "second Esc returns to Select")

print("ok: Esc clears Combine picks then returns to Select")
bearcad.quit()
