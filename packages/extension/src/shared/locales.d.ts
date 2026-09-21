/**
 * `vtype:locales` is a virtual module: the build (esbuild) and the tests (vite) both replace it
 * with the contents of `_locales/<code>/messages.json`, Chrome's own format, with the
 * `{ message }` wrapper removed. See `locales.mjs`.
 */
declare module "vtype:locales" {
  const messages: Record<string, Record<string, string>>;
  export default messages;
}
