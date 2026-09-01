-- #1901: double-clicking a cuboid's Elements row reopens it with its dimensions
-- loaded. Clicking Height must not commit and blank the ValueInputs.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 40, depth = 20, height = 10 }
bearcad.ui.wait(8)

local row = bearcad.ui.elements_row_rect("Cuboid 0")
assert(row and row.x, "the cuboid has an Elements row")
bearcad.ui.double_click(row)
bearcad.ui.wait(8)
assert(bearcad.ui.tool() == "shape",
  "double-click reopens the Shape tool, got " .. tostring(bearcad.ui.tool()))

local height = bearcad.ui.context_row_rect("Height")
assert(height and height.x, "the Height ValueInput is showing")
bearcad.ui.click(height)
bearcad.ui.wait(5)

local status = bearcad.status()
assert(not tostring(status):find("Edited the cuboid"),
  "clicking Height must not commit, status is " .. tostring(status))
assert(bearcad.ui.tool() == "shape",
  "still editing, tool is " .. tostring(bearcad.ui.tool()))
assert(bearcad.ui.context_row_rect("Height"), "the Height field is still there")
local s = bearcad.get{ kind = "shape", index = 0 }
assert(math.abs(s.height - 10) < 1e-3, "the cuboid is unchanged, height " .. tostring(s.height))

print("ok: reopening a cuboid and clicking Height does not blank it")
bearcad.quit()
