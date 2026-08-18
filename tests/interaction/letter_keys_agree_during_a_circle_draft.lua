-- #1482: every letter-key tool shortcut consults the same "does this draft own the next
-- click" predicate. Mid-circle, R used to abandon the circle while D/E stayed put.
bearcad.new()
bearcad.begin_sketch("construction_plane", 0)
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.tool("circle")
bearcad.ui.wait(5)
bearcad.ui.click_ground(0, 0)
bearcad.ui.move_ground(10, 0)
bearcad.ui.wait(5)
assert(bearcad.tool_row().tool == "circle", "circle draft should be armed")

for _, key in ipairs({ "r", "l", "d", "e", "m", "k", "f", "s" }) do
  bearcad.ui.key(key)
  bearcad.ui.wait(2)
  assert(bearcad.tool_row().tool == "circle",
    "mid-circle, " .. key .. " must not switch tools, got " .. bearcad.tool_row().tool)
end

print("ok: letter keys agree during a circle draft")
bearcad.quit()
