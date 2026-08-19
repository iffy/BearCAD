-- #1567: on Combine in Cut/Intersect/Difference, the first Side A pick arms Side B
-- so the next click is the other operand. Adding further Side A bodies (user re-armed
-- Side A) keeps focus on Side A. Union has no Side B, so it stays on Bodies.
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
    bearcad.ui.key("escape")
    bearcad.ui.wait(3)
  end
end

for _, mode in ipairs({ "cut", "intersect", "difference" }) do
  ensure_combine_empty()
  bearcad.ui.tool_mode(mode)
  bearcad.ui.wait(5)
  assert(picker("Side A").focused, mode .. ": Side A starts focused")
  assert(not picker("Side B").focused, mode .. ": Side B starts unarmed")

  -- First body → Side B arms.
  bearcad.select{ kind = "body", index = 0 }
  bearcad.ui.wait(5)
  local pa = picker("Side A")
  local pb = picker("Side B")
  assert(pa and #pa.items == 1, mode .. ": first body in Side A")
  assert(pb.focused, mode .. ": first Side A pick arms Side B")
  assert(not pa.focused, mode .. ": Side A must not keep focus")

  -- Re-arm Side A and add a second body — stay on Side A.
  bearcad.ui.picker_focus("Side A")
  bearcad.ui.wait(5)
  assert(picker("Side A").focused, mode .. ": Side A re-armed")
  bearcad.select{ kind = "body", index = 1 }
  bearcad.ui.wait(5)
  pa = picker("Side A")
  pb = picker("Side B")
  assert(#pa.items == 2, mode .. ": second body in Side A")
  assert(pa.focused, mode .. ": a non-first Side A pick must keep focus on Side A")
  assert(not pb.focused, mode .. ": Side B stays unarmed")
end

-- Union: one picker, first pick stays on it.
ensure_combine_empty()
bearcad.ui.wait(5)
assert(side_a().focused, "Bodies starts focused")
bearcad.select{ kind = "body", index = 0 }
bearcad.ui.wait(5)
assert(#side_a().items == 1, "first body in Bodies")
assert(side_a().focused, "union first pick stays on Bodies")
assert(not picker("Side B"), "union has no Side B picker")

print("ok: first Combine Side A pick arms Side B; further picks keep Side A")
bearcad.quit()
