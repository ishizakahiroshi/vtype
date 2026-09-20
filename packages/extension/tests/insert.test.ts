import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  COMPOSITION_TIMEOUT_MS,
  beginLiveInsert,
  insertAtCursor,
  isComposing,
  startCompositionTracking,
} from "../src/content/insert";

// Plan C6 検証方法 numbers are in the describe titles ([1] .. [8]).

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

function recordInputs(target: EventTarget): Array<{ inputType: string | undefined; data: string | null | undefined }> {
  const seen: Array<{ inputType: string | undefined; data: string | null | undefined }> = [];
  target.addEventListener("input", (e) => {
    const ie = e as InputEvent;
    seen.push({ inputType: ie.inputType, data: ie.data });
  });
  return seen;
}

/** Put a collapsed caret at `offset` of the first text node of `host`. */
function caretIn(host: HTMLElement, offset: number): void {
  const text = host.firstChild;
  if (text === null) throw new Error("fixture has no text node");
  const range = document.createRange();
  range.setStart(text, offset);
  range.collapse(true);
  const sel = document.getSelection();
  sel?.removeAllRanges();
  sel?.addRange(range);
}

type ExecStub = ReturnType<typeof vi.fn>;

/** Replace document.execCommand for one test (happy-dom's own behaviour is not relied on). */
function stubExecCommand(impl: ((command: string, ui?: boolean, value?: string) => boolean) | undefined): ExecStub | undefined {
  const fn = impl === undefined ? undefined : vi.fn(impl);
  Object.defineProperty(document, "execCommand", { configurable: true, writable: true, value: fn });
  return fn;
}

/**
 * A stand-in for an editor honouring insertText: it edits the text node at the caret itself
 * (insertData), never through Range.insertNode, so the Range path can be told apart.
 */
function editorLikeInsertText(command: string, _ui?: boolean, value?: string): boolean {
  if (command !== "insertText" || value === undefined) return false;
  const sel = document.getSelection();
  if (sel === null || sel.rangeCount === 0) return false;
  // Chrome's insertText replaces whatever is selected; a collapsed caret deletes nothing.
  const selected = sel.getRangeAt(0);
  if (!selected.collapsed) {
    selected.deleteContents();
    sel.removeAllRanges();
    sel.addRange(selected);
  }
  const node = sel.anchorNode;
  if (node === null || node === undefined) return false;
  const offset = sel.anchorOffset;
  if (node.nodeType === Node.TEXT_NODE) {
    (node as Text).insertData(offset, value);
    sel.collapse(node, offset + value.length);
    return true;
  }
  // caret between child nodes of an element
  const text = document.createTextNode(value);
  node.insertBefore(text, node.childNodes[offset] ?? null);
  sel.collapse(text, value.length);
  return true;
}

beforeEach(() => {
  startCompositionTracking(document);
});

