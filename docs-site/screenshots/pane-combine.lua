-- Documentation screenshot: the Combine tool's Context pane with help mode on (#672).
--
-- Two shots, because the pickers follow the mode: Combine is one-sided (a single Bodies
-- picker), while a two-sided operation like Cut asks for side A and side B separately.
-- The mode row is an egui widget a scripted click can't reach (#130), so `tool_mode`
-- switches it.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". Two PNGs.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-combine"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

local block_sides = bearcad.rect{ x = 0, y = 0, width = 30, height = 20, name = "Block" }
bearcad.exit_sketch()
local block = bearcad.extrude{ profiles = block_sides, distance = 12, name = "Block" }

bearcad.begin_sketch{ kind = "plane", index = 0 }
local bite_sides = bearcad.rect{ x = 18, y = 6, width = 24, height = 8, name = "Bite" }
bearcad.exit_sketch()
local bite = bearcad.extrude{ profiles = bite_sides, distance = 20, name = "Bite" }

bearcad.clear_selection()
bearcad.ui.tool("combine")
bearcad.select(block)
bearcad.ui.wait(6)
bearcad.ui.screenshot(out .. "-combine.png", "context")

-- Cut: the second body goes on side B, which Combine folds away.
bearcad.ui.tool_mode("cut")
bearcad.select(bite)
bearcad.ui.wait(6)
bearcad.ui.screenshot(out .. "-cut.png", "context")

bearcad.quit()
