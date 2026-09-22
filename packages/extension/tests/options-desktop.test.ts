// The options page's desktop link section.

import { describe, expect, it } from "vitest";
import { DESKTOP_INSTALL_URL, initOptionsPage } from "../src/options/options";
import type { OptionsToBackground } from "../src/shared/messages";
import { BRIDGE_STATUS_KEY, DESKTOP_BRIDGE_KEY, NATIVE_CONFIG_KEY } from "../src/shared/settings";
import { settle, stubStorage, type Stub } from "./stub-storage";

const CONFIG = {
  hotkey: null,
  icon: { visible: true, x: 10, y: 20, hideOnFullscreen: true },
  inject: "auto",
  besideField: { enabled: false, trigger: "focus" },
  extraExtensionIds: ["abcdefghijklmnopabcdefghijklmnop"],
};

function mount(): void {
  document.body.innerHTML = [
    `<h2><span id="desktop-title"></span></h2><p id="desktop-lead"></p><p id="desktop-status"></p>`,
    `<button id="desktop-enable" type="button"></button><button id="desktop-disable" type="button" hidden></button>`,
    `<button id="desktop-retry" type="button" hidden></button>`,
    `<p id="desktop-install" hidden><a id="desktop-install-link"></a></p>`,
    `<form id="desktop-config" hidden>`,
    `<input id="nc-hotkey" type="text"><input type="checkbox" id="nc-icon-visible"><input type="checkbox" id="nc-icon-fullscreen">`,
    `<select id="nc-inject"><option value="auto"></option><option value="type"></option><option value="paste"></option></select>`,
    `<input type="checkbox" id="nc-beside"><select id="nc-beside-trigger"><option value="focus"></option><option value="hover"></option></select>`,
    `<button id="nc-save" type="submit"></button></form><p id="status"></p>`,
  ].join("");
}

/** A stub with chrome.storage.session (and its change events) too. */
function withSession(s: Stub, initial: Record<string, unknown> = {}): Record<string, unknown> {
  const items: Record<string, unknown> = structuredClone(initial);
  const listeners: Array<(c: Record<string, { newValue?: unknown }>, area: string) => void> = [];
  const events = s.view.onChanged!;
  const add = events.addListener.bind(events);
  events.addListener = (l) => {
    listeners.push(l);
    add(l);
  };
  s.view.session = {
    get: async (keys) => Object.fromEntries((keys as string[]).filter((k) => k in items).map((k) => [k, items[k]])),
    set: async (next) => {
      Object.assign(items, structuredClone(next));
      const changes = Object.fromEntries(Object.entries(next).map(([k, v]) => [k, { newValue: v }]));
      for (const l of [...listeners]) l(changes, "session");
    },
  };
  return items;
}

const hidden = (id: string): boolean => document.getElementById(id)!.hidden;
const statusLine = (): string => document.getElementById("desktop-status")?.textContent ?? "";

function open(store: Stub, extra: Parameters<typeof initOptionsPage>[0] = {}): OptionsToBackground[] {
  const sent: OptionsToBackground[] = [];
  initOptionsPage({
    storage: store.view,
    language: "en",
    permissions: { request: async () => true },
    sendToBackground: async (m) => void sent.push(m),
    ...extra,
  });
  return sent;
}

describe("the desktop link section", () => {
  it("offers to turn it on, asks for the permission, and stores the switch", async () => {
    const store = stubStorage();
    withSession(store);
    mount();
    const asked: string[][] = [];
    open(store, {
      permissions: {
        request: async ({ permissions }) => {
          asked.push(permissions);
          return true;
        },
      },
    });
    await settle();
    expect(statusLine()).toBe("Off.");
    expect(hidden("desktop-enable")).toBe(false);
    document.getElementById("desktop-enable")!.click();
    await settle();
    expect(asked).toEqual([["nativeMessaging"]]);
    expect(store.sync[DESKTOP_BRIDGE_KEY]).toBe(true);
    expect(hidden("desktop-enable")).toBe(true);
    expect(hidden("desktop-disable")).toBe(false);
  });

  it("stays off when the permission is refused", async () => {
    const store = stubStorage();
    withSession(store);
    mount();
    open(store, { permissions: { request: async () => false } });
    document.getElementById("desktop-enable")!.click();
    await settle();
    expect(store.sync[DESKTOP_BRIDGE_KEY]).toBeUndefined();
    expect(document.getElementById("status")?.textContent).toContain("not granted");
  });

  it("points to the installation and offers a retry when the desktop app is missing", async () => {
    const store = stubStorage({ sync: { [DESKTOP_BRIDGE_KEY]: true } });
    withSession(store, { [BRIDGE_STATUS_KEY]: { state: "not-installed" } });
    mount();
    const sent = open(store);
    await settle();
    expect(statusLine()).toContain("not found");
    expect(hidden("desktop-install")).toBe(false);
    expect((document.getElementById("desktop-install-link") as HTMLAnchorElement).href).toBe(DESKTOP_INSTALL_URL);
    expect(hidden("desktop-config")).toBe(true);
    document.getElementById("desktop-retry")!.click();
    await settle();
    expect(sent).toEqual([{ target: "background", type: "native-retry" }]);
  });

  it("shows the desktop app's settings only while connected, and sends changes", async () => {
    const store = stubStorage({ sync: { [DESKTOP_BRIDGE_KEY]: true } });
    const session = withSession(store, { [BRIDGE_STATUS_KEY]: { state: "connecting" }, [NATIVE_CONFIG_KEY]: CONFIG });
    mount();
    const sent = open(store);
    await settle();
    expect(hidden("desktop-config")).toBe(true);

    await store.view.session!.set({ [BRIDGE_STATUS_KEY]: { state: "connected", nativeVersion: "0.1.0" } });
    await settle();
    expect(session[BRIDGE_STATUS_KEY]).toBeDefined();
    expect(statusLine()).toBe("Connected to the desktop app 0.1.0.");
    expect(hidden("desktop-config")).toBe(false);
    expect((document.getElementById("nc-icon-visible") as HTMLInputElement).checked).toBe(true);

    (document.getElementById("nc-hotkey") as HTMLInputElement).value = "Ctrl+Shift+F9";
    (document.getElementById("nc-inject") as HTMLSelectElement).value = "paste";
    (document.getElementById("nc-beside") as HTMLInputElement).checked = true;
    document.getElementById("desktop-config")!.dispatchEvent(new Event("submit", { cancelable: true }));
    await settle();
    expect(sent).toEqual([
      {
        target: "background",
        type: "native-config-set",
        config: {
          ...CONFIG,
          hotkey: "Ctrl+Shift+F9",
          inject: "paste",
          besideField: { enabled: true, trigger: "focus" },
        },
      },
    ]);

    await store.view.session!.set({ [BRIDGE_STATUS_KEY]: { state: "connecting" } });
    await settle();
    expect(hidden("desktop-config")).toBe(true);
  });
});
