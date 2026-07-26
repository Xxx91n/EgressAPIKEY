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

const en = flatten(load("en"), "", new Set());
const zh = flatten(load("zh"), "", new Set());
const missingZh = [...en].filter((k) => !zh.has(k));
const missingEn = [...zh].filter((k) => !en.has(k));

if (missingZh.length === 0 && missingEn.length === 0) {
  console.log("i18n coverage OK: en and zh in sync");
  process.exit(0);
}
if (missingZh.length) console.error("Missing in zh:", missingZh.join(", "));
if (missingEn.length) console.error("Missing in en:", missingEn.join(", "));
process.exit(1);
