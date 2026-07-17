# BearCAD website & docs

The BearCAD website and documentation, built with [Docusaurus](https://docusaurus.io/).

## Writing style

The best documentation is the app. Docs exist to reveal what the app can't show at a
glance — hidden interactions, keyboard shortcuts, rules, and the scripting API. Everything
else is noise. Concretely:

- **Don't describe what the screen shows.** No visual styling (colors, stroke weights,
  arrowheads), no hover/highlight feedback, no restating a labeled button's effect.
- **Lead with the action, imperatively.** "Click edges to show/hide dimensions" — not
  "With the tool active, the edge under the cursor highlights — click it to…".
- **One idea per sentence; one interaction per paragraph.** No em-dash chains or stacked
  parentheticals. Prefer a heading and three short lines over a dense paragraph.
- **Cut automatic behavior** the user never invokes (auto-stacking, auto-sharing) unless
  omitting it would confuse.
- **Keep** hidden gestures (Shift+click, double-click, drag), shortcuts, defaults,
  non-obvious rules, and scripting examples.
- **No reassurances or narration** ("so it's clear which…", "you can also…"). State the
  fact once and stop.

- The **landing page** is served at the site root (`/`) from `src/pages/index.js`.
- The **documentation** is served under `/docs/` from the `docs/` folder.

## Install

```bash
npm ci
```

## Local development

```bash
npm run start
```

Starts a local dev server and opens a browser window. Most changes reload live.

## Build

```bash
npm run build
```

Generates the static site into `build/` — both `build/index.html` (landing page) and
`build/docs/` (documentation). Serve it locally with `npm run serve`.

## Deployment

Deployment is automated: pushes to `master` that touch `docs-site/**` trigger
[`.github/workflows/docs.yml`](../.github/workflows/docs.yml), which runs `npm run build` and
publishes `build/` to GitHub Pages.
