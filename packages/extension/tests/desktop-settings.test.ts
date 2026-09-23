// The desktop app's settings page (standalone plan C5): reads and writes the desktop app's config
// through `api/config` next to the page.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { beforeEach, describe, expect, it } from "vitest";
import { apiUrl, initSettingsPage, templateFromHref } from "../src/desktop-settings/settings";
import { translate } from "../src/shared/i18n";

const PAGE = "http://127.0.0.1:47213/t/0123456789abcdef0123456789abcdef/settings";
const API = "http://127.0.0.1:47213/t/0123456789abcdef0123456789abcdef/api/config";
const ABOUT_API = "http://127.0.0.1:47213/t/0123456789abcdef0123456789abcdef/api/about";
const OPEN_API = "http://127.0.0.1:47213/t/0123456789abcdef0123456789abcdef/api/open";

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

/**
 * A desktop app that stores what it is sent and answers with it (or fails). "About vtype" is
 * answered aside: its version, and the links it was asked to open in `opened`, not in `calls`.
 */
function fakeApp(initial: unknown = CONFIG) {
  let stored: unknown = structuredClone(initial);
  const calls: Call[] = [];
  const opened: unknown[] = [];
  let failing = false;
  const fetch = async (url: string, init?: { method?: string; body?: string }) => {
    const method = init?.method ?? "GET";
    const body = init?.body === undefined ? undefined : (JSON.parse(init.body) as unknown);
    if (url === ABOUT_API) return { ok: !failing, json: async () => ({ version: "9.8.7" }) };
    if (url === OPEN_API) {
      if (!failing) opened.push(body);
      return { ok: !failing, json: async () => ({}) };
    }
    calls.push({ url, method, body });
    if (failing) return { ok: false, json: async () => ({}) };
    if (method === "POST") stored = body;
    return { ok: true, json: async () => structuredClone(stored) };
  };
  return {
    fetch,
    calls,
    opened,
    fail: () => {
      failing = true;
    },
  };
}

function settingsBody(): string {
  const html = readFileSync(join(__dirname, "..", "src", "desktop-settings", "settings.html"), "utf8");
  return html.slice(html.indexOf("<body>") + 6, html.indexOf("<script"));
}

function mount(): void {
  document.body.innerHTML = settingsBody();
}

/** Another settings window's document. */
function secondWindow(): Document {
  const doc = document.implementation.createHTMLDocument("");
  doc.body.innerHTML = settingsBody();
  return doc;
}

type Listener = (event: { data: unknown }) => void;

