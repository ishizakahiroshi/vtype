import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CLOSE_DELAY_MS, HOST_TAG, MIC_GAP_PX, MIC_SIZE_PX, OPEN_DELAY_MS } from "../src/content/anchor";
import { startContentScript, type ContentScript } from "../src/content/index";
import {
  SETTINGS_AREA,
  TRIGGER_KEY,
  isTriggerMode,
  type StorageChangeListener,
  type StorageView,
  type TriggerMode,
} from "../src/shared/settings";

// ---- synthetic layout -------------------------------------------------------------------
// happy-dom does no layout, so element boxes come from this table. The stub is installed on
// the prototype, never on the field itself, so it cannot show up as a change to the field.

interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

const boxes = new Map<Element, Box>();

function setBox(el: Element, box: Box): void {
  boxes.set(el, box);
}

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

// ---- helpers ----------------------------------------------------------------------------

let script: ContentScript | null = null;
let hoverCapable = true;

function start(extra: Parameters<typeof startContentScript>[0] = {}): ContentScript {
  script = startContentScript({ hoverCapable: () => hoverCapable, ...extra });
  return script;
}

function mount(html: string): HTMLElement {
  const wrap = document.createElement("div");
  wrap.innerHTML = html;
  document.body.append(wrap);
  return wrap;
}

function pick<T extends Element = HTMLElement>(root: ParentNode, selector: string): T {
  const el = root.querySelector(selector);
  if (el === null) throw new Error(`fixture is missing ${selector}`);
  return el as T;
}

function box(s: ContentScript): HTMLElement {
  const root = s.anchor.root;
  if (root === null) throw new Error("anchor has no shadow root yet");
  return pick(root, ".box");
}

function micPosition(s: ContentScript): { left: number; top: number } {
  const b = box(s);
  return { left: parseFloat(b.style.left), top: parseFloat(b.style.top) };
}

function pointer(target: EventTarget, type: string, pointerType: string): void {
  target.dispatchEvent(new PointerEvent(type, { pointerType, bubbles: type !== "pointerenter" && type !== "pointerleave" }));
}

function nextFrames(n = 2): void {
  vi.advanceTimersByTime(16 * n + 1);
}

function blurTo(el: HTMLElement | null): void {
  if (el === null) (document.activeElement as HTMLElement | null)?.blur();
  else el.focus();
  vi.advanceTimersByTime(1); // onFocusOut settles on a 0 ms timer
}

beforeEach(() => {
  vi.useFakeTimers();
  hoverCapable = true;
  boxes.clear();
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    return rectOf(boxes.get(this) ?? { left: 0, top: 0, width: 0, height: 0 });
  });
  // happy-dom computes `overflow-x/y` as "" even for `overflow: auto`, so scroll containers
  // are declared through the inline `overflow` of the fixture instead.
  vi.spyOn(window, "getComputedStyle").mockImplementation((el: Element) => {
    const overflow = (el as HTMLElement).style?.overflow || "visible";
    return { overflowX: overflow, overflowY: overflow } as CSSStyleDeclaration;
  });
});

