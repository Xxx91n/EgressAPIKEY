#!/usr/bin/env node
// Perf baseline orchestrator.
// CI-only evidence path: .github/workflows/bench.yml runs this on pinned
// runners; local runs are dev aids, not acceptance evidence.
//
// Usage:
//   node scripts/bench/run-bench.mjs [--duration short|long]
//     [--gate warn|enforce] [--resin <path>] [--app-exe <path>]
//     [--headless-exe <path>]  (vps-headless profile: whole-tree idle + cold start)
//     [--out <dir>]
//     [--phases <csv>]  an explicit list is EXACT (no exe-gated appends).
//                       Default: idle,paired,rps,sse,soak,healthz,dbprobe
//                       plus app/appstart (--app-exe) + headless/headlessstart
//                       (--headless-exe). "faultinject" is opt-in evidence:
//                       real WAN egress required, never in the default set.
// Env: RESIN_BIN, VEGETA, BENCH_OUT

import { spawn, spawnSync } from "node:child_process";
import net from "node:net";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { roundStats, summarize, round, sleep, nowMs } from "./lib/stats.mjs";
import { startMockUpstream, startMockNode } from "./lib/mock.mjs";
import {
  findResinBinary,
  pickFreePort,
  spawnResin,
  waitHealthz,
  resinApi,
  ensureRoutable,
} from "./lib/resin.mjs";
import { Sampler } from "./lib/sampler.mjs";
import { sampleDescendantTreeMB, waitMainWindowTitle } from "./lib/winproc.mjs";
import { runLatency } from "./driver-latency.mjs";
import { runSse } from "./driver-sse.mjs";
import { runRps } from "./driver-rps.mjs";
import { runSoak } from "./driver-soak.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

const arg = (name, def) => {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : def;
};
const DURATION = arg("duration", process.env.BENCH_DURATION || "short");
const GATE = arg("gate", process.env.BENCH_GATE || "warn");
const APP_EXE = arg("app-exe", null);
const HEADLESS_EXE = arg("headless-exe", process.env.HEADLESS_BIN || null);
const FORWARDER_BIN = arg("forwarder", process.env.FORWARDER_BIN || null);
// The workspace .cargo/config.toml pins target=x86_64-pc-windows-msvc, so a
// Windows build lands under target/<triple>/release while a Linux
// --target-override build lands under its own triple dir. Resolve the first
// existing candidate instead of hardcoding a path per-runner.
function resolveForwarder() {
  const cands = [];
  if (FORWARDER_BIN) cands.push(FORWARDER_BIN);
  const rel = path.join(root, "target");
  try {
    for (const d of fs.readdirSync(rel)) {
      for (const n of ["bench-forwarder.exe", "bench-forwarder"]) {
        cands.push(path.join(rel, d, "release", n));
      }
    }
    cands.push(path.join(rel, "release", process.platform === "win32" ? "bench-forwarder.exe" : "bench-forwarder"));
  } catch {}
  return cands.find((c) => fs.existsSync(c)) ?? null;
}
const APPSTART_N = Number(process.env.BENCH_APPSTART_N || 5);
const HEADLESS_N = Number(process.env.BENCH_HEADLESS_N || 5);
const OUT = arg("out", process.env.BENCH_OUT || path.join(root, "bench-results"));
const PHASES = (arg("phases", null) ??
  "idle,paired,rps,sse,soak,healthz,dbprobe" + (APP_EXE ? ",app,appstart" : "") + (HEADLESS_EXE ? ",headless,headlessstart" : ""))
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean);
const LONG = DURATION === "long";

const log = (msg) => console.log(`[bench ${new Date().toISOString()}] ${msg}`);
const fatal = (msg) => {
  console.error(`[bench FATAL] ${msg}`);
  process.exitCode = 2;
};

fs.mkdirSync(OUT, { recursive: true });
const adminToken = crypto.randomBytes(16).toString("hex");
const identity = "bench.bench";
const children = [];

const results = {
  meta: {
    tool: "scripts/bench/run-bench.mjs",
 basis: "D-002 acceptance line (supersedes D-004)",
    startedAt: new Date().toISOString(),
    duration: DURATION,
    gate: GATE,
    os: process.platform,
    arch: process.arch,
    node: process.version,
    cpus: os.cpus().length,
    cpuModel: os.cpus()[0]?.model ?? "?",
    totalMemMB: Math.round(os.totalmem() / 1048576),
    ci: {
      runId: process.env.GITHUB_RUN_ID ?? null,
      runUrl:
        process.env.GITHUB_SERVER_URL && process.env.GITHUB_REPOSITORY && process.env.GITHUB_RUN_ID
          ? `${process.env.GITHUB_SERVER_URL}/${process.env.GITHUB_REPOSITORY}/actions/runs/${process.env.GITHUB_RUN_ID}`
          : null,
      sha: process.env.GITHUB_SHA ?? null,
      runnerOs: process.env.RUNNER_OS ?? null,
      imageVersion: process.env.ImageVersion ?? null,
    },
  },
  env: {},
  phases: {},
  gates: [],
};

function sha256Of(file) {
  try {
    return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
  } catch {
    return null;
  }
}

function killResin(r) {
  try {
    if (r?.child && r.child.exitCode === null) r.child.kill("SIGKILL");
  } catch { }
}

async function resinRssMB(pid) {
  const s = new Sampler(pid, 1000);
  await s.sampleOnce();
  const last = s.samples.at(-1);
  return last ? last.rssBytes / 1048576 : null;
}

