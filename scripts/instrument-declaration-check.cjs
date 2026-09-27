#!/usr/bin/env node
// r12-wave-i D-003.2 + D-007.1 (the register's "machine gate" line):
// enforces the standing procedural rule that every NEW trigger-line
// instrument declares three fields at birth. An entry in
// docs/agents/trigger-line-register.md is instrument-class when its text
// carries instrument vocabulary (instrument|counter|probe), OR it is an
// armed-status row in the "Other registered trigger lines" table (r12-wave-k
// audit extension: armed registrations are instruments too); such an entry
// MUST contain the three literal field markers:
//
//   obs-env:    observation environment - e.g. dev | release | feature:<name>
//   zero-read:  what a zero reading proves AND what it does not prove
//   evidence:   evidence-retention class - e.g. observation | delivery | none
//
// Detection scope: titled bullets (- **name**: ...) and table rows of the
// "Other registered trigger lines" table. Plain (untitled) meta bullets -
// the Procedure rules themselves - are out of scope.
//
// Pinned-out entries live in LEGACY_EXEMPTIONS below (waiver-ledger
// pattern: waivers are pinned, visible, and must be maintained). Two
// groups: (a) LEGACY instruments predating the gate; (b) META entries that
// merely mention instrument vocabulary in passing (a gate note, a verdict
// about an instrument) - they are not registrations, but the pin makes the
// exclusion explicit. A pin that matches no current entry is stale and
// fails the gate - drop it in the same commit that removed the entry.
// Additionally every entry carrying a covered-by: note must carry a
// machine-checkable reopen-when: condition (r12-wave-i D-005(ii)).

const fs = require("node:fs");
const path = require("node:path");

const REGISTER = path.join(__dirname, "..", "docs", "agents", "trigger-line-register.md");

const INSTRUMENT_WORD = /\b(instruments?|counters?|probes?)\b/i;
const FIELD_MARKERS = [
  ["obs-env", /\bobs-env:\s*\S/i],
  ["zero-read", /\bzero-read:\s*\S/i],
  ["evidence", /\bevidence:\s*\S/i],
];

// Entries live before this gate existed; pinned by a stable substring of
// their entry title / first table cell.
// (a) legacy instrument entries predating the gate:
const LEGACY_EXEMPTIONS = [
  "React Query migration (R12-01)",
  "paired_request_added_latency_p95 flake (C9)",
  "DbPool single-lock (bb8) (critique #6)",
  "DbPool read-path unlock (P3, r12-wave-f D-002)",
  "convergeDevMark overlap counter",
  "s1 post-land verification",
  "probe hang-pathology bound (audit)",
  "impl-session local rustc cfg-harness (audit)",
  "DbPool arm (i)",
  "R12-H2 implemented (D-004 steps 0+1)",
  "STANDING PROCEDURAL RULES (r12-wave-i D-007)",
  "Resin upstream probe issue",
  "NODE_FC counter-window semantics (C8, r12-wave-b D-003)",
  "ubuntu s1 waitForTitle flake",
  "i18n-check Rust-side blind spot",
  "STANDING RULING (scope, r12-wave-h D-005)",
  "Resin upstream-first sequence (r12-wave-h D-002/D-003)",
  // armed trigger rows predating the three-field rule (r12-wave-k audit):
  "AGENTS.md 32 KiB ceiling",
  "strategy_service split preemption (r12-wave-b D-001, extends wave-a D-003-7)",
  "forwarder port-scan O(n) suggestion path (critique #2)",
  "backup envelope versioning (critique #7)",
  "monthly faultinject schedule heartbeat",
  "Mode A shell forwarder boundary law (r12-wave-e D-002)",
  "macos-x86_64 delivery demotion (r12-wave-e D-003)",
  "census extractor AST upgrade (r12-wave-f D-003)",
  "snapshot.rs prod-lines gate (r12-wave-j D-005)",
  "ipc.ts interface-role drift (r12-wave-j D-006)",
  "macOS test-signal leg (r12-wave-j D-004)",
  "suggest_free_entry_port squatter escalation (r12-wave-j D-003)",
  // (b) meta entries: mention instrument vocabulary without registering
  // an instrument - pinned so the exclusion is explicit, not accidental.
  "db-lock-metrics feature landed", // landing note for the gated observer
];

const text = fs.readFileSync(REGISTER, "utf8");
const lines = text.split(/\r?\n/);

// Collect entries: { name, line, text } - titled bullets plus table rows of
// the "Other registered trigger lines" table.
const entries = [];
let inTriggerTable = false;
for (let i = 0; i < lines.length; i++) {
  const line = lines[i];
  const ln = i + 1;
  if (/^##\s/.test(line)) {
    inTriggerTable = /^##\s+Other registered trigger lines/.test(line);
    continue;
  }
  const bullet = line.match(/^- \*\*(.+?)\*\*\s*:/);
  if (bullet) {
    entries.push({ name: bullet[1], line: ln, text: line, armedRow: false });
    continue;
  }
  if (inTriggerTable && /^\|/.test(line)) {
    const cells = line.split("|").map((c) => c.trim()).filter((c) => c.length > 0);
    if (cells.length >= 2 && cells[0] !== "Item" && !/^---+$/.test(cells[0].replace(/\s/g, ""))) {
      const statusCell = cells[cells.length - 1] || "";
      entries.push({ name: cells[0], line: ln, text: line, armedRow: /(^|[^a-z0-9_])armed([^a-z0-9_]|$)/i.test(statusCell) });
    }
  }
}

const exempted = new Set();
const problems = [];
let scanned = 0;

for (const e of entries) {
  const isInstrument = INSTRUMENT_WORD.test(e.text) || e.armedRow === true;
  // Directional match only (r12-wave-i audit nit): the pin must be a
  // substring of the ENTRY name - a short future entry name can never be
  // silently exempted by containing a longer pin's tail.
  const pin = LEGACY_EXEMPTIONS.find((x) => e.name.includes(x));
  if (pin) exempted.add(pin);

  // Rule (ii): a covered-by note without reopen-when is a ledger error.
  if (/\bcovered-by:/i.test(e.text) && !/\breopen-when:/i.test(e.text)) {
    problems.push(`line ${e.line} (${e.name}): carries 'covered-by:' but no 'reopen-when:' condition`);
  }

  if (!isInstrument || pin) continue;
  scanned++;
  for (const [label, re] of FIELD_MARKERS) {
    if (!re.test(e.text)) {
      problems.push(
        `line ${e.line} (${e.name}): instrument-class entry missing '${label}:' declaration (r12-wave-i D-007 rule i)`,
      );
    }
  }
}

// Stale pins fail too - a waiver that matches nothing is unmaintained debt.
for (const pin of LEGACY_EXEMPTIONS) {
  if (!exempted.has(pin)) {
    problems.push(`stale exemption pin: no register entry matches "${pin}" - remove the pin in the commit that removed the entry`);
  }
}

if (problems.length > 0) {
  console.error("FAIL instrument-declaration-check:");
  for (const p2 of problems) console.error(`  - ${p2}`);
  process.exit(1);
}
console.log(
  `instrument-declaration-check OK: ${entries.length} entries scanned, ${scanned} instrument-class checked, ${exempted.size} legacy pins`,
);
