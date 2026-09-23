// "About vtype": the version, where the voice goes, who makes vtype, the source and the licences.
// The extension's options page and the desktop app's settings page both end with it.
//
// The links are the desktop app's too: packages/native/src/about.rs opens them for its settings
// page (a Chrome window of its own) in the usual browser. Both test against
// packages/native/tests/fixtures/about-links.json, so they cannot drift apart.

import type { Translate } from "./i18n";

const REPO = "https://github.com/ishizakahiroshi/vtype";

export const ABOUT_LINKS = {
  privacy: `${REPO}/blob/main/PRIVACY.md`,
  homepage: "https://ishizakahiroshi.com/",
  source: REPO,
  license: `${REPO}/blob/main/LICENSE`,
} as const;

/** The links by name; `notices` is the desktop app's crates' licences. */
export type AboutLink = keyof typeof ABOUT_LINKS | "notices";

/** Where `link` goes. The desktop app's notices are attached to its release `version`. */
export function aboutUrl(link: AboutLink, version: string): string {
  return link === "notices" ? `${REPO}/releases/download/native-v${version}/THIRD_PARTY_NOTICES.txt` : ABOUT_LINKS[link];
}

/** `https://github.com/a/b` as `github.com/a/b`: a link text that says where it goes. */
export function shortUrl(url: string): string {
  const u = new URL(url);
  return `${u.host}${u.pathname}`.replace(/\/$/, "");
}

export interface AboutOptions {
  /** Empty when unknown: the version row and the desktop app's notices are left out. */
  version: string;
  /** Where the voice goes, in the words the user was shown before the first recording. */
  voice: string;
  /** The desktop app lists the licences of its own parts too. */
  desktop?: boolean;
  /**
   * Opens a link elsewhere (the desktop app: in the usual browser). Resolves false when it could
   * not, and the page opens the link itself. None: the page opens every link itself.
   */
  open?: (link: AboutLink) => Promise<boolean>;
}

/** Fills `section` (empty in the page) with the heading and the rows. */
export function renderAbout(section: HTMLElement, t: Translate, options: AboutOptions): void {
  const doc = section.ownerDocument;
  const list = doc.createElement("dl");
  list.className = "about";

  const link = (text: string, href: string, name?: AboutLink): HTMLAnchorElement => {
    const a = doc.createElement("a");
    a.textContent = text;
    a.href = href;
    a.target = "_blank";
    a.rel = "noopener";
    const open = options.open;
    if (name !== undefined && open !== undefined) {
      a.addEventListener("click", (e) => {
        e.preventDefault();
        void open(name).then((opened) => {
          if (!opened) doc.defaultView?.open(a.href, "_blank", "noopener");
        });
      });
    }
    return a;
  };
  const named = (name: AboutLink, text: string): HTMLAnchorElement => link(text, aboutUrl(name, options.version), name);
  const row = (label: string, ...value: Array<Node | string>): void => {
    const dt = doc.createElement("dt");
    dt.textContent = label;
    const dd = doc.createElement("dd");
    dd.append(...value);
    list.append(dt, dd);
  };

  if (options.version !== "") row(t("about_version"), t("about_versionValue", { version: options.version }));
  row(t("about_voice"), `${options.voice} `, named("privacy", t("about_privacy")));
  row(t("about_developer"), named("homepage", shortUrl(ABOUT_LINKS.homepage)));
  row(t("about_source"), named("source", shortUrl(ABOUT_LINKS.source)));
  row(t("about_license"), named("license", t("about_licenseMit")));
  const notices: HTMLAnchorElement[] = [];
  if (options.desktop === true && options.version !== "") notices.push(named("notices", t("about_noticesDesktop")));
  // The katakana mode's dictionary travels with its licences (build.mjs copies them into dict/,
  // next to both pages).
  notices.push(
    link(t("about_noticesKuromoji"), "dict/LICENSE-kuromoji.txt"),
    link(t("about_noticesDictionary"), "dict/NOTICE.md"),
  );
  row(t("about_notices"), ...notices.flatMap((a, i) => (i === 0 ? [a] : [" / ", a])));

  const heading = doc.createElement("h2");
  heading.textContent = t("about_title");
  section.replaceChildren(heading, list);
}
