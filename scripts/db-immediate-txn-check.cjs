#!/usr/bin/env node
// r12-wave-i D-004: the BEGIN-IMMEDIATE requirement for
// DbPool::replace_ports. Previously a #[test] read its own source file and
// contains()'d it - the check and the checked were the same artifact, and
// the assertion lived in the wall-clock push gate where it did not belong.
// This is a real source gate now (keyhog-scanner pattern): strip line and
// block comments, collapse ALL whitespace, then require
// transaction_with_behavior(TransactionBehavior::Immediate) inside the
// replace_ports body. Fails closed when the function or the call cannot be
// found.

const fs = require("node:fs");
const path = require("node:path");

const SRC = path.join(__dirname, "..", "crates", "resin-core", "src", "db.rs");

const raw = fs.readFileSync(SRC, "utf8");
// Comment-stripped + whitespace-collapsed view: a comment-only or fmt-only
// rewrite can never flip this gate green/red by accident. db.rs contains
// no string literal carrying '//' or '/*' today, so the naive strip is
// exact for this file.
const flat = raw
  .replace(/\/\*[\s\S]*?\*\//g, "")
  .replace(/\/\/[^\n]*/g, "")
  .replace(/\s+/g, "");

const idx = flat.indexOf("fnreplace_ports(");
if (idx < 0) {
  console.error("FAIL db-immediate-txn-check: 'fn replace_ports(' not found in crates/resin-core/src/db.rs");
  process.exit(1);
}
// Brace-match the body (string literals in this file carry no unbalanced
// braces, so the naive counter is exact here).
const open = flat.indexOf("{", idx);
let depth = 0;
let end = -1;
for (let i = open; i < flat.length; i++) {
  const ch = flat[i];
  if (ch === "{") depth++;
  else if (ch === "}") {
    depth--;
    if (depth === 0) {
      end = i;
      break;
    }
  }
}
if (end < 0) {
  console.error("FAIL db-immediate-txn-check: replace_ports body brace never closed");
  process.exit(1);
}
const body = flat.slice(open, end + 1);

if (!body.includes("transaction_with_behavior(TransactionBehavior::Immediate)")) {
  console.error(
    "FAIL db-immediate-txn-check: replace_ports lost transaction_with_behavior(TransactionBehavior::Immediate) - the write txn MUST take the write lock at BEGIN (deferred upgrade can hit SQLITE_BUSY_SNAPSHOT under WAL)",
  );
  process.exit(1);
}

console.log("db-immediate-txn-check OK: replace_ports begins TransactionBehavior::Immediate");
