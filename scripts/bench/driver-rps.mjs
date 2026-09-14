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
  agent.destroy();
  const elapsedS = durationS;
  const offeredRate = sent / elapsedS;
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
    achievedRate: round(done / elapsedS, 1),
    driverLimited: offeredRate < rate * 0.97,
    maxInFlight,
    latencyMs: roundStats(summarize(lats)),
  };
}

function vegetaRun(bin, { epPort, upstreamPort, identity, rate, durationS, workDir }) {
  return new Promise((resolve) => {
    const binFile = path.join(workDir, "vegeta-attack.bin");
    const reportFile = path.join(workDir, "vegeta-report.json");
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
          resolve({
            engine: "vegeta",
            targetRate: rate,
            durationS,
            requests: j.requests,
            achievedRate: round(j.rate ?? 0, 1),
            successRatio: j.success,
            latencyMs: {
              mean: round((ns.mean ?? 0) / 1e6),
              p50: round((ns["50th"] ?? 0) / 1e6),
              p95: round((ns["95th"] ?? 0) / 1e6),
              p99: round((ns["99th"] ?? 0) / 1e6),
              max: round((ns.max ?? 0) / 1e6),
            },
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
  if (v && !v.error && !v.skipped && (v.successRatio ?? 0) > 0.99) {
    out.achievedRate = v.achievedRate;
    out.latencyP99 = v.latencyMs.p99;
  } else {
    out.achievedRate = out.openLoop.achievedRate;
    out.latencyP99 = out.openLoop.latencyMs.p99;
  }
  return out;
}
