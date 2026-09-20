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

/** Set by traceOrder: records when the content script *asks* for something, not when it arrives. */
let orderSink: string[] | null = null;

function start(tabId = 1, frameId = 0): ContentScript {
  const runtime = hub.contentRuntime(tabId, frameId);
  script = startContentScript({
    hoverCapable: () => true,
    language: "en",
    runtime: {
      onMessage: runtime.onMessage,
      sendMessage: (message) => {
        const type = (message as { type?: string }).type;
        if (type === "stop" && orderSink !== null) orderSink.push("stop");
        return runtime.sendMessage(message);
      },
    },
  });
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
  orderSink = null;
  document.body.innerHTML = "";
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("[1] results are shown in the panel only, never in the field while recording", () => {
  it("interim and final results leave the field untouched", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="HelloWorld">`), "#f");
    field.focus();
    field.setSelectionRange(5, 5); // caret in the middle
    const sr = await beginRecording(s);
    expect(s.anchor.ui?.state).toBe("recording");

    sr.fireResult("hel", false);
    await flush(vi);
    expect(field.value).toBe("HellohelWorld"); // in the field at once, around the existing text
    expect(transcriptText(s)).toBe(""); // the panel stays empty while writing works

    sr.fireResult("hello", false);
    await flush(vi);
    expect(field.value).toBe("HellohelloWorld"); // the previous interim was replaced, not appended

    sr.fireResult("hello there", true);
    await flush(vi);
    expect(field.value).toBe("Hellohello thereWorld");
    expect(field.selectionStart).toBe("Hellohello there".length); // caret after the new text
  });

  it("[C7d 2] interim after a final goes behind the confirmed text", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    const sr = await beginRecording(s);
    sr.fireResult("first part", true);
    await flush(vi);
    sr.fireResult("second", false);
    await flush(vi);
    expect(field.value).toBe("first part second");
    sr.fireResult("seconds later", false);
    await flush(vi);
    expect(field.value).toBe("first part seconds later"); // only the interim tail changed
  });
});

describe("[C7d 3] stopping does not insert the text a second time", () => {
  it("the text stays exactly once, and nothing arrives later", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="HelloWorld">`), "#f");
    field.focus();
    field.setSelectionRange(5, 5);
    const sr = await beginRecording(s);
    sr.fireResult(", dear", true);
    await flush(vi);
    const inputsAfterSpeaking = countInputs(field);
    await pressMic(s); // stop
    expect(field.value).toBe("Hello, dearWorld");
    expect(inputsAfterSpeaking.n).toBe(0); // the stop writes nothing at all
    expect(s.anchor.ui?.state).toBe("idle");
    expect(transcriptText(s)).toBe("");
    expect(s.controller.pendingText).toBe("");
    await flush(vi, STOP_TIMEOUT_MS + 100);
    expect(field.value).toBe("Hello, dearWorld");
  });

  it("text from several recognition cycles accumulates in the field, not in the panel", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    const sr = await beginRecording(s);
    sr.fireResult("first part", true);
    sr.fireEnd(); // Chrome ends after one utterance; the session continues
    await flush(vi);
    expect(field.value).toBe("first part");
    const next = FakeSpeechRecognition.started();
    next.fireStart();
    next.fireResult("second part", true);
    await flush(vi);
    expect(field.value).toBe("first part second part");
    await pressMic(s);
    expect(field.value).toBe("first part second part"); // unchanged by the stop
  });

  it("a late final after the stop replaces the interim in the field instead of adding to it", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    const sr = await beginRecording(s);
    sr.fireResult("こんにち", false);
    await flush(vi);
    expect(field.value).toBe("こんにち");
    await pressMic(s);
    expect(s.anchor.ui?.state).toBe("processing");
    sr.fireResult("こんにちは", true); // late final from the replaced instance (isCurrent: false)
    await flush(vi);
    expect(field.value).toBe("こんにちは");
  });

  it("an empty final removes the interim it belongs to", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="typed">`), "#f");
    field.focus();
    const sr = await beginRecording(s);
    sr.fireResult("noise", false);
    await flush(vi);
    expect(field.value).toBe("typednoise");
    sr.fireResult("", true); // Chrome's empty final during silence
    await flush(vi);
    expect(field.value).toBe("typed");
    await pressMic(s);
    expect(field.value).toBe("typed");
  });
});

describe("[3] the field chosen at start receives the text even if focus moves", () => {
  it("writes into the first field, not the one focused later", async () => {
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
    expect(a.value).toBe("to the first field");
    expect(b.value).toBe("");
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
    expect(a.value).toBe("kept text"); // it was written live while the field still existed
    b.focus();
    await flush(vi, 10);
    a.remove();
    // the next result cannot reach the removed field
    sr.fireResult("lost to the page", true);
    await flush(vi);
    expect(b.value).toBe("");
    expect(transcriptText(s)).toBe("lost to the page");
    expect(s.controller.pendingText).toBe("lost to the page");
    expect(pick(root(s), ".message").hidden).toBe(false);
    expect(pick(root(s), ".message").textContent).toContain("kept here");

    // Stopping cannot place it either: the field it belongs to is gone, so it stays.
    await pressMic(s);
    expect(b.value).toBe("");
    expect(s.controller.pendingText).toBe("lost to the page");

    // The next recording writes it into the new field, ahead of the new words.
    const sr2 = await beginRecording(s); // b has focus now
    sr2.fireResult("and more", true);
    await flush(vi);
    expect(b.value).toBe("lost to the page and more");
    expect(s.controller.pendingText).toBe("");
    expect(transcriptText(s)).toBe("");
  });

  it("[C7d 5] the field is not rewritten while an IME composition runs; the text is kept", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    const sr = await beginRecording(s);
    field.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, composed: true }));
    sr.fireResult("waiting text", true);
    await flush(vi);
    expect(field.value).toBe(""); // nothing written into the middle of the composition

    await flush(vi, COMPOSITION_TIMEOUT_MS + 100); // the composition never ends
    expect(field.value).toBe("");
    expect(s.controller.pendingText).toBe("waiting text");
    expect(pick(root(s), ".message").textContent).toContain("IME");
    field.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, composed: true }));
  });

  it("[C7d 5] a composition that ends in time lets the text through", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    const sr = await beginRecording(s);
    field.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, composed: true }));
    sr.fireResult("after the ime", true);
    await flush(vi);
    expect(field.value).toBe("");
    field.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, composed: true }));
    await flush(vi, 10);
    expect(field.value).toBe("after the ime");
    expect(s.controller.pendingText).toBe("");
  });

  it("[C7d 6] × during recording empties the field and the next text starts from the front", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="old text">`), "#f");
    field.focus();
    field.setSelectionRange(8, 8);
    const sr = await beginRecording(s);
    sr.fireResult("spoken", true);
    await flush(vi);
    expect(field.value).toBe("old textspoken");

    pick<HTMLButtonElement>(root(s), ".clear").click(); // × while still recording
    await flush(vi);
    expect(field.value).toBe("");

    sr.fireResult("after clearing", false);
    await flush(vi);
    expect(field.value).toBe("after clearing");
  });

  // "composition tracking runs from startup" lives in startup.test.ts: it needs a module state
  // in which no earlier test has called insertAtCursor on this document.
});

