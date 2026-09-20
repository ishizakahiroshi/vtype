import { afterEach, describe, expect, it, vi } from "vitest";
import {
  MIN_FIELD_HEIGHT_PX,
  MIN_FIELD_WIDTH_PX,
  TARGET_INPUT_TYPES,
  isFieldOnScreen,
  isTargetField,
  resolveTarget,
  visibleTargetFields,
} from "../src/content/detect";

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
  vi.restoreAllMocks();
});

// ---- C7g: finding the fields to put a mic on --------------------------------------------
// happy-dom does no layout, so each element's box comes from this table.

interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

const ROOMY: Box = { left: 10, top: 10, width: 300, height: 30 };

function layout(boxes: Map<Element, Box>): void {
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    const b = boxes.get(this) ?? { left: 0, top: 0, width: 0, height: 0 };
    return {
      x: b.left,
      y: b.top,
      left: b.left,
      top: b.top,
      width: b.width,
      height: b.height,
      right: b.left + b.width,
      bottom: b.top + b.height,
      toJSON: () => b,
    } as DOMRect;
  });
}

/** Give every element matching `selector` the same box. */
function boxAll(selector: string, box: Box = ROOMY, extra: Array<[Element, Box]> = []): void {
  const boxes = new Map<Element, Box>(extra);
  for (const el of document.querySelectorAll(selector)) if (!boxes.has(el)) boxes.set(el, box);
  layout(boxes);
}

describe("[C7g] visibleTargetFields", () => {
  it("finds the target fields in document order", () => {
    const root = mount(`<input id="a" type="text"><textarea id="b"></textarea><div id="c" contenteditable="true"></div>`);
    boxAll("input, textarea, div");
    expect(visibleTargetFields(document, 10)).toEqual([pick(root, "#a"), pick(root, "#b"), pick(root, "#c")]);
  });

  it("leaves out what resolveTarget refuses", () => {
    mount(
      `<input id="a" type="text"><input type="password"><input type="text" readonly><input type="text" disabled><input type="number"><input type="text" autocomplete="current-password">`,
    );
    boxAll("input");
    expect(visibleTargetFields(document, 10)).toEqual([pick(document, "#a")]);
  });

  it("leaves out a field that is too small to hang a mic on", () => {
    const root = mount(`<input id="a" type="text"><input id="narrow" type="text"><input id="flat" type="text">`);
    boxAll("input", ROOMY, [
      [pick(root, "#narrow"), { ...ROOMY, width: MIN_FIELD_WIDTH_PX - 1 }],
      [pick(root, "#flat"), { ...ROOMY, height: MIN_FIELD_HEIGHT_PX - 1 }],
    ]);
    expect(visibleTargetFields(document, 10)).toEqual([pick(root, "#a")]);
  });

  it("leaves out a field that is not in the window, and takes it back when it is", () => {
    const root = mount(`<input id="a" type="text"><input id="below" type="text">`);
    const below = pick(root, "#below");
    const vh = document.documentElement.clientHeight || window.innerHeight;
    boxAll("input", ROOMY, [[below, { ...ROOMY, top: vh + 100 }]]);
    expect(visibleTargetFields(document, 10)).toEqual([pick(root, "#a")]);
    expect(isFieldOnScreen(below, document)).toBe(false);

    boxAll("input");
    expect(isFieldOnScreen(below, document)).toBe(true);
    expect(visibleTargetFields(document, 10)).toHaveLength(2);
  });

  it("stops at the limit, keeping the first ones on the page", () => {
    const root = mount(Array.from({ length: 6 }, (_, i) => `<input id="f${i}" type="text">`).join(""));
    boxAll("input");
    expect(visibleTargetFields(document, 4)).toEqual([
      pick(root, "#f0"),
      pick(root, "#f1"),
      pick(root, "#f2"),
      pick(root, "#f3"),
    ]);
    expect(visibleTargetFields(document, 0)).toEqual([]);
  });

  it("reports an editable region once, not once per element inside it", () => {
    const root = mount(`<div id="host" contenteditable="true"><p id="inner" contenteditable="true">text</p></div>`);
    boxAll("div, p");
    expect(visibleTargetFields(document, 10)).toEqual([pick(root, "#host")]);
  });
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
