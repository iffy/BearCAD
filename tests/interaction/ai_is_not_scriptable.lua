-- Interaction regression (#1616): a script cannot reach the AI at all.
--
-- Scripts get copied off the internet and run without being read. None of them gets to
-- talk to a backend, spend a key, start an MCP server or install an agent skill, so the
-- whole `bearcad.ai` namespace is gone -- and no flat `ai_*` spelling is left behind for
-- one to fall back on. The AI pane itself still works; only the script surface is gone.
bearcad.new()
bearcad.ui.tool("select")

assert(bearcad.ai == nil, "there should be no bearcad.ai namespace")

local gone = { "backends", "backend", "add_backend", "update_backend", "remove_backend",
               "set_backend", "send", "ask", "stop", "clear", "consented", "streaming",
               "messages", "context_scope", "context_preview", "usage", "reset_usage",
               "blocks", "run_block", "seed_reply", "skill", "skill_targets",
               "install_skill", "uninstall_skill", "api", "mcp_start", "mcp_stop",
               "mcp_status", "mcp_token", "mcp_new_token", "mcp_configs" }

for _, name in ipairs(gone) do
  local ok = pcall(function() return bearcad.ai[name]() end)
  assert(not ok, "bearcad.ai." .. name .. " should not be callable")
  assert(bearcad["ai_" .. name] == nil,
    "bearcad.ai_" .. name .. " should not exist either")
end

-- The pane is still a pane: showing it is not talking to a model.
bearcad.ui.pane("ai", "show")
bearcad.ui.pane("ai", "hide")

print("bearcad.ai is not reachable from a script")
