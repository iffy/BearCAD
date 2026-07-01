-- Documentation screenshot: a capital letter "B" drawn with the Line tool, extruded
-- into a solid, then given its two counters (the enclosed holes) as real 3D cuts.
--
-- The B's outer silhouette (a straight spine on the left, two bumps on the right with a
-- waist notch between them) is one closed polygon: traced segment by segment with the
-- Line tool, the loop closed by making each segment's end coincident with the next
-- segment's start (`bearcad.select` two endpoints + `add_geometric_constraint`), and the
-- resulting closed-loop face extruded 12 mm into a solid body.
--
-- A single sketch face is a simple loop (no holes), so the two counters are punched
-- afterwards as solid subtractions (#35): a sketch is opened on the extrusion's top cap,
-- a small rectangle is drawn inside each bowl, and each is extruded downward through the
-- body with `body = "cut"` — an OCCT boolean subtract that carves a real through-hole.
-- That turns the silhouette into an unmistakable "B". (Requires the OCCT kernel, on by
-- default; a --no-default-features build cannot subtract solids.)
--
-- Captured from a fixed orthographic top view so the letter reads clearly and the output
-- is deterministic (SPEC §8).
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh), falling
-- back to ".". The PNG is only written where a real GPU frame renders (a display, or CI
-- Linux with xvfb + software Vulkan); otherwise the capture never resolves and --timeout
-- force-exits without a PNG, which is expected.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/letter-b.png"

-- Outer silhouette of a blocky capital "B" in natural letter coordinates (x = width,
-- rightward; y = height, upward), traced clockwise from the bottom-left. Its 2D area is
-- 2928 mm^2 (see the shoelace checks in `src/extrude.rs`'s letter_b tests).
local pts = {
  { 0, 0 },   -- bottom-left
  { 0, 72 },  -- up the left spine
  { 44, 72 }, -- across the top (upper bump top)
  { 44, 44 }, -- down the upper bump's right edge
  { 22, 44 }, -- in to the waist, near the spine (separates the two bumps)
  { 22, 28 }, -- down the waist notch (the concave dip between the two bumps)
  { 48, 28 }, -- out to the lower bump (a little wider than the upper one)
  { 48, 0 },  -- down the lower bump's right edge; back to (0,0) closes the loop
}

-- The two counters (holes), fully enclosed inside the upper and lower bowls, given as
-- letter-coordinate rectangles {x, y, width, height}. Each leaves a solid rim on every
-- side (spine on the left, and a margin at the top/right/bottom), so cutting it clean
-- through the body reads as a real B counter.
local upper_hole = { 12, 50, 20, 16 }
local lower_hole = { 12, 6, 22, 16 }

-- The B is sketched flat on the XY ground plane and viewed from the top. The top view's
-- fixed roll maps world +x to screen-down and world +y to screen-right, so drawing the
-- letter in raw (x, y) would show it rotated 90 degrees. Rotate each point into sketch
-- space as (u, v) = (H/2 - y, x - W/2) (letter height H = 72, width W = 48) so the top
-- view reads it upright — spine on the left, two bumps on the right, wider bump at the
-- bottom — and centred on the origin so it sits in the middle of the ground plane.
local H, W = 72, 48
local function u(p) return H / 2 - p[2] end
local function v(p) return p[1] - W / 2 end

-- The cap sketch shares the ground plane's (u, v) axes, but a face sketch is anchored at
-- the profile loop's first vertex (line 0's start = pts[1] here), not the plane origin —
-- so cap-local coordinates are ground (u, v) minus that vertex. Offset the hole rectangles
-- by it so they line up over the two bowls of the extruded B.
local off_u = u(pts[1])
local off_v = v(pts[1])

-- A letter-coordinate rectangle {lx, ly, lw, lh} maps into the cap sketch frame: the
-- rectangle's local x runs along u and y along v (width becomes the letter height, height
-- the letter width), shifted by the loop-anchor offset above.
local function hole_rect(r)
  local lx, ly, lw, lh = r[1], r[2], r[3], r[4]
  return {
    x = (H / 2 - (ly + lh)) - off_u, -- min u (hole's top edge), cap-relative
    y = (lx - W / 2) - off_v,        -- min v (hole's left edge), cap-relative
    width = lh,                      -- u extent = letter height
    height = lw,                     -- v extent = letter width
  }
end

bearcad.new()

-- Trace the outline with the Line tool: one segment per edge, each starting where the
-- previous one ended (the first line auto-enters a ground-plane sketch).
local n = #pts
for i = 1, n do
  local a = pts[i]
  local b = pts[i % n + 1]
  bearcad.line{ x = u(a), y = v(a), x1 = u(b), y1 = v(b) }
end

-- Close the loop: make each segment's end coincident with the next segment's start.
-- Lines are 0-indexed in the document (line i-1 for the i-th segment drawn).
for i = 0, n - 1 do
  local nxt = (i + 1) % n
  bearcad.select{ kind = "line", index = i, ["end"] = "end" }
  bearcad.select({ kind = "line", index = nxt, ["end"] = "start" }, true)
  bearcad.add_geometric_constraint("coincident")
end

-- Extrude the closed-loop face 12 mm into a solid body (extrusion 0, body 0). `polygon`
-- lists the loop's line indices in order.
bearcad.extrude{ polygon = { 0, 1, 2, 3, 4, 5, 6, 7 }, distance = 12, name = "B" }

-- Open a sketch on the extrusion's top cap and draw the two counter rectangles there
-- (line indices 8..11 and 12..15). The cap shares the ground plane's (u, v) axes, shifted
-- up to z = 12, so the same hole_rect mapping lines them up over the two bowls.
bearcad.begin_sketch{
  kind = "extrude_cap",
  extrusion = 0,
  profile = "polygon",
  profile_lines = { 0, 1, 2, 3, 4, 5, 6, 7 },
  top = true,
}
local up = hole_rect(upper_hole)
bearcad.rect{ x = up.x, y = up.y, width = up.width, height = up.height }
local lo = hole_rect(lower_hole)
bearcad.rect{ x = lo.x, y = lo.y, width = lo.width, height = lo.height }

-- Cut each rectangle straight down through the 12 mm body (negative distance = into the
-- solid, opposite the cap's outward normal). `body = "cut"` subtracts it from body 0 via
-- an OCCT boolean, punching a real hole (#35). Two cuts -> the two counters of a B.
bearcad.extrude{ polygon = { 8, 9, 10, 11 }, distance = -13, body = "cut" }
bearcad.extrude{ polygon = { 12, 13, 14, 15 }, distance = -13, body = "cut" }

-- Leave the sketch and hide the ground construction plane so the render is a clean solid
-- against the background (no dimension overlay, no plane fill behind the letter). Also drop
-- the selection and hide the cap sketch (index 1) so its hole outlines don't overlay the
-- solid — the counters are real cuts in the body, not the sketch rectangles.
bearcad.exit_sketch()
bearcad.clear_selection()
bearcad.set_visible({ kind = "sketch", index = 1 }, "hide")
bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")

-- Orthographic top view: looking straight down the extrusion (+Z) axis shows the whole
-- letter undistorted. A fixed preset, so the framing is deterministic.
bearcad.ui.view("ortho")
bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.wheel(12) -- zoom in so the letter fills more of the frame
bearcad.ui.wait(2)
bearcad.ui.screenshot(out)

bearcad.quit()
