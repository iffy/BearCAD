-- Documentation screenshot: the Move tool's Context pane with help mode on (#672).
--
-- Help mode is the documentation for these controls: it draws a note beside each row
-- saying what that row wants, and a pane capture widens to include them. One shot per
-- Translate mode, since the two ask for different things.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". Two PNGs, one per mode.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-move"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.rect{ width = 20, height = 14, name = "Block" }
bearcad.exit_sketch()
bearcad.extrude{ polygon = { 0, 1, 2, 3 }, distance = 8, name = "Block" }

bearcad.ui.tool("move")
bearcad.select({ kind = "body", index = 0 })
bearcad.ui.wait(6)
-- Snap is the mode the tool starts in.
bearcad.ui.screenshot(out .. "-snap.png", "context")

bearcad.ui.tool_mode("free")
bearcad.ui.wait(6)
bearcad.ui.screenshot(out .. "-free.png", "context")

bearcad.quit()