// Shared whole-tree sampling caliber for the app/headless steady-state
// phases: settleMs settle -> 30 x 2s descendant-tree samples -> kill.
// Attribution is parentage, never name-matching (D-002); same caliber on
// both OSes (CIM descendant walk on Windows, /proc PPid BFS on Linux).
async function sampleTreeWindow(child, settleMs) {
  await sleep(settleMs);
  const alive = child.exitCode === null;
  const samples = [];
  const wv2Samples = [];
  let lastProcs = null;
  for (let i = 0; i < 30; i++) {
    const tree = await sampleDescendantTreeMB(child.pid);
    if (tree != null && tree.totalMB > 0) {
      samples.push(tree.totalMB);
      wv2Samples.push(tree.webview2MB);
      lastProcs = tree.procs;
    }
    await sleep(2000);
  }
  killTree(child);
  return { alive, samples, wv2Samples, lastProcs };
}

// ---- phases ---------------------------------------------------------------

async function phaseIdle(ctx) {
  // Whole-SUT idle caliber: every data-plane process the bench spawned —
  // sidecar + shell forwarder when present (the VPS profile's real tree is
  // egressapikey-headless + sidecar; this harness measures the data-plane
  // set it actually drives, see acceptance.json env note).
  await sleep(15000); // settle past startup churn
  const s = new Sampler(ctx.sutPids ?? [ctx.resinPid], 2000).start();
  await sleep(60000);
  const st = await s.stop();
  return { windowS: 60, rssMB: st.rssMB, cpuCores: st.cpuCores, cpuPct: st.cpuPctOfOneCore, n: st.n, pids: ctx.sutPids ?? [ctx.resinPid] };
}

async function phaseHealthz(ctx) {
  // kill the configured sidecar, then time N fresh spawns -> first healthz 200
  killResin(ctx.resin);
  await sleep(1500);
  const samples = [];
  for (let i = 0; i < 5; i++) {
    const workDir = fs.mkdtempSync(path.join(os.tmpdir(), "bench-resin-hz-"));
    const t0 = nowMs();
    const r = spawnResin({
      bin: ctx.bin,
      workDir,
      apiPort: ctx.apiPort,
      adminToken,
      cpuPin: ctx.cpuPin,
    });
    const ms = await waitHealthz(ctx.apiPort, 30000, t0);
    samples.push(round(ms, 1));
    killResin(r);
    await sleep(800);
  }
  const st = summarize(samples);
  return { coldStartMs: ctx.coldStartMs, warmSamplesMs: samples, p50: round(st.p50, 1), p95: round(st.p95, 1), max: round(st.max, 1) };
}

async function phaseApp(ctx) {
  // Whole-app steady state: launch the built exe, let it reach steady state,
  // then sum WorkingSet64 over the DESCENDANT TREE rooted at the spawned PID.
  // Attribution is parentage, never process-name matching: WebView2
  // (msedgewebview2) is a shared runtime whose processes pool per
  // user-data-dir and re-parent across app boundaries, so a name glob would
 // count other apps' webview processes (D-002).
  if (process.platform !== "win32") {
    return { skipped: "whole-app steady-state is measured on the windows job" };
  }
  if (!APP_EXE || !fs.existsSync(APP_EXE)) {
    return { skipped: `--app-exe missing or not found: ${APP_EXE}` };
  }
  const child = spawn(APP_EXE, [], { stdio: "ignore", detached: true });
  children.push(child);
  // 25s = boot + first paint + settle
  const win = await sampleTreeWindow(child, 25000);
  const st = summarize(win.samples);
  const wv2 = summarize(win.wv2Samples);
  return {
    appExe: APP_EXE,
    appAlive: win.alive,
    childPid: child.pid,
    attribution: "descendant-tree",
    n: win.samples.length,
    totalMB: roundStats(st),
    webview2MB: roundStats(wv2),
    procs: win.lastProcs,
  };
}

// Acceptance-line metric ⑦: cold start -> MainWindowTitle ready, N fresh
// launches, p50/p95. Windows-only (MainWindowTitle is a Win32 property);
// Defender/cold-disk variance is why the gate is a distribution, not one run.
async function phaseAppStart(ctx) {
  if (process.platform !== "win32") {
    return { skipped: "MainWindowTitle liveness is a windows-desktop metric" };
  }
  if (!APP_EXE || !fs.existsSync(APP_EXE)) {
    return { skipped: `--app-exe missing or not found: ${APP_EXE}` };
  }
  const samples = [];
  const titles = [];
  for (let i = 0; i < APPSTART_N; i++) {
    const child = spawn(APP_EXE, [], { stdio: "ignore", detached: true });
    children.push(child);
    const ready = await waitMainWindowTitle(child.pid, 30000);
    samples.push(ready ? ready.ms : null);
    titles.push(ready ? ready.title : null);
    killTree(child);
    await sleep(1500); // settle between cold launches (Defender / disk cache)
  }
  const ok = samples.filter((v) => v != null);
  const st = summarize(ok);
  return {
    appExe: APP_EXE,
    n: ok.length,
    attempted: APPSTART_N,
    titles,
    titleMatch: titles.filter((t) => t === "EgressAPIKEY").length,
    coldStartMs: roundStats(st),
  };
}

// ---- headless (vps-headless profile) ---------------------------------------

// TCP-connectable readiness: the headless control surface binds ONLY after
// the resin sidecar has booted (headless_main.rs binds post-boot), so a
// connect() on bind:port is the "tree is up" signal - the headless
// analogue of MainWindowTitle liveness.
async function waitTcpReady(port, deadlineMs, t0 = nowMs()) {
  for (;;) {
    const ok = await new Promise((res) => {
      const sk = net.connect(port, "127.0.0.1", () => {
        sk.destroy();
        res(true);
      });
      sk.on("error", () => res(false));
      sk.setTimeout(400, () => {
        sk.destroy();
        res(false);
      });
    });
    if (ok) return nowMs() - t0;
    if (nowMs() - t0 > deadlineMs) return null;
    await sleep(150);
  }
}

