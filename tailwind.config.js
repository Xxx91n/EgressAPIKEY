/** @type {import('tailwindcss').Config} */
// darkMode: 'class' so the theme store toggles <html class="dark"> manually
// (Re1). ReactFlow 12 reads colorMode separately; Tailwind dark: variants
// follow the html class so both surfaces stay in sync.
module.exports = {
  darkMode: "class",
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {},
  },
  plugins: [],
};
module.exports.default = module.exports;
