// "Report a problem": the GitHub issue form, opened with what vtype knows filled in. Nothing is
// sent anywhere; the user reads the form and decides whether to submit it.
//
// The desktop app builds the same URL in packages/native/src/report.rs. Both are tested against
// packages/native/tests/fixtures/report-url.json, and the field ids and dropdown labels here are
// checked against .github/ISSUE_TEMPLATE/bug_report.yml, so the three cannot drift apart.

export const ISSUE_FORM = "https://github.com/ishizakahiroshi/vtype/issues/new";
export const ISSUE_TEMPLATE = "bug_report.yml";

export type ReportSurface = "extension" | "desktop";

/** The option labels of the form's `surface` dropdown; GitHub fills a dropdown by its label. */
export const SURFACE_LABELS: Readonly<Record<ReportSurface, string>> = {
  extension: "Chrome extension / Chrome 拡張",
  desktop: "Desktop app / デスクトップ版",
};

export interface ReportInfo {
  readonly surface: ReportSurface;
  readonly version: string;
  readonly os: string;
  readonly browser: string;
}

export function bugReportUrl(info: ReportInfo): string {
  const query = [
    ["template", ISSUE_TEMPLATE],
    ["surface", SURFACE_LABELS[info.surface]],
    ["version", info.version],
    ["os", info.os],
    ["browser", info.browser],
  ]
    .map(([key, value]) => `${key}=${key === "template" ? value : encodeURIComponent(value!)}`)
    .join("&");
  return `${ISSUE_FORM}?${query}`;
}

interface NavigatorLike {
  readonly userAgent?: string;
  readonly userAgentData?: {
    readonly platform?: string;
    readonly brands?: ReadonlyArray<{ readonly brand: string; readonly version: string }>;
  };
}

/** `Windows`, `macOS`, `Linux`, `ChromeOS`, or what the browser calls it. Empty when unknown. */
export function describeOs(nav: NavigatorLike | undefined): string {
  const platform = nav?.userAgentData?.platform;
  if (platform !== undefined && platform !== "") return platform;
  const ua = nav?.userAgent ?? "";
  if (/Windows/.test(ua)) return "Windows";
  if (/Mac OS X|Macintosh/.test(ua)) return "macOS";
  if (/CrOS/.test(ua)) return "ChromeOS";
  if (/Linux/.test(ua)) return "Linux";
  return "";
}

/** e.g. `Google Chrome 140`. Empty when unknown. */
export function describeBrowser(nav: NavigatorLike | undefined): string {
  const brands = nav?.userAgentData?.brands ?? [];
  const named = brands.find((b) => b.brand === "Google Chrome") ?? brands.find((b) => b.brand === "Chromium");
  if (named !== undefined) return `${named.brand} ${named.version}`;
  const match = /Chrome\/([\d.]+)/.exec(nav?.userAgent ?? "");
  return match === null ? "" : `Chrome ${match[1]}`;
}
