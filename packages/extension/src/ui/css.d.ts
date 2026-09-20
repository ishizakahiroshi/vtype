// `import css from "./styles.css?raw"` yields the stylesheet text. Vite/vitest handle `?raw`
// natively; build.mjs maps it to esbuild's text loader.
declare module "*.css?raw" {
  const css: string;
  export default css;
}

// Tests read a source file's own text this way (e.g. to prove an API is never called).
declare module "*.ts?raw" {
  const source: string;
  export default source;
}
