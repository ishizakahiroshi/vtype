import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { startContentScript, type ContentScript } from "../src/content/index";
import { STOP_TIMEOUT_MS } from "../src/content/controller";
import { COMPOSITION_TIMEOUT_MS } from "../src/content/insert";
import { FakeChromeHub, FakeSpeechRecognition, flush } from "./fake-chrome";

// End to end inside one test process: content script (real) -> fake Chrome bus -> background
// (real) -> offscreen (real) -> vtype-core (real) -> fake SpeechRecognition.
// Plan C7b 検証方法: [1] .. [6] in the describe titles.

let hub: FakeChromeHub;
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

function start(tabId = 1, frameId = 0): ContentScript {
  script = startContentScript({ hoverCapable: () => true, runtime: hub.contentRuntime(tabId, frameId), language: "en" });
  return script;
}

function root(s: ContentScript): ShadowRoot {
  if (s.anchor.root === null) throw new Error("no shadow root");
  return s.anchor.root;
}

async function pressMic(s: ContentScript): Promise<void> {
  pick<HTMLButtonElement>(root(s), ".record").click();
  await flush(vi);
}

/** Press the mic and let the recognizer start. */
async function beginRecording(s: ContentScript): Promise<FakeSpeechRecognition> {
  await pressMic(s);
  const sr = FakeSpeechRecognition.started();
  sr.fireStart();
  await flush(vi);
  return sr;
}

function countInputs(field: Element): { n: number } {
  const c = { n: 0 };
  field.addEventListener("input", () => c.n++);
  return c;
}

function transcriptText(s: ContentScript): string {
  return pick(root(s), ".transcript").textContent ?? "";
}

beforeEach(() => {
  vi.useFakeTimers();
  FakeSpeechRecognition.reset();
  hub = new FakeChromeHub();
  hub.startBackground();
});

afterEach(() => {
  script?.stop();
  script = null;
  document.body.innerHTML = "";
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("[1] results are shown in the panel only, never in the field while recording", () => {
  it("interim and final results leave the field untouched", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="draft">`), "#f");
    field.focus();
    const inputs = countInputs(field);
    const sr = await beginRecording(s);
    expect(s.anchor.ui?.state).toBe("recording");
    sr.fireResult("hello", false);
    await flush(vi);
    expect(field.value).toBe("draft");
    expect(transcriptText(s)).toBe("hello");
    sr.fireResult("hello world", true);
    await flush(vi);
    expect(field.value).toBe("draft");
    expect(inputs.n).toBe(0);
    expect(transcriptText(s)).toBe("hello world");
  });
});

describe("[2] stopping inserts the confirmed text at the remembered caret, once", () => {
  it("inserts at the caret and clears the panel", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="HelloWorld">`), "#f");
    field.focus();
    field.setSelectionRange(5, 5);
    const inputs = countInputs(field);
    const sr = await beginRecording(s);
    sr.fireResult(", dear", true);
    await flush(vi);
    await pressMic(s); // stop
    expect(field.value).toBe("Hello, dearWorld"); // Chrome's segment, inserted as is at the caret
    expect(inputs.n).toBe(1);
    expect(s.anchor.ui?.state).toBe("idle");
    expect(transcriptText(s)).toBe("");
    expect(s.controller.pendingText).toBe("");
    // nothing more arrives later (no second insert on a stray ended / the stop timeout)
    await flush(vi, STOP_TIMEOUT_MS + 100);
    expect(inputs.n).toBe(1);
  });

  it("joins segments of several recognition cycles and inserts them together", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    const sr = await beginRecording(s);
    sr.fireResult("first part", true);
    sr.fireEnd(); // Chrome ends after one utterance; the session continues
    await flush(vi);
    const next = FakeSpeechRecognition.started();
    next.fireStart();
    next.fireResult("second part", true);
    await flush(vi);
    expect(field.value).toBe("");
    await pressMic(s);
    expect(field.value).toBe("first part second part");
  });

  it("an interim result still pending at stop becomes final and is inserted", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    const sr = await beginRecording(s);
    sr.fireResult("こんにち", false);
    await flush(vi);
    await pressMic(s);
    expect(s.anchor.ui?.state).toBe("processing");
    sr.fireResult("こんにちは", true); // late final from the replaced instance (isCurrent: false)
    await flush(vi);
    expect(field.value).toBe("こんにちは");
  });

  it("an empty final result is ignored", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    const inputs = countInputs(field);
    const sr = await beginRecording(s);
    sr.fireResult("", true);
    await flush(vi);
    await pressMic(s);
    expect(inputs.n).toBe(0);
    expect(field.value).toBe("");
  });
});

