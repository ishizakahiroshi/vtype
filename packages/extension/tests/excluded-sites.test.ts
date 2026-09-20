import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { HOST_TAG } from "../src/content/anchor";
import { startWhenAllowed, type SiteGate } from "../src/content/index";
import { createBackground, type BackgroundChrome } from "../src/background/index";
import { initOptionsPage } from "../src/options/options";
import { TOGGLE_SITE_ACK } from "../src/shared/messages";
import {
  EXCLUDED_KEY,
  MAX_EXCLUDED_SITES,
  isExcluded,
  matchesExclusion,
  normalizeExclusion,
  sanitizeExcludedSites,
  withExcluded,
  withoutExcluded,
  withoutExclusionEntry,
  type ExcludedSites,
  type StorageChangeListener,
  type StorageView,
} from "../src/shared/settings";

// C9: the sites vtype stays off on.
//
// Every domain here is a documentation domain (example.com / example.org / example.test /
// evil.test): this list is exactly the kind of setting whose test data would otherwise say
// which sites the author uses.

// ---- synthetic layout --------------------------------------------------------------------
// happy-dom does no layout, so element boxes come from this table (the same device as
// content.test.ts). The stub sits on the prototype, never on a field itself.

interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

const boxes = new Map<Element, Box>();
const FIELD_A: Box = { left: 40, top: 40, width: 200, height: 30 };
const FIELD_B: Box = { left: 40, top: 120, width: 200, height: 30 };

function rectOf(box: Box): DOMRect {
  const { left, top, width, height } = box;
  return {
    x: left,
    y: top,
    left,
    top,
    width,
    height,
    right: left + width,
    bottom: top + height,
    toJSON: () => box,
  } as DOMRect;
}

// ---- storage stub ------------------------------------------------------------------------

interface StubStorage {
  readonly view: StorageView;
  /** What one area holds right now (what another page would read). */
  items(area?: "sync" | "local"): Record<string, unknown>;
}

/** `refuse`: the areas that are unavailable (enterprise policy, quota, no extension context). */
function stubStorage(initial: Record<string, unknown> = {}, refuse: ReadonlyArray<"sync" | "local"> = []): StubStorage {
  const items: Record<"sync" | "local", Record<string, unknown>> = {
    sync: structuredClone(initial),
    local: {},
  };
  const listeners: StorageChangeListener[] = [];
  const area = (name: "sync" | "local"): NonNullable<StorageView["sync"]> => ({
    get: async (keys) => {
      if (refuse.includes(name)) throw new Error("this area is unavailable");
      const wanted = keys === null ? Object.keys(items[name]) : typeof keys === "string" ? [keys] : keys;
      const out: Record<string, unknown> = {};
      for (const key of wanted) if (key in items[name]) out[key] = structuredClone(items[name][key]);
      return out;
    },
    set: async (next) => {
      if (refuse.includes(name)) throw new Error("this area is unavailable");
      const changes: Record<string, { newValue?: unknown }> = {};
      for (const [key, value] of Object.entries(next)) {
        items[name][key] = structuredClone(value);
        changes[key] = { newValue: structuredClone(value) };
      }
      // chrome tells every page about a write, the writer's own page included.
      for (const l of [...listeners]) l(changes, name);
    },
  });
  return {
    view: {
      sync: area("sync"),
      local: area("local"),
      onChanged: {
        addListener: (l) => listeners.push(l),
        removeListener: (l) => {
          const i = listeners.indexOf(l);
          if (i >= 0) listeners.splice(i, 1);
        },
      },
    },
    items: (name = "sync") => structuredClone(items[name]),
  };
}

// ---- runtime stub ------------------------------------------------------------------------

type StubListener = (m: unknown, sender?: unknown, sendResponse?: (response?: unknown) => void) => unknown;

interface StubRuntime {
  /**
   * What the background does to the tab's top frame when the toolbar icon is pressed, and what
   * the page answers it with: chrome reports a send nobody answers as a failure, so the answer
   * is what keeps the background from treating a delivered press as a page without vtype.
   */
  deliver(message: unknown): unknown[];
  readonly runtime: NonNullable<Parameters<typeof startWhenAllowed>[0]>["runtime"];
}

