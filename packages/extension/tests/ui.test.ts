import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import css from "../src/ui/styles.css?raw";
import { HOST_TAG, OPEN_DELAY_MS } from "../src/content/anchor";
import { startContentScript, type ContentScript } from "../src/content/index";
import { translate } from "../src/shared/i18n";
import { SMOOTHING, START_TARGET, TRANSCRIPT_TARGET, createWaveform } from "../src/ui/waveform";
// The waveform's own source, to prove no microphone API is used (plan C7c 検証 4).
import waveformSource from "../src/ui/waveform.ts?raw";

// happy-dom does no layout: boxes come from a table (stubbed on the prototype, never on the
// field). It does apply stylesheets with shadow-DOM scoping, which the hostile-CSS tests use.

const boxes = new Map<Element, { left: number; top: number; width: number; height: number }>();

let script: ContentScript | null = null;

function mount(html: string): HTMLElement {
  const wrap = document.createElement("div");
  wrap.innerHTML = html;
  document.body.append(wrap);
  return wrap;
}

function pick<T extends Element = HTMLElement>(root: ParentNode, selector: string): T {
  const el = root.querySelector(selector);
  if (el === null) throw new Error(`missing ${selector}`);
  return el as T;
}

/** Start the content script, focus a synthetic field and return the shadow root. */
function focusField(html = `<input id="f" type="text" value="">`): { s: ContentScript; field: HTMLElement; root: ShadowRoot } {
  script = startContentScript({ hoverCapable: () => true });
  const field = pick(mount(html), "#f");
  boxes.set(field, { left: 100, top: 50, width: 200, height: 30 });
  field.focus();
  const root = script.anchor.root;
  if (root === null) throw new Error("no shadow root");
  return { s: script, field, root };
}

function openByHover(s: ContentScript, root: ShadowRoot): void {
  pick(root, ".box").dispatchEvent(new PointerEvent("pointerenter", { pointerType: "mouse" }));
  vi.advanceTimersByTime(OPEN_DELAY_MS);
  expect(s.anchor.isOpen).toBe(true);
}

function typeInto(field: HTMLInputElement, value: string): void {
  // what the user typing looks like to the page: the value changes, then `input`
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(field, value);
  field.dispatchEvent(new InputEvent("input", { bubbles: true, composed: true, inputType: "insertText", data: value }));
}

function addPageStyle(text: string): HTMLStyleElement {
  const style = document.createElement("style");
  style.textContent = text;
  document.head.append(style);
  return style;
}

beforeEach(() => {
  vi.useFakeTimers();
  boxes.clear();
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    const b = boxes.get(this) ?? { left: 0, top: 0, width: 0, height: 0 };
    return { ...b, x: b.left, y: b.top, right: b.left + b.width, bottom: b.top + b.height, toJSON: () => b } as DOMRect;
  });
});

