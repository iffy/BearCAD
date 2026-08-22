-- #1640: the technical-drawing walkthrough runs on the drawing workbench — Bear's bubble
-- follows the user onto the sheet, and the assists leave a real page behind.
bearcad.ui.tool("select")
bearcad.ui.tutorial("drawing")
bearcad.ui.wait(8)
assert(bearcad.ui.tutorial_step() == 0, "the drawing tutorial starts on its intro")
assert(bearcad.count("body") >= 1, "it seeds a bracket to draw")

-- The steps that point into the Context pane (the view's orientation bear, its Style row)
-- only ring anything if the tutorial overlay is drawn on the drawing workbench too.
local guard = 0
local orb_on_the_pane = nil
while bearcad.ui.tutorial_step() ~= nil do
  guard = guard + 1
  assert(guard < 60, "the walkthrough should finish")
  bearcad.ui.wait(5)
  local orb = bearcad.ui.tutorial_orb()
  if orb and bearcad.count("drawing") == 1 then
    orb_on_the_pane = orb
  end
  bearcad.ui.tutorial_assist()
  bearcad.ui.wait(3)
  if bearcad.ui.tutorial_step() ~= nil then
    bearcad.ui.tutorial_next()
    bearcad.ui.wait(3)
  end
end

assert(bearcad.count("drawing") == 1, "the walkthrough leaves one drawing")
local views = bearcad.drawing_views(0)
assert(#views == 4, "front + top + side + three-quarter, got " .. #views)
assert(views[1].orientation == "Front", "the base view is the front")
local aligned, shaded = 0, nil
for _, v in ipairs(views) do
  if v.aligned_to == 0 then
    aligned = aligned + 1
    assert(v.align_lines, v.orientation .. " should show its projection lines")
  end
  if v.style == "Shaded" then shaded = v.orientation end
end
assert(aligned == 2, "two views aligned to the front, got " .. aligned)
assert(shaded and shaded:find("-"),
  "the at-an-angle view should be shaded, got " .. tostring(shaded))

assert(orb_on_the_pane,
  "the Context-pane steps should ring something while the page is open")
local context = bearcad.ui.pane_rect("context")
assert(type(context) == "table", "the Context pane is shown")
assert(orb_on_the_pane.x >= context.x - 8,
  string.format("the orb should sit on the Context pane, x=%.0f pane starts %.0f",
    orb_on_the_pane.x, context.x))

print("ok: the technical-drawing tutorial runs on the drawing workbench")
bearcad.quit()