afterEach(() => {
  delete (document as { execCommand?: unknown }).execCommand;
  document.body.innerHTML = "";
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("[1] input: insert at the caret keeps the text before and after", () => {
  it("inserts in the middle and moves the caret to the end of the inserted text", async () => {
    const input = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="HelloWorld">`), "#f");
    input.setSelectionRange(5, 5);
    const events = recordInputs(input);
    const result = await insertAtCursor(input, ", ");
    expect(result).toEqual({ ok: true, path: "native-setter", waitedForComposition: false });
    expect(input.value).toBe("Hello, World");
    expect(input.selectionStart).toBe(7);
    expect(input.selectionEnd).toBe(7);
    expect(events).toEqual([{ inputType: "insertText", data: ", " }]);
  });

  it("replaces a selected range, keeping both sides", async () => {
    const input = pick<HTMLInputElement>(mount(`<input id="f" type="search" value="one two three">`), "#f");
    input.setSelectionRange(4, 7);
    await insertAtCursor(input, "2");
    expect(input.value).toBe("one 2 three");
    expect(input.selectionStart).toBe(5);
  });

  it("works without focus: uses the stored selection and does not focus the field", async () => {
    const root = mount(`<input id="f" type="text" value="abcdef"><button id="b">b</button>`);
    const input = pick<HTMLInputElement>(root, "#f");
    input.focus();
    input.setSelectionRange(3, 3);
    pick(root, "#b").focus();
    expect(document.activeElement).not.toBe(input);
    await insertAtCursor(input, "-");
    expect(input.value).toBe("abc-def");
    expect(document.activeElement).not.toBe(input);
  });
});

describe("[2] textarea: insert at the caret keeps the text before and after", () => {
  it("inserts in the middle of multi-line text", async () => {
    const ta = pick<HTMLTextAreaElement>(mount(`<textarea id="f"></textarea>`), "#f");
    ta.value = "line one\nline two";
    ta.setSelectionRange(9, 9); // start of "line two"
    const events = recordInputs(ta);
    const result = await insertAtCursor(ta, "new ");
    expect(result).toMatchObject({ ok: true, path: "native-setter" });
    expect(ta.value).toBe("line one\nnew line two");
    expect(ta.selectionStart).toBe(13);
    expect(events).toHaveLength(1);
  });
});

describe("[3] contenteditable: insert at the caret keeps the text before and after", () => {
  it("inserts in the middle (browser without execCommand)", async () => {
    stubExecCommand(undefined);
    const host = pick(mount(`<div id="f" contenteditable="true" tabindex="0">HelloWorld</div>`), "#f");
    host.focus();
    caretIn(host, 5);
    const events = recordInputs(host);
    const result = await insertAtCursor(host, ", ");
    expect(result).toMatchObject({ ok: true });
    expect(host.textContent).toBe("Hello, World");
    expect(events).toEqual([{ inputType: "insertText", data: ", " }]);
    // caret right after the inserted text: typing "!" there lands before "World"
    await insertAtCursor(host, "!");
    expect(host.textContent).toBe("Hello, !World");
  });

  it("not focused and selection elsewhere: focuses the host and appends at the end", async () => {
    stubExecCommand(undefined);
    const root = mount(`<div id="f" contenteditable="true" tabindex="0">draft</div><p id="p">page text</p>`);
    const host = pick(root, "#f");
    const p = pick(root, "#p");
    const range = document.createRange();
    range.selectNodeContents(p);
    document.getSelection()?.removeAllRanges();
    document.getSelection()?.addRange(range);
    await insertAtCursor(host, " more");
    expect(document.activeElement).toBe(host);
    expect(host.textContent).toBe("draft more");
    expect(p.textContent).toBe("page text");
  });
});

describe("[4] native setter + input event (React-like controlled input)", () => {
  it("bypasses an instance-level value override and the input listener sees the new value", async () => {
    const input = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="ab">`), "#f");
    input.setSelectionRange(1, 1);

    // What React's value tracker does: shadow `value` on the instance to remember the last
    // value it saw, and ignore an input event whose value equals that remembered value.
    const native = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
    if (native?.get === undefined || native.set === undefined) throw new Error("no native accessor");
    let tracked = native.get.call(input) as string;
    const instanceWrites: string[] = [];
    Object.defineProperty(input, "value", {
      configurable: true,
      get() {
        return native.get!.call(this);
      },
      set(v: string) {
        instanceWrites.push(v);
        tracked = v;
        native.set!.call(this, v);
      },
    });
    const prototypeSetter = vi.spyOn(HTMLInputElement.prototype, "value", "set");

    const observed: Array<{ value: string; frameworkWouldFire: boolean }> = [];
    input.addEventListener("input", () => {
      const now = native.get!.call(input) as string;
      observed.push({ value: now, frameworkWouldFire: now !== tracked });
    });

    await insertAtCursor(input, "X");

    expect(instanceWrites).toEqual([]); // the instance override was not used
    expect(prototypeSetter).toHaveBeenCalledTimes(1);
    expect(prototypeSetter).toHaveBeenCalledWith("aXb");
    expect(observed).toEqual([{ value: "aXb", frameworkWouldFire: true }]);
  });
});

