-- Documentation screenshot: the Loft tool.
--
-- Blends a wide circle on the ground plane into a small circle 15 mm up — the classic
-- horn/funnel loft.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/loft.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

local base = bearcad.circle{ x = 0, y = 0, r = 10 }
local up = bearcad.plane{ offset = 15 }
bearcad.begin_sketch{ kind = "plane", index = up }
local top = bearcad.circle{ x = 3, y = 0, r = 4 }
bearcad.exit_sketch()
bearcad.loft{ profiles = {base, top}, name = "Horn" }

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
