-- #1723: the step says to type the tilt, but the plane tool opens with its *Offset* field
-- armed — so 30 became a 30 mm offset and the plane never tilted. The step now focuses Tilt
-- as it lands, and rings that field so the typing hint has somewhere to hang.
bearcad.ui.tool("select")
bearcad.ui.tutorial("tilted_plane")
bearcad.ui.wait(8)

-- The datum planes are cleared away first (#1722), leaving nothing but the axes.
assert(bearcad.count("construction_plane") == 0,
  "the walkthrough starts with no datum planes, got " .. bearcad.count("construction_plane"))

-- Do the steps before the tilt for real, so the tutorial lands on the tilt step the way it
-- does for a user — its focus is set as the step is entered.
bearcad.ui.tutorial_next()
bearcad.ui.wait(4)
bearcad.ui.tool("construction_plane")
bearcad.ui.wait(6)
bearcad.ui.click_world(0, 60, 0)
bearcad.ui.wait(8)
local text = bearcad.ui.tutorial_narration()
assert(text and text:find("for the tilt"),
  "picking the axis should bring up the tilt step, got " .. tostring(text))

-- The step rings the Tilt field, so the typing guide has somewhere to hang.
assert(bearcad.ui.tutorial_orb(), "the tilt step rings the field it wants typed into")

-- Type it the way the step asks, with no clicking first.
bearcad.ui.type("30")
bearcad.ui.wait(4)
bearcad.ui.key("Enter")
bearcad.ui.wait(10)

assert(bearcad.count("construction_plane") == 1,
  "the plane is made, got " .. bearcad.count("construction_plane"))
local plane = bearcad.get{ kind = "construction_plane", index = 0 }
-- Tilted 30 degrees about the Y axis: the normal leans out of the XY plane, and the plane
-- still sits on the axis rather than 30 mm out from it.
assert(math.abs(plane.normal[3]) > 0.4,
  string.format("30 belongs in Tilt: normal (%.2f, %.2f, %.2f)",
    plane.normal[1], plane.normal[2], plane.normal[3]))
assert(math.abs(plane.origin[1]) < 1 and math.abs(plane.origin[3]) < 1,
  string.format("and not in Offset: origin (%.2f, %.2f, %.2f)",
    plane.origin[1], plane.origin[2], plane.origin[3]))

print("ok: the angled-plane walkthrough types the tilt into the Tilt field")
bearcad.quit()
