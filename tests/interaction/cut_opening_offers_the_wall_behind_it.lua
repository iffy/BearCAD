-- Interaction regression (#1847): a pocket that breaks out through a wall leaves a hole
-- there. Two analytic faces still span that hole — the wall's own, and the cut prism's side
-- sitting flush in it — and the flush one is nearest the camera, so sketch-on-face offered a
-- surface you can see straight through instead of the pocket's inner wall behind it.
--
-- 60 x 40 x 40 block with a 20 x 16 pocket 20 deep, flush with the x = 60 wall. Looking at
-- that wall, the cursor inside the opening must land on the pocket's back wall at x = 40.
bearcad.new()
bearcad.rect{ width = 60, height = 40 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 40 }
bearcad.exit_sketch()
bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 0, profile = "polygon",
                      profile_lines = {0, 1, 2, 3}, top = true }
bearcad.rect{ x = 40, y = 12, width = 20, height = 16 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = -20, body = "cut" }
bearcad.exit_sketch()
bearcad.clear_selection()

bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("right")
bearcad.ui.wait(6)
bearcad.ui.camera{ target = {40, 20, 30}, distance = 200 }
bearcad.ui.wait(8)

bearcad.ui.tool("rectangle")
bearcad.ui.wait(5)

-- The middle of the opening: the pocket's back wall is the only material on this ray.
bearcad.ui.move_world(40, 20, 30)
bearcad.ui.wait(8)
local h = bearcad.hovered()
assert(h, "hovering into the pocket should highlight a face")
assert(h.kind == "face", "expected a face hover, got " .. tostring(h.kind))

bearcad.ui.click_world(40, 20, 30)
bearcad.ui.wait(12)
assert(bearcad.count("sketch") == 3, "the click should open a third sketch, got "
  .. bearcad.count("sketch"))

-- Which plane it opened on, measured rather than named: a small prism raised off the new
-- sketch has to straddle x = 40, never the x = 60 plane of the opening.
bearcad.ui.tool("select")
bearcad.ui.wait(3)
bearcad.circle{ x = 0, y = 0, r = 3 }
bearcad.extrude{ circle = 0, distance = 5 }
bearcad.ui.wait(8)
local stats = bearcad.body_stats(bearcad.count("body") - 1)
assert(stats, "the test prism should have a mesh")
local lo, hi = stats.bbox.min.x, stats.bbox.max.x
assert(math.min(math.abs(lo - 40), math.abs(hi - 40)) < 0.5, string.format(
  "sketch should sit on the pocket wall at x = 40, got a prism spanning x %.2f..%.2f",
  lo, hi))

print("ok: a cut's opening offers the wall behind it, not the hole")
bearcad.quit()