/** BroadcastChannel as settings windows see it: a message reaches every other window, later. */
function channels() {
  const all: Listener[][] = [];
  return () => {
    const mine: Listener[] = [];
    all.push(mine);
    return {
      postMessage(message: unknown) {
        const data = structuredClone(message);
        for (const other of all) if (other !== mine) for (const l of other) setTimeout(() => l({ data }), 0);
      },
      addEventListener(_type: "message", listener: Listener) {
        mine.push(listener);
      },
    };
  };
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

  it("opens the template the templates menu's Edit chose, ready to type in", async () => {
    expect(templateFromHref(`${PAGE}#template-1`)).toBe(1);
    expect(templateFromHref(PAGE)).toBeNull();
    const app = fakeApp({ ...CONFIG, templates: ["one", "two"] });
    await initSettingsPage({ href: `${PAGE}#template-1`, fetch: app.fetch, language: "en" }).loaded;
    expect(apiUrl(`${PAGE}#template-1`)).toBe(API);
    const area = document.querySelector<HTMLTextAreaElement>("#tpl-list .tpl-edit");
    expect(area?.value).toBe("two");
    expect(document.activeElement).toBe(area);
    // A template that is gone by now: the list as usual.
    mount();
    await initSettingsPage({ href: `${PAGE}#template-5`, fetch: fakeApp({ ...CONFIG, templates: ["one"] }).fetch }).loaded;
    expect(document.querySelector("#tpl-list .tpl-edit")).toBeNull();
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
    // A config without silenceStopSec (an older desktop app) shows 3 s.
    expect($<HTMLSelectElement>("nc-silence-stop").value).toBe("3");
    $<HTMLSelectElement>("nc-silence-stop").value = "0";
    $<HTMLInputElement>("nc-beside").checked = true;
    $("nc-form").dispatchEvent(new Event("submit", { cancelable: true }));
    await flush();
    expect(app.calls[1]!.body).toEqual({
      ...CONFIG,
      hotkey: "Ctrl+Alt+V",
      // A config without scale (an older desktop app) is 100 %.
      icon: { ...CONFIG.icon, scale: 100 },
      inject: "paste",
      sendKey: "ctrl-enter",
      silenceStopSec: 0,
      // A config without inChrome (an older desktop app) shows the Chrome switch on.
      besideField: { enabled: true, trigger: "focus", inChrome: true },
    });
  });

  it("shows the Chrome switch of the mic beside fields as saved and saves it off", async () => {
    const app = fakeApp({ ...CONFIG, besideField: { enabled: true, trigger: "focus", inChrome: true } });
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "ja" }).loaded;
    expect($("nc-beside-chrome-label").textContent).toBe(translate("optionsNativeBesideChrome", "ja"));
    expect($("nc-beside-chrome-hint").textContent).toBe(translate("optionsNativeBesideChromeHint", "ja"));
    expect($<HTMLInputElement>("nc-beside-chrome").checked).toBe(true);
    $<HTMLInputElement>("nc-beside-chrome").checked = false;
    $("nc-form").dispatchEvent(new Event("submit", { cancelable: true }));
    await flush();
    expect((app.calls[1]!.body as typeof CONFIG).besideField).toEqual({ enabled: true, trigger: "focus", inChrome: false });
    expect($<HTMLInputElement>("nc-beside-chrome").checked).toBe(false);
  });

  it("offers off and 1-10 s to stop after speaking, shows the saved one and saves the chosen one", async () => {
    const app = fakeApp({ ...CONFIG, silenceStopSec: 7 });
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en" }).loaded;
    const select = $<HTMLSelectElement>("nc-silence-stop");
    expect(select.value).toBe("7");
    const choices = [...select.options].map((o) => [o.value, o.textContent]);
    expect(choices).toEqual([
      ["0", translate("settings_silenceStopOff", "en")],
      ...Array.from({ length: 10 }, (_, i) => [String(i + 1), translate("settings_silenceStopSeconds", "en", { sec: i + 1 })]),
    ]);
    expect(choices[3]![1]).not.toContain("{");
    expect($("nc-silence-stop-label").textContent).toBe(translate("settings_silenceStop", "en"));
    expect($("nc-silence-stop-hint").textContent).toBe(translate("settings_silenceStopHint", "en"));

    select.value = "10";
    $("nc-form").dispatchEvent(new Event("submit", { cancelable: true }));
    await flush();
    expect(app.calls.at(-1)!.body).toMatchObject({ silenceStopSec: 10 });
    expect(select.value).toBe("10");
  });

  it("saves the mic's size at once, within 50-500 %, and puts it back to 100 %", async () => {
    const app = fakeApp({ ...CONFIG, icon: { ...CONFIG.icon, scale: 120 } });
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en" }).loaded;
    const number = $<HTMLInputElement>("nc-icon-scale");
    const range = $<HTMLInputElement>("nc-icon-scale-range");
    const scale = () => (app.calls.at(-1)!.body as { icon: { scale: number } }).icon.scale;
    expect(number.value).toBe("120");
    expect(range.value).toBe("120");
    expect($("nc-icon-scale-hint").textContent).toBe(translate("settings_iconScaleHint", "en", { min: 50, max: 500 }));

    number.value = "137.4";
    number.dispatchEvent(new Event("change"));
    await flush();
    expect(scale()).toBe(137);
    number.value = "900";
    number.dispatchEvent(new Event("change"));
    await flush();
    expect(scale()).toBe(500);
    expect(number.value).toBe("500");
    // The page's own limits are the same as the app's.
    expect([range.min, range.max, number.min, number.max]).toEqual(["50", "500", "50", "500"]);

    // Not a number: nothing is sent and the size shown stays.
    const saves = app.calls.length;
    number.value = "";
    number.dispatchEvent(new Event("change"));
    await flush();
    expect(app.calls.length).toBe(saves);
    expect(number.value).toBe("500");

    // The slider shows the number while it moves and saves when let go.
    range.value = "60";
    range.dispatchEvent(new Event("input"));
    expect(number.value).toBe("60");
    range.dispatchEvent(new Event("change"));
    await flush();
    expect(scale()).toBe(60);

    $("nc-icon-scale-reset").click();
    await flush();
    expect(scale()).toBe(100);
    expect(number.value).toBe("100");
    // The rest of the config goes along unchanged.
    expect(app.calls.at(-1)!.body).toEqual({ ...CONFIG, icon: { ...CONFIG.icon, scale: 100 } });
  });

  it("takes in a size changed with Ctrl+wheel when the page comes back to the front", async () => {
    const app = fakeApp();
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en" }).loaded;
    $<HTMLInputElement>("nc-hotkey").value = "Ctrl+Alt+V"; // typed, not saved yet
    // The desktop app resized (and so moved) the mic meanwhile.
    await app.fetch(API, { method: "POST", body: JSON.stringify({ ...CONFIG, icon: { ...CONFIG.icon, x: 5, scale: 150 } }) });
    window.dispatchEvent(new Event("focus"));
    await flush();
    await flush();
    expect($<HTMLInputElement>("nc-icon-scale").value).toBe("150");
    expect($<HTMLInputElement>("nc-hotkey").value).toBe("Ctrl+Alt+V");
    // Saving the form keeps the new size and place.
    $("nc-form").dispatchEvent(new Event("submit", { cancelable: true }));
    await flush();
    expect(app.calls.at(-1)!.body).toMatchObject({ hotkey: "Ctrl+Alt+V", icon: { x: 5, scale: 150 } });
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

describe("one settings window at a time", () => {
  const editFirstTemplate = (text: string) => {
    const edit = [...document.querySelectorAll("#tpl-list li")][0]!.querySelectorAll("button");
    [...edit].find((b) => b.textContent === translate("settings_templatesEdit", "en"))!.click();
    document.querySelector<HTMLTextAreaElement>("#tpl-list .tpl-edit")!.value = text;
  };
  const settle = async () => {
    for (let i = 0; i < 3; i++) await flush();
  };

  it("an older window hands what is not saved to the newer one and closes", async () => {
    const app = fakeApp({ ...CONFIG, templates: ["one", "two"] });
    const channel = channels();
    let closed = 0;
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en", channel: channel(), openedAt: 1, close: () => closed++ })
      .loaded;
    $<HTMLTextAreaElement>("repl-text").value = "a => b";
    $<HTMLInputElement>("nc-hotkey").value = "Ctrl+Alt+V";
    $<HTMLInputElement>("nc-beside").checked = true;
    $<HTMLTextAreaElement>("tpl-new").value = "three";
    editFirstTemplate("ONE");

    const second = secondWindow();
    let secondClosed = 0;
    await initSettingsPage({
      doc: second,
      href: PAGE,
      fetch: app.fetch,
      language: "en",
      channel: channel(),
      openedAt: 2,
      close: () => secondClosed++,
    }).loaded;
    await settle();
    expect(closed).toBe(1);
    expect(secondClosed).toBe(0);
    const at = <T extends HTMLElement>(id: string) => second.getElementById(id) as T;
    expect(at<HTMLTextAreaElement>("repl-text").value).toBe("a => b");
    expect(at<HTMLInputElement>("nc-hotkey").value).toBe("Ctrl+Alt+V");
    expect(at<HTMLInputElement>("nc-beside").checked).toBe(true);
    expect(at<HTMLTextAreaElement>("tpl-new").value).toBe("three");
    expect(second.querySelector<HTMLTextAreaElement>("#tpl-list .tpl-edit")?.value).toBe("ONE");
    // Handed over, not saved: the user still decides.
    expect(app.calls.filter((c) => c.method === "POST")).toEqual([]);
  });

  it("a changed time to stop after speaking alone is handed over too", async () => {
    const app = fakeApp();
    const channel = channels();
    let closed = 0;
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en", channel: channel(), openedAt: 1, close: () => closed++ })
      .loaded;
    $<HTMLSelectElement>("nc-silence-stop").value = "5";
    const second = secondWindow();
    await initSettingsPage({ doc: second, href: PAGE, fetch: app.fetch, language: "en", channel: channel(), openedAt: 2 }).loaded;
    await settle();
    expect(closed).toBe(1);
    expect((second.getElementById("nc-silence-stop") as HTMLSelectElement).value).toBe("5");
    expect(app.calls.filter((c) => c.method === "POST")).toEqual([]);
  });

  it("a changed Chrome switch of the mic beside fields alone is handed over too", async () => {
    const app = fakeApp();
    const channel = channels();
    let closed = 0;
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en", channel: channel(), openedAt: 1, close: () => closed++ })
      .loaded;
    $<HTMLInputElement>("nc-beside-chrome").checked = false;
    const second = secondWindow();
    await initSettingsPage({ doc: second, href: PAGE, fetch: app.fetch, language: "en", channel: channel(), openedAt: 2 }).loaded;
    await settle();
    expect(closed).toBe(1);
    expect((second.getElementById("nc-beside-chrome") as HTMLInputElement).checked).toBe(false);
    expect(app.calls.filter((c) => c.method === "POST")).toEqual([]);
  });

  it("an older window with nothing unsaved just closes", async () => {
    const app = fakeApp();
    const channel = channels();
    let closed = 0;
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en", channel: channel(), openedAt: 1, close: () => closed++ })
      .loaded;
    const second = secondWindow();
    await initSettingsPage({ doc: second, href: PAGE, fetch: app.fetch, language: "en", channel: channel(), openedAt: 2 }).loaded;
    await settle();
    expect(closed).toBe(1);
    expect((second.getElementById("repl-text") as HTMLTextAreaElement).value).toBe("ぶいたいぷ => vtype");
    expect((second.getElementById("nc-hotkey") as HTMLInputElement).value).toBe("");
  });

  it("the window opened to edit a template keeps that template", async () => {
    const app = fakeApp({ ...CONFIG, templates: ["one", "two"] });
    const channel = channels();
    let closed = 0;
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en", channel: channel(), openedAt: 1, close: () => closed++ })
      .loaded;
    editFirstTemplate("ONE");
    const second = secondWindow();
    await initSettingsPage({ doc: second, href: `${PAGE}#template-1`, fetch: app.fetch, language: "en", channel: channel(), openedAt: 2 })
      .loaded;
    await settle();
    expect(closed).toBe(1);
    expect(second.querySelector<HTMLTextAreaElement>("#tpl-list .tpl-edit")?.value).toBe("two");
  });
});

describe("About vtype on the desktop settings page", () => {
  const rows = (): Record<string, HTMLElement> =>
    Object.fromEntries([...document.querySelectorAll("#about dt")].map((dt) => [dt.textContent, dt.nextElementSibling as HTMLElement]));

  it("shows the desktop app's version, the consent's words and its own parts' licenses", async () => {
    const app = fakeApp();
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en" }).about;
    expect($("about").querySelector("h2")?.textContent).toBe(translate("about_title", "en"));
    const row = rows();
    expect(row[translate("about_version", "en")]?.textContent).toBe("vtype 9.8.7");
    expect(row[translate("about_voice", "en")]?.textContent).toContain(translate("speech_consent_lead", "en"));
    const notices = [...row[translate("about_notices", "en")]!.querySelectorAll("a")].map((a) => a.href);
    expect(notices[0]).toBe("https://github.com/ishizakahiroshi/vtype/releases/download/native-v9.8.7/THIRD_PARTY_NOTICES.txt");
    expect(notices).toHaveLength(3);
    // Reading the version is not a settings call.
    expect(app.calls).toEqual([{ url: API, method: "GET", body: undefined }]);
  });

  it("has its links opened by the desktop app, by name", async () => {
    const app = fakeApp();
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en" }).about;
    const privacy = [...document.querySelectorAll<HTMLAnchorElement>("#about a")].find(
      (a) => a.textContent === translate("about_privacy", "en"),
    )!;
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    privacy.dispatchEvent(click);
    await flush();
    expect(click.defaultPrevented).toBe(true);
    expect(app.opened).toEqual([{ link: "privacy" }]);
  });

  it("is shown without the version when the desktop app does not answer", async () => {
    const app = fakeApp();
    app.fail();
    await initSettingsPage({ href: PAGE, fetch: app.fetch, language: "en" }).about;
    expect(rows()[translate("about_version", "en")]).toBeUndefined();
    expect($("about").querySelectorAll("a").length).toBeGreaterThan(0);
  });
});
