-- #975: the world axes are pickable — a Revolve axis, a Repeat path, a plane anchor all take
-- one — but `collect_pick_candidates` never offered them, so the Exploder could not fan the
-- very thing the armed picker was asking for. Everything a picker can take belongs in the crowd.
bearcad.new()
-- A profile straddling the X axis, so the axis runs under the framed view.
bearcad.rect{ x = 5, y = -5, width = 20, height = 10 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
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
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

local function fan_kinds(x, y)
  bearcad.ui.move_ground(x, y)
  bearcad.ui.wait(3)
  bearcad.ui.key("space")
  bearcad.ui.wait(5)
  local seen = {}
  for _, leaf in ipairs(bearcad.ui.exploder()) do seen[leaf.kind] = true end
  bearcad.ui.key("escape")
  bearcad.ui.wait(4)
  return seen
end

-- The Select tool takes everything, so the axis under the cursor is one of its leaves.
bearcad.ui.tool("select")
bearcad.ui.wait(5)
assert(fan_kinds(15, 0)["axis"], "Select's fan should offer the axis under the cursor")

-- The case from the report: with the Revolve tool's Axis picker armed, the fan over the X axis
-- must offer it. The picker takes straight references and nothing else, so the profile the
-- cursor is also inside is correctly absent.
bearcad.ui.tool("revolve")
bearcad.ui.wait(5)
bearcad.ui.click_ground(15, -2)
bearcad.ui.wait(5)
assert(#picker("Profile").items == 1, "the click should take the profile")
assert(picker("Axis").focused, "so the Axis picker is armed")

local seen = fan_kinds(15, 0)
assert(seen["axis"], "the armed Axis picker takes the axis, so the fan must offer it")
assert(not seen["face"], "and not the profile it cannot take")

print("ok: the exploder offers the world axes the armed picker can take")
bearcad.quit()
