// "Report a problem": the URL, the options page button, and the issue form it points at.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { initOptionsPage } from "../src/options/options";
import { SURFACE_LABELS, bugReportUrl, describeBrowser, describeOs, type ReportInfo } from "../src/shared/report";
import { settle, stubStorage } from "./stub-storage";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");

/** Shared with the desktop app's tests (packages/native/src/report.rs). */
const fixture = JSON.parse(
  readFileSync(join(repoRoot, "packages", "native", "tests", "fixtures", "report-url.json"), "utf8"),
) as Array<{ input: ReportInfo; url: string }>;

const form = readFileSync(join(repoRoot, ".github", "ISSUE_TEMPLATE", "bug_report.yml"), "utf8");

describe("bugReportUrl", () => {
  it("builds the same URL as the desktop app for every shared case", () => {
    expect(fixture.length).toBeGreaterThan(0);
    for (const { input, url } of fixture) expect(bugReportUrl(input)).toBe(url);
  });

  it("puts all four fields in the query and encodes them", () => {
    const url = new URL(bugReportUrl({ surface: "extension", version: "1.2.3", os: "A & B", browser: "C=D" }));
    expect(url.searchParams.get("template")).toBe("bug_report.yml");
    expect(url.searchParams.get("surface")).toBe(SURFACE_LABELS.extension);
    expect(url.searchParams.get("version")).toBe("1.2.3");
    expect(url.searchParams.get("os")).toBe("A & B");
    expect(url.searchParams.get("browser")).toBe("C=D");
  });
});

describe("the issue form", () => {
  it("has a field for every query parameter the URL fills", () => {
    const ids = [...form.matchAll(/^\s+id:\s*([\w-]+)\s*$/gm)].map((m) => m[1]);
    for (const id of ["surface", "version", "os", "browser"]) expect(ids, id).toContain(id);
  });

  it("offers exactly the labels the URL uses for the surface", () => {
    const block = form.slice(form.indexOf("id: surface"), form.indexOf("validations:", form.indexOf("id: surface")));
    const options = [...block.matchAll(/^\s+- (.+)$/gm)].map((m) => m[1]!.trim());
    expect(options).toEqual(Object.values(SURFACE_LABELS));
  });
});

describe("what the page tells about the machine", () => {
  it("prefers the client hints, and falls back to the user agent", () => {
    expect(describeOs({ userAgentData: { platform: "Windows" } })).toBe("Windows");
    expect(describeOs({ userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)" })).toBe("macOS");
    expect(describeOs({ userAgent: "Mozilla/5.0 (X11; Linux x86_64)" })).toBe("Linux");
    expect(describeOs(undefined)).toBe("");
    expect(
      describeBrowser({ userAgentData: { brands: [{ brand: "Not A Brand", version: "99" }, { brand: "Google Chrome", version: "140" }] } }),
    ).toBe("Google Chrome 140");
    expect(describeBrowser({ userAgent: "Mozilla/5.0 Chrome/140.0.7339.80 Safari/537.36" })).toBe("Chrome 140.0.7339.80");
  });
});

describe("the options page button", () => {
  it("opens the form for the extension with the version filled in", async () => {
    document.body.innerHTML = `<h2 id="report-title"></h2><p id="report-lead"></p><button id="report-open" type="button"></button><p id="status"></p>`;
    const opened: string[] = [];
    initOptionsPage({ storage: stubStorage().view, language: "en", version: "0.1.0", openUrl: (u) => opened.push(u) });
    await settle();
    expect(document.getElementById("report-open")?.textContent).toBe("Open the report form");
    document.getElementById("report-open")!.click();
    expect(opened).toHaveLength(1);
    const url = new URL(opened[0]!);
    expect(url.origin + url.pathname).toBe("https://github.com/ishizakahiroshi/vtype/issues/new");
    expect(url.searchParams.get("surface")).toBe(SURFACE_LABELS.extension);
    expect(url.searchParams.get("version")).toBe("0.1.0");
  });
});
