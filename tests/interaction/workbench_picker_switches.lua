-- Interaction regression (#1686): the toolbar's workbench picker names where you are, and
-- opening a cross-section view puts you on the View workbench (the plane tool arrives with
-- #1687, so the bar is Select plus the way back).
bearcad.new()
bearcad.ui.tool("select")
assert(bearcad.ui.workbench() == "model", "a fresh document is on the model")

bearcad.rect{ width = 30, height = 20 }
assert(bearcad.ui.workbench() == "sketch", "drawing opens a sketch")
bearcad.exit_sketch()
assert(bearcad.ui.workbench() == "model", "and leaving it comes back")

bearcad.cross_section{ name = "Front half" }
bearcad.ui.wait(5)
assert(bearcad.ui.workbench() == "view", "a new view opens the View workbench")
-- The View bar carries only its own tools: Select and the cutting-plane tool (#1687).
local tools = bearcad.ui.toolbar_tools()
assert(#tools == 2 and tools[1] == "select" and tools[2] == "section_plane",
  "the View bar is Select plus the cutting-plane tool, got " .. table.concat(tools, ", "))

bearcad.ui.workbench("model")
bearcad.ui.wait(3)
assert(bearcad.ui.workbench() == "model", "the picker leaves the workbench")
assert(#bearcad.ui.toolbar_tools() > 5, "and the modeling bar is back")

bearcad.ui.workbench("view")
bearcad.ui.wait(3)
assert(bearcad.ui.workbench() == "view", "and returns to the view")

print("ok: the workbench picker names and switches the workbench")
bearcad.quit()
