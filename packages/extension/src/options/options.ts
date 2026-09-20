// Settings page (C7e), registered as `options_ui` in the manifest.
//
// It has one setting: what starts a recording. Written to chrome.storage.sync, which every
// open page is already listening to (shared/settings.ts), so a change takes effect at once
// without reloading anything.
//
// C9 adds the list of sites vtype stays off on. It is the one part of this page that is a
// list rather than a choice, so it is also the one part that is rendered from what is stored
// rather than only ticked: the entry that goes in is what the user typed, put into the shape
// the content script matches on (`https://example.com` or `*.example.com`).
//
// Built and worded like the permission page (src/permission/): one classic bundle, an HTML
// page with empty elements, and the text filled in here from the browser's language.

import {
  DEFAULT_MIC_DISPLAY,
  DEFAULT_TRIGGER,
  clearOffsets,
  extensionStorage,
  isMicDisplay,
  isTriggerMode,
  normalizeExclusion,
  readExcludedSites,
  readMicDisplay,
  readTrigger,
  watchExcludedSites,
  withExcluded,
  withoutExclusionEntry,
  writeExcludedSites,
  writeMicDisplay,
  writeTrigger,
  type ExcludedSites,
  type MicDisplay,
  type StorageView,
  type TriggerMode,
} from "../shared/settings";

interface Texts {
  title: string;
  lead: string;
  legend: string;
  clickLabel: string;
  clickHint: string;
  hoverLabel: string;
  hoverHint: string;
  displayLegend: string;
  displayAllLabel: string;
  displayAllHint: string;
  displayHoverLabel: string;
  displayHoverHint: string;
  saved: string;
  failed: string;
  sitesTitle: string;
  sitesLead: string;
  sitesPlaceholder: string;
  sitesAdd: string;
  sitesEmpty: string;
  sitesRemove: string;
  sitesAdded: (site: string) => string;
  sitesRemoved: (site: string) => string;
  sitesBadInput: string;
  sitesFailed: string;
  positionsTitle: string;
  positionsLead: string;
  reset: string;
  resetDone: (count: number) => string;
  resetNone: string;
  resetFailed: string;
}

