#!/usr/bin/env node
// Perf baseline orchestrator.
// CI-only evidence path: .github/workflows/bench.yml runs this on pinned
// runners; local runs are dev aids, not acceptance evidence.
//
// Usage:
//   node scripts/bench/run-bench.mjs [--duration short|long]
//     [--gate warn|enforce] [--resin <path>] [--app-exe <path>]
//     [--out <dir>] [--phases idle,paired,rps,sse,soak,healthz,app]
// Env: RESIN_BIN, VEGETA, BENCH_OUT

import { spawn } from "node:child_process";
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
const OUT = arg("out", process.env.BENCH_OUT || path.join(root, "bench-results"));
const PHASES = (arg("phases", "idle,paired,rps,sse,soak,healthz") + (APP_EXE ? ",app,appstart" : ""))
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
    basis: "r12-wave-a D-002 acceptance line (supersedes round8 D-004)",
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

// ---- phases ---------------------------------------------------------------

async function phaseIdle(ctx) {
  await sleep(15000); // settle past startup churn
  const s = new Sampler(ctx.resinPid, 2000).start();
  await sleep(60000);
  const st = await s.stop();
  return { windowS: 60, rssMB: st.rssMB, cpuCores: st.cpuCores, cpuPct: st.cpuPctOfOneCore, n: st.n };
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
  // count other apps' webview processes (r12-wave-a D-002).
  if (process.platform !== "win32") {
    return { skipped: "whole-app steady-state is measured on the windows job" };
  }
  if (!APP_EXE || !fs.existsSync(APP_EXE)) {
    return { skipped: `--app-exe missing or not found: ${APP_EXE}` };
  }
  const child = spawn(APP_EXE, [], { stdio: "ignore", detached: true });
  children.push(child);
  await sleep(25000); // boot + first paint + settle
  const appAlive = child.exitCode === null;
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
  try {
    spawn("taskkill", ["/F", "/T", "/PID", String(child.pid)]);
  } catch { }
  const st = summarize(samples);
  const wv2 = summarize(wv2Samples);
  return {
    appExe: APP_EXE,
    appAlive,
    childPid: child.pid,
    attribution: "descendant-tree",
    n: samples.length,
    totalMB: roundStats(st),
    webview2MB: roundStats(wv2),
    procs: lastProcs,
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
    try {
      spawn("taskkill", ["/F", "/T", "/PID", String(child.pid)]);
    } catch { }
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
    } else if (danger != null && cmp(measured, danger)) {
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
    // readiness: TCP connectable
    const tF = nowMs();
    for (;;) {
      try {
        await new Promise((res, rej) => {
          const sk = net.connect(modeAPort, "127.0.0.1", () => { sk.destroy(); res(); });
          sk.on("error", rej);
          sk.setTimeout(500, () => sk.destroy());
        });
        break;
      } catch {
        if (nowMs() - tF > 15000) { modeAPort = null; break; }
        await sleep(100);
      }
    }
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
    apiPort,
    bin,
    cpuPin,
    coldStartMs,
    outDir: OUT,
    resinAlive: async () => resin.child.exitCode === null,
    resinRssMB: () => resinRssMB(resin.pid),
    modeAPort,
  };

  const runPhase = async (name, fn) => {
    if (!PHASES.includes(name)) return;
    log(`phase ${name} start`);
    try {
      results.phases[name] = await fn(ctx);
      log(`phase ${name} done: ${JSON.stringify(results.phases[name]).slice(0, 300)}`);
    } catch (e) {
      results.phases[name] = { error: String(e?.stack ?? e) };
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

  // gates
  const acc = JSON.parse(fs.readFileSync(path.join(root, "scripts/bench/acceptance.json"), "utf8"));
  results.gates = evalGates(acc);
  results.acceptance = acc;

  // Failure semantics (r12 D-002): measurement numbers are always RECORDED -
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
