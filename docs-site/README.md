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

## Preview locally

Node 20+. From the repo root:

```bash
cd docs-site
npm ci          # first time, or after package-lock changes
npm start       # http://localhost:3000/ — live reload
```

Landing page is `/`. Docs are `/docs/`.

Production-like preview (what CI publishes):

```bash
npm run build
npm run serve   # http://localhost:3000/
```

Doc screenshots under `static/img/screenshots/` are CI artifacts. A local build
warns if they're missing; the pages still serve.

## Deployment

Deployment is automated: pushes to `master` that touch `docs-site/**` trigger
[`.github/workflows/docs.yml`](../.github/workflows/docs.yml), which runs `npm run build` and
publishes `build/` to GitHub Pages.
