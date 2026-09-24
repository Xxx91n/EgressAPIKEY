#!/usr/bin/env node
'use strict';

/*
 * mode-a-contract-check.cjs - ADR-0068 D4 four-scenario contract gate, live
 * black-box against the real Resin sidecar binary.
 *
 * Evidence lineage: the original Mode A landing data-plane Mode A with the
 * byte-level D4 gate table verified live against the bundled Resin binary
 * (.scratch/-closed-2026-09-16/repro/{d4-mixed,t17-contract}).
 * This script re-establishes the same evidence on every CI run so a Resin
 * upgrade that silently changes the engine contract fails the build instead
 * of surfacing as a user-visible regression. Hand recipe + baseline table:
 * tests/fixtures/t17-contract/README.md.
 *
 * Scenarios (per docs/adr/0068 D4, verified on custom endpoints):
 *   mixed  (allow_socks5 + allow_http_forward):
 *            HTTP CONNECT    -> HTTP/1.1 503 + X-Resin-Error: NO_AVAILABLE_NODES
 *            SOCKS5 greeting -> 05 00 (method accepted)
 *   http   (allow_http_forward only):
 *            SOCKS5 greeting -> 05 ff (no acceptable method)
 *   socks5 (allow_socks5 only):
 *            HTTP CONNECT    -> HTTP/1.1 403 + X-Resin-Error: ENDPOINT_CAPABILITY_DISABLED
 *
 * Identity attribution ("credential is identity", tests/fixtures/t17-contract): with
 * RESIN_PROXY_TOKEN empty the no-auth default endpoint still PUBLISHES the
 * client-supplied credential into routing metadata - HTTP Basic T17G.ac1 and
 * SOCKS5 RFC1929 UserPass T17G.ac2 both land in GET /api/v1/request-logs with
 * the account attributed even on the 503 / NO_AVAILABLE_NODES failure path,
 * while a no-credential control logs an empty account.
 *
 * Modes: CI (Resin binary dropped by scripts/fetch_resin.sh) runs the full
 * live probe and exits 1 on any mismatch. Locally without a binary this runs
 * static self-checks only, warns, and exits 0 (CI-only build policy
 * 2026-09-04: local never builds or downloads; live evidence = CI runs).
 *
 * Boundary law (R12-E1, registered legislation - NOT a new amnesty): Mode A
 * is a legislated dataplane under ADR-0068 D1/D3, so the static section also
 * freezes its boundary against tests/fixtures/mode-a-boundary/census.json:
 * protocol-family freeze {socks5-handshake, http-connect,
 * absolute-form->CONNECT}, no TLS fronting (ADR-0068 D4 verbatim), and no
 * new L7 features (function inventory + StreamSensor dimensions). Net-new
 * dataplane behavior belongs to the ADR-0068 D2 fork line; a deliberate,
 * adjudicated change updates the census in the same commit.
 */

const fs = require('fs');
const os = require('os');
const path = require('path');
const net = require('net');
const http = require('http');
const crypto = require('crypto');
const { spawn } = require('child_process');

const CRLF = String.fromCharCode(13, 10);

const REPO_ROOT = path.resolve(__dirname, '..');
const BIN_DIR = path.join(REPO_ROOT, 'src-tauri', 'binaries');
const IS_CI = process.env.CI === 'true';
const WIN = process.platform === 'win32';

// Extractor generation marker (r12-wave-f D-003): any change to the
// fn-extraction regex below MUST bump this constant AND census.json's
// extractor_version + refresh the census in the same commit - the gate
// asserts the equality so a regex-only edit fails closed.
const EXTRACTOR_VERSION = 2;

const seen = new Set();
let failures = 0;
function record(ok, label, detail) {
  const line = (ok ? 'OK   ' : 'FAIL ') + label + (detail ? '  :: ' + detail : '');
  console.log(line);
  seen.add(label);
  if (!ok) failures += 1;
}

