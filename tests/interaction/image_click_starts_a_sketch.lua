-- #1588: a face-click tool starts a sketch on an imported image, including the
-- part of the image that is not sitting on a construction-plane display quad.
bearcad.new()
bearcad.import_image("rectangle_preview.png")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
-- 410×1144 px at 1 px = 1 mm, centered on the origin. Click in the -X/-Y quadrant,
-- well clear of the 5..105 datum-plane quads and the world axes.
bearcad.ui.camera{ target = {-80, -200, 0}, distance = 2800 }
bearcad.ui.wait(10)

local seen = {}
for _, row in ipairs(bearcad.debug.tool_table()) do
  if row.opens_sketch and not seen[row.tool] then
    seen[row.tool] = true
    bearcad.new()
    bearcad.import_image("rectangle_preview.png")
    bearcad.ui.pane("elements", "hide")
    bearcad.ui.pane("context", "hide")
    bearcad.ui.pane("parameters", "hide")
    bearcad.ui.auto_zoom(false)
    bearcad.ui.ground("off")
    bearcad.ui.view("top")
    bearcad.ui.wait(3)
    bearcad.ui.camera{ target = {-80, -200, 0}, distance = 2800 }
    bearcad.ui.wait(3)
    bearcad.ui.tool(row.tool)
    bearcad.ui.wait(3)
    bearcad.ui.move_ground(-80, -200)
    bearcad.ui.wait(5)
    local h = bearcad.ui.hovered()
    assert(h and h.kind == "image",
      row.tool .. " should hover the image, got " .. (h and h.kind or "nothing"))
    bearcad.ui.click_ground(-80, -200)
    bearcad.ui.wait(8)
    local live = bearcad.debug.tool_row()
    assert(live.space == "sketch",
      row.tool .. " image click should start a sketch, space=" .. tostring(live.space)
        .. " status=" .. bearcad.status())
    assert(bearcad.count("sketch") >= 1,
      row.tool .. " image click should create a sketch")
    if row.tool == "sketch" then
      assert(live.tool == "select",
        "Sketch tool resets to Select, got " .. live.tool)
    else
      assert(live.tool == row.tool,
        row.tool .. " should survive into the sketch, got " .. live.tool)
    end
  end
end

assert(seen["sketch"] and seen["line"],
  "Sketch and Line must be face-click tools")

print("ok: face-click tools start a sketch on an imported image")
bearcad.quit()
