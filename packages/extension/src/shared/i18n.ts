// User-facing text, in the browser's language.
//
// Every string lives in `_locales/<code>/messages.json` (Chrome's own format) and nowhere else:
// Chrome reads those files for the manifest's `__MSG_*` fields, which is what makes the name and
// the description in the store follow the reader's language, and the bundles read the same files
// through the virtual module `vtype:locales` (see locales.mjs).
//
// Adding a language is adding one file. Nothing here names a language: the supported set is
// whatever `_locales/` holds, and `locales.mjs` refuses to build when a locale's keys differ
// from the default one's.
//
// `chrome.i18n.getMessage` is deliberately not used for these strings. It resolves against
// Chrome's UI language with no way to ask for another one, and the pages and the content script
// take the language as an argument so the tests can render both without a browser.

import messages from "vtype:locales";

/** The locale used when the browser's is not among `_locales/`. Matches manifest.default_locale. */
export const DEFAULT_LOCALE = "en";

/** Locale codes `_locales/` holds, sorted. */
export const LOCALES: readonly string[] = Object.keys(messages).sort();

export type Vars = Record<string, string | number>;

/**
 * The locale to read, from a language tag such as `ja`, `ja-JP` or `pt-BR`.
 *
 * An exact match wins (`pt-BR` -> `pt_BR`, Chrome's folder spelling), then the language alone
 * (`ja-JP` -> `ja`), then the default. Unknown, empty and undefined all mean the default.
 */
export function resolveLocale(language: string | undefined): string {
  const tag = (language ?? "").trim().toLowerCase().replace(/-/g, "_");
  if (tag === "") return DEFAULT_LOCALE;

  for (const code of LOCALES) if (code.toLowerCase() === tag) return code;

  const primary = tag.split("_")[0];
  for (const code of LOCALES) if (code.toLowerCase() === primary) return code;

  return DEFAULT_LOCALE;
}

/**
 * One string. `{name}` in the message is replaced by `vars.name` (the same shape many-ai-cli's
 * dictionaries use). A key that is missing from the resolved locale falls back to the default
 * locale, and then to the key itself: a page with one untranslated line still renders.
 */
export function translate(key: string, language?: string, vars?: Vars): string {
  const locale = resolveLocale(language);
  const message = messages[locale]?.[key] ?? messages[DEFAULT_LOCALE]?.[key] ?? key;
  if (vars === undefined) return message;
  return message.replace(/\{(\w+)\}/g, (whole, name: string) => {
    const value = vars[name];
    return value === undefined ? whole : String(value);
  });
}

export type Translate = (key: string, vars?: Vars) => string;

/** `translate` with the language fixed, for a page or a content script that resolved it once. */
export function translator(language: string | undefined): Translate {
  const locale = resolveLocale(language);
  return (key, vars) => translate(key, locale, vars);
}
