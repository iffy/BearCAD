-- Documentation screenshots: one shot per joint kind — each a base slab and an arm
-- joined at the shared corner and posed so the kind's motion reads — plus one combined
-- shot with all eight side by side.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh), falling
-- back to ".". Nine PNGs: joint-kinds-<kind>.png and joint-kinds-all.png.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/joint-kinds"

local kinds = {
  { kind = "rigid", label = "Rigid" },
  { kind = "slider", label = "Sliding", axis_to = {0, 0, 0}, position = 12 },
  { kind = "revolute", label = "Revolute", position = 45 },
  { kind = "cylindrical", label = "Cylindrical", position = 6, position2 = 30 },
  { kind = "planar", label = "Planar", position = 8, position2 = 5, position3 = 15 },
  { kind = "ball", label = "Ball", position = 20, position2 = 15 },
  { kind = "pin_slot", label = "Pin-slot", position = 8, position2 = 25 },
  -- The screw is a rod threaded through a hole in the plate, so its turn-into-travel
  -- reads as what it is: two full turns at 4 mm of lead drive it 8 mm up.
  { kind = "screw", label = "Screw", rod = true, lead = 4, position = 720 },
}

-- Build one kind's pair at (ox, oy) — the base body first, then the moveable one —
-- advancing the sketch's line/circle counters in `idx`. Returns the joint's mating
-- points for the two body indices `a` (base) and `b` (moveable).
local function build_pair(spec, ox, oy, idx, a, b)
  if spec.rod then
    bearcad.rect{ x = ox, y = oy, width = 30, height = 20 }
    local plate = { idx.line, idx.line + 1, idx.line + 2, idx.line + 3 }
    idx.line = idx.line + 4
    bearcad.circle{ x = ox + 15, y = oy + 10, r = 4 }
    bearcad.circle{ x = ox + 15, y = oy + 10, r = 3.5 }
    local hole, rod = idx.circle, idx.circle + 1
    idx.circle = idx.circle + 2
    -- The plate is the rectangle minus the hole; the rod passes through it.
    bearcad.extrude{
      boolean = { op = "difference", a = { polygon = plate }, b = { circle = hole } },
      distance = 5,
    }
    bearcad.extrude{ circle = rod, distance = 24, symmetric = true }
    return {
      from   = { body = b, on_edge = { ox + 15, oy + 10, 0 } },
      to     = { body = a, on_edge = { ox + 15, oy + 10, 0 } },
      from_b = { body = b, on_edge = { ox + 15, oy + 10, 10 } },
      to_b   = { body = a, on_edge = { ox + 15, oy + 10, 10 } },
    }
  end
  bearcad.rect{ x = ox, y = oy, width = 30, height = 20 }
  local slab = { idx.line, idx.line + 1, idx.line + 2, idx.line + 3 }
  bearcad.rect{ x = ox + 40, y = oy, width = 25, height = 8 }
  local arm = { idx.line + 4, idx.line + 5, idx.line + 6, idx.line + 7 }
  idx.line = idx.line + 8
  bearcad.extrude{ polygon = slab, distance = 5 }
  bearcad.extrude{ polygon = arm, distance = 5 }
  local axis_to = { ox + 30, oy, 5 }
  if spec.axis_to then axis_to = { ox + spec.axis_to[1], oy + spec.axis_to[2], spec.axis_to[3] } end
  return {
    from   = { body = b, vertex = { ox + 40, oy, 0 } },
    to     = { body = a, vertex = { ox + 30, oy, 0 } },
    from_b = { body = b, vertex = { ox + 40, oy, 5 } },
    to_b   = { body = a, vertex = axis_to },
  }
end

for _, spec in ipairs(kinds) do
  bearcad.new()
  bearcad.ui.pane("elements", "hide")
  bearcad.ui.pane("context", "hide")
  bearcad.ui.pane("parameters", "hide")

  local frames = build_pair(spec, 0, 0, { line = 0, circle = 0 }, 0, 1)
  bearcad.exit_sketch()

  bearcad.joint{
    a = 0, b = 1, kind = spec.kind, lead = spec.lead,
    from = frames.from, to = frames.to, from_b = frames.from_b, to_b = frames.to_b,
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

local idx = { line = 0, circle = 0 }
for i, spec in ipairs(kinds) do
  local col = (i - 1) % 4
  local row = math.floor((i - 1) / 4)
  local ox = col * 100
  local oy = row * -70
  local base = (i - 1) * 2
  local frames = build_pair(spec, ox, oy, idx, base, base + 1)
  -- Named after their joint, so the linked document reads as eight labelled pairs.
  bearcad.set_name(bearcad.element("body", base), spec.label .. " Base")
  bearcad.set_name(bearcad.element("body", base + 1), spec.label .. " Moveable")
  bearcad.joint{
    a = base, b = base + 1, kind = spec.kind, lead = spec.lead,
    from = frames.from, to = frames.to, from_b = frames.from_b, to_b = frames.to_b,
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
-- The same document as the web JSON codec, deployed beside the shot: the docs page links
-- the screenshot to the web app with `?open=` pointing here.
bearcad.save(out .. "-all.bearcad.json")

bearcad.quit()
