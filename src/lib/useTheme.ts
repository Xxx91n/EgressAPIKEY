import { useEffect } from "react";
import { useAppStore, type Theme } from "../store/appStore";

/**
 * Apply the current theme to the document root and keep it in sync with OS
 * preference when `theme === "system"`. Tailwind `darkMode: "class"` reads
 * the `dark` class on <html>; ReactFlow 12 reads its own `colorMode` prop
 * (wired in TopologyView from the same store value) so both surfaces track one
 * source of truth.
 *
 * Resolves "system" to light/dark via `matchMedia(prefers-color-scheme)` and
 * re-evaluates when the OS theme flips while running. Re-runs only when the
 * `theme` store value changes, not on every render.
 */
export function useTheme(): void {
  const theme = useAppStore((s) => s.theme);
  useEffect(() => {
    const apply = (t: Theme) => {
      const isDark =
        t === "dark" ||
        (t === "system" &&
          window.matchMedia("(prefers-color-scheme: dark)").matches);
      document.documentElement.classList.toggle("dark", isDark);
    };
    apply(theme);
    if (theme !== "system") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const handler = () => apply("system");
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  }, [theme]);
}