describe("[5] IME: nothing is inserted between compositionstart and compositionend", () => {
  function composition(target: Element, type: "compositionstart" | "compositionend"): void {
    target.dispatchEvent(new CompositionEvent(type, { bubbles: true, composed: true, data: "" }));
  }

  it("waits for compositionend, then inserts queued requests once each, in order", async () => {
    vi.useFakeTimers();
    const input = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="ab">`), "#f");
    input.setSelectionRange(1, 1);
    const events = recordInputs(input);
    composition(input, "compositionstart");
    expect(isComposing(input)).toBe(true);

    const first = insertAtCursor(input, "X");
    const second = insertAtCursor(input, "Y");
    await vi.advanceTimersByTimeAsync(2_000);
    expect(input.value).toBe("ab");
    expect(events).toHaveLength(0);

    composition(input, "compositionend");
    await vi.advanceTimersByTimeAsync(1);
    expect(await first).toEqual({ ok: true, path: "native-setter", waitedForComposition: true });
    expect(await second).toMatchObject({ ok: true });
    expect(input.value).toBe("aXYb");
    expect(events).toHaveLength(2);
  });

  it("contenteditable also waits", async () => {
    vi.useFakeTimers();
    stubExecCommand(undefined);
    const host = pick(mount(`<div id="f" contenteditable="true" tabindex="0">ab</div>`), "#f");
    host.focus();
    caretIn(host, 1);
    composition(host, "compositionstart");
    const pending = insertAtCursor(host, "X");
    await vi.advanceTimersByTimeAsync(2_000);
    expect(host.textContent).toBe("ab");
    composition(host, "compositionend");
    await vi.advanceTimersByTimeAsync(1);
    expect(await pending).toMatchObject({ ok: true, waitedForComposition: true });
    expect(host.textContent).toBe("aXb");
  });

  it("gives up without inserting if compositionend never comes", async () => {
    vi.useFakeTimers();
    const input = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="ab">`), "#f");
    composition(input, "compositionstart");
    const pending = insertAtCursor(input, "X", { compositionTimeoutMs: 100 });
    await vi.advanceTimersByTimeAsync(100);
    expect(await pending).toEqual({ ok: false, reason: "composition-timeout" });
    expect(input.value).toBe("ab");
    composition(input, "compositionend");
    await vi.advanceTimersByTimeAsync(1);
    expect(input.value).toBe("ab"); // not inserted late either
    expect(COMPOSITION_TIMEOUT_MS).toBeGreaterThan(1_000);
  });

  it("a composition in another field does not hold this one", async () => {
    const root = mount(`<input id="a" type="text" value=""><input id="b" type="text" value="">`);
    const a = pick<HTMLInputElement>(root, "#a");
    const b = pick<HTMLInputElement>(root, "#b");
    composition(b, "compositionstart");
    expect(await insertAtCursor(a, "X")).toMatchObject({ ok: true, waitedForComposition: false });
    expect(a.value).toBe("X");
    composition(b, "compositionend");
  });
});

describe("[7] contenteditable: execCommand('insertText') returning true is used, Range is not", () => {
  it("goes through execCommand only", async () => {
    const exec = stubExecCommand(editorLikeInsertText);
    const insertNode = vi.spyOn(Range.prototype, "insertNode");
    const deleteContents = vi.spyOn(Range.prototype, "deleteContents");
    const host = pick(mount(`<div id="f" contenteditable="true" tabindex="0">HelloWorld</div>`), "#f");
    host.focus();
    caretIn(host, 5);
    const events = recordInputs(host);

    const result = await insertAtCursor(host, ", ");

    expect(result).toEqual({ ok: true, path: "execCommand", waitedForComposition: false });
    expect(exec).toHaveBeenCalledTimes(1);
    expect(exec).toHaveBeenCalledWith("insertText", false, ", ");
    expect(insertNode).not.toHaveBeenCalled();
    expect(deleteContents).not.toHaveBeenCalled();
    expect(events).toHaveLength(0); // the editor's own pipeline fires input, not vtype
    expect(host.textContent).toBe("Hello, World"); // inserted exactly once
  });

  it("focuses an unfocused host before calling execCommand", async () => {
    let activeAtCall: Element | null = null;
    stubExecCommand((command, ui, value) => {
      activeAtCall = document.activeElement;
      return editorLikeInsertText(command, ui, value);
    });
    const root = mount(`<div id="f" contenteditable="true" tabindex="0">draft</div><button id="b">b</button>`);
    const host = pick(root, "#f");
    pick(root, "#b").focus();
    const insertNode = vi.spyOn(Range.prototype, "insertNode");
    expect(await insertAtCursor(host, "!")).toMatchObject({ ok: true, path: "execCommand" });
    expect(insertNode).not.toHaveBeenCalled();
    expect(activeAtCall).toBe(host);
    expect(host.textContent).toBe("draft!"); // caret placed at the end first
  });
});

describe("[8] contenteditable: execCommand missing or false falls back to Range", () => {
  it.each([
    ["missing", undefined],
    ["returns false", (): boolean => false],
    [
      "throws",
      (): boolean => {
        throw new Error("synthetic failure");
      },
    ],
  ] as Array<[string, (() => boolean) | undefined]>)("execCommand %s", async (_label, impl) => {
    const exec = stubExecCommand(impl);
    const insertNode = vi.spyOn(Range.prototype, "insertNode");
    const host = pick(mount(`<div id="f" contenteditable="true" tabindex="0">HelloWorld</div>`), "#f");
    host.focus();
    caretIn(host, 5);
    const events = recordInputs(host);

    const result = await insertAtCursor(host, ", ");

    expect(result).toEqual({ ok: true, path: "range", waitedForComposition: false });
    if (exec !== undefined) expect(exec).toHaveBeenCalledTimes(1);
    expect(insertNode).toHaveBeenCalledTimes(1);
    expect(host.textContent).toBe("Hello, World");
    expect(events).toEqual([{ inputType: "insertText", data: ", " }]);
  });

  it("keeps markup on both sides of the caret", async () => {
    stubExecCommand(undefined);
    const host = pick(mount(`<div id="f" contenteditable="true" tabindex="0"><b>bold</b>plain<i>it</i></div>`), "#f");
    host.focus();
    const plain = host.childNodes[1];
    if (plain === undefined) throw new Error("fixture");
    const range = document.createRange();
    range.setStart(plain, 2);
    range.collapse(true);
    document.getSelection()?.removeAllRanges();
    document.getSelection()?.addRange(range);
    await insertAtCursor(host, "X");
    expect(host.textContent).toBe("boldplXain" + "it");
    expect(host.querySelector("b")?.textContent).toBe("bold");
    expect(host.querySelector("i")?.textContent).toBe("it");
  });
});

