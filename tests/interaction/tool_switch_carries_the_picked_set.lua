-- #956: switching tools hands the outgoing focused picker's items to the new tool's primary
-- picker, keeping whatever the new one can accept. Gathering bodies in one tool and then
-- realising you wanted a different tool shouldn't mean picking them all again.
bearcad.new()
bearcad.rect{ width = 30, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.rect{ width = 30, height = 30, x = 50 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 10 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")

local function picker(name)
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

local function indices(name)
  local out = {}
  for _, item in ipairs(picker(name).items) do out[#out + 1] = item.index end
  table.sort(out)
  return table.concat(out, ",")
end

-- Gather two bodies in the Combine tool's side A.
bearcad.ui.tool("combine")
bearcad.ui.wait(5)
bearcad.select{ kind = "body", index = 0 }
bearcad.select{ kind = "body", index = 1 }
bearcad.ui.wait(5)
assert(indices("Bodies") == "0,1",
  "Combine should hold both bodies, got " .. indices("Bodies"))

-- Switch to Move: the same bodies are what you want to move.
bearcad.ui.tool("move")
bearcad.ui.wait(5)
assert(indices("Bodies") == "0,1",
  "Move should inherit Combine's bodies, got " .. indices("Bodies"))

-- And on to Repeat, which takes bodies too.
bearcad.ui.tool("repeat")
bearcad.ui.wait(5)
assert(indices("Bodies") == "0,1",
  "Repeat should inherit them too, got " .. indices("Bodies"))

-- The Revolve tool's primary picker takes faces, not bodies, so nothing carries over —
-- an invalid item is dropped rather than forced in.
bearcad.ui.tool("revolve")
bearcad.ui.wait(5)
assert(#picker("Profile").items == 0,
  "a face picker should not inherit bodies, got " .. #picker("Profile").items)

print("ok: a tool switch carries the picked set, dropping what the new picker refuses")
bearcad.quit()
