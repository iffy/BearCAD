---
sidebar_position: 6
title: AI agent skill
---

# AI agent skill

One page that teaches an AI agent to drive BearCAD: [**bearcad-skill.md**](pathname:///bearcad-skill.md).

Point an agent at that URL, or install it locally:

```sh
bearcad skill install              # every AI tool found on this machine
bearcad skill targets              # what it found, and what is already installed
bearcad skill install --target claude
bearcad skill print > SKILL.md     # the markdown, for anything not listed
```

The AI pane's **Agents & Skill** section does the same from inside the app.

Installing writes into a shared file (`AGENTS.md`, Copilot instructions) between
`BEGIN/END BearCAD skill` markers, so the rest of that file is left alone; `bearcad skill
uninstall` removes only that region. Dedicated skill files are replaced whole.

## What the agent gets

How to run a script, the declarative/`bearcad.ui.*` split, sketching, solids, parameters and
constraints, reading state back to verify its own work, files and export, and how to reach a
document that is already open over [MCP](#).

The examples in it are exercised by
[`tests/interaction/ai_skill_examples_run.lua`](https://github.com/iffy/BearCAD/blob/master/tests/interaction/ai_skill_examples_run.lua),
and a test asserts every API call it mentions exists — an agent following it is not reading
last release's syntax.
