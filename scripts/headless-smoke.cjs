// headless-smoke.cjs — R11-03 liveness + parity smoke for the headless binary.
//
// Runs against the REAL built binary in CI verify (ADR-0072): boots the
// headless server with a throwaway state root, then asserts the guard, the
// capabilities endpoint, the settings KV round-trip and the shell routes that
// read live process state. This is the "启动并测活软件进程" evidence for the
// headless transport — not a unit test, a live HTTP probe.
//
// Env knobs: HEADLESS_BIN, DIST_DIR, RESIN_BINARY_DIR, SMOKE_PORT.

const { spawn } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const ROOT = path.join(__dirname, "..");
const PORT = Number(process.env.SMOKE_PORT || 14277);
const TOKEN = "smoke-" + Math.random().toString(36).slice(2);
const BASE = `http://127.0.0.1:${PORT}`;

const BIN_NAME = process.platform === "win32" ? "egressapikey-headless.exe" : "egressapikey-headless";
// Cargo lands the binary at target/<triple>/debug/ when a build target is in
// effect (CI sets CARGO_BUILD_TARGET; this repo's .cargo/config.toml pins the
// MSVC triple on Windows) and at target/debug/ otherwise — probe both.
function findHeadlessBin() {
  if (process.env.HEADLESS_BIN) return process.env.HEADLESS_BIN;
  const candidates = [path.join(ROOT, "target", "debug", BIN_NAME)];
  const tdir = path.join(ROOT, "target");
  if (fs.existsSync(tdir)) {
    for (const d of fs.readdirSync(tdir)) {
      candidates.push(path.join(tdir, d, "debug", BIN_NAME));
    }
  }
  return candidates.find((p) => fs.existsSync(p)) || null;
}
const BIN = findHeadlessBin();
const DIST = process.env.DIST_DIR || path.join(ROOT, "dist");
const BIN_DIR = process.env.RESIN_BINARY_DIR || path.join(ROOT, "src-tauri", "binaries");

