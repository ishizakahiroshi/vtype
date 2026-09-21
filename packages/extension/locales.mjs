// The one place that reads `_locales/`, shared by the build (esbuild) and the tests (vite).
//
// `_locales/<code>/messages.json` is Chrome's own format and the single source of every
// user-facing string: Chrome itself reads it for the manifest's `__MSG_*` fields (the name and
// the description the store shows), and the bundles read the same files through the virtual
// module `vtype:locales`, so a string exists exactly once.
//
// Adding a language is adding one file: drop `_locales/<code>/messages.json` next to the others
// with the same keys. Nothing here, in the manifest or in the source lists the languages.
//
// The parity check is deliberately a hard error rather than a warning. A missing key would show
// an English sentence inside an otherwise Japanese page, which nobody reports as a bug.

import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/** What `import messages from "vtype:locales"` resolves to. */
export const LOCALES_MODULE_ID = "vtype:locales";

/** The locale every other one is compared against, and the fallback at runtime. */
export const DEFAULT_LOCALE = "en";

const here = dirname(fileURLToPath(import.meta.url));

export const LOCALES_DIR = join(here, "_locales");

/**
 * Every locale as `{ "<code>": { "<key>": "<message>" } }`, with Chrome's `{ message }` wrapper
 * removed. Throws if a locale is unreadable or its keys differ from the default locale's.
 */
export function readLocales(dir = LOCALES_DIR) {
  if (!existsSync(dir)) throw new Error(`_locales is missing: ${dir}`);

  const locales = {};
  for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
    if (!entry.isDirectory()) continue;
    const file = join(dir, entry.name, "messages.json");
    if (!existsSync(file)) throw new Error(`_locales/${entry.name} has no messages.json`);

    let raw;
    try {
      raw = JSON.parse(readFileSync(file, "utf8"));
    } catch (err) {
      throw new Error(`_locales/${entry.name}/messages.json is not valid JSON: ${err.message}`);
    }

    const dict = {};
    for (const [key, value] of Object.entries(raw)) {
      const message = value?.message;
      if (typeof message !== "string" || message.trim() === "") {
        throw new Error(`_locales/${entry.name}/messages.json: "${key}" has no message`);
      }
      dict[key] = message;
    }
    locales[entry.name] = dict;
  }

  const codes = Object.keys(locales);
  if (!codes.includes(DEFAULT_LOCALE)) {
    throw new Error(`_locales has no ${DEFAULT_LOCALE}/ (the default locale must exist)`);
  }

  const expected = Object.keys(locales[DEFAULT_LOCALE]).sort();
  for (const code of codes) {
    if (code === DEFAULT_LOCALE) continue;
    const actual = Object.keys(locales[code]).sort();
    const missing = expected.filter((key) => !actual.includes(key));
    const extra = actual.filter((key) => !expected.includes(key));
    if (missing.length > 0 || extra.length > 0) {
      const parts = [];
      if (missing.length > 0) parts.push(`missing: ${missing.join(", ")}`);
      if (extra.length > 0) parts.push(`not in ${DEFAULT_LOCALE}: ${extra.join(", ")}`);
      throw new Error(`_locales/${code}/messages.json does not match ${DEFAULT_LOCALE} (${parts.join("; ")})`);
    }
  }

  return locales;
}

function moduleSource(dir) {
  return `export default ${JSON.stringify(readLocales(dir))};`;
}

/** esbuild plugin: resolves `vtype:locales` to the dictionaries (used by build.mjs). */
export function esbuildLocales(dir = LOCALES_DIR) {
  return {
    name: "vtype-locales",
    setup(build) {
      build.onResolve({ filter: /^vtype:locales$/ }, () => ({ path: LOCALES_MODULE_ID, namespace: "vtype-locales" }));
      build.onLoad({ filter: /.*/, namespace: "vtype-locales" }, () => ({
        contents: moduleSource(dir),
        loader: "js",
      }));
    },
  };
}

/** Vite plugin: the same module for vitest, which runs the source rather than the bundle. */
export function viteLocales(dir = LOCALES_DIR) {
  const resolved = "\0" + LOCALES_MODULE_ID;
  return {
    name: "vtype-locales",
    resolveId(id) {
      return id === LOCALES_MODULE_ID ? resolved : null;
    },
    load(id) {
      return id === resolved ? moduleSource(dir) : null;
    },
  };
}
