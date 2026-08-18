-- Documentation screenshot: materials — a 2x2x2 of cubes plus a centre cube.
--
-- Eight cuboids on a small grid, assigned the eight contrasting palette colours
-- (Blue through Pink), and a ninth Grey cube of the same size at the cluster
-- centre so it overlaps every corner cube. One corner has a circle extruded
-- through it, one has a sphere cut from a side, one is chamfered, one is
-- filleted — real features, so the colours stay on distinct bodies.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/materials.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

-- Seeded palette ordinals 1–8: Blue, Green, Red, Yellow, Purple, Orange, Cyan, Pink.
-- Grey (9) is the centre cube. Unobtainium (0) stays unused.
local palette = {
  { name = "Blue", material = 1 },
  { name = "Green", material = 2 },
  { name = "Red", material = 3 },
  { name = "Yellow", material = 4 },
  { name = "Purple", material = 5 },
  { name = "Orange", material = 6 },
  { name = "Cyan", material = 7 },
  { name = "Pink", material = 8 },
}

-- `at` is the cuboid's base centre. Offset so the world origin sits in the
-- gap between the four columns — otherwise the Z axis spears a cube.
local size = 20
-- Wide enough that the centre cube reads in the windows between the corners,
-- still overlapping every corner so the cluster is one connected block.
local gap = 10
local step = size + gap
local half = step / 2
local i = 0
local at = {}
for z = 0, 1 do
  for y = 0, 1 do
    for x = 0, 1 do
      i = i + 1
      local spec = palette[i]
      at[i] = { (x * 2 - 1) * half, (y * 2 - 1) * half, z * step }
      bearcad.cuboid{
        at = at[i],
        width = size,
        depth = size,
        height = size,
        name = spec.name,
      }
      bearcad.set_material{ body = i - 1, material = spec.material }
    end
  end
end

-- Same size as the corners, centred in the cluster so it overlaps all eight.
bearcad.cuboid{
  at = { 0, 0, step / 2 },
  width = size,
  depth = size,
  height = size,
  name = "Centre",
}
bearcad.set_material{ body = 8, material = 9 }

local function last_body()
  return bearcad.count("body") - 1
end

local function vertical_sides()
  return {
    { kind = "vertical", face = 0, edge = 0 },
    { kind = "vertical", face = 0, edge = 1 },
    { kind = "vertical", face = 0, edge = 2 },
    { kind = "vertical", face = 0, edge = 3 },
  }
end

-- Orange (right-front-top): a circle through the centre of the top face, cut
-- down through the cube. Sketch (0, 0) is the top face's −u −v corner.
bearcad.begin_sketch{ kind = "primitive_face", primitive = 5, face = "top" }
bearcad.circle{ x = size / 2, y = size / 2, r = 5, name = "Hole" }
bearcad.extrude{ circle = 0, distance = -(size + 2), body = "cut" }
bearcad.exit_sketch()
bearcad.set_visible({ kind = "sketch", index = 0 }, "hide")

-- Green (right-front-bottom): subtract a sphere from the right (+X) side.
-- Centre on the wall (a hemispherical bite). Radius is a little larger than
-- half the face so the cut overshoots the four edges instead of sitting
-- as a circle inside the wall.
local green = at[2]
local bite_r = size / 2 + 1
bearcad.sphere{
  at = { green[1] + size / 2, green[2], green[3] + size / 2 - bite_r },
  radius = bite_r,
  name = "Bite",
}
bearcad.combine{ op = "cut", a = {1}, b = { last_body() } }
bearcad.set_material{ body = last_body(), material = 2 }

-- Purple (left-front-top): chamfer the four vertical sides.
bearcad.chamfer_edge{
  primitive = 4,
  edges = vertical_sides(),
  distance = 4,
}
bearcad.set_material{ body = last_body(), material = 5 }

-- Pink (right-back-top): fillet the four vertical sides.
bearcad.fillet_edge{
  primitive = 7,
  edges = vertical_sides(),
  radius = 4,
}
bearcad.set_material{ body = last_body(), material = 8 }

bearcad.clear_selection()
for i = 0, 2 do bearcad.set_visible({ kind = "construction_plane", index = i }, "hide") end
bearcad.ui.ground("off")
bearcad.ui.shading("realistic")
-- The OS cursor would hover-highlight whichever face it sits on; Dimension has no
-- pick hover, so the colours stay clean.
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_right_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)
bearcad.ui.screenshot(out)
-- The document behind this picture, so the docs page can link the screenshot into
-- the web app with `?open=` pointing here (#1049 pattern).
bearcad.save((out:gsub("%.png$", ".bearcad.json")))

bearcad.quit()