// Kill a spawned headless process AND its whole tree: taskkill /T on
// Windows, the detached process group (kill -pid) elsewhere. Descendant
// boundary matches the sampling caliber exactly (R12-B2).
function killTree(child) {
  try {
    if (process.platform === "win32") {
      spawn("taskkill", ["/F", "/T", "/PID", String(child.pid)]);
    } else if (child.exitCode === null) {
      try {
        process.kill(-child.pid, "SIGKILL");
      } catch {
        child.kill("SIGKILL");
      }
    }
  } catch { }
}

function spawnHeadless(port, token, stateDir, ctx) {
  return spawn(
    HEADLESS_EXE,
    [
      "--bind", "127.0.0.1",
      "--port", String(port),
      "--auth-token", token,
      "--no-browser",
      "--state-root", stateDir,
      "--log-root", path.join(stateDir, "logs"),
      // the resin-* sidecar glob resolves inside the bench resin dir
      "--binary-dir", path.dirname(ctx.bin),
    ],
    // detached -> own process group on POSIX so killTree can signal the
    // whole descendant set (headless + sidecar) with kill(-pid).
    { stdio: "ignore", detached: true },
  );
}

// VPS profile whole-tree idle (R12-B2): spawn egressapikey-headless, wait
// for its control surface (sidecar already up by then), settle, then sum
// RSS over the DESCENDANT TREE every 2s for 60s - same attribution caliber
// as the desktop app phase (PPid BFS on Linux, CIM descendant walk on
// Windows; a cgroup boundary was considered and rejected: the bench spawns
// ad-hoc children without a dedicated cgroup, and PPid-BFS needs no root).
async function phaseHeadless(ctx) {
  if (!HEADLESS_EXE || !fs.existsSync(HEADLESS_EXE)) {
    return { skipped: "--headless-exe missing or not found: " + HEADLESS_EXE };
  }
  const port = await pickFreePort();
  const token = crypto.randomBytes(16).toString("hex");
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "bench-headless-"));
  const t0 = nowMs();
  const child = spawnHeadless(port, token, stateDir, ctx);
  children.push(child);
  const readyMs = await waitTcpReady(port, 90000, t0);
  if (readyMs == null) {
    killTree(child);
    return { skipped: "headless control surface never bound in 90s", port };
  }
  const win = await sampleTreeWindow(child, 15000); // settle past boot churn (same caliber as phases.idle)
  const st = summarize(win.samples);
  return {
    headlessExe: HEADLESS_EXE,
    port,
    readyMs: round(readyMs, 1),
    attribution: "descendant-tree",
    n: win.samples.length,
    totalMB: roundStats(st),
    procs: win.lastProcs,
  };
}

// VPS profile cold start (R12-B2): N fresh headless launches -> control
// surface TCP-ready (sidecar boot included inside the timed span).
async function phaseHeadlessStart(ctx) {
  if (!HEADLESS_EXE || !fs.existsSync(HEADLESS_EXE)) {
    return { skipped: "--headless-exe missing or not found: " + HEADLESS_EXE };
  }
  const token = crypto.randomBytes(16).toString("hex");
  const samples = [];
  for (let i = 0; i < HEADLESS_N; i++) {
    const port = await pickFreePort();
    const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "bench-headless-hz-"));
    const t0 = nowMs();
    const child = spawnHeadless(port, token, stateDir, ctx);
    children.push(child);
    const ms = await waitTcpReady(port, 90000, t0);
    samples.push(ms == null ? null : round(ms, 1));
    killTree(child);
    await sleep(1500); // settle between cold launches
  }
  const ok = samples.filter((v) => v != null);
  return {
    headlessExe: HEADLESS_EXE,
    n: ok.length,
    attempted: HEADLESS_N,
    samples,
    coldStartMs: roundStats(summarize(ok)),
  };
}

// r12-wave-i D-004: the two duration assertions that used to live in the
// cargo-test push gate (replace_ports p99 <= 50ms; lock-wait probe p99 <
// 250ms hang bound) record here as measurement rows instead - the phase
// runs the measurement tests and parses their DBPROBE key=value markers.
// kind=measure semantics apply: numbers are recorded evidence, never a
// job failure. Needs cargo on the runner (bench jobs install stable).
async function phaseDbProbe() {
  const marks = {};
  const errors = [];
  for (const filter of [
    "replace_ports_commit_latency_measurement",
    "concurrent_list_ports_lock_wait_probe_reports_p99",
  ]) {
    const r = spawnSync(
      "cargo",
      ["test", "-p", "resin-core", "--features", "db-lock-metrics", "--lib", filter, "--", "--nocapture", "--test-threads=1"],
      { cwd: root, encoding: "utf8", timeout: 600000 },
    );
    const text = String(r.stdout ?? "") + "\n" + String(r.stderr ?? "");
    for (const m of text.matchAll(/DBPROBE\s+([^\n]+)/g)) {
      for (const kv of m[1].matchAll(/(\w+)=([^\s()]+)/g)) {
        marks[kv[1]] = kv[2];
      }
    }
    if (r.status !== 0 && r.status !== null) {
      errors.push(filter + ": exit " + r.status + (r.error ? " (" + r.error + ")" : ""));
    } else if (r.error) {
      errors.push(filter + ": " + r.error);
    }
  }
  const num = (k) => (marks[k] != null ? Number(marks[k]) : null);
  const out = {
    replacePortsP99Us: num("replace_ports_p99_us"),
    lockWaitP99Us: num("lock_wait_p99_us"),
    lockWaitMaxUs: num("max_us"),
    lockWaitOverThreshold: num("over_threshold"),
    lockWaitOverMagnitude: num("over_magnitude"),
    readWaitOverThreshold: num("read_over_threshold"),
  };
  if (errors.length) out.error = errors.join("; ");
  return out;
}

