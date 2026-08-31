-- #1472: while placing a dimension label, nothing else hover-highlights or
-- selects. A click on the sketched-on face must drop the label, not pick the face.
bearcad.new()
bearcad.circle{ x = 0, y = 0, r = 20 }
bearcad.extrude{ circle = 0, distance = 10 }
bearcad.begin_sketch{
  kind = "extrude_cap",
  extrusion = 0,
  profile = "circle",
  profile_index = 0,
  top = true,
}
bearcad.circle{ x = 8, y = 0, r = 3 }

-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
-- The cap sits at z = 10; its origin is the extruded circle's centre.
bearcad.ui.camera{ target = {0, 0, 10}, distance = 220 }
bearcad.ui.wait(5)
bearcad.ui.tool("dimension")
bearcad.ui.wait(3)

-- Origin, then Shift+click the little circle: a placeable origin-to-circle distance.
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(8)
bearcad.ui.click_ground(8, 0, { shift = true })
bearcad.ui.wait(8)
assert(bearcad.status():find("place the dimension"),
  "expected a placeable origin-to-circle distance, got: " .. bearcad.status())

-- Interior of the cap, clear of the origin, the small circle, and the rim.
bearcad.ui.move_ground(0, 10)
bearcad.ui.wait(6)
local h = bearcad.ui.hovered()
assert(h == nil,
  "nothing should hover-highlight while placing a dimension, got "
    .. tostring(h and (h.kind .. (h.label and (" " .. h.label) or ""))))

local before = bearcad.count("constraint")
bearcad.ui.click_ground(0, 10)
bearcad.ui.wait(10)
local sel = bearcad.selection()
for _, e in ipairs(sel) do
  assert(e.kind ~= "face",
    "a placement click must not select the face, got " .. e.kind)
end
assert(bearcad.status():find("type length") or bearcad.status():find("Enter commit"),
  "the click should place the label and open the value editor, got: " .. bearcad.status())

bearcad.ui.type("12")
bearcad.ui.wait(4)
bearcad.ui.key("Enter")
bearcad.ui.wait(12)
assert(bearcad.count("constraint") > before,
  "placing the label should commit the origin-to-circle distance")

print("ok: dimension placement ignores the face under the cursor")
bearcad.quit()
