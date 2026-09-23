// About vtype: the links (shared with the desktop app) and the options page's section.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { initOptionsPage } from "../src/options/options";
import { aboutUrl, shortUrl, type AboutLink } from "../src/shared/about";
import { translate } from "../src/shared/i18n";
import { settle, stubStorage } from "./stub-storage";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");

/** Shared with the desktop app's tests (packages/native/src/about.rs). */
const fixture = JSON.parse(
  readFileSync(join(repoRoot, "packages", "native", "tests", "fixtures", "about-links.json"), "utf8"),
) as { version: string; links: Record<AboutLink, string> };

describe("the About vtype links", () => {
  it("go where the desktop app's go", () => {
    const names = Object.keys(fixture.links) as AboutLink[];
    expect(names).toHaveLength(5);
    for (const name of names) expect(aboutUrl(name, fixture.version), name).toBe(fixture.links[name]);
  });

  it("say where they go", () => {
    expect(shortUrl("https://ishizakahiroshi.com/")).toBe("ishizakahiroshi.com");
    expect(shortUrl("https://github.com/ishizakahiroshi/vtype")).toBe("github.com/ishizakahiroshi/vtype");
  });
});

/** The real options.html body. */
function mount(): void {
  const html = readFileSync(join(__dirname, "..", "src", "options", "options.html"), "utf8");
  document.body.innerHTML = html.slice(html.indexOf("<body>") + 6, html.indexOf("<script"));
}

describe("About vtype on the options page", () => {
  it("shows the version, the microphone page's words, the developer, the source and the licenses", async () => {
    mount();
    initOptionsPage({ storage: stubStorage().view, language: "en", version: "0.2.0" });
    await settle();
    const about = document.getElementById("about")!;
    expect(about.querySelector("h2")?.textContent).toBe(translate("about_title", "en"));
    const row = Object.fromEntries(
      [...about.querySelectorAll("dt")].map((dt) => [dt.textContent, dt.nextElementSibling as HTMLElement]),
    );
    expect(row[translate("about_version", "en")]?.textContent).toBe("vtype 0.2.0");
    expect(row[translate("about_voice", "en")]?.textContent).toContain(translate("permissionPrivacy", "en"));
    const href = (label: string) => row[translate(label, "en")]?.querySelector("a")?.href;
    expect(href("about_voice")).toBe(fixture.links.privacy);
    expect(href("about_developer")).toBe(fixture.links.homepage);
    expect(href("about_source")).toBe(fixture.links.source);
    expect(href("about_license")).toBe(fixture.links.license);
    // The extension has no desktop parts: only the dictionary's licenses, which it carries.
    const notices = [...row[translate("about_notices", "en")]!.querySelectorAll("a")];
    expect(notices.map((a) => a.getAttribute("href"))).toEqual(["dict/LICENSE-kuromoji.txt", "dict/NOTICE.md"]);
    // It opens in a new tab and says nothing about the desktop app.
    for (const a of about.querySelectorAll("a")) expect(a.target).toBe("_blank");
    expect(about.textContent).not.toContain(translate("optionsDesktopTitle", "en"));
  });
});
