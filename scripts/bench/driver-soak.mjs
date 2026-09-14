// Concurrency soak (D-004: 200 concurrent SSE streams, zero OOM/deadlock,
// sidecar RSS<=128MB under load, CPU <0.5 core on 1 vCPU).
// Opens N proxied SSE streams, holds them for durationS while sampling resin
// RSS/CPU, then verifies: all connected inside the connect window, zero
// unexpected closes, events still flowing in the final window, resin alive.

import { summarize, roundStats, round, sleep, nowMs } from "./lib/stats.mjs";
import { Sampler } from "./lib/sampler.mjs";
import { openStream } from "./driver-sse.mjs";

export async function runSoak(ctx, { streams = 200, durationS = 240, interval = 500 } = {}) {
  const { epPort, upstreamPort, identity, resinPid, resinAlive } = ctx;
  const sampler = new Sampler(resinPid, 5000).start();
  const conns = [];
  const gaps = [];
  let eventsTotal = 0;
  const perStream = [];
  const rampStart = nowMs();

  for (let i = 0; i < streams; i++) {
    const rec = { idx: i, events: 0, lastArrive: null, closedEarly: false, err: null };
    const c = openStream({
      epPort,
      identity,
      url: `http://127.0.0.1:${upstreamPort}/sse?interval=${interval}&size=512`,
      onEvent: (ev) => {
        rec.events++;
        eventsTotal++;
        if (rec.lastArrive != null) gaps.push(ev.arrive - rec.lastArrive);
        rec.lastArrive = ev.arrive;
      },
    });
    c.done.then((st) => {
      rec.status = st.status;
      if (st.bodyTail && st.status && st.status !== 200) rec.bodyTail = st.bodyTail.slice(-160);
      if (!st.err && st.ended) rec.closedEarly = true;
      if (st.err) rec.err = st.err;
    });
    rec.conn = c;
    perStream.push(rec);
    conns.push(c);
    if (i % 25 === 24) await sleep(100); // ramp: 25 new conns per 100ms
  }

  // connect window: wait until every stream has seen first data (or deadline)
  const connectDeadline = nowMs() + 90000;
  let connected = 0;
  while (nowMs() < connectDeadline) {
    connected = conns.filter((c) => c.st.firstDataMs != null).length;
    if (connected >= streams) break;
    await sleep(500);
  }
  const connectWindowMs = nowMs() - rampStart;

  const end = nowMs() + durationS * 1000;
  let eventsInFinalWindow = 0;
  while (nowMs() < end) {
    const pre = eventsTotal;
    await sleep(Math.min(10000, end - nowMs()));
    eventsInFinalWindow = eventsTotal - pre; // events in the last <=10s slice
  }

  const unexpectedCloses = perStream.filter((r) => r.closedEarly || r.err).length;
  const statusHist = {};
  for (const r of perStream) {
    const st = r.conn?.st?.status ?? r.status ?? "none";
    statusHist[st] = (statusHist[st] ?? 0) + 1;
  }
  const errSamples = [...new Set(perStream.map((r) => r.err).filter(Boolean))].slice(0, 5);
  const bodySamples = [...new Set(perStream.map((r) => r.bodyTail).filter(Boolean))].slice(0, 3);
  const alive = await resinAlive();
  for (const c of conns) c.st.destroy();
  const res = await sampler.stop();

  return {
    streams,
    durationS,
    connectWindowMs: Math.round(connectWindowMs),
    connected,
    unexpectedCloses,
    resinAlive: alive,
    eventsTotal,
    eventsInFinalWindow,
    statusHist,
    errSamples,
    bodySamples,
    eventGapMs: roundStats(summarize(gaps)),
    rssMB: res.rssMB,
    cpuCores: res.cpuCores,
    cpuPctOfOneCore: res.cpuPctOfOneCore,
    samples: res.n,
    samplerErrors: res.errors,
    resinLogTail: (ctx.resin?.logTail?.() ?? "").split("\n").slice(-25),
  };
}
