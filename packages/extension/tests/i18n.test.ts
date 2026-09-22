// The dictionaries are the one place a string lives, so these are the checks that keep a new
// language honest. Adding `_locales/<code>/messages.json` is meant to be the whole job: every
// test here works off what `_locales/` holds, and none of them names ja or en except where the
// default locale is the subject.

import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import messages from "vtype:locales";
import { DEFAULT_LOCALE, LOCALES, resolveLocale, translate } from "../src/shared/i18n";

const extensionRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const localesDir = join(extensionRoot, "_locales");

const manifest = JSON.parse(readFileSync(join(extensionRoot, "manifest.json"), "utf8")) as Record<string, unknown>;

/** Source files that hold user-facing text, so the keys they use can be checked against the dictionaries. */
const sources = [
  "src/content/controller.ts",
  "src/options/options.ts",
  "src/permission/permission.ts",
  "src/speech/speech.ts",
  "src/ui/toolbar.ts",
].map((path) => readFileSync(join(extensionRoot, path), "utf8"));

// Keys that start with `native_` belong to the desktop app (packages/native), which compiles them
// in with build.rs. They count as used when its Rust source asks for them.
const nativeSrc = join(extensionRoot, "..", "native", "src");
const nativeSources = readdirSync(nativeSrc, { recursive: true, encoding: "utf8" })
  .filter((path) => path.endsWith(".rs"))
  .map((path) => readFileSync(join(nativeSrc, path), "utf8"));
const NATIVE_KEY = /\bt(?:_with)?\(\s*"(native_[A-Za-z0-9_]+)"/g;

const defaultKeys = Object.keys(messages[DEFAULT_LOCALE] ?? {});

describe("locales", () => {
  it("has at least the default locale, and every folder was loaded", () => {
    const folders = readdirSync(localesDir, { withFileTypes: true })
      .filter((entry) => entry.isDirectory())
      .map((entry) => entry.name)
      .sort();
    expect(folders).toEqual([...LOCALES]);
    expect(folders).toContain(DEFAULT_LOCALE);
  });

  it("gives every locale the same keys as the default one", () => {
    for (const code of LOCALES) {
      expect(Object.keys(messages[code]!).sort(), `_locales/${code}`).toEqual([...defaultKeys].sort());
    }
  });

  it("has no empty message anywhere", () => {
    for (const code of LOCALES) {
      for (const [key, message] of Object.entries(messages[code]!)) {
        expect(message.trim(), `${code}/${key}`).not.toBe("");
      }
    }
  });

  it("uses the same placeholders in every locale", () => {
    const placeholders = (text: string): string[] => (text.match(/\{\w+\}/g) ?? []).sort();
    for (const key of defaultKeys) {
      const expected = placeholders(messages[DEFAULT_LOCALE]![key]!);
      for (const code of LOCALES) {
        expect(placeholders(messages[code]![key]!), `${code}/${key}`).toEqual(expected);
      }
    }
  });

  it("keeps the store's summary within the 132 characters Chrome allows", () => {
    for (const code of LOCALES) {
      expect(messages[code]!.appDescription!.length, `${code}/appDescription`).toBeLessThanOrEqual(132);
    }
  });
});

describe("the manifest and the dictionaries agree", () => {
  it("declares the default locale the dictionaries are compared against", () => {
    expect(manifest.default_locale).toBe(DEFAULT_LOCALE);
  });

  it("points every __MSG_*__ field at a message that exists", () => {
    const fields = [manifest.name, manifest.description, (manifest.action as { default_title?: string })?.default_title];
    const used = fields
      .filter((value): value is string => typeof value === "string")
      .map((value) => value.match(/^__MSG_(\w+)__$/)?.[1])
      .filter((key): key is string => key !== undefined);

    expect(used).toContain("appName");
    expect(used).toContain("appDescription");
    for (const key of used) expect(defaultKeys, `manifest uses ${key}`).toContain(key);
  });
});

describe("the keys the code asks for", () => {
  it("all exist in the dictionaries", () => {
    const used = new Set<string>();
    for (const source of sources) {
      for (const match of source.matchAll(/\bt\(\s*"([A-Za-z0-9_]+)"/g)) used.add(match[1]!);
      for (const match of source.matchAll(/\bt\(\s*\w+ === 1 \? "([A-Za-z0-9_]+)" : "([A-Za-z0-9_]+)"/g)) {
        used.add(match[1]!);
        used.add(match[2]!);
      }
    }
    // A typo in a key is invisible at runtime (the key itself is rendered), so it has to fail here.
    expect(used.size).toBeGreaterThan(50);
    for (const key of used) expect(defaultKeys, `used in the source: ${key}`).toContain(key);
  });

  it("has every key the desktop app asks for", () => {
    const used = new Set<string>();
    for (const source of nativeSources) for (const match of source.matchAll(NATIVE_KEY)) used.add(match[1]!);
    expect(used.size).toBeGreaterThan(5);
    for (const key of used) expect(defaultKeys, `used in packages/native: ${key}`).toContain(key);
  });

  it("leaves no message in the dictionaries that nothing uses", () => {
    const text = sources.join("\n") + JSON.stringify(manifest);
    const nativeUsed = new Set<string>();
    for (const source of nativeSources) for (const match of source.matchAll(NATIVE_KEY)) nativeUsed.add(match[1]!);
    const unused = defaultKeys.filter((key) =>
      key.startsWith("native_")
        ? !nativeUsed.has(key)
        : !text.includes(`"${key}"`) && !text.includes(`__MSG_${key}__`),
    );
    expect(unused).toEqual([]);
  });
});

describe("resolveLocale", () => {
  it("takes the language alone from a regional tag", () => {
    for (const code of LOCALES) {
      expect(resolveLocale(code)).toBe(code);
      expect(resolveLocale(`${code}-ZZ`)).toBe(code);
      expect(resolveLocale(code.toUpperCase())).toBe(code);
    }
  });

  it("falls back to the default locale for anything it does not have", () => {
    expect(resolveLocale(undefined)).toBe(DEFAULT_LOCALE);
    expect(resolveLocale("")).toBe(DEFAULT_LOCALE);
    expect(resolveLocale("  ")).toBe(DEFAULT_LOCALE);
    expect(resolveLocale("zz-ZZ")).toBe(DEFAULT_LOCALE);
  });
});

describe("translate", () => {
  it("fills {name} placeholders from the vars", () => {
    expect(translate("panelOtherError", DEFAULT_LOCALE, { code: "network" })).toContain("network");
    expect(translate("optionsSitesAdded", DEFAULT_LOCALE, { site: "https://example.com" })).toContain(
      "https://example.com",
    );
  });

  it("leaves a placeholder alone when nothing was passed for it", () => {
    expect(translate("panelOtherError", DEFAULT_LOCALE)).toContain("{code}");
    expect(translate("panelOtherError", DEFAULT_LOCALE, {})).toContain("{code}");
  });

  it("returns the key itself when there is no such message", () => {
    expect(translate("thereIsNoSuchKey", DEFAULT_LOCALE)).toBe("thereIsNoSuchKey");
  });

  it("reads the requested locale, not the default one", () => {
    for (const code of LOCALES) {
      expect(translate("micSend", code)).toBe(messages[code]!.micSend);
    }
  });
});
