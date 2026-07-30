-- #970: the Repeat tool decided whether a click meant "the path" or "a body" from a flag it
-- read off its own state, with a second copy of the pick→axis mapping to go with it. Which set
-- a click feeds is the armed picker's business now, and the mapping is
-- `SceneElement::as_revolve_axis`.
bearcad.new()
bearcad.rect{ width = 30, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
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
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

bearcad.ui.tool("repeat")
bearcad.ui.wait(5)
assert(picker("Bodies").focused, "with nothing gathered, Bodies is armed")

-- A click on the solid gathers it, and that arms the Path picker: with something to repeat,
-- the path is the next thing to pick.
bearcad.ui.click_ground(15, 10)
bearcad.ui.wait(5)
assert(#picker("Bodies").items == 1,
  "the click should gather the body, got " .. #picker("Bodies").items)
assert(picker("Path").focused, "with a body gathered, the Path picker takes over")

-- The same kind of click — on a body edge — now sets the path instead of toggling the body,
-- because Path is what's armed.
bearcad.ui.click_ground(15, 0)
bearcad.ui.wait(5)
assert(#picker("Path").items == 1,
  "the edge should become the path, got " .. #picker("Path").items)
assert(#picker("Bodies").items == 1, "and the body should still be gathered")

-- With a path set, Bodies is armed again, so a click on the body's edge toggles the body.
assert(picker("Bodies").focused, "with a path set, Bodies takes over again")
bearcad.ui.click_ground(15, 10)
bearcad.ui.wait(5)
assert(#picker("Bodies").items == 0,
  "re-clicking the body drops it, got " .. #picker("Bodies").items)

print("ok: the armed picker decides whether a click is the path or a body")
bearcad.quit()
