// Mock upstream target + mock outbound node for the perf baseline harness.
// All loopback, zero external deps, zero internet requirement on the data path.
// EXCEPTION: the opt-in faultinject phase (R12-C1) egresses for real -
// probe_exit_ip is hardcoded to 1.1.1.1/cdn-cgi/trace through this node and
// Resin node health probes need live WAN reachability to flip routable.
//
// mock upstream (HTTP server):
//   GET /health          -> 200 "ok"
//   GET /echo            -> 200 fixed small JSON (fast path for latency/RPS)
//   GET /sse?interval&count&size -> text/event-stream; count=0 means infinite
//   GET /sse-error?after=K     -> K events then in-band data:{"error":...} + EOF
//   GET /__stats         -> counters: active streams, cancels, blocked writes
//
// mock node (HTTP CONNECT proxy on loopback): Resin's `http` outbound dials
// CONNECT host:port through it; we pipe to the real target. Load traffic
// targets only loopback (the mock upstream), but external CONNECTs ARE
// served - Resin's mandatory node health probes (cloudflare.com/gstatic)
// and the R12-C1 faultinject probe_exit_ip leg to 1.1.1.1 ride it.

import http from "node:http";
import net from "node:net";

export function listen(server, port, host = "127.0.0.1") {
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, host, () =>
      resolve({ server, port: server.address().port }),
    );
  });
}

export function startMockUpstream(port = 0) {
  const stats = {
    activeSse: 0,
    totalSse: 0,
    cancels: 0,
    chunksWritten: 0,
    blockedWrites: 0,
    bytesWritten: 0,
    echo: 0,
    writableLength: 0,
  };

  const server = http.createServer((req, res) => {
    const u = new URL(req.url, "http://mock");
    if (u.pathname === "/health") {
      res.writeHead(200);
      res.end("ok");
      return;
    }
    if (u.pathname === "/echo") {
      stats.echo++;
      res.writeHead(200, {
        "content-type": "application/json",
        "content-length": 15,
      });
      res.end('{"ok":1,"ts":0}');
      return;
    }
    if (u.pathname === "/__stats") {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify(stats));
      return;
    }
    if (u.pathname === "/sse" || u.pathname === "/sse-error") {
      const interval = Math.max(1, Number(u.searchParams.get("interval")) || 50);
      const count = Math.max(0, Number(u.searchParams.get("count")) || 0);
      const size = Math.max(16, Number(u.searchParams.get("size")) || 40);
      const after = Math.max(1, Number(u.searchParams.get("after")) || 5);
      stats.activeSse++;
      stats.totalSse++;
      res.writeHead(200, {
        "content-type": "text/event-stream",
        "cache-control": "no-cache",
        connection: "keep-alive",
        "x-accel-buffering": "no",
      });
      res.flushHeaders?.();
      let seq = 0;
      let done = false;
      const end = (byClient) => {
        if (done) return;
        done = true;
        clearInterval(timer);
        stats.activeSse--;
        if (byClient) stats.cancels++;
        try {
          res.end();
        } catch {}
      };
      const mk = (n) => {
        const head = `data: {"seq":${n},"ts":${Date.now()},"p":"`;
        const tail = '"}\n\n';
        return head + "x".repeat(Math.max(0, size - head.length - tail.length)) + tail;
      };
      const write = (line) => {
        const ok = res.write(line);
        stats.chunksWritten++;
        stats.bytesWritten += line.length;
        stats.writableLength = res.writableLength;
        if (!ok) stats.blockedWrites++;
      };
      const timer = setInterval(() => {
        if (done) return;
        seq++;
        if (u.pathname === "/sse-error" && seq > after) {
          write('data: {"error":{"code":"UPSTREAM_BOOM","message":"forced mid-stream"}}\n\n');
          end(false);
          return;
        }
        write(mk(seq));
        if (count > 0 && seq >= count) end(false);
      }, interval);
      req.on("close", () => end(true));
      res.on("close", () => end(true));
      return;
    }
    res.writeHead(404);
    res.end("not found");
  });
  server.keepAliveTimeout = 0;
  server.requestTimeout = 0;
  return listen(server, port).then((r) => ({ ...r, stats }));
}

// Minimal HTTP CONNECT proxy (the bench's "node" in the Clash subscription).
// Load traffic only ever targets loopback (the mock upstream); external
// CONNECTs are also served because Resin's mandatory node health probes
// (probe-egress -> cloudflare.com/cdn-cgi/trace, probe-latency -> gstatic
// generate_204) must egress for real before a node becomes routable.
// Listener is loopback-bound and short-lived (exists only inside a bench run).
export function startMockNode(port = 0) {
  const stats = {
    tunnels: 0,
    externalTunnels: 0,
    activeTunnels: 0,
    bytesUp: 0,
    bytesDown: 0,
    errors: 0,
  };
  const server = net.createServer((client) => {
    client.setNoDelay(true);
    let opened = false;
    let upstream = null;
    let headerBuf = Buffer.alloc(0);
    client.on("data", (head) => {
      if (opened) return; // after CONNECT established, pipe handles the rest
      headerBuf = Buffer.concat([headerBuf, head]);
      const hidx = headerBuf.indexOf("\r\n\r\n");
      if (hidx < 0) {
        if (headerBuf.length > 16384) {
          stats.errors++;
          client.destroy();
        }
        return; // wait for the complete CONNECT header block
      }
      const m = /^CONNECT ([^\s:]+):(\d+) HTTP\//i.exec(
        headerBuf.subarray(0, hidx).toString("latin1"),
      );
      if (!m) {
        stats.errors++;
        client.end("HTTP/1.1 405 Method Not Allowed\r\n\r\n");
        return;
      }
      const host = m[1];
      const targetPort = Number(m[2]);
      const loopback =
        host === "127.0.0.1" || host === "localhost" || host === "::1" || host === "[::1]";
      if (!loopback) stats.externalTunnels++;
      opened = true;
      const rest = headerBuf.subarray(hidx + 4);
      upstream = net.connect(targetPort, host, () => {
        client.write("HTTP/1.1 200 Connection Established\r\n\r\n");
        stats.tunnels++;
        stats.activeTunnels++;
        if (rest.length) upstream.write(rest);
        upstream.on("data", (c) => (stats.bytesDown += c.length));
        client.on("data", (c) => (stats.bytesUp += c.length));
        client.pipe(upstream);
        upstream.pipe(client);
      });
      upstream.setNoDelay(true);
      upstream.on("error", () => {
        stats.errors++;
        client.destroy();
      });
      const bye = () => {
        stats.activeTunnels = Math.max(0, stats.activeTunnels - 1);
      };
      client.once("close", () => {
        bye();
        upstream?.destroy();
      });
      upstream.once("close", () => client.destroy());
    });
    client.on("error", () => {});
  });
  // Track every accepted socket so a fault-injection kill can actually drop
  // live tunnels: server.close() alone hangs on open CONNECT pipes, which
  // would hold the port and keep the node partially alive.
  const sockets = new Set();
  server.on("connection", (s) => {
    sockets.add(s);
    s.on("close", () => sockets.delete(s));
  });
  const kill = async () => {
    for (const s of sockets) s.destroy();
    await new Promise((res) => server.close(() => res()));
  };
  return listen(server, port).then((r) => ({ ...r, stats, sockets, kill }));
}