// ---- gate evaluation ------------------------------------------------------

function resolvePath(obj, dotted) {
  return dotted.split(".").reduce((o, k) => (o == null ? undefined : o[k]), obj);
}

function evalGates(acc) {
  const gates = [];
  for (const item of acc.items) {
    const measured =
      resolvePath(results, item.measure) ??
      (item.measureAlt ? resolvePath(results, item.measureAlt) : undefined);
    // Two-layer single-metric calibers may carry a separate measure path
    // for the danger layer (e.g. the same good-event ratio evaluated at a
    // wider threshold window).
    const dangerMeasured = item.dangerMeasure
      ? resolvePath(results, item.dangerMeasure) ?? measured
      : measured;
    const kind = item.kind ?? "measure";
    const target = item.target ?? item.threshold;
    const danger = item.danger ?? null;
    const cmp = (v, th) =>
      item.op === "<=" ? v <= th
        : item.op === "<" ? v < th
          : item.op === ">=" ? v >= th
            : item.op === "==" ? v === th
              : false;
    let status;
    if (item.exempt) {
      // Registered Exemption row: the measurement is evidence, not a verdict.
      status = "exempt";
    } else if (item.op === "measure-only") {
      status = measured == null ? "pending" : "measured";
    } else if (measured == null || !Number.isFinite(measured)) {
      status = "n/a";
    } else if (cmp(measured, target)) {
      status = "pass";
    } else if (danger != null && dangerMeasured != null && cmp(dangerMeasured, danger)) {
      status = "degraded"; // between target and danger line
    } else {
      status = "breach";
    }
    gates.push({
      id: item.id,
      desc: item.desc,
      kind,
      op: item.op,
      target,
      danger,
      unit: item.unit,
      scope: item.scope,
      measured: measured ?? null,
      status,
      exempt: item.exempt ? item.exemption ?? true : undefined,
    });
  }
  return gates;
}

// ---- faultinject (R12-C1): opt-in evidence phase ---------------------------

// SUT = egressapikey-headless (Mode B transport) driving the ADR-0080
// signal-plane common-mode suppressor under REAL fault injection. The
// assertions ARE the evidence: a breach is a hard fail (fatal), never a
// warn-only measurement. Opt-in only - never in the default phase set or
// the verify gate.
//
// Fixture (fresh state root per run, seeded BEFORE the SUT spawns):
//   egressapikey.db - written directly, bypassing the establish cascade
//     (the legal fixture path): 5 platforms x 1 enabled `mixed` port each.
//     The probe plane enumerates from DbPool::list_ports(), not a whitebox
//     shortcut. n=5 is the minimum non-trivial suppressor cell
//     (ceil(0.8*5)=4 local fails needed to trip it).
//   egressapikey-strategy.json - orchestration { enabled, autonomy:suggest,
//     slow_call_ms:9000 }: suggest parks every transition so Auto-tier
//     migration stays outside the assertion surface; the inflated
//     slow_call_ms absorbs shared-runner WAN jitter (the probe client
//     timeout is 10s).
//
// Dataplane provenance (D-002): each row port is materialised by a
// bench-forwarder child - a real Mode A shell-side entry - relaying into an
// endpoint on the SUT's OWN resin sidecar (configured through the headless
// admin-proxy surface), exiting via the shared mock CONNECT node to the
// real WAN (probe_exit_ip is hardcoded to 1.1.1.1/cdn-cgi/trace). The
// mock.mjs "zero internet" carve-out does NOT apply to this phase.
//
// Injection semantics (D-002):
//   kill-forwarder == bind-refusal / Mode A entry failure class: loopback
//     dead -> local_fail common-mode -> EnvironmentSuspect (suppressor on).
//   kill-node == egress-path failure behind a live entry: loopback ok +
//     egress dead -> remote_fail; the suppressor must stay OFF.
//   kill-sidecar is NOT a valid injection: both probes die together and it
//     would stamp local_fail (rejected in the ticket provenance).

// Write the fixture state root (db rows + strategy whitebox). The db is
// created at user_version=4 so the SUT's migrator sees the shipping schema.
function writeFaultFixture(stateDir, ports, platformNames, DatabaseSync) {
  const db = new DatabaseSync(path.join(stateDir, "egressapikey.db"));
  db.exec(
    "CREATE TABLE port_mappings (" +
      "port INTEGER PRIMARY KEY, protocol TEXT NOT NULL DEFAULT 'mixed'," +
      " platform_name TEXT NOT NULL, account TEXT NOT NULL," +
      " label TEXT NOT NULL DEFAULT '', enabled INTEGER NOT NULL DEFAULT 1," +
      " auth_required INTEGER NOT NULL DEFAULT 1);",
  );
  const ins = db.prepare(
    "INSERT INTO port_mappings" +
      " (port, protocol, platform_name, account, label, enabled, auth_required)" +
      " VALUES (?, 'mixed', ?, 'fi', 'faultinject', 1, 0)",
  );
  ports.forEach((p, i) => ins.run(p, platformNames[i]));
  db.exec("PRAGMA user_version = 4;");
  db.close();
  fs.writeFileSync(
    path.join(stateDir, "egressapikey-strategy.json"),
    JSON.stringify(
      {
        version: 1,
        platforms: platformNames.map((n) => ({
          platform_name: n,
          a_class: "region",
          b_class: "BALANCED",
          regions: ["us"],
        })),
        orchestration: {
          params: { enabled: true, autonomy: "suggest", slow_call_ms: 9000 },
        },
      },
      null,
      2,
    ) + "\n",
  );
}

