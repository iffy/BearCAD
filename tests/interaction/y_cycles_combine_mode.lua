-- #1534: the same Y that cycles Extrude Output walks Combine's Mode.
bearcad.new()
bearcad.ui.tool("combine")
bearcad.ui.wait(3)
assert(bearcad.ui.tool_mode() == "union", "SetTool arms Union mode")

local expected = { "cut", "intersect", "xor", "union" }
for _, mode in ipairs(expected) do
  bearcad.ui.key("y")
  bearcad.ui.wait(3)
  assert(bearcad.ui.tool_mode() == mode, "Y should land on " .. mode .. ", got " .. tostring(bearcad.ui.tool_mode()))
end

print("ok: Y cycles Combine mode")
bearcad.quit()