function sleep(ms) { return new Promise(r => setTimeout(r, ms)); }

function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.listen(0, '127.0.0.1', () => {
      const port = server.address().port;
      server.close(() => resolve(port));
    });
    server.on('error', reject);
  });
}

function adminRequest(port, token, method, urlPath, bodyObj) {
  return new Promise((resolve, reject) => {
    const payload = bodyObj === undefined ? null : JSON.stringify(bodyObj);
    const headers = { Authorization: 'Bearer ' + token };
    if (payload) {
      headers['Content-Type'] = 'application/json';
      headers['Content-Length'] = Buffer.byteLength(payload);
    }
    const req = http.request({ host: '127.0.0.1', port: port, method: method, path: urlPath, headers: headers }, res => {
      const chunks = [];
      res.on('data', c => chunks.push(c));
      res.on('end', () => {
        const text = Buffer.concat(chunks).toString('utf8');
        let json = null;
        try { json = text ? JSON.parse(text) : null; } catch (e) { json = null; }
        resolve({ status: res.statusCode, text: text, json: json });
      });
    });
    req.on('error', reject);
    req.setTimeout(10000, () => req.destroy(new Error('admin request timeout: ' + method + ' ' + urlPath)));
    if (payload) req.write(payload);
    req.end();
  });
}

// Raw HTTP-proxy exchange on a proxy port: connect, write request lines,
// collect until the header-block terminator arrives. latin1 keeps bytes 1:1.
function httpExchange(port, requestLines) {
  return new Promise((resolve, reject) => {
    const wire = requestLines.join(CRLF) + CRLF + CRLF;
    const sock = net.connect({ host: '127.0.0.1', port: port });
    const chunks = [];
    let settled = false;
    const finish = (err) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      sock.destroy();
      if (err) reject(err); else resolve(Buffer.concat(chunks).toString('latin1'));
    };
    const timer = setTimeout(() => finish(new Error('http exchange timeout on port ' + port)), 8000);
    sock.on('connect', () => sock.write(Buffer.from(wire, 'latin1')));
    sock.on('data', d => {
      chunks.push(d);
      if (Buffer.concat(chunks).toString('latin1').indexOf(CRLF + CRLF) !== -1) finish(null);
    });
    sock.on('error', e => finish(e));
  });
}

// Sends the SOCKS5 greeting (05 01 00 = offer no-auth) and resolves with the
// 2-byte method reply hex, e.g. "0500" (accepted) or "05ff" (refused).
function socks5GreetingReply(port) {
  return new Promise((resolve, reject) => {
    const sock = net.connect({ host: '127.0.0.1', port: port });
    const chunks = [];
    let settled = false;
    const finish = (err) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      sock.destroy();
      if (err) reject(err); else resolve(Buffer.concat(chunks).toString('hex'));
    };
    const timer = setTimeout(() => finish(new Error('socks5 greeting timeout on port ' + port)), 8000);
    sock.on('connect', () => sock.write(Buffer.from([0x05, 0x01, 0x00])));
    sock.on('data', d => {
      chunks.push(d);
      if (Buffer.concat(chunks).length >= 2) finish(null);
    });
    sock.on('error', e => finish(e));
  });
}

