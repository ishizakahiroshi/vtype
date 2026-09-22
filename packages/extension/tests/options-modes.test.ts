// The options page's input mode and replacement table.

import { describe, expect, it } from "vitest";
import { MAX_REPLACEMENT_RULES } from "vtype-core";
import { formatReplacementText, initOptionsPage, parseReplacementText } from "../src/options/options";
import { INPUT_MODE_KEY, REPLACEMENTS_KEY } from "../src/shared/settings";
import { settle, stubStorage } from "./stub-storage";

function mount(): void {
  document.body.innerHTML = [
    `<legend id="mode-legend"></legend><p id="mode-lead"></p>`,
    `<input type="radio" id="mode-normal" name="inputMode" value="normal"><span id="mode-normal-label"></span>`,
    `<input type="radio" id="mode-en" name="inputMode" value="en"><span id="mode-en-label"></span>`,
    `<input type="radio" id="mode-kana" name="inputMode" value="kana"><span id="mode-kana-label"></span>`,
    `<h2 id="repl-title"></h2><textarea id="repl-text"></textarea>`,
    `<button id="repl-save" type="button"></button><span id="repl-count"></span><p id="repl-bad" hidden></p>`,
    `<p id="status"></p>`,
  ].join("");
}

const radio = (id: string): HTMLInputElement => document.getElementById(id) as HTMLInputElement;
const text = (): HTMLTextAreaElement => document.getElementById("repl-text") as HTMLTextAreaElement;
const status = (): string => document.getElementById("status")?.textContent ?? "";

describe("parseReplacementText", () => {
  it("reads one rule per line and reports the lines it could not read", () => {
    const parsed = parseReplacementText("ぶいたいぷ => vtype\n\nno arrow here\n => nothing left\nGitHub=>GitHub\n");
    expect(parsed.rules).toEqual([
      { from: "ぶいたいぷ", to: "vtype" },
      { from: "GitHub", to: "GitHub" },
    ]);
    expect(parsed.badLines).toEqual([3, 4]);
    expect(parsed.truncated).toBe(false);
  });

  it("keeps the first of two rules for the same word and stops at the limit", () => {
    expect(parseReplacementText("a => 1\nA => 2").rules).toEqual([{ from: "a", to: "1" }]);
    const many = Array.from({ length: MAX_REPLACEMENT_RULES + 5 }, (_, i) => `w${i} => W${i}`).join("\n");
    const parsed = parseReplacementText(many);
    expect(parsed.rules).toHaveLength(MAX_REPLACEMENT_RULES);
    expect(parsed.truncated).toBe(true);
  });

  it("formats what it parses back into the same lines", () => {
    const rules = [
      { from: "a", to: "b" },
      { from: "しー", to: "C" },
    ];
    expect(parseReplacementText(formatReplacementText(rules)).rules).toEqual(rules);
  });
});

describe("the options page", () => {
  it("shows the stored mode and stores the one that is chosen", async () => {
    const store = stubStorage({ sync: { [INPUT_MODE_KEY]: "en" } });
    mount();
    initOptionsPage({ storage: store.view, language: "en" });
    await settle();
    expect(radio("mode-en").checked).toBe(true);

    radio("mode-kana").checked = true;
    radio("mode-kana").dispatchEvent(new Event("change"));
    await settle();
    expect(store.sync[INPUT_MODE_KEY]).toBe("kana");
    expect(status()).toBe("Saved.");
  });

  it("follows a mode changed elsewhere (the desktop app's tray)", async () => {
    const store = stubStorage();
    mount();
    initOptionsPage({ storage: store.view, language: "en" });
    await settle();
    expect(radio("mode-normal").checked).toBe(true);
    await store.view.sync!.set({ [INPUT_MODE_KEY]: "kana" });
    expect(radio("mode-kana").checked).toBe(true);
  });

  it("saves the table to local, shows the count and the unread lines, and shows it again later", async () => {
    const store = stubStorage();
    mount();
    initOptionsPage({ storage: store.view, language: "en" });
    await settle();
    text().value = "ぶいたいぷ => vtype\nbroken line";
    document.getElementById("repl-save")!.click();
    await settle();
    expect(store.local[REPLACEMENTS_KEY]).toEqual([{ from: "ぶいたいぷ", to: "vtype" }]);
    expect(document.getElementById("repl-count")?.textContent).toBe("1 saved");
    const bad = document.getElementById("repl-bad")!;
    expect(bad.hidden).toBe(false);
    expect(bad.textContent).toContain("2");

    mount();
    initOptionsPage({ storage: store.view, language: "en" });
    await settle();
    expect(text().value).toBe("ぶいたいぷ => vtype");
    expect(document.getElementById("repl-count")?.textContent).toBe("1 saved");
  });

  it("says so when the table cannot be saved", async () => {
    const store = stubStorage({}, true);
    mount();
    initOptionsPage({ storage: store.view, language: "en" });
    await settle();
    text().value = "a => b";
    document.getElementById("repl-save")!.click();
    await settle();
    expect(status()).not.toBe("Saved.");
  });
});
