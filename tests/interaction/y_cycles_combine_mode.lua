-- #1534: the same Y that cycles Extrude Output walks Combine's Mode.
bearcad.new()
bearcad.ui.tool("combine")
bearcad.ui.wait(3)
assert(bearcad.ui.tool_mode() == "combine", "SetTool arms Combine mode")

local expected = { "cut", "intersect", "difference", "combine" }
for _, mode in ipairs(expected) do
  bearcad.ui.key("y")
  bearcad.ui.wait(3)
  assert(bearcad.ui.tool_mode() == mode, "Y should land on " .. mode .. ", got " .. tostring(bearcad.ui.tool_mode()))
end

print("ok: Y cycles Combine mode")
bearcad.quit()
