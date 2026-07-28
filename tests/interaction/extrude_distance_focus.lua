-- #880: clicking a face with the Extrude tool leaves the distance field holding the
-- keyboard — an expression typed straight away sets the depth, with no click on the field.
bearcad.new()
bearcad.parameter("add", "deep", "12mm")
bearcad.rect{ width = 80, height = 40 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
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
bearcad.ui.type("deep")
bearcad.ui.wait(4)
bearcad.ui.key("enter")
bearcad.ui.wait(10)

assert(bearcad.count("extrusion") == 1,
  "expected the typed depth to commit one extrusion, got " .. bearcad.count("extrusion"))
assert(bearcad.status():find("12"),
  "expected a 12 mm extrusion from `deep`, got: " .. bearcad.status())

print("ok: a picked extrude face takes typing without a click on its field")
bearcad.quit()
