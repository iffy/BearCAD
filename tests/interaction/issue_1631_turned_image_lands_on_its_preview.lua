-- #1631: a Free move that turns a tracing image must land exactly where the preview drew
-- it, and stay put when a later move recomputes the document. The turn pivots about the
-- image's pristine pose, so the pivot can't drift along with the move it belongs to.
bearcad.new()
bearcad.import_image("rectangle_preview.png")
bearcad.ui.tool("select")
bearcad.ui.wait(3)

-- The live preview of a turn-and-slide, as the viewport draws it.
bearcad.ui.begin_move{ images = {0}, x = -56.6, y = 73.9, rz = -90 }
bearcad.ui.wait(3)
local preview = bearcad.image_corners(0)

-- Drop the armed preview, then commit the very same move.
bearcad.ui.key("Escape")
bearcad.ui.wait(3)
bearcad.move_bodies{ images = {0}, x = -56.6, y = 73.9, rz = -90 }
bearcad.ui.wait(3)
local committed = bearcad.image_corners(0)
for i = 1, 4 do
  for axis = 1, 3 do
    assert(math.abs(committed[i][axis] - preview[i][axis]) < 0.01,
      string.format("commit should land on the preview, corner %d axis %d: %.3f vs %.3f",
        i, axis, committed[i][axis], preview[i][axis]))
  end
end

-- A second move recomputes every moved image; the first move's turn must not walk away.
bearcad.move_bodies{ images = {0}, x = 10 }
bearcad.ui.wait(3)
local settled = bearcad.image_corners(0)
for i = 1, 4 do
  local dx = settled[i][1] - committed[i][1]
  local dy = settled[i][2] - committed[i][2]
  local dz = settled[i][3] - committed[i][3]
  assert(math.abs(dx - 10) < 0.01 and math.abs(dy) < 0.01 and math.abs(dz) < 0.01,
    string.format("a later +10 mm slide should be exactly that, corner %d moved (%.3f, %.3f, %.3f)",
      i, dx, dy, dz))
end

print("ok: a turned image lands on its preview and stays there")
bearcad.quit()
