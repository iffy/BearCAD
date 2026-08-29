#!/usr/bin/env node
// Every /img/screenshots/... asset a doc page links to must actually exist (#1837).
//
// Docusaurus only checks markdown `![]()` images, and nearly every doc picture is JSX
// `<img src={useBaseUrl("/img/screenshots/x.png")} />`, so a page can ship with broken
// images and the build stays silent. This runs as `prebuild`, so `npm run build` cannot
// skip it.
//
// Two kinds of missing:
//   * nothing captures it — a typo, or a page written against a picture that was never
//     planned. Always fatal: no amount of waiting produces it.
//   * a capture script exists but has not run here — the push path only fetches whatever
//     the last nightly published, so this legitimately lags. A warning, and fatal under
//     --strict (BEARCAD_DOCS_REQUIRE_SCREENSHOTS=1), which is the nightly's own path.
//
// Usage: node docs-site/scripts/check-doc-screenshots.mjs [--strict]
//        [--docs=DIR]... [--static=DIR] [--scripts=DIR]

import { readdirSync, readFileSync, statSync, appendFileSync, existsSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const site = resolve(here, "..");

const args = process.argv.slice(2);
const opt = (name) =>
  args.filter((a) => a.startsWith(`--${name}=`)).map((a) => a.slice(name.length + 3));

const docRoots = opt("docs").length ? opt("docs") : [join(site, "docs"), join(site, "src")];
const staticDir = opt("static")[0] ?? join(site, "static/img/screenshots");
const scriptDir = opt("scripts")[0] ?? join(site, "screenshots");
const strict =
  args.includes("--strict") ||
  ["1", "true"].includes(process.env.BEARCAD_DOCS_REQUIRE_SCREENSHOTS ?? "");

const META = new Set(["manifest.txt", ".manifest", "screenshots-commit.txt", ".screenshots-commit"]);

function walk(dir, keep) {
  const out = [];
  let entries;
  try {
    entries = readdirSync(dir);
  } catch {
    return out;
  }
  for (const name of entries.sort()) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) out.push(...walk(path, keep));
    else if (keep(name)) out.push(path);
  }
  return out;
}

// --- What the docs ask for --------------------------------------------------------
const REF = /\/img\/screenshots\/([A-Za-z0-9._/-]+)/g;
const pages = walk(docRoots.length ? docRoots[0] : "", () => true);
for (const root of docRoots.slice(1)) pages.push(...walk(root, () => true));

/** asset name -> the pages that reference it */
const wanted = new Map();
for (const page of pages) {
  if (!/\.(md|mdx|js|jsx|ts|tsx|html)$/.test(page)) continue;
  const text = readFileSync(page, "utf8");
  for (const m of text.matchAll(REF)) {
    const asset = m[1];
    if (!wanted.has(asset)) wanted.set(asset, []);
    const from = page.startsWith(site) ? page.slice(site.length + 1) : page;
    if (!wanted.get(asset).includes(from)) wanted.get(asset).push(from);
  }
}

// --- What exists --------------------------------------------------------------------
const present = new Set(
  walk(staticDir, (n) => !META.has(n)).map((p) => p.slice(staticDir.length + 1)),
);
// A fetched deploy advertises its own list; trust it as well as the files on disk.
const manifest = join(staticDir, "manifest.txt");
if (existsSync(manifest)) {
  for (const line of readFileSync(manifest, "utf8").split("\n")) {
    const rel = line.trim();
    if (rel && !META.has(rel)) present.add(rel);
  }
}

// --- What could exist ----------------------------------------------------------------
const capturers = walk(scriptDir, (n) => n.endsWith(".lua")).map((p) => basename(p, ".lua"));
const capturable = (asset) => capturers.some((c) => asset === c || asset.startsWith(`${c}-`) || asset.startsWith(`${c}.`));

// --- Report ---------------------------------------------------------------------------
const orphans = [];
const pending = [];
for (const [asset, from] of [...wanted].sort()) {
  if (present.has(asset)) continue;
  (capturable(asset) ? pending : orphans).push([asset, from]);
}

const ci = !!process.env.GITHUB_ACTIONS;
const line = ([asset, from]) => `  /img/screenshots/${asset}  <- ${from.join(", ")}`;

if (orphans.length) {
  console.error(
    `error: ${orphans.length} screenshot(s) referenced by the docs that no capture script produces:`,
  );
  for (const o of orphans) {
    console.error(line(o));
    if (ci) console.log(`::error::missing screenshot /img/screenshots/${o[0]} (${o[1].join(", ")})`);
  }
  console.error("  Add a docs-site/screenshots/*.lua that captures it, or fix the reference.");
}
if (pending.length) {
  console.error(
    `warning: ${pending.length} screenshot(s) not captured here yet (the nightly publishes them):`,
  );
  for (const p of pending) {
    console.error(line(p));
    if (ci) console.log(`::warning::screenshot /img/screenshots/${p[0]} is not published yet`);
  }
}

const summary = process.env.GITHUB_STEP_SUMMARY;
if (summary && (orphans.length || pending.length)) {
  const md = ["### Missing doc screenshots", ""];
  for (const [asset, from] of orphans) md.push(`- ❌ \`${asset}\` — nothing captures it (${from.join(", ")})`);
  for (const [asset, from] of pending) md.push(`- ⏳ \`${asset}\` — not published yet (${from.join(", ")})`);
  md.push("");
  appendFileSync(summary, md.join("\n"));
}

if (orphans.length || (strict && pending.length)) {
  process.exit(1);
}
if (!orphans.length && !pending.length) {
  console.log(`All ${wanted.size} referenced doc screenshot(s) present.`);
}
