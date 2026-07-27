import { create } from "zustand";

/// One log entry forwarded from the Rust tracing WebviewLayer
/// (tauri-plugin-tracing `tracing://log` event) or from a JS console hook.
export interface LogEntry {
  level: number; // 1=trace 2=debug 3=info 4=warn 5=error (plugin's LogLevel)
  message: string;
  ts: number; // Date.now()
}

const CAP = 500;

interface LogState {
  logs: LogEntry[];
  open: boolean;
  append: (e: LogEntry) => void;
  setOpen: (o: boolean) => void;
  clear: () => void;
}

/**
 * Bounded log buffer for the desktop log panel. Capped at `CAP` entries so a
 * chatty kernel thread can never grow memory unbounded in the webview; new
 * entries drop the oldest (FIFO ring). Selectors should read `logs` slices,
 * never the whole array, while the panel is collapsed.
 */
export const useLogStore = create<LogState>((set) => ({
  logs: [],
  open: false,
  append: (e) =>
    set((s) => ({
      logs: s.logs.length >= CAP ? [...s.logs.slice(s.logs.length - CAP + 1), e] : [...s.logs, e],
    })),
  setOpen: (open) => set({ open }),
  clear: () => set({ logs: [] }),
}));
