// The desktop app's settings page (standalone plan C5): reads and writes the desktop app's config
// through `api/config` next to the page.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { beforeEach, describe, expect, it } from "vitest";
import { apiUrl, initSettingsPage } from "../src/desktop-settings/settings";
import { translate } from "../src/shared/i18n";

const PAGE = "http://127.0.0.1:47213/t/0123456789abcdef0123456789abcdef/settings";
const API = "http://127.0.0.1:47213/t/0123456789abcdef0123456789abcdef/api/config";

const CONFIG = {
  hotkey: null,
  icon: { visible: true, x: 10, y: 20, hideOnFullscreen: true },
  inject: "auto",
  besideField: { enabled: false, trigger: "focus" },
  extraExtensionIds: [],
  inputMode: "normal",
  replacements: [{ from: "ぶいたいぷ", to: "vtype" }],
};

type Call = { url: string; method: string; body: unknown };

/** A desktop app that stores what it is sent and answers with it (or fails). */
function fakeApp(initial: unknown = CONFIG) {
  let stored: unknown = structuredClone(initial);
  const calls: Call[] = [];
  let failing = false;
  const fetch = async (url: string, init?: { method?: string; body?: string }) => {
    const method = init?.method ?? "GET";
    const body = init?.body === undefined ? undefined : (JSON.parse(init.body) as unknown);
    calls.push({ url, method, body });
    if (failing) return { ok: false, json: async () => ({}) };
    if (method === "POST") stored = body;
    return { ok: true, json: async () => structuredClone(stored) };
  };
  return {
    fetch,
    calls,
    fail: () => {
      failing = true;
    },
  };
}

function mount(): void {
  const html = readFileSync(join(__dirname, "..", "src", "desktop-settings", "settings.html"), "utf8");
  document.body.innerHTML = html.slice(html.indexOf("<body>") + 6, html.indexOf("<script"));
}

const $ = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;
const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(mount);

describe("apiUrl", () => {
  it("sits next to the page, under the same token", () => {
    expect(apiUrl(PAGE)).toBe(API);
    expect(apiUrl(`${PAGE}?x=1#y`)).toBe(API);
  });
});

describe("the desktop settings page", () => {
  it("shows the desktop app's settings", async () => {
    const app = fakeApp();
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en" }).loaded;
    expect(app.calls).toEqual([{ url: API, method: "GET", body: undefined }]);
    expect($<HTMLInputElement>("mode-normal").checked).toBe(true);
    expect($<HTMLTextAreaElement>("repl-text").value).toBe("ぶいたいぷ => vtype");
    expect($<HTMLInputElement>("nc-icon-visible").checked).toBe(true);
    expect($<HTMLSelectElement>("nc-inject").value).toBe("auto");
    expect($("title").textContent).toBe(translate("optionsTitle", "en"));
    expect(document.title).toBe(translate("optionsTitle", "en"));
  });

  it("saves the input mode when a mode is chosen", async () => {
    const app = fakeApp();
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en" }).loaded;
    const kana = $<HTMLInputElement>("mode-kana");
    kana.checked = true;
    kana.dispatchEvent(new Event("change"));
    await flush();
    expect(app.calls[1]).toMatchObject({ url: API, method: "POST", body: { ...CONFIG, inputMode: "kana" } });
    expect($("status").textContent).toBe(translate("optionsSaved", "en"));
  });

  it("saves the replacement table and reports lines it could not read", async () => {
    const app = fakeApp();
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en" }).loaded;
    $<HTMLTextAreaElement>("repl-text").value = "a => b\nno arrow\n  c=>d  ";
    $("repl-save").click();
    await flush();
    expect(app.calls[1]!.body).toMatchObject({ replacements: [{ from: "a", to: "b" }, { from: "c", to: "d" }] });
    expect($("repl-bad").hidden).toBe(false);
    expect($("repl-bad").textContent).toContain("2");
    expect($("repl-count").textContent).toBe(translate("optionsReplCount", "en", { count: 2 }));
  });

  it("saves the desktop app's own settings from the form", async () => {
    const app = fakeApp();
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en" }).loaded;
    $<HTMLInputElement>("nc-hotkey").value = " Ctrl+Alt+V ";
    $<HTMLSelectElement>("nc-inject").value = "paste";
    $<HTMLInputElement>("nc-beside").checked = true;
    $("nc-form").dispatchEvent(new Event("submit", { cancelable: true }));
    await flush();
    expect(app.calls[1]!.body).toEqual({
      ...CONFIG,
      hotkey: "Ctrl+Alt+V",
      inject: "paste",
      besideField: { enabled: true, trigger: "focus" },
    });
  });

  it("says so when the desktop app cannot be reached", async () => {
    const app = fakeApp();
    app.fail();
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en" }).loaded;
    expect($("status").textContent).toBe(translate("settings_failed", "en"));
    expect($("status").className).toBe("err");
  });

  it("puts the saved state back when a save fails", async () => {
    const app = fakeApp();
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en" }).loaded;
    app.fail();
    const en = $<HTMLInputElement>("mode-en");
    en.checked = true;
    en.dispatchEvent(new Event("change"));
    await flush();
    expect($("status").textContent).toBe(translate("settings_failed", "en"));
    expect($<HTMLInputElement>("mode-normal").checked).toBe(true);
  });
});
