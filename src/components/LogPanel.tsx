import { useTranslation } from "react-i18next";
import { useEffect, useRef } from "react";
import { useLogStore } from "../store/logStore";

const LEVEL_CLASS: Record<number, string> = {
  1: "text-zinc-400 dark:text-zinc-500", // trace
  2: "text-zinc-500 dark:text-zinc-400", // debug
  3: "text-zinc-700 dark:text-zinc-300", // info
  4: "text-amber-600 dark:text-amber-400", // warn
  5: "text-red-600 dark:text-red-400", // error
};
const LEVEL_TAG: Record<number, string> = { 1: "trc", 2: "dbg", 3: "inf", 4: "wrn", 5: "err" };

export function LogPanel() {
  const { t } = useTranslation();
  const open = useLogStore((s) => s.open);
  const setOpen = useLogStore((s) => s.setOpen);
  const clear = useLogStore((s) => s.clear);
  const logs = useLogStore((s) => s.logs);
  const endRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to newest only while open.
  useEffect(() => {
    if (open) endRef.current?.scrollIntoView({ block: "end" });
  }, [logs, open]);

  if (!open) {
    return (
      <button
        onClick={() => setOpen(true)}
        className="fixed bottom-2 right-2 px-2 py-1 text-xs rounded border border-zinc-300 dark:border-zinc-700 bg-white/80 dark:bg-zinc-900/80 backdrop-blur hover:bg-zinc-100 dark:hover:bg-zinc-800"
        title={t("log.title")}
      >
        {t("log.title")} ({logs.length})
      </button>
    );
  }
  return (
    <section
      className="fixed bottom-0 inset-x-0 h-48 flex flex-col border-t border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-950"
      aria-label={t("log.title")}
    >
      <header className="flex items-center justify-between px-3 py-1 border-b border-zinc-200 dark:border-zinc-800 text-xs">
        <span className="font-medium">{t("log.title")}</span>
        <div className="flex gap-2">
          <button onClick={clear} className="hover:underline">{t("log.clear")}</button>
          <button onClick={() => setOpen(false)} className="hover:underline">{t("log.collapse")}</button>
        </div>
      </header>
      <div className="flex-1 overflow-auto font-mono text-xs px-3 py-1 space-y-0.5">
        {logs.length === 0 ? (
          <p className="text-zinc-400 dark:text-zinc-600 italic">{t("log.empty")}</p>
        ) : (
          logs.map((l, i) => (
            <div key={i} className={LEVEL_CLASS[l.level] ?? "text-zinc-700 dark:text-zinc-300"}>
              <span className="opacity-60">[{LEVEL_TAG[l.level] ?? "log"}]</span> {l.message}
            </div>
          ))
        )}
        <div ref={endRef} />
      </div>
    </section>
  );
}
