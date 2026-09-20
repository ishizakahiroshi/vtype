// Settings page (C7e), registered as `options_ui` in the manifest.
//
// It has one setting: what starts a recording. Written to chrome.storage.sync, which every
// open page is already listening to (shared/settings.ts), so a change takes effect at once
// without reloading anything.
//
// Built and worded like the permission page (src/permission/): one classic bundle, an HTML
// page with empty elements, and the text filled in here from the browser's language.

import {
  DEFAULT_TRIGGER,
  extensionStorage,
  isTriggerMode,
  readTrigger,
  writeTrigger,
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
  saved: string;
  failed: string;
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
    saved: "Saved.",
    failed: "Could not save the setting. vtype keeps starting when you press the mic.",
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
    saved: "保存しました。",
    failed: "設定を保存できませんでした。マイクを押して始める動作のままになります。",
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

  const radios = [doc.getElementById("click"), doc.getElementById("hover")].filter(
    (el): el is HTMLInputElement => el instanceof HTMLInputElement,
  );

  function show(mode: TriggerMode): void {
    for (const radio of radios) radio.checked = radio.value === mode;
  }

  // The stored value decides what is ticked; an unreadable store shows the default.
  show(DEFAULT_TRIGGER);
  void readTrigger(storage).then(show);

  for (const radio of radios) {
    radio.addEventListener("change", () => {
      if (!radio.checked || !isTriggerMode(radio.value)) return;
      const mode = radio.value;
      void writeTrigger(storage, mode).then((ok) => {
        if (ok) {
          setText(doc, "status", t.saved, "ok");
          return;
        }
        // Nothing was stored, so the page must not claim a setting the extension does not have.
        setText(doc, "status", t.failed, "err");
        show(DEFAULT_TRIGGER);
      });
    });
  }
}

if (typeof document !== "undefined" && document.getElementById("click") !== null) initOptionsPage();
