import { defineConfig } from "vitest/config";

// The same `vtype:locales` module the build inlines, so the tests read the real
// `_locales/<code>/messages.json` rather than a copy of the strings.
import { viteLocales } from "./locales.mjs";

export default defineConfig({
  plugins: [viteLocales()],
  test: {
    environment: "happy-dom",
    include: ["tests/**/*.test.ts"],
    // vitest replaces CSS imports with "" unless the file is included here, which would make
    // `import css from "../ui/styles.css?raw"` empty and every style test vacuous.
    css: { include: [/src[\\/]ui[\\/].*\.css/] },
  },
});