describe("[5] a start elsewhere ends this session (content side)", () => {
  it("the superseded tab keeps what was already written and says why", async () => {
    const s1 = start(1, 0);
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    const sr = await beginRecording(s1);
    sr.fireResult("unfinished", true);
    await flush(vi);
    expect(field.value).toBe("unfinished");
    // another tab starts (its content script is represented by a direct message)
    hub.contentRuntime(2, 0).onMessage.addListener(() => undefined);
    await hub.contentRuntime(2, 0).sendMessage({ target: "background", type: "start", sessionId: "other" });
    await flush(vi);
    expect(s1.controller.phase).toBe("idle");
    expect(s1.anchor.ui?.state).toBe("idle");
    expect(field.value).toBe("unfinished"); // written once, not doubled
    expect(s1.controller.pendingText).toBe("");
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

describe("[C8 0/0b] the send button interrupts recognition", () => {
  /** Records the order of: the stop reaching the background, the field's input, the submit. */
  function traceOrder(field: Element, form: HTMLFormElement): string[] {
    const order: string[] = [];
    orderSink = order;
    field.addEventListener("input", () => order.push("insert"));
    form.addEventListener("submit", (e) => {
      order.push("submit");
      e.preventDefault();
    });
    vi.spyOn(form, "requestSubmit").mockImplementation(() => {
      form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });
    return order;
  }

  it("[0] stops recognition and then submits; the text is already in the field (C7d)", async () => {
    const s = start();
    const page = mount(`<form id="form"><input id="f" type="search" value=""></form>`);
    const form = pick<HTMLFormElement>(page, "#form");
    const field = pick<HTMLInputElement>(page, "#f");
    field.focus();
    const sr = await beginRecording(s);
    sr.fireResult("weather tomorrow", true);
    await flush(vi);
    expect(field.value).toBe("weather tomorrow"); // written live, before send
    const order = traceOrder(field, form);

    pick<HTMLButtonElement>(root(s), ".send").click();
    await flush(vi);

    expect(order).toEqual(["stop", "submit"]); // no second insert; the stop comes first
    expect(field.value).toBe("weather tomorrow");
    expect(s.controller.phase).toBe("idle");
    expect(s.anchor.ui?.state).toBe("idle");
    expect(sr.stopCalls).toBe(1); // recognition really stopped
    expect(s.controller.pendingText).toBe("");
  });

  it("[0] text kept in the panel is inserted before the submit, in that order", async () => {
    const s = start();
    const page = mount(`<form id="form"><input id="f" type="search" value=""></form>`);
    const form = pick<HTMLFormElement>(page, "#form");
    const field = pick<HTMLInputElement>(page, "#f");
    field.focus();
    const sr = await beginRecording(s);
    // an IME composition blocks the live write, so the text lands in the panel instead
    field.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, composed: true }));
    sr.fireResult("kept then sent", true);
    await flush(vi, COMPOSITION_TIMEOUT_MS + 100);
    expect(s.controller.pendingText).toBe("kept then sent");
    field.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, composed: true }));
    const order = traceOrder(field, form);

    pick<HTMLButtonElement>(root(s), ".send").click();
    await flush(vi);

    expect(order).toEqual(["stop", "insert", "submit"]);
    expect(field.value).toBe("kept then sent");
  });

  it("[0] does not wait for the offscreen grace period: the interim in the field is what is sent", async () => {
    const s = start();
    const page = mount(`<form id="form"><input id="f" type="search" value=""></form>`);
    const form = pick<HTMLFormElement>(page, "#form");
    const field = pick<HTMLInputElement>(page, "#f");
    field.focus();
    const sr = await beginRecording(s);
    sr.fireResult("half a sen", false); // still interim
    await flush(vi);
    const order = traceOrder(field, form);
    pick<HTMLButtonElement>(root(s), ".send").click();
    await flush(vi);
    expect(order).toEqual(["stop", "submit"]); // at once, without waiting 1.5 s
    expect(field.value).toBe("half a sen");
  });

  it("[0b] does not submit when the kept text could not be inserted", async () => {
    const s = start();
    const page = mount(`<form id="form"><input id="f" type="search" value=""></form>`);
    const form = pick<HTMLFormElement>(page, "#form");
    const field = pick<HTMLInputElement>(page, "#f");
    field.focus();
    const sr = await beginRecording(s);
    field.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, composed: true }));
    sr.fireResult("never inserted", true);
    await flush(vi, COMPOSITION_TIMEOUT_MS + 100);
    expect(s.controller.pendingText).toBe("never inserted");
    const order = traceOrder(field, form);

    // the composition is still running when send is pressed
    pick<HTMLButtonElement>(root(s), ".send").click();
    await flush(vi, COMPOSITION_TIMEOUT_MS + 100);

    expect(order).toEqual(["stop"]); // no insert, and above all no submit
    expect(field.value).toBe("");
    expect(s.controller.pendingText).toBe("never inserted");
    field.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, composed: true }));
  });

  it("while idle it inserts what the panel still holds and then submits", async () => {
    const s = start();
    const page = mount(`<form id="form"><input id="f" type="search" value=""></form>`);
    const form = pick<HTMLFormElement>(page, "#form");
    const field = pick<HTMLInputElement>(page, "#f");
    field.focus();
    const sr = await beginRecording(s);
    field.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, composed: true }));
    sr.fireResult("left over", true);
    await flush(vi, COMPOSITION_TIMEOUT_MS + 100);
    field.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, composed: true }));
    await pressMic(s); // stop, but the field is gone from the insert's point of view? no: it inserts
    await flush(vi);
    // the stop already inserted the kept text, so pressing send now only submits
    expect(field.value).toBe("left over");
    expect(s.controller.pendingText).toBe("");
    const order = traceOrder(field, form);
    pick<HTMLButtonElement>(root(s), ".send").click();
    await flush(vi);
    expect(order).toEqual(["submit"]);
  });

  it("while idle with an empty panel it just submits", async () => {
    const s = start();
    const page = mount(`<form id="form"><input id="f" type="search" value="typed by hand"></form>`);
    const form = pick<HTMLFormElement>(page, "#form");
    const field = pick<HTMLInputElement>(page, "#f");
    field.focus();
    const order = traceOrder(field, form);
    pick<HTMLButtonElement>(root(s), ".send").click();
    await flush(vi);
    expect(order).toEqual(["submit"]);
    expect(field.value).toBe("typed by hand");
  });

  it("says so when the page's送信 path is unknown, and keeps the text in the field", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    const sr = await beginRecording(s);
    sr.fireResult("nowhere to send", true);
    await flush(vi);
    pick<HTMLButtonElement>(root(s), ".send").click();
    await flush(vi);
    expect(field.value).toBe("nowhere to send"); // inserted
    expect(pick(root(s), ".message").textContent).toContain("could not tell");
  });
});

