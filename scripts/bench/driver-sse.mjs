// SSE measurement + the four hard behavior assertions (r12-wave-a D-002):
//   TTFB added latency (paired direct vs proxied first-event delta) p95 <=5ms
//   event-gap added delta as a GOOD-EVENT RATIO (<=10ms @>=99% target /
//     <=500ms @>=99% danger) - Mode A shell CONNECT tunnel only; the Mode B
//     forward-GET path is a Registered Exemption (buffered io.Copy upstream).
//   assertion 1: per-event flush (arrival spread ~= emit window)
//   assertion 2: client disconnect cascades cancel upstream (stats.cancels)
//   assertion 3: bounded backpressure (upstream sees blocked writes AND resin
//                RSS delta stays under cap while a paused client stalls)
//   assertion 4: in-band error signaling (data:{"error":...} + clean EOF)
//
// Legs: modeB = forward-GET through the engine endpoint (Resin data plane);
// tunnel = CONNECT through the endpoint; modeA = CONNECT through the shell
// forwarder (bench-forwarder bin) - the desktop-shipped path the acceptance
// line actually gates. When no forwarder is present the tunnel leg stands in
// as the CONNECT-path reference and modeA is recorded as skipped.

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
        st.endMs = nowMs() - t0;
        resolve(st);
      });
      res.on("error", (e) => {
        st.ended = true;
        st.endMs = nowMs() - t0;
        st.err = String(e?.message ?? e);
        resolve(st);
      });
      res.on("close", () => {
        if (!st.ended) {
          st.ended = true;
          st.endMs = nowMs() - t0;
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
    req.end(); // flush the request head - without this the socket sits idle
  });
  return { st, done };
}

// SSE over a CONNECT tunnel through an entry port. When the port is the
// engine endpoint the caller passes identity (Mode B credential); when it is
// the Mode A shell forwarder, pass creds:null - the port IS the identity and
// the forwarder injects the credential upstream. Resin's tunnel path does
// raw socket copy (tunnel.go) - unbuffered.
export function openTunnelStream({ epPort, identity, url, onEvent, creds }) {
  const target = new URL(url);
  const useCreds =
    creds === undefined
      ? Buffer.from(`${identity}:`).toString("base64")
      : creds;
  const t0 = nowMs();
  const st = {
    chunks: 0,
    events: [],
    firstDataMs: null,
    headersMs: null,
    ended: false,
    err: null,
    status: null,
    sock: null,
    destroy: () => {},
  };
  const done = new Promise((resolve) => {
    const sock = net.connect(Number(epPort), "127.0.0.1", () => {
      const authLine = useCreds
        ? `proxy-authorization: Basic ${useCreds}\r\n`
        : "";
      sock.write(
        `CONNECT ${target.host} HTTP/1.1\r\nhost: ${target.host}\r\n` +
          authLine +
          "\r\n",
      );
    });
    st.sock = sock;
    st.destroy = () => sock.destroy();
    let buf = Buffer.alloc(0);
    let phase = 0; // 0 = awaiting CONNECT response, 1 = streaming
    let textBuf = "";
    sock.on("data", (c) => {
      if (phase === 0) {
        buf = Buffer.concat([buf, c]);
        const idx = buf.indexOf("\r\n\r\n");
        if (idx < 0) return;
        const statusLine = buf
          .subarray(0, buf.indexOf("\r\n"))
          .toString("latin1");
        const m = /^HTTP\/1\.[01] (\d+)/.exec(statusLine);
        st.status = m ? Number(m[1]) : null;
        phase = 1;
        if (st.status !== 200) {
          st.ended = true;
          sock.destroy();
          return resolve(st);
        }
        sock.write(
          `GET ${target.pathname}${target.search} HTTP/1.1\r\nhost: ${target.host}\r\naccept: text/event-stream\r\n\r\n`,
        );
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
            const ev = {
              seq: j.seq ?? null,
              ts: j.ts ?? null,
              arrive: nowMs(),
              arriveWall: Date.now(),
              error: j.error ?? null,
            };
            stt.events.push(ev);
            cb?.(ev);
          } catch {}
        }
      }
    }
    sock.on("end", () => {
      st.ended = true;
      st.endMs = nowMs() - t0;
      resolve(st);
    });
    sock.on("error", (e) => {
      st.ended = true;
      st.endMs = nowMs() - t0;
      st.err = String(e?.message ?? e);
      resolve(st);
    });
    sock.on("close", () => {
      if (!st.ended) {
        st.ended = true;
        st.endMs = nowMs() - t0;
        resolve(st);
      }
    });
    sock.setTimeout(60000, () => sock.destroy());
  });
  return { st, done };
}

