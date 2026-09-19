import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "happy-dom",
    include: ["tests/**/*.test.ts"],
    // vitest replaces CSS imports with "" unless the file is included here, which would make
    // `import css from "../ui/styles.css?raw"` empty and every style test vacuous.
    css: { include: [/src[\\/]ui[\\/].*\.css/] },
  },
});
