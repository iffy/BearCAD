-- #1529: after placing a Shape, Esc returns to Select. The hover ghost that
-- follows the cursor is not a pick, so the first Esc must leave the tool.
bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 0}, distance = 260 }
bearcad.ui.wait(5)

-- Empty 3D drafts: first Esc goes to Select. Walk the Solid rows so a new
-- 3D tool inherits the contract. Skip Select; do this on a blank document
-- so tool-switch handoff cannot seed picks.
for _, row in ipairs(bearcad.debug.tool_table()) do
  if row.space == "solid" and row.tool ~= "select" then
    bearcad.ui.tool(row.tool)
    bearcad.ui.wait(2)
    bearcad.ui.key("escape")
    bearcad.ui.wait(2)
    assert(bearcad.debug.tool_row().tool == "select",
      "empty " .. row.tool .. " Esc goes to Select")
  end
end

bearcad.ui.tool("shape")
bearcad.ui.wait(4)
bearcad.ui.click_ground(-20, -10)
bearcad.ui.wait(4)
bearcad.ui.click_ground(20, 10)
bearcad.ui.wait(4)
bearcad.ui.type("12")
bearcad.ui.wait(4)
bearcad.ui.key("enter")
bearcad.ui.wait(8)

assert(bearcad.count("body") == 1, "should have placed a cuboid")
assert(bearcad.debug.tool_row().tool == "shape", "Shape stays armed after commit")

-- Move the pointer so the hover ghost sizes itself, like a real session.
bearcad.ui.move_ground(40, 40)
bearcad.ui.wait(4)
bearcad.ui.key("escape")
bearcad.ui.wait(5)
assert(bearcad.debug.tool_row().tool == "select", "Esc after a committed Shape goes to Select")

print("ok: Esc after a committed Shape (and empty 3D tools) returns to Select")
bearcad.quit()