// Wait until a predicate on st holds or the timeout lapses - the engine's
// tunnel does not propagate upstream EOF to the client promptly (observed
// ~60s idle before close on a completed counted stream), so legs poll on
// event arrival instead of awaiting natural end, then destroy.
async function until(st, pred, timeoutMs) {
  const t0 = nowMs();
  while (!pred(st) && nowMs() - t0 < timeoutMs) await sleep(25);
  return pred(st);
}
const untilFirstData = (st, timeoutMs) => until(st, (s) => s.firstDataMs != null || s.ended, timeoutMs);
const untilEvents = (st, n, timeoutMs) => until(st, (s) => s.events.length >= n || s.ended, timeoutMs);

async function upStats(upStatsUrl) {
  const r = await fetch(upStatsUrl, { signal: AbortSignal.timeout(5000) });
  return r.json();
}

// Per-event arrival lag vs the producer's own timestamp (same clock - the
// mock upstream is in-process). The acceptance-line caliber is the ADDED
// delta over the direct control leg, expressed as a good-event ratio:
//   delta_i = lag_leg_i - median(lag_direct)
//   goodRatio@T = fraction of events with delta_i <= T ms
function legLags(st) {
  return st.events
    .filter((e) => e.ts != null)
    .map((e) => e.arriveWall - e.ts);
}

function goodEventRatio(legSt, directMedianLag, thresholdMs) {
  const lags = legLags(legSt);
  if (lags.length === 0) return null;
  const good = lags.filter((l) => l - directMedianLag <= thresholdMs).length;
  return round(good / lags.length, 4);
}

// Arrival spread ratio used by the flush assertion: ~1 = per-event streamed,
// ~0 = buffered-to-EOF bursts.
function arrivalSpread(st, intervalMs) {
  return st.events.length > 1
    ? (st.events.at(-1).arrive - st.events[0].arrive) /
        ((st.events.length - 1) * intervalMs)
    : 0;
}

// Paired TTFB delta against the direct leg (alternating measurements cancel
// runner jitter, Floe/Istio-style). opener(url, which) with which = null for
// the direct control and "proxy" for the leg under test.
async function pairedTtfb({ upstreamPort, opener, pairs = 100, warmup = 20 }) {
  const deltas = [];
  const d0 = nowMs();
  for (let i = 0; i < warmup + pairs; i++) {
    const u = `http://127.0.0.1:${upstreamPort}/sse?interval=25&count=2`;
    const d = opener(u, null);
    await untilFirstData(d.st, 8000);
    d.st.destroy();
    const p = opener(u, "proxy");
    await untilFirstData(p.st, 8000);
    p.st.destroy();
    if (process.env.BENCH_SSE_DEBUG && i % 10 === 0) {
      console.log(`[ttfb] i=${i} t=${Math.round(nowMs()-d0)}ms d=${d.st.firstDataMs} p=${p.st.firstDataMs} perr=${p.st.err} pstatus=${p.st.status}`);
    }
    if (i >= warmup && d.st.firstDataMs != null && p.st.firstDataMs != null) {
      deltas.push(p.st.firstDataMs - d.st.firstDataMs);
    }
  }
  return roundStats(summarize(deltas));
}

const dbg=(m)=>{if(process.env.BENCH_SSE_DEBUG)console.log("[sse "+Date.now()%100000+"] "+m)};

