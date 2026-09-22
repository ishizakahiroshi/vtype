// Builds the loadable extension into dist/ (gitignored): load packages/extension/dist unpacked.
//
// Every script is bundled into one self-contained IIFE file (vtype-core inlined): MV3 content
// scripts are classic scripts, and a classic service worker / page script cannot import either.
// The build fails if any output keeps an import/export statement, and if the manifest or an
// HTML page points at a file the build did not produce.
//
// vtype-core is consumed from its built dist/ (workspace package): run
// `pnpm -F vtype-core build` first on a fresh clone.
//
// User-facing text comes from `_locales/<code>/messages.json` (Chrome's own format). The same
// files are copied into dist/ for Chrome (the manifest's `__MSG_*` fields) and inlined into the
// bundles through the virtual module `vtype:locales` (locales.mjs), so a string exists once and
// adding a language is adding one file.

import { build } from "esbuild";
import { copyFile, mkdir, readFile, readdir, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { DEFAULT_LOCALE, esbuildLocales, readLocales } from "./locales.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const dist = join(here, "dist");

await rm(dist, { recursive: true, force: true });
await mkdir(dist, { recursive: true });

/** `import css from "./x.css?raw"` -> the file's text (same meaning as Vite's `?raw`, used by vitest). */
const rawImports = {
  name: "raw-imports",
  setup(b) {
    // esbuild prints `// raw:<path>` above the module in the unminified bundle, so the path it is
    // given must be relative: an absolute one ships the builder's directory to the store.
    b.onResolve({ filter: /\?raw$/ }, (args) => {
      const file = join(args.resolveDir, args.path.slice(0, -"?raw".length));
      return { path: relative(here, file).split(sep).join("/"), namespace: "raw", pluginData: { file } };
    });
    b.onLoad({ filter: /.*/, namespace: "raw" }, async (args) => ({
      contents: await readFile(args.pluginData.file, "utf8"),
      loader: "text",
    }));
  },
};

/**
 * kuromoji (kana mode's kanji reading, src/offscreen/reading.ts) in a browser bundle:
 * - its dictionary loader is replaced by src/offscreen/packaged-dictionary-loader.cjs, which can
 *   only read files inside the extension (kuromoji's own browser loader uses XMLHttpRequest on
 *   any URL, which validate-extension.ps1 refuses to ship);
 * - the one Node module it needs, `path`, is given a `join` that is enough for "dict/" + name.
 */
const kuromojiWith = (loader) => ({
  name: "kuromoji-for-browser",
  setup(b) {
    b.onResolve({ filter: /NodeDictionaryLoader(\.js)?$/ }, () => ({ path: join(here, loader) }));
    b.onResolve({ filter: /^path$/ }, () => ({ path: "path-join", namespace: "shim" }));
    b.onLoad({ filter: /.*/, namespace: "shim" }, () => ({
      contents: `module.exports = { join: function () { return Array.prototype.join.call(arguments, "/").replace(/\\/+/g, "/"); } };`,
      loader: "js",
    }));
  },
});
const kuromojiForBrowser = kuromojiWith("src/offscreen/packaged-dictionary-loader.cjs");

const kuromojiDir = dirname(createRequire(import.meta.url).resolve("kuromoji/package.json"));

// Throws on a locale whose keys differ from the default one's: a missing key would show an
// English sentence inside an otherwise translated page, which nobody reports as a bug.
const locales = readLocales();

const scripts = [
  ["src/content/index.ts", "content.js"],
  ["src/background/index.ts", "background.js"],
  ["src/offscreen/offscreen.ts", "offscreen.js"],
  ["src/permission/permission.ts", "permission.js"],
  ["src/options/options.ts", "options.js"],
];

const bundle = (entry, outfile, kuromoji) =>
  build({
    plugins: [rawImports, esbuildLocales(), kuromoji],
    entryPoints: [join(here, entry)],
    outfile,
    bundle: true,
    format: "iife",
    platform: "browser",
    target: "chrome116",
    // Kept readable: store reviewers read the shipped code.
    minify: false,
    sourcemap: false,
    legalComments: "none",
    logLevel: "info",
  });

for (const [entry, out] of scripts) await bundle(entry, join(dist, out), kuromojiForBrowser);

// The extension's icons, baked from the one SVG source (assets/icon.svg) by the icon pipeline.
// Chrome's own names are used in dist/ so the manifest reads like any other extension's.
const icons = [
  ["favicon-16.png", "icon16.png"],
  ["favicon-32.png", "icon32.png"],
  ["favicon-48.png", "icon48.png"],
  ["icon-128.png", "icon128.png"],
];
await mkdir(join(dist, "icons"), { recursive: true });
for (const [from, to] of icons) {
  await copyFile(join(here, "../../assets/icons", from), join(dist, "icons", to));
}

// Chrome reads these itself for the manifest's `__MSG_*` fields.
for (const code of Object.keys(locales)) {
  await mkdir(join(dist, "_locales", code), { recursive: true });
  await copyFile(join(here, "_locales", code, "messages.json"), join(dist, "_locales", code, "messages.json"));
}

// The IPADIC dictionary for kana mode, with the licenses it ships under (Apache-2.0 for
// kuromoji; NOTICE.md carries the dictionary's own terms, which require the notice to travel
// with every copy).
const dictFiles = (await readdir(join(kuromojiDir, "dict"))).filter((name) => name.endsWith(".dat.gz"));
if (dictFiles.length === 0) throw new Error(`no dictionary files in ${join(kuromojiDir, "dict")}`);
async function copyDictionary(to) {
  await mkdir(join(to, "dict"), { recursive: true });
  for (const name of dictFiles) await copyFile(join(kuromojiDir, "dict", name), join(to, "dict", name));
  await copyFile(join(kuromojiDir, "LICENSE-2.0.txt"), join(to, "dict", "LICENSE-kuromoji.txt"));
  await copyFile(join(kuromojiDir, "NOTICE.md"), join(to, "dict", "NOTICE.md"));
}
await copyDictionary(dist);

await copyFile(join(here, "manifest.json"), join(dist, "manifest.json"));
await copyFile(join(here, "src/offscreen/offscreen.html"), join(dist, "offscreen.html"));
await copyFile(join(here, "src/permission/permission.html"), join(dist, "permission.html"));
await copyFile(join(here, "src/options/options.html"), join(dist, "options.html"));

for (const [, out] of scripts) {
  const code = await readFile(join(dist, out), "utf8");
  const moduleSyntax = code.match(/^\s*(import|export)\b.*$/m);
  if (moduleSyntax !== null) {
    throw new Error(`dist/${out} contains module syntax, which a classic script cannot run: ${moduleSyntax[0]}`);
  }
}

// Everything the manifest and the pages reference must exist in dist/.
const manifest = JSON.parse(await readFile(join(dist, "manifest.json"), "utf8"));
const referenced = [
  manifest.background?.service_worker,
  manifest.options_ui?.page,
  ...(manifest.content_scripts ?? []).flatMap((s) => s.js ?? []),
  ...Object.values(manifest.icons ?? {}),
].filter(Boolean);
for (const page of ["offscreen.html", "permission.html", "options.html"]) {
  const html = await readFile(join(dist, page), "utf8");
  for (const m of html.matchAll(/<script[^>]*\bsrc="([^"]+)"/g)) referenced.push(m[1]);
}
for (const file of referenced) {
  await readFile(join(dist, file)); // throws if missing
}
if (manifest.background?.type === "module") throw new Error("background must be a classic service worker bundle");

// The manifest is the single source of the version. package.json carries one too (pnpm wants it
// on a workspace package), so the two are compared here rather than trusted to stay in step.
const pkg = JSON.parse(await readFile(join(here, "package.json"), "utf8"));
if (pkg.version !== manifest.version) {
  throw new Error(`package.json is ${pkg.version} but manifest.json is ${manifest.version}; the manifest is the source`);
}

// Every `__MSG_key__` in the manifest has to exist, or Chrome refuses to load the extension.
if (manifest.default_locale !== DEFAULT_LOCALE) {
  throw new Error(`manifest.default_locale must be "${DEFAULT_LOCALE}" (locales.mjs compares every locale against it)`);
}
for (const [field, value] of Object.entries(manifest)) {
  const name = typeof value === "string" ? value.match(/^__MSG_(\w+)__$/) : null;
  if (name !== null && !(name[1] in locales[DEFAULT_LOCALE])) {
    throw new Error(`manifest.${field} points at a message that does not exist: ${name[1]}`);
  }
}
const actionTitle = manifest.action?.default_title?.match?.(/^__MSG_(\w+)__$/);
if (actionTitle && !(actionTitle[1] in locales[DEFAULT_LOCALE])) {
  throw new Error(`manifest.action.default_title points at a message that does not exist: ${actionTitle[1]}`);
}

console.log(`built ${dist}: ${(await readdir(dist)).sort().join(", ")}`);

// The desktop app's speech page (standalone plan C2): served by packages/native on 127.0.0.1 and
// embedded in its executable by build.rs. Not part of the extension, so it goes to its own
// dist-desktop/ (gitignored) and never into the store zip. It reads the dictionary from its own
// origin (src/speech/desktop-dictionary-loader.cjs) because it has no chrome.runtime.
const distDesktop = join(here, "dist-desktop");
await rm(distDesktop, { recursive: true, force: true });
await mkdir(distDesktop, { recursive: true });
await bundle("src/speech/speech.ts", join(distDesktop, "speech.js"), kuromojiWith("src/speech/desktop-dictionary-loader.cjs"));
await copyFile(join(here, "src/speech/speech.html"), join(distDesktop, "speech.html"));
await copyDictionary(distDesktop);
{
  const code = await readFile(join(distDesktop, "speech.js"), "utf8");
  const moduleSyntax = code.match(/^\s*(import|export)\b.*$/m);
  if (moduleSyntax !== null) throw new Error(`dist-desktop/speech.js contains module syntax: ${moduleSyntax[0]}`);
  if (code.includes("chrome.runtime.getURL")) throw new Error("dist-desktop/speech.js uses the extension's dictionary loader");
  const html = await readFile(join(distDesktop, "speech.html"), "utf8");
  for (const m of html.matchAll(/<script[^>]*\bsrc="([^"]+)"/g)) await readFile(join(distDesktop, m[1]));
}
console.log(`built ${distDesktop}: ${(await readdir(distDesktop)).sort().join(", ")}`);
console.log(`locales: ${Object.keys(locales).join(", ")} (${Object.keys(locales[DEFAULT_LOCALE]).length} messages each)`);