afterEach(() => {
  script?.stop();
  script = null;
  document.body.innerHTML = "";
  document.head.innerHTML = "";
  for (const stale of document.querySelectorAll(HOST_TAG)) stale.remove();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

// ---- [検証 1] the panel and its three buttons render ------------------------------------

describe("panel and buttons render", () => {
  it("the panel holds × / send / mic in many-ai-cli's order, as real buttons", () => {
    const { s, root } = focusField();
    const panel = s.anchor.panel;
    expect(panel?.classList.contains("panel")).toBe(true);
    const buttons = [...pick(root, ".panel .toolbar").children];
    expect(buttons.map((b) => b.localName)).toEqual(["button", "button", "button"]);
    expect(buttons.map((b) => b.classList[1])).toEqual(["clear", "send", "record"]);
    for (const b of buttons) {
      expect((b as HTMLButtonElement).type).toBe("button"); // never submits a page form
      expect(b.getAttribute("aria-label")).toBeTruthy();
      expect(b.querySelector("svg")).not.toBeNull();
    }
  });

  it("the thin mic beside the field is a labelled button that reports expanded state", () => {
    const { s, root } = focusField();
    const mic = pick(root, ".box > .mic");
    expect(mic.localName).toBe("button");
    expect(mic.getAttribute("aria-label")).toBeTruthy();
    expect(mic.getAttribute("aria-expanded")).toBe("false");
    openByHover(s, root);
    expect(mic.getAttribute("aria-expanded")).toBe("true");
    expect(s.anchor.panel?.hidden).toBe(false);
  });

  it("send and mic are keyboard-focusable; the thin mic is not in the tab order", () => {
    const { root } = focusField();
    expect(pick<HTMLButtonElement>(root, ".send").tabIndex).toBe(0);
    expect(pick<HTMLButtonElement>(root, ".record").tabIndex).toBe(0);
    expect(pick<HTMLButtonElement>(root, ".box > .mic").tabIndex).toBe(-1);
  });

  it("uses class names only: no id anywhere in the shadow tree", () => {
    const { root } = focusField();
    expect(root.querySelectorAll("[id]").length).toBe(0);
    expect(root.querySelectorAll("*").length).toBeGreaterThan(10);
  });

  it("labels follow the browser language", () => {
    expect(translate("micSend", "ja-JP")).toBe("送信");
    expect(translate("micSend", "en-US")).toBe("Send");
    expect(translate("micSend", undefined)).toBe("Send");
  });
});

// ---- × only when the field has text -----------------------------------------------------

describe("× appears only when the field has text", () => {
  it("follows the field's input events", () => {
    const { root, field } = focusField();
    const clear = pick<HTMLButtonElement>(root, ".clear");
    expect(clear.classList.contains("has-text")).toBe(false);
    expect(clear.disabled).toBe(true);
    expect(clear.getAttribute("aria-hidden")).toBe("true");
    typeInto(field as HTMLInputElement, "hello");
    expect(clear.classList.contains("has-text")).toBe(true);
    expect(clear.disabled).toBe(false);
    expect(clear.tabIndex).toBe(0);
    typeInto(field as HTMLInputElement, "");
    expect(clear.classList.contains("has-text")).toBe(false);
  });

  it("a field focused with text already shows ×", () => {
    const { root } = focusField(`<input id="f" type="text" value="prefilled">`);
    expect(pick(root, ".clear").classList.contains("has-text")).toBe(true);
  });

  it("text set by the page without an input event is picked up when the panel opens", () => {
    const { s, root, field } = focusField();
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(field, "set by page");
    expect(pick(root, ".clear").classList.contains("has-text")).toBe(false);
    openByHover(s, root);
    expect(pick(root, ".clear").classList.contains("has-text")).toBe(true);
  });

  it("contenteditable: × follows its text", () => {
    const { root, field } = focusField(`<div id="f" contenteditable="true" tabindex="0"></div>`);
    const clear = pick(root, ".clear");
    expect(clear.classList.contains("has-text")).toBe(false);
    field.textContent = "typed";
    field.dispatchEvent(new InputEvent("input", { bubbles: true, composed: true }));
    expect(clear.classList.contains("has-text")).toBe(true);
  });

  it("pressing × clears through C6 clearField: empty field, input event, × hides, onClear runs", () => {
    const { s, root, field } = focusField(`<input id="f" type="text" value="some text">`);
    const seen: string[] = [];
    field.addEventListener("input", () => seen.push((field as HTMLInputElement).value));
    const onClear = vi.fn();
    const ui = s.anchor.ui;
    if (ui === null) throw new Error("no ui");
    ui.onClear = onClear;
    pick<HTMLButtonElement>(root, ".clear").click();
    expect((field as HTMLInputElement).value).toBe("");
    expect(seen).toEqual([""]);
    expect(onClear).toHaveBeenCalledTimes(1);
    expect(pick(root, ".clear").classList.contains("has-text")).toBe(false);
  });

  it("stops listening to the field after blur", () => {
    const root0 = mount(`<button id="other">x</button>`);
    const { s, root, field } = focusField();
    pick(root0, "#other").focus();
    vi.advanceTimersByTime(1);
    expect(s.anchor.target).toBeNull();
    expect(s.anchor.ui?.field).toBeNull();
    typeInto(field as HTMLInputElement, "later");
    expect(pick(root, ".clear").classList.contains("has-text")).toBe(false);
  });
});

// ---- hooks and state (for C7b / C8) -----------------------------------------------------

describe("hooks and state", () => {
  it("send and mic call their hooks, and do nothing while unwired", () => {
    const { s, root } = focusField();
    const ui = s.anchor.ui;
    if (ui === null) throw new Error("no ui");
    expect(() => pick<HTMLButtonElement>(root, ".send").click()).not.toThrow();
    expect(() => pick<HTMLButtonElement>(root, ".record").click()).not.toThrow();
    const onSend = vi.fn();
    const onMic = vi.fn();
    ui.onSend = onSend;
    ui.onMic = onMic;
    pick<HTMLButtonElement>(root, ".send").click();
    pick<HTMLButtonElement>(root, ".record").click();
    pick<HTMLButtonElement>(root, ".record").click();
    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onMic).toHaveBeenCalledTimes(2);
  });

  it("pressing any panel button does not take focus from the field", () => {
    const { root } = focusField();
    for (const selector of [".clear", ".send", ".record", ".box > .mic"]) {
      const down = new MouseEvent("mousedown", { bubbles: true, cancelable: true, composed: true });
      pick(root, selector).dispatchEvent(down);
      expect(down.defaultPrevented, selector).toBe(true);
    }
  });

  it("setState renders idle / recording / processing on the mic button, the thin mic and the panel", () => {
    const { s, root } = focusField();
    const ui = s.anchor.ui;
    if (ui === null) throw new Error("no ui");
    const record = pick(root, ".record");
    const mic = pick(root, ".box > .mic");
    const panel = pick(root, ".panel");

    ui.setState("recording");
    expect(ui.state).toBe("recording");
    expect(record.classList.contains("recording")).toBe(true);
    expect(mic.classList.contains("recording")).toBe(true);
    expect(record.getAttribute("aria-pressed")).toBe("true");
    expect(record.getAttribute("aria-label")).toBe(translate("micStop", navigator.language));

    ui.setState("processing");
    expect(record.classList.contains("recording")).toBe(false);
    expect(panel.classList.contains("processing")).toBe(true);
    expect(mic.classList.contains("processing")).toBe(true);

    ui.setState("idle");
    for (const el of [record, mic, panel]) {
      expect(el.classList.contains("recording")).toBe(false);
      expect(el.classList.contains("processing")).toBe(false);
    }
    expect(record.getAttribute("aria-pressed")).toBe("false");
  });

  it("transcript and message lines show only when they have text", () => {
    const { s, root } = focusField();
    const ui = s.anchor.ui;
    if (ui === null) throw new Error("no ui");
    const transcript = pick(root, ".transcript");
    const message = pick(root, ".message");
    expect(transcript.hidden).toBe(true);
    expect(message.hidden).toBe(true);
    ui.setTranscript("confirmed ", "still guessing");
    expect(transcript.hidden).toBe(false);
    expect(pick(root, ".transcript .final").textContent).toBe("confirmed ");
    expect(pick(root, ".transcript .interim").textContent).toBe("still guessing");
    ui.setMessage("synthetic error");
    expect(message.hidden).toBe(false);
    ui.setTranscript("", "");
    ui.setMessage(null);
    expect(transcript.hidden).toBe(true);
    expect(message.hidden).toBe(true);
  });

  it("rendering states never touches the field's DOM", async () => {
    const { s, root, field } = focusField(`<input id="f" type="text" value="keep" class="page">`);
    const before = field.outerHTML;
    const records: MutationRecord[] = [];
    const mo = new MutationObserver((l) => records.push(...l));
    mo.observe(document, { subtree: true, attributes: true, childList: true, characterData: true });
    openByHover(s, root);
    s.anchor.ui?.setState("recording");
    s.anchor.ui?.setTranscript("a", "b");
    s.anchor.ui?.setMessage("m");
    s.anchor.ui?.setState("processing");
    s.anchor.ui?.setState("idle");
    await Promise.resolve();
    mo.disconnect();
    records.push(...mo.takeRecords());
    expect(records).toEqual([]);
    expect(field.outerHTML).toBe(before);
  });
});

// ---- stylesheet: animations, reduced motion ---------------------------------------------

describe("stylesheet", () => {
  it("recording pulses every 1.2 s and processing every 1.4 s, like many-ai-cli", () => {
    expect(css).toMatch(/\.record\.recording,\s*\.mic\.recording\s*\{[^}]*animation:\s*vt-voice-pulse 1\.2s/);
    expect(css).toMatch(/\.panel\.processing\s*\{[^}]*animation:\s*vt-processing-pulse 1400ms/);
  });

  it("prefers-reduced-motion turns every animation off", () => {
    const code = css.replace(/\/\*[\s\S]*?\*\//g, "");
    const at = code.indexOf("@media (prefers-reduced-motion: reduce)");
    expect(at).toBeGreaterThan(0);
    const block = code.slice(at);
    const animated = [...code.slice(0, at).matchAll(/([^{}]+)\{[^}]*\banimation:\s*vt-/g)].flatMap((m) =>
      (m[1] ?? "").split(",").map((sel) => sel.trim()),
    );
    expect(animated.length).toBeGreaterThanOrEqual(3);
    for (const selector of animated) {
      expect(block, selector).toContain(selector);
    }
    expect(block).toMatch(/animation:\s*none/);
  });

  it("is the only stylesheet, and it lives inside the shadow root", () => {
    const outsideBefore = document.querySelectorAll("style, link[rel=stylesheet]").length;
    const { root } = focusField();
    expect(root.querySelectorAll("style").length).toBe(1);
    expect(root.querySelector("style")?.textContent).toBe(css);
    expect(document.querySelectorAll("style, link[rel=stylesheet]").length).toBe(outsideBefore);
    expect(document.head.innerHTML).not.toContain("vt-voice-pulse");
  });
});

// ---- [検証 2 / 3] hostile page CSS ------------------------------------------------------

const HOSTILE = `
* { box-sizing: content-box !important }
* { font-size: 24px !important }
div { display: flex !important }
`;

describe("hostile page CSS (plan C7 検証 2)", () => {
  it("the three hostile rules do not change our elements' computed style", () => {
    addPageStyle(HOSTILE);
    const { s, root } = focusField();
    openByHover(s, root);
    const box = pick(root, ".box");
    const panel = pick(root, ".panel");
    const toolbar = pick(root, ".toolbar");
    const buttons = [...root.querySelectorAll(".btn, .mic")];
    expect(buttons.length).toBe(4);

    // box-sizing: ours everywhere
    for (const el of [box, panel, toolbar, ...buttons]) {
      expect(getComputedStyle(el).boxSizing).toBe("border-box");
    }
    // div { display: flex }: the box keeps display:block; the panel is our column flex
    expect(getComputedStyle(box).display).toBe("block");
    expect(getComputedStyle(panel).display).toBe("flex");
    expect(getComputedStyle(panel).flexDirection).toBe("column");
    const transcript = pick(root, ".transcript");
    s.anchor.ui?.setTranscript("x");
    expect(getComputedStyle(transcript).display).toBe("block");
    // font-size: px from our stylesheet, not 24px
    expect(getComputedStyle(box).fontSize).toBe("13px");
    for (const b of root.querySelectorAll(".btn")) expect(getComputedStyle(b).fontSize).toBe("13px");
    // our explicit sizes survive
    expect(getComputedStyle(pick(root, ".send")).width).toBe("32px");
    expect(getComputedStyle(pick(root, ".record")).width).toBe("28px");
    expect(getComputedStyle(pick(root, ".clear")).width).toBe("26px");
  });

  it("the same elements compute the same with and without the hostile rules", () => {
    const snapshot = (root: ShadowRoot) =>
      [...root.querySelectorAll(".box, .panel, .toolbar, .btn, .mic")].map((el) => {
        const c = getComputedStyle(el);
        return [el.className, c.boxSizing, c.display, c.fontSize, c.width, c.height].join("|");
      });
    const first = focusField();
    openByHover(first.s, first.root);
    const clean = snapshot(first.root);
    script?.stop();
    script = null;
    document.body.innerHTML = "";

    addPageStyle(HOSTILE);
    const second = focusField();
    openByHover(second.s, second.root);
    expect(snapshot(second.root)).toEqual(clean);
  });

  it("page rules aimed at our class names and host do not apply", () => {
    addPageStyle(`
      .panel, .box, .btn, .send, .mic { display: none !important; width: 999px !important }
      vtype-root { display: none !important; position: static !important }
    `);
    const { s, root } = focusField();
    openByHover(s, root);
    expect(getComputedStyle(pick(root, ".panel")).display).toBe("flex");
    expect(getComputedStyle(pick(root, ".send")).width).toBe("32px");
    const host = s.anchor.host;
    if (host === null) throw new Error("no host");
    expect(host.style.getPropertyPriority("display")).toBe("important");
    expect(host.style.getPropertyValue("display")).toBe("block");
    expect(css).toMatch(/:host\s*\{[^}]*display:\s*block !important/);
  });
});

// ---- [C7c] the waveform ------------------------------------------------------------------

describe("[C7c] the waveform appears while recording", () => {
  it("is a canvas inside the shadow root, hidden until recording and hidden again at idle", () => {
    const { s, root } = focusField();
    const canvas = pick<HTMLCanvasElement>(root, ".panel .waveform");
    expect(canvas.localName).toBe("canvas");
    expect(canvas.hidden).toBe(true);
    expect(s.anchor.ui?.waveform.running).toBe(false);

    s.anchor.ui?.setState("recording");
    expect(canvas.hidden).toBe(false);
    expect(s.anchor.ui?.waveform.running).toBe(true);
    expect(s.anchor.ui?.waveform.target).toBe(START_TARGET);

    s.anchor.ui?.setState("processing"); // still waiting for the last result: bars stay
    expect(canvas.hidden).toBe(false);

    s.anchor.ui?.setState("idle");
    expect(canvas.hidden).toBe(true);
    expect(s.anchor.ui?.waveform.running).toBe(false);
  });

  it("keeps the same intensity model as many-ai-cli for each activity kind", () => {
    const { s } = focusField();
    const ui = s.anchor.ui;
    if (ui === null) throw new Error("no ui");
    ui.setState("recording");
    const targets: Record<string, number> = {};
    for (const kind of ["soundstart", "speechstart", "speechend", "audioend"] as const) {
      ui.setActivity(kind);
      targets[kind] = ui.waveform.target;
    }
    expect(targets).toEqual({ soundstart: 0.55, speechstart: 0.9, speechend: 0.25, audioend: 0.03 });
    // the order relations the plan asks for
    expect(targets.audioend).toBeLessThan(targets.speechend as number);
    expect(targets.speechend).toBeLessThan(targets.soundstart as number);
    expect(targets.soundstart).toBeLessThan(targets.speechstart as number);
  });

  it("ignores the kinds many-ai-cli does not use", () => {
    const { s } = focusField();
    const ui = s.anchor.ui;
    if (ui === null) throw new Error("no ui");
    ui.setState("recording");
    ui.setActivity("speechstart");
    const before = ui.waveform.target;
    for (const kind of ["audiostart", "soundend", "nomatch"] as const) ui.setActivity(kind);
    expect(ui.waveform.target).toBe(before);
  });

  it("a transcript that grew lifts the target and a final resets the measure", () => {
    const { s } = focusField();
    const ui = s.anchor.ui;
    if (ui === null) throw new Error("no ui");
    ui.setState("recording");
    expect(ui.waveform.target).toBe(START_TARGET);
    ui.waveform.noteTranscript("hello", false);
    expect(ui.waveform.target).toBeGreaterThanOrEqual(TRANSCRIPT_TARGET);
    ui.setActivity("speechend");
    ui.waveform.noteTranscript("hi", false); // shorter than the last interim: no lift
    expect(ui.waveform.target).toBe(0.25);
  });

  it("the drawn intensity follows the target, it does not jump", () => {
    const wave = createWaveform({ doc: document, now: () => 0, reducedMotion: () => true });
    wave.start();
    wave.setActivity("speechstart"); // target 0.9
    const seen: number[] = [];
    for (let i = 0; i < 3; i++) {
      wave.step();
      seen.push(Number(wave.intensity.toFixed(4)));
    }
    expect(seen[0]).toBeCloseTo(0.9 * SMOOTHING, 4);
    expect(seen[0]).toBeLessThan(seen[1] as number);
    expect(seen[1]).toBeLessThan(seen[2] as number);
    expect(seen[2]).toBeLessThan(0.9);
  });

  it("nothing is animated when the viewer asked for reduced motion", () => {
    const raf = vi.spyOn(globalThis, "requestAnimationFrame");
    const still = createWaveform({ doc: document, reducedMotion: () => true });
    still.start();
    expect(still.running).toBe(true);
    expect(raf).not.toHaveBeenCalled();

    raf.mockClear();
    const moving = createWaveform({ doc: document, reducedMotion: () => false });
    moving.start();
    expect(raf).toHaveBeenCalledTimes(1);
    moving.stop();
  });

  it("the bars are never driven by the microphone: no audio API is used", () => {
    // Comments explain why those APIs are avoided, so only the code lines are checked.
    const code = waveformSource
      .replace(/\/\*[\s\S]*?\*\//g, "")
      .split("\n")
      .filter((line) => !line.trim().startsWith("//"))
      .join("\n");
    expect(code).not.toMatch(/getUserMedia|AudioContext|AnalyserNode|createAnalyser|mediaDevices/);
    expect(code).toContain("Math.sin"); // the shape comes from sine waves plus noise
  });
});

describe("our styles do not leak out of the shadow root (plan C7 検証 3)", () => {
  it("page elements with our class names are not styled by us", () => {
    const page = mount(
      `<div class="panel" id="p1">page</div><button class="btn send" id="p2">page</button><div class="box" id="p3"></div><button class="mic recording" id="p4">m</button>`,
    );
    focusField();
    const p1 = getComputedStyle(pick(page, "#p1"));
    const p2 = getComputedStyle(pick(page, "#p2"));
    const p3 = getComputedStyle(pick(page, "#p3"));
    const p4 = getComputedStyle(pick(page, "#p4"));
    expect(p1.position).not.toBe("absolute");
    expect(p1.width).not.toBe("240px");
    expect(p2.width).not.toBe("32px");
    expect(p2.borderRadius).not.toBe("8px");
    expect(p3.position).not.toBe("fixed");
    expect(p4.animationName ?? "").not.toContain("vt-voice-pulse");
    expect(p4.width).not.toBe("24px");
  });
});
