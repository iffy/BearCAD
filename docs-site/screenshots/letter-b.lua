-- Documentation screenshot: a capital letter "B" drawn with the Line tool (using bezier
-- curves for its two rounded lobes AND its two rounded counters), extruded into a solid.
--
-- The B's outer silhouette is one closed loop: a straight spine on the left, two rounded
-- bumps on the right formed by bezier curves (#54), and a waist notch between them. Each
-- segment is drawn with the Line tool; the loop is closed by making each segment's end
-- coincident with the next segment's start, and the closed-loop face is extruded 12 mm.
--
-- A single sketch face is a simple loop (no holes), so the two counters are punched
-- afterwards as solid subtractions (#35): a sketch is opened on the extrusion's top cap,
-- an oval (four bezier quarter-arcs) is drawn inside each bowl, and each is extruded
-- downward through the body with `body = "cut"` — an OCCT boolean subtract that carves a
-- real rounded through-hole. That turns the silhouette into an unmistakable, fully-curved
-- "B". (Requires the OCCT kernel, on by default.)
--
-- Captured from a fixed orthographic top view for a clear, deterministic render (SPEC §8).
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh), else ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/letter-b.png"

-- Letter bounding box (letter coords: x = width rightward, y = height upward). The bezier
-- lobes bulge out to ~x=56.
local H, W = 72, 56

-- Outline segments, clockwise from the bottom-left, in letter coordinates. Each is
-- { x0, y0, x1, y1 } and may carry `bez = { c0x, c0y, c1x, c1y }` — two cubic-bezier control
-- handles (absolute letter coords) that round the two lobes outward to the right.
local segs = {
  { 0, 0, 0, 72 },                                  -- spine, straight (bottom-left -> top-left)
  { 0, 72, 18, 40, bez = { 54, 74, 54, 40 } },      -- upper lobe: bulges right into a round bump
  { 18, 40, 18, 32 },                               -- waist neck (concave gap between lobes)
  { 18, 32, 14, 0, bez = { 58, 32, 58, -2 } },      -- lower lobe: a slightly larger round bump
  { 14, 0, 0, 0 },                                  -- bottom edge back to the spine foot
}

-- The two counters (holes) as ellipses in letter coords: center (cx, cy) + radii (rx, ry),
-- sitting inside the upper and lower bowls with a solid rim all around.
local upper_oval = { cx = 23, cy = 56, rx = 13, ry = 8 }
local lower_oval = { cx = 24, cy = 16, rx = 14, ry = 8 }

-- The B is sketched flat on the XY ground plane and viewed from the top. The top view maps
-- world +x to screen-down and +y to screen-right, so rotate each letter point into sketch
-- (u, v) as (H/2 - y, x - W/2) to show it upright and centred on the origin.
local function u(y) return H / 2 - y end
local function v(x) return x - W / 2 end

bearcad.new()

-- Trace the outline with the Line tool: one segment per edge (the first auto-enters a
-- ground-plane sketch). Endpoints and bezier handles are both transformed into (u, v).
local n = #segs
for i = 1, n do
  local s = segs[i]
  local line = { x = u(s[2]), y = v(s[1]), x1 = u(s[4]), y1 = v(s[3]) }
  if s.bez then
    line.bezier = { { u(s.bez[2]), v(s.bez[1]) }, { u(s.bez[4]), v(s.bez[3]) } }
  end
  bearcad.line(line)
end

-- Close the outline loop: each segment's end coincident with the next segment's start.
for i = 0, n - 1 do
  local nxt = (i + 1) % n
  bearcad.select{ kind = "line", index = i, ["end"] = "end" }
  bearcad.select({ kind = "line", index = nxt, ["end"] = "start" }, true)
  bearcad.add_geometric_constraint("coincident")
end

-- Extrude the closed-loop face 12 mm into a solid body (extrusion 0, body 0).
local outline = { 0, 1, 2, 3, 4 }
bearcad.extrude{ polygon = outline, distance = 12, name = "B" }

-- The cap sketch shares the ground plane's (u, v) axes but is anchored at the profile loop's
-- first vertex (line 0's start = the outline's (0, 0)), so a letter point (lx, ly) maps into
-- cap-local coordinates as simply (-ly, lx).
local function cap(lx, ly) return -ly, lx end

-- Draw one counter as an ellipse: four cubic-bezier quarter-arcs (kappa = 4/3·(√2−1) is the
-- standard circle/ellipse control offset), each a Line-tool segment on the current sketch.
-- Returns nothing; appends 4 lines. `first` is the index of this counter's first line.
local KAPPA = 0.5522847498307936
local function draw_counter(o, first)
  local cx, cy, rx, ry = o.cx, o.cy, o.rx, o.ry
  local kx, ky = KAPPA * rx, KAPPA * ry
  -- (start, end, ctrl-near-start, ctrl-near-end) per quarter, in letter coords, CCW from +x.
  local arcs = {
    { { cx + rx, cy }, { cx, cy + ry }, { cx + rx, cy + ky }, { cx + kx, cy + ry } },
    { { cx, cy + ry }, { cx - rx, cy }, { cx - kx, cy + ry }, { cx - rx, cy + ky } },
    { { cx - rx, cy }, { cx, cy - ry }, { cx - rx, cy - ky }, { cx - kx, cy - ry } },
    { { cx, cy - ry }, { cx + rx, cy }, { cx + kx, cy - ry }, { cx + rx, cy - ky } },
  }
  for _, a in ipairs(arcs) do
    local x0, y0 = cap(a[1][1], a[1][2])
    local x1, y1 = cap(a[2][1], a[2][2])
    local c0x, c0y = cap(a[3][1], a[3][2])
    local c1x, c1y = cap(a[4][1], a[4][2])
    bearcad.line{ x = x0, y = y0, x1 = x1, y1 = y1, bezier = { { c0x, c0y }, { c1x, c1y } } }
  end
  -- Close this oval loop (its 4 arcs share endpoints): each arc's end coincident with the
  -- next arc's start.
  for k = 0, 3 do
    local i = first + k
    local nxt = first + (k + 1) % 4
    bearcad.select{ kind = "line", index = i, ["end"] = "end" }
    bearcad.select({ kind = "line", index = nxt, ["end"] = "start" }, true)
    bearcad.add_geometric_constraint("coincident")
  end
end

-- Open a sketch on the top cap, draw the two oval counters there (lines 5..8 and 9..12),
-- then cut each straight down through the 12 mm body — an OCCT boolean subtract per counter.
bearcad.begin_sketch{
  kind = "extrude_cap",
  extrusion = 0,
  profile = "polygon",
  profile_lines = outline,
  top = true,
}
draw_counter(upper_oval, 5)
draw_counter(lower_oval, 9)

bearcad.extrude{ polygon = { 5, 6, 7, 8 }, distance = -13, body = "cut" }
bearcad.extrude{ polygon = { 9, 10, 11, 12 }, distance = -13, body = "cut" }

-- Clean render: leave the sketch, hide the cap sketch's outlines and the ground plane so
-- only the solid B (with its real rounded cut holes) shows.
bearcad.exit_sketch()
bearcad.clear_selection()
bearcad.set_visible({ kind = "sketch", index = 1 }, "hide")
bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")

-- Orthographic top view, fixed framing.
bearcad.ui.view("ortho")
bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.wheel(12)
bearcad.ui.wait(2)
bearcad.ui.screenshot(out)

bearcad.quit()
