// Whole-process-tree WorkingSet/RSS sampler rooted at a spawned PID.
//
// Attribution model (r12-wave-a D-002 / R12-00): the desktop app's process
// tree is bounded by DESCENDANCY from the spawned root PID - never by
// process-name globbing. WebView2 ships a shared runtime (msedgewebview2)
// whose processes are pooled per user-data-dir and can be re-parented across
// app boundaries; name matching would count other apps' webview processes.
// Descendant BFS from our root PID is exact: every WebView2 browser/renderer/
// GPU/utility process we own is a transitive child of the app process.
//
//   windows: Get-CimInstance Win32_Process (ProcessId, ParentProcessId, Name,
//            CommandLine, WorkingSetSize) -> BFS children-of map -> sum WS64.
//   linux:   /proc/<pid>/{stat,status} PPid walk -> VmRSS sum.
//
// One-shot snapshot per call; the caller re-walks every sample so renderers
// spawned mid-run (navigation, crash-recovery) are picked up while dead PIDs
// drop out. Returns { totalMB, webview2MB, procs:[{pid,name,mb}], pids } or
// null when the tree is gone (root exited). webview2MB is the separate column
// the acceptance line demands: msedgewebview2* members of OUR tree only.

import { execFile } from "node:child_process";
import fs from "node:fs";

function bfsDescendants(rootPid, childrenOf) {
  const pids = [];
  const queue = [rootPid];
  const seen = new Set([rootPid]);
  while (queue.length) {
    const cur = queue.shift();
    for (const c of childrenOf.get(cur) ?? []) {
      if (!seen.has(c)) {
        seen.add(c);
        pids.push(c);
        queue.push(c);
      }
    }
  }
  return pids;
}

// Windows: one CIM query for the whole process table, then BFS in JS -
// one powershell spawn per sample instead of one per node.
function windowsTree(rootPid) {
  return new Promise((resolve) => {
    const ps =
      "Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,WorkingSetSize" +
      " | ConvertTo-Json -Compress";
    execFile(
      "powershell",
      ["-NoProfile", "-Command", ps],
      { timeout: 15000, maxBuffer: 8 * 1024 * 1024 },
      (err, stdout) => {
        if (err || !stdout.trim()) return resolve(null);
        let rows;
        try {
          const parsed = JSON.parse(stdout.trim());
          rows = Array.isArray(parsed) ? parsed : [parsed];
        } catch {
          return resolve(null);
        }
        const childrenOf = new Map();
        const byPid = new Map();
        for (const r of rows) {
          const pid = Number(r.ProcessId);
          const ppid = Number(r.ParentProcessId);
          byPid.set(pid, r);
          if (!childrenOf.has(ppid)) childrenOf.set(ppid, []);
          childrenOf.get(ppid).push(pid);
        }
        if (!byPid.has(rootPid)) return resolve(null); // root exited
        const pids = [rootPid, ...bfsDescendants(rootPid, childrenOf)];
        const procs = [];
        let total = 0;
        let webview2 = 0;
        for (const pid of pids) {
          const r = byPid.get(pid);
          if (!r) continue;
          const mb = Number(r.WorkingSetSize ?? 0) / 1048576;
          const name = String(r.Name ?? "");
          total += mb;
          if (/^msedgewebview2/i.test(name)) webview2 += mb;
          procs.push({ pid, name, mb: Math.round(mb * 100) / 100 });
        }
        resolve({
          totalMB: total,
          webview2MB: webview2,
          procs,
          pids,
        });
      },
    );
  });
}

function linuxTree(rootPid) {
  try {
    const childrenOf = new Map();
    const byPid = new Map();
    for (const d of fs.readdirSync("/proc")) {
      if (!/^\d+$/.test(d)) continue;
      const pid = Number(d);
      try {
        const stat = fs.readFileSync(`/proc/${pid}/stat`, "utf8");
        const after = stat.slice(stat.lastIndexOf(")") + 2).split(/\s+/);
        const ppid = Number(after[1]); // field 4 -> index 1 after comm split
        const comm = stat.slice(stat.indexOf("(") + 1, stat.lastIndexOf(")"));
        const status = fs.readFileSync(`/proc/${pid}/status`, "utf8");
        const m = /VmRSS:\s*(\d+)\s*kB/.exec(status);
        byPid.set(pid, { name: comm, rss: m ? Number(m[1]) * 1024 : 0 });
        if (!childrenOf.has(ppid)) childrenOf.set(ppid, []);
        childrenOf.get(ppid).push(pid);
      } catch {}
    }
    if (!byPid.has(rootPid)) return null;
    const pids = [rootPid, ...bfsDescendants(rootPid, childrenOf)];
    const procs = [];
    let total = 0;
    for (const pid of pids) {
      const r = byPid.get(pid);
      if (!r) continue;
      const mb = r.rss / 1048576;
      total += mb;
      procs.push({ pid, name: r.name, mb: Math.round(mb * 100) / 100 });
    }
    return { totalMB: total, webview2MB: 0, procs, pids };
  } catch {
    return null;
  }
}

// Sum the WorkingSet/RSS of the process tree rooted at rootPid.
// Returns { totalMB, webview2MB, procs, pids } or null when root is gone.
export function sampleDescendantTreeMB(rootPid) {
  if (process.platform === "win32") return windowsTree(rootPid);
  return Promise.resolve(linuxTree(rootPid));
}

// Poll a spawned Windows process until its main window has a non-empty title
// (ADR-0072 liveness contract: MainWindowTitle == productName means the shell
// actually booted, not just that the process exists). Returns ms elapsed, or
// null on timeout. Linux has no window title to poll - callers skip there.
export function waitMainWindowTitle(pid, deadlineMs = 30000) {
  return new Promise((resolve) => {
    const t0 = Date.now();
    const tick = () => {
      const ps =
        `$p=Get-Process -Id ${pid} -ErrorAction SilentlyContinue;` +
        `if($p){$p.MainWindowTitle}`;
      execFile(
        "powershell",
        ["-NoProfile", "-Command", ps],
        { timeout: 5000 },
        (err, stdout) => {
          if (err) return resolve(null);
          const title = (stdout ?? "").trim();
          if (title) return resolve({ ms: Date.now() - t0, title });
          if (Date.now() - t0 > deadlineMs) return resolve(null);
          setTimeout(tick, 40);
        },
      );
    };
    tick();
  });
}
