// Fixed-rate throughput phase (D-004: >=500 RPS at p99<50ms on 1 vCPU).
// Two engines, same target path:
//   openLoop  — built-in zero-dep scheduler: requests fire on a fixed interval
//               regardless of in-flight count (no coordinated omission; the
//               in-flight cap is a safety valve that flags saturation).
//   vegeta    — optional cross-check when the binary is present (CI installs
//               it). Hits Resin's reverse-proxy path so no proxy flag needed.

import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { summarize, roundStats, round, sleep, nowMs } from "./lib/stats.mjs";
import { proxyFetch } from "./lib/resin.mjs";

async function openLoop({ epPort, upstreamPort, identity, rate, durationS }) {
  const url = `http://127.0.0.1:${upstreamPort}/echo`;
  const agent = new http.Agent({ keepAlive: true, maxSockets: 1024 });
  // Batched open-loop scheduler: a plain setInterval(1000/rate) cannot sustain
  // >~250/s on Windows (timer granularity ~4ms), so we emit rate*tick/1000
  // requests per tick instead — open-loop semantics preserved (scheduled send
  // regardless of in-flight), only the emission is batched.
  const tickMs = Math.max(10, Math.ceil(4000 / rate)); // 500/s -> 10ms tick
  // Deficit accounting against wall-clock elapsed per tick: self-corrects when
  // the OS clamps timer granularity (Windows ~15.6ms default resolution).
  const lats = [];
  let sent = 0;
  let done = 0;
  let errors = 0;
  let non200 = 0;
  const statusHist = {};
  let timeouts = 0;
  let inFlight = 0;
  let maxInFlight = 0;
  let carry = 0;
  let lastTick = nowMs();
  const end = lastTick + durationS * 1000;
  // Concurrent direct-baseline sampler (r12 acceptance line): low-rate (~1 rps)
  // unpinned probes of the same upstream while the load leg runs, so the p99
  // delta is the PROXY-ADDED component, not absolute latency contaminated by
  // shared-runner jitter.
  const directLat = [];
  let baselineStop = false;
  const baseline = (async () => {
    while (!baselineStop) {
      const bt0 = nowMs();
      try {
        const r = await fetch(`http://127.0.0.1:${upstreamPort}/echo`, {
          signal: AbortSignal.timeout(5000),
        });
        await r.arrayBuffer();
        directLat.push(nowMs() - bt0);
      } catch {}
      await sleep(1000);
    }
  })();
  const sendOne = () => {
    sent++;
    inFlight++;
    if (inFlight > maxInFlight) maxInFlight = inFlight;
    const t = nowMs();
    proxyFetch({ epPort, identity, url, agent, timeoutMs: 15000 })
      .then((r) => {
        inFlight--;
        done++;
        lats.push(nowMs() - t);
        statusHist[r.status ?? "none"] = (statusHist[r.status ?? "none"] ?? 0) + 1;
        if (r.status !== 200) non200++;
      })
      .catch((e) => {
        inFlight--;
        errors++;
        if (/timeout/i.test(String(e?.message ?? e))) timeouts++;
      });
  };
  await new Promise((resolve) => {
    const timer = setInterval(() => {
      const now = nowMs();
      if (now >= end) {
        clearInterval(timer);
        resolve();
        return;
      }
      carry += rate * ((now - lastTick) / 1000);
      lastTick = now;
      const n = Math.floor(carry);
      carry -= n;
      for (let k = 0; k < n; k++) sendOne();
    }, tickMs);
    timer.unref?.();
  });
  // drain in-flight
  const drainEnd = nowMs() + 20000;
  while (inFlight > 0 && nowMs() < drainEnd) await sleep(100);
  baselineStop = true;
  await baseline;
  agent.destroy();
  const elapsedS = durationS;
  const offeredRate = sent / elapsedS;
  const achievedRate = done / elapsedS;
  const latStats = roundStats(summarize(lats));
  const directStats = roundStats(summarize(directLat));
  return {
    engine: "openLoop",
    targetRate: rate,
    durationS: elapsedS,
    sent,
    completed: done,
    errors,
    non200,
    statusHist,
    timeouts,
    offeredRate: round(offeredRate, 1),
    achievedRate: round(achievedRate, 1),
    achievedRatio: round(achievedRate / rate, 4),
    errorRate: round((errors + non200) / Math.max(sent, 1), 4),
    driverLimited: offeredRate < rate * 0.97,
    maxInFlight,
    latencyMs: latStats,
    directLatencyMs: directStats,
    deltaP99:
      latStats.p99 != null && directStats.p99 != null
        ? round(latStats.p99 - directStats.p99, 3)
        : null,
  };
}