// Poll until every port REFUSES a loopback connect, so the first injected
// tick cannot observe a half-dead window (kill signals are async).
async function waitAllClosed(ports, deadlineMs, t0 = nowMs()) {
  for (;;) {
    const states = await Promise.all(
      ports.map(
        (p) =>
          new Promise((res) => {
            const sk = net.connect(p, "127.0.0.1", () => {
              sk.destroy();
              res(true);
            });
            sk.on("error", () => res(false));
            sk.setTimeout(800, () => {
              sk.destroy();
              res(true); // a hung dial is not evidence of refusal
            });
          }),
      ),
    );
    if (states.every((open) => !open)) return nowMs() - t0;
    if (nowMs() - t0 > deadlineMs) return null;
    await sleep(150);
  }
}

async function phaseFaultInject(ctx) {
  const fwdBin = resolveForwarder();
  if (!fwdBin) {
    return {
      skipped:
        "bench-forwarder binary not found (FORWARDER_BIN or target/**/release) - faultinject skipped",
    };
  }
  if (!HEADLESS_EXE || !fs.existsSync(HEADLESS_EXE)) {
    return { skipped: `--headless-exe missing or not found: ${HEADLESS_EXE ?? "(unset)"}` };
  }
  let DatabaseSync;
  try {
    ({ DatabaseSync } = await import("node:sqlite"));
  } catch (e) {
    return { skipped: `node:sqlite unavailable (${e.message}) - faultinject skipped` };
  }

  const failures = [];
  const assert = (cond, msg) => {
    if (!cond) {
      failures.push(msg);
      log(`faultinject ASSERT-FAIL: ${msg}`);
    }
    return cond;
  };

  // n=5: ceil(0.8*5)=4 -> the minimum cell where the suppressor is non-trivial.
  const PLATFORM_NAMES = ["fi-p1", "fi-p2", "fi-p3", "fi-p4", "fi-p5"];
  const rowPorts = [];
  for (let i = 0; i < PLATFORM_NAMES.length; i++) rowPorts.push(await pickFreePort());
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "egressapikey-fi-"));
  writeFaultFixture(stateDir, rowPorts, PLATFORM_NAMES, DatabaseSync);

  const hp = await pickFreePort();
  const token = crypto.randomBytes(12).toString("hex");
  const sut = spawnHeadless(hp, token, stateDir, ctx);
  children.push(sut);

  const t0 = nowMs();
  const timeline = [];
  const verdictMap = (r) =>
    Object.fromEntries((r?.platform_verdicts ?? []).map((v) => [v.platform, v.verdict]));
  const allAre = (r, want) => PLATFORM_NAMES.every((n) => verdictMap(r)[n] === want);
  const tick = async () => {
    const res = await fetch(`http://127.0.0.1:${hp}/api/v1/shell/orchestration/tick`, {
      method: "POST",
      headers: { authorization: `Bearer ${token}` },
    });
    if (!res.ok) throw new Error(`tick http ${res.status}: ${(await res.text()).slice(0, 200)}`);
    return res.json();
  };
  const snap = (tag, r) =>
    timeline.push({
      tag,
      tMs: nowMs() - t0,
      streak: r?.environment_status?.suspect_streak ?? null,
      verdicts: verdictMap(r),
      details: r?.verdict_details ?? [],
      executedActions: (r?.actions ?? []).filter((a) => a.ok),
    });

  // The control surface binds only after the SUT's sidecar boots
  // (headless_main binds post-boot), so connectable == whole tree up.
  const readyMs = await waitTcpReady(hp, 90000);
  if (readyMs == null) {
    killTree(sut);
    return { skipped: "headless control surface never bound within 90s", stateDir };
  }

  // Wire the SUT's own sidecar dataplane through the shared mock node via
  // the headless admin-proxy surface (Bearer <auth-token>; the proxy injects
  // the resin admin token). Probe path: row port -> forwarder -> SUT resin
  // endpoint -> mock CONNECT node -> 1.1.1.1 (real egress).
  const sutApi = resinApi(hp, token);
  const sutEpPort = await pickFreePort();
  const route = await ensureRoutable({
    api: sutApi,
    nodePort: ctx.mockNode.port,
    epPort: sutEpPort,
    upstreamPort: ctx.upstreamPort,
  });

  // Materialise the five row ports as killable Mode A shell-side entries.
  const spawnEntries = async () => {
    const procs = rowPorts.map((p) => {
      const c = spawn(fwdBin, [], {
        env: {
          ...process.env,
          BENCH_FWD_PORT: String(p),
          BENCH_FWD_ENGINE_PORT: String(sutEpPort),
          BENCH_FWD_PLATFORM: "bench",
          BENCH_FWD_ACCOUNT: "bench",
          BENCH_FWD_PROXY_TOKEN: "",
          RUST_LOG: "warn",
        },
        stdio: "ignore",
      });
      children.push(c);
      return c;
    });
    for (const p of rowPorts) {
      if ((await waitTcpReady(p, 15000)) == null) {
        throw new Error(`entry forwarder :${p} never bound`);
      }
    }
    return procs;
  };
  let entries = await spawnEntries();

  // 1. baseline: all platforms ok (one retry - WAN jitter is the documented
  //    flake source; the retry is the ticket's mitigation).
  let r = await tick();
  snap("baseline-1", r);
  if (!allAre(r, "ok")) {
    await sleep(4000);
    r = await tick();
    snap("baseline-2", r);
  }
  assert(allAre(r, "ok"), `baseline not all ok: ${JSON.stringify(verdictMap(r))}`);

  // 2. kill-forwarder -> three manual ticks. Dead entries refuse the
  //    loopback probe -> local_fail common-mode -> suppressor stamps
  //    environment_suspect; the streak must climb monotonically to >=3 (the
  //    audit row fires at the crossing tick). The 60s driver cannot produce
  //    a clean tick inside this window, so driver interleavings only ever
  //    add suspect ticks - the assertions stay monotonic-safe.
  for (const c of entries) killTree(c);
  // Trace only, never an acceptance signal (r12 wave-d): killTree already
  // proved the children are dead; port closure can lag milliseconds behind
  // process teardown. The wait is diagnostic output, not a gate.
  const allClosedMs = await waitAllClosed(rowPorts, 15000);
  log(
    allClosedMs === null
      ? "waitAllClosed: entry ports still answering after the 15s deadline - continuing"
      : `entry ports refused within ${allClosedMs}ms`,
  );
  const suspectTicks = [];
  // ADR-0080: the suspect-streak audit row fires when the streak CROSSES 3
  // (hardwired threshold - anti-Goodhart). Pinned as a named constant so the
  // assertions read against the legislation, not a bare literal; three
  // manual ticks are required to cross it.
  const SUSPECT_AUDIT_STREAK = 3;
  for (let i = 0; i < SUSPECT_AUDIT_STREAK; i++) {
    const t = await tick();
    snap(`suspect-${i + 1}`, t);
    suspectTicks.push(t);
  }
  const streaks = suspectTicks.map((t) => t?.environment_status?.suspect_streak ?? -1);
  assert(
    streaks.every((s, i) => s >= 1 && (i === 0 || s >= streaks[i - 1])),
    `suspect streak not monotonic non-decreasing >=1: ${streaks.join(",")}`,
  );
  assert(
    streaks.at(-1) >= SUSPECT_AUDIT_STREAK,
    `suspect streak did not cross ${SUSPECT_AUDIT_STREAK} within ${SUSPECT_AUDIT_STREAK} injected ticks: ${streaks.join(",")}`,
  );
  suspectTicks.forEach((t, i) =>
    assert(
      allAre(t, "environment_suspect"),
      `suspect tick ${i + 1} verdicts not all environment_suspect: ${JSON.stringify(verdictMap(t))}`,
    ),
  );

  // 3. restore entries -> poll until probes go ok again (forwarder bind is
  //    near-instant; the first clean tick resets the streak to 0).
  entries = await spawnEntries();
  let restored = null;
  for (let i = 0; i < 15; i++) {
    const t = await tick();
    snap(`restore-${i}`, t);
    if (allAre(t, "ok")) {
      restored = t;
      break;
    }
    await sleep(4000);
  }
  assert(restored != null, "entries restored but probes never went ok within ~60s");
  assert(
    (restored?.environment_status?.suspect_streak ?? -1) === 0,
    `suspect streak did not reset on the clean tick: ${restored?.environment_status?.suspect_streak}`,
  );

  // 4. kill-node -> one tick. Entries stay up (loopback ok) but the egress
  //    path dies at the CONNECT hop -> remote_fail; the suppressor must NOT
  //    engage and the streak must stay 0.
  await ctx.mockNode.kill();
  const rr = await tick();
  snap("remote-fail", rr);
  assert(
    allAre(rr, "remote_fail"),
    `node kill must stamp remote_fail on every platform, got ${JSON.stringify(verdictMap(rr))}`,
  );
  assert(
    (rr?.environment_status?.suspect_streak ?? -1) === 0,
    `remote_fail must not move the suspect streak: ${rr?.environment_status?.suspect_streak}`,
  );

  // 5. restart the node on the same port and drive Resin's own health
  //    probes until the endpoint is routable again (the external probe
  //    cadence is engine-side - the tick verdict is the truth source).
  ctx.mockNode = await startMockNode(ctx.mockNode.port);
  let recovered = null;
  for (let i = 0; i < 24; i++) {
    for (const act of ["probe-egress", "probe-latency"]) {
      try {
        await sutApi.post(`/nodes/${route.nodeHash}/actions/${act}`, {});
      } catch {}
    }
    const t = await tick();
    snap(`recover-${i}`, t);
    if (allAre(t, "ok")) {
      recovered = t;
      break;
    }
    await sleep(5000);
  }
  assert(recovered != null, "node restarted but never routable again within ~120s");
  assert(
    (recovered?.environment_status?.suspect_streak ?? -1) === 0,
    `final clean tick must keep suspect streak at 0: ${recovered?.environment_status?.suspect_streak}`,
  );

  // 6. no migration may have executed anywhere in the run: under suggest the
  //    evaluator parks every transition, so an ok:true switch row would mean
  //    a real set_platform_regions+apply landed mid-evidence.
  const migrated = timeline
    .flatMap((s) => s.executedActions)
    .filter((a) => a.action === "switch" && a.ok === true);
  assert(migrated.length === 0, `executed migrations during evidence run: ${JSON.stringify(migrated)}`);

  // 7. audit.jsonl: exactly one signal-plane/environment_suspect/orchestration:tick
  //    row for the whole crossing (streak 4/5 must NOT re-emit). Other audit
  //    rows (per-tick strategy status writes) are expected and ignored.
  let envRows = [];
  try {
    envRows = fs
      .readFileSync(path.join(stateDir, "audit.jsonl"), "utf8")
      .split(/\r?\n/)
      .filter(Boolean)
      .map((l) => {
        try {
          return JSON.parse(l);
        } catch {
          return null;
        }
      })
      .filter(
        (r) =>
          r?.target === "signal-plane" &&
          r?.op === "environment_suspect" &&
          r?.actor === "orchestration:tick",
      );
  } catch {}
  assert(
    envRows.length === 1,
    `expected exactly 1 signal-plane/environment_suspect/orchestration:tick audit row, got ${envRows.length}`,
  );

  killTree(sut);
  const evidence = {
    sut: "egressapikey-headless",
    probePath:
      "row-port -> bench-forwarder (Mode A entry) -> SUT resin endpoint -> mock CONNECT node -> 1.1.1.1/cdn-cgi/trace (real WAN)",
    platforms: PLATFORM_NAMES,
    rowPorts,
    stateDir,
    headlessReadyMs: readyMs,
    sutEndpointPort: sutEpPort,
    sutNodeHash: route.nodeHash,
    timeline,
    envAuditRow: envRows[0] ?? null,
    failures,
    pass: failures.length === 0,
  };
  if (failures.length) {
    fatal(`faultinject: ${failures.length} assertion(s) failed -> ${failures.join(" | ")}`);
  }
  return evidence;
}

