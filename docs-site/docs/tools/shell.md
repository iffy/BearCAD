---
sidebar_position: 23
title: Shell
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/shell.svg")} width="30" /> Shell

Hollow a solid to a wall thickness. Optionally open faces so the inside is accessible.

<a
  href={useBaseUrl("/app/") + "?open=" + encodeURIComponent(useBaseUrl("/img/screenshots/shell.bearcad.json"))}
  target="_blank"
  rel="noopener noreferrer"
  title="Open this model in BearCAD"
>
  <img src={useBaseUrl("/img/screenshots/shell.png")} alt="A block hollowed to a thin wall with its top face open, seen from above" />
</a>

## How to use it

1. Pick the **Shell** tool and click one or more bodies (**Bodies**).
2. After the first body, focus moves to **Open faces** — pick faces on the selected bodies to remove (leave wall thickness around them). Adjacent open faces also remove the shared wall.
3. Set **Thickness** (mm expression) in the context pane, or drag the push/pull handle on a face (first open face, else a body face). Clicking the handle focuses Thickness for overwrite typing.
4. Press **Enter**.

While you pick, the hollowed result (what remains) previews semi-transparent on the target bodies.

## What you get

Each hollowed solid is a new body nested under the shell element. The input lives on as a **shadow body**. **Edit shell** re-opens the pickers and thickness; deleting the shell restores the input.

See [Scripting](/docs/scripting).
