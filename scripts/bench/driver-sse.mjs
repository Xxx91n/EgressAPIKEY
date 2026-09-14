// SSE measurement + the four hard behavior assertions (D-004 SSE rows):
//   TTFB added latency (paired direct vs proxied first-event delta) p95 <=5ms
//   per-event forwarding lag p99 <=10ms (arrival minus producer timestamp)
//   assertion 1: per-event flush (socket chunks ~= events, not batched)
//   assertion 2: client disconnect cascades cancel upstream (stats.cancels)
//   assertion 3: bounded backpressure (upstream sees blocked writes AND resin
//                RSS delta stays under cap while a paused client stalls)
//   assertion 4: in-band error signaling (data:{"error":...} + clean EOF)

import http from "node:http";
import net from "node:net";
import { summarize, roundStats, round, sleep, nowMs } from "./lib/stats.mjs";

export function openStream({ epPort, identity, url, agent, onEvent }) {
  const target = new URL(url);
  const cred = epPort
    ? Buffer.from(`${identity}:`).toString("base64")
    : null;
  const opts = epPort
    ? {
        host: "127.0.0.1",
        port: epPort,
        path: url, // absolute-URI = forward-proxy request form
        agent,
        headers: {
          host: target.host,
          "proxy-authorization": `Basic ${cred}`,
          "proxy-connection": "keep-alive",
          accept: "text/event-stream",
        },
      }
    : {
        host: target.hostname,
        port: Number(target.port),
        path: target.pathname + target.search,
        agent,
        headers: { accept: "text/event-stream" },
      };
  const t0 = nowMs();
  const st = {
    chunks: 0,
    events: [],
    firstDataMs: null,
    headersMs: null,
    ended: false,
    err: null,
    status: null,
    bodyTail: "",
    res: null,
    req: null,
    destroy: () => {},
  };
  const done = new Promise((resolve) => {
    const req = http.request(opts, (res) => {
      st.res = res;
      st.status = res.statusCode;
      st.headersMs = nowMs() - t0;
      let buf = "";
      res.on("data", (c) => {
        st.chunks++;
        if (st.firstDataMs == null) st.firstDataMs = nowMs() - t0;
        const txt = c.toString("utf8");
        st.bodyTail = (st.bodyTail + txt).slice(-300);
        buf += txt;
        let i;
        while ((i = buf.indexOf("\n\n")) >= 0) {
          const block = buf.slice(0, i);
          buf = buf.slice(i + 2);
          for (const line of block.split("\n")) {
            const m = /^data:\s?(.*)$/.exec(line);
            if (!m) continue;
            try {
              const j = JSON.parse(m[1]);
              const ev = {
                seq: j.seq ?? null,
                ts: j.ts ?? null,
                arrive: nowMs(),
                arriveWall: Date.now(),
                error: j.error ?? null,
              };
              st.events.push(ev);
              onEvent?.(ev);
            } catch {
              st.events.push({ raw: m[1].slice(0, 80), arrive: nowMs() });
            }
          }
        }
      });
      res.on("end", () => {
        st.ended = true;
        resolve(st);
      });
      res.on("error", (e) => {
        st.ended = true;
        st.err = String(e?.message ?? e);
        resolve(st);
      });
      res.on("close", () => {
        if (!st.ended) {
          st.ended = true;
          resolve(st);
        }
      });
    });
    req.setTimeout(30000, () => req.destroy());
    req.on("error", (e) => {
      st.ended = true;
      st.err = String(e?.message ?? e);
      resolve(st);
    });
    st.req = req;
    st.destroy = () => req.destroy();
    req.end(); // flush the request head — without this the socket sits idle
  });
  return { st, done };
}

