// headless-capability-check.cjs — headless capability contract guard.
//
// Asserts the three headless capability surfaces can never drift apart:
//   1. src/lib/headless-capabilities.json — the single source of truth, served
//      verbatim at GET /api/v1/capabilities and driving DISABLED_COMMANDS.
//   2. src/lib/headless-routes.ts        — the transport route table.
//   3. src-tauri/src/**/*.rs             — the registered command universe.
//
// Invariants:
//   - every registered #[tauri::command] appears in the registry exactly once
//   - enabled entries exist in CMD_TO_HTTP with the same method + path
//   - disabled entries carry a reason in the CommandDisabledReason union
//   - enabled ∩ disabled = ∅, and neither table lists a command twice
//
// Usage: node scripts/headless-capability-check.cjs   (exits non-zero on drift)

const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.join(__dirname, "..");
const CAPS_PATH = path.join(ROOT, "src", "lib", "headless-capabilities.json");
const ROUTES_PATH = path.join(ROOT, "src", "lib", "headless-routes.ts");
const SRC_DIR = path.join(ROOT, "src-tauri", "src");

const VALID_REASONS = new Set([
  "desktop_only_tray",
  "desktop_only_os",
  "desktop_only_local_path",
  "desktop_only_transport",
]);

function listRsFiles(dir) {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...listRsFiles(p));
    else if (e.name.endsWith(".rs")) out.push(p);
  }
  return out;
}

// Same extraction discipline as scripts/ipc-manifest-check.cjs.
function extractCommandNames(src) {
  const names = [];
  let pending = false;
  let rename = null;
  for (const raw of src.split(/\r?\n/)) {
    const line = raw.trim();
    if (line.startsWith("#[tauri::command")) {
      const rm = line.match(/rename\s*=\s*"([^"]+)"/);
      rename = rm ? rm[1] : null;
      const rest = line.slice(line.indexOf("]") + 1);
      const fm = rest.match(/(?:pub\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)/);
      if (fm) {
        names.push(rename || fm[1]);
        pending = false;
        rename = null;
        continue;
      }
      pending = true;
      continue;
    }
    if (pending) {
      if (line.startsWith("//") || line.startsWith("#") || line === "") continue;
      const fm = line.match(/(?:pub\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)/);
      if (fm) names.push(rename || fm[1]);
      pending = false;
      rename = null;
    }
  }
  return names;
}

function fail(msg) {
  console.error(`[headless-capability-check] ${msg}`);
  process.exitCode = 1;
}

const caps = JSON.parse(fs.readFileSync(CAPS_PATH, "utf8"));
const entries = caps.commands ?? {};
const capsKeys = Object.keys(entries);
const enabled = capsKeys.filter((k) => entries[k].status === "enabled");
const disabled = capsKeys.filter((k) => entries[k].status === "disabled");

// (a) registry is complete + partition is disjoint
const seen = new Set();
for (const k of capsKeys) {
  if (seen.has(k)) fail(`registry lists ${k} twice`);
  seen.add(k);
  const e = entries[k];
  if (e.status !== "enabled" && e.status !== "disabled")
    fail(`${k}: unknown status ${JSON.stringify(e.status)}`);
  if (e.status === "enabled" && (!e.method || !e.path))
    fail(`${k}: enabled entry needs method+path`);
  if (e.status === "disabled" && !VALID_REASONS.has(e.reason))
    fail(`${k}: disabled entry needs a valid reason (got ${JSON.stringify(e.reason)})`);
}

// (b) every registered #[tauri::command] is covered exactly once
const universe = new Set();
for (const f of listRsFiles(SRC_DIR)) {
  for (const name of extractCommandNames(fs.readFileSync(f, "utf8"))) universe.add(name);
}
for (const cmd of universe) {
  if (!seen.has(cmd)) fail(`registered command ${cmd} missing from headless-capabilities.json`);
}
for (const k of capsKeys) {
  if (!universe.has(k)) fail(`registry entry ${k} has no matching #[tauri::command]`);
}

// (c) enabled entries match CMD_TO_HTTP exactly (method + path)
const routesSrc = fs.readFileSync(ROUTES_PATH, "utf8");
const mapBlock = routesSrc.slice(routesSrc.indexOf("CMD_TO_HTTP"));
const routeKeys = [...mapBlock.matchAll(/^\s{2}([a-z_0-9]+):\s*\{\s*method:\s*"([A-Z]+)",\s*path:\s*"([^"]+)"/gm)];
const httpMap = new Map(routeKeys.map((m) => [m[1], { method: m[2], path: m[3] }]));

for (const cmd of enabled) {
  const want = entries[cmd];
  const got = httpMap.get(cmd);
  if (!got) {
    fail(`${cmd}: enabled in registry but absent from CMD_TO_HTTP`);
    continue;
  }
  if (got.method !== want.method || got.path !== want.path)
    fail(`${cmd}: registry ${want.method} ${want.path} != route table ${got.method} ${got.path}`);
}
for (const cmd of httpMap.keys()) {
  if (!enabled.includes(cmd)) fail(`${cmd}: routed in CMD_TO_HTTP but not enabled in registry`);
}

if (process.exitCode) {
  console.error("[headless-capability-check] FAILED");
} else {
  console.log(
    `[headless-capability-check] OK — ${universe.size} commands covered (${enabled.length} enabled, ${disabled.length} disabled)`,
  );
}
