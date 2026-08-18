-- Documentation screenshot: the Chamfer tool's Context pane with help mode on (#672).
--
-- Chamfer asks for different things in a sketch than on a solid, so both are captured:
-- in a sketch it collects corners, and on a body it collects edges. Neither carries the
-- distance — that is typed in the viewport once something is picked.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". Two PNGs.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-chamfer"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.rect{ width = 40, height = 30, name = "Profile" }

-- In a sketch: one corner picked, so the picker shows a filled count.
bearcad.ui.tool("chamfer")
bearcad.select({ kind = "line", index = 1, ["end"] = "end" })
bearcad.ui.wait(6)
bearcad.ui.screenshot(out .. "-sketch.png", "context")

-- On a solid: the edge set, empty as the tool starts.
bearcad.ui.tool("select")
bearcad.clear_selection()
bearcad.extrude{ polygon = { 0, 1, 2, 3 }, distance = 20, name = "Block" }
bearcad.exit_sketch()
bearcad.clear_selection()
bearcad.ui.tool("chamfer")
bearcad.ui.wait(6)
bearcad.ui.screenshot(out .. "-body.png", "context")

bearcad.quit()
