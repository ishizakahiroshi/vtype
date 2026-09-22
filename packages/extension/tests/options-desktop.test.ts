// The options page's note about the desktop app (standalone plan C6): a separate app, so one line
// and a link, with nothing to connect.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { DESKTOP_INSTALL_URL, initOptionsPage } from "../src/options/options";
import { translate } from "../src/shared/i18n";
import { settle, stubStorage } from "./stub-storage";

/** The real options.html body, so a control left in the page is seen. */
function mount(): void {
  const html = readFileSync(join(__dirname, "..", "src", "options", "options.html"), "utf8");
  document.body.innerHTML = html.slice(html.indexOf("<body>") + 6, html.indexOf("<script"));
}

describe("the desktop app section", () => {
  it("is one line and a link to the desktop app", async () => {
    mount();
    initOptionsPage({ storage: stubStorage().view, language: "en" });
    await settle();
    expect(document.getElementById("desktop-title")?.textContent).toBe(translate("optionsDesktopTitle", "en"));
    expect(document.getElementById("desktop-lead")?.textContent).toBe(translate("optionsDesktopLead", "en"));
    const link = document.getElementById("desktop-install-link") as HTMLAnchorElement;
    expect(link.href).toBe(DESKTOP_INSTALL_URL);
    expect(link.textContent).toBe(translate("optionsDesktopInstallLink", "en"));
  });

  it("has nothing that connects to the desktop app", () => {
    mount();
    for (const id of ["desktop-enable", "desktop-disable", "desktop-retry", "desktop-status", "desktop-config", "nc-hotkey"]) {
      expect(document.getElementById(id), id).toBeNull();
    }
  });
});
