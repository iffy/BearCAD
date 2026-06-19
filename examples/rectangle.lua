-- Example Lua script — sketch on the default XY plane, draw a rectangle, screenshot.
-- Run: cargo run -- --script examples/rectangle.lua --exit

le3.import()

new()
begin_sketch("construction_plane", 0)
tool("rectangle")

-- Viewport coordinates are relative to the 3D panel (below the toolbar).
click(480, 320)
wait(2)
move(580, 380)
wait(2)
set_dim("width", "80")
key("tab")
set_dim("height", "50")
key("enter")
exit_sketch()

-- Name the committed rectangle for later lookup.
set_name(element("rect", 0), "Preview box")

wait_ms(100)
screenshot("rectangle_preview.png")