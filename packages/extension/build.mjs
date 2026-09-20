// Builds the loadable extension into dist/ (gitignored): load packages/extension/dist unpacked.
//
// Every script is bundled into one self-contained IIFE file (vtype-core inlined): MV3 content
// scripts are classic scripts, and a classic service worker / page script cannot import either.
// The build fails if any output keeps an import/export statement, and if the manifest or an
// HTML page points at a file the build did not produce.
//
// vtype-core is consumed from its built dist/ (workspace package): run
// `pnpm -F vtype-core build` first on a fresh clone.

import { build } from "esbuild";
import { copyFile, mkdir, readFile, readdir, rm } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const dist = join(here, "dist");

await rm(dist, { recursive: true, force: true });
await mkdir(dist, { recursive: true });

/** `import css from "./x.css?raw"` -> the file's text (same meaning as Vite's `?raw`, used by vitest). */
const rawImports = {
  name: "raw-imports",
  setup(b) {
    b.onResolve({ filter: /\?raw$/ }, (args) => ({
      path: join(args.resolveDir, args.path.slice(0, -"?raw".length)),
      namespace: "raw",
    }));
    b.onLoad({ filter: /.*/, namespace: "raw" }, async (args) => ({
      contents: await readFile(args.path, "utf8"),
      loader: "text",
    }));
  },
};

const scripts = [
  ["src/content/index.ts", "content.js"],
  ["src/background/index.ts", "background.js"],
  ["src/offscreen/offscreen.ts", "offscreen.js"],
  ["src/permission/permission.ts", "permission.js"],
  ["src/options/options.ts", "options.js"],
];

for (const [entry, out] of scripts) {
  await build({
    plugins: [rawImports],
    entryPoints: [join(here, entry)],
    outfile: join(dist, out),
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
}

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
].filter(Boolean);
for (const page of ["offscreen.html", "permission.html", "options.html"]) {
  const html = await readFile(join(dist, page), "utf8");
  for (const m of html.matchAll(/<script[^>]*\bsrc="([^"]+)"/g)) referenced.push(m[1]);
}
for (const file of referenced) {
  await readFile(join(dist, file)); // throws if missing
}
if (manifest.background?.type === "module") throw new Error("background must be a classic service worker bundle");

console.log(`built ${dist}: ${(await readdir(dist)).sort().join(", ")}`);
