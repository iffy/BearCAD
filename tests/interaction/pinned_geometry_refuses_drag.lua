-- Interaction regression (#459): genuinely rigid geometry (dimensioned AND pinned)
-- refuses dragging — the block exists, it just must not overreach.
bearcad.new()
bearcad.rect{ width = 10, height = 10 }
bearcad.select{ kind = "line", index = 0, ["end"] = "start" }
bearcad.select({ kind = "origin" }, true)
bearcad.add_geometric_constraint("coincident")
bearcad.clear_selection()
local ok, err = pcall(function()
  bearcad.ui.drag_vertex({ kind = "line", index = 0, ["end"] = "end" }, 13, 0)
end)
assert(not ok, "a pinned dimensioned rect corner must refuse to drag")
assert(tostring(err):find("constrained"), "unexpected error: " .. tostring(err))
local _, _, x1, _ = bearcad.line_endpoints(0)
assert(math.abs(x1 - 10) < 1e-3, "corner must not have moved")
print("ok: rigid geometry refuses")
bearcad.quit()