describe("[3] the field chosen at start receives the text even if focus moves", () => {
  it("inserts into the first field, not the one focused at stop", async () => {
    const s = start();
    const page = mount(`<input id="a" type="text" value=""><textarea id="b"></textarea>`);
    const a = pick<HTMLInputElement>(page, "#a");
    const b = pick<HTMLTextAreaElement>(page, "#b");
    a.focus();
    const sr = await beginRecording(s);
    b.focus();
    await flush(vi, 10);
    expect(s.anchor.target).toBe(b);
    sr.fireResult("to the first field", true);
    await flush(vi);
    await pressMic(s);
    expect(a.value).toBe("to the first field");
    expect(b.value).toBe("");
  });
});

describe("[4] a field that disappeared gets nothing; the text stays in the panel", () => {
  it("keeps the text and says why; the next recording carries it on", async () => {
    const s = start();
    const page = mount(`<input id="a" type="text" value=""><input id="b" type="text" value="">`);
    const a = pick<HTMLInputElement>(page, "#a");
    const b = pick<HTMLInputElement>(page, "#b");
    a.focus();
    const sr = await beginRecording(s);
    sr.fireResult("kept text", true);
    await flush(vi);
    b.focus();
    await flush(vi, 10);
    a.remove();
    await pressMic(s);
    expect(a.value).toBe("");
    expect(b.value).toBe("");
    expect(transcriptText(s)).toBe("kept text");
    expect(s.controller.pendingText).toBe("kept text");
    expect(pick(root(s), ".message").hidden).toBe(false);
    expect(pick(root(s), ".message").textContent).toContain("kept here");

    const sr2 = await beginRecording(s); // b is focused now
    sr2.fireResult("and more", true);
    await flush(vi);
    await pressMic(s);
    expect(b.value).toBe("kept text and more");
    expect(s.controller.pendingText).toBe("");
  });

  it("insertAtCursor ok:false (IME never finished) also keeps the text", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    const sr = await beginRecording(s);
    sr.fireResult("waiting text", true);
    await flush(vi);
    field.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, composed: true }));
    await pressMic(s);
    expect(field.value).toBe("");
    await flush(vi, COMPOSITION_TIMEOUT_MS + 100);
    expect(field.value).toBe("");
    expect(s.controller.pendingText).toBe("waiting text");
    expect(pick(root(s), ".message").textContent).toContain("IME");
    field.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, composed: true }));
  });

  // "composition tracking runs from startup" lives in startup.test.ts: it needs a module state
  // in which no earlier test has called insertAtCursor on this document.
});

describe("[5] a start elsewhere ends this session (content side)", () => {
  it("the superseded tab keeps its text, does not insert, and says why", async () => {
    const s1 = start(1, 0);
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    const sr = await beginRecording(s1);
    sr.fireResult("unfinished", true);
    await flush(vi);
    // another tab starts (its content script is represented by a direct message)
    hub.contentRuntime(2, 0).onMessage.addListener(() => undefined);
    await hub.contentRuntime(2, 0).sendMessage({ target: "background", type: "start", sessionId: "other" });
    await flush(vi);
    expect(s1.controller.phase).toBe("idle");
    expect(s1.anchor.ui?.state).toBe("idle");
    expect(field.value).toBe("");
    expect(s1.controller.pendingText).toBe("unfinished");
    expect(pick(root(s1), ".message").textContent).toContain("somewhere else");
  });
});