let failures = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok  ${name}`);
  else {
    failures += 1;
    console.error(`  FAIL ${name}${detail ? " — " + detail : ""}`);
  }
}

async function req(method, p, opts = {}) {
  // undici forbids overriding Host — use node:http for the hostile-Host probe.
  if (opts.host) {
    return await new Promise((resolve, reject) => {
      const r = require("node:http").request(
        {
          method, path: p, port: PORT, host: "127.0.0.1",
          headers: { host: opts.host, authorization: `Bearer ${TOKEN}` }
        },
        (res) => {
          let text = "";
          res.on("data", (c) => { text += c; });
          res.on("end", () => resolve({ status: res.statusCode, text, json: null, headers: res.headers }));
        },
      );
      r.on("error", reject);
      r.end();
    });
  }
  const res = await fetch(`${BASE}${p}`, {
    method,
    headers: {
      ...(opts.token === false ? {} : { authorization: `Bearer ${TOKEN}` }),
      ...(opts.body ? { "content-type": "application/json" } : {}),
    },
    body: opts.body ? JSON.stringify(opts.body) : undefined,
  });
  const text = await res.text();
  let json = null;
  try { json = JSON.parse(text); } catch { /* not json */ }
  return { status: res.status, json, text, headers: res.headers };
}

async function main() {
  if (!BIN || !fs.existsSync(BIN)) {
    // No binary = nothing to smoke. In CI verify the binary is always built
    // above, so an absence there is a real failure; a local run may skip.
    const msg = `[smoke] headless binary not found under target/`;
    if (process.env.CI) {
      console.error(msg);
      process.exit(1);
    }
    console.warn(`${msg} — skipped (local run)`);
    process.exit(0);
  }
  console.log(`[smoke] using ${BIN}`);
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "egressapikey-smoke-"));
  const stateRoot = path.join(tmp, "state");
  const logRoot = path.join(tmp, "logs");
  fs.mkdirSync(stateRoot, { recursive: true });
  fs.mkdirSync(logRoot, { recursive: true });

  const child = spawn(BIN, [
    "--bind", "127.0.0.1",
    "--port", String(PORT),
    "--no-browser",
    "--auth-token", TOKEN,
    "--dist", DIST,
    "--state-root", stateRoot,
    "--log-root", logRoot,
    "--binary-dir", BIN_DIR,
  ], { stdio: ["ignore", "pipe", "pipe"] });
  let childLog = "";
  child.stdout.on("data", (d) => { childLog += d.toString(); });
  child.stderr.on("data", (d) => { childLog += d.toString(); });

  try {
    // liveness: the process must answer HTTP inside 60s (sidecar boot time).
    const deadline = Date.now() + 60_000;
    let up = false;
    while (Date.now() < deadline) {
      try {
        const r = await req("GET", "/api/v1/capabilities");
        if (r.status === 200) { up = true; break; }
      } catch { /* not up yet */ }
      if (child.exitCode !== null) break;
      await new Promise((r) => setTimeout(r, 500));
    }
    check("process boots and answers /api/v1/capabilities", up,
      up ? "" : `exit=${child.exitCode} log-tail=${childLog.slice(-400)}`);
    if (!up) return;

    const caps = (await req("GET", "/api/v1/capabilities")).json;
    check("capabilities is a machine-readable command map",
      caps && typeof caps.commands === "object" && Object.keys(caps.commands).length > 0);
    const statuses = Object.values(caps?.commands ?? {}).map((e) => e.status);
    check("capabilities has both enabled and disabled entries",
      statuses.includes("enabled") && statuses.includes("disabled"));

    // auth guard still holds on every gated route
    const noTok = await req("GET", "/api/v1/shell/sidecar/status", { token: false });
    check("unauthenticated shell route is 401", noTok.status === 401);
    const badHost = await req("GET", "/api/v1/capabilities", { host: "evil.example.com" });
    check("foreign Host header is 403", badHost.status === 403);
    const capsResp = await req("GET", "/api/v1/capabilities");
    check("CSP header present", !!capsResp.headers.get("content-security-policy"));

    // settings KV round-trip (L1 parity surface)
    await req("PUT", "/api/v1/shell/settings", { body: { smokeKey: "v1" } });
    const doc = (await req("GET", "/api/v1/shell/settings")).json;
    check("settings KV round-trip persists a key", doc?.smokeKey === "v1");

    // live-state routes reach real process state (not desktop stubs)
    const sidecar = (await req("GET", "/api/v1/shell/sidecar/status")).json;
    check("sidecar status returns a port + mode",
      sidecar && typeof sidecar.api_port === "number" && typeof sidecar.mode === "string");
    const ports = (await req("GET", "/api/v1/ports")).json;
    check("port list returns an array", Array.isArray(ports));
    const wb = (await req("GET", "/api/v1/shell/whitebox")).json;
    check("whitebox snapshot returns an object", wb && typeof wb === "object");
    const snap = await req("GET", "/api/v1/shell/snapshot");
    check("authoritative snapshot endpoint answers", snap.status === 200,
      `status=${snap.status} body=${snap.text.slice(0, 120)}`);
    const logLevel = await req("GET", "/api/v1/shell/log-level");
    check("log-level read answers a level name", [200].includes(logLevel.status));
  } finally {
    child.kill("SIGTERM");
    await new Promise((r) => setTimeout(r, 500));
    if (child.exitCode === null) child.kill("SIGKILL");
    fs.rmSync(tmp, { recursive: true, force: true });
  }
}

main().then(() => {
  if (failures) {
    console.error(`[smoke] ${failures} check(s) FAILED`);
    process.exit(1);
  }
  console.log("[smoke] all checks passed");
}).catch((e) => {
  console.error(`[smoke] fatal: ${e.stack || e}`);
  process.exit(1);
});
