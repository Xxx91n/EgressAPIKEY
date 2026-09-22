// Real-WebView smoke: drives the CI-built
// portable exe through the embedded WebDriver server compiled in by
// --features "custom-protocol wdio-smoke" (tauri-plugin-wdio-webdriver listens
// on TAURI_WEBDRIVER_PORT; tauri-plugin-wdio backs browser.tauri.execute).
//
// Report-only harness: assertions are recorded into smoke-results.json and
// printed; the process exits non-zero on failure. Report-only semantics come
// from the trigger design, not a job flag: the workflow runs on post-merge
// pushes and nightly only, so a failure is evidence on main, never a PR gate.
//
//   S1 window up     - webview session opens + document title == EgressAPIKEY
//                      (+ MainWindowTitle on win32 - the ADR-0072 liveness)
//   S2 nav switch    - Topology -> Settings renders the network save control
//   S3 ipc roundtrip - port_list returns an array through the real IPC bridge
//   S4 remove path   - platform_add -> card renders -> UI delete -> gone
//                      (the previously weakened e2e remove assertion, restored)
//   S5 glyph matrix  - ar/hi/th locales: no horizontal overflow, nav intact,
//                      one screenshot per locale (uploaded as CI artifact)
//
// Env: APP_EXE (required), TAURI_WEBDRIVER_PORT (default 4445), OUT_DIR.

import { spawn, execFile } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { remote } from "webdriverio";

const APP_EXE = process.env.APP_EXE;
const PORT = Number(process.env.TAURI_WEBDRIVER_PORT || 4445);
const OUT_DIR = process.env.OUT_DIR || "webview-smoke-results";
const isWin = process.platform === "win32";

const results = {
  meta: {
    appExe: APP_EXE,
    os: process.platform,
    ciRunId: process.env.GITHUB_RUN_ID ?? null,
    sha: process.env.GITHUB_SHA ?? null,
    startedAt: new Date().toISOString(),
  },
  assertions: {},
  stdoutTail: [],
};
const record = (id, pass, detail) => {
  results.assertions[id] = { pass: !!pass, ...(detail ?? {}) };
  console.log(`[smoke] ${id}: ${pass ? "PASS" : "FAIL"} ${JSON.stringify(detail ?? "")}`);
};

if (!APP_EXE || !fs.existsSync(APP_EXE)) {
  console.error(`APP_EXE missing: ${APP_EXE}`);
  results.assertions.env = { pass: false, detail: `APP_EXE missing: ${APP_EXE}` };
  fs.mkdirSync(OUT_DIR, { recursive: true });
  fs.writeFileSync(path.join(OUT_DIR, "smoke-results.json"), JSON.stringify(results, null, 2));
  process.exit(1);
}
fs.mkdirSync(OUT_DIR, { recursive: true });

// ---- launch ---------------------------------------------------------------
const child = spawn(APP_EXE, [], {
  env: { ...process.env, TAURI_WEBDRIVER_PORT: String(PORT) },
  stdio: ["ignore", "pipe", "pipe"],
});
child.stdout.on("data", (d) => {
  results.stdoutTail.push(...String(d).split("\n").filter(Boolean));
  if (results.stdoutTail.length > 200) results.stdoutTail.splice(0, results.stdoutTail.length - 200);
});
child.stderr.on("data", (d) => {
  results.stdoutTail.push(...String(d).split("\n").filter(Boolean));
  if (results.stdoutTail.length > 200) results.stdoutTail.splice(0, results.stdoutTail.length - 200);
});

const waitPort = (port, deadlineMs) =>
  new Promise((resolve, reject) => {
    const t0 = Date.now();
    const tick = () => {
      const s = net.connect(port, "127.0.0.1", () => {
        s.destroy();
        resolve();
      });
      s.on("error", () => {
        if (Date.now() - t0 > deadlineMs) reject(new Error("webdriver port never opened"));
        else setTimeout(tick, 250);
      });
      s.setTimeout(500, () => s.destroy());
    };
    tick();
  });

