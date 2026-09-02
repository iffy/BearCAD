-- #1946: dragging a jointed part whose position is driven by a parameter writes the new
-- value **into that parameter** instead of smashing the expression to a number. The
-- Parameters pane, `set_parameter` and dragging then stay in sync. Real pointer input.
bearcad.new()
bearcad.add_parameter("swing", "0")
local jamb = bearcad.cuboid{ width = 40, depth = 3, height = 40, at = {20, 1.5, 0} }
local door = bearcad.cuboid{ width = 40, depth = 3, height = 40, at = {-20, 1.5, 0} }
bearcad.joint{ a = jamb, b = door, kind = "revolute",
               frame_axis = { axis = "z" }, position = "swing" }
bearcad.ui.tool("select")
bearcad.ui.wait(4)

bearcad.ui.drag_world(-20, 1.5, 20, -20, -30, 20)
bearcad.ui.wait(4)

local j = bearcad.get{ kind = "joint", index = 0 }
assert(j.position == "swing",
       "the drag must keep the parameter driving the joint, got " .. tostring(j.position))
local swung = bearcad.parameter_value("swing")
assert(math.abs(swung) > 1,
       "the drag must write the angle into `swing`, got " .. tostring(swung))

-- And the parameter still drives the joint the other way round.
bearcad.set_parameter("swing", "0")
assert(bearcad.get{ kind = "joint", index = 0 }.position == "swing")
assert(math.abs(bearcad.parameter_value("swing")) < 1e-6)

-- A joint with a literal position keeps the old behaviour: the drag writes a number.
bearcad.new()
local a = bearcad.cuboid{ width = 40, depth = 3, height = 40, at = {20, 1.5, 0} }
local b = bearcad.cuboid{ width = 40, depth = 3, height = 40, at = {-20, 1.5, 0} }
bearcad.joint{ a = a, b = b, kind = "revolute", frame_axis = { axis = "z" }, position = "0" }
bearcad.ui.tool("select")
bearcad.ui.wait(4)
bearcad.ui.drag_world(-20, 1.5, 20, -20, -30, 20)
bearcad.ui.wait(4)
local literal = bearcad.get{ kind = "joint", index = 0 }.position
assert(tonumber(literal) ~= nil and math.abs(tonumber(literal)) > 1,
       "a literal position still lands as a number, got " .. tostring(literal))
