-- #1336: destination picks during Move ignore the body being moved, so a click through
-- it lands on the stationary geometry behind — as if the moving body weren't there.
bearcad.new()
-- Stationary slab at the origin. The moving block is larger and sits over it, so from
-- the top the slab's far corner is hidden behind the block.
bearcad.rect{ width = 40, height = 40 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.rect{ x = -5, y = -5, width = 50, height = 50 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 20 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.tool("move")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {20, 20, 0}, distance = 260 }
bearcad.ui.wait(5)

-- Body 1 is the covering block. Start A is already set so the next click is End A.
bearcad.begin_move{
  bodies = {1},
  from   = { body = 1, vertex = {-5, -5, 0} },
}
bearcad.ui.tool_mode("snap")
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
end

assert(picker("End point A") and picker("End point A").focused,
  "end point A should be armed for the destination click")
assert(#picker("End point A").items == 0, "end A starts empty")

-- (40, 40) is a corner of the slab. The moving block covers that pixel from above.
bearcad.ui.move_ground(40, 40)
bearcad.ui.wait(8)
local h = bearcad.hovered()
assert(h, "hovering the buried slab corner should highlight something")
assert(h.kind == "body_vertex",
  "it should be the slab's corner, not the moving block's face, got " .. tostring(h.kind))

bearcad.ui.click_ground(40, 40)
bearcad.ui.wait(8)
assert(#picker("End point A").items == 1,
  "the click should take the slab corner through the moving body, got "
    .. #picker("End point A").items .. " (status: " .. bearcad.status() .. ")")
assert(bearcad.status():find("Body 0") or bearcad.status():find("end A"),
  "end A should land on the stationary slab, got: " .. bearcad.status())

bearcad.ui.key("enter")
bearcad.ui.wait(10)
assert(bearcad.count("body") == 3,
  "the move should commit a moved body, got " .. bearcad.count("body") ..
  " (status: " .. bearcad.status() .. ")")
-- Start A was (-5, -5); it lands on the slab's (40, 40), so the copy's min xy is 40.
local placed = bearcad.body_stats(bearcad.count("body") - 1).bbox
assert(math.abs(placed.min.x - 40) < 0.05 and math.abs(placed.min.y - 40) < 0.05,
  "the block's start corner should land on the slab's (40, 40), got min "
    .. placed.min.x .. ", " .. placed.min.y)

print("ok: destination pick clicks through the moving body")
bearcad.quit()
