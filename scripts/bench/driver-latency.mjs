// Paired-request full-link added latency (D-004: p95 <=5ms, p99 <=10ms).
// Same runner, same job: direct-to-mock-upstream vs through-Resin-endpoint
// requests alternate per pair; the delta distribution is what we assert on,
// so runner CPU jitter hits both legs and cancels out (Floe/Istio-style
// differential measurement).

import http from "node:http";
import { summarize, roundStats, nowMs } from "./lib/stats.mjs";
import { proxyFetch } from "./lib/resin.mjs";

function directGet(url, agent, timeoutMs = 10000) {
  return new Promise((resolve, reject) => {
    const u = new URL(url);
    const req = http.request(
      { host: u.hostname, port: u.port, path: u.pathname + u.search, agent, method: "GET" },
      (res) => {
        res.resume();
        res.on("end", resolve);
        res.on("error", reject);
      },
    );
    req.setTimeout(timeoutMs, () => req.destroy(new Error("direct timeout")));
    req.on("error", reject);
    req.end();
  });
}

export async function runLatency({ epPort, upstreamPort, identity, pairs = 600, warmup = 60 }) {
  const url = `http://127.0.0.1:${upstreamPort}/echo`;
  const directAgent = new http.Agent({ keepAlive: true, maxSockets: 8 });
  const proxyAgent = new http.Agent({ keepAlive: true, maxSockets: 8 });
  const deltas = [];
  const ds = [];
  const ps = [];
  let errors = 0;
  for (let i = 0; i < warmup + pairs; i++) {
    try {
      const t0 = nowMs();
      await directGet(url, directAgent);
      const t1 = nowMs();
      await proxyFetch({ epPort, identity, url, agent: proxyAgent });
      const t2 = nowMs();
      if (i >= warmup) {
        ds.push(t1 - t0);
        ps.push(t2 - t1);
        deltas.push(t2 - t1 - (t1 - t0));
      }
    } catch {
      if (i >= warmup) errors++;
    }
  }
  directAgent.destroy();
  proxyAgent.destroy();
  return {
    pairs: deltas.length,
    errors,
    delta: roundStats(summarize(deltas)),
    directMs: roundStats(summarize(ds)),
    proxiedMs: roundStats(summarize(ps)),
  };
}
