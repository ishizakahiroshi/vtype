import { afterEach, describe, expect, it, vi } from "vitest";
import { submitFrom } from "../src/content/submit";

// Plan C8 検証方法 [1] .. [5]. Nothing here touches a real site: every form and handler is
// synthetic and lives in this test's own document.

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

afterEach(() => {
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

describe("[1][2] a field inside a form", () => {
  it("submits through requestSubmit and reports it", () => {
    const form = pick<HTMLFormElement>(
      mount(`<form id="form" action="https://example.com/search"><input id="f" type="search"></form>`),
      "#form",
    );
    const requestSubmit = vi.spyOn(form, "requestSubmit").mockImplementation(() => {
      form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });
    const nativeSubmit = vi.spyOn(form, "submit");
    let submitted = 0;
    form.addEventListener("submit", (e) => {
      submitted++;
      e.preventDefault(); // as a single-page app does; it still counts as taken
    });

    const result = submitFrom(pick(form, "#f"));

    expect(result).toEqual({ submitted: true, path: "requestSubmit" });
    expect(requestSubmit).toHaveBeenCalledTimes(1);
    expect(nativeSubmit).not.toHaveBeenCalled(); // [2] form.submit() is never used
    expect(submitted).toBe(1);
  });

  it("a textarea and a contenteditable inside the form use the same path", () => {
    for (const html of [
      `<form id="form"><textarea id="f"></textarea></form>`,
      `<form id="form"><div id="f" contenteditable="true"></div></form>`,
    ]) {
      document.body.innerHTML = "";
      const form = pick<HTMLFormElement>(mount(html), "#form");
      vi.spyOn(form, "requestSubmit").mockImplementation(() => {
        form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
      });
      form.addEventListener("submit", (e) => e.preventDefault());
      expect(submitFrom(pick(form, "#f"))).toEqual({ submitted: true, path: "requestSubmit" });
    }
  });

  it("a form that refuses (HTML validation: no submit event) reports not submitted", () => {
    const form = pick<HTMLFormElement>(
      mount(`<form id="form"><input id="f" type="text"><input required value=""></form>`),
      "#form",
    );
    const requestSubmit = vi.spyOn(form, "requestSubmit").mockImplementation(() => {
      // invalid form: the browser shows its message and fires no submit event
    });
    const result = submitFrom(pick(form, "#f"));
    expect(requestSubmit).toHaveBeenCalledTimes(1);
    expect(result).toEqual({ submitted: false, path: "none" });
  });

  it("leaves no listener of its own on the form", () => {
    const form = pick<HTMLFormElement>(mount(`<form id="form"><input id="f" type="text"></form>`), "#form");
    vi.spyOn(form, "requestSubmit").mockImplementation(() => {
      form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });
    submitFrom(pick(form, "#f"));
    let seen = 0;
    form.addEventListener("submit", () => seen++);
    form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    expect(seen).toBe(1); // only the listener this test just added
  });
});

describe("[3] a field with no form", () => {
  it("sends a full Enter sequence that the page can take", () => {
    const field = pick(mount(`<div id="f" contenteditable="true"></div>`), "#f");
    const seen: Array<{ type: string; key: string; keyCode: number; bubbles: boolean }> = [];
    field.addEventListener("keydown", (e) => {
      const k = e as KeyboardEvent;
      seen.push({ type: k.type, key: k.key, keyCode: k.keyCode, bubbles: k.bubbles });
      k.preventDefault(); // the page's own "send on Enter"
    });
    for (const type of ["keypress", "keyup"]) {
      field.addEventListener(type, (e) => {
        const k = e as KeyboardEvent;
        seen.push({ type: k.type, key: k.key, keyCode: k.keyCode, bubbles: k.bubbles });
      });
    }

    const result = submitFrom(field);

    expect(result).toEqual({ submitted: true, path: "enter-key" });
    expect(seen.map((s) => s.type)).toEqual(["keydown", "keypress", "keyup"]);
    expect(seen.every((s) => s.key === "Enter" && s.keyCode === 13 && s.bubbles)).toBe(true);
  });

  it("the Enter sequence reaches a listener on an ancestor (bubbles)", () => {
    const page = mount(`<div id="box"><input id="f" type="text"></div>`);
    pick(page, "#box").addEventListener("keydown", (e) => e.preventDefault());
    expect(submitFrom(pick(page, "#f"))).toEqual({ submitted: true, path: "enter-key" });
  });
});

describe("[4][5] nothing takes the Enter: nothing is sent and the text stays", () => {
  it("reports not submitted and leaves the field's value alone", () => {
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="dictated text">`), "#f");
    const before = field.outerHTML;
    const result = submitFrom(field);
    expect(result).toEqual({ submitted: false, path: "none" }); // [4]
    expect(field.value).toBe("dictated text"); // [5]
    expect(field.outerHTML).toBe(before);
  });

  it("a listener that only looks at Enter (without preventDefault) is not treated as a submit", () => {
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text" value="x">`), "#f");
    let seen = 0;
    field.addEventListener("keydown", () => seen++);
    expect(submitFrom(field)).toEqual({ submitted: false, path: "none" });
    expect(seen).toBe(1);
  });
});

describe("fields vtype does not work on", () => {
  it.each([
    [`<form id="form"><input id="f" type="password"></form>`],
    [`<form id="form"><input id="f" type="text" readonly></form>`],
  ])("%s is refused without touching the form", (html) => {
    const form = pick<HTMLFormElement>(mount(html), "#form");
    const requestSubmit = vi.spyOn(form, "requestSubmit");
    expect(submitFrom(pick(form, "#f"))).toEqual({ submitted: false, path: "none" });
    expect(requestSubmit).not.toHaveBeenCalled();
  });

  it("a field removed from the page is refused", () => {
    const field = pick<HTMLInputElement>(mount(`<input id="f" type="text">`), "#f");
    field.remove();
    expect(submitFrom(field)).toEqual({ submitted: false, path: "none" });
  });
});
