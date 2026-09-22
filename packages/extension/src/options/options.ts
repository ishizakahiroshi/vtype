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

import { isInputMode, MAX_REPLACEMENT_RULES, type InputMode, type ReplacementRule } from "vtype-core";
import { translator } from "../shared/i18n";
import { bugReportUrl, describeBrowser, describeOs } from "../shared/report";
import { clearDiagLog, formatDiagLog, readDiagLog, watchDiagLog, type DiagEntry } from "../shared/diagnostics";
import {
  DEFAULT_INPUT_MODE,
  DEFAULT_MIC_DISPLAY,
  DEFAULT_TRIGGER,
  clearOffsets,
  extensionStorage,
  isMicDisplay,
  isTriggerMode,
  normalizeExclusion,
  readDiagnostics,
  readExcludedSites,
  readInputMode,
  readMicDisplay,
  readReplacements,
  readTrigger,
  watchExcludedSites,
  watchInputMode,
  withExcluded,
  withoutExclusionEntry,
  writeDiagnostics,
  writeExcludedSites,
  writeInputMode,
  writeMicDisplay,
  writeReplacements,
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
  /** Opens a web page in a new tab. Default: window.open. */
  openUrl?: (url: string) => void;
  /** The extension's version. Default: the manifest's. */
  version?: string;
}

function manifestVersion(): string {
  const runtime = (globalThis as { chrome?: { runtime?: { getManifest?: () => { version?: string } } } }).chrome
    ?.runtime;
  return runtime?.getManifest?.().version ?? "";
}

/** The separator between what was recognized and what to type, one rule per line. */
export const REPLACEMENT_ARROW = "=>";

export interface ParsedReplacements {
  readonly rules: ReplacementRule[];
  /** 1-based numbers of the lines that could not be read. Blank lines are not counted. */
  readonly badLines: number[];
  /** More rules were written than MAX_REPLACEMENT_RULES; the rest were dropped. */
  readonly truncated: boolean;
}

/** `認識結果 => 入れたい形`, one per line. Spaces around the arrow are ignored. */
export function parseReplacementText(text: string): ParsedReplacements {
  const rules: ReplacementRule[] = [];
  const badLines: number[] = [];
  let truncated = false;
  text.split(/\r?\n/).forEach((line, index) => {
    if (line.trim() === "") return;
    const at = line.indexOf(REPLACEMENT_ARROW);
    const from = at < 0 ? "" : line.slice(0, at).trim();
    if (from === "") {
      badLines.push(index + 1);
      return;
    }
    if (rules.some((r) => r.from.toLowerCase() === from.toLowerCase())) return;
    if (rules.length >= MAX_REPLACEMENT_RULES) {
      truncated = true;
      return;
    }
    rules.push({ from, to: line.slice(at + REPLACEMENT_ARROW.length).trim() });
  });
  return { rules, badLines, truncated };
}