const TEXTS: Record<"en" | "ja", Texts> = {
  en: {
    title: "vtype settings",
    lead: "vtype puts what you say into the text field you are working in. Choose how recording starts.",
    legend: "What starts recording",
    clickLabel: "Press the mic (recommended)",
    clickHint:
      "Press the small mic next to the field to start, and press it again to stop. Resting the mouse on it only opens the panel.",
    hoverLabel: "Rest the mouse on the mic",
    hoverHint:
      "Recording starts as soon as the panel opens by itself, so you can speak straight away. You can still press the mic to start and stop.",
    displayLegend: "Where the mic is shown",
    displayAllLabel: "On every text field on screen (recommended)",
    displayAllHint:
      "Each text field you can see gets a faint mic next to it, without you touching anything. On a page with very many fields, only the first ones get one; the rest appear as you move the mouse over them.",
    displayHoverLabel: "Only on the field I am using",
    displayHoverHint:
      "A mic appears on the field the mouse is over, and on the field the caret is in. Moving away takes it back.",
    saved: "Saved.",
    failed: "Could not save the setting. vtype keeps starting when you press the mic.",
    sitesTitle: "Sites vtype stays off on",
    sitesLead:
      "vtype is on everywhere by default. A site on this list gets no mic at all. You can also switch the site you are on off from the panel itself, and on again with the vtype button in the toolbar.",
    sitesPlaceholder: "example.com or *.example.com",
    sitesAdd: "Add",
    sitesEmpty: "vtype is on everywhere: no site has been switched off.",
    sitesRemove: "Switch back on",
    sitesAdded: (site) => `vtype stays off on ${site}.`,
    sitesRemoved: (site) => `vtype is on again on ${site}.`,
    sitesBadInput: "That is not a site address. Write it like example.com, https://example.com or *.example.com.",
    sitesFailed: "Could not save the list.",
    positionsTitle: "Where the mic sits",
    positionsLead:
      "You can drag the small mic aside on a site where it covers one of the site's own buttons. vtype remembers that for the site. This puts every site back to where vtype normally puts it.",
    reset: "Put the mic back on every site",
    resetDone: (count) => `Done: ${count} ${count === 1 ? "site" : "sites"} put back.`,
    resetNone: "The mic has not been moved on any site.",
    resetFailed: "Could not clear the saved positions.",
  },
  ja: {
    title: "vtype の設定",
    lead: "vtype は話した言葉を、今使っている入力欄に文字で入れます。録音の始め方を選べます。",
    legend: "録音の始め方",
    clickLabel: "マイクを押して始める（おすすめ）",
    clickHint:
      "入力欄の横の小さなマイクを押すと始まり、もう一度押すと止まります。マウスを乗せるだけではパネルが開くだけです。",
    hoverLabel: "マイクにマウスを乗せて始める",
    hoverHint:
      "パネルが開いた時点で録音が始まるので、そのまま話せます。マイクを押して始める・止めることもできます。",
    displayLegend: "マイクを出す場所",
    displayAllLabel: "画面に見えている入力欄すべて（おすすめ）",
    displayAllHint:
      "見えている入力欄の横に、薄いマイクが最初から出ます。操作は要りません。欄がとても多いページでは先頭のぶんだけ出て、残りはマウスを乗せたときに出ます。",
    displayHoverLabel: "今使っている欄だけ",
    displayHoverHint:
      "マウスを乗せた欄と、カーソルがある欄にだけマイクが出ます。離れると消えます。",
    saved: "保存しました。",
    failed: "設定を保存できませんでした。マイクを押して始める動作のままになります。",
    sitesTitle: "vtype を使わないサイト",
    sitesLead:
      "vtype は既定ですべてのサイトで動きます。ここに入れたサイトではマイクが一切出ません。今開いているサイトはパネルからも切れます。戻すときはツールバーの vtype のボタンを押してください。",
    sitesPlaceholder: "example.com または *.example.com",
    sitesAdd: "追加",
    sitesEmpty: "切ったサイトはありません。すべてのサイトで動きます。",
    sitesRemove: "また使う",
    sitesAdded: (site) => `${site} では vtype を使いません。`,
    sitesRemoved: (site) => `${site} で vtype をまた使います。`,
    sitesBadInput: "サイトのアドレスとして読めません。example.com / https://example.com / *.example.com の形で書いてください。",
    sitesFailed: "一覧を保存できませんでした。",
    positionsTitle: "マイクの位置",
    positionsLead:
      "サイト自身のボタンとマイクが重なる場合は、小さなマイクをドラッグしてずらせます。ずらした位置はそのサイトごとに覚えています。ここで全サイトぶんを元の位置に戻せます。",
    reset: "すべてのサイトでマイクの位置を元に戻す",
    resetDone: (count) => `${count} 件のサイトの位置を元に戻しました。`,
    resetNone: "位置をずらしたサイトはありません。",
    resetFailed: "保存された位置を消せませんでした。",
  },
};

export function textsFor(language: string | undefined): Texts {
  return language?.toLowerCase().startsWith("ja") ? TEXTS.ja : TEXTS.en;
}

export interface OptionsPageOptions {
  doc?: Document;
  /** chrome.storage by default; null shows the default and cannot save. */
  storage?: StorageView | null;
  language?: string;
}

function setText(doc: Document, id: string, text: string, className?: string): void {
  const el = doc.getElementById(id);
  if (el === null) return;
  el.textContent = text;
  if (className !== undefined) el.className = className;
}

