#!/usr/bin/env node
// @egressapikey/server launcher.
//
// Resolves (or builds) the Rust `egressapikey-headless` binary, then spawns it
// with CLI flags so a browser-only user gets the same control surface the
// Tauri desktop app exposes. No GUI, no Tauri plugin surface.
//
// Priority for binary discovery:
//   1. `EGRESSAPIKEY_HEADLESS_BIN` env var (absolute path — used by tests
//      and CI which build outside the standard layout).
//   2. `binaries/egressapikey-headless<ext>` next to this launcher
//      (shippable npm package layout).
//   3. `../../target/release/egressapikey-headless<ext>` (dev build output).
//   4. `../../target/debug/egressapikey-headless<ext>` (dev convenience).
//
// Priority for `--dist`:
//   1. `EGRESSAPIKEY_DIST` env var.
//   2. `../../dist/` relative to this launcher.
//   3. `dist/` relative to CWD (Tauri build output convention).
//
// Ponytail: ~80 lines. CommonJS (no top-level await for Node 18 compat).

const { spawn } = require("node:child_process");
const { existsSync } = require("node:fs");
const path = require("node:path");

const EXT = process.platform === "win32" ? ".exe" : "";
const LAUNCHER_DIR = __dirname;

function resolveBin() {
  const env = process.env.EGRESSAPIKEY_HEADLESS_BIN;
  if (env && existsSync(env)) return env;
  // Build output directory discovery.
  //
  // cargo emits to `target/release/<bin>` on default-ABI hosts, but to
  // `target/<host-triple>/release/<bin>` when the host's canonical target
  // happens to differ from the flat `target/release` alias. On Windows with
  // both msvc and gnu toolchains installed, cargo uses the toolchain default
  // (msvc on mainstream Windows installs, gnu on hosts that only have MinGW).
  //
  // We can't read the host triple from Node, so we glob any subdirectory of
  // `target/` that matches `x86_64-pc-windows-*` and check the flattened
  // path first. Ponytail: one fs.readdirSync, deterministic by sort.
  const candidates = [
    path.join(LAUNCHER_DIR, "binaries", `egressapikey-headless${EXT}`),
  ];
  const targetDir = path.join(LAUNCHER_DIR, "..", "..", "target");
  const releaseFlat = path.join(targetDir, "release", `egressapikey-headless${EXT}`);
  const debugFlat = path.join(targetDir, "debug", `egressapikey-headless${EXT}`);
  candidates.push(releaseFlat);
  candidates.push(debugFlat);
  if (existsSync(targetDir)) {
    try {
      const sub = require("node:fs").readdirSync(targetDir, { withFileTypes: true });
      const triples = sub.filter(e => e.isDirectory()).map(e => e.name).filter(n =>
        /^x86_64-pc-windows-(msvc|gnu)$|^aarch64-pc-windows-(msvc|gnu)$|^x86_64-apple-darwin$|^aarch64-apple-darwin$|^x86_64-unknown-linux-gnu$|^aarch64-unknown-linux-gnu$/.test(n)
      ).sort();
      for (const t of triples) {
        candidates.push(path.join(targetDir, t, "release", `egressapikey-headless${EXT}`));
        candidates.push(path.join(targetDir, t, "debug", `egressapikey-headless${EXT}`));
      }
    } catch (_) { /* fall through */ }
  }
  for (const c of candidates) {
    if (existsSync(c)) return c;
  }
  return null;
}

function resolveDist() {
  const env = process.env.EGRESSAPIKEY_DIST;
  if (env && existsSync(env)) return env;
  const candidates = [
    path.join(LAUNCHER_DIR, "..", "..", "dist"),
    path.join(process.cwd(), "dist"),
  ];
  for (const c of candidates) {
    if (existsSync(c)) return c;
  }
  return null;
}

function main() {
  // Parse minimal CLI — every flag is forwarded to the Rust binary.
  const args = process.argv.slice(2);
  const isDryRun = args.includes("--dry-run");

  const bin = resolveBin();
  const dist = resolveDist();
  if (isDryRun) {
    console.log(JSON.stringify({
      bin: bin || "(not found)",
      dist: dist || "(not found)",
      args,
      env: {
        EGRESSAPIKEY_HEADLESS_BIN: process.env.EGRESSAPIKEY_HEADLESS_BIN || null,
        EGRESSAPIKEY_DIST: process.env.EGRESSAPIKEY_DIST || null,
      },
    }, null, 2));
    return;
  }
  if (!bin) {
    console.error("EgressAPIKEY headless binary not found. Set EGRESSAPIKEY_HEADLESS_BIN or build + ship it next to this launcher.");
    process.exit(1);
  }
  // Ensure --dist points at the resolved dist if the user did not pass it.
  if (dist && !args.some(a => a.startsWith("--dist=") || a === "--dist")) {
    args.push("--dist", dist);
  }
  const child = spawn(bin, args, { stdio: "inherit" });
  child.on("exit", (code, signal) => {
    if (signal) process.kill(process.pid, signal);
    process.exit(code ?? 0);
  });
}

main();