describe("[6] a password field cannot start a recording", () => {
  it("nothing is sent and no recognition starts", async () => {
    const s = start();
    const pw = pick<HTMLInputElement>(mount(`<input id="pw" type="password">`), "#pw");
    pw.focus();
    expect(s.anchor.target).toBeNull();
    s.controller.toggle(); // what the mic would do
    await flush(vi);
    expect(hub.createDocumentCalls).toHaveLength(0);
    expect(FakeSpeechRecognition.instances.filter((i) => i.startCalls > 0)).toHaveLength(0);
    expect(s.controller.phase).toBe("idle");
  });

  it("a field that became a password field after focus is refused at start", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text">`), "#f");
    field.focus();
    const mic = pick<HTMLButtonElement>(root(s), ".record");
    // the page flips the type; the MutationObserver would detach, but press before it runs
    field.type = "password";
    mic.click();
    await flush(vi);
    expect(hub.createDocumentCalls).toHaveLength(0);
    expect(s.controller.phase).toBe("idle");
  });
});

describe("permission path (plan C7b C2)", () => {
  it("not-allowed shows a message with an action that opens the permission page", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text">`), "#f");
    field.focus();
    await pressMic(s);
    const sr = FakeSpeechRecognition.started();
    sr.fireError("not-allowed"); // before onstart, as the C1 spike observed
    sr.fireEnd();
    await flush(vi);
    expect(s.controller.phase).toBe("idle");
    expect(s.anchor.ui?.state).toBe("idle");
    const message = pick(root(s), ".message");
    expect(message.textContent).toContain("not allowed");
    const action = pick<HTMLButtonElement>(message, ".message-action");
    action.click();
    await flush(vi);
    expect(hub.createdTabs).toEqual(["chrome-extension://synthetic-id/permission.html"]);
  });

  it("without an extension runtime the mic only explains", async () => {
    script = startContentScript({ hoverCapable: () => true, runtime: null, language: "en" });
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text">`), "#f");
    field.focus();
    await pressMic(script);
    expect(script.controller.phase).toBe("idle");
    expect(pick(root(script), ".message").textContent).toContain("not available");
  });
});

describe("silence and stop timeout", () => {
  it("a session that ends in silence keeps its text and does not insert", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    let sr = await beginRecording(s);
    sr.fireResult("said once", true);
    sr.fireEnd();
    await flush(vi);
    for (let i = 0; i < 3; i++) {
      sr = FakeSpeechRecognition.started();
      sr.fireStart();
      sr.fireError("no-speech");
      sr.fireEnd();
      await flush(vi);
    }
    expect(s.controller.phase).toBe("idle");
    expect(field.value).toBe("");
    expect(s.controller.pendingText).toBe("said once");
    expect(pick(root(s), ".message").textContent).toContain("silence");
  });

  it("if the stop cannot be delivered at all, the text is inserted right away", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    const sr = await beginRecording(s);
    sr.fireResult("rescued", true);
    await flush(vi);
    hub.backgroundListeners.length = 0; // nothing listens: sendMessage rejects
    hub.offscreenListeners.length = 0;
    await pressMic(s);
    expect(field.value).toBe("rescued");
  });

  it("if the stop is delivered but never answered, the text is inserted after STOP_TIMEOUT_MS", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    const sr = await beginRecording(s);
    sr.fireResult("rescued late", true);
    await flush(vi);
    hub.backgroundListeners.splice(0, hub.backgroundListeners.length, () => undefined); // silent
    hub.offscreenListeners.length = 0;
    await pressMic(s);
    await flush(vi, STOP_TIMEOUT_MS - 500);
    expect(field.value).toBe("");
    await flush(vi, 1000);
    expect(field.value).toBe("rescued late");
  });
});
