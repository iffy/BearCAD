-- Interaction regression (#991): every joint kind but Rigid joins exactly two parts, and which
-- one moves is the whole meaning of the joint — so they are picked as two named slots, the
-- **mobile** part first and the **fixed** one holding it second, rather than as one list plus a
-- "swap which side is held" button. Rigid keeps the plain list: it joins any number and nothing
-- moves.
bearcad.new()
bearcad.rect{ width = 30, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.plane{ origin = {60, 0, 0}, normal = {0, 0, 1} }
bearcad.begin_sketch("construction_plane", 3)
bearcad.rect{ width = 20, height = 20 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 10 }
bearcad.exit_sketch()
bearcad.clear_selection()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {45, 20, 0}, distance = 320 }
bearcad.ui.wait(5)
bearcad.ui.tool("joint")
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
end

-- A slider names both sides; nothing is picked yet, so Mobile wears the ring.
bearcad.begin_joint{ parts = {}, kind = "slider" }
bearcad.ui.wait(8)
assert(picker("Mobile"), "a slider should offer a Mobile picker")
assert(picker("Fixed"), "and a Fixed one")
assert(not picker("Parts"), "the two slots replace the plain Parts list")
assert(picker("Mobile").focused, "the mobile part is picked first")
assert(not picker("Fixed").focused, "one ring at a time")

-- The first click fills Mobile and hands the ring to Fixed.
bearcad.ui.click_ground(15, 15)
bearcad.ui.wait(8)
assert(#picker("Mobile").items == 1,
  "the first click should fill Mobile, got " .. #picker("Mobile").items)
assert(#picker("Fixed").items == 0, "and leave Fixed empty")
assert(picker("Fixed").focused, "the ring moves on to Fixed")

-- The second click fills Fixed.
bearcad.ui.click_ground(70, 10)
bearcad.ui.wait(8)
assert(#picker("Mobile").items == 1, "the mobile part stays put")
assert(#picker("Fixed").items == 1,
  "the second click should fill Fixed, got " .. #picker("Fixed").items)

-- Rigid joins any number of parts and none of them moves, so it keeps the plain list.
bearcad.begin_joint{ parts = {1, 0}, kind = "rigid" }
bearcad.ui.wait(8)
assert(picker("Parts"), "a rigid group keeps its Parts list")
assert(not picker("Mobile") and not picker("Fixed"),
  "a rigid group has no moving side to name")

print("ok: a two-sided joint picks its mobile part, then the part holding it")
bearcad.quit()