// Full RFC1929 UserPass flow with identity T17G.ac2 (+ empty proxy token as
// password), then CONNECT 1.1.1.1:80; resolves with the CONNECT reply bytes
// (expect 05 01 general failure, delivered in-band per the D4 baseline).
function socks5UserPassConnectReply(port) {
  return new Promise((resolve, reject) => {
    const sock = net.connect({ host: '127.0.0.1', port: port });
    const chunks = [];
    let stage = 0;
    let settled = false;
    const user = Buffer.from('T17G.ac2', 'ascii');
    const authBuf = Buffer.alloc(3 + user.length);
    authBuf[0] = 0x01;
    authBuf[1] = user.length;
    user.copy(authBuf, 2);
    authBuf[2 + user.length] = 0x00;
    const connectBuf = Buffer.from([0x05, 0x01, 0x00, 0x01, 1, 1, 1, 1, 0, 80]);
    const finish = (err) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      sock.destroy();
      if (err) reject(err); else resolve(Buffer.concat(chunks));
    };
    const timer = setTimeout(() => finish(new Error('socks5 userpass flow timeout on port ' + port)), 10000);
    sock.on('connect', () => sock.write(Buffer.from([0x05, 0x01, 0x02])));
    sock.on('data', d => {
      chunks.push(d);
      const buf = Buffer.concat(chunks);
      if (stage === 0 && buf.length >= 2) {
        stage = 1;
        chunks.length = 0;
        sock.write(authBuf);
        return;
      }
      if (stage === 1 && buf.length >= 2) {
        stage = 2;
        chunks.length = 0;
        sock.write(connectBuf);
        return;
      }
      if (stage === 2 && buf.length >= 2) finish(null);
    });
    sock.on('close', () => {
      if (!settled && chunks.length >= 2) finish(null);
      else if (!settled) finish(new Error('socks5 connection closed before reply'));
    });
    sock.on('error', e => finish(e));
  });
}

function findResinBinary() {
  let entries;
  try {
    entries = fs.readdirSync(BIN_DIR);
  } catch (e) {
    return null;
  }
  const candidates = entries.filter(n => n.startsWith('resin-') && !n.endsWith('.sha256') && !n.endsWith('.tar.gz') && !n.endsWith('.zip'));
  const chosen = WIN ? candidates.find(n => n.endsWith('.exe')) : candidates.find(n => !n.endsWith('.exe'));
  const name = chosen || candidates[0];
  if (!name) return null;
  const full = path.join(BIN_DIR, name);
  if (!WIN) {
    try { fs.chmodSync(full, 493); } catch (e) { /* best effort */ }
  }
  return full;
}

function healthzOnce(port) {
  return new Promise(resolve => {
    const req = http.get({ host: '127.0.0.1', port: port, path: '/healthz', timeout: 1500 }, res => {
      res.resume();
      resolve(res.statusCode === 200);
    });
    req.on('timeout', () => { req.destroy(); resolve(false); });
    req.on('error', () => resolve(false));
  });
}

async function waitHealthy(port, deadlineMs) {
  const start = Date.now();
  while (Date.now() - start < deadlineMs) {
    if (await healthzOnce(port)) return true;
    await sleep(300);
  }
  return false;
}

