-- Example Lua script — draw a line on the default sketch plane, screenshot.
-- Run: cargo run -- --script examples/line.lua --exit

le3.new()
le3.begin_sketch("construction_plane", 0)
le3.tool("line")

le3.click(480, 320)
le3.wait(2)
le3.move(580, 360)
le3.wait(2)
le3.set_dim("length", "80")
le3.key("enter")
le3.exit_sketch()

le3.set_name(le3.element("line", 0), "Guide line")
assert(le3.find("Guide line") ~= nil)

le3.wait_ms(100)
le3.screenshot("line_preview.png")
le3.quit()