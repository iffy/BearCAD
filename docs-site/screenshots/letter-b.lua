-- Documentation screenshot: a capital letter "B" drawn with the Line tool, extruded into a
-- solid, with real cut counters — everything curved with bezier (#54) for a typographic B.
--
-- Outer silhouette: a straight left spine, then two rounded lobes formed by bezier curves
-- whose outer edges meet at a single waist point on the right (no notch). It's one closed
-- loop (each segment's end made coincident with the next segment's start), extruded 12 mm.
--
-- The two counters are punched afterwards as solid subtractions (#35): a sketch on the top
-- cap holds a "D"-shaped profile for each bowl — a flat left edge (toward the spine) and a
-- rounded right edge (two bezier quarter-arcs) — extruded down through the body with
-- `body = "cut"` (an OCCT boolean subtract) to carve a real D-counter. (Needs the OCCT
-- kernel, on by default.)
--
-- Captured from a fixed orthographic top view for a clear, deterministic render (SPEC §8).
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh), else ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/letter-b.png"

-- Letter bounding box (letter coords: x = width rightward, y = height upward). The bezier
-- lobes bulge out to ~x=56.
local H, W = 72, 56

-- Outline segments, clockwise from the bottom-left, in letter coordinates. Each is
-- { x0, y0, x1, y1 } and may carry `bez = { c0x, c0y, c1x, c1y }`. The upper and lower lobe
-- curves both end/start at the single waist point (18, 36) — so the two outside curves meet
-- at one point instead of a flat notch.
local segs = {
  { 0, 0, 0, 72 },                                  -- spine, straight (bottom-left -> top-left)
  { 0, 72, 18, 36, bez = { 54, 72, 50, 42 } },      -- upper lobe -> waist point
  { 18, 36, 14, 0, bez = { 50, 30, 58, -2 } },      -- waist point -> lower lobe (bulges wider)
  { 14, 0, 0, 0 },                                  -- bottom edge back to the spine foot
}

-- The two counters as "D" shapes in letter coords: a flat left edge at x=`lx` spanning
-- `cy` ± `hh`, and a rounded right edge bulging to x=`lx + w`. Sit inside the two bowls.
local upper_d = { lx = 10, cy = 54, w = 24, hh = 9 }
local lower_d = { lx = 10, cy = 16, w = 26, hh = 9 }

-- The top view shows sketch +x screen-right and +y screen-up (#100), so letter coordinates
-- map straight through — just centre the letter on the origin.
local function u(x) return x - W / 2 end
local function v(y) return y - H / 2 end

bearcad.new()
-- Hide the side panes so the captured viewport is landscape (#150).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")


-- Trace the outline with the Line tool (the first segment auto-enters a ground-plane sketch).
local n = #segs
for i = 1, n do
  local s = segs[i]
  local line = { x = u(s[1]), y = v(s[2]), x1 = u(s[3]), y1 = v(s[4]) }
  if s.bez then
    line.bezier = { { u(s.bez[1]), v(s.bez[2]) }, { u(s.bez[3]), v(s.bez[4]) } }
  end
  bearcad.line(line)
end

-- Close the outline loop.
for i = 0, n - 1 do
  local nxt = (i + 1) % n
  bearcad.select{ kind = "line", index = i, ["end"] = "end" }
  bearcad.select({ kind = "line", index = nxt, ["end"] = "start" }, true)
  bearcad.add_geometric_constraint("coincident")
end

-- Extrude the closed-loop face 12 mm into a solid body (extrusion 0, body 0).
local outline = { 0, 1, 2, 3 }
bearcad.extrude{ polygon = outline, distance = 12, name = "B" }

-- The cap sketch shares the ground plane's (u, v) axes but is anchored at the profile loop's
-- first vertex (the outline's (0, 0)), so a letter point (lx, ly) is already cap-local.
local function cap(lx, ly) return lx, ly end

-- Draw one "D" counter (flat left + rounded right) as three Line-tool segments on the current
-- sketch, starting at line index `first`; kappa is the standard circle/ellipse control offset.
local KAPPA = 0.5522847498307936
local function draw_d_counter(d, first)
  local lx, cy, w, hh = d.lx, d.cy, d.w, d.hh
  local ty, by, rx = cy + hh, cy - hh, lx + w
  local kx, ky = KAPPA * w, KAPPA * hh
  -- (start, end[, ctrl-near-start, ctrl-near-end]) in letter coords.
  local parts = {
    { { lx, by }, { lx, ty } },                                              -- flat left edge
    { { lx, ty }, { rx, cy }, { lx + kx, ty }, { rx, cy + ky } },            -- top-right arc
    { { rx, cy }, { lx, by }, { rx, cy - ky }, { lx + kx, by } },            -- bottom-right arc
  }
  for _, p in ipairs(parts) do
    local x0, y0 = cap(p[1][1], p[1][2])
    local x1, y1 = cap(p[2][1], p[2][2])
    local line = { x = x0, y = y0, x1 = x1, y1 = y1 }
    if p[3] then
      local c0x, c0y = cap(p[3][1], p[3][2])
      local c1x, c1y = cap(p[4][1], p[4][2])
      line.bezier = { { c0x, c0y }, { c1x, c1y } }
    end
    bearcad.line(line)
  end
  -- Close this D loop (3 segments share endpoints).
  for k = 0, 2 do
    local i = first + k
    local nxt = first + (k + 1) % 3
    bearcad.select{ kind = "line", index = i, ["end"] = "end" }
    bearcad.select({ kind = "line", index = nxt, ["end"] = "start" }, true)
    bearcad.add_geometric_constraint("coincident")
  end
end

-- Open a sketch on the top cap, draw the two D counters (lines 4..6 and 7..9), then cut each
-- straight down through the 12 mm body — an OCCT boolean subtract per counter.
bearcad.begin_sketch{
  kind = "extrude_cap",
  extrusion = 0,
  profile = "polygon",
  profile_lines = outline,
  top = true,
}
draw_d_counter(upper_d, 4)
draw_d_counter(lower_d, 7)

bearcad.extrude{ polygon = { 4, 5, 6 }, distance = -13, body = "cut" }
bearcad.extrude{ polygon = { 7, 8, 9 }, distance = -13, body = "cut" }

-- Clean render: leave the sketch, hide the cap sketch's outlines and the ground plane.
bearcad.exit_sketch()
bearcad.clear_selection()
bearcad.set_visible({ kind = "sketch", index = 1 }, "hide")
bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")

-- Hide the ground grid too for a clean background (#579).
bearcad.ui.ground("off")
-- Orthographic top view, fixed framing.
bearcad.ui.view("ortho")
bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.wheel(12)
bearcad.ui.wait(2)
bearcad.ui.screenshot(out)

bearcad.quit()