// ---- [C7d] live insertion ----------------------------------------------------------------

describe("[C7d 1] live insert into input / textarea", () => {
  it("writes at the caret and replaces the previous interim, keeping both sides", async () => {
    const input = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="HelloWorld">`), "#f");
    input.setSelectionRange(5, 5);
    const events = recordInputs(input);
    const live = beginLiveInsert(input);

    expect(await live.update("goo")).toEqual({ ok: true });
    expect(input.value).toBe("HellogooWorld");
    expect(await live.update("good day")).toEqual({ ok: true });
    expect(input.value).toBe("Hellogood dayWorld"); // replaced, not appended
    expect(input.selectionStart).toBe("Hellogood day".length);

    expect(await live.commit("good day")).toEqual({ ok: true });
    expect(await live.update(" to you")).toEqual({ ok: true });
    expect(input.value).toBe("Hellogood day to youWorld"); // the confirmed part stayed
    expect(live.committedText).toBe("good day");
    expect(events.length).toBeGreaterThan(0);
    expect(events.every((e) => e.inputType === "insertText")).toBe(true);
  });

  it("an empty interim removes what was shown", async () => {
    const ta = pick<HTMLTextAreaElement>(mount(`<textarea id="f">keep</textarea>`), "#f");
    ta.setSelectionRange(4, 4);
    const live = beginLiveInsert(ta);
    await live.update("noise");
    expect(ta.value).toBe("keepnoise");
    await live.update("");
    expect(ta.value).toBe("keep");
  });

  it("resync takes the field as it is now (× emptied it)", async () => {
    const input = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    const live = beginLiveInsert(input);
    await live.update("before");
    expect(input.value).toBe("before");
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, "");
    live.resync();
    await live.update("after");
    expect(input.value).toBe("after");
  });

  it("an edit made by someone else is not overwritten; the text follows it", async () => {
    const input = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    const live = beginLiveInsert(input);
    await live.commit("spoken");
    // the user types at the end themselves
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, "spoken typed");
    input.setSelectionRange("spoken typed".length, "spoken typed".length);
    await live.update("more");
    expect(input.value).toBe("spoken typedmore");
  });

  it("stops when the field is gone or is no longer a target", async () => {
    const input = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    const live = beginLiveInsert(input);
    input.readOnly = true;
    expect(await live.update("x")).toEqual({ ok: false, reason: "not-a-target" });
    expect(live.active).toBe(false);

    const other = pick<HTMLInputElement>(mount(`<input id="g" type="text" value="">`), "#g");
    const live2 = beginLiveInsert(other);
    other.remove();
    expect(await live2.update("x")).toEqual({ ok: false, reason: "disconnected" });
  });

  it("end() stops further writing", async () => {
    const input = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    const live = beginLiveInsert(input);
    live.end();
    expect(await live.update("x")).toEqual({ ok: false, reason: "ended" });
    expect(input.value).toBe("");
  });
});

describe("[C7d 4] live insert into contenteditable goes through execCommand", () => {
  it("replaces the previous interim with insertText and never rebuilds nodes", async () => {
    const exec = stubExecCommand(editorLikeInsertText);
    const insertNode = vi.spyOn(Range.prototype, "insertNode");
    const host = pick(mount(`<div id="f" contenteditable="true" tabindex="0"></div>`), "#f");
    host.focus();
    const live = beginLiveInsert(host);

    expect(await live.update("こん")).toEqual({ ok: true });
    expect(host.textContent).toBe("こん");
    expect(await live.update("こんにちは")).toEqual({ ok: true });
    expect(host.textContent).toBe("こんにちは"); // replaced in place, not stacked
    expect(await live.commit("こんにちは")).toEqual({ ok: true });
    expect(await live.update("みなさん")).toEqual({ ok: true });
    expect(host.textContent).toBe("こんにちはみなさん");

    expect(insertNode).not.toHaveBeenCalled(); // no DOM surgery: the editor did the writing
    expect(exec?.mock.calls.map((c) => c[0])).toEqual(["insertText", "insertText", "insertText", "insertText"]);
  });

  it("keeps the text that was already in the editor", async () => {
    stubExecCommand(editorLikeInsertText);
    const host = pick(mount(`<div id="f" contenteditable="true" tabindex="0">draft </div>`), "#f");
    host.focus();
    const sel = document.getSelection();
    const range = document.createRange();
    range.selectNodeContents(host);
    range.collapse(false);
    sel?.removeAllRanges();
    sel?.addRange(range);
    const live = beginLiveInsert(host);
    await live.update("spoken");
    expect(host.textContent).toBe("draft spoken");
    await live.update("spoken words");
    expect(host.textContent).toBe("draft spoken words");
  });

  it("falls back to a Range when the browser has no execCommand", async () => {
    stubExecCommand(undefined);
    const insertNode = vi.spyOn(Range.prototype, "insertNode");
    const host = pick(mount(`<div id="f" contenteditable="true" tabindex="0"></div>`), "#f");
    host.focus();
    const live = beginLiveInsert(host);
    await live.update("first");
    expect(host.textContent).toBe("first");
    await live.update("first and more");
    expect(host.textContent).toBe("first and more");
    expect(insertNode).toHaveBeenCalled();
  });
});

describe("[C7d 5] live insert waits for the IME", () => {
  it("writes nothing while composing and writes the latest text afterwards", async () => {
    vi.useFakeTimers();
    const input = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    startCompositionTracking(document);
    const live = beginLiveInsert(input);
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, composed: true }));
    const first = live.update("early");
    const second = live.update("early words");
    await vi.advanceTimersByTimeAsync(500);
    expect(input.value).toBe("");
    input.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, composed: true }));
    await vi.advanceTimersByTimeAsync(5);
    expect(await first).toEqual({ ok: true });
    expect(await second).toEqual({ ok: true });
    expect(input.value).toBe("early words"); // the latest state, written once
  });

  it("gives up if the composition never ends", async () => {
    vi.useFakeTimers();
    const input = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    startCompositionTracking(document);
    const live = beginLiveInsert(input, { compositionTimeoutMs: 100 });
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, composed: true }));
    const pending = live.update("never");
    await vi.advanceTimersByTimeAsync(150);
    expect(await pending).toEqual({ ok: false, reason: "composition-timeout" });
    expect(input.value).toBe("");
    input.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, composed: true }));
  });
});

describe("refusals (password stays excluded)", () => {
  it("input type=password is refused and left untouched, with no event", async () => {
    const pw = pick<HTMLInputElement>(mount(`<input id="f" type="password" value="secret-synthetic">`), "#f");
    const events = recordInputs(pw);
    expect(await insertAtCursor(pw, "X")).toEqual({ ok: false, reason: "not-a-target" });
    expect(pw.value).toBe("secret-synthetic");
    expect(events).toHaveLength(0);
  });

  it("a field turned into a password field while waiting for IME is refused", async () => {
    vi.useFakeTimers();
    const input = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, composed: true }));
    const pending = insertAtCursor(input, "X");
    input.type = "password";
    input.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, composed: true }));
    await vi.advanceTimersByTimeAsync(1);
    expect(await pending).toEqual({ ok: false, reason: "not-a-target" });
    expect(input.value).toBe("");
  });

  it.each([`<input id="f" type="text" readonly value="ro">`, `<textarea id="f" disabled>ro</textarea>`])(
    "%s is refused",
    async (html) => {
      const field = pick<HTMLInputElement>(mount(html), "#f");
      expect(await insertAtCursor(field, "X")).toEqual({ ok: false, reason: "not-a-target" });
      expect(field.value).toBe("ro");
    },
  );

  it("a removed field and empty text are refused", async () => {
    const input = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="">`), "#f");
    expect(await insertAtCursor(input, "")).toEqual({ ok: false, reason: "empty-text" });
    input.remove();
    expect(await insertAtCursor(input, "X")).toEqual({ ok: false, reason: "disconnected" });
  });

  it("input type=email (no selection API) appends at the end", async () => {
    const input = pick<HTMLInputElement>(mount(`<input id="f" type="email" value="user@">`), "#f");
    const result = await insertAtCursor(input, "example.com");
    expect(result).toMatchObject({ ok: true, path: "native-setter" });
    expect(input.value).toBe("user@example.com");
  });
});
