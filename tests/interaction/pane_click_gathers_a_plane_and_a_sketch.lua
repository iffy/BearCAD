-- #963: the Elements-pane click cascade had a `(SceneElement, Tool)` arm per non-body kind a
-- tool could gather — a plane for Move, a plane/sketch/extrusion for Repeat — each written
-- only there. They go through the tool's picker now, so the pane, the viewport and a script
-- all gather the same things.
bearcad.new()
bearcad.rect{ width = 30, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
-- Hide only the panes the pointer tests need out of the way; the Elements pane is the one
-- being clicked (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

bearcad.ui.tool("repeat")
bearcad.ui.wait(5)
assert(picker("Bodies").focused, "the Repeat tool's Bodies picker is armed")

-- A construction plane is a thing Repeat repeats (#221). Selecting it from the pane should
-- land it in the picker, not in the plain selection.
bearcad.select{ kind = "construction_plane", index = 0 }
bearcad.ui.wait(5)
local bodies = picker("Bodies")
assert(#bodies.items == 1,
  "the plane should land in the Repeat set, got " .. #bodies.items)
assert(bodies.items[1].kind == "construction_plane",
  "and it should be the plane, got " .. bodies.items[1].kind)

-- So is a sketch (#231).
bearcad.select{ kind = "sketch", index = 0 }
bearcad.ui.wait(5)
assert(#picker("Bodies").items == 2,
  "the sketch should join it, got " .. #picker("Bodies").items)

-- Selecting the plane again takes it back out — the picker's own toggle.
bearcad.select{ kind = "construction_plane", index = 0 }
bearcad.ui.wait(5)
assert(#picker("Bodies").items == 1,
  "re-selecting the plane drops it, got " .. #picker("Bodies").items)

print("ok: a pane pick of a plane or a sketch goes through the tool's picker")
bearcad.quit()
