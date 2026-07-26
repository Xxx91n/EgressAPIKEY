import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en/common.json";
import zh from "./locales/zh/common.json";

/**
 * Decoupled i18n bootstrap. The catalog is split per-locale under
 * src/locales/<locale>/*.json. Adding user-visible text means adding the key
 * to BOTH base locales (en, zh) in the same commit; `pnpm i18n:check` fails
 * the build on missing keys.
 */
void i18n
  .use(initReactI18next)
  .init({
    resources: {
      en: { common: en },
      zh: { common: zh },
    },
    lng: "en",
    fallbackLng: "en",
    defaultNS: "common",
    interpolation: {
      escapeValue: false,
    },
  });

export default i18n;
