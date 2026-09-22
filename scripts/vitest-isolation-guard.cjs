#!/usr/bin/env node
/**
 * vitest-isolation-guard.cjs (scope and
 * generalized beyond its original single-view pin)
 *
 * Scannable regression guard for the view-test isolation pattern.
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
 * The rule is no longer pinned to NodesView.
 *   1. Every src/views/*.test.tsx file is scanned (targets discovered, not
 *      hardcoded), so a new view test that introduces the pattern is covered
 *      the day it lands.
 *   2. Any file that touches a role=button sub-header MUST declare the guarded
 *      helper block (clickSubHeaderExpand / clickSubHeaderCollapse, which await
 *      the settled chevron state before clicking).
 *   3. Raw sub-header clicks are detected in both the single-line form (a
 *      fireEvent.click whose argument resolves the element via closest
 *      role=button) and the two-step form (resolve the element first, click the
 *      variable later), outside the guarded helper block.
 *
 * Deliberately NOT generalized: a blanket ban on getByRole button clicks.
 * Ordinary buttons (import / add / export / sort) are clicked directly all over
 * the view tests; only the collapsible sub-header carries the seeding race, so a
 * blanket ban would be a false-positive machine, not a guard.
 *
 * Usage: node scripts/vitest-isolation-guard.cjs   (exit 0 = pass, 1 = fail)
 */
const fs = require("fs");
const path = require("path");

const MARKER = "vitest-isolation-guard: sub-header click helpers";
const HELPER_FNS = [
  "function subHeaderButton",
  "function clickSubHeaderExpand",
  "function clickSubHeaderCollapse",
];
const viewDir = path.resolve(__dirname, "..", "src", "views");

function fail(msg) {
  console.error("[vitest-isolation-guard] FAIL: " + msg);
  process.exit(1);
}

// A line that resolves a sub-header element. Substring matching keeps the
// detector agnostic to quote style and to the exact selector spelling.
function isRoleButtonClosest(line) {
  return line.includes("closest(") && line.includes("role=") && line.includes("button");
}

// Comment-only lines cannot execute, so they never count as violations nor as
// a reason to demand the guarded helper block.
function isComment(line) {
  const t = line.trim();
  return t.startsWith("//") || t.startsWith("*") || t.startsWith("/*");
}

// Name declared by a const/let/var assignment line, else null.
function declaredName(line) {
  const t = line.trim();
  for (const kw of ["const ", "let ", "var "]) {
    if (!t.startsWith(kw)) continue;
    const rest = t.slice(kw.length);
    const eq = rest.indexOf("=");
    if (eq <= 0) return null;
    const name = rest.slice(0, eq).trim();
    return /^[A-Za-z_][A-Za-z0-9_]*$/.test(name) ? name : null;
  }
  return null;
}

const targets = fs.readdirSync(viewDir).filter((f) => f.endsWith(".test.tsx")).sort();
if (targets.length === 0) fail("no src/views/*.test.tsx targets found under " + viewDir);

const violations = [];
const guarded = [];

for (const name of targets) {
  const src = fs.readFileSync(path.join(viewDir, name), "utf8");
  const lines = src.split("\n");
  const rel = "src/views/" + name;

  const bi = lines.findIndex((l) => l.includes(MARKER + " — BEGIN"));
  const ei = lines.findIndex((l) => l.includes(MARKER + " — END"));
  const hasMarkers = bi >= 0 || ei >= 0;
  const touchesSubHeader = lines.some((l) => !isComment(l) && isRoleButtonClosest(l));

  if (hasMarkers && bi < 0) { violations.push(rel + ": guard helper block BEGIN marker missing"); continue; }
  if (hasMarkers && ei < 0) { violations.push(rel + ": guard helper block END marker missing"); continue; }
  if (hasMarkers && ei <= bi) { violations.push(rel + ": guard helper block markers out of order"); continue; }
  if (touchesSubHeader && !hasMarkers) {
    violations.push(rel + ": touches a role=button sub-header but declares no guarded helper block - click via the settled-state helpers (" + HELPER_FNS.join(" / ") + ")");
    continue;
  }
  if (!hasMarkers) continue;

  const block = lines.slice(bi, ei + 1).join("\n");
  for (const fn of HELPER_FNS) {
    if (!block.includes(fn)) violations.push(rel + ": helper missing in guarded block: " + fn);
  }
  // The helpers are only isolation-safe while they wait for the settled
  // chevron before clicking - that await IS the fix.
  if (!block.includes("waitFor(") || !block.includes("fireEvent.click(")) {
    violations.push(rel + ": guarded helper block no longer awaits the settled state before fireEvent.click");
  }

  const outsideVars = new Set();
  lines.forEach((line, idx) => {
    if (idx >= bi && idx <= ei) return;
    if (isComment(line)) return;
    if (!isRoleButtonClosest(line)) return;
    const n = declaredName(line);
    if (n) outsideVars.add(n);
  });

  lines.forEach((line, idx) => {
    if (idx >= bi && idx <= ei) return;
    if (isComment(line)) return;
    if (!line.includes("fireEvent.click(")) return;
    if (isRoleButtonClosest(line)) {
      violations.push(rel + ":" + (idx + 1) + ": raw sub-header click outside the guarded block: " + line.trim());
      return;
    }
    for (const v of outsideVars) {
      if (line.includes(v)) {
        violations.push(rel + ":" + (idx + 1) + ": raw sub-header click on variable " + v + " (resolved via closest role=button outside the block): " + line.trim());
        return;
      }
    }
  });

  guarded.push(rel + " (helper block lines " + (bi + 1) + "-" + (ei + 1) + ")");
}

if (violations.length > 0) {
  fail(violations.length + " violation(s):\n  " + violations.join("\n  "));
}

console.log(
  "[vitest-isolation-guard] OK: " + targets.length + " view test file(s) scanned; " +
  guarded.length + " guarded helper block(s) intact; 0 raw sub-header clicks outside a guard block" +
  (guarded.length ? " [" + guarded.join("; ") + "]" : "")
);