function stubRuntime(): StubRuntime {
  const listeners: StubListener[] = [];
  return {
    deliver(message: unknown): unknown[] {
      const replies: unknown[] = [];
      for (const l of [...listeners]) l(message, undefined, (response?: unknown) => replies.push(response));
      return replies;
    },
    runtime: {
      sendMessage: () => undefined,
      onMessage: {
        addListener: (l: StubListener) => {
          listeners.push(l);
        },
        removeListener: (l: StubListener) => {
          const i = listeners.indexOf(l);
          if (i >= 0) listeners.splice(i, 1);
        },
      },
    },
  };
}

// ---- helpers -----------------------------------------------------------------------------

/** A made-up origin: no test may depend on a real site (or on the test runner's own URL). */
const ORIGIN = "https://example.test";

let gate: SiteGate | null = null;

/** Let the storage reads (promise chains, not timers) finish. */
async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await Promise.resolve();
}

function mount(html: string, layout: Record<string, Box>): Record<string, HTMLElement> {
  const wrap = document.createElement("div");
  wrap.innerHTML = html;
  document.body.append(wrap);
  const fields: Record<string, HTMLElement> = {};
  for (const [id, box] of Object.entries(layout)) {
    const el = wrap.querySelector("#" + id);
    if (el === null) throw new Error("fixture is missing #" + id);
    boxes.set(el, box);
    fields[id] = el as HTMLElement;
  }
  return fields;
}

async function open(storage: StorageView | null, origin = ORIGIN, rt = stubRuntime()): Promise<SiteGate> {
  gate = startWhenAllowed({ hoverCapable: () => true, runtime: rt.runtime, language: "en", storage, origin });
  await settle();
  return gate;
}

/**
 * Whether vtype has put anything on the page at all. Read from the page's side, because that
 * is the claim C9 makes: on an excluded site the page is untouched. (The shadow root is closed,
 * so the mics inside it are counted through the anchor instead — see micCount.)
 */
function hostOnPage(): boolean {
  return document.querySelector(HOST_TAG) !== null;
}

/** How many mics exist. Zero without a content script, which is the excluded case. */
function micCount(g: SiteGate): number {
  const root = g.script?.anchor.root ?? null;
  return root === null ? 0 : root.querySelectorAll(".mic").length;
}

function siteOffButton(g: SiteGate): HTMLButtonElement {
  const button = g.script?.anchor.root?.querySelector(".site-off");
  if (button === null || button === undefined) throw new Error("the panel has no site-off line");
  return button as HTMLButtonElement;
}

beforeEach(() => {
  vi.useFakeTimers();
  boxes.clear();
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    return rectOf(boxes.get(this) ?? { left: 0, top: 0, width: 0, height: 0 });
  });
  vi.spyOn(window, "getComputedStyle").mockImplementation((el: Element) => {
    const overflow = (el as HTMLElement).style?.overflow || "visible";
    return { overflowX: overflow, overflowY: overflow } as CSSStyleDeclaration;
  });
});

