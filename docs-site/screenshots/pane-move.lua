-- Documentation screenshot: the Move tool's Context pane, in both Translate modes (#672).
--
-- The docs page annotates these shots field by field, so what matters is that every
-- control the tool offers is on screen: the body picker, the mode dropdown, and the
-- inputs each mode brings with it. A single body is selected so the pane shows the
-- picked-something state rather than an empty tool.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". Two PNGs, one per mode.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-move"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ width = 20, height = 14, name = "Block" }
bearcad.exit_sketch()
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 8, name = "Block" }

bearcad.ui.tool("move")
bearcad.select({ kind = "body", index = 0 })
bearcad.ui.wait(5)
-- Snap is the mode the tool starts in.
bearcad.ui.screenshot(out .. "-snap.png", "context")

-- M cycles to Free, which swaps the point pickers for X/Y/Z expressions.
bearcad.ui.key("m")
bearcad.ui.wait(5)
bearcad.ui.screenshot(out .. "-free.png", "context")

bearcad.quit()
