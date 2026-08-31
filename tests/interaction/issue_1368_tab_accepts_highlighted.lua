-- #1368: Tab accepts the *currently-highlighted* completion (the arrow-navigated item),
-- not just the top match. `deep`=12mm, `deeper`=25mm; `dee` → ArrowDown picks `deeper` →
-- Tab accepts it → Enter commits a 25mm extrusion.
bearcad.new()
bearcad.add_parameter("deep", "12mm")
bearcad.add_parameter("deeper", "25mm")
bearcad.rect{ width = 80, height = 40 }
bearcad.exit_sketch()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {40, 20, 0}, distance = 260 }
bearcad.ui.wait(5)
bearcad.ui.tool("extrude")
bearcad.ui.wait(3)

bearcad.ui.click_ground(40, 20)
bearcad.ui.wait(6)
bearcad.ui.type("dee")
bearcad.ui.wait(4)
-- Move the highlight down to `deeper`, then accept it with Tab.
bearcad.ui.key("down")
bearcad.ui.wait(2)
bearcad.ui.key("tab")
bearcad.ui.wait(4)
bearcad.ui.key("enter")
bearcad.ui.wait(10)

assert(bearcad.count("extrusion") == 1,
  "expected the typed depth to commit one extrusion, got " .. bearcad.count("extrusion"))
assert(bearcad.status():find("25"),
  "expected a 25 mm extrusion from the highlighted `deeper`, got: " .. bearcad.status())

print("ok: Tab accepts the currently-highlighted completion candidate")
bearcad.quit()