export function initOptionsPage(options: OptionsPageOptions = {}): void {
  const doc = options.doc ?? document;
  const storage = options.storage !== undefined ? options.storage : extensionStorage();
  const t = textsFor(options.language ?? globalThis.navigator?.language);

  setText(doc, "title", t.title);
  setText(doc, "lead", t.lead);
  setText(doc, "legend", t.legend);
  setText(doc, "click-label", t.clickLabel);
  setText(doc, "click-hint", t.clickHint);
  setText(doc, "hover-label", t.hoverLabel);
  setText(doc, "hover-hint", t.hoverHint);
  setText(doc, "display-legend", t.displayLegend);
  setText(doc, "display-all-label", t.displayAllLabel);
  setText(doc, "display-all-hint", t.displayAllHint);
  setText(doc, "display-hover-label", t.displayHoverLabel);
  setText(doc, "display-hover-hint", t.displayHoverHint);
  setText(doc, "sites-title", t.sitesTitle);
  setText(doc, "sites-lead", t.sitesLead);
  setText(doc, "site-add", t.sitesAdd);
  setText(doc, "positions-title", t.positionsTitle);
  setText(doc, "positions-lead", t.positionsLead);
  setText(doc, "reset", t.reset);

  function radioGroup(ids: readonly string[]): HTMLInputElement[] {
    return ids.map((id) => doc.getElementById(id)).filter((el): el is HTMLInputElement => el instanceof HTMLInputElement);
  }

  /** One group of radios: show what is stored, store what is chosen, own the status line. */
  function wireChoice<T extends string>(
    ids: readonly string[],
    fallback: T,
    isValue: (v: unknown) => v is T,
    read: (s: StorageView | null) => Promise<T>,
    write: (s: StorageView | null, v: T) => Promise<boolean>,
  ): void {
    const radios = radioGroup(ids);
    const show = (value: T): void => {
      for (const radio of radios) radio.checked = radio.value === value;
    };
    // The stored value decides what is ticked; an unreadable store shows the default.
    show(fallback);
    void read(storage).then(show);
    for (const radio of radios) {
      radio.addEventListener("change", () => {
        if (!radio.checked || !isValue(radio.value)) return;
        void write(storage, radio.value).then((ok) => {
          if (ok) {
            setText(doc, "status", t.saved, "ok");
            return;
          }
          // Nothing was stored, so the page must not claim a setting the extension has not got.
          setText(doc, "status", t.failed, "err");
          show(fallback);
        });
      });
    }
  }

  wireChoice<TriggerMode>(["click", "hover"], DEFAULT_TRIGGER, isTriggerMode, readTrigger, writeTrigger);
  wireChoice<MicDisplay>(
    ["display-all", "display-hover"],
    DEFAULT_MIC_DISPLAY,
    isMicDisplay,
    readMicDisplay,
    writeMicDisplay,
  );

  // ---- C9: the sites vtype stays off on --------------------------------------------------

  const siteInput = doc.getElementById("site-input");
  const siteList = doc.getElementById("site-list");
  const siteEmpty = doc.getElementById("site-empty");
  if (siteInput instanceof HTMLInputElement) siteInput.placeholder = t.sitesPlaceholder;

  let sites: ExcludedSites = [];

  function renderSites(next: ExcludedSites): void {
    sites = next;
    if (siteEmpty !== null) {
      siteEmpty.textContent = t.sitesEmpty;
      siteEmpty.hidden = next.length > 0;
    }
    if (siteList === null) return;
    siteList.replaceChildren();
    for (const site of next) {
      const row = doc.createElement("li");
      const label = doc.createElement("span");
      label.className = "site";
      label.textContent = site;
      const remove = doc.createElement("button");
      remove.type = "button";
      remove.textContent = t.sitesRemove;
      remove.addEventListener("click", () => {
        void save(withoutExclusionEntry(sites, site), t.sitesRemoved(site));
      });
      row.append(label, remove);
      siteList.append(row);
    }
  }

  /** Store the list and say what happened. The list on screen only follows a write that went through. */
  async function save(next: ExcludedSites, message: string): Promise<void> {
    const ok = await writeExcludedSites(storage, next);
    if (!ok) {
      // Nothing was stored, so the page must not show a list the extension has not got.
      setText(doc, "status", t.sitesFailed, "err");
      return;
    }
    renderSites(next);
    setText(doc, "status", message, "ok");
  }

  renderSites([]);
  void readExcludedSites(storage).then(renderSites);
  // Another tab, another device, or the panel's own "don't use vtype on this site".
  watchExcludedSites(storage, renderSites);

  doc.getElementById("site-form")?.addEventListener("submit", (e) => {
    e.preventDefault();
    if (!(siteInput instanceof HTMLInputElement)) return;
    const typed = siteInput.value;
    const pattern = normalizeExclusion(typed);
    if (pattern === null) {
      // Said plainly instead of stored: an entry that matches nothing looks like a bug later.
      setText(doc, "status", t.sitesBadInput, "err");
      return;
    }
    siteInput.value = "";
    void save(withExcluded(sites, pattern), t.sitesAdded(pattern));
  });

  // C7f: forget every dragged mic position. Open pages hear about it through
  // chrome.storage.onChanged and put their mic back without being reloaded.
  doc.getElementById("reset")?.addEventListener("click", () => {
    if (storage === null) {
      setText(doc, "status", t.resetFailed, "err");
      return;
    }
    void clearOffsets(storage).then((count) => {
      setText(doc, "status", count === 0 ? t.resetNone : t.resetDone(count), "ok");
    });
  });
}

if (typeof document !== "undefined" && document.getElementById("click") !== null) initOptionsPage();
