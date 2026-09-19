import { afterEach, describe, expect, it, vi } from "vitest";
import { clearField, hasText } from "../src/content/clear";

// Plan C6 検証方法 [6]: × empties the field and fires `input`.

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

function countInputs(target: EventTarget): { count: number; types: Array<string | undefined> } {
  const seen = { count: 0, types: [] as Array<string | undefined> };
  target.addEventListener("input", (e) => {
    seen.count++;
    seen.types.push((e as InputEvent).inputType);
  });
  return seen;
}

function stubExecCommand(impl: ((command: string) => boolean) | undefined): ReturnType<typeof vi.fn> | undefined {
  const fn = impl === undefined ? undefined : vi.fn(impl);
  Object.defineProperty(document, "execCommand", { configurable: true, writable: true, value: fn });
  return fn;
}

afterEach(() => {
  delete (document as { execCommand?: unknown }).execCommand;
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

describe("[6] × empties the field and fires input", () => {
  it("input: native setter, value empty, one input event", () => {
    const input = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="some text">`), "#f");
    const setter = vi.spyOn(HTMLInputElement.prototype, "value", "set");
    const seen = countInputs(input);
    expect(hasText(input)).toBe(true);
    expect(clearField(input)).toBe(true);
    expect(input.value).toBe("");
    expect(setter).toHaveBeenCalledWith("");
    expect(seen).toEqual({ count: 1, types: ["deleteContent"] });
    expect(hasText(input)).toBe(false);
  });

  it("textarea: value empty, one input event", () => {
    const ta = pick<HTMLTextAreaElement>(mount(`<textarea id="f">a\nb</textarea>`), "#f");
    const seen = countInputs(ta);
    expect(clearField(ta)).toBe(true);
    expect(ta.value).toBe("");
    expect(seen.count).toBe(1);
  });

  it("contenteditable without execCommand: Range path empties it and fires input", () => {
    stubExecCommand(undefined);
    const host = pick(mount(`<div id="f" contenteditable="true" tabindex="0"><p>one</p><p>two</p></div>`), "#f");
    const seen = countInputs(host);
    expect(clearField(host)).toBe(true);
    expect(host.textContent).toBe("");
    expect(seen).toEqual({ count: 1, types: ["deleteContent"] });
  });

  it("contenteditable with execCommand('delete') true: the editor does it, no Range deletion", () => {
    const exec = stubExecCommand((command) => {
      if (command !== "delete") return false;
      // Stand-in editor: deletes what the selection covers, which vtype set to the whole host.
      const sel = document.getSelection();
      const host = document.querySelector("#f");
      if (sel === null || host === null || sel.rangeCount === 0) return false;
      const range = sel.getRangeAt(0);
      if (range.startContainer !== host || range.endOffset !== host.childNodes.length) return false;
      host.textContent = "";
      return true;
    });
    const deleteContents = vi.spyOn(Range.prototype, "deleteContents");
    const host = pick(mount(`<div id="f" contenteditable="true" tabindex="0">text</div>`), "#f");
    const seen = countInputs(host);
    expect(clearField(host)).toBe(true);
    expect(exec).toHaveBeenCalledTimes(1);
    expect(exec).toHaveBeenCalledWith("delete", false, undefined);
    expect(deleteContents).not.toHaveBeenCalled();
    expect(seen.count).toBe(0); // the editor fires its own input
    expect(document.activeElement).toBe(host);
    expect(host.textContent).toBe("");
  });

  it("contenteditable with execCommand false: falls back to Range", () => {
    const exec = stubExecCommand(() => false);
    const host = pick(mount(`<div id="f" contenteditable="true" tabindex="0">text</div>`), "#f");
    const seen = countInputs(host);
    expect(clearField(host)).toBe(true);
    expect(exec).toHaveBeenCalledTimes(1);
    expect(host.textContent).toBe("");
    expect(seen.count).toBe(1);
  });
});

describe("× is not used on an empty field", () => {
  it.each([
    [`<input id="f" type="text" value="">`],
    [`<textarea id="f"></textarea>`],
    [`<div id="f" contenteditable="true" tabindex="0"><br></div>`],
    [`<div id="f" contenteditable="true" tabindex="0">​</div>`],
  ])("%s: returns false and fires nothing", (html) => {
    const exec = stubExecCommand(() => true);
    const field = pick(mount(html), "#f");
    const seen = countInputs(field);
    expect(hasText(field)).toBe(false);
    expect(clearField(field)).toBe(false);
    expect(seen.count).toBe(0);
    expect(exec).not.toHaveBeenCalled();
  });

  it("password fields are refused even with text", () => {
    const pw = pick<HTMLInputElement>(mount(`<input id="f" type="password" value="secret-synthetic">`), "#f");
    const seen = countInputs(pw);
    expect(clearField(pw)).toBe(false);
    expect(pw.value).toBe("secret-synthetic");
    expect(seen.count).toBe(0);
  });

  it("readonly fields are refused", () => {
    const input = pick<HTMLInputElement>(mount(`<input id="f" type="text" readonly value="ro">`), "#f");
    expect(clearField(input)).toBe(false);
    expect(input.value).toBe("ro");
  });
});
