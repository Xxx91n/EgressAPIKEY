// Resin sidecar lifecycle + admin API client for the perf baseline harness.
// Mirrors src-tauri/src/sidecar.rs env contract (RESIN_AUTH_VERSION=V1,
// loopback bind, single consolidated port, admin token server-side only).
// The bench runs Resin in "B mode" (engine direct listen): the client sends
// Platform.Account as the proxy credential on every request.

import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import path from "node:path";
import { sleep, nowMs } from "./stats.mjs";

export function findResinBinary(root) {
  const envBin = process.env.RESIN_BIN;
  if (envBin && fs.existsSync(envBin)) return envBin;
  const dir = path.join(root, "src-tauri", "binaries");
  if (!fs.existsSync(dir)) {
    throw new Error(`resin binary dir missing: ${dir} (run scripts/fetch_resin.sh first or set RESIN_BIN)`);
  }
  const isWin = process.platform === "win32";
  const arch = process.arch === "x64" ? "x86_64" : process.arch;
  const osName = { win32: "windows", darwin: "darwin", linux: "linux" }[process.platform];
  const cand = fs
    .readdirSync(dir)
    .filter((f) => f.startsWith("resin-") && (isWin ? f.endsWith(".exe") : !f.endsWith(".exe")))
    .sort(
      (a, b) =>
        Number(b.includes(arch) && b.includes(osName)) -
        Number(a.includes(arch) && a.includes(osName)),
    );
  if (cand.length === 0) {
    throw new Error(`no resin-* binary in ${dir} for ${arch}/${osName} (run scripts/fetch_resin.sh or set RESIN_BIN)`);
  }
  return path.join(dir, cand[0]);
}

export function pickFreePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.once("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const p = srv.address().port;
      srv.close(() => resolve(p));
    });
  });
}

// Spawn the sidecar. cpuPin options:
//   linux:   { taskset: "0", gomaxprocs: 1 } -> taskset -c 0 + GOMAXPROCS=1
//   windows: { affinity: 1, gomaxprocs: 1 }   -> ProcessorAffinity post-spawn
// Returns { child, apiPort, adminToken, pid, logTail() }.
export function spawnResin({ bin, workDir, apiPort, adminToken, cpuPin }) {
  const stateDir = path.join(workDir, "state");
  const cacheDir = path.join(workDir, "cache");
  const logDir = path.join(workDir, "logs");
  for (const d of [stateDir, cacheDir, logDir]) fs.mkdirSync(d, { recursive: true });

  const env = {
    ...process.env,
    RESIN_AUTH_VERSION: "V1",
    RESIN_ADMIN_TOKEN: adminToken,
    RESIN_PROXY_TOKEN: "",
    RESIN_LISTEN_ADDRESS: "127.0.0.1",
    RESIN_PORT: String(apiPort),
    RESIN_STATE_DIR: stateDir,
    RESIN_CACHE_DIR: cacheDir,
    RESIN_LOG_DIR: logDir,
  };
  if (cpuPin?.gomaxprocs) env.GOMAXPROCS = String(cpuPin.gomaxprocs);

  const useTaskset = process.platform === "linux" && cpuPin?.taskset;
  const cmd = useTaskset ? "taskset" : bin;
  const args = useTaskset ? ["-c", String(cpuPin.taskset), bin] : [];

  const child = spawn(cmd, args, { env, stdio: ["ignore", "pipe", "pipe"] });
  const tail = [];
  const push = (buf) => {
    tail.push(...buf.toString("utf8").split("\n").filter(Boolean));
    if (tail.length > 60) tail.splice(0, tail.length - 60);
  };
  child.stdout.on("data", push);
  child.stderr.on("data", push);

  if (process.platform === "win32" && cpuPin?.affinity && child.pid) {
    try {
      const ps = spawn("powershell", [
        "-NoProfile",
        "-Command",
        `(Get-Process -Id ${child.pid}).ProcessorAffinity=${cpuPin.affinity}`,
      ]);
      ps.on("error", () => {});
      ps.stderr.on("data", () => {});
    } catch {}
  }

  return {
    child,
    apiPort,
    adminToken,
    pid: child.pid,
    logTail: () => tail.slice(-30).join("\n"),
  };
}

// Poll GET /healthz until 200; returns ms elapsed from t0.
export async function waitHealthz(apiPort, deadlineMs = 30000, t0 = nowMs()) {
  const deadline = nowMs() + deadlineMs;
  for (;;) {
    try {
      const r = await fetch(`http://127.0.0.1:${apiPort}/healthz`, {
        signal: AbortSignal.timeout(1000),
      });
      if (r.ok) return nowMs() - t0;
    } catch {}
    if (nowMs() > deadline) throw new Error(`healthz not ready within ${deadlineMs}ms`);
    await sleep(50);
  }
}

