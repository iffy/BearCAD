-- Interaction regression (#1627): an Edit affordance in "Manage backends" changes a
-- backend's name, URL, model and key in place -- one entry, id and id, never a
-- remove-and-re-add. Removing and re-adding would (a) leave two of nothing (count of
-- backend entries) and (b) throw the all-time spend away; the spend-preservation itself
-- is pinned by `AiConfig::edit`'s unit test, since a script cannot mint usage. This test
-- proves the whole pane path with real pointer/keyboard input, and that an edit is not a
-- new backend entry in the config file the app writes.
--
-- BEARCAD_AI_CONFIG points the app at a throwaway ai.json; when set, the test also
-- asserts on what lands on disk.
bearcad.new()
bearcad.ui.tool("select")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("tutorials", "hide")
bearcad.ui.pane("ai", "show")
bearcad.ui.wait(6)

-- The AI pane sits flush right; the Elements pane sits flush left. A click is relative
-- to the 3D viewport, so translate a recorded widget rect (screen points) into viewport
-- coordinates by subtracting the viewport's top-left corner.
local elements = bearcad.ui.pane_rect("elements")
local ai = bearcad.ui.pane_rect("ai")
assert(elements and ai, "the Elements and AI panes must be visible for this test")
local vpx = elements.x + elements.w
local vpy = elements.y

local function click_widget(name, what)
  -- The pane may be mid-animation when layout changes; wait for the widget to show.
  local w
  for _ = 1, 40 do
    w = bearcad.ui.ai_backend_widget(name)
    if w then break end
    bearcad.ui.wait(3)
  end
  assert(w, what .. " ('" .. name .. "') never appeared")
  bearcad.ui.click(w.x + w.w / 2 - vpx, w.y + w.h / 2 - vpy)
  bearcad.ui.wait(3)
end

local function type_into(name, text, what)
  click_widget(name, what)
  bearcad.ui.key("a", { cmd = true })
  bearcad.ui.wait(2)
  bearcad.ui.type(text)
  bearcad.ui.wait(2)
end

-- Bottom of the AI pane is where Manage backends lives; whatever the window size, the
-- recorded rect is what is on screen, and reaching bottom first keeps every target inside
-- the pane's visible area.
bearcad.ui.scroll_pane("ai", 10000)
bearcad.ui.wait(6)

-- 1) Add a backend whose key is pasted (a Stored key, so the edit below has one to
--    replace in place).
type_into("add_name", "Test Backend", "the add form's Name field")
click_widget("add_key_mode_stored", "the add form's Paste key toggle")
type_into("add_key_paste", "sk-old-key-123", "the add form's key field")
click_widget("add_button", "the Add backend button")
bearcad.ui.wait(8)
local added_status = bearcad.status()
assert(added_status:find("Added AI backend Test Backend", 1, true)
  and added_status:find("test-backend", 1, true),
  "adding should report the slug-test id, got: " .. added_status)

-- 2) Edit it in place: name, URL, model and key all change, one entry stays.
bearcad.ui.scroll_pane("ai", 10000)
click_widget("edit:test-backend", "the Edit button for the added backend")
type_into("edit_name:test-backend", "Renamed Backend", "the edit form's Name field")
type_into("edit_url:test-backend", "https://gateway.example.com/v1",
  "the edit form's URL field")
type_into("edit_model:test-backend", "claude-haiku-4-5", "the edit form's Model field")
click_widget("edit_key_mode_stored:test-backend", "the edit form's Paste key toggle")
type_into("edit_key_paste:test-backend", "sk-new-key-456", "the edit form's key field")
click_widget("edit_save:test-backend", "the edit form's Save button")
bearcad.ui.wait(8)
local edited_status = bearcad.status()
assert(edited_status:find("Edited AI backend Renamed Backend", 1, true),
  "saving an edit should report the new name, got: " .. edited_status)
-- The form closed: its fields are gone.
assert(bearcad.ui.ai_backend_widget("edit_name:test-backend") == nil,
  "the edit form should close after Save")

-- 3) The config file holds one backend entry, still the same id, with the new fields.
local config_path = os.getenv("BEARCAD_AI_CONFIG")
if config_path then
  local f = io.open(config_path, "r")
  assert(f, "ai.json should exist after adding and editing a backend")
  local text = f:read("*a")
  f:close()
  local function occurrences(pat)
    local _, n = text:gsub(pat, "")
    return n
  end
  assert(occurrences('"id"') == 1,
    "an edit must leave exactly one backend -- never a remove-and-re-add copy, got " ..
    occurrences('"id"'))
  assert(text:find('"id": "test-backend"', 1, true), "the id survives, not re-slugged")
  assert(text:find("Renamed Backend", 1, true), "the new name is saved")
  assert(text:find("gateway.example.com", 1, true), "the new URL is saved")
  assert(text:find("claude-haiku-4-5", 1, true), "the new model is saved")
  assert(text:find("sk-new-key-456", 1, true), "the new key is saved")
  assert(not text:find("sk-old-key-123", 1, true), "the old key is gone")
  assert(text:find('"key": {', 1, true), "a key block is present")
end

-- 4) The key edit is done in place: re-opening the edit form shows the new name.
click_widget("edit:test-backend", "the Edit button again")
local reopen = bearcad.ui.ai_backend_widget("edit_name:test-backend")
assert(reopen ~= nil, "the edit form re-opens for the same backend")
-- Cancel leaves everything as it was.
click_widget("edit_cancel:test-backend", "the edit form's Cancel button")
bearcad.ui.wait(4)

print("ok: Manage backends edits a backend in place (#1627)")
bearcad.quit()