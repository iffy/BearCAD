-- #955: the Construction Plane tool's Anchor was the last label-only input. The plane's
-- reference is a *derived* frame — an origin and a direction — so the identity of what was
-- clicked used to be thrown away at pick time and the input had nothing to hold.
bearcad.new()
-- Horizontal line along +X (the normal direction), and a separate segment whose start is
-- the point the plane should pass through — the line+point anchor of #483.
bearcad.line{ x = 0, y = 5, x1 = 30, y1 = 5 }
bearcad.line{ x = 10, y = 20, x1 = 12, y1 = 22 }
bearcad.exit_sketch()
bearcad.ui.tool("construction_plane")
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

local function anchor()
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == "Anchor" then return p end
  end
  return nil
end

local a = anchor()
assert(a, "the plane tool should register an Anchor picker")
assert(a.focused, "the anchor is the only thing this tool picks, so it is armed")
assert(#a.items == 0, "starting empty, got " .. #a.items)
-- A plane can be anchored on geometry, never on a whole body.
local takes = {}
for _, k in ipairs(a.accepts) do takes[k] = true end
assert(takes["line"] and takes["vertex"] and takes["face"],
  "the anchor takes lines, vertices and faces")
assert(not takes["body"], "but not a whole body")

-- Pick the line: one row, and the picker holds it.
bearcad.ui.click_ground(15, 5)
bearcad.ui.wait(8)
a = anchor()
assert(#a.items == 1, "the picked line should be the one anchor row, got " .. #a.items)
assert(a.items[1].kind == "line", "and it should be the line, got " .. a.items[1].kind)

-- The other line's start point completes the set: both halves are held, point first.
bearcad.ui.click_ground(10, 20)
bearcad.ui.wait(8)
a = anchor()
assert(#a.items == 2, "line + point is a two-row anchor, got " .. #a.items)
assert(a.items[1].kind == "point",
  "the point leads the rows, got " .. a.items[1].kind)
assert(a.items[2].kind == "line", "then the line, got " .. a.items[2].kind)

print("ok: the plane's Anchor input holds the elements it was picked from")
bearcad.quit()
