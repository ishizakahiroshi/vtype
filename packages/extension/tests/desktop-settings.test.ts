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

  it("adds, edits, moves and deletes templates, and saves the send-right-away switch", async () => {
    const app = fakeApp({ ...CONFIG, templates: ["one", "two"], templateSendImmediate: false });
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en" }).loaded;
    const items = () => [...document.querySelectorAll("#tpl-list .tpl-text")].map((e) => e.textContent);
    const buttons = (i: number) => [...document.querySelectorAll("#tpl-list li")][i]!.querySelectorAll("button");
    const click = (i: number, label: string) =>
      [...buttons(i)].find((b) => b.textContent === translate(label, "en"))!.click();
    const last = () => app.calls.at(-1)!.body as { templates: string[]; templateSendImmediate: boolean };
    expect(items()).toEqual(["one", "two"]);
    expect($("tpl-empty").hidden).toBe(true);

    // Add (trimmed, line breaks kept); a duplicate is refused without saving.
    $<HTMLTextAreaElement>("tpl-new").value = "  three\nlines  ";
    $("tpl-add").click();
    await flush();
    expect(last().templates).toEqual(["one", "two", "three\nlines"]);
    expect($<HTMLTextAreaElement>("tpl-new").value).toBe("");
    const saves = app.calls.length;
    $<HTMLTextAreaElement>("tpl-new").value = "one";
    $("tpl-add").click();
    await flush();
    expect(app.calls.length).toBe(saves);
    expect($("status").textContent).toBe(translate("settings_templatesDuplicate", "en"));

    // Move the last one up, edit the first, delete the second.
    click(2, "settings_templatesUp");
    await flush();
    expect(last().templates).toEqual(["one", "three\nlines", "two"]);
    click(0, "settings_templatesEdit");
    document.querySelector<HTMLTextAreaElement>("#tpl-list .tpl-edit")!.value = "ONE";
    click(0, "settings_templatesDone");
    await flush();
    expect(items()).toEqual(["ONE", "three\nlines", "two"]);
    click(1, "settings_templatesDelete");
    await flush();
    expect(last().templates).toEqual(["ONE", "two"]);
    expect($("tpl-count").textContent).toBe(translate("settings_templatesCount", "en", { count: 2, max: 100 }));

    const send = $<HTMLInputElement>("tpl-send");
    send.checked = true;
    send.dispatchEvent(new Event("change"));
    await flush();
    expect(last().templateSendImmediate).toBe(true);
  });

  it("says so when there are no templates yet", async () => {
    const app = fakeApp();
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en" }).loaded;
    expect($("tpl-empty").hidden).toBe(false);
    expect($<HTMLInputElement>("tpl-send").checked).toBe(false);
  });

  it("saves the desktop app's own settings from the form", async () => {
    const app = fakeApp();
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en" }).loaded;
    $<HTMLInputElement>("nc-hotkey").value = " Ctrl+Alt+V ";
    $<HTMLSelectElement>("nc-inject").value = "paste";
    // A config without sendKey (an older desktop app) shows Enter.
    expect($<HTMLSelectElement>("nc-send-key").value).toBe("enter");
    $<HTMLSelectElement>("nc-send-key").value = "ctrl-enter";
    $<HTMLInputElement>("nc-beside").checked = true;
    $("nc-form").dispatchEvent(new Event("submit", { cancelable: true }));
    await flush();
    expect(app.calls[1]!.body).toEqual({
      ...CONFIG,
      hotkey: "Ctrl+Alt+V",
      inject: "paste",
      sendKey: "ctrl-enter",
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
