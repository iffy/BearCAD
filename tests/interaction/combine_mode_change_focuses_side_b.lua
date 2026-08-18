-- #1533: on Combine, if Side A already has a body, changing the mode to Cut /
-- Intersect / Difference focuses the Side B picker immediately — not Side A.
bearcad.new()
bearcad.rect{ width = 30, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.begin_sketch{ kind = "plane", index = 0 }
bearcad.rect{ x = 40, y = 0, width = 30, height = 20 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 10 }
bearcad.exit_sketch()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

local function side_a()
  return picker("Bodies") or picker("Side A")
end

local function ensure_combine_empty()
  bearcad.ui.tool("combine")
  bearcad.ui.tool_mode("combine")
  bearcad.ui.wait(3)
  local a = side_a()
  if a and #a.items > 0 then
    -- First Esc empties picks and stays on Combine (#1484).
    bearcad.ui.key("escape")
    bearcad.ui.wait(3)
  end
end

for _, mode in ipairs({ "cut", "intersect", "difference" }) do
  ensure_combine_empty()
  bearcad.select{ kind = "body", index = 0 }
  bearcad.ui.wait(3)
  local a = side_a()
  assert(a and #a.items == 1, mode .. ": Side A should hold the selected body")
  assert(a.focused, mode .. ": Bodies/Side A starts focused")
  bearcad.ui.tool_mode(mode)
  bearcad.ui.wait(5)
  local pa = picker("Side A")
  local pb = picker("Side B")
  assert(pa, mode .. ": Side A picker should appear")
  assert(pb, mode .. ": Side B picker should appear")
  assert(#pa.items == 1, mode .. ": Side A should keep the body")
  assert(pb.focused, mode .. ": Side B should take focus after a Side A pick")
  assert(not pa.focused, mode .. ": Side A must not keep focus")
end

-- Nothing on A: stay on Side A.
ensure_combine_empty()
bearcad.ui.tool_mode("cut")
bearcad.ui.wait(5)
assert(picker("Side A").focused, "empty Side A stays focused when switching to Cut")
assert(not picker("Side B").focused, "empty Side A must not jump to Side B")

print("ok: Combine mode change after a Side A pick focuses Side B")
bearcad.quit()
