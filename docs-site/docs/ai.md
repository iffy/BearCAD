---
sidebar_position: 13
title: AI
---

# AI

Agents drive BearCAD from the outside, two ways — **Integration ▸ AI**, or
**View ▸ Panes ▸ AI**. Both are off until you switch them on.

## MCP server

**Start** in the **MCP Server** section, and an outside agent can read and edit the document
you have open. It listens on 127.0.0.1 only, and every request needs the token.

**Connect a client** has ready-made configurations for Claude Code, Cursor, VS Code and
Codex — copy one, paste it into that client. From a terminal, `bearcad mcp-install` prints
the same. A client that speaks only stdio uses `bearcad mcp`, which bridges to the running
app.

The token is never shown on screen, only copied — a screenshot of the pane would carry it.
**New token** invalidates whatever you copied before. **Activity** lists what a connected
agent has done.

The five tools an agent gets: `document_summary`, `document_lua`, `run_lua`, `undo`,
`screenshot`. `run_lua` reaches the whole [scripting API](/docs/scripting).

## Agent skill

One page that teaches an AI agent to drive BearCAD: see [AI agent
skill](/docs/scripting/agent-skill). The **Agent Skill** section installs it for the tools
on this machine.

## Security

Starting the server and installing the skill are hand-only, with no scripting API behind
them: a `.lua` file off the internet cannot open your document to anything.
