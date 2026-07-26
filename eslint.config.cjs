/** @type {import('eslint').Linter.Config} */
module.exports = [
  {
    files: ["src/**/*.{ts,tsx}", "e2e/**/*.ts", "scripts/**/*.cjs"],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "module",
      parserOptions: { ecmaFeatures: { jsx: true } },
    },
    rules: {},
  },
  {
    ignores: ["dist/**", "node_modules/**", "src-tauri/target/**", "release/**"],
  },
];