// Minimal admin REST client (Authorization: Bearer).
export function resinApi(apiPort, adminToken) {
  const base = `http://127.0.0.1:${apiPort}/api/v1`;
  const call = async (method, p, body) => {
    const r = await fetch(base + p, {
      method,
      headers: {
        authorization: `Bearer ${adminToken}`,
        "content-type": "application/json",
      },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(30000),
    });
    const text = await r.text();
    let json = null;
    try {
      json = JSON.parse(text);
    } catch {}
    return { status: r.status, json, text };
  };
  return {
    get: (p) => call("GET", p),
    post: (p, body) => call("POST", p, body),
    patch: (p, body) => call("PATCH", p, body),
    del: (p) => call("DELETE", p),
  };
}

// GET through the forward-proxy endpoint: absolute-URI request-target plus
// Proxy-Authorization: Basic <base64(Platform.Account:)> (empty proxy token
// means the password half is unchecked; the user half still binds identity).
export function proxyFetch({ epPort, identity, url, timeoutMs = 10000, agent }) {
  return new Promise((resolve, reject) => {
    const u = new URL(url);
    const cred = Buffer.from(`${identity}:`, "utf8").toString("base64");
    const req = http.request(
      {
        host: "127.0.0.1",
        port: epPort,
        method: "GET",
        path: url, // absolute-URI = forward-proxy request form
        agent,
        headers: {
          host: u.host,
          "proxy-authorization": `Basic ${cred}`,
          "proxy-connection": "keep-alive",
        },
      },
      (res) => {
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () =>
          resolve({ status: res.statusCode, body: Buffer.concat(chunks).toString("utf8") }),
        );
        res.on("error", reject);
      },
    );
    req.setTimeout(timeoutMs, () => req.destroy(new Error("proxyFetch timeout")));
    req.on("error", reject);
    req.end();
  });
}

// Bring the sidecar to a routable state for the bench:
//   platform "bench" + local Clash subscription (http CONNECT node at
//   nodePort) + node health probes (egress IP + latency record close the
//   initial circuit break) + mixed entry endpoint. Polls until a real proxied
//   /echo answers 200. Returns { endpointId, nodeHash }.
export async function ensureRoutable({ api, nodePort, epPort, upstreamPort }) {
  const clash = [
    "proxies:",
    "  - name: bench-node",
    "    type: http",
    "    server: 127.0.0.1",
    `    port: ${nodePort}`,
    "",
  ].join("\n");

  let r = await api.post("/platforms", { name: "bench" });
  if (![200, 201, 409].includes(r.status)) {
    throw new Error(`create platform: HTTP ${r.status} ${r.text.slice(0, 200)}`);
  }

  r = await api.post("/subscriptions", {
    name: "bench-sub",
    source_type: "local",
    content: clash,
    update_interval: "30s",
  });
  if (![200, 201].includes(r.status)) {
    throw new Error(`create subscription: HTTP ${r.status} ${r.text.slice(0, 200)}`);
  }

  r = await api.post("/endpoints", {
    port: epPort,
    enabled: true,
    allow_management: false,
    allow_proxy: true,
    require_proxy_auth_info: false,
    allow_http_forward: true,
    allow_http_reverse: true,
    allow_socks5: true,
  });
  if (![200, 201].includes(r.status)) {
    throw new Error(`create endpoint: HTTP ${r.status} ${r.text.slice(0, 200)}`);
  }
  const endpointId = r.json?.id ?? null;

  const deadline = nowMs() + 90000;
  let nodeHash = null;
  let lastProbe = "n/a";
  for (;;) {
    const nodes = await api.get("/nodes?limit=500");
    const items = nodes.json?.items ?? (Array.isArray(nodes.json) ? nodes.json : []);
    const mine = items.find(
      (n) => JSON.stringify(n).includes("bench-node") || JSON.stringify(n).includes(String(nodePort)),
    );
    if (mine && !nodeHash) nodeHash = mine.hash ?? mine.node_hash ?? mine.id ?? null;
    if (nodeHash) {
      const pe = await api.post(`/nodes/${nodeHash}/actions/probe-egress`);
      const pl = await api.post(`/nodes/${nodeHash}/actions/probe-latency`);
      lastProbe = `egress=${pe.status} latency=${pl.status}`;
      try {
        const resp = await proxyFetch({
          epPort,
          identity: "bench.bench",
          url: `http://127.0.0.1:${upstreamPort}/echo`,
          timeoutMs: 5000,
        });
        if (resp.status === 200) {
          return { endpointId, nodeHash, probe: lastProbe };
        }
      } catch {}
    }
    if (nowMs() > deadline) break;
    await sleep(2000);
  }
  throw new Error(`node never became routable (lastProbe=${lastProbe}, nodeHash=${nodeHash})`);
}
