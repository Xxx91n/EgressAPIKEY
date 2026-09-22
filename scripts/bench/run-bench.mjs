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
import { Sampler, sampleProcessTreeMB } from "./lib/sampler.mjs";
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
const OUT = arg("out", process.env.BENCH_OUT || path.join(root, "bench-results"));
const PHASES = (arg("phases", "idle,paired,rps,sse,soak,healthz") + (APP_EXE ? ",app" : ""))
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
    basis: "D-004 / A-015",
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
  // Whole-app steady state: launch the built exe, let it reach
  // steady state, then sum WorkingSet64 over the app + resin + webview procs.
  if (process.platform !== "win32") {
    return { skipped: "whole-app steady-state is measured on the windows job" };
  }
  if (!APP_EXE || !fs.existsSync(APP_EXE)) {
    return { skipped: `--app-exe missing or not found: ${APP_EXE}` };
  }
  const launchAt = Date.now();
  const child = spawn(APP_EXE, [], { stdio: "ignore", detached: true });
  children.push(child);
  await sleep(25000); // boot + first paint + settle
  const appAlive = child.exitCode === null;
  const samples = [];
  let lastPerName = null;
  let filtered = true;
  for (let i = 0; i < 30; i++) {
    let mb = await sampleProcessTreeMB(["EgressAPIKEY*", "resin*", "msedgewebview2*"], {
      minStartEpochMs: launchAt - 2000,
    });
    if (mb == null || mb.totalMB <= 0) {
      // fallback: unfiltered sum; flagged so the report knows attribution is loose
      const raw = await sampleProcessTreeMB(["EgressAPIKEY*", "resin*", "msedgewebview2*"]);
      if (raw != null && raw.totalMB > 0) {
        filtered = false;
        mb = raw;
      }
    }
    if (mb != null && mb.totalMB > 0) {
      samples.push(mb.totalMB);
      lastPerName = mb.perName;
    }
    await sleep(2000);
  }
  try {
    spawn("taskkill", ["/F", "/T", "/PID", String(child.pid)]);
  } catch { }
  const st = summarize(samples);
  return {
    appExe: APP_EXE,
    appAlive,
    childPid: child.pid,
    filtered,
    n: samples.length,
    totalMB: roundStats(st),
    perNameMB: lastPerName,
  };
}

// ---- gate evaluation ------------------------------------------------------

function resolvePath(obj, dotted) {
  return dotted.split(".").reduce((o, k) => (o == null ? undefined : o[k]), obj);
}

function evalGates(acc) {
  const gates = [];
  for (const item of acc.items) {
    const measured = resolvePath(results, item.measure);
    let status;
    if (item.op === "measure-only") {
      status = measured == null ? "pending" : "measured";
    } else if (measured == null || !Number.isFinite(measured)) {
      status = "n/a";
    } else {
      const ok =
        item.op === "<=" ? measured <= item.threshold
          : item.op === "<" ? measured < item.threshold
            : item.op === ">=" ? measured >= item.threshold
              : item.op === "==" ? measured === item.threshold
                : false;
      status = ok ? "pass" : "fail";
    }
    gates.push({ id: item.id, desc: item.desc, op: item.op, threshold: item.threshold, unit: item.unit, scope: item.scope, measured: measured ?? null, status });
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

  // gates
  const acc = JSON.parse(fs.readFileSync(path.join(root, "scripts/bench/acceptance.json"), "utf8"));
  results.gates = evalGates(acc);
  results.acceptance = acc;

  const fails = results.gates.filter((g) => g.status === "fail");
  const measured = results.gates.filter((g) => g.status === "measured");
  log(`gates: ${results.gates.filter((g) => g.status === "pass").length} pass, ${fails.length} fail, ${measured.length} measured-only`);
  if (GATE === "enforce" && fails.length > 0) {
    fatal(`${fails.length} gate(s) breached`);
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
    `| item | op | threshold | measured | unit | status |`,
    `|---|---|---|---|---|---|`,
    ...results.gates.map(
      (g) => `| ${g.id} | ${g.op} | ${g.threshold ?? "-"} | ${g.measured ?? "n/a"} | ${g.unit} | ${g.status} |`,
    ),
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
