---
sidebar_position: 23
title: Shell
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/shell.svg")} width="30" /> Shell

Hollow a solid to a wall thickness. Optionally open faces so the inside is accessible.

## How to use it

1. Pick the **Shell** tool and click one or more bodies (**Bodies**).
2. After the first body, focus moves to **Open faces** — pick faces on the selected bodies to remove (leave wall thickness around them). Adjacent open faces also remove the shared wall.
3. Set **Thickness** (mm expression) in the context pane.
4. Press **Enter**.

While you pick, the hollowed result previews semi-transparent on the target bodies.

## What you get

Each hollowed solid is a new body nested under the shell element. The input lives on as a **shadow body**. **Edit shell** re-opens the pickers and thickness; deleting the shell restores the input.

## Scripting

```lua
bearcad.shell{ bodies = {0}, faces = {{ kind = "extrude_cap", extrusion = 0, profile = 0, top = true }}, thickness = "1" }
bearcad.shell{ bodies = {0}, thickness = "2" }  -- closed hollow
bearcad.edit_shell{ index = 0, bodies = {0}, faces = {}, thickness = "1.5" }
```