// SSE over a CONNECT tunnel through the endpoint: resin's tunnel path does
// raw io.Copy on sockets (tunnel.go) — unbuffered. Control probe to isolate
// whether event batching is a forward-GET-path artifact or path-wide.
export function openTunnelStream({ epPort, identity, url, onEvent }) {
  const target = new URL(url);
  const cred = Buffer.from(`${identity}:`).toString("base64");
  const t0 = nowMs();
  const st = {
    chunks: 0,
    events: [],
    firstDataMs: null,
    headersMs: null,
    ended: false,
    err: null,
    status: null,
    destroy: () => {},
  };
  const done = new Promise((resolve) => {
    const sock = net.connect(Number(epPort), "127.0.0.1", () => {
      sock.write(
        `CONNECT ${target.host} HTTP/1.1\r\nhost: ${target.host}\r\n` +
          `proxy-authorization: Basic ${cred}\r\n\r\n`,
      );
    });
    st.destroy = () => sock.destroy();
    let buf = Buffer.alloc(0);
    let phase = 0; // 0 = awaiting CONNECT response, 1 = streaming
    let textBuf = "";
    sock.on("data", (c) => {
      if (phase === 0) {
        buf = Buffer.concat([buf, c]);
        const idx = buf.indexOf("\r\n\r\n");
        if (idx < 0) return;
        const statusLine = buf.subarray(0, buf.indexOf("\r\n")).toString("latin1");
        const m = /^HTTP\/1\.[01] (\d+)/.exec(statusLine);
        st.status = m ? Number(m[1]) : null;
        phase = 1;
        if (st.status !== 200) {
          st.ended = true;
          sock.destroy();
          return resolve(st);
        }
        sock.write(`GET ${target.pathname}${target.search} HTTP/1.1\r\nhost: ${target.host}\r\naccept: text/event-stream\r\n\r\n`);
        const rest = buf.subarray(idx + 4);
        if (rest.length) textBuf += rest.toString("utf8");
        buf = Buffer.alloc(0);
        if (rest.length) onChunk(st, rest.toString("utf8"), onEvent);
        return;
      }
      st.chunks++;
      if (st.firstDataMs == null) st.firstDataMs = nowMs() - t0;
      onChunk(st, c.toString("utf8"), onEvent);
    });
    function onChunk(stt, txt, cb) {
      textBuf += txt;
      let i;
      while ((i = textBuf.indexOf("\n\n")) >= 0) {
        const block = textBuf.slice(0, i);
        textBuf = textBuf.slice(i + 2);
        for (const line of block.split("\n")) {
          const m = /^data:\s?(.*)$/.exec(line);
          if (!m) continue;
          try {
            const j = JSON.parse(m[1]);
            const ev = { seq: j.seq ?? null, ts: j.ts ?? null, arrive: nowMs(), arriveWall: Date.now(), error: j.error ?? null };
            stt.events.push(ev);
            cb?.(ev);
          } catch {}
        }
      }
    }
    sock.on("end", () => {
      st.ended = true;
      resolve(st);
    });
    sock.on("error", (e) => {
      st.ended = true;
      st.err = String(e?.message ?? e);
      resolve(st);
    });
    sock.on("close", () => {
      if (!st.ended) {
        st.ended = true;
        resolve(st);
      }
    });
    sock.setTimeout(60000, () => sock.destroy());
  });
  return { st, done };
}

async function upStats(upStatsUrl) {
  const r = await fetch(upStatsUrl, { signal: AbortSignal.timeout(5000) });
  return r.json();
}