describe("[C7c] activity events reach the waveform", () => {
  it("an activity from the offscreen document moves the bars, and results kick them", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    const sr = await beginRecording(s);
    const wave = s.anchor.ui?.waveform;
    if (wave === undefined) throw new Error("no waveform");
    expect(wave.running).toBe(true);

    sr.onsoundstart?.({});
    await flush(vi);
    expect(wave.target).toBe(0.55);
    sr.onspeechstart?.({});
    await flush(vi);
    expect(wave.target).toBe(0.9);
    sr.onspeechend?.({});
    await flush(vi);
    expect(wave.target).toBe(0.25);

    sr.fireResult("growing text", false);
    await flush(vi);
    expect(wave.target).toBeGreaterThanOrEqual(0.85); // the transcript grew

    await pressMic(s); // stop; the offscreen document still waits for the pending interim
    expect(wave.running).toBe(true); // bars stay while the last result is awaited
    await flush(vi, 2000); // the grace period runs out
    expect(s.anchor.ui?.state).toBe("idle");
    expect(wave.running).toBe(false);
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

describe("[C7e] the `hover` setting: a session nobody pressed a button for", () => {
  /** How often the extension has actually started recognition (the microphone was asked). */
  function started(): number {
    return FakeSpeechRecognition.instances.reduce((n, i) => n + i.startCalls, 0);
  }

  it("[1] autoStart runs the same path as the mic: the words land in the field", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    s.controller.autoStart(); // what the panel opening does
    await flush(vi);
    expect(s.controller.phase).toBe("recording");
    expect(s.anchor.ui?.state).toBe("recording");
    const sr = FakeSpeechRecognition.started();
    sr.fireStart();
    await flush(vi);
    sr.fireResult("spoken without pressing", true);
    await flush(vi);
    expect(field.value).toBe("spoken without pressing");
  });

  it("[7] a refused microphone stops the automatic starts, not the mic button", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text">`), "#f");
    field.focus();
    await pressMic(s);
    const sr = FakeSpeechRecognition.started();
    sr.fireError("not-allowed");
    sr.fireEnd();
    await flush(vi);
    expect(s.controller.phase).toBe("idle");
    const afterRefusal = started();

    s.controller.autoStart();
    await flush(vi);
    expect(s.controller.phase).toBe("idle");
    expect(started()).toBe(afterRefusal); // the microphone was not asked again

    await pressMic(s); // pressing still tries
    expect(s.controller.phase).toBe("recording");
    expect(started()).toBe(afterRefusal + 1);
  });

  it("[7] a start that succeeds afterwards allows the automatic starts again", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text">`), "#f");
    field.focus();
    await pressMic(s);
    const refused = FakeSpeechRecognition.started();
    refused.fireError("not-allowed");
    refused.fireEnd();
    await flush(vi);

    // the user allows the microphone and presses the mic, which starts for real
    const sr = await beginRecording(s); // press + started event
    await pressMic(s); // stop
    sr.fireEnd();
    await flush(vi, STOP_TIMEOUT_MS);
    expect(s.controller.phase).toBe("idle");

    const before = started();
    s.controller.autoStart();
    await flush(vi);
    expect(s.controller.phase).toBe("recording");
    expect(started()).toBe(before + 1);
  });

  it("[8] with no usable field nothing is sent and no recognition starts", async () => {
    const s = start();
    const pw = pick<HTMLInputElement>(mount(`<input id="pw" type="password">`), "#pw");
    pw.focus();
    expect(s.anchor.target).toBeNull();
    s.controller.autoStart();
    await flush(vi);
    expect(hub.createDocumentCalls).toHaveLength(0);
    expect(started()).toBe(0);
    expect(s.controller.phase).toBe("idle");
  });
});

