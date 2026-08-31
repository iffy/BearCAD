-- #1625: opening the AI pane then creating a rectangle must not log the egui
-- "Widget rect … changed id between passes" multipass warning. Opening a sketch
-- used to mount a fresh Snapping row above the Default-units block, shifting its
-- comboboxes onto a rect a sibling row occupied the pass before.
bearcad.new()
bearcad.ui.tool("select")
bearcad.ui.pane("ai", "show")
bearcad.ui.wait(5)
bearcad.rect{ width = 80, height = 50 }
bearcad.ui.wait(5)

local w = bearcad.debug.widget_id_warnings()
assert(w == 0,
  "AI pane + rect must not change widget ids between passes, got " .. tostring(w))

print("ok: rect with the AI pane open keeps widget ids stable")
bearcad.quit()