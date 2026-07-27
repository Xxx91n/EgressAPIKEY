const fs = require("node:fs");
const path = require("node:path");

function load(locale) {
  const file = path.join(__dirname, "..", "src", "locales", locale, "common.json");
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function flatten(obj, prefix, acc) {
  for (const [k, v] of Object.entries(obj)) {
    const key = prefix ? prefix + "." + k : k;
    if (v && typeof v === "object" && !Array.isArray(v)) {
      flatten(v, key, acc);
    } else {
      acc.add(key);
    }
  }
  return acc;
}

// Canonical catalog is en; every shipped locale must match its set of keys.
// Keep this list in sync with src/locales/ directories and the Locale union in
// src/store/appStore.ts. Adding a language means adding the directory, all en
// keys translated, AND registering it here.
const ALL = ["en", "zh", "ja", "es", "fr", "de", "ko", "ru", "pt", "ar", "it", "nl", "pl", "tr", "vi", "th", "id", "hi"];
const en = flatten(load("en"), "", new Set());
let missing = 0;
let extra = 0;
for (const lc of ALL) {
  const set = flatten(load(lc), "", new Set());
  const miss = [...en].filter((k) => !set.has(k));
  const ext = [...set].filter((k) => !en.has(k));
  if (miss.length) console.error(`Missing in ${lc}:`, miss.join(", "));
  if (ext.length) console.error(`Unexpected in ${lc}:`, ext.join(", "));
  missing += miss.length;
  extra += ext.length;
}
if (missing === 0 && extra === 0) {
  console.log(`i18n coverage OK: all ${ALL.length} locales match en (${en.size} keys)`);
  process.exit(0);
}
process.exit(1);