afterEach(() => {
  script?.stop();
  script = null;
  document.body.innerHTML = "";
  for (const stale of document.querySelectorAll(HOST_TAG)) stale.remove();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

const FIELD: Box = { left: 100, top: 50, width: 200, height: 30 };

// ---- detection end to end ---------------------------------------------------------------

describe("mic appears for each target kind", () => {
  it.each([
    ["input type=text", `<input id="f" type="text">`],
    ["input type=search", `<input id="f" type="search">`],
    ["input type=email", `<input id="f" type="email">`],
    ["input type=url", `<input id="f" type="url">`],
    ["input type=tel", `<input id="f" type="tel">`],
    ["textarea", `<textarea id="f"></textarea>`],
    ["contenteditable", `<div id="f" contenteditable="true" tabindex="0">x</div>`],
  ])("%s", (_label, html) => {
    const s = start();
    const field = pick(mount(html), "#f");
    setBox(field, FIELD);
    field.focus();
    expect(s.anchor.target).toBe(field);
    expect(s.anchor.isVisible).toBe(true);
    expect(s.anchor.host?.parentNode).toBe(document.documentElement);
    expect(box(s).hidden).toBe(false);
    // outside, to the right of the field, vertically centered on a single-line box
    expect(micPosition(s)).toEqual({
      left: FIELD.left + FIELD.width + MIC_GAP_PX,
      top: FIELD.top + (FIELD.height - MIC_SIZE_PX) / 2,
    });
  });

  it("an input inside an open shadow root gets the mic", () => {
    const s = start();
    const shadowHost = mount(`<div id="sh"></div>`).querySelector("#sh") as HTMLElement;
    const shadow = shadowHost.attachShadow({ mode: "open" });
    shadow.innerHTML = `<input id="inner" type="text">`;
    const inner = pick(shadow, "#inner");
    setBox(inner, FIELD);
    inner.focus();
    expect(s.anchor.target).toBe(inner);
    expect(s.anchor.isVisible).toBe(true);
  });

  it("a field focused before the script starts gets the mic at start", () => {
    const field = pick(mount(`<input id="f" type="text">`), "#f");
    setBox(field, FIELD);
    field.focus();
    const s = start();
    expect(s.anchor.target).toBe(field);
  });
});

describe("password fields never get the mic", () => {
  it("input type=password: no target and no host is ever created", () => {
    const s = start();
    const pw = pick(mount(`<input id="pw" type="password">`), "#pw");
    setBox(pw, FIELD);
    pw.focus();
    expect(document.activeElement).toBe(pw);
    expect(s.anchor.target).toBeNull();
    expect(s.anchor.host).toBeNull();
    expect(document.querySelector(HOST_TAG)).toBeNull();
  });

  it("moving focus from a text field to a password field hides the mic", () => {
    const s = start();
    const root = mount(`<input id="t" type="text"><input id="pw" type="password">`);
    const text = pick(root, "#t");
    const pw = pick(root, "#pw");
    setBox(text, FIELD);
    setBox(pw, { ...FIELD, top: 100 });
    text.focus();
    expect(s.anchor.isVisible).toBe(true);
    blurTo(pw);
    expect(s.anchor.target).toBeNull();
    expect(s.anchor.isVisible).toBe(false);
    expect(box(s).hidden).toBe(true);
  });

  it("a focused text field that the page turns into type=password loses the mic", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text">`), "#f");
    setBox(field, FIELD);
    field.focus();
    expect(s.anchor.isVisible).toBe(true);
    field.type = "password"; // the page does this ("hide password" toggle), not vtype
    await Promise.resolve(); // MutationObserver callbacks are microtasks
    expect(s.anchor.target).toBeNull();
    expect(s.anchor.isVisible).toBe(false);
  });
});

// ---- the field's DOM is never written ---------------------------------------------------

describe("the field's DOM is untouched", () => {
  it.each([
    ["input", `<input id="f" type="text" class="page-cls" style="color: red" value="abc">`],
    ["textarea", `<textarea id="f" class="page-cls" style="color: red">abc</textarea>`],
    ["contenteditable", `<div id="f" contenteditable="true" class="page-cls" style="color: red">abc</div>`],
  ])("%s: focus, hover-open, scroll, resize, tap and blur leave it byte-identical", async (_label, html) => {
    const root = mount(html + `<button id="other">x</button>`);
    const field = pick(root, "#f");
    const other = pick(root, "#other");
    setBox(field, FIELD);

    const parent = field.parentNode;
    const before = {
      outerHTML: field.outerHTML,
      attributes: [...field.attributes].map((a) => `${a.name}=${a.value}`),
      style: field.getAttribute("style"),
      siblings: [...(parent?.childNodes ?? [])],
    };

    const records: MutationRecord[] = [];
    const observer = new MutationObserver((list) => records.push(...list));
    observer.observe(document, { subtree: true, attributes: true, childList: true, characterData: true });

    const s = start();
    field.focus();
    const b = box(s);
    pointer(b, "pointerenter", "mouse");
    vi.advanceTimersByTime(OPEN_DELAY_MS);
    expect(s.anchor.isOpen).toBe(true);
    document.dispatchEvent(new Event("scroll"));
    window.dispatchEvent(new Event("resize"));
    setBox(field, { ...FIELD, top: 80 });
    nextFrames();
    pointer(b, "pointerleave", "mouse");
    vi.advanceTimersByTime(CLOSE_DELAY_MS);
    pointer(pick(s.anchor.root as ShadowRoot, ".mic"), "pointerup", "touch");
    blurTo(other);
    await Promise.resolve();
    observer.disconnect();
    records.push(...observer.takeRecords());

    expect(field.outerHTML).toBe(before.outerHTML);
    expect([...field.attributes].map((a) => `${a.name}=${a.value}`)).toEqual(before.attributes);
    expect(field.getAttribute("style")).toBe(before.style);
    expect(field.parentNode).toBe(parent);
    expect([...(parent?.childNodes ?? [])]).toEqual(before.siblings);

    // The only mutation of the page's DOM is our host being appended to <html>.
    expect(records.length).toBeGreaterThan(0);
    const foreign = records.filter(
      (r) => !(r.type === "childList" && r.target === document.documentElement && [...r.addedNodes].every((n) => n === s.anchor.host) && r.removedNodes.length === 0),
    );
    expect(foreign).toEqual([]);
  });
});

// ---- hover / tap ------------------------------------------------------------------------

describe("hover opens after a delay and closes after a separate delay", () => {
  function setup(): { s: ContentScript; b: HTMLElement; mic: HTMLElement } {
    const s = start();
    const field = pick(mount(`<input id="f" type="text">`), "#f");
    setBox(field, FIELD);
    field.focus();
    return { s, b: box(s), mic: pick(s.anchor.root as ShadowRoot, ".mic") };
  }

  it("uses different open and close delays", () => {
    expect(OPEN_DELAY_MS).not.toBe(CLOSE_DELAY_MS);
    expect(OPEN_DELAY_MS).toBeGreaterThan(0);
    expect(CLOSE_DELAY_MS).toBeGreaterThan(0);
  });

  it("opens only after OPEN_DELAY_MS of hovering", () => {
    const { s, b } = setup();
    pointer(b, "pointerenter", "mouse");
    vi.advanceTimersByTime(OPEN_DELAY_MS - 1);
    expect(s.anchor.isOpen).toBe(false);
    vi.advanceTimersByTime(1);
    expect(s.anchor.isOpen).toBe(true);
    expect(s.anchor.panel?.hidden).toBe(false);
  });

  it("a pass-over shorter than the open delay never opens", () => {
    const { s, b } = setup();
    pointer(b, "pointerenter", "mouse");
    vi.advanceTimersByTime(OPEN_DELAY_MS - 50);
    pointer(b, "pointerleave", "mouse");
    vi.advanceTimersByTime(OPEN_DELAY_MS * 3);
    expect(s.anchor.isOpen).toBe(false);
  });

  it("closes only after CLOSE_DELAY_MS once the pointer leaves", () => {
    const { s, b } = setup();
    pointer(b, "pointerenter", "mouse");
    vi.advanceTimersByTime(OPEN_DELAY_MS);
    pointer(b, "pointerleave", "mouse");
    vi.advanceTimersByTime(CLOSE_DELAY_MS - 1);
    expect(s.anchor.isOpen).toBe(true);
    vi.advanceTimersByTime(1);
    expect(s.anchor.isOpen).toBe(false);
    expect(s.anchor.panel?.hidden).toBe(true);
  });

  it("leaving the mic and entering the panel within the close delay keeps it open", () => {
    const { s, b } = setup();
    pointer(b, "pointerenter", "mouse");
    vi.advanceTimersByTime(OPEN_DELAY_MS);
    pointer(b, "pointerleave", "mouse"); // crossing a gap between mic and panel
    vi.advanceTimersByTime(CLOSE_DELAY_MS - 100);
    pointer(b, "pointerenter", "mouse");
    vi.advanceTimersByTime(CLOSE_DELAY_MS * 3);
    expect(s.anchor.isOpen).toBe(true);
  });

  it("the panel sits inside the same hover region as the mic", () => {
    const { s, b } = setup();
    expect(s.anchor.panel?.parentElement).toBe(b);
    expect(s.anchor.mic?.parentElement).toBe(b);
  });

  // C7e: pressing the mic used to do nothing where the pointer can hover (hover was the only
  // way to open the panel). A press is now the way to start a recording, so it opens too.
  it("a mouse press on the mic opens the panel at once, without the hover delay", () => {
    const { s, mic } = setup();
    pointer(mic, "pointerup", "mouse");
    expect(s.anchor.isOpen).toBe(true);
  });

  it("pressing the mic does not take focus away from the field", () => {
    const { b } = setup();
    const down = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    b.dispatchEvent(down);
    expect(down.defaultPrevented).toBe(true);
  });
});

describe("tap opens on touch / no-hover devices", () => {
  function setup(): { s: ContentScript; b: HTMLElement; mic: HTMLElement } {
    const s = start();
    const field = pick(mount(`<input id="f" type="text">`), "#f");
    setBox(field, FIELD);
    field.focus();
    return { s, b: box(s), mic: pick(s.anchor.root as ShadowRoot, ".mic") };
  }

  // C7e: a second tap used to close the panel. It now starts and stops the recording instead,
  // so it leaves the panel open (that is where the recording is stopped from); the panel goes
  // away when the field loses focus.
  it("a touch tap opens the panel immediately, and tapping again keeps it open", () => {
    const { s, mic } = setup();
    pointer(mic, "pointerup", "touch");
    expect(s.anchor.isOpen).toBe(true);
    pointer(mic, "pointerup", "touch");
    expect(s.anchor.isOpen).toBe(true);
  });

  it("touch pointerenter does not start the hover timer", () => {
    const { s, b } = setup();
    pointer(b, "pointerenter", "touch");
    vi.advanceTimersByTime(OPEN_DELAY_MS * 3);
    expect(s.anchor.isOpen).toBe(false);
  });

  it("with (hover: none), any pointer taps open and hover does nothing", () => {
    hoverCapable = false;
    const { s, b, mic } = setup();
    pointer(b, "pointerenter", "mouse");
    vi.advanceTimersByTime(OPEN_DELAY_MS * 3);
    expect(s.anchor.isOpen).toBe(false);
    pointer(mic, "pointerup", "mouse");
    expect(s.anchor.isOpen).toBe(true);
  });
});

// ---- C7e: what starts a recording -------------------------------------------------------
// Two settings: `click` (the default: the thin mic beside the field starts and stops) and
// `hover` (the panel opening starts it by itself). The whole extension bus is not needed here:
// this checks the wiring between the panel, the setting and the controller, so the runtime is a
// stub that records what the content script sends and can hand back session events, and the
// storage is a stub that can be changed while the page runs. (The end-to-end path with the real
// background, offscreen document and recognizer runs in controller.test.ts.)

interface StubRuntime {
  /** Everything the content script sent to the background. */
  readonly sent: Array<Record<string, unknown>>;
  /** Deliver a session event for the session that is currently running. */
  emit(event: Record<string, unknown>): void;
  readonly runtime: NonNullable<Parameters<typeof startContentScript>[0]>["runtime"];
}

function stubRuntime(): StubRuntime {
  const sent: Array<Record<string, unknown>> = [];
  const listeners: Array<(m: unknown) => void> = [];
  return {
    sent,
    emit(event: Record<string, unknown>): void {
      const started = [...sent].reverse().find((m) => m.type === "start");
      const sessionId = started?.sessionId ?? "";
      for (const l of [...listeners]) l({ target: "content", type: "session-event", sessionId, event });
    },
    runtime: {
      sendMessage: (message: unknown) => {
        sent.push(message as Record<string, unknown>);
      },
      onMessage: {
        addListener: (l: (m: unknown) => void) => {
          listeners.push(l);
        },
        removeListener: (l: (m: unknown) => void) => {
          const i = listeners.indexOf(l);
          if (i >= 0) listeners.splice(i, 1);
        },
      },
    },
  };
}

interface StubStorage {
  readonly view: StorageView;
  /** The options page (or another device) stored a new value; undefined clears it. */
  change(mode: TriggerMode | undefined): void;
}

/** `broken`: a profile where storage is unavailable (policy, quota, no extension context). */
function stubStorage(initial?: TriggerMode, broken = false): StubStorage {
  let stored = initial;
  const listeners: StorageChangeListener[] = [];
  const refuse = (): never => {
    throw new Error("storage is unavailable");
  };
  return {
    view: {
      sync: {
        get: async () => (broken ? refuse() : stored === undefined ? {} : { [TRIGGER_KEY]: stored }),
        set: async (items) => {
          if (broken) refuse();
          const value = items[TRIGGER_KEY];
          stored = isTriggerMode(value) ? value : undefined;
        },
      },
      onChanged: {
        addListener: (l) => {
          if (broken) refuse();
          listeners.push(l);
        },
        removeListener: (l) => {
          const i = listeners.indexOf(l);
          if (i >= 0) listeners.splice(i, 1);
        },
      },
    },
    change(mode: TriggerMode | undefined): void {
      stored = mode;
      for (const l of [...listeners]) l({ [TRIGGER_KEY]: { newValue: mode } }, SETTINGS_AREA);
    },
  };
}

/** Let the storage read (a promise chain, not a timer) finish. */
async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

interface Setup {
  s: ContentScript;
  b: HTMLElement;
  mic: HTMLElement;
  field: HTMLInputElement;
  rt: StubRuntime;
}

function setupWith(storage: StorageView | null): Setup {
  const rt = stubRuntime();
  const s = start({ runtime: rt.runtime, language: "en", storage });
  const field = pick<HTMLInputElement>(mount(`<input id="f" type="text">`), "#f");
  setBox(field, FIELD);
  field.focus();
  return { s, b: box(s), mic: pick(s.anchor.root as ShadowRoot, ".mic"), field, rt };
}

function sentOfType(rt: StubRuntime, type: string): Array<Record<string, unknown>> {
  return rt.sent.filter((m) => m.type === type);
}

function hoverOpen(b: HTMLElement): void {
  pointer(b, "pointerenter", "mouse");
  vi.advanceTimersByTime(OPEN_DELAY_MS);
}

/** Press the thin mic beside the field (a click; a tap arrives the same way). */
function pressMic(s: ContentScript, pointerType = "mouse"): void {
  pointer(pick(s.anchor.root as ShadowRoot, ".mic"), "pointerup", pointerType);
}

function panelMic(s: ContentScript): HTMLButtonElement {
  return pick<HTMLButtonElement>(s.anchor.root as ShadowRoot, ".record");
}

function messageLine(s: ContentScript): HTMLElement {
  return pick(s.anchor.root as ShadowRoot, ".message");
}

describe("[C7e] pressing the thin mic starts and stops (the default `click` setting)", () => {
  /** No storage at all: the default setting, and nothing is ever read. */
  function setup(): Setup {
    return setupWith(null);
  }

  it("[1] pressing the mic opens the panel and starts one session", () => {
    const { s, rt } = setup();
    expect(s.anchor.isOpen).toBe(false);
    pressMic(s);
    expect(s.anchor.isOpen).toBe(true);
    expect(sentOfType(rt, "start")).toHaveLength(1);
    expect(s.controller.phase).toBe("recording");
    expect(s.anchor.ui?.state).toBe("recording");
  });

  it("[1] a tap does the same on a touch / no-hover device", () => {
    hoverCapable = false;
    const { s, rt } = setup();
    pressMic(s, "touch");
    expect(s.anchor.isOpen).toBe(true);
    expect(sentOfType(rt, "start")).toHaveLength(1);
    expect(s.controller.phase).toBe("recording");
  });

  it("[2] hovering only opens the panel; nothing starts", () => {
    const { s, b, rt } = setup();
    hoverOpen(b);
    expect(s.anchor.isOpen).toBe(true);
    vi.advanceTimersByTime(OPEN_DELAY_MS * 3); // and waiting longer changes nothing
    expect(sentOfType(rt, "start")).toHaveLength(0);
    expect(s.controller.phase).toBe("idle");
    expect(s.anchor.ui?.state).toBe("idle");
    expect(messageLine(s).hidden).toBe(true);
  });

  it("[2] a pass-over neither opens the panel nor starts anything", () => {
    const { s, b, rt } = setup();
    pointer(b, "pointerenter", "mouse");
    vi.advanceTimersByTime(OPEN_DELAY_MS - 50);
    pointer(b, "pointerleave", "mouse");
    vi.advanceTimersByTime(OPEN_DELAY_MS * 3);
    expect(s.anchor.isOpen).toBe(false);
    expect(sentOfType(rt, "start")).toHaveLength(0);
  });

  it("[3] pressing the mic again stops the recording", () => {
    const { s, rt } = setup();
    pressMic(s);
    pressMic(s);
    expect(sentOfType(rt, "stop")).toHaveLength(1);
    expect(sentOfType(rt, "stop")[0]?.sessionId).toBe(sentOfType(rt, "start")[0]?.sessionId);
    expect(s.controller.phase).toBe("stopping");
    expect(s.anchor.isOpen).toBe(true); // the panel is where the user sees it stop
    rt.emit({ kind: "ended", reason: "user" });
    expect(s.controller.phase).toBe("idle");
    expect(sentOfType(rt, "start")).toHaveLength(1); // the second press started nothing
  });

  it("[4] the panel stays open while recording, however long the pointer is away", () => {
    const { s, b } = setup();
    pressMic(s);
    pointer(b, "pointerleave", "mouse");
    vi.advanceTimersByTime(CLOSE_DELAY_MS * 4);
    expect(s.controller.phase).toBe("recording");
    expect(s.anchor.isOpen).toBe(true);
    expect(s.anchor.panel?.hidden).toBe(false);
  });

  it("[5] once the recording has ended, leaving closes the panel as before", () => {
    const { s, b, rt } = setup();
    pressMic(s);
    panelMic(s).click(); // stop from inside the panel
    rt.emit({ kind: "ended", reason: "user" });
    expect(s.controller.phase).toBe("idle");
    pointer(b, "pointerleave", "mouse");
    vi.advanceTimersByTime(CLOSE_DELAY_MS - 1);
    expect(s.anchor.isOpen).toBe(true);
    vi.advanceTimersByTime(1);
    expect(s.anchor.isOpen).toBe(false);
  });

  it("[5] a close that was held back happens as soon as the recording ends", () => {
    const { s, b, rt } = setup();
    pressMic(s);
    pointer(b, "pointerleave", "mouse"); // the pointer is already gone while recording
    vi.advanceTimersByTime(CLOSE_DELAY_MS * 2);
    expect(s.anchor.isOpen).toBe(true);
    // With the pointer away, the end comes from the session itself (a long silence).
    rt.emit({ kind: "ended", reason: "silence" });
    expect(s.controller.phase).toBe("idle");
    vi.advanceTimersByTime(CLOSE_DELAY_MS);
    expect(s.anchor.isOpen).toBe(false);
  });

  it("[6] a refused microphone is shown in the panel, with the way to allow it", () => {
    const { s, rt } = setup();
    pressMic(s);
    rt.emit({ kind: "ended", reason: "error", code: "not-allowed" });
    const line = messageLine(s);
    expect(line.hidden).toBe(false);
    expect(line.textContent).toContain("not allowed");
    pick<HTMLButtonElement>(line, ".message-action").click();
    expect(sentOfType(rt, "open-permission")).toHaveLength(1);
    // A press is a press: the click path keeps no memory of the refusal and tries again.
    pressMic(s);
    expect(sentOfType(rt, "start")).toHaveLength(2);
  });

  it("[7] a field that is not a target starts nothing and says why", () => {
    const { s, field, rt } = setup();
    field.type = "password"; // the page's "show password" toggle; the observer runs later
    pressMic(s);
    expect(sentOfType(rt, "start")).toHaveLength(0);
    expect(s.controller.phase).toBe("idle");
    expect(messageLine(s).textContent).toContain("text field"); // the press gets an answer
  });

  it("[7] with no field the mic does nothing at all", () => {
    const { s, rt } = setup();
    blurTo(null);
    expect(s.anchor.target).toBeNull();
    pressMic(s);
    expect(s.anchor.isOpen).toBe(false);
    expect(sentOfType(rt, "start")).toHaveLength(0);
    expect(s.controller.phase).toBe("idle");
  });

  it("[8] pressing the mic leaves the field focused, with the caret where it was", () => {
    const { s, mic, field, rt } = setup();
    field.value = "hello world";
    field.setSelectionRange(5, 5);
    const down = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    mic.dispatchEvent(down);
    expect(down.defaultPrevented).toBe(true); // this is what keeps the focus in a browser
    pressMic(s);
    expect(document.activeElement).toBe(field);
    expect(field.selectionStart).toBe(5);
    expect(field.selectionEnd).toBe(5);
    expect(sentOfType(rt, "start")).toHaveLength(1); // and the press did start the recording
  });
});

describe("[C7e 10] the `hover` setting: the panel opening starts the recording", () => {
  async function setup(): Promise<Setup> {
    const ready = setupWith(stubStorage("hover").view);
    await settle();
    expect(ready.s.trigger).toBe("hover");
    return ready;
  }

  /** The user pressed the panel's mic to stop, and the session ended. */
  function stopByUser(s: ContentScript, rt: StubRuntime): void {
    panelMic(s).click();
    rt.emit({ kind: "ended", reason: "user" });
  }

  it("hovering the mic until the panel opens starts one session", async () => {
    const { s, b, rt } = await setup();
    expect(s.controller.phase).toBe("idle");
    hoverOpen(b);
    expect(s.anchor.isOpen).toBe(true);
    expect(sentOfType(rt, "start")).toHaveLength(1);
    expect(s.controller.phase).toBe("recording");
    expect(s.anchor.ui?.state).toBe("recording");
  });

  it("a pass-over neither opens the panel nor starts anything", async () => {
    const { s, b, rt } = await setup();
    pointer(b, "pointerenter", "mouse");
    vi.advanceTimersByTime(OPEN_DELAY_MS - 50);
    pointer(b, "pointerleave", "mouse");
    vi.advanceTimersByTime(OPEN_DELAY_MS * 3);
    expect(s.anchor.isOpen).toBe(false);
    expect(sentOfType(rt, "start")).toHaveLength(0);
    expect(s.controller.phase).toBe("idle");
  });

  it("pressing the thin mic still starts once, and stops", async () => {
    const { s, rt } = await setup();
    pressMic(s); // opening the panel must not start a second session on top of this one
    expect(sentOfType(rt, "start")).toHaveLength(1);
    expect(s.controller.phase).toBe("recording");
    pressMic(s);
    expect(sentOfType(rt, "stop")).toHaveLength(1);
    expect(sentOfType(rt, "start")).toHaveLength(1);
  });

  it("the panel stays open while recording, however long the pointer is away", async () => {
    const { s, b } = await setup();
    hoverOpen(b);
    pointer(b, "pointerleave", "mouse");
    vi.advanceTimersByTime(CLOSE_DELAY_MS * 4);
    expect(s.controller.phase).toBe("recording");
    expect(s.anchor.isOpen).toBe(true);
  });

  it("once the recording has ended, leaving closes the panel as before", async () => {
    const { s, b, rt } = await setup();
    hoverOpen(b);
    stopByUser(s, rt);
    pointer(b, "pointerleave", "mouse");
    vi.advanceTimersByTime(CLOSE_DELAY_MS);
    expect(s.anchor.isOpen).toBe(false);
  });

  it("a recording the user stopped does not start again while the panel stays open", async () => {
    const { s, b, rt } = await setup();
    hoverOpen(b);
    stopByUser(s, rt);
    expect(s.controller.phase).toBe("idle");
    expect(s.anchor.isOpen).toBe(true);
    pointer(b, "pointerenter", "mouse"); // the pointer keeps moving over the open panel
    vi.advanceTimersByTime(OPEN_DELAY_MS * 3);
    expect(s.anchor.isOpen).toBe(true);
    expect(s.controller.phase).toBe("idle");
    expect(sentOfType(rt, "start")).toHaveLength(1);
  });

  it("closing the panel and opening it again starts a new session", async () => {
    const { s, b, rt } = await setup();
    hoverOpen(b);
    stopByUser(s, rt);
    pointer(b, "pointerleave", "mouse");
    vi.advanceTimersByTime(CLOSE_DELAY_MS);
    expect(s.anchor.isOpen).toBe(false);
    hoverOpen(b);
    expect(s.controller.phase).toBe("recording");
    expect(sentOfType(rt, "start")).toHaveLength(2);
    expect(sentOfType(rt, "start")[0]?.sessionId).not.toBe(sentOfType(rt, "start")[1]?.sessionId);
  });

  it("after the microphone was refused, opening no longer starts; the mic still tries", async () => {
    const { s, b, rt } = await setup();
    hoverOpen(b);
    rt.emit({ kind: "ended", reason: "error", code: "not-allowed" });
    expect(messageLine(s).textContent).toContain("not allowed");

    pointer(b, "pointerleave", "mouse");
    vi.advanceTimersByTime(CLOSE_DELAY_MS);
    hoverOpen(b); // opening again must not ask the microphone a second time
    expect(s.anchor.isOpen).toBe(true);
    expect(sentOfType(rt, "start")).toHaveLength(1);

    panelMic(s).click(); // pressing is still allowed to try
    expect(sentOfType(rt, "start")).toHaveLength(2);
    expect(s.controller.phase).toBe("recording");
  });

  it("a field that is no longer a target starts nothing, and says nothing", async () => {
    const { s, b, field, rt } = await setup();
    field.type = "password"; // the page's "show password" toggle; the observer runs later
    hoverOpen(b);
    expect(s.anchor.isOpen).toBe(true);
    expect(sentOfType(rt, "start")).toHaveLength(0);
    expect(messageLine(s).hidden).toBe(true); // a hover must not produce an error message
  });

  it("with no field at all nothing starts", async () => {
    const { s, rt } = await setup();
    blurTo(null);
    expect(s.anchor.target).toBeNull();
    s.controller.autoStart();
    expect(sentOfType(rt, "start")).toHaveLength(0);
    expect(s.controller.phase).toBe("idle");
  });
});

describe("[C7e] the setting itself", () => {
  it("[9] with nothing stored it behaves as click", async () => {
    const { s, b, rt } = setupWith(stubStorage().view);
    await settle();
    expect(s.trigger).toBe("click");
    hoverOpen(b);
    expect(sentOfType(rt, "start")).toHaveLength(0);
    pressMic(s);
    expect(sentOfType(rt, "start")).toHaveLength(1);
  });

  it("[9] a stored value that is not one of the two modes behaves as click", async () => {
    const store = stubStorage();
    const { s, b, rt } = setupWith(store.view);
    await settle();
    store.change("whatever" as TriggerMode); // an older or newer version wrote something else
    expect(s.trigger).toBe("click");
    hoverOpen(b);
    expect(sentOfType(rt, "start")).toHaveLength(0);
  });

  it("[10] switching to hover takes effect without reloading the page", async () => {
    const store = stubStorage();
    const { s, b, rt } = setupWith(store.view);
    await settle();
    hoverOpen(b);
    expect(sentOfType(rt, "start")).toHaveLength(0); // click setting: hover does not start
    pointer(b, "pointerleave", "mouse");
    vi.advanceTimersByTime(CLOSE_DELAY_MS);
    expect(s.anchor.isOpen).toBe(false);

    store.change("hover"); // the options page saves; the page is not reloaded
    expect(s.trigger).toBe("hover");
    hoverOpen(b);
    expect(sentOfType(rt, "start")).toHaveLength(1);
  });

  it("[11] switching back to click stops the automatic start", async () => {
    const store = stubStorage("hover");
    const { s, b, rt } = setupWith(store.view);
    await settle();
    expect(s.trigger).toBe("hover");
    store.change("click");
    expect(s.trigger).toBe("click");
    hoverOpen(b);
    expect(sentOfType(rt, "start")).toHaveLength(0);
    pressMic(s); // and the mic still works
    expect(sentOfType(rt, "start")).toHaveLength(1);
  });

  it("[11] clearing the setting goes back to click", async () => {
    const store = stubStorage("hover");
    const { s, b, rt } = setupWith(store.view);
    await settle();
    store.change(undefined);
    expect(s.trigger).toBe("click");
    hoverOpen(b);
    expect(sentOfType(rt, "start")).toHaveLength(0);
  });

  it("[12] a storage that cannot be read breaks nothing: it behaves as click", async () => {
    const { s, b, rt } = setupWith(stubStorage("hover", true).view);
    await settle();
    expect(s.trigger).toBe("click");
    hoverOpen(b);
    expect(sentOfType(rt, "start")).toHaveLength(0);
    pressMic(s);
    expect(sentOfType(rt, "start")).toHaveLength(1);
    expect(s.controller.phase).toBe("recording");
  });

  it("[12] with no chrome.storage at all it behaves as click", async () => {
    const { s, b, rt } = setupWith(null);
    await settle();
    expect(s.trigger).toBe("click");
    hoverOpen(b);
    expect(sentOfType(rt, "start")).toHaveLength(0);
    pressMic(s);
    expect(sentOfType(rt, "start")).toHaveLength(1);
  });
});

// ---- position tracking ------------------------------------------------------------------

describe("the mic follows the field", () => {
  it("follows a scroll inside a scroll container", () => {
    const s = start();
    const root = mount(`<div id="sc" style="overflow: auto"><input id="f" type="text"></div>`);
    const sc = pick(root, "#sc");
    const field = pick(root, "#f");
    setBox(sc, { left: 0, top: 0, width: 600, height: 400 });
    setBox(field, FIELD);
    field.focus();
    setBox(field, { ...FIELD, top: 20 });
    sc.dispatchEvent(new Event("scroll")); // scroll does not bubble; caught in the capture phase
    nextFrames();
    expect(micPosition(s).top).toBe(20 + (FIELD.height - MIC_SIZE_PX) / 2);
  });

  it("follows the field moving without any event", () => {
    const s = start();
    const field = pick(mount(`<input id="f" type="text">`), "#f");
    setBox(field, FIELD);
    field.focus();
    setBox(field, { ...FIELD, left: 10, top: 300 });
    nextFrames();
    expect(micPosition(s)).toEqual({
      left: 10 + FIELD.width + MIC_GAP_PX,
      top: 300 + (FIELD.height - MIC_SIZE_PX) / 2,
    });
  });

  it("follows a window resize", () => {
    const s = start();
    const field = pick(mount(`<input id="f" type="text">`), "#f");
    setBox(field, FIELD);
    field.focus();
    setBox(field, { ...FIELD, width: 400 });
    window.dispatchEvent(new Event("resize"));
    nextFrames();
    expect(micPosition(s).left).toBe(FIELD.left + 400 + MIC_GAP_PX);
  });

  it("hides when the field is scrolled out of its scroll container, and returns", () => {
    const s = start();
    const root = mount(`<div id="sc" style="overflow: auto"><input id="f" type="text"></div>`);
    const sc = pick(root, "#sc");
    const field = pick(root, "#f");
    setBox(sc, { left: 0, top: 0, width: 600, height: 100 });
    setBox(field, FIELD);
    field.focus();
    expect(s.anchor.isVisible).toBe(true);
    setBox(field, { ...FIELD, top: 200 }); // below the container's visible area
    sc.dispatchEvent(new Event("scroll"));
    nextFrames();
    expect(s.anchor.isVisible).toBe(false);
    expect(box(s).hidden).toBe(true);
    setBox(field, FIELD);
    sc.dispatchEvent(new Event("scroll"));
    nextFrames();
    expect(s.anchor.isVisible).toBe(true);
  });

  it("a textarea gets the mic on its first line, not its vertical middle", () => {
    const s = start();
    const ta = pick(mount(`<textarea id="f"></textarea>`), "#f");
    setBox(ta, { left: 100, top: 50, width: 300, height: 200 });
    ta.focus();
    expect(micPosition(s).top).toBeLessThan(50 + 36);
  });

  it("a field touching the right edge keeps the mic inside the viewport", () => {
    const s = start();
    const field = pick(mount(`<input id="f" type="text">`), "#f");
    const vw = document.documentElement.clientWidth || window.innerWidth;
    setBox(field, { left: vw - 200, top: 50, width: 200, height: 30 });
    field.focus();
    expect(micPosition(s).left + MIC_SIZE_PX).toBeLessThanOrEqual(vw);
  });

  it("detaches when the field is removed from the page", () => {
    const s = start();
    const field = pick(mount(`<input id="f" type="text">`), "#f");
    setBox(field, FIELD);
    field.focus();
    field.remove();
    nextFrames();
    expect(s.anchor.target).toBeNull();
  });
});

// ---- lifecycle / leaks ------------------------------------------------------------------

describe("listeners do not leak", () => {
  it("scroll and resize listeners are removed on blur, across repeated focus cycles", () => {
    const counts = new Map<string, number>();
    const bump = (key: string, d: number) => counts.set(key, (counts.get(key) ?? 0) + d);
    const docAdd = document.addEventListener.bind(document);
    const docRemove = document.removeEventListener.bind(document);
    const winAdd = window.addEventListener.bind(window);
    const winRemove = window.removeEventListener.bind(window);
    vi.spyOn(document, "addEventListener").mockImplementation((type, l, o) => {
      bump(`doc:${type}`, 1);
      docAdd(type, l, o);
    });
    vi.spyOn(document, "removeEventListener").mockImplementation((type, l, o) => {
      bump(`doc:${type}`, -1);
      docRemove(type, l, o);
    });
    vi.spyOn(window, "addEventListener").mockImplementation((type: string, l: EventListenerOrEventListenerObject, o?: boolean | AddEventListenerOptions) => {
      bump(`win:${type}`, 1);
      winAdd(type, l, o);
    });
    vi.spyOn(window, "removeEventListener").mockImplementation((type: string, l: EventListenerOrEventListenerObject, o?: boolean | EventListenerOptions) => {
      bump(`win:${type}`, -1);
      winRemove(type, l, o);
    });

    const s = start();
    const root = mount(`<input id="f" type="text"><button id="other">x</button>`);
    const field = pick(root, "#f");
    const other = pick(root, "#other");
    setBox(field, FIELD);
    for (let i = 0; i < 5; i++) {
      field.focus();
      expect(s.anchor.isVisible).toBe(true);
      blurTo(other);
      expect(s.anchor.target).toBeNull();
    }
    expect(counts.get("doc:scroll")).toBe(0);
    expect(counts.get("win:resize")).toBe(0);
  });

  it("no animation frame keeps running after blur", () => {
    const raf = vi.spyOn(globalThis, "requestAnimationFrame");
    const s = start();
    const root = mount(`<input id="f" type="text"><button id="other">x</button>`);
    const field = pick(root, "#f");
    setBox(field, FIELD);
    field.focus();
    nextFrames(3);
    const other = pick(root, "#other");
    blurTo(other);
    expect(s.anchor.target).toBeNull();
    const callsAfterBlur = raf.mock.calls.length;
    nextFrames(10);
    expect(raf.mock.calls.length).toBe(callsAfterBlur);
  });

  it("stop() removes the host from the page", () => {
    const s = start();
    const field = pick(mount(`<input id="f" type="text">`), "#f");
    setBox(field, FIELD);
    field.focus();
    expect(document.querySelector(HOST_TAG)).not.toBeNull();
    s.stop();
    script = null;
    expect(document.querySelector(HOST_TAG)).toBeNull();
  });
});
