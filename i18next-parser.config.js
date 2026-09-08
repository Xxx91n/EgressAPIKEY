// i18next-parser config — extracts t() keys from src into src/locales.
// Replaces the deprecated i18next-scanner. Run: pnpm i18n:scan
// `pnpm i18n:check` then fails the build if any base locale is missing a key.
/** @type {import('i18next-parser').Config} */
module.exports = {
  input: ["src/**/*.{ts,tsx}"],
  output: "src/locales/$LOCALE/$NAMESPACE.json",
  locales: ["en", "zh", "ja", "es", "fr", "de", "ko", "ru", "pt", "ar", "it", "nl", "pl", "tr", "vi", "th", "id", "hi"],
  defaultLocale: "en",
  defaultNamespace: "common",
  namespace: "common",
  keySeparator: ".",
  nsSeparator: false,
  // Preserve existing translations; only append missing keys as empty strings.
  // The check script enforces that no empty/missing key ships.
  sort: true,
  jsonIndent: 2,
  lineEnding: "\n",
  createOldCatalogs: false,
  // react-i18next patterns: t('key'), t(`key`), i18n.t('key'), <Trans i18nKey="key" />
  lexers: {
    tsx: [
      { lexer: "JsxLexer", functions: ["t", "i18n.t"], attr: "i18nKey" },
    ],
    ts: [{ lexer: "JavascriptLexer", functions: ["t", "i18n.t"] }],
    default: ["JavascriptLexer"],
  },
};