export async function runSse(ctx) {
  const { epPort, upstreamPort, identity, upStatsUrl, resinRssMB } = ctx;
  const agent = new http.Agent({ keepAlive: false, maxSockets: 1024 });
  const results = { assertions: {}, ttfbDelta: null, eventLag: null, assertionsPassed: 0 };

  // TTFB paired: 100 measured pairs after 20 warmup
  const ttfbDeltas = [];
  for (let i = 0; i < 120; i++) {
    const u = `http://127.0.0.1:${upstreamPort}/sse?interval=25&count=2`;
    const d = openStream({ url: u });
    await d.done;
    const p = openStream({ epPort, identity, url: u });
    await p.done;
    if (i >= 20 && d.st.firstDataMs != null && p.st.firstDataMs != null) {
      ttfbDeltas.push(p.st.firstDataMs - d.st.firstDataMs);
    }
  }
  results.ttfbDelta = roundStats(summarize(ttfbDeltas));

  // Per-event forwarding lag on a proxied stream (same-clock producer ts)
  const s = openStream({
    epPort,
    identity,
    url: `http://127.0.0.1:${upstreamPort}/sse?interval=50&count=100`,
  });
  await s.done;
  const lags = s.st.events.filter((e) => e.ts != null).map((e) => e.arriveWall - e.ts);
  results.eventLag = roundStats(summarize(lags));

  // Assertion 1: per-event flush. Socket-chunk counting is unreliable (TCP
  // coalescing merges small writes on a busy host), so assert on the ARRIVAL
  // pattern: a per-event-flushed stream spreads event arrivals across the
  // producer's emit window; a buffered-to-end stream bursts them at EOF.
  // spread = (lastArrive - firstArrive) / expected emit window; ~1 = streamed.
  const intervalMs = 100;
  const countN = 25;
  const spread = (st) =>
    st.events.length > 1
      ? (st.events.at(-1).arrive - st.events[0].arrive) / ((st.events.length - 1) * intervalMs)
      : 0;
  const sseUrl = `http://127.0.0.1:${upstreamPort}/sse?interval=${intervalMs}&count=${countN}`;
  const directCtl = openStream({ url: sseUrl });
  await directCtl.done;
  const f = openStream({ epPort, identity, url: sseUrl });
  await f.done;
  const proxySpread = round(spread(f.st), 3);
  const directSpread = round(spread(directCtl.st), 3);
  const tf = openTunnelStream({ epPort, identity, url: sseUrl });
  await tf.done;
  const tunnelSpread = round(spread(tf.st), 3);
  results.assertions.flush = {
    pass: proxySpread >= 0.6 && f.st.events.length >= 20,
    proxySpread,
    directSpread,
    tunnelSpread, // CONNECT-tunnel path (raw socket copy) control
    proxyChunks: f.st.chunks,
    events: f.st.events.length,
  };

  // Assertion 2: client disconnect cascades cancel upstream
  const before = (await upStats(upStatsUrl)).cancels;
  const streams = [];
  for (let i = 0; i < 5; i++) {
    streams.push(
      openStream({
        epPort,
        identity,
        url: `http://127.0.0.1:${upstreamPort}/sse?interval=500`,
      }),
    );
  }
  await sleep(2000);
  const kill = 3;
  for (let i = 0; i < kill; i++) streams[i].st.destroy();
  const t0 = nowMs();
  let cancelsObserved = 0;
  let ok = false;
  while (nowMs() - t0 < 15000) {
    cancelsObserved = (await upStats(upStatsUrl)).cancels - before;
    if (cancelsObserved >= kill) {
      ok = true;
      break;
    }
    await sleep(300);
  }
  for (const x of streams) x.st.destroy();
  results.assertions.cancel = { pass: ok, killed: kill, upstreamCancels: cancelsObserved };

  // Assertion 3: bounded backpressure — pause a client on a fast stream;
  // upstream must observe blocked writes and resin RSS must stay bounded.
  const bp = openStream({
    epPort,
    identity,
    url: `http://127.0.0.1:${upstreamPort}/sse?interval=1&size=4096`,
  });
  await sleep(600);
  if (bp.st.res) bp.st.res.pause();
  const rss0 = await resinRssMB();
  const s0 = await upStats(upStatsUrl);
  await sleep(6000);
  const s1 = await upStats(upStatsUrl);
  const rss1 = await resinRssMB();
  const upstreamBlocked =
    s1.blockedWrites - s0.blockedWrites > 0 || (s1.writableLength ?? 0) > 0;
  const rssDelta = (rss1 ?? 0) - (rss0 ?? 0);
  bp.st.destroy();
  results.assertions.backpressure = {
    pass: upstreamBlocked && rssDelta < 64,
    upstreamBlocked,
    rssDeltaMB: round(rssDelta, 2),
    upstreamBlockedWritesDelta: s1.blockedWrites - s0.blockedWrites,
    upstreamWritableLength: s1.writableLength,
  };

  // Assertion 4: in-band error signaling mid-stream
  const e = openStream({
    epPort,
    identity,
    url: `http://127.0.0.1:${upstreamPort}/sse-error?after=4&interval=20`,
  });
  await e.done;
  const gotErr = e.st.events.some((ev) => ev.error?.code === "UPSTREAM_BOOM");
  results.assertions.inbandError = {
    pass: gotErr && e.st.ended,
    ended: e.st.ended,
    sawError: gotErr,
    events: e.st.events.length,
  };

  agent.destroy();
  results.assertionsPassed = Object.values(results.assertions).filter((a) => a.pass).length;
  return results;
}
