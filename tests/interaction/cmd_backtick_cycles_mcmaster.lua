-- Interaction regression (#1477): ⌘` / Ctrl+` cycles every OS window, including
-- the DEV Report issue window and the McMaster-Carr catalog helper. Jumping
-- straight to the catalog (the old #1023 toggle) would skip Report issue.
bearcad.new()
bearcad.ui.tool("select")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.ui.report_issue("show")
bearcad.ui.mcmaster("show", "91290A115")
bearcad.ui.wait(60)
assert(not bearcad.status():find("could not"),
  "the catalog window should open, got: " .. bearcad.status())

local names = bearcad.ui.windows()
local listed = {}
for i = 1, #names do listed[names[i]] = true end
assert(listed["main"], "windows() should include main, got: " .. table.concat(names, ","))
assert(listed["report_issue"],
  "windows() should include report_issue, got: " .. table.concat(names, ","))
assert(listed["mcmaster"],
  "windows() should include mcmaster, got: " .. table.concat(names, ","))

local seen = {}
local start = bearcad.ui.focused_window()
local n = #names
for _ = 1, n do
  seen[bearcad.ui.focused_window()] = true
  bearcad.ui.key("`", { cmd = true })
  bearcad.ui.wait(5)
end
assert(seen["main"], "cycle never landed on main, start=" .. start)
assert(seen["report_issue"],
  "cycle skipped Report issue (jumped to the catalog?), start=" .. start)
assert(seen["mcmaster"], "cycle never landed on the catalog, start=" .. start)
assert(bearcad.ui.focused_window() == start,
  "a full cycle should return to " .. start .. ", got " .. bearcad.ui.focused_window())

bearcad.ui.mcmaster("hide")
bearcad.ui.report_issue("hide")
bearcad.ui.wait(5)

print("ok: ⌘` cycles through main, Report issue, and the McMaster-Carr catalog")
bearcad.quit()
