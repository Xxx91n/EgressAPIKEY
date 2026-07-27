import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en/common.json";
import zh from "./locales/zh/common.json";
import ja from "./locales/ja/common.json";
import es from "./locales/es/common.json";
import fr from "./locales/fr/common.json";

/**
 * Decoupled i18n bootstrap. The catalog is split per-locale under
 * src/locales/<locale>/*.json. Adding user-visible text means adding the key
 * to EVERY locale (en, zh, ja, es, fr) in the same commit; `pnpm i18n:check`
 * fails the build on any missing key per the canonical en catalog.
 */
void i18n
  .use(initReactI18next)
  .init({
    resources: {
      en: { common: en },
      zh: { common: zh },
      ja: { common: ja },
      es: { common: es },
      fr: { common: fr },
    },
    lng: "en",
    fallbackLng: "en",
    defaultNS: "common",
    interpolation: {
      escapeValue: false,
    },
  });

export default i18n;
