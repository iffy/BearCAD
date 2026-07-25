---
sidebar_position: 12
title: Settings
---

# Settings

**⌘/Ctrl+,** opens the Settings window; pressing it again closes it. It's also in the
command palette, and in the app menu (macOS) or File menu (Windows).

Settings belong to the machine, not the document, and save the moment you change them.

Turn on **help mode** — the command palette's *Turn On Help Mode* — and every row
explains itself:

![The Settings window, each field explained](/img/screenshots/pane-settings.png)

## Library directory

The folder your reusable parts live in. A document can import a file under it by its
library path, so the import resolves on any machine whose library holds the same parts —
see [Files, import & export](/docs/files).

- **Choose…** picks the folder.
- **✕** clears it.

```lua
bearcad.ui.settings("show")   -- "hide" / "toggle"
```