const invoke = async (browser, cmd, args) => {
  // tauri-plugin-wdio exposes __TAURI_INTERNALS__ regardless of withGlobalTauri.
  return browser.execute(
    (c, a) => window.__TAURI_INTERNALS__.invoke(c, a),
    cmd,
    args ?? {},
  );
};

const killTree = () => {
  try {
    if (isWin) {
      spawn("taskkill", ["/F", "/T", "/PID", String(child.pid)]);
    } else {
      child.kill("SIGKILL");
    }
  } catch { }
};

let browser = null;
try {
  await waitPort(PORT, 30000);
  browser = await remote({
    hostname: "127.0.0.1",
    port: PORT,
    path: "/",
    capabilities: {
      // The embedded server accepts a bare alwaysMatch; browserName is a
      // formality - the session binds the app's own webview.
      browserName: "webview",
      "wdio:enforceWebDriverClassic": true,
    },
    logLevel: "warn",
    connectionRetryCount: 5,
    connectionRetryTimeout: 30000,
  });

  // ---- S1: window up + title liveness --------------------------------------
  {
    let title = null;
    try {
      title = await browser.getTitle();
    } catch (e) {
      title = `getTitle failed: ${e.message}`;
    }
    let nativeTitle = null;
    if (isWin) {
      nativeTitle = await new Promise((resolve) => {
        execFile(
          "powershell",
          ["-NoProfile", "-Command", `(Get-Process -Id ${child.pid} -ErrorAction SilentlyContinue).MainWindowTitle`],
          { timeout: 8000 },
          (_e, out) => resolve((out ?? "").trim() || null),
        );
      });
    }
    const tracingOk = results.stdoutTail.some((l) => /tracing initialized/i.test(l));
    const panic = results.stdoutTail.some((l) => /panic/i.test(l));
    record("s1-window-up", title === "EgressAPIKEY" && (nativeTitle === null || nativeTitle === "EgressAPIKEY") && !panic, {
      documentTitle: title,
      nativeTitle,
      panic,
      tracingOk,
    });
  }

  // ---- S2: nav switch -------------------------------------------------------
  {
    let ok = false;
    try {
      // Side-rail nav buttons are icon-only (label lives on aria-label),
      // so the smoke drives navigation via the stable nav-<view> testid.
      {
        const navBtn = await browser.$('button[data-testid="nav-settings"]');
        await navBtn.waitForExist({ timeout: 8000 });
        await navBtn.click();
      }
      const el = await browser.$('[data-testid="net-save-btn"]');
      ok = await el.waitForExist({ timeout: 8000 });
    } catch (e) {
      record("s2-nav-switch", false, { error: String(e).slice(0, 300) });
      ok = null;
    }
    if (ok !== null) record("s2-nav-switch", ok);
  }

  // ---- S3: IPC roundtrip (port_list) ----------------------------------------
  {
    try {
      const list = await invoke(browser, "port_list");
      record("s3-ipc-roundtrip", Array.isArray(list), { type: typeof list, len: Array.isArray(list) ? list.length : null });
    } catch (e) {
      record("s3-ipc-roundtrip", false, { error: String(e).slice(0, 300) });
    }
  }

  // ---- S4: platform remove path -------------------------------
  {
    const name = `smoke-${Date.now().toString(36)}`;
    try {
      await invoke(browser, "platform_add", { name });
      // render check: the card appears after refreshPlatforms
      {
        const navBtn = await browser.$('button[data-testid="nav-platforms"]');
        await navBtn.waitForExist({ timeout: 8000 });
        await navBtn.click();
      }
      const card = await browser.$(`[data-testid="platform-card-${name}"]`);
      const shown = await card.waitForExist({ timeout: 10000 }).then(() => true).catch(() => false);
      const del = await browser.$(`[data-testid="platform-delete-${name}"]`);
      let clicked = false;
      if (await del.isExisting()) {
        await del.click();
        clicked = true;
      }
      // post-state: IPC list + DOM both clean
      await new Promise((r) => setTimeout(r, 1500));
      const list = await invoke(browser, "platform_list");
      const stillThere = Array.isArray(list) && list.some((p) => p.name === name);
      const cardGone = !(await card.isExisting());
      record("s4-remove-path", shown && clicked && !stillThere && cardGone, {
        shown,
        clicked,
        stillThere,
        cardGone,
      });
    } catch (e) {
      record("s4-remove-path", false, { error: String(e).slice(0, 300) });
    }
  }

  // ---- S5: ar/hi/th glyph non-overflow ---------------------------------------
  {
    {
      const navBtn = await browser.$('button[data-testid="nav-settings"]');
      await navBtn.waitForExist({ timeout: 8000 });
      await navBtn.click();
    }
    const per = {};
    let allOk = true;
    for (const loc of ["ar", "hi", "th"]) {
      try {
        await browser.execute((l) => {
          const sel = document.querySelector('[data-testid="settings-locale-select"]');
          const proto = Object.getPrototypeOf(sel);
          const setter = Object.getOwnPropertyDescriptor(proto, "value").set;
          setter.call(sel, l);
          sel.dispatchEvent(new Event("change", { bubbles: true }));
        }, loc);
        await new Promise((r) => setTimeout(r, 1200));
        const metrics = await browser.execute(() => {
          const de = document.documentElement;
          const navBtns = [...document.querySelectorAll("nav button, [role='navigation'] button")];
          return {
            scrollW: de.scrollWidth,
            clientW: de.clientWidth,
            scrollH: de.scrollHeight,
            navButtons: navBtns.length,
            navClipped: navBtns.filter((b) => b.offsetWidth === 0 || b.getBoundingClientRect().right > window.innerWidth + 1).length,
            bodyTextLen: document.body.innerText.length,
          };
        });
        const overflow = metrics.scrollW - metrics.clientW;
        const ok = overflow <= 2 && metrics.navButtons >= 5 && metrics.navClipped === 0 && metrics.bodyTextLen > 20;
        per[loc] = { ok, ...metrics };
        if (!ok) allOk = false;
        const shot = path.join(OUT_DIR, `locale-${loc}.png`);
        try {
          fs.writeFileSync(shot, await browser.takeScreenshot(), "base64");
        } catch { }
      } catch (e) {
        per[loc] = { ok: false, error: String(e).slice(0, 200) };
        allOk = false;
      }
    }
    // restore en
    try {
      await browser.execute(() => {
        const sel = document.querySelector('[data-testid="settings-locale-select"]');
        const proto = Object.getPrototypeOf(sel);
        Object.getOwnPropertyDescriptor(proto, "value").set.call(sel, "en");
        sel.dispatchEvent(new Event("change", { bubbles: true }));
      });
    } catch { }
    record("s5-glyph-matrix", allOk, per);
  }
} catch (e) {
  results.assertions.harness = { pass: false, error: String(e?.stack ?? e).slice(0, 500) };
  console.error("[smoke] harness error:", e);
} finally {
  try {
    await browser?.deleteSession();
  } catch { }
  killTree();
  results.meta.finishedAt = new Date().toISOString();
  results.passed = Object.values(results.assertions).filter((a) => a.pass).length;
  results.total = Object.keys(results.assertions).length;
  fs.writeFileSync(path.join(OUT_DIR, "smoke-results.json"), JSON.stringify(results, null, 2));
  console.log(`[smoke] ${results.passed}/${results.total} assertions passed -> ${OUT_DIR}/smoke-results.json`);
  await new Promise((r) => setTimeout(r, 400));
  process.exit(results.passed === results.total && results.total >= 5 ? 0 : 1);
}
