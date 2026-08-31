-- #1578: a sphere is selectable through the middle of its silhouette, not only on the
-- rim — including when it overlaps a cuboid whose edges share that pixel.
bearcad.new()
bearcad.cuboid{ width = 40, depth = 40, height = 20 }
bearcad.sphere{ at = {20, 20, 0}, radius = 12 }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.tool("select")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {20, 20, 0}, distance = 220 }
bearcad.ui.wait(10)

-- The rest-point of the sphere is the centre of its disc from above.
bearcad.ui.click_ground(20, 20)
bearcad.ui.wait(5)
local sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "body" and sel[1].index == 1,
  "clicking the middle of the sphere should select the sphere, got " ..
  (#sel > 0 and (sel[1].kind .. " " .. tostring(sel[1].index)) or "nothing"))

-- Combine: cuboid on Side A (scripted, like other Combine tests), then the sphere
-- through a real click in the middle of its disc.
bearcad.clear_selection()
bearcad.ui.wait(5)
bearcad.ui.tool("combine")
bearcad.ui.tool_mode("cut")
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

bearcad.select{ kind = "body", index = 0 }
bearcad.ui.wait(5)
local pa = picker("Side A")
assert(pa and #pa.items == 1 and pa.items[1].index == 0, "the cuboid should be Side A")
assert(picker("Side B") and picker("Side B").focused, "Side B should arm")

bearcad.ui.click_ground(20, 20)
bearcad.ui.wait(5)
local pb = picker("Side B")
assert(pb and #pb.items == 1 and pb.items[1].kind == "body" and pb.items[1].index == 1,
  "clicking the middle of the sphere should put it on Side B, got " ..
  (pb and (#pb.items > 0 and (pb.items[1].kind .. " " .. tostring(pb.items[1].index)) or "empty") or "no picker"))

print("ok: a sphere is selectable through the middle of its disc")
bearcad.quit()
