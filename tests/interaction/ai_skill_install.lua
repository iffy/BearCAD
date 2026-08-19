-- Interaction regression (#1604): the agent skill installs and uninstalls from inside the
-- app, and a project install leaves the rest of the file alone.
--
-- Run with BEARCAD_SKILL_PROJECT set to a throwaway directory (CI does); the test writes
-- an AGENTS.md there.
bearcad.new()
bearcad.ui.tool("select")
bearcad.ui.pane("ai", "show")

local dir = os.getenv("BEARCAD_SKILL_PROJECT")
assert(dir and #dir > 0, "set BEARCAD_SKILL_PROJECT to a throwaway directory")

-- The pane lists every target with where it would go.
local targets = bearcad.ai.skill_targets(dir)
assert(#targets >= 4, "expected several install targets, got " .. #targets)
local by_id = {}
for _, t in ipairs(targets) do by_id[t.id] = t end
assert(by_id["claude"] and by_id["claude"].scope == "user", "Claude Code is a user target")
assert(by_id["agents"] and by_id["agents"].scope == "project", "AGENTS.md is a project target")
assert(not by_id["agents"].installed, "nothing installed in a fresh directory")

-- Install into the project, next to content that must survive.
local agents = dir .. "/AGENTS.md"
local f = io.open(agents, "w")
f:write("# House rules\n\nAlways run the tests.\n")
f:close()

bearcad.ai.install_skill{ target = "agents", dir = dir }
assert(bearcad.status():find("Installed"), "install should report itself, got: " .. bearcad.status())

local text = io.open(agents):read("a")
assert(text:find("# House rules"), "the user's own content survives an install")
assert(text:find("BEGIN BearCAD skill"), "the skill is marked off")
assert(text:find("bearcad.extrude"), "and the skill is actually in there")
assert(bearcad.ai.skill_targets(dir)[1] ~= nil)

local after = bearcad.ai.skill_targets(dir)
for _, t in ipairs(after) do
  if t.id == "agents" then assert(t.installed, "the pane now shows it as installed") end
end

-- Removing takes out only the marked region.
bearcad.ai.uninstall_skill{ target = "agents", dir = dir }
local text2 = io.open(agents):read("a")
assert(text2 == "# House rules\n\nAlways run the tests.\n",
  "uninstall should restore the file exactly, got: " .. text2)

-- The skill markdown is available to a script that wants to place it itself.
assert(#bearcad.ai.skill() > 1000, "the skill should be a real document")

-- An unknown target fails rather than writing somewhere unexpected.
local ok = pcall(function() bearcad.ai.install_skill{ target = "not-a-tool", dir = dir } end)
assert(not ok, "an unknown target should fail")

print("ok: the agent skill installs and uninstalls without disturbing existing content")
bearcad.quit()
