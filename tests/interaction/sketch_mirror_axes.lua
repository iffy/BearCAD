-- #1538: 2D Mirror's first pick is a mirror line. Global and local (origin) axes
-- must hover-highlight as a valid choice and appear in the Exploder's fan.
bearcad.new()
-- A square well off the axes so a hover on +X is not a sketch line.
bearcad.rect{ x = 20, y = 20, width = 20, height = 20 }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(30)
bearcad.ui.camera{ target = {20, 10, 0}, distance = 220 }
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

local function is_axis(kind)
  return kind == "axis" or kind == "face_edge"
end

bearcad.ui.tool("mirror")
bearcad.ui.wait(5)
local line_picker = picker("Mirror line")
assert(line_picker and line_picker.focused, "Mirror line is the first pick")

-- Hover the +X axis, clear of the origin and of the square.
bearcad.ui.move_ground(12, 0)
bearcad.ui.wait(5)
local h = bearcad.hovered()
assert(h and is_axis(h.kind),
  "hovering a world/local axis should highlight it as a mirror line, got "
    .. tostring(h and h.kind))

bearcad.ui.key("space")
bearcad.ui.wait(5)
local seen = {}
for _, leaf in ipairs(bearcad.exploder()) do seen[leaf.kind] = true end
bearcad.ui.key("escape")
bearcad.ui.wait(4)
assert(seen["axis"] or seen["face_edge"],
  "the Exploder must offer a global or local axis as a mirror-line choice")

bearcad.ui.click_ground(12, 0)
bearcad.ui.wait(5)
line_picker = picker("Mirror line")
assert(line_picker and #line_picker.items == 1,
  "clicking the axis should set the mirror line, got "
    .. tostring(line_picker and #line_picker.items))
assert(is_axis(line_picker.items[1].kind),
  "the picked mirror line should be an axis, got " .. line_picker.items[1].kind)

local shapes = picker("Shapes")
assert(shapes and shapes.focused, "with an axis set, Shapes takes the next click")
bearcad.ui.click_ground(30, 20)
bearcad.ui.wait(5)
shapes = picker("Shapes")
assert(shapes and #shapes.items >= 1, "a shape on the square should be picked")

local n_before = bearcad.count("line")
bearcad.ui.key("enter")
bearcad.ui.wait(8)
assert(bearcad.count("line") > n_before,
  "committing a mirror across an axis should emit a reflected line")

print("ok: 2D mirror hovers, fans and takes global/local axes")
bearcad.quit()
