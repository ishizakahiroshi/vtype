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
// The text itself lives in `_locales/` (one file per language) and is read through shared/i18n.

import { translator } from "../shared/i18n";
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
  const t = translator(options.language ?? globalThis.navigator?.language);

  setText(doc, "title", t("optionsTitle"));
  setText(doc, "lead", t("optionsLead"));
  setText(doc, "legend", t("optionsLegend"));
  setText(doc, "click-label", t("optionsClickLabel"));
  setText(doc, "click-hint", t("optionsClickHint"));
  setText(doc, "hover-label", t("optionsHoverLabel"));
  setText(doc, "hover-hint", t("optionsHoverHint"));
  setText(doc, "display-legend", t("optionsDisplayLegend"));
  setText(doc, "display-all-label", t("optionsDisplayAllLabel"));
  setText(doc, "display-all-hint", t("optionsDisplayAllHint"));
  setText(doc, "display-hover-label", t("optionsDisplayHoverLabel"));
  setText(doc, "display-hover-hint", t("optionsDisplayHoverHint"));
  setText(doc, "sites-title", t("optionsSitesTitle"));
  setText(doc, "sites-lead", t("optionsSitesLead"));
  setText(doc, "site-add", t("optionsSitesAdd"));
  setText(doc, "positions-title", t("optionsPositionsTitle"));
  setText(doc, "positions-lead", t("optionsPositionsLead"));
  setText(doc, "reset", t("optionsReset"));

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
            setText(doc, "status", t("optionsSaved"), "ok");
            return;
          }
          // Nothing was stored, so the page must not claim a setting the extension has not got.
          setText(doc, "status", t("optionsFailed"), "err");
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
  if (siteInput instanceof HTMLInputElement) siteInput.placeholder = t("optionsSitesPlaceholder");

  let sites: ExcludedSites = [];

  function renderSites(next: ExcludedSites): void {
    sites = next;
    if (siteEmpty !== null) {
      siteEmpty.textContent = t("optionsSitesEmpty");
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
      remove.textContent = t("optionsSitesRemove");
      remove.addEventListener("click", () => {
        void save(withoutExclusionEntry(sites, site), t("optionsSitesRemoved", { site }));
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
      setText(doc, "status", t("optionsSitesFailed"), "err");
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
      setText(doc, "status", t("optionsSitesBadInput"), "err");
      return;
    }
    siteInput.value = "";
    void save(withExcluded(sites, pattern), t("optionsSitesAdded", { site: pattern }));
  });

  // C7f: forget every dragged mic position. Open pages hear about it through
  // chrome.storage.onChanged and put their mic back without being reloaded.
  doc.getElementById("reset")?.addEventListener("click", () => {
    if (storage === null) {
      setText(doc, "status", t("optionsResetFailed"), "err");
      return;
    }
    void clearOffsets(storage).then((count) => {
      setText(doc, "status", count === 0 ? t("optionsResetNone") : t(count === 1 ? "optionsResetDoneOne" : "optionsResetDoneMany", { count }), "ok");
    });
  });
}

if (typeof document !== "undefined" && document.getElementById("click") !== null) initOptionsPage();
