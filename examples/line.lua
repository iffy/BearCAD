-- Example — make a line on the default ground plane with a single call.
-- Run: cargo run -- --script examples/line.lua --exit

le3.new()

-- One call: enters a ground-plane sketch if needed, then creates an 80 mm line (horizontal
-- by default; pass `angle` in degrees, or explicit `x1`/`y1` endpoints) and names it.
le3.line{ length = 80, name = "Guide line" }
assert(le3.find("Guide line") ~= nil)

le3.wait_ms(100)
le3.screenshot("line_preview.png")
le3.quit()
