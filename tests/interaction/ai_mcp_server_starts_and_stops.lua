-- Interaction regression (#1605): the local MCP server is off until asked, listens on
-- loopback when started, and never hands its token out by accident.
--
-- The protocol itself (initialize, tools/list, tools/call, auth) is covered by the Rust
-- tests in src/ai/mcp.rs, which can act as a client; this checks the app's side of it.
--
-- Run with BEARCAD_AI_CONFIG pointing at a throwaway file (CI does).
bearcad.new()
bearcad.ui.tool("select")
bearcad.ui.pane("ai", "show")

-- Off by default. This is the whole opt-in promise.
local status = bearcad.ai.mcp_status()
assert(status.running == false, "the MCP server must not be running until it is switched on")
assert(bearcad.ai.mcp_token() == nil, "no token exists before the server has ever run")

-- Port 0 asks the OS for a free one, so the test cannot collide with anything.
bearcad.ai.mcp_start{ port = 0 }
status = bearcad.ai.mcp_status()
assert(status.running, "the server should be running")
assert(status.port > 0, "it should report the port it got, got " .. tostring(status.port))
assert(status.url:find("127.0.0.1", 1, true), "loopback only, got " .. status.url)
assert(status.url:find("/mcp"), "the URL should name the endpoint, got " .. status.url)

-- The token exists, but status never carries it: status is what gets printed and screenshot.
local token = bearcad.ai.mcp_token()
assert(type(token) == "string" and #token >= 32, "a token should exist once running")
for key, value in pairs(status) do
  assert(type(value) ~= "string" or not value:find(token, 1, true),
    "mcp_status leaked the token in field " .. key)
end

-- Starting twice is refused rather than silently rebinding.
local ok = pcall(function() bearcad.ai.mcp_start{} end)
assert(not ok, "starting an already-running server should fail")

-- A new token replaces the old one (any client configured with it stops working).
bearcad.ai.mcp_new_token()
local fresh = bearcad.ai.mcp_token()
assert(fresh ~= token, "regenerating should actually change the token")
assert(bearcad.ai.mcp_status().running, "and leave the server running with the new one")

-- Ready-made client configurations carry this server's URL and token, so what the pane
-- copies is what a client needs.
local configs = bearcad.ai.mcp_configs()
assert(#configs >= 4, "expected configurations for the usual clients, got " .. #configs)
local seen = {}
for _, c in ipairs(configs) do seen[c.id] = c end
assert(seen["claude"] and seen["json"] and seen["codex"], "the named clients are covered")
assert(seen["json"].text:find(bearcad.ai.mcp_status().url, 1, true), "the JSON config has the URL")
assert(seen["json"].text:find(fresh, 1, true), "and the current token")
-- Codex takes the name of an environment variable, not the value.
assert(seen["codex"].text:find("bearer_token_env_var"), "Codex's shape is its own")

bearcad.ai.mcp_stop()
assert(not bearcad.ai.mcp_status().running, "stop should stop it")

-- With the server down there is nothing to configure a client against.
assert(not pcall(function() return bearcad.ai.mcp_configs() end),
  "configs need a running server")

print("ok: the MCP server is opt-in, loopback-only, and keeps its token to itself")
bearcad.quit()
