#!/usr/bin/env node
/**
 * vitest-isolation-guard.cjs (architecture-recovery ticket 18)
 *
 * Scannable regression guard for the NodesView test-isolation pattern.
 *
 * Root cause it guards against (verified 2026-09-01, runs 2+3 of 6 full-suite
 * rounds): NodesView seeds default-collapse in a post-commit effect
 * (NodesView.tsx seeding useEffect, wholesale setCollapsed(new Set([...grouped.keys()]))).
 * A sub-header click that lands before the seeding effect toggles the
 * pre-seed EXPANDED state (expand -> collapse) and the seeding then
 * force-collapses every group, so the rows the test waits for never render.
 * Full-suite CPU contention widens the commit-to-effect gap, making the flake
 * full-run-only (standalone always green).
 *
 * Enforcement: in src/views/NodesView.test.tsx every sub-header toggle MUST
 * go through the guarded click helpers (clickSubHeaderExpand /
 * clickSubHeaderCollapse, which first await the settled chevron state).
 * Any raw fireEvent.click on a [role='button'] sub-header outside the
 * helper block fails this guard.
 *
 * Usage: node scripts/vitest-isolation-guard.cjs   (exit 0 = pass, 1 = fail)
 */
const fs = require("fs");
const path = require("path");

const target = path.resolve(__dirname, "..", "src", "views", "NodesView.test.tsx");
const src = fs.readFileSync(target, "utf8");
const lines = src.split("\n");

const bi = lines.findIndex((l) => l.includes("vitest-isolation-guard: sub-header click helpers — BEGIN (ticket 18)"));
const ei = lines.findIndex((l) => l.includes("vitest-isolation-guard: sub-header click helpers — END"));

function fail(msg) {
  console.error("[vitest-isolation-guard] FAIL: " + msg);
  process.exit(1);
}

if (bi < 0) fail("guard helper block BEGIN marker missing in " + target);
if (ei < 0) fail("guard helper block END marker missing in " + target);
if (ei <= bi) fail("guard helper block markers out of order");

let inBlock = false;
let violations = 0;
lines.forEach((line, idx) => {
  if (idx === bi) { inBlock = true; return; }
  if (idx === ei) { inBlock = false; return; }
  if (inBlock) return;
  // Raw sub-header toggle clicks are forbidden outside the helper block.
  if (line.includes("fireEvent.click(") && line.includes(".closest(\"[role='button']\")")) {
    console.error("  raw sub-header click @ line " + (idx + 1) + ": " + line.trim());
    violations += 1;
  }
});

const block = lines.slice(bi, ei + 1).join("\n");
for (const fn of ["function subHeaderButton", "function clickSubHeaderExpand", "function clickSubHeaderCollapse"]) {
  if (!block.includes(fn)) fail("helper missing in guarded block: " + fn);
}

if (violations > 0) {
  fail(violations + " raw sub-header click(s) outside the guarded helper block. Use await clickSubHeaderExpand(name)/clickSubHeaderCollapse(name) so the click lands after the default-collapse seeding effect (see NodesView.tsx seeding useEffect).");
}

console.log("[vitest-isolation-guard] OK: " + target.split(path.sep).join("/") + " - helpers intact, 0 raw sub-header clicks outside guard block (lines " + (bi + 1) + "-" + (ei + 1) + ")");