function summaryAndExit() {
  console.log('');
  console.log('[mode-a-contract-check] checks: ' + seen.size + ', failures: ' + failures);
  process.exit(failures > 0 ? 1 : 0);
}
async function main() {
  // ---- static self-checks (always run; these ship in the repo) ----
  let verifyScript = '';
  try { verifyScript = fs.readFileSync(path.join(REPO_ROOT, 'scripts', 'verify-build.sh'), 'utf8'); } catch (e) { }
  record(verifyScript.indexOf('mode-a-contract-check.cjs') !== -1,
    'gate mounted in scripts/verify-build.sh',
    'contracts sub-step must invoke scripts/mode-a-contract-check.cjs');
  record(fs.existsSync(path.join(REPO_ROOT, 'tests', 'fixtures', 't17-contract', 'README.md')),
    'tests/fixtures/t17-contract/README.md present (baseline table + hand recipe)');

  // ---- Mode A boundary law (R12-E1 registered legislation assertions) ----
  // These run on every invocation (CI and local): they assert the Mode A
  // dataplane still matches the frozen census, so net-new dataplane
  // behavior trips the register line instead of landing quietly.
  const censusPath = path.join(REPO_ROOT, 'tests', 'fixtures', 'mode-a-boundary', 'census.json');
  let census = null;
  try { census = JSON.parse(fs.readFileSync(censusPath, 'utf8')); } catch (e) { }
  record(census !== null, 'boundary: census fixture present + parses',
    'tests/fixtures/mode-a-boundary/census.json');
  const fwdSrc = fs.readFileSync(path.join(REPO_ROOT, 'crates', 'resin-core', 'src', 'port_forwarder.rs'), 'utf8');
  const fwdPre = fwdSrc.split('#[cfg(test)]')[0];
  if (census) {
    // Ban 3 - no new L7 features: the non-test function inventory must
    // equal the census exactly (either direction of drift fails).
    // Extractor v2 (r12-wave-f D-003): pub(..)? then ANY
    // combination/order of const / async / unsafe / extern ".." before
    // fn (extern may also appear bare - Rust defaults to "C").
    // Qualifier combos that are invalid Rust may over-report -
    // fail-closed is intended: a phantom fn in actualFns fails the
    // inventory check loudly instead of a real fn slipping by silently.
    // Macro-generated fns are unreachable for ANY regex - the
    // fail-closed assertion below covers that blind spot.
    const actualFns = [...fwdPre.matchAll(/^\s*(?:pub\s*(?:\([^)]*\)\s*)?)?(?:(?:const|async|unsafe|extern(?:\s+"[^"\n]*")?)\s+)*fn\s+([A-Za-z_][A-Za-z0-9_]*)/gm)].map(m => m[1]);
    const missing = census.functions.filter(f => !actualFns.includes(f));
    const extra = actualFns.filter(f => !census.functions.includes(f));
    record(missing.length === 0 && extra.length === 0,
      'boundary: function inventory frozen at census (net-new dataplane behavior goes to the ADR-0068 D2 fork line)',
      missing.map(m => 'missing:' + m).concat(extra.map(e => 'new:' + e)).join(', ') || undefined);

    // Extractor/census version coupling (r12-wave-f D-003): the
    // assertion self-documents the rule - a regex upgrade bumps
    // EXTRACTOR_VERSION above AND census.extractor_version AND refreshes
    // the census, all in one commit.
    record(census.extractor_version === EXTRACTOR_VERSION,
      'boundary: census extractor_version matches gate extractor v' + EXTRACTOR_VERSION,
      census.extractor_version === EXTRACTOR_VERSION ? undefined
        : 'census has ' + JSON.stringify(census.extractor_version)
          + ' - bump EXTRACTOR_VERSION + census.extractor_version + refresh census in the same commit');

    // Declarative-macro fail-closed (r12-wave-f D-003): the regex
    // extractor cannot see macro-generated fns, so ANY macro_rules!
    // or macro definition in port_forwarder.rs fails the gate with the
    // offending line number - a ten-second human decision, not a silent
    // blind spot (cargo-public-api #858 panic precedent). Whole file,
    // not just the non-test prefix: over-reporting is the intent.
    const macroLines = fwdSrc.split('\n').map((l, i) => ({ t: l.trim(), n: i + 1 }))
      .filter(x => /^\s*(?:(?:pub\s+)?macro\s+[A-Za-z_][A-Za-z0-9_]*|macro_rules!\s*[A-Za-z_][A-Za-z0-9_]*)/.test(x.t));
    record(macroLines.length === 0,
      'boundary: no declarative-macro definitions in port_forwarder.rs (extractor blind spot - fail-closed)',
      macroLines.length === 0 ? undefined
        : macroLines.map(x => 'line ' + x.n + ': ' + x.t).join('; ')
          + ' - disposal: inline the expansion into census.json OR rewrite as a plain fn');

    // Ban 1 - protocol-family freeze: wire dialects stay {socks5, http}
    // (detect_protocol arms) and the declared entry-protocol value set
    // stays {http, mixed, socks5}.
    const detectBody = fwdPre.slice(fwdPre.indexOf('fn detect_protocol'), fwdPre.indexOf('fn dialect_allowed'));
    const dialectsOk = detectBody.includes('"socks5"') && detectBody.includes('"http"');
    const epSrc = fs.readFileSync(path.join(REPO_ROOT, 'crates', 'resin-core', 'src', 'entry_protocol.rs'), 'utf8');
    const epMatch = epSrc.match(/ENTRY_PORT_PROTOCOLS[^=]*=\s*\[([^\]]*)\]/);
    const epVals = epMatch ? [...epMatch[1].matchAll(/"([^"]+)"/g)].map(m => m[1]).sort() : [];
    record(dialectsOk && JSON.stringify(epVals) === JSON.stringify(census.entry_port_protocols.slice().sort()),
      'boundary: protocol-family freeze - dialects {socks5,http} + entry protocols {http,mixed,socks5}',
      'entry=' + JSON.stringify(epVals));

    // Rewrite whitelist anchors: the three legislated paths exist; the
    // census inventory above is what catches any additional rewrite rule.
    const anchors = ['fn handle_socks5', 'method == "CONNECT"', 'fn split_absolute_form', 'fn build_replayed_head'];
    const absent = anchors.filter(a => !fwdPre.includes(a));
    record(absent.length === 0,
      'boundary: rewrite whitelist {socks5-handshake, http-connect, absolute-form->CONNECT} anchors intact',
      absent.join(', ') || undefined);

    // Ban 2 - no TLS fronting: cite ADR-0068 D4 verbatim, do not
    // re-legislate. Tokens are code-shaped (a comment saying "TLS" is not
    // a violation; a TlsAcceptor or a 0x16 handshake byte is).
    const esc = s => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const tlsHits = census.banned_tls_tokens.filter(t =>
      new RegExp('(?<![A-Za-z0-9_])' + esc(t) + '(?![A-Za-z0-9_])', 'i').test(fwdSrc));
    record(tlsHits.length === 0,
      'boundary: no TLS fronting - ADR-0068 D4 verbatim: "' + census.adr_0068_d4_verbatim + '"',
      tlsHits.join(', ') || undefined);

    // StreamSensor classification dimensions frozen (new dimension =
    // net-new dataplane behavior per the register line).
    const sensorSrc = fs.readFileSync(path.join(REPO_ROOT, 'crates', 'resin-core', 'src', 'stream_sensor.rs'), 'utf8');
    const enumBody = (sensorSrc.split('enum StreamKind')[1] || '').split('}')[0];
    const dims = [...enumBody.matchAll(/([A-Za-z]+)\s*,/g)].map(m => m[1].toLowerCase()).sort();
    record(JSON.stringify(dims) === JSON.stringify(census.stream_sensor_dimensions.slice().sort()),
      'boundary: StreamSensor dimensions frozen {unary,sse,websocket,unknown}',
      'got ' + JSON.stringify(dims));
  }
  // Register wording: line count is a warn-only reference column, not a
  // trigger - report it, never assert on it. Convention (r12-wave-f
  // D-003): content.split('\n').length, matching census.json's
  // line_count_convention; the frozen figure is read from census
  // .frozen_at, not hardcoded.
  console.log('WARN  port_forwarder.rs = ' + fwdSrc.split('\n').length
    + ' lines (census frozen_at: ' + (census ? census.frozen_at : 'n/a')
    + '; warn-only reference, not a trigger)');

  // ---- locate the sidecar binary ----
  const bin = findResinBinary();
  if (!bin) {
    if (IS_CI) {
      record(false, 'resin sidecar binary present',
        'CI needs scripts/fetch_resin.sh output under src-tauri/binaries before this gate');
      summaryAndExit();
    }
    console.log('WARN  resin sidecar binary not found under ' + BIN_DIR);
    console.log('WARN  live contract probes skipped (local no-build/no-download policy;'
      + ' live evidence is produced by CI runs)');
    summaryAndExit();
  }

  // ---- boot Resin with fresh temp state ----
  const adminPort = await freePort();
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'egressapikey-contract-'));
  const stateDir = path.join(tmpRoot, 'state');
  const cacheDir = path.join(tmpRoot, 'cache');
  const logDir = path.join(tmpRoot, 'log');
  fs.mkdirSync(stateDir); fs.mkdirSync(cacheDir); fs.mkdirSync(logDir);
  const adminToken = crypto.randomBytes(24).toString('hex');
  const child = spawn(bin, [], {
    cwd: tmpRoot,
    stdio: ['ignore', 'pipe', 'pipe'],
    env: Object.assign({}, process.env, {
      RESIN_AUTH_VERSION: 'V1',
      RESIN_ADMIN_TOKEN: adminToken,
      RESIN_PROXY_TOKEN: '',
      RESIN_LISTEN_ADDRESS: '127.0.0.1',
      RESIN_PORT: String(adminPort),
      RESIN_STATE_DIR: stateDir,
      RESIN_CACHE_DIR: cacheDir,
      RESIN_LOG_DIR: logDir,
      RESIN_REQUEST_LOG_QUEUE_FLUSH_INTERVAL: '2s',
    }),
  });
  child.stdout.on('data', () => { });
  child.stderr.on('data', () => { });
  let childDead = false;
  child.on('exit', () => { childDead = true; });

  try {
    const healthy = await waitHealthy(adminPort, 20000);
    if (childDead) {
      record(false, 'resin booted (healthz 200)',
        'sidecar process exited before becoming healthy: ' + bin);
      summaryAndExit();
    }
    record(healthy, 'resin booted (healthz 200 within 20s)');
    if (!healthy) summaryAndExit();

    // ---- provision endpoints + platform ----
    const mixedPort = await freePort();
    const httpOnlyPort = await freePort();
    const socks5OnlyPort = await freePort();
    const eps = [
      ['mixed (allow_socks5 + allow_http_forward)', mixedPort, { allow_socks5: true, allow_http_forward: true }],
      ['http-only (allow_http_forward only)', httpOnlyPort, { allow_socks5: false, allow_http_forward: true }],
      ['socks5-only (allow_socks5 only)', socks5OnlyPort, { allow_socks5: true, allow_http_forward: false }],
    ];
    let epFail = '';
    for (const entry of eps) {
      const body = Object.assign({
        port: entry[1],
        enabled: true,
        allow_management: false,
        allow_proxy: true,
        allow_http_reverse: false,
      }, entry[2]);
      const st = await adminRequest(adminPort, adminToken, 'POST', '/api/v1/endpoints', body);
      if (st.status !== 201) {
        epFail = entry[0] + ' -> HTTP ' + st.status + ' ' + st.text.slice(0, 200);
        break;
      }
    }
    record(epFail === '', 'three custom endpoints created (mixed / http / socks5)', epFail || undefined);
    if (epFail) summaryAndExit();

    const platSt = await adminRequest(adminPort, adminToken, 'POST', '/api/v1/platforms', { name: 'T17G' });
    record(platSt.status === 201, 'identity platform T17G created',
      platSt.status === 201 ? '' : 'HTTP ' + platSt.status + ' ' + platSt.text.slice(0, 200));

    // ---- scenario 1+2: the mixed port speaks both dialects ----
    let resp = await httpExchange(mixedPort, [
      'CONNECT 1.1.1.1:443 HTTP/1.1',
      'Host: 1.1.1.1:443',
    ]);
    record(resp.startsWith('HTTP/1.1 503') && resp.toLowerCase().indexOf('x-resin-error: no_available_nodes') !== -1,
      'scenario 1: mixed port answers HTTP CONNECT with 503 + NO_AVAILABLE_NODES',
      resp.split(CRLF)[0]);

    const mixedGreet = await socks5GreetingReply(mixedPort);
    record(mixedGreet === '0500',
      'scenario 2: mixed port answers SOCKS5 greeting with method 05 00',
      'reply=' + mixedGreet);

    // ---- scenario 3: the http-only port refuses SOCKS5 ----
    const httpGreet = await socks5GreetingReply(httpOnlyPort);
    record(httpGreet === '05ff',
      'scenario 3: http-only port refuses SOCKS5 with 05 ff',
      'reply=' + httpGreet);

    // ---- scenario 4: the socks5-only port refuses HTTP ----
    resp = await httpExchange(socks5OnlyPort, [
      'CONNECT 1.1.1.1:443 HTTP/1.1',
      'Host: 1.1.1.1:443',
    ]);
    record(resp.startsWith('HTTP/1.1 403') && resp.toLowerCase().indexOf('x-resin-error: endpoint_capability_disabled') !== -1,
      'scenario 4: socks5-only port answers HTTP CONNECT with 403 + ENDPOINT_CAPABILITY_DISABLED',
      resp.split(CRLF)[0]);

    // ---- identity attribution on the no-auth default endpoint ----
    const basicAc1 = Buffer.from('T17G.ac1:', 'ascii').toString('base64');
    resp = await httpExchange(adminPort, [
      'GET http://1.1.1.1/ HTTP/1.1',
      'Host: 1.1.1.1',
      'Proxy-Authorization: Basic ' + basicAc1,
    ]);
    record(resp.startsWith('HTTP/1.1 503'),
      'identity: Basic T17G.ac1 request reaches routing (503 NO_AVAILABLE_NODES)',
      resp.split(CRLF)[0]);

    const upReply = await socks5UserPassConnectReply(adminPort);
    const upHex = upReply.toString('hex').slice(0, 4);
    record(upHex === '0500' || upHex === '0501',
      'identity: SOCKS5 RFC1929 T17G.ac2 handshake completed (in-band reply)',
      'reply=' + upHex);

    resp = await httpExchange(adminPort, [
      'GET http://1.1.1.1/ HTTP/1.1',
      'Host: 1.1.1.1',
    ]);
    record(resp.startsWith('HTTP/1.1 503'),
      'identity: no-credential control reaches routing (503)',
      resp.split(CRLF)[0]);

    // ---- assert attribution through the request-logs API ----
    const deadline = Date.now() + 20000;
    let sawRows = [];
    let attributed = false;
    while (Date.now() < deadline && !childDead) {
      const st = await adminRequest(adminPort, adminToken, 'GET', '/api/v1/request-logs?limit=100');
      if (st.status === 200 && st.json && Array.isArray(st.json.items)) {
        const items = st.json.items;
        const hasAc1 = items.some(r => r.account === 'ac1' && r.resin_error === 'NO_AVAILABLE_NODES');
        const hasAc2 = items.some(r => r.account === 'ac2' && r.resin_error === 'NO_AVAILABLE_NODES');
        const hasAnon = items.some(r => (r.account === '' || r.account == null) && r.resin_error === 'NO_AVAILABLE_NODES');
        if (hasAc1 && hasAc2 && hasAnon) {
          attributed = true;
          record(true, 'request-logs: Basic T17G.ac1 + UserPass T17G.ac2 + anon control all attributed on the failure path');
          break;
        }
        sawRows = items;
      }
      await sleep(500);
    }
    if (!attributed) {
      record(false, 'request-logs: identity rows visible within 20s',
        JSON.stringify(sawRows.slice(0, 8).map(r => ({ acct: r.account, status: r.http_status, err: r.resin_error }))));
    }
  } finally {
    try { child.kill(); } catch (e) { }
    await sleep(300);
    try { fs.rmSync(tmpRoot, { recursive: true, force: true }); } catch (e) { }
  }
  summaryAndExit();
}

main().catch(err => {
  console.error('FATAL ' + (err && err.message ? err.message : err));
  process.exit(1);
});
