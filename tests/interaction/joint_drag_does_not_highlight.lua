-- #1949: while a jointed part is being dragged through its joint, nothing under the
-- cursor highlights for selection — the pointer is steering the part, not pointing at
-- things. Driven with a held press so the middle of the drag is observable.
bearcad.new()
bearcad.add_parameter("swing", "0")
local jamb = bearcad.cuboid{ width = 40, depth = 3, height = 40, at = {20, 1.5, 0} }
local door = bearcad.cuboid{ width = 40, depth = 3, height = 40, at = {-20, 1.5, 0} }
bearcad.joint{ a = jamb, b = door, kind = "revolute", frame_axis = { axis = "z" },
               position = "swing" }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.view("iso")
bearcad.ui.zoom_fit()
bearcad.ui.tool("select")
bearcad.ui.wait(6)

-- Hovering the jamb without a drag does highlight it: that is the control.
bearcad.ui.move_world(20, 1.5, 0)
bearcad.ui.wait(4)
assert(bearcad.ui.hovered() ~= nil, "hovering a part normally highlights it")

-- Grab the door leaf and sweep it across the jamb, holding the button.
bearcad.ui.press_world(-20, 1.5, 0)
bearcad.ui.wait(3)
bearcad.ui.move_world(-5, -19, 0)
bearcad.ui.wait(3)
bearcad.ui.move_world(19, 2.5, 0)
bearcad.ui.wait(3)
-- Mid-drag the joint carries the live angle as a literal; the parameter only takes it on
-- release (#1946), so the joint slot is what says the drag is under way.
assert(bearcad.get{ kind = "joint", index = 0 }.position ~= "swing",
       "the drag is under way")
assert(bearcad.ui.hovered() == nil,
       "nothing should highlight mid-drag, got " .. tostring(bearcad.ui.hovered() and
         bearcad.ui.hovered().kind))
bearcad.ui.release()
bearcad.ui.wait(4)

-- Once the drag lets go, hovering works again.
bearcad.ui.move_world(20, 1.5, 0)
bearcad.ui.wait(4)
assert(bearcad.ui.hovered() ~= nil, "hover comes back after the drag")
