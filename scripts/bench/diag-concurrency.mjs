// Concurrency bisection diagnostic: setup resin + open N burst streams at once,
// print per-stream outcome. Usage: node scripts/bench/diag-concurrency.mjs [N]
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { sleep, nowMs } from "./lib/stats.mjs";
import { startMockUpstream, startMockNode } from "./lib/mock.mjs";
import { findResinBinary, pickFreePort, spawnResin, waitHealthz, resinApi, ensureRoutable, proxyFetch } from "./lib/resin.mjs";
import { openStream } from "./driver-sse.mjs";
import { Sampler } from "./lib/sampler.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const N = Number(process.argv[2] || 20);
const bin = findResinBinary(root);
const adminToken = crypto.randomBytes(16).toString("hex");
const identity = "bench.bench";

const up = await startMockUpstream(0);
const node = await startMockNode(0);
const apiPort = await pickFreePort();
const epPort = await pickFreePort();
const workDir = fs.mkdtempSync(path.join(os.tmpdir(), "bench-diag-"));
const resin = spawnResin({ bin, workDir, apiPort, adminToken, cpuPin: { affinity: 1, gomaxprocs: 1 } });
console.log("ports up=%d node=%d api=%d ep=%d resinPid=%d", up.port, node.port, apiPort, epPort, resin.pid);
await waitHealthz(apiPort, 30000);
const api = resinApi(apiPort, adminToken);
const setup = await ensureRoutable({ api, nodePort: node.port, epPort, upstreamPort: up.port });
console.log("routable:", JSON.stringify(setup));

// sanity: one sequential stream first
const warm = openStream({ epPort, identity, url: `http://127.0.0.1:${up.port}/sse?interval=20&count=3` });
await warm.done;
console.log("warmup stream: status=%s events=%d err=%s", warm.st.status, warm.st.events.length, warm.st.err);

// burst N concurrent (with sampler running, like runSoak)
const sampler = process.env.DIAG_SAMPLER ? new Sampler(resin.pid, 5000).start() : null;
const conns = [];
for (let i = 0; i < N; i++) {
  conns.push(openStream({ epPort, identity, url: `http://127.0.0.1:${up.port}/sse?interval=1000&size=48` }));
}
await sleep(4000);
console.log("upstream stats:", JSON.stringify(up.stats));
console.log("node stats:", JSON.stringify(node.stats));
let live = 0;
for (const c of conns) if (!c.st.ended && c.st.status === 200) live++;
console.log("after 4s: live=%d/%d", live, N);
await sleep(6000);
for (const c of conns) c.st.destroy();
const settled = await Promise.all(conns.map((c) => c.done));
const hist = {};
for (const st of settled) {
  const k = `status=${st.status} events=${st.events.length} err=${st.err ?? "-"} ended=${st.ended}`;
  hist[k] = (hist[k] ?? 0) + 1;
}
console.log("outcome histogram:", JSON.stringify(hist, null, 1));
console.log("resin log tail:\n" + resin.logTail());
try { resin.child.kill("SIGKILL"); } catch {}
process.exit(0);
