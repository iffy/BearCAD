-- Documentation screenshots: one shot per joint kind — each a base slab and an arm
-- joined at the shared corner and posed so the kind's motion reads — plus one combined
-- shot with all eight side by side.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh), falling
-- back to ".". Nine PNGs: joint-kinds-<kind>.png and joint-kinds-all.png.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/joint-kinds"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

-- Faces and edges are named by their own geometry, which `body_faces`/`body_edges` report.
local function near(p, q)
  return math.abs(p[1] - q[1]) < 0.01 and math.abs(p[2] - q[2]) < 0.01
     and math.abs(p[3] - q[3]) < 0.01
end
local function face_facing(body, n)
  for _, f in ipairs(bearcad.body_faces(body)) do
    if near(f.normal, n) then return f end
  end
  error("no face of body " .. body .. " faces {" .. table.concat(n, ", ") .. "}")
end
local function edge_between(body, a, b)
  for _, e in ipairs(bearcad.body_edges(body)) do
    if (near(e.edge[1], a) and near(e.edge[2], b))
       or (near(e.edge[1], b) and near(e.edge[2], a)) then return e end
  end
  error("no edge of body " .. body .. " runs between those corners")
end

local kinds = {
  { kind = "rigid", label = "Rigid" },
  -- The two kinds whose slide is travel take their direction from a line-up row, since a
  -- part flush on a face slides along it rather than off it.
  { kind = "slider", label = "Sliding", line_up = true, position = 12 },
  { kind = "revolute", label = "Revolute", position = 45 },
  { kind = "cylindrical", label = "Cylindrical", position = 6, position2 = 30 },
  { kind = "planar", label = "Planar", position = 8, position2 = 5, position3 = 15 },
  { kind = "ball", label = "Ball", position = 20, position2 = 15 },
  { kind = "pin_slot", label = "Pin-slot", line_up = true, position = 8, position2 = 25 },
  -- The screw is a rod threaded through a hole in the plate, so its turn-into-travel
  -- reads as what it is: two full turns at 4 mm of lead drive it 8 mm up.
  { kind = "screw", label = "Screw", rod = true, lead = 4, position = 720 },
}

-- Build one kind's pair at (ox, oy) — the base body first, then the moveable one.
-- Returns the mate that places the moveable body against the base: a face on a face,
-- plus a line-up row where the kind needs a direction to travel in — and the two bodies.
local function build_pair(spec, ox, oy)
  if spec.rod then
    local plate = bearcad.rect{ x = ox, y = oy, width = 30, height = 20 }
    local hole = bearcad.circle{ x = ox + 15, y = oy + 10, r = 4 }
    local rod = bearcad.circle{ x = ox + 15, y = oy + 10, r = 3.5 }
    -- The plate is the rectangle minus the hole; the rod passes through it.
    local a = bearcad.extrude{
      boolean = { op = "difference", a = { polygon = plate }, b = { circle = hole } },
      distance = 5,
    }
    local b = bearcad.extrude{ profiles = rod, distance = 24, symmetric = true }
    -- The rod's lower cap onto the plate's underside, both facing the same way, so the rod
    -- stands through the hole it was cut for and screws along that face's normal.
    return {
      face = {
        moving = face_facing(b, {0, 0, -1}),
        fixed  = face_facing(a, {0, 0, -1}),
        flip = true,
      },
    }, a, b
  end
  local slab = bearcad.rect{ x = ox, y = oy, width = 30, height = 20 }
  local arm = bearcad.rect{ x = ox + 40, y = oy, width = 25, height = 8 }
  local a = bearcad.extrude{ profiles = slab, distance = 5 }
  local b = bearcad.extrude{ profiles = arm, distance = 5 }
  -- The arm's inner face onto the slab's outer one, so it stands against the slab.
  local mate = {
    face = { moving = face_facing(b, {-1, 0, 0}), fixed = face_facing(a, {1, 0, 0}) },
  }
  if spec.line_up then
    mate.line_up = {
      {
        moving = edge_between(b, { ox + 40, oy, 0 }, { ox + 40, oy + 8, 0 }),
        fixed  = edge_between(a, { ox + 30, oy, 0 }, { ox + 30, oy + 20, 0 }),
      },
    }
  end
  return mate, a, b
end

for _, spec in ipairs(kinds) do
  bearcad.new()
  bearcad.ui.pane("elements", "hide")
  bearcad.ui.pane("context", "hide")
  bearcad.ui.pane("parameters", "hide")

  local mate, base, moveable = build_pair(spec, 0, 0)
  bearcad.exit_sketch()

  bearcad.joint{
    a = base, b = moveable, kind = spec.kind, lead = spec.lead,
    face = mate.face, line_up = mate.line_up,
    position = spec.position, position2 = spec.position2, position3 = spec.position3,
  }

  bearcad.set_visible({ kind = "plane" }, false)
  bearcad.ui.ground("off")
  bearcad.ui.view("corner", "front_left_top")
  bearcad.ui.wait(2)
  bearcad.ui.zoom_fit()
  bearcad.ui.wait(2)
  bearcad.ui.screenshot(out .. "-" .. spec.kind .. ".png")
  bearcad.save(out .. "-" .. spec.kind .. ".bearcad.json")
end

-- The combined shot: every kind in one document, laid out four across in the table's
-- order, each pair posed through its joint.
bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

for i, spec in ipairs(kinds) do
  local col = (i - 1) % 4
  local row = math.floor((i - 1) / 4)
  local ox = col * 100
  local oy = row * -70
  local mate, base, moveable = build_pair(spec, ox, oy)
  -- Named after their joint, so the linked document reads as eight labelled pairs.
  bearcad.set_name(base, spec.label .. " Base")
  bearcad.set_name(moveable, spec.label .. " Moveable")
  bearcad.joint{
    a = base, b = moveable, kind = spec.kind, lead = spec.lead,
    face = mate.face, line_up = mate.line_up,
    position = spec.position, position2 = spec.position2, position3 = spec.position3,
  }
end
bearcad.exit_sketch()

bearcad.set_visible({ kind = "plane" }, false)
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
