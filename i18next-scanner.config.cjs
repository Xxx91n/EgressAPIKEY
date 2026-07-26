const path = require("node:path");

module.exports = {
  input: ["src/**/*.{ts,tsx}"],
  output: "src/locales",
  lngs: ["en", "zh"],
  defaultLng: "en",
  defaultNs: "common",
  ns: ["common"],
  resource: {
    loadPath: "{{lng}}/{{ns}}.json",
    savePath: "{{lng}}/{{ns}}.json",
    jsonIndent: 2,
    lineEnding: "\n",
  },
  func: {
    list: ["t", "i18n.t"],
    extensions: [".ts", ".tsx"],
  },
  keySeparator: ".",
  nsSeparator: false,
};
module.exports.default = module.exports;
