-- Documentation screenshot: the Move tool.
--
-- A box moved 30 mm along X and rotated 30 degrees about Z — the original lives on as a
-- shadow body; the moved copy is a real body.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/move.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

local sides = bearcad.rect{ x = 0, y = 0, width = 20, height = 14, name = "Block" }
bearcad.exit_sketch()
local box = bearcad.extrude{ profiles = sides, distance = 8, name = "Block" }

bearcad.move_bodies{ bodies = {box}, x = "30", rz = "30", name = "Shifted" }

-- Hide the three datum planes a new document opens with.
bearcad.set_visible({ kind = "plane" }, false)
-- Hide the ground grid too for a clean background (#579).
bearcad.ui.ground("off")
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_left_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)
-- The document behind this picture, so the docs page can link the screenshot into
-- the web app with `?open=` pointing here (#1049 pattern, from joint-kinds.lua).
bearcad.save((out:gsub("%.png$", ".bearcad.json")))

bearcad.quit()