export function formatReplacementText(rules: readonly ReplacementRule[]): string {
  return rules.map((r) => `${r.from} ${REPLACEMENT_ARROW} ${r.to}`).join("\n");
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
  setText(doc, "diag-title", t("optionsDiagTitle"));
  setText(doc, "diag-lead", t("optionsDiagLead"));
  setText(doc, "diag-label", t("optionsDiagLabel"));
  setText(doc, "diag-hint", t("optionsDiagHint"));
  setText(doc, "diag-copy", t("optionsDiagCopy"));
  setText(doc, "diag-clear", t("optionsDiagClear"));
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

  // ---- input mode and replacement table (native plan C2) ---------------------------------

  setText(doc, "mode-legend", t("optionsModeLegend"));
  setText(doc, "mode-lead", t("optionsModeLead"));
  setText(doc, "mode-normal-label", t("optionsModeNormalLabel"));
  setText(doc, "mode-normal-hint", t("optionsModeNormalHint"));
  setText(doc, "mode-en-label", t("optionsModeEnLabel"));
  setText(doc, "mode-en-hint", t("optionsModeEnHint"));
  setText(doc, "mode-kana-label", t("optionsModeKanaLabel"));
  setText(doc, "mode-kana-hint", t("optionsModeKanaHint"));
  setText(doc, "repl-title", t("optionsReplTitle"));
  setText(doc, "repl-lead", t("optionsReplLead"));
  setText(doc, "repl-save", t("optionsReplSave"));

  const modeIds = ["mode-normal", "mode-en", "mode-kana"];
  wireChoice<InputMode>(modeIds, DEFAULT_INPUT_MODE, isInputMode, readInputMode, writeInputMode);
  // The desktop app's tray switches the same setting; follow it while the page is open.
  watchInputMode(storage, (mode) => {
    for (const radio of radioGroup(modeIds)) radio.checked = radio.value === mode;
  });

  const replText = doc.getElementById("repl-text");
  const replBad = doc.getElementById("repl-bad");

  function showRuleCount(count: number): void {
    setText(doc, "repl-count", t("optionsReplCount", { count }));
  }

  function showBadLines(lines: readonly number[]): void {
    if (replBad === null) return;
    replBad.textContent = lines.length === 0 ? "" : t("optionsReplBadLines", { lines: lines.join(", ") });
    replBad.hidden = lines.length === 0;
  }

  if (replText instanceof HTMLTextAreaElement) {
    replText.placeholder = t("optionsReplPlaceholder");
    showRuleCount(0);
    void readReplacements(storage).then((rules) => {
      replText.value = formatReplacementText(rules);
      showRuleCount(rules.length);
    });
    doc.getElementById("repl-save")?.addEventListener("click", () => {
      const parsed = parseReplacementText(replText.value);
      showBadLines(parsed.badLines);
      void writeReplacements(storage, parsed.rules).then((ok) => {
        if (!ok) {
          setText(doc, "status", t("optionsFailed"), "err");
          return;
        }
        showRuleCount(parsed.rules.length);
        setText(doc, "status", parsed.truncated ? t("optionsReplTruncated") : t("optionsSaved"), "ok");
      });
    });
  }

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

  // ---- the diagnostic log (off by default; shared/diagnostics.ts) --------------------------
  //
  // The log is written by the background and read here. It holds no transcript, so showing it
  // in full is safe; "copy" exists because the point of the log is to be handed to someone.

  const diagToggle = doc.getElementById("diag");
  const diagLog = doc.getElementById("diag-log");
  const diagEmpty = doc.getElementById("diag-empty");
  let diagText = "";

  function renderDiag(entries: readonly DiagEntry[]): void {
    diagText = formatDiagLog(entries);
    if (diagEmpty !== null) {
      diagEmpty.textContent = t("optionsDiagEmpty");
      diagEmpty.hidden = entries.length > 0;
    }
    if (diagLog === null) return;
    diagLog.textContent = diagText;
    diagLog.hidden = entries.length === 0;
  }

  renderDiag([]);
  void readDiagLog(storage).then(renderDiag);
  // A recording happens on another tab, so the page has to follow the log rather than read it once.
  watchDiagLog(storage, renderDiag);

  if (diagToggle instanceof HTMLInputElement) {
    void readDiagnostics(storage).then((on) => {
      diagToggle.checked = on;
    });
    diagToggle.addEventListener("change", () => {
      const wanted = diagToggle.checked;
      void writeDiagnostics(storage, wanted).then((ok) => {
        if (ok) {
          setText(doc, "status", t("optionsSaved"), "ok");
          return;
        }
        // Nothing was stored, so the page must not claim a setting the extension has not got.
        setText(doc, "status", t("optionsFailed"), "err");
        diagToggle.checked = !wanted;
      });
    });
  }

  doc.getElementById("diag-copy")?.addEventListener("click", () => {
    const clipboard = (doc.defaultView?.navigator as { clipboard?: { writeText(t: string): Promise<void> } } | undefined)
      ?.clipboard;
    if (clipboard === undefined) {
      setText(doc, "status", t("optionsDiagCopyFailed"), "err");
      return;
    }
    void clipboard
      .writeText(diagText)
      .then(() => setText(doc, "status", t("optionsDiagCopied"), "ok"))
      .catch(() => setText(doc, "status", t("optionsDiagCopyFailed"), "err"));
  });

  doc.getElementById("diag-clear")?.addEventListener("click", () => {
    void clearDiagLog(storage).then(() => {
      renderDiag([]);
      setText(doc, "status", t("optionsDiagCleared"), "ok");
    });
  });

  // ---- report a problem (native plan C2) ------------------------------------------------
  //
  // Opens GitHub's issue form with the version, OS and browser filled in. Nothing is sent: the
  // user sees the form and decides.

  setText(doc, "diag-report-note", t("optionsDiagReportNote"));
  setText(doc, "report-title", t("optionsReportTitle"));
  setText(doc, "report-lead", t("optionsReportLead"));
  setText(doc, "report-open", t("optionsReportOpen"));
  doc.getElementById("report-open")?.addEventListener("click", () => {
    const nav = doc.defaultView?.navigator as Parameters<typeof describeOs>[0];
    const url = bugReportUrl({
      surface: "extension",
      version: options.version ?? manifestVersion(),
      os: describeOs(nav),
      browser: describeBrowser(nav),
    });
    const open = options.openUrl ?? ((u: string) => void doc.defaultView?.open(u, "_blank", "noopener"));
    open(url);
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
