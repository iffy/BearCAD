-- Documentation screenshot: a capital letter "B" drawn with the Line tool (using bezier
-- curves for its two rounded lobes), extruded into a solid, then given its two counters
-- (the enclosed holes) as real 3D cuts.
--
-- The B's outer silhouette is one closed loop: a straight spine on the left, two rounded
-- bumps on the right formed by bezier curves (#54), and a waist notch between them. Each
-- segment is drawn with the Line tool; the loop is closed by making each segment's end
-- coincident with the next segment's start, and the resulting closed-loop face is extruded
-- 12 mm into a solid body.
--
-- A single sketch face is a simple loop (no holes), so the two counters are punched
-- afterwards as solid subtractions (#35): a sketch is opened on the extrusion's top cap,
-- a rectangle is drawn inside each bowl, and each is extruded downward through the body
-- with `body = "cut"` — an OCCT boolean subtract that carves a real through-hole. That
-- turns the silhouette into an unmistakable "B". (Requires the OCCT kernel, on by default.)
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

-- The two counters (holes) as letter-coordinate rectangles {x, y, width, height}, sitting
-- inside the upper and lower bowls with a solid rim all around.
local upper_hole = { 10, 48, 26, 16 }
local lower_hole = { 10, 8, 28, 16 }

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

-- Close the loop: each segment's end coincident with the next segment's start (0-indexed).
for i = 0, n - 1 do
  local nxt = (i + 1) % n
  bearcad.select{ kind = "line", index = i, ["end"] = "end" }
  bearcad.select({ kind = "line", index = nxt, ["end"] = "start" }, true)
  bearcad.add_geometric_constraint("coincident")
end

-- Extrude the closed-loop face 12 mm into a solid body (extrusion 0, body 0).
local outline = { 0, 1, 2, 3, 4 }
bearcad.extrude{ polygon = outline, distance = 12, name = "B" }

-- The cap sketch is anchored at the profile loop's first vertex (line 0's start), so
-- cap-local coords are ground (u, v) minus that vertex.
local off_u = u(segs[1][2])
local off_v = v(segs[1][1])
local function hole_rect(r)
  local lx, ly, lw, lh = r[1], r[2], r[3], r[4]
  return {
    x = (H / 2 - (ly + lh)) - off_u, -- min u (hole's top edge), cap-relative
    y = (lx - W / 2) - off_v,        -- min v (hole's left edge), cap-relative
    width = lh,                      -- u extent = letter height
    height = lw,                     -- v extent = letter width
  }
end

-- Open a sketch on the top cap and draw the two counter rectangles there (lines 5..8 and
-- 9..12), then cut each straight down through the 12 mm body via an OCCT boolean subtract.
bearcad.begin_sketch{
  kind = "extrude_cap",
  extrusion = 0,
  profile = "polygon",
  profile_lines = outline,
  top = true,
}
local up = hole_rect(upper_hole)
bearcad.rect{ x = up.x, y = up.y, width = up.width, height = up.height }
local lo = hole_rect(lower_hole)
bearcad.rect{ x = lo.x, y = lo.y, width = lo.width, height = lo.height }

bearcad.extrude{ polygon = { 5, 6, 7, 8 }, distance = -13, body = "cut" }
bearcad.extrude{ polygon = { 9, 10, 11, 12 }, distance = -13, body = "cut" }

-- Clean render: leave the sketch, hide the cap sketch's outlines and the ground plane so
-- only the solid B (with its real cut holes) shows.
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
