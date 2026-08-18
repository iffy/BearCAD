-- #1494: every face-click tool starts a sketch on a plane click and keeps the tool
-- (the Sketch tool itself resets to Select).
bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 0}, distance = 200 }
bearcad.ui.wait(5)

local seen = {}
for _, row in ipairs(bearcad.tool_table()) do
  if row.opens_sketch and not seen[row.tool] then
    seen[row.tool] = true
    bearcad.new()
    bearcad.ui.pane("elements", "hide")
    bearcad.ui.pane("context", "hide")
    bearcad.ui.pane("parameters", "hide")
    bearcad.ui.auto_zoom(false)
    bearcad.ui.ground("off")
    bearcad.ui.view("top")
    bearcad.ui.wait(3)
    bearcad.ui.camera{ target = {0, 0, 0}, distance = 200 }
    bearcad.ui.wait(3)
    bearcad.ui.tool(row.tool)
    bearcad.ui.wait(3)
    bearcad.ui.click_ground(0, 0)
    bearcad.ui.wait(8)
    local live = bearcad.tool_row()
    assert(live.space == "sketch",
      row.tool .. " face click should start a sketch, space=" .. tostring(live.space)
        .. " status=" .. bearcad.status())
    if row.tool == "sketch" then
      assert(live.tool == "select",
        "Sketch tool resets to Select, got " .. live.tool)
    else
      assert(live.tool == row.tool,
        row.tool .. " should survive into the sketch, got " .. live.tool)
    end
  end
end

assert(seen["constraint"] and seen["project"],
  "Constraint and Project must be face-click tools")

print("ok: face-click tools start a sketch and keep the tool")
bearcad.quit()
