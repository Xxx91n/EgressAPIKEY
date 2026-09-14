// Cross-platform per-PID memory/CPU sampler.
//   linux:   /proc/<pid>/status VmRSS + /proc/<pid>/stat jiffy deltas
//            (pidstat -r -u used as cross-check when sysstat is installed)
//   windows: Get-Process WorkingSet64 + CPU seconds deltas
// Units reported: rssBytes + cpuCores (fraction of one core) + cpuPct
// (percent of ONE core, matching the D-004 "<1% idle" wording).

import { execFile } from "node:child_process";
import fs from "node:fs";
import { summarize, roundStats } from "./stats.mjs";

const CLK_TCK = 100; // Linux USER_HZ; virtualized runners are ~always 100.

function readLinuxRss(pid) {
  const s = fs.readFileSync(`/proc/${pid}/status`, "utf8");
  const m = /VmRSS:\s*(\d+)\s*kB/.exec(s);
  return m ? Number(m[1]) * 1024 : null;
}

function readLinuxJiffies(pid) {
  const s = fs.readFileSync(`/proc/${pid}/stat`, "utf8");
  // fields 14/15 = utime/stime; comm may contain spaces/parens -> split after last ')'
  const after = s.slice(s.lastIndexOf(")") + 2).split(/\s+/);
  const utime = Number(after[11]); // field 14 -> index 11 after the split
  const stime = Number(after[12]); // field 15
  return utime + stime;
}

function readWindowsProc(pid) {
  return new Promise((resolve) => {
    execFile(
      "powershell",
      [
        "-NoProfile",
        "-Command",
        `$p=Get-Process -Id ${pid} -ErrorAction SilentlyContinue;` +
          `if($p){($p.WorkingSet64,$p.CPU) -join ','}`,
      ],
      { timeout: 5000 },
      (err, stdout) => {
        if (err || !stdout) return resolve(null);
        const m = /(\d+)[,\s]+([\d.]+)/.exec(stdout.trim());
        if (!m) return resolve(null);
        resolve({ rssBytes: Number(m[1]), cpuSeconds: Number(m[2]) });
      },
    );
  });
}

export class Sampler {
  constructor(pid, intervalMs = 1000) {
    this.pid = pid;
    this.intervalMs = intervalMs;
    this.samples = []; // {t, rssBytes, cpuCores}
    this.timer = null;
    this._prevCpu = null;
    this._prevT = null;
    this.errors = 0;
  }

  async sampleOnce() {
    const t = Date.now();
    try {
      if (process.platform === "win32") {
        const r = await readWindowsProc(this.pid);
        if (!r) return;
        let cpuCores = 0;
        if (this._prevCpu != null) {
          cpuCores = Math.max(0, (r.cpuSeconds - this._prevCpu) / ((t - this._prevT) / 1000));
        }
        this._prevCpu = r.cpuSeconds;
        this._prevT = t;
        this.samples.push({ t, rssBytes: r.rssBytes, cpuCores });
      } else {
        const rssBytes = readLinuxRss(this.pid);
        const jif = readLinuxJiffies(this.pid);
        let cpuCores = 0;
        if (this._prevCpu != null) {
          cpuCores = Math.max(0, (jif - this._prevCpu) / CLK_TCK / ((t - this._prevT) / 1000));
        }
        this._prevCpu = jif;
        this._prevT = t;
        if (rssBytes != null) this.samples.push({ t, rssBytes, cpuCores });
      }
    } catch {
      this.errors++;
    }
  }

  start() {
    this.timer = setInterval(() => this.sampleOnce(), this.intervalMs);
    this.timer.unref?.();
    return this;
  }

  async stop() {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
    await this.sampleOnce(); // flush a final sample
    return this.stats();
  }

  stats() {
    const rss = this.samples.map((s) => s.rssBytes / 1048576);
    const cpu = this.samples.map((s) => s.cpuCores);
    return {
      n: this.samples.length,
      errors: this.errors,
      rssMB: roundStats(summarize(rss)),
      cpuCores: roundStats(summarize(cpu)),
      cpuPctOfOneCore: roundStats(summarize(cpu.map((c) => c * 100))),
    };
  }
}

// Sum WorkingSet64 (MB) over a set of process-name globs on Windows
// (whole-app steady-state: EgressAPIKEY + resin* + msedgewebview2*).
// opts.minStartEpochMs filters out pre-existing instances (e.g. a user's own
// app instance already running on the dev host): only procs started at/after
// that instant count. Returns { totalMB, perName } or null.
export function sampleProcessTreeMB(namePatterns, opts = {}) {
  return new Promise((resolve) => {
    if (process.platform === "win32") {
      const cond = namePatterns.map((p) => `$_.ProcessName -like '${p}'`).join(" -or ");
      const minT = Number(opts.minStartEpochMs || 0);
      const ps =
        `$min=[DateTimeOffset]::FromUnixTimeMilliseconds(${minT}).LocalDateTime;` +
        `$out=@{};Get-Process | Where-Object { (${cond}) -and (${minT} -le 0 -or $_.StartTime -ge $min) }` +
        ` | ForEach-Object { $k=$_.ProcessName; $v=0.0; if($out.ContainsKey($k)){$v=$out[$k]}; $out[$k]=[math]::Round($v + $_.WorkingSet64/1MB,2) };` +
        `if($out.Count -eq 0){'{}'}else{$out | ConvertTo-Json -Compress}`;
      execFile("powershell", ["-NoProfile", "-Command", ps], { timeout: 10000 }, (err, stdout) => {
        if (err || !stdout.trim()) return resolve(null);
        try {
          const perName = JSON.parse(stdout.trim());
          const totalMB = Object.values(perName).reduce((a, v) => a + Number(v), 0);
          resolve({ totalMB, perName });
        } catch {
          resolve(null);
        }
      });
    } else {
      let total = 0;
      try {
        for (const pid of fs.readdirSync("/proc").filter((d) => /^\d+$/.test(d))) {
          try {
            const comm = fs.readFileSync(`/proc/${pid}/comm`, "utf8").trim();
            if (namePatterns.some((p) => new RegExp(p.replace(/\*/g, ".*"), "i").test(comm))) {
              const s = fs.readFileSync(`/proc/${pid}/status`, "utf8");
              const m = /VmRSS:\s*(\d+)\s*kB/.exec(s);
              if (m) total += Number(m[1]) / 1024;
            }
          } catch {}
        }
        resolve(total ? { totalMB: total, perName: null } : null);
      } catch {
        resolve(null);
      }
    }
  });
}