-- Documentation screenshots: one shot per joint kind — each a base slab and an arm
-- joined at the shared corner and posed so the kind's motion reads — plus one combined
-- shot with all eight side by side.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh), falling
-- back to ".". Nine PNGs: joint-kinds-<kind>.png and joint-kinds-all.png.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/joint-kinds"

local kinds = {
  { kind = "rigid" },
  { kind = "slider", axis_to = {0, 0, 0}, position = 12 },
  { kind = "revolute", position = 45 },
  { kind = "cylindrical", position = 6, position2 = 30 },
  { kind = "planar", position = 8, position2 = 5, position3 = 15 },
  { kind = "ball", position = 20, position2 = 15 },
  { kind = "pin_slot", position = 8, position2 = 25 },
  { kind = "screw", lead = 2, position = 540 },
}

for _, spec in ipairs(kinds) do
  bearcad.new()
  bearcad.ui.pane("elements", "hide")
  bearcad.ui.pane("context", "hide")
  bearcad.ui.pane("parameters", "hide")

  bearcad.rect{ width = 30, height = 20 }
  bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
  bearcad.rect{ x = 40, y = 0, width = 25, height = 8 }
  bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
  bearcad.exit_sketch()

  bearcad.joint{
    a = 0, b = 1, kind = spec.kind, lead = spec.lead,
    from   = { body = 1, vertex = {40, 0, 0} },
    to     = { body = 0, vertex = {30, 0, 0} },
    from_b = { body = 1, vertex = {40, 0, 5} },
    to_b   = { body = 0, vertex = spec.axis_to or {30, 0, 5} },
    position = spec.position, position2 = spec.position2, position3 = spec.position3,
  }

  for i = 0, 2 do bearcad.set_visible({ kind = "construction_plane", index = i }, "hide") end
  bearcad.ui.ground("off")
  bearcad.ui.view("corner", "front_left_top")
  bearcad.ui.wait(2)
  bearcad.ui.zoom_fit()
  bearcad.ui.wait(2)
  bearcad.ui.screenshot(out .. "-" .. spec.kind .. ".png")
end

-- The combined shot: every kind in one document, laid out four across in the table's
-- order, each pair posed through its joint.
bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

local vertex_count = 0
for i, spec in ipairs(kinds) do
  local col = (i - 1) % 4
  local row = math.floor((i - 1) / 4)
  local ox = col * 100
  local oy = row * -70
  bearcad.rect{ x = ox, y = oy, width = 30, height = 20 }
  bearcad.extrude{
    polygon = { vertex_count, vertex_count + 1, vertex_count + 2, vertex_count + 3 },
    distance = 5,
  }
  bearcad.rect{ x = ox + 40, y = oy, width = 25, height = 8 }
  bearcad.extrude{
    polygon = { vertex_count + 4, vertex_count + 5, vertex_count + 6, vertex_count + 7 },
    distance = 5,
  }
  vertex_count = vertex_count + 8
  local base = (i - 1) * 2
  local axis_to = { ox + 30, oy, 5 }
  if spec.axis_to then axis_to = { ox + spec.axis_to[1], oy + spec.axis_to[2], spec.axis_to[3] } end
  bearcad.joint{
    a = base, b = base + 1, kind = spec.kind, lead = spec.lead,
    from   = { body = base + 1, vertex = { ox + 40, oy, 0 } },
    to     = { body = base, vertex = { ox + 30, oy, 0 } },
    from_b = { body = base + 1, vertex = { ox + 40, oy, 5 } },
    to_b   = { body = base, vertex = axis_to },
    position = spec.position, position2 = spec.position2, position3 = spec.position3,
  }
end
bearcad.exit_sketch()

for i = 0, 2 do bearcad.set_visible({ kind = "construction_plane", index = i }, "hide") end
bearcad.ui.ground("off")
bearcad.ui.view("corner", "front_left_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)
bearcad.ui.screenshot(out .. "-all.png")

bearcad.quit()
