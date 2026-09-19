import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CLOSE_DELAY_MS, HOST_TAG, MIC_GAP_PX, MIC_SIZE_PX, OPEN_DELAY_MS } from "../src/content/anchor";
import { startContentScript, type ContentScript } from "../src/content/index";

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

  it("a mouse click does not open the panel on a hover-capable device", () => {
    const { s, mic } = setup();
    pointer(mic, "pointerup", "mouse");
    expect(s.anchor.isOpen).toBe(false);
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

  it("a touch tap toggles the panel immediately", () => {
    const { s, mic } = setup();
    pointer(mic, "pointerup", "touch");
    expect(s.anchor.isOpen).toBe(true);
    pointer(mic, "pointerup", "touch");
    expect(s.anchor.isOpen).toBe(false);
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