function vegetaRun(bin, { epPort, upstreamPort, identity, rate, durationS, workDir }) {
  return new Promise((resolve) => {
    const binFile = path.join(workDir, "vegeta-attack.bin");
    const reportFile = path.join(workDir, "vegeta-report.json");
    // Same concurrent direct-baseline caliber as the openLoop engine.
    const directLat = [];
    let baselineStop = false;
    const baseline = (async () => {
      while (!baselineStop) {
        const bt0 = nowMs();
        try {
          const r = await fetch(`http://127.0.0.1:${upstreamPort}/echo`, {
            signal: AbortSignal.timeout(5000),
          });
          await r.arrayBuffer();
          directLat.push(nowMs() - bt0);
        } catch {}
        await sleep(1000);
      }
    })();
    // Reverse-proxy path form: /token/Platform.Account/protocol/host/path.
    // Empty RESIN_PROXY_TOKEN means any dummy token segment is accepted.
    const target = `http://127.0.0.1:${epPort}/bench/${identity}/http/127.0.0.1:${upstreamPort}/echo`;
    const attack = spawn(
      bin,
      [
        "attack",
        `-rate=${rate}/1s`,
        `-duration=${durationS}s`,
        "-max-workers=512",
        "-timeout=15s",
        "-keepalive=true",
        `-output=${binFile}`,
      ],
      { stdio: ["pipe", "ignore", "pipe"] },
    );
    let stderr = "";
    attack.stderr.on("data", (d) => (stderr += d));
    attack.stdin.write(`GET ${target}\n`);
    attack.stdin.end();
    attack.on("error", (e) => resolve({ engine: "vegeta", error: String(e) }));
    attack.on("close", (code) => {
      if (code !== 0 || !fs.existsSync(binFile)) {
        return resolve({ engine: "vegeta", error: `attack exit=${code} ${stderr.slice(-400)}` });
      }
      baselineStop = true;
      baseline.then(() => {});
      const rep = spawn(bin, ["report", "-type=json", binFile], { stdio: ["ignore", "pipe", "pipe"] });
      let out = "";
      rep.stdout.on("data", (d) => (out += d));
      rep.on("error", (e) => resolve({ engine: "vegeta", error: String(e) }));
      rep.on("close", (rc) => {
        if (rc !== 0) return resolve({ engine: "vegeta", error: `report exit=${rc}` });
        try {
          fs.writeFileSync(reportFile, out);
          const j = JSON.parse(out);
          const ns = j.latencies ?? {};
          const achieved = j.rate ?? 0;
          const p99 = round((ns["99th"] ?? 0) / 1e6);
          const directStats = roundStats(summarize(directLat));
          const totalReq = j.requests ?? 0;
          const failReq = Math.round((1 - (j.success ?? 0)) * totalReq);
          resolve({
            engine: "vegeta",
            targetRate: rate,
            durationS,
            requests: j.requests,
            achievedRate: round(achieved, 1),
            achievedRatio: round(achieved / rate, 4),
            successRatio: j.success,
            errorRate: round(1 - (j.success ?? 0), 4),
            non200: failReq,
            latencyMs: {
              mean: round((ns.mean ?? 0) / 1e6),
              p50: round((ns["50th"] ?? 0) / 1e6),
              p95: round((ns["95th"] ?? 0) / 1e6),
              p99,
              max: round((ns.max ?? 0) / 1e6),
            },
            directLatencyMs: directStats,
            deltaP99:
              p99 != null && directStats.p99 != null
                ? round(p99 - directStats.p99, 3)
                : null,
          });
        } catch (e) {
          resolve({ engine: "vegeta", error: `parse: ${e}` });
        }
      });
    });
  });
}

function findVegeta() {
  if (process.env.VEGETA && fs.existsSync(process.env.VEGETA)) return process.env.VEGETA;
  const pathExt = process.platform === "win32" ? [".exe", ".bat", ".cmd", ""] : [""];
  for (const dir of (process.env.PATH ?? "").split(path.delimiter)) {
    for (const ext of pathExt) {
      const p = path.join(dir, `vegeta${ext}`);
      try {
        if (fs.existsSync(p)) return p;
      } catch { }
    }
  }
  return null;
}

export async function runRps(ctx, { rate = 500, durationS = 60 } = {}) {
  const out = {};
  out.openLoop = await openLoop({ ...ctx, rate, durationS });
  const bin = findVegeta();
  if (bin) {
    out.vegeta = await vegetaRun(bin, { ...ctx, rate, durationS, workDir: ctx.outDir });
  } else {
    out.vegeta = { engine: "vegeta", skipped: "binary not on PATH (set VEGETA)" };
  }
  // Gate values: prefer vegeta when it ran clean, else openLoop.
  const v = out.vegeta;
  const src = v && !v.error && !v.skipped && (v.successRatio ?? 0) > 0.99 ? v : out.openLoop;
  out.achievedRate = src.achievedRate;
  out.latencyP99 = src.latencyMs?.p99 ?? src.latencyP99;
  out.achievedRatio = src.achievedRatio;
  out.errorRate = src.errorRate;
  out.deltaP99 = src.deltaP99;
  out.directLatencyMs = src.directLatencyMs;
  out.gateSource = src.engine;
  return out;
}
