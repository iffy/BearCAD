-- Documentation screenshot: a selected unit instance's Context pane with help mode on (#734).
--
-- An imported unit selected, so the pane shows the instance rows: Name, Link, Source,
-- Placement, and Rotation.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-unit.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)
local tmp = os.getenv("TMPDIR") or "/tmp"

-- The part: a small box.
bearcad.new()
local sides = bearcad.rect{ width = 20, height = 12 }
bearcad.extrude{ profiles = sides, distance = 8 }
bearcad.save(tmp .. "/bearcad_docs_bracket.bearcad")

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)
bearcad.save(tmp .. "/bearcad_docs_assembly.bearcad")
bearcad.import_unit{ path = tmp .. "/bearcad_docs_bracket.bearcad", name = "bracket" }
bearcad.select("bracket")

bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")
bearcad.quit()