afterEach(() => {
  gate?.stop();
  gate = null;
  document.body.innerHTML = "";
  for (const stale of document.querySelectorAll(HOST_TAG)) stale.remove();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

// ---- 検証 1-5: which origins an entry covers ---------------------------------------------

describe("[C9] what an entry in the list covers", () => {
  it("[1] an empty list leaves vtype on, whatever the origin", () => {
    for (const origin of ["https://example.com", "http://example.org", "https://a.b.example.test:8443"]) {
      expect(isExcluded([], origin)).toBe(false);
    }
  });

  it("[2] an origin on the list is off", () => {
    expect(isExcluded(["https://example.com"], "https://example.com")).toBe(true);
  });

  it("[3] that same entry leaves another site alone", () => {
    const sites = ["https://example.com"];
    expect(isExcluded(sites, "https://other.example.org")).toBe(false);
    // A different scheme, port or host is a different origin, and an origin entry says only itself.
    expect(isExcluded(sites, "http://example.com")).toBe(false);
    expect(isExcluded(sites, "https://example.com:8443")).toBe(false);
    expect(isExcluded(sites, "https://www.example.com")).toBe(false);
  });

  it("[4] `*.example.com` covers every host under it, on http and https alike", () => {
    const sites = ["*.example.com"];
    expect(isExcluded(sites, "https://a.example.com")).toBe(true);
    expect(isExcluded(sites, "https://b.example.com")).toBe(true);
    expect(isExcluded(sites, "http://deep.nested.example.com")).toBe(true);
    // ... and the site itself, which is what someone writing a wildcard means by "this site"
    expect(isExcluded(sites, "https://example.com")).toBe(true);
  });

  it("[5] `*.example.com` does not catch a host that merely begins with it", () => {
    const sites = ["*.example.com"];
    expect(isExcluded(sites, "https://example.com.evil.test")).toBe(false);
    expect(isExcluded(sites, "https://notexample.com")).toBe(false);
    expect(isExcluded(sites, "https://evil-example.com")).toBe(false);
    expect(matchesExclusion("*.example.com", "https://example.com.evil.test")).toBe(false);
  });

  it("an origin vtype cannot know (an empty one) is never excluded", () => {
    expect(isExcluded(["https://example.com"], "")).toBe(false);
  });
});

describe("[C9] what the user types becomes an entry", () => {
  it.each([
    ["example.com", "https://example.com"],
    ["https://example.com", "https://example.com"],
    ["https://example.com/inbox?q=1", "https://example.com"],
    ["HTTPS://Example.COM", "https://example.com"],
    ["  example.com  ", "https://example.com"],
    ["http://example.com", "http://example.com"],
    ["https://example.com:8443", "https://example.com:8443"],
    ["*.example.com", "*.example.com"],
    ["*.Example.com/", "*.example.com"],
  ])("%s becomes %s", (typed, stored) => {
    expect(normalizeExclusion(typed)).toBe(stored);
  });

  it.each([[""], ["   "], ["*"], ["*."], ["ftp://example.com"], ["chrome://settings"], ["*.example.com:8443"], [42]])(
    "%s is refused rather than stored as something that matches nothing",
    (typed) => {
      expect(normalizeExclusion(typed)).toBeNull();
    },
  );
});

describe("[C9] keeping the list", () => {
  it("adding normalises, and adding the same site twice changes nothing", () => {
    const once = withExcluded([], "Example.com");
    expect(once).toEqual(["https://example.com"]);
    expect(withExcluded(once, "https://example.com/")).toEqual(["https://example.com"]);
  });

  it("something that is not a site address is not added", () => {
    expect(withExcluded(["https://example.com"], "not a site")).toEqual(["https://example.com"]);
  });

  it("switching a site back on removes every entry that covered it", () => {
    const sites = ["*.example.com", "https://a.example.com", "https://other.example.org"];
    expect(withoutExcluded(sites, "https://a.example.com")).toEqual(["https://other.example.org"]);
  });

  it("removing one entry of the options page's list removes only that one", () => {
    const sites = ["*.example.com", "https://a.example.com"];
    expect(withoutExclusionEntry(sites, "*.example.com")).toEqual(["https://a.example.com"]);
  });

  it("the list is capped, oldest first", () => {
    let sites: ExcludedSites = [];
    for (let i = 0; i < MAX_EXCLUDED_SITES + 5; i++) sites = withExcluded(sites, "https://s" + i + ".example.com");
    expect(sites).toHaveLength(MAX_EXCLUDED_SITES);
    expect(sites[0]).toBe("https://s5.example.com");
  });

  it("a stored list from another version, or a hand-edited one, is read for what is usable", () => {
    expect(sanitizeExcludedSites(["https://example.com", 7, null, "not a site", "example.org", "example.org"])).toEqual([
      "https://example.com",
      "https://example.org",
    ]);
    expect(sanitizeExcludedSites("https://example.com")).toEqual([]);
    expect(sanitizeExcludedSites(undefined)).toEqual([]);
  });
});

// ---- 検証 6: an excluded site gets nothing at all ----------------------------------------

describe("[C9] on an excluded site not one mic is made", () => {
  const stored = { [EXCLUDED_KEY]: [ORIGIN] };

  it("[6] the field that has the caret gets none (the focus path)", async () => {
    const fields = mount(`<input id="a" type="text">`, { a: FIELD_A });
    fields.a.focus();
    const g = await open(stubStorage(stored).view);

    expect(g.excluded).toBe(true);
    expect(g.script).toBeNull();
    expect(hostOnPage()).toBe(false);
    expect(micCount(g)).toBe(0);
  });

  it("[6] the fields on screen get none either (the every-field path)", async () => {
    mount(`<input id="a" type="text"><textarea id="b"></textarea>`, { a: FIELD_A, b: FIELD_B });
    const g = await open(stubStorage(stored).view);
    // The every-field pass also runs on a timer after the page changes; let that timer run.
    await vi.advanceTimersByTimeAsync(500);

    expect(g.script).toBeNull();
    expect(hostOnPage()).toBe(false);
    expect(micCount(g)).toBe(0);
  });

  it("[6] the same page on a site that is not excluded does get them (the control)", async () => {
    const fields = mount(`<input id="a" type="text"><textarea id="b"></textarea>`, { a: FIELD_A, b: FIELD_B });
    fields.a.focus();
    const g = await open(stubStorage(stored).view, "https://allowed.example.org");

    expect(g.excluded).toBe(false);
    expect(g.script).not.toBeNull();
    expect(hostOnPage()).toBe(true);
    expect(micCount(g)).toBe(2);
  });

  it("a wildcard entry keeps every host under it clear", async () => {
    mount(`<input id="a" type="text">`, { a: FIELD_A });
    const g = await open(stubStorage({ [EXCLUDED_KEY]: ["*.example.test"] }).view, "https://mail.example.test");

    expect(g.excluded).toBe(true);
    expect(micCount(g)).toBe(0);
  });

  it("a list that cannot be read anywhere leaves vtype on: the default is every site", async () => {
    mount(`<input id="a" type="text">`, { a: FIELD_A });
    const g = await open(stubStorage(stored, ["sync", "local"]).view);

    expect(g.excluded).toBe(false);
    expect(micCount(g)).toBe(1);
  });
});

// ---- the way out, and the way back --------------------------------------------------------

describe("[C9] switching this site off from the panel", () => {
  it("the panel's line stores the origin and the mics go away on their own", async () => {
    const store = stubStorage();
    const fields = mount(`<input id="a" type="text">`, { a: FIELD_A });
    fields.a.focus();
    const g = await open(store.view);
    expect(micCount(g)).toBe(1);

    const siteOff = siteOffButton(g);
    expect(siteOff.hidden).toBe(false);
    siteOff.click();
    await settle();

    expect(store.items()[EXCLUDED_KEY]).toEqual([ORIGIN]);
    expect(g.excluded).toBe(true);
    expect(g.script).toBeNull();
    expect(hostOnPage()).toBe(false);
  });

  it("the line is not offered where pressing it could not be remembered", async () => {
    const fields = mount(`<input id="a" type="text">`, { a: FIELD_A });
    fields.a.focus();
    const g = await open(null);

    expect(siteOffButton(g).hidden).toBe(true);
  });
});

describe("[C9] switching it back on from the toolbar icon", () => {
  it("the message from the toolbar icon takes this site off the list, and the mics come back", async () => {
    const store = stubStorage({ [EXCLUDED_KEY]: [ORIGIN] });
    const rt = stubRuntime();
    const fields = mount(`<input id="a" type="text">`, { a: FIELD_A });
    fields.a.focus();
    const g = await open(store.view, ORIGIN, rt);
    expect(g.excluded).toBe(true);
    expect(micCount(g)).toBe(0);

    rt.deliver({ target: "content", type: "toggle-site" });
    await settle();

    expect(store.items()[EXCLUDED_KEY]).toEqual([]);
    expect(g.excluded).toBe(false);
    expect(micCount(g)).toBe(1);
  });

  it("pressing it on a site that is on switches that site off", async () => {
    const store = stubStorage();
    const rt = stubRuntime();
    mount(`<input id="a" type="text">`, { a: FIELD_A });
    const g = await open(store.view, ORIGIN, rt);

    rt.deliver({ target: "content", type: "toggle-site" });
    await settle();

    expect(store.items()[EXCLUDED_KEY]).toEqual([ORIGIN]);
    expect(g.excluded).toBe(true);
  });

  it("the page answers the press, so the background knows vtype was there", async () => {
    const store = stubStorage();
    const rt = stubRuntime();
    const g = await open(store.view, ORIGIN, rt);

    expect(rt.deliver({ target: "content", type: "toggle-site" })).toEqual([TOGGLE_SITE_ACK]);
    await settle();
    expect(g.excluded).toBe(true);
  });

  it("a message that is not the toolbar icon is left alone", async () => {
    const store = stubStorage();
    const rt = stubRuntime();
    const g = await open(store.view, ORIGIN, rt);

    rt.deliver({ target: "content", type: "session-event", sessionId: "s", event: { kind: "ended", reason: "user" } });
    await settle();

    expect(store.items()[EXCLUDED_KEY]).toBeUndefined();
    expect(g.excluded).toBe(false);
  });
});

// ---- the background side of the toolbar icon ---------------------------------------------

interface ActionChrome {
  press(tab: { id?: number }): void;
  readonly toTab: Array<{ tabId: number; frameId: number | undefined; message: unknown }>;
  readonly optionsOpened: number;
}

/** Just enough chrome for the toolbar icon; the message bus itself is tested in background.test.ts. */
function actionChrome(reachableTabs: readonly number[], options: { answers?: boolean } = {}): ActionChrome {
  // A reachable page answers TOGGLE_SITE_ACK; `answers: false` is a page that was reached and
  // said nothing, which chrome reports to the sender exactly like an empty answer.
  const answer = options.answers === false ? undefined : TOGGLE_SITE_ACK;
  const toTab: Array<{ tabId: number; frameId: number | undefined; message: unknown }> = [];
  const clicked: Array<(tab: { id?: number }) => void> = [];
  let optionsOpened = 0;
  const chrome: BackgroundChrome = {
    runtime: {
      getURL: (path) => "chrome-extension://synthetic-id/" + path,
      sendMessage: async () => undefined,
      onMessage: { addListener: () => undefined },
      onInstalled: { addListener: () => undefined },
      openOptionsPage: async () => {
        optionsOpened++;
      },
    },
    action: { onClicked: { addListener: (l) => clicked.push(l) } },
    tabs: {
      sendMessage: async (tabId, message, options) => {
        if (!reachableTabs.includes(tabId)) throw new Error("Receiving end does not exist.");
        toTab.push({ tabId, frameId: options?.frameId, message });
        return answer;
      },
      create: async () => ({}),
      onRemoved: { addListener: () => undefined },
    },
    offscreen: { createDocument: async () => undefined },
  };
  createBackground(chrome);
  return {
    press: (tab) => {
      for (const l of clicked) l(tab);
    },
    toTab,
    get optionsOpened() {
      return optionsOpened;
    },
  };
}

describe("[C9] the toolbar icon in the background worker", () => {
  it("asks the tab's top frame, and only the top frame", async () => {
    const a = actionChrome([7]);
    a.press({ id: 7 });
    await settle();

    expect(a.toTab).toEqual([{ tabId: 7, frameId: 0, message: { target: "content", type: "toggle-site" } }]);
    expect(a.optionsOpened).toBe(0);
  });

  it("falls back to the options page where no content script can answer", async () => {
    const a = actionChrome([]);
    a.press({ id: 7 });
    await settle();

    expect(a.toTab).toEqual([]);
    expect(a.optionsOpened).toBe(1);
  });

  it("a page that is reached but answers nothing counts as no page at all", async () => {
    // Chrome closes the port when no listener answers and reports the send as a failure, so
    // "it arrived" cannot be told from the send alone. Only TOGGLE_SITE_ACK means it arrived.
    const a = actionChrome([7], { answers: false });
    a.press({ id: 7 });
    await settle();

    expect(a.toTab).toHaveLength(1);
    expect(a.optionsOpened).toBe(1);
  });

  it("a tab with no id is ignored rather than guessed at", async () => {
    const a = actionChrome([7]);
    a.press({});
    await settle();

    expect(a.toTab).toEqual([]);
    expect(a.optionsOpened).toBe(0);
  });
});

describe("[C9] a change that lands while the first read is still out", () => {
  it("is not undone when that older answer arrives", async () => {
    const store = stubStorage();
    const sync = store.view.sync;
    if (sync === undefined) throw new Error("the stub has no sync area");
    // The first read answers with what was stored when it was asked, but is not let go until
    // this test says so: that gap is the window in which a press elsewhere (the options page,
    // another device) lands on a page that has only just started.
    let letGo = (): void => undefined;
    const held = new Promise<void>((resolve) => {
      letGo = () => resolve();
    });
    let firstRead = true;
    const slow: StorageView = {
      ...store.view,
      sync: {
        get: async (keys) => {
          const answer = (await sync.get(keys)) ?? {};
          if (firstRead) {
            firstRead = false;
            await held;
          }
          return answer;
        },
        set: (next) => sync.set(next),
      },
    };

    mount(`<input id="a" type="text">`, { a: FIELD_A });
    gate = startWhenAllowed({
      hoverCapable: () => true,
      runtime: stubRuntime().runtime,
      language: "en",
      storage: slow,
      origin: ORIGIN,
    });
    const g = gate;
    await settle();
    expect(g.script).toBeNull();

    await sync.set({ [EXCLUDED_KEY]: [ORIGIN] });
    await settle();
    expect(g.excluded).toBe(true);

    letGo();
    await settle();

    expect(g.excluded).toBe(true);
    expect(hostOnPage()).toBe(false);
    expect(micCount(g)).toBe(0);
  });
});

// ---- the options page --------------------------------------------------------------------

describe("[C9] the list in the options page", () => {
  /** The options page's own markup, cut down to the part C9 owns. */
  function mountOptions(): void {
    document.body.innerHTML = [
      `<h2 id="sites-title"></h2><p id="sites-lead"></p>`,
      `<form id="site-form"><input id="site-input" type="text"><button id="site-add" type="submit"></button></form>`,
      `<p id="site-empty"></p><ul id="site-list"></ul><p id="status"></p>`,
    ].join("");
  }

  function shownSites(): string[] {
    return [...document.querySelectorAll("#site-list .site")].map((el) => el.textContent ?? "");
  }

  function type(value: string): void {
    const input = document.getElementById("site-input") as HTMLInputElement;
    input.value = value;
    document.getElementById("site-form")?.dispatchEvent(new Event("submit", { cancelable: true, bubbles: true }));
  }

  function status(): string {
    return document.getElementById("status")?.textContent ?? "";
  }

  it("shows what is stored, and says so when nothing is", async () => {
    mountOptions();
    initOptionsPage({ storage: stubStorage({ [EXCLUDED_KEY]: ["https://example.com"] }).view, language: "en" });
    expect(document.getElementById("site-empty")?.hidden).toBe(false);
    await settle();

    expect(shownSites()).toEqual(["https://example.com"]);
    expect(document.getElementById("site-empty")?.hidden).toBe(true);
  });

  it("adds what was typed, in the shape the content script matches on", async () => {
    const store = stubStorage();
    mountOptions();
    initOptionsPage({ storage: store.view, language: "en" });
    await settle();

    type("Example.com/inbox");
    await settle();

    expect(store.items()[EXCLUDED_KEY]).toEqual(["https://example.com"]);
    expect(shownSites()).toEqual(["https://example.com"]);
    expect((document.getElementById("site-input") as HTMLInputElement).value).toBe("");
  });

  it("says so instead of storing something that would never match", async () => {
    const store = stubStorage();
    mountOptions();
    initOptionsPage({ storage: store.view, language: "en" });
    await settle();

    type("not a site");
    await settle();

    expect(store.items()[EXCLUDED_KEY]).toBeUndefined();
    expect(status()).toContain("not a site address");
  });

  it("switching one back on removes it from the list and from storage", async () => {
    const store = stubStorage({ [EXCLUDED_KEY]: ["https://example.com", "*.example.org"] });
    mountOptions();
    initOptionsPage({ storage: store.view, language: "en" });
    await settle();

    (document.querySelectorAll("#site-list button")[0] as HTMLButtonElement).click();
    await settle();

    expect(store.items()[EXCLUDED_KEY]).toEqual(["*.example.org"]);
    expect(shownSites()).toEqual(["*.example.org"]);
  });

  it("a site switched off from a page arrives here without a reload", async () => {
    const store = stubStorage();
    mountOptions();
    initOptionsPage({ storage: store.view, language: "en" });
    await settle();

    await store.view.sync?.set({ [EXCLUDED_KEY]: ["https://example.com"] });
    await settle();

    expect(shownSites()).toEqual(["https://example.com"]);
  });
});

// ---- where the list is kept ---------------------------------------------------------------

describe("[C9] a profile where chrome.storage.sync is refused", () => {
  it("keeps the list in local storage instead, and still switches the site off", async () => {
    const store = stubStorage({}, ["sync"]);
    const fields = mount(`<input id="a" type="text">`, { a: FIELD_A });
    fields.a.focus();
    const g = await open(store.view);
    expect(micCount(g)).toBe(1);

    siteOffButton(g).click();
    await settle();

    expect(store.items("local")[EXCLUDED_KEY]).toEqual([ORIGIN]);
    expect(g.excluded).toBe(true);
    expect(micCount(g)).toBe(0);
  });

  it("a site switched off before sync broke is still off", async () => {
    const store = stubStorage({}, ["sync"]);
    await store.view.local?.set({ [EXCLUDED_KEY]: [ORIGIN] });
    mount(`<input id="a" type="text">`, { a: FIELD_A });
    const g = await open(store.view);

    expect(g.excluded).toBe(true);
    expect(micCount(g)).toBe(0);
  });
});
