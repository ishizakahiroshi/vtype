import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { startContentScript, type ContentScript } from "../src/content/index";
import { FakeChromeHub, FakeSpeechRecognition, flush } from "./fake-chrome";

// Kept alone in this file on purpose. insert.ts tracks IME composition per document from the
// first call on; any earlier insertAtCursor in the same module instance would install that
// tracking and hide a content script that forgot to start it (C7b 作業内容 8). vitest gives
// each test file its own module instance, so here nothing has touched `document` before.

let script: ContentScript | null = null;

beforeEach(() => {
  vi.useFakeTimers();
  FakeSpeechRecognition.reset();
});

afterEach(() => {
  script?.stop();
  script = null;
  document.body.innerHTML = "";
  vi.useRealTimers();
});

it("a composition begun before the first insert is waited for (tracking starts with the content script)", async () => {
  const hub = new FakeChromeHub();
  hub.startBackground();
  script = startContentScript({ hoverCapable: () => true, runtime: hub.contentRuntime(1, 0), language: "en" });
  const wrap = document.createElement("div");
  wrap.innerHTML = `<input id="f" type="text" value="">`;
  document.body.append(wrap);
  const field = wrap.querySelector("#f") as HTMLInputElement;
  field.focus();
  field.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, composed: true }));

  const mic = script.anchor.root?.querySelector(".record") as HTMLButtonElement;
  mic.click();
  await flush(vi);
  const sr = FakeSpeechRecognition.started();
  sr.fireStart();
  sr.fireResult("after ime", true);
  await flush(vi);
  mic.click(); // stop
  await flush(vi);
  expect(field.value).toBe(""); // not written into the middle of the composition

  field.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, composed: true }));
  await flush(vi, 10);
  expect(field.value).toBe("after ime");
});
