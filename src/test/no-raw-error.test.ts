import { describe, it, expect } from "vitest";
import * as fs from "fs";
import * as path from "path";

/// Guard against raw error objects reaching user-facing display
/// functions. Scans all .ts/.tsx source files for catch blocks that pass
/// the caught variable directly to a display function WITHOUT going through
/// translateError(). This prevents "[object Object]" regression.
///
/// Allowed patterns:
///   - translateError(e, t)  -- safe
///   - e instanceof Error ? e.message : String(e)  -- safe narrowing
///   - console.warn(e) / console.log(e)  -- debug, not user-facing
///   - setXxx({ ..., error: e instanceof Error ? e.message : String(e) })  -- safe

const SRC_DIR = path.resolve(__dirname, "..");
const DISPLAY_FNS = [
  "showToast", "setError", "setConfigMsg", "setStrategyMsg",
  "setBackupMsg", "setNetMsg", "setProbeResult", "setHealthResult",
  "setVerifyResult", "setCreateFormError", "setToast",
];

function scanFile(filePath: string): Array<{ line: number; code: string; fn: string }> {
  const src = fs.readFileSync(filePath, "utf8");
  const lines = src.split("\n");
  const issues: Array<{ line: number; code: string; fn: string }> = [];

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    // Match catch (e) or catch (err) etc
    const catchMatch = line.match(/catch\s*\(\s*(\w+)\s*\)\s*\{/);
    if (!catchMatch) continue;
    const varName = catchMatch[1];

    // Scan next ~15 lines for the catch body
    const bodyEnd = Math.min(i + 20, lines.length);
    for (let j = i + 1; j < bodyEnd; j++) {
      const bodyLine = lines[j];
      // Check if body line contains a display function call with raw var
      for (const fn of DISPLAY_FNS) {
        // Pattern: fn("err", varName) or fn(varName) where varName is raw
        // But NOT translateError(varName, ...)
        const rawPattern = new RegExp(fn + "[^ ]*\\b" + varName + "\\b");
        if (rawPattern.test(bodyLine)) {
          // Check it's NOT wrapped in translateError
          if (!bodyLine.includes("translateError")) {
            // Check it's NOT instanceof narrowing (safe)
            if (!bodyLine.includes("instanceof Error")) {
              issues.push({ line: j + 1, code: bodyLine.trim(), fn });
            }
          }
        }
      }
      // Stop at catch body close (simplified -- } at same indent as catch)
      if (lines[j].trim() === "}" && j > i + 1) break;
    }
  }
  return issues;
}

function collectFiles(dir: string, ext: string[]): string[] {
  const out: string[] = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory() && entry.name !== "node_modules" && entry.name !== "dist") {
      out.push(...collectFiles(full, ext));
    } else if (entry.isFile() && ext.some(e => entry.name.endsWith(e))) {
      // Skip test files themselves and .test.ts(x)
      if (!entry.name.includes(".test.")) {
        out.push(full);
      }
    }
  }
  return out;
}

describe("T20: no raw error in display functions (catch-block lint guard)", () => {
  it("all catch blocks pass errors through translateError or instanceof narrowing", () => {
    const files = collectFiles(SRC_DIR, [".ts", ".tsx"]);
    const allIssues: Array<{ file: string; line: number; code: string; fn: string }> = [];
    for (const f of files) {
      const issues = scanFile(f);
      for (const iss of issues) {
        allIssues.push({ file: path.basename(f), ...iss });
      }
    }
    if (allIssues.length > 0) {
      const msg = allIssues.map(i =>
        "  " + i.file + ":" + i.line + " -- " + i.fn + " got raw error: " + i.code
      ).join("\n");
      expect.fail("Found " + allIssues.length + " catch block(s) passing raw error to display function without translateError.\n" + msg);
    }
    expect(allIssues).toHaveLength(0);
  });
});