export async function runSse(ctx) {
  const { epPort, upstreamPort, identity, upStatsUrl, resinRssMB, modeAPort } =
    ctx;
  const agent = new http.Agent({ keepAlive: false, maxSockets: 1024 });
  // In-band error leg starts FIRST: Resin tunnels hold the client open on an
  // ~80s engine-side timer after upstream EOF (measured 79.3s), so its
  // EOF-propagation wait overlaps the rest of the phase instead of adding
  // ~80s of tail latency.
  const inband = modeAPort
    ? openTunnelStream({
        epPort: modeAPort,
        url: `http://127.0.0.1:${upstreamPort}/sse-error?after=4&interval=20`,
        creds: null,
      })
    : openStream({
        epPort,
        identity,
        url: `http://127.0.0.1:${upstreamPort}/sse-error?after=4&interval=20`,
        agent,
      });
  const results = {
    assertions: {},
    assertionsPassed: 0,
    exemptions: [],
    modeB: {},
    tunnel: {},
    modeA: {
      skipped: modeAPort ? null : "no forwarder port (build bench-forwarder)",
    },
  };

  // ---- TTFB paired deltas --------------------------------------------------
  // Mode B leg (forward-GET through the endpoint) - recorded, Resin domain.
  dbg("modeB ttfb start");results.modeB.ttfbDelta = await pairedTtfb({
    upstreamPort,
    opener: (u, which) =>
      which === "proxy"
        ? openStream({ epPort, identity, url: u, agent })
        : openStream({ url: u }),
  });

  // ---- Mode B event lag (recorded; registered exemption applies) -----------
  dbg("modeB ttfb done; modeB lag leg");const sB = openStream({
    epPort,
    identity,
    url: `http://127.0.0.1:${upstreamPort}/sse?interval=50&count=100`,
    agent,
  });
  const directCtl = openStream({
    url: `http://127.0.0.1:${upstreamPort}/sse?interval=50&count=100`,
  });
  await untilEvents(sB.st, 100, 30000);
  sB.st.destroy();
  await untilEvents(directCtl.st, 100, 30000);
  directCtl.st.destroy();
  dbg("modeB lag done events="+sB.st.events.length);
  await directCtl.done;
  results.modeB.eventLag = roundStats(summarize(legLags(sB.st)));
  const directLagMedian = summarize(legLags(directCtl.st)).p50 ?? 0;
  results.modeB.goodEventRatio10ms = goodEventRatio(sB.st, directLagMedian, 10);
  results.modeB.goodEventRatio500ms = goodEventRatio(
    sB.st,
    directLagMedian,
    500,
  );
  results.exemptions.push({
    id: "mode-b-sse-buffering",
    scope: "Mode B forward-GET path (Resin data plane)",
    registeredIn:
      "docs/how-to/PERF-BENCH.md Known-measured-behavior + RESIN_UPSTREAM_MANIFEST.yaml",
    removalCondition:
      "upstream implements per-event flush on the forward path",
  });

  // ---- CONNECT legs: endpoint tunnel (reference) + Mode A forwarder --------
  const intervalMs = 100;
  const countN = 25;
  const sseUrl = `http://127.0.0.1:${upstreamPort}/sse?interval=${intervalMs}&count=${countN}`;

  dbg("tunnel leg start");const tf = openTunnelStream({ epPort, identity, url: sseUrl });
  await untilEvents(tf.st, countN, intervalMs * countN * 4 + 15000);
  tf.st.destroy();
  dbg("tunnel done events="+tf.st.events.length);
  results.tunnel.spread = round(arrivalSpread(tf.st, intervalMs), 3);
  results.tunnel.events = tf.st.events.length;

  // Mode A leg - the desktop-shipped path. Runs only when the harness spawned
  // a bench-forwarder (run-bench passes ctx.modeAPort).
  if (modeAPort) {
    results.modeA.skipped = null;
    dbg("modeA ttfb start");results.modeA.ttfbDelta = await pairedTtfb({
      upstreamPort,
      opener: (u, which) =>
        which === "proxy"
          ? openTunnelStream({ epPort: modeAPort, url: u, creds: null })
          : openStream({ url: u }),
    });

    dbg("modeA ttfb done; lag leg");const legA = openTunnelStream({
      epPort: modeAPort,
      url: `http://127.0.0.1:${upstreamPort}/sse?interval=50&count=100`,
      creds: null,
    });
    const directA = openStream({
      url: `http://127.0.0.1:${upstreamPort}/sse?interval=50&count=100`,
    });
    await untilEvents(legA.st, 100, 30000);
    legA.st.destroy();
    await untilEvents(directA.st, 100, 30000);
    directA.st.destroy();
    dbg("modeA lag done events="+legA.st.events.length);
    await directA.done;
    const directLagMedianA = summarize(legLags(directA.st)).p50 ?? 0;
    const lagsA = legLags(legA.st);
    results.modeA.eventLag = roundStats(summarize(lagsA));
    results.modeA.eventGapDelta = roundStats(
      summarize(lagsA.map((l) => l - directLagMedianA)),
    );
    results.modeA.goodEventRatio10ms = goodEventRatio(
      legA.st,
      directLagMedianA,
      10,
    );
    results.modeA.goodEventRatio500ms = goodEventRatio(
      legA.st,
      directLagMedianA,
      500,
    );

    dbg("modeA flush leg");const fA = openTunnelStream({ epPort: modeAPort, url: sseUrl, creds: null });
    await untilEvents(fA.st, countN, intervalMs * countN * 4 + 15000);
    fA.st.destroy();
    dbg("modeA flush done");
    results.modeA.spread = round(arrivalSpread(fA.st, intervalMs), 3);
    results.modeA.events = fA.st.events.length;
  }

  // ---- Assertion 1: per-event flush -----------------------------------------
  // Asserted on the CONNECT-tunnel class (modeA when the forwarder ran, else
  // the endpoint tunnel). The Mode B forward-path spread is recorded as the
  // registered exemption's evidence, never as a failing assertion.
  const flushSpread = modeAPort ? results.modeA.spread : results.tunnel.spread;
  const flushEvents = modeAPort ? results.modeA.events : results.tunnel.events;
  results.assertions.flush = {
    pass: flushSpread >= 0.6 && flushEvents >= 20,
    leg: modeAPort ? "modeA" : "tunnel",
    spread: flushSpread,
    events: flushEvents,
    modeB: {
      spread: round(arrivalSpread(sB.st, 50), 3),
      events: sB.st.events.length,
      registeredExemption: "mode-b-sse-buffering",
    },
  };

  // ---- Assertion 2: client disconnect cascades cancel upstream --------------
  // Path-agnostic (the upstream /__stats.cancels counter sees the relay drop
  // whichever leg carried it); driven on the Mode A leg when available.
  {
    dbg("cancel leg");const before = (await upStats(upStatsUrl)).cancels;
    const openLeg = () =>
      modeAPort
        ? openTunnelStream({
            epPort: modeAPort,
            url: `http://127.0.0.1:${upstreamPort}/sse?interval=500`,
            creds: null,
          })
        : openStream({
            epPort,
            identity,
            url: `http://127.0.0.1:${upstreamPort}/sse?interval=500`,
            agent,
          });
    const streams = [];
    for (let i = 0; i < 5; i++) streams.push(openLeg());
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
    results.assertions.cancel = {
      pass: ok,
      leg: modeAPort ? "modeA" : "modeB",
      killed: kill,
      upstreamCancels: cancelsObserved,
    };
  }

  // ---- Assertion 3: bounded backpressure ------------------------------------
  // Pause reads on a fast stream; upstream must observe blocked writes and
  // resin RSS must stay bounded. On the tunnel/modeA leg backpressure is
  // induced by sock.pause() - no reads means the kernel buffer fills.
  {
    dbg("backpressure leg");const bp = modeAPort
      ? openTunnelStream({
          epPort: modeAPort,
          url: `http://127.0.0.1:${upstreamPort}/sse?interval=1&size=4096`,
          creds: null,
        })
      : openStream({
          epPort,
          identity,
          url: `http://127.0.0.1:${upstreamPort}/sse?interval=1&size=4096`,
          agent,
        });
    await sleep(600);
    if (bp.st.res) bp.st.res.pause();
    if (bp.st.sock) bp.st.sock.pause();
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
      leg: modeAPort ? "modeA" : "modeB",
      upstreamBlocked,
      rssDeltaMB: round(rssDelta, 2),
      upstreamBlockedWritesDelta: s1.blockedWrites - s0.blockedWrites,
      upstreamWritableLength: s1.writableLength,
    };
  }

  // ---- Assertion 4: in-band error signaling mid-stream ----------------------
  // The error frame must arrive in-band AND the tunnel must close cleanly.
  // Resin's tunnel close is timer-driven (~80s after upstream EOF), so the
  // leg was opened at phase start and we wait out the engine timer here.
  {
    const e = inband;
    await until(
      e.st,
      (s) => s.events.some((ev) => ev.error != null) || s.ended,
      10000,
    );
    dbg("inband error frame seen; awaiting engine tunnel close");
    await until(e.st, (s) => s.ended, 85000);
    e.st.destroy();
    dbg("inband done");
    const gotErr = e.st.events.some((ev) => ev.error?.code === "UPSTREAM_BOOM");
    results.assertions.inbandError = {
      pass: gotErr && e.st.ended,
      leg: modeAPort ? "modeA" : "modeB",
      ended: e.st.ended,
      eofMs: e.st.endMs ?? null,
      sawError: gotErr,
      events: e.st.events.length,
    };
  }

  agent.destroy();
  results.assertionsPassed = Object.values(results.assertions).filter(
    (a) => a.pass,
  ).length;
  return results;
}