describe("silence and stop timeout", () => {
  it("a session that ends in silence leaves the text in the field and says so", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    let sr = await beginRecording(s);
    sr.fireResult("said once", true);
    sr.fireEnd();
    await flush(vi);
    expect(field.value).toBe("said once");
    for (let i = 0; i < 3; i++) {
      sr = FakeSpeechRecognition.started();
      sr.fireStart();
      sr.fireError("no-speech");
      sr.fireEnd();
      await flush(vi);
    }
    expect(s.controller.phase).toBe("idle");
    expect(field.value).toBe("said once"); // written live, not doubled by the ending
    expect(s.controller.pendingText).toBe("");
    expect(pick(root(s), ".message").textContent).toContain("silence");
  });

  it("if the stop cannot be delivered at all, the text in the field stays as it is", async () => {
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
    expect(s.controller.phase).toBe("idle");
  });

  it("if the stop is delivered but never answered, the panel still goes idle after STOP_TIMEOUT_MS", async () => {
    const s = start();
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    field.focus();
    const sr = await beginRecording(s);
    sr.fireResult("rescued late", true);
    await flush(vi);
    hub.backgroundListeners.splice(0, hub.backgroundListeners.length, () => undefined); // silent
    hub.offscreenListeners.length = 0;
    await pressMic(s);
    expect(s.controller.phase).toBe("stopping");
    await flush(vi, STOP_TIMEOUT_MS + 100);
    expect(s.controller.phase).toBe("idle");
    expect(field.value).toBe("rescued late"); // still exactly once
  });
});
