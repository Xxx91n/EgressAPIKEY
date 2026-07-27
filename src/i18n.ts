import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import resourcesToBackend from "i18next-resources-to-backend";

/**
 * Decoupled i18n bootstrap with lazy per-locale chunk loading.
 *
 * Locales live under `src/locales/<locale>/<namespace>.json` and are loaded
 * on demand via `i18next-resources-to-backend` — Vite statically analyses the
 * dynamic `import(`./locales/${lng}/${ns}.json`)` template and emits one
 * chunk per (locale, namespace), so an English user never downloads the
 * Japanese bundle. `partialBundledLanguages: true` lets i18next mix any
 * bundled resources with backend-loaded ones; we keep zero bundled resources
 * and fetch everything lazily, with `fallbackLng: "en"` as the safety net.
 *
 * `react.useSuspense: false` makes a missing locale recoverable (renders the
 * key, then re-renders when the chunk arrives) instead of throwing into a
 * Suspense boundary — important for the Tauri webview which has no router-
 * level Suspense wrapper above the views.
 *
 * Adding a user-visible string means adding the key to EVERY base locale in
 * the same commit; `pnpm i18n:check` fails the build on any missing key per
 * the canonical `en` catalog. Base locales: en, zh, ja, es, fr, de, ko, ru,
 * pt, ar.
 */
void i18n
  .use(
    resourcesToBackend((lng: string, ns: string) =>
      import(`./locales/${lng}/${ns}.json`),
    ),
  )
  .use(initReactI18next)
  .init({
    lng: "en",
    fallbackLng: "en",
    defaultNS: "common",
    ns: ["common"],
    partialBundledLanguages: true,
    interpolation: {
      escapeValue: false, // React already escapes
    },
    react: {
      useSuspense: false,
    },
  });

export default i18n;
