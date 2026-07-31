-- Interaction regression (#993): two lines ruled across a box's top cap read as three faces to
-- anyone looking at them, but neither line closes a loop with anything — the regions are bounded
-- partly by the *face's own outline*, which the sketch never drew. So loop finding, which only
-- ever saw the sketch's own lines, found none and the whole cap stayed the only thing to extrude.
--
-- A 60x40 box 10 tall, cut by lines at y = 12 and y = 28. Extruding the middle band 6mm adds
-- 60 x 16 x 6 = 5760 to the base 24000 — the whole cap would add 14400.
bearcad.new()
bearcad.rect{ width = 60, height = 40 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 0, profile = "polygon",
                      lines = {0, 1, 2, 3}, top = true }
bearcad.line{ x = 0, y = 12, x1 = 60, y1 = 12 }
bearcad.line{ x = 0, y = 28, x1 = 60, y1 = 28 }
bearcad.exit_sketch()
bearcad.clear_selection()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {30, 20, 10}, distance = 220 }
bearcad.ui.wait(5)

local function picked_faces()
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == "Faces" then return #p.items end
  end
  return -1
end

bearcad.ui.tool("extrude")
bearcad.ui.wait(5)
assert(picked_faces() == 0, "nothing picked yet")

-- Click inside the middle band. It should take that band, not the cap it sits on.
bearcad.ui.click_ground(30, 20)
bearcad.ui.wait(8)
assert(picked_faces() == 1,
  "clicking a region should pick exactly that region, got " .. picked_faces())

bearcad.ui.type("6")
bearcad.ui.wait(4)
bearcad.ui.key("Enter")
bearcad.ui.wait(14)

local stats = bearcad.body_stats(0)
assert(math.abs(stats.volume - 29760) < 60,
  "extruding the middle band should add only that band (24000 + 5760); the whole cap would be "
    .. "38400 — got " .. string.format("%.0f", stats.volume))
assert(math.abs(stats.bbox.max[3] - 16) < 0.01,
  "and it should stand 6mm proud of the 10mm box, got " .. stats.bbox.max[3])

print("ok: lines ruled across a face divide it into separately extrudable regions")
bearcad.quit()
