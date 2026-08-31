-- Interaction regression (#1769): the cutting plane tool scopes which bodies a plane takes
-- — an Exclude picker whose picks spare bodies, plus a scriptable cut list ("all" or
-- explicit indices) read back through `section_planes`.
bearcad.new()
bearcad.ui.tool("select")
bearcad.rect{ width = 30, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
bearcad.exit_sketch()
bearcad.begin_sketch{ kind = "plane", index = 0 }
bearcad.rect{ x = 40, y = 0, width = 30, height = 20 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 10 }
bearcad.exit_sketch()
bearcad.cross_section{ name = "Scoped" }

bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.camera{ target = {0, 0, 0}, distance = 260 }
bearcad.ui.wait(3)

bearcad.ui.tool("section_plane")
bearcad.ui.wait(3)

local function picker(name)
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

-- Anchor the draft on body 0's top face; the scope pickers appear alongside it.
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(5)
assert(picker("Anchor") and #picker("Anchor").items == 1, "the draft has its anchor")
assert(picker("Cut bodies") and picker("Exclude"),
  "an anchored draft shows the Cut bodies and Exclude pickers")
assert(not picker("Exclude").focused, "the scopes do not steal focus from the gizmos (#1750)")

-- Arm Exclude from the pane, click body 0 into it, accept.
bearcad.ui.picker_focus("Exclude")
bearcad.ui.wait(5)
assert(picker("Exclude").focused, "the pane click armed Exclude")
bearcad.select{ kind = "body", index = 0 }
bearcad.ui.wait(5)
assert(#picker("Exclude").items == 1, "the body landed in Exclude")
assert(#picker("Cut bodies").items == 0, "and Cut bodies still reads All (empty list)")

bearcad.ui.key("Enter")
bearcad.ui.wait(5)
assert(#bearcad.section_planes() == 1, "accept hung the scoped plane")

local cuts = bearcad.section_planes()
assert(cuts[1].bodies == "all", "with no explicit list the plane reads All bodies")
assert(#cuts[1].excludes == 1 and cuts[1].excludes[1] == 0,
  "body 0 is excluded from the hanging plane")

-- The scripted path rescopes a hanging plane: explicit list, reset to All.
bearcad.edit_section_plane{ cut = 0, bodies = {1} }
cuts = bearcad.section_planes()
assert(type(cuts[1].bodies) == "table" and cuts[1].bodies[1] == 1,
  "bodies = {1} restricts the cut to body 1")
assert(cuts[1].excludes[1] == 0, "the exclusion survives a rescope")

bearcad.edit_section_plane{ cut = 0, exclude_bodies = false }
cuts = bearcad.section_planes()
assert(#cuts[1].excludes == 0, "exclude_bodies = false clears the exclusions")

bearcad.edit_section_plane{ cut = 0, bodies = "all" }
cuts = bearcad.section_planes()
assert(cuts[1].bodies == "all", "bodies = \"all\" restores every body")

-- A scope naming no body fails loudly instead of dropping silently.
local refused = pcall(function() bearcad.edit_section_plane{ cut = 0, bodies = {99} } end)
assert(not refused, "edit_section_plane refuses unknown body indices")

print("ok: cutting planes can list and exclude bodies, in the pane and in scripts")
bearcad.quit()
