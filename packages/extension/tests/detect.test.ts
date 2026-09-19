import { afterEach, describe, expect, it } from "vitest";
import { TARGET_INPUT_TYPES, isTargetField, resolveTarget } from "../src/content/detect";

function mount(html: string): HTMLElement {
  const wrap = document.createElement("div");
  wrap.innerHTML = html;
  document.body.append(wrap);
  return wrap;
}

function pick(root: ParentNode, selector: string): Element {
  const el = root.querySelector(selector);
  if (el === null) throw new Error(`fixture is missing ${selector}`);
  return el;
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("resolveTarget: targets", () => {
  it.each(["text", "search", "email", "url", "tel"])("input type=%s is a target", (type) => {
    const root = mount(`<input type="${type}">`);
    const input = pick(root, "input");
    expect(resolveTarget(input)).toBe(input);
    expect(isTargetField(input)).toBe(true);
  });

  it("input without a type attribute (defaults to text) is a target", () => {
    const input = pick(mount(`<input>`), "input");
    expect(resolveTarget(input)).toBe(input);
  });

  it("input with an unknown type (falls back to text) is a target", () => {
    const input = pick(mount(`<input type="no-such-type">`), "input");
    expect(resolveTarget(input)).toBe(input);
  });

  it("textarea is a target", () => {
    const ta = pick(mount(`<textarea></textarea>`), "textarea");
    expect(resolveTarget(ta)).toBe(ta);
  });

  it.each(["", "true", "plaintext-only", "TRUE"])('contenteditable="%s" is a target', (value) => {
    const div = pick(mount(`<div contenteditable="${value}">hello</div>`), "div");
    expect(resolveTarget(div)).toBe(div);
  });

  it("a node inside a contenteditable region resolves to the editing host", () => {
    const root = mount(`<div id="host" contenteditable="true"><p><b id="inner">x</b></p></div>`);
    expect(resolveTarget(pick(root, "#inner"))).toBe(pick(root, "#host"));
  });

  it("an invalid contenteditable value inherits from the parent", () => {
    const root = mount(`<div id="host" contenteditable="true"><span id="s" contenteditable="bogus">x</span></div>`);
    expect(resolveTarget(pick(root, "#s"))).toBe(pick(root, "#host"));
  });
});

describe("resolveTarget: password is excluded structurally", () => {
  it("password is not in the input-type allowlist", () => {
    expect(TARGET_INPUT_TYPES.has("password")).toBe(false);
  });

  it("input type=password is not a target", () => {
    const input = pick(mount(`<input type="password">`), "input");
    expect(resolveTarget(input)).toBeNull();
    expect(isTargetField(input)).toBe(false);
  });

  it("input type=PASSWORD (any case) is not a target", () => {
    const input = pick(mount(`<input type="PASSWORD">`), "input");
    expect(resolveTarget(input)).toBeNull();
  });

  it("a password input inside a contenteditable region is still not a target", () => {
    const root = mount(`<div contenteditable="true"><input id="pw" type="password"></div>`);
    expect(resolveTarget(pick(root, "#pw"))).toBeNull();
  });

  it.each(["current-password", "new-password", "username current-password"])(
    'a revealed password field (type=text, autocomplete="%s") is not a target',
    (ac) => {
      const input = pick(mount(`<input type="text" autocomplete="${ac}">`), "input");
      expect(resolveTarget(input)).toBeNull();
    },
  );
});

describe("resolveTarget: other exclusions", () => {
  it.each(["hidden", "number", "checkbox", "date", "submit", "file"])("input type=%s is not a target", (type) => {
    const input = pick(mount(`<input type="${type}">`), "input");
    expect(resolveTarget(input)).toBeNull();
  });

  it.each([
    [`<input type="text" readonly>`, "input"],
    [`<input type="text" disabled>`, "input"],
    [`<textarea readonly></textarea>`, "textarea"],
    [`<textarea disabled></textarea>`, "textarea"],
  ])("%s is not a target", (html, selector) => {
    expect(resolveTarget(pick(mount(html), selector))).toBeNull();
  });

  it("contenteditable=false inside an editable region is not a target", () => {
    const root = mount(`<div contenteditable="true"><div id="off" contenteditable="false">x</div></div>`);
    expect(resolveTarget(pick(root, "#off"))).toBeNull();
  });

  it("plain elements and null are not targets", () => {
    const root = mount(`<div id="d">x</div><button>b</button>`);
    expect(resolveTarget(pick(root, "#d"))).toBeNull();
    expect(resolveTarget(pick(root, "button"))).toBeNull();
    expect(resolveTarget(null)).toBeNull();
  });
});
