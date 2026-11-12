#!/usr/bin/env node
// Regenerates kxbps-qt/resources/catalog.json from the web app's package table.
//
// The web app (qxbps, TypeScript) is the authoritative source for the catalog, so the
// Rust build never hand-maintains package data: this script imports `PACKAGES` from
// `catalog.ts` and serialises it in the field order of `kxbps_core::types::Pkg`.
//
// Usage:
//   node scripts/gen-catalog.mjs ../qxbps/src/lib/xbps/catalog.ts
//
// Requires Node >= 22.18 / 24, where `import()` of a `.ts` file strips types natively.

import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const REPO_IDS = new Set(["current", "nonfree", "multilib", "debug"]);
const CATEGORIES = new Set([
  "base",
  "devel",
  "desktop",
  "net",
  "media",
  "graphics",
  "office",
  "games",
  "fonts",
  "admin",
]);

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outputPath = resolve(repoRoot, "kxbps-qt/resources/catalog.json");

const sourcePath = process.argv[2];
if (!sourcePath) {
  console.error("usage: node scripts/gen-catalog.mjs <path/to/catalog.ts>");
  process.exit(2);
}

const module = await import(pathToFileURL(resolve(sourcePath)).href);
const packages = module.PACKAGES;
if (!Array.isArray(packages) || packages.length === 0) {
  console.error(`error: ${sourcePath} does not export a non-empty PACKAGES array`);
  process.exit(1);
}

const problems = [];
const seen = new Set();

const isString = (value) => typeof value === "string";
const isCount = (value) => Number.isInteger(value) && value >= 0;

for (const [index, pkg] of packages.entries()) {
  const where = `PACKAGES[${index}] (${pkg?.name ?? "?"})`;
  if (!isString(pkg?.name) || pkg.name.length === 0) problems.push(`${where}: missing name`);
  if (!isString(pkg?.version) || pkg.version.length === 0) problems.push(`${where}: missing version`);
  if (!REPO_IDS.has(pkg?.repo)) problems.push(`${where}: unknown repo ${JSON.stringify(pkg?.repo)}`);
  if (!CATEGORIES.has(pkg?.category)) problems.push(`${where}: unknown category ${JSON.stringify(pkg?.category)}`);
  if (!isCount(pkg?.size)) problems.push(`${where}: size is not a byte count`);
  if (!isCount(pkg?.download)) problems.push(`${where}: download is not a byte count`);
  for (const key of ["short", "homepage", "license"]) {
    if (!isString(pkg?.[key])) problems.push(`${where}: ${key} is not a string`);
  }
  for (const key of ["depends", "files"]) {
    if (!Array.isArray(pkg?.[key]) || !pkg[key].every(isString)) {
      problems.push(`${where}: ${key} is not a string array`);
    }
  }
  if (seen.has(pkg?.name)) problems.push(`${where}: duplicate package name`);
  seen.add(pkg?.name);
}

if (problems.length > 0) {
  console.error(`error: ${problems.length} problem(s) in ${sourcePath}`);
  for (const problem of problems) console.error(`  - ${problem}`);
  process.exit(1);
}

// Explicit projection: keeps the JSON field order identical to the Rust struct and
// drops anything the web app grows that the Rust model does not know about yet.
const catalog = packages.map((pkg) => ({
  name: pkg.name,
  version: pkg.version,
  repo: pkg.repo,
  category: pkg.category,
  size: pkg.size,
  download: pkg.download,
  short: pkg.short,
  homepage: pkg.homepage,
  license: pkg.license,
  depends: pkg.depends,
  files: pkg.files,
}));

await mkdir(dirname(outputPath), { recursive: true });
await writeFile(outputPath, `${JSON.stringify(catalog, null, 2)}\n`, "utf8");
console.log(`wrote ${catalog.length} packages to ${outputPath}`);