// ---- main -----------------------------------------------------------------

async function main() {
  const bin = arg("resin", process.env.RESIN_BIN) ?? findResinBinary(root);
  results.env.resinBinary = bin;
  results.env.resinSha256 = sha256Of(bin);
  // Default: unpinned (matches production — the app spawns the sidecar
  // unpinned). Single-core pinning via BENCH_CPU_PIN=1 exists for controlled
  // experiments but makes results host-contention-sensitive.
  const cpuPin = process.env.BENCH_CPU_PIN
    ? process.platform === "linux"
      ? { taskset: "0", gomaxprocs: 1 }
      : process.platform === "win32"
        ? { affinity: 1, gomaxprocs: 1 }
        : { gomaxprocs: 1 }
    : null;
  results.env.cpuPin = cpuPin;
  results.env.phases = PHASES;
  log(`resin=${bin} sha256=${results.env.resinSha256?.slice(0, 12)} cpuPin=${JSON.stringify(cpuPin)}`);

  const up = await startMockUpstream(0);
  const node = await startMockNode(0);
  const apiPort = await pickFreePort();
  const epPort = await pickFreePort();
  results.env.ports = { upstream: up.port, node: node.port, api: apiPort, endpoint: epPort };
  log(`mock upstream=:${up.port} node=:${node.port} api=:${apiPort} endpoint=:${epPort}`);

  const workDir = fs.mkdtempSync(path.join(os.tmpdir(), "bench-resin-"));
  const t0 = nowMs();
  let resin = spawnResin({ bin, workDir, apiPort, adminToken, cpuPin });
  children.push(resin.child);
  const coldStartMs = await waitHealthz(apiPort, 30000, t0);
  log(`healthz ready in ${round(coldStartMs, 1)}ms (cold)`);

  // Mode A shell forwarder (bench-forwarder bin): the desktop data-plane path
  // the acceptance line gates. Absent binary -> Mode A phases record skipped.
  let forwarder = null;
  let modeAPort = null;
  const fwdBin = resolveForwarder();
  if (fwdBin) {
    modeAPort = await pickFreePort();
    forwarder = spawn(fwdBin, [], {
      env: {
        ...process.env,
        BENCH_FWD_PORT: String(modeAPort),
        BENCH_FWD_ENGINE_PORT: String(epPort),
        BENCH_FWD_PLATFORM: "bench",
        BENCH_FWD_ACCOUNT: "bench",
        BENCH_FWD_PROXY_TOKEN: "",
        RUST_LOG: "warn",
      },
      stdio: ["ignore", "pipe", "pipe"],
    });
    children.push(forwarder);
    forwarder.stdout.on("data", (d) => log(`forwarder: ${String(d).trim()}`));
    forwarder.stderr.on("data", (d) => log(`forwarder-err: ${String(d).trim()}`));
    // readiness: TCP connectable (same waitTcpReady caliber as headless)
    const boundMs = await waitTcpReady(modeAPort, 15000);
    if (boundMs == null) modeAPort = null;
    log(modeAPort ? `mode-A forwarder on :${modeAPort}` : "forwarder never bound - modeA skipped");
  } else {
    log(`no bench-forwarder under target/**/release (FORWARDER_BIN=${FORWARDER_BIN}) - modeA legs skipped`);
  }

  const api = resinApi(apiPort, adminToken);
  log("configuring platform+subscription+endpoint + node probes...");
  const setup = await ensureRoutable({
    api,
    nodePort: node.port,
    epPort,
    upstreamPort: up.port,
  });
  results.phases.setup = { ...setup, coldStartMs: round(coldStartMs, 1) };
  log(`routable: nodeHash=${setup.nodeHash} endpoint=${setup.endpointId} probe=${setup.probe}`);

  const ctx = {
    epPort,
    upstreamPort: up.port,
    identity,
    upStatsUrl: `http://127.0.0.1:${up.port}/__stats`,
    resinPid: resin.pid,
    resin,
    sutPids: [resin.pid, forwarder?.pid].filter((p) => p != null),
    apiPort,
    bin,
    cpuPin,
    coldStartMs,
    outDir: OUT,
    resinAlive: async () => resin.child.exitCode === null,
    resinRssMB: () => resinRssMB(resin.pid),
    modeAPort,
    mockNode: node,
  };

  const runPhase = async (name, fn) => {
    if (!PHASES.includes(name)) return;
    log(`phase ${name} start`);
    try {
      results.phases[name] = await fn(ctx);
      log(`phase ${name} done: ${JSON.stringify(results.phases[name]).slice(0, 300)}`);
    } catch (e) {
      results.phases[name] = { error: String(e?.stack ?? e) };
      // faultinject assertions are evidence, not measurements: an unexpected
      // throw (e.g. ensureRoutable timing out) must turn the job red.
      if (name === "faultinject") fatal(`phase ${name} threw: ${String(e?.message ?? e)}`);
      console.error(`[bench ERROR] phase ${name}:`, e);
    }
  };

  await runPhase("idle", (c) => phaseIdle(c));
  await runPhase("paired", (c) =>
    runLatency({ epPort: c.epPort, upstreamPort: c.upstreamPort, identity: c.identity, pairs: 600, warmup: 60 }),
  );
  await runPhase("rps", (c) => runRps(c, { rate: 500, durationS: LONG ? 300 : 60 }));
  await runPhase("sse", (c) => runSse(c));
  await runPhase("soak", (c) =>
    runSoak(c, {
      streams: Number(process.env.BENCH_SOAK_STREAMS || 200),
      durationS: Number(process.env.BENCH_SOAK_SECONDS || (LONG ? 1800 : 240)),
      interval: Number(process.env.BENCH_SOAK_INTERVAL_MS || 500),
    }),
  );
  await runPhase("healthz", (c) => phaseHealthz(c));
  await runPhase("app", (c) => phaseApp(c));
  await runPhase("appstart", (c) => phaseAppStart(c));
  await runPhase("headless", (c) => phaseHeadless(c));
  await runPhase("headlessstart", (c) => phaseHeadlessStart(c));
  await runPhase("dbprobe", () => phaseDbProbe());
  await runPhase("faultinject", (c) => phaseFaultInject(c));

  // gates
  const acc = JSON.parse(fs.readFileSync(path.join(root, "scripts/bench/acceptance.json"), "utf8"));
  results.gates = evalGates(acc);
  results.acceptance = acc;

 // Failure semantics (D-002): measurement numbers are always RECORDED -
  // a breach is evidence, not a job failure, until representative-hardware
  // pins activate. Only assertion/smoke rows hard-fail under --gate enforce.
  const fails = results.gates.filter((g) => g.status === "breach");
  const hardFails = fails.filter((g) => g.kind !== "measure");
  const measured = results.gates.filter((g) => g.status === "measured");
  const exempt = results.gates.filter((g) => g.status === "exempt");
  log(`gates: ${results.gates.filter((g) => g.status === "pass").length} pass, ${fails.length} breach (${hardFails.length} assert/smoke), ${results.gates.filter((g) => g.status === "degraded").length} degraded, ${exempt.length} exempt, ${measured.length} measured-only`);
  if (GATE === "enforce" && hardFails.length > 0) {
    fatal(`${hardFails.length} assertion/smoke gate(s) breached: ${hardFails.map((g) => g.id).join(", ")}`);
    process.exitCode = 1;
  }

  // summary markdown
  const lines = [
    `# perf-baseline summary`,
    ``,
    `- os: ${results.meta.os}/${results.meta.arch} node=${results.meta.node} cpus=${results.meta.cpus}`,
    `- resin: ${bin} sha256=${results.env.resinSha256?.slice(0, 16)}`,
    `- cpuPin: ${JSON.stringify(cpuPin)}  duration=${DURATION} gate=${GATE}`,
    `- ci run: ${results.meta.ci.runUrl ?? "(local)"}`,
    ``,
    `## Measurements (recorded; breaches are warn-only until representative-hardware pins activate)`,
    ``,
    `| item | op | target | danger | measured | unit | status |`,
    `|---|---|---|---|---|---|---|`,
    ...results.gates.filter((g) => g.kind === "measure").map(
      (g) => `| ${g.id} | ${g.op} | ${g.target ?? "-"} | ${g.danger ?? "-"} | ${g.measured ?? "n/a"} | ${g.unit} | ${g.status} |`,
    ),
    ``,
    `## Assertions & smoke (the only rows that hard-fail under --gate enforce)`,
    ``,
    `| item | op | target | measured | unit | status |`,
    `|---|---|---|---|---|---|`,
    ...results.gates.filter((g) => g.kind !== "measure").map(
      (g) => `| ${g.id} | ${g.op} | ${g.target ?? "-"} | ${g.measured ?? "n/a"} | ${g.unit} | ${g.status} |`,
    ),
    ``,
    ...(results.phases.sse?.exemptions?.length
      ? [
          `## Registered exemptions`,
          ``,
          ...results.phases.sse.exemptions.map(
            (x) => `- **${x.id}** (${x.scope}) — removal: ${x.removalCondition}`,
          ),
        ]
      : []),
  ];
  fs.writeFileSync(path.join(OUT, "SUMMARY.md"), lines.join("\n") + "\n");
  fs.writeFileSync(path.join(OUT, "results.json"), JSON.stringify(results, null, 2));
  log(`wrote ${path.join(OUT, "results.json")} + SUMMARY.md`);
}

main()
  .catch((e) => {
    fatal(String(e?.stack ?? e));
    results.phases.fatal = String(e?.stack ?? e);
    try {
      fs.writeFileSync(path.join(OUT, "results.json"), JSON.stringify(results, null, 2));
    } catch { }
  })
  .finally(async () => {
    for (const c of children) {
      try {
        if (c.exitCode === null) c.kill("SIGKILL");
      } catch { }
    }
    await sleep(300);
    // Hard exit: mock servers / SSE sockets / the detached sidecar keep
    // handles alive past cleanup — without this the CI job hangs until the
    // workflow timeout kills it (observed 2026-09-21, run 35619850050).
    process.exit(process.exitCode ?? 0);
  });
