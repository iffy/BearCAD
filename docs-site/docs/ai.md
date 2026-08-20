---
sidebar_position: 13
title: AI
---

# AI

**View ▸ Panes ▸ AI** opens one pane with three sections: chat, the agent skill, and a local
MCP server. All three are off until you switch them on — nothing here reaches the network
until you configure a backend and press Send.

## Chat

Add a backend under **Manage backends**: pick Anthropic, OpenAI, OpenRouter, xAI or an
OpenAI-compatible server (Ollama and friends). A backend is added *without* a model —
which models it has is a question only the backend can answer, so the **Model** row asks it
once the backend is connected and fills a dropdown from the answer.

**OpenRouter** (and any provider that offers it) connects with a browser click:
**Connect to OpenRouter** sends you to the provider, you approve, and the app is given the
key — you never see or paste one. Everywhere else still takes a key, which is either read
from an environment variable at send time — nothing is stored — or pasted and kept in
`ai.json` next to your settings, written owner-only. Neither the pane, nor a script, nor
`--show-commands`, nor a Lua export will hand a key back out.

The first message to a backend asks first, naming where it would go. Answer once per
backend.

**Sees** chooses what each message carries: this document, or every document open in the
app. Each document goes as the Lua script that recreates it, so the model reads it in the
same language it answers in. The Lua API catalog goes with it, so the model writes calls
the app can run. After a message, the line under the thread expands to show exactly what
was sent.

Costs sit under each reply — tokens, and money when the model's rate is known. An unknown
model shows tokens only; it never invents a price. **Manage backends** keeps a running
all-time total per backend, with **Reset**.

Lua in a reply gets a **Run** button. Nothing runs on its own: one click runs one block
against the active document, and **⌘/Ctrl+Z** takes it back.

The conversation is never saved — not with the document, not to disk.

## Agent skill

One page that teaches an AI agent to drive BearCAD: see [AI agent
skill](/docs/scripting/agent-skill). **Agents & Skill** installs it for the tools on this
machine; **Help ▸ Install AI Agent Skill…** opens that section.

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

## Not scriptable

Nothing here has a scripting API. A script cannot add a backend, send a message, start
the MCP server or install the skill — so a `.lua` file off the internet cannot spend your
key or open your document to anything.
