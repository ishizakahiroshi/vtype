// The desktop app's settings page (standalone plan C5).
//
// Served by the desktop app on 127.0.0.1 next to the speech page and opened from its tray in its
// own Chrome. It reads and changes the desktop app's config.json through `api/config` (same
// directory, same per-start token): the input mode, the replacement table, and the desktop app's
// own settings (shortcut, the icon, how text goes in). The words are the extension's options page's
// (`optionsMode*`, `optionsRepl*`, `optionsNative*`), so a language added once covers both.
//
// Every change sends the whole config and shows what the desktop app answers with, which is the
// config as applied (the desktop app keeps what the page does not edit, such as the icon's
// position and the consent).

import { isInputMode, type InputMode } from "vtype-core";
import { translator } from "../shared/i18n";
import { isNativeConfig, type NativeConfig } from "../shared/native-messages";
import { formatReplacementText, parseReplacementText } from "../shared/replacement-text";

type Fetch = (url: string, init?: { method?: string; headers?: Record<string, string>; body?: string }) => Promise<{
  ok: boolean;
  json(): Promise<unknown>;
}>;

export interface SettingsPageOptions {
  doc?: Document;
  language?: string;
  /** The page's own URL (the API sits next to it). Default: location.href. */
  href?: string;
  fetch?: Fetch;
}

export interface SettingsPage {
  /** Resolves once the current settings were read (or could not be). */
  readonly loaded: Promise<void>;
}

/** `…/t/<token>/api/config` for a page at `…/t/<token>/settings`. */
export function apiUrl(href: string): string {
  const url = new URL("api/config", href);
  url.search = "";
  url.hash = "";
  return url.toString();
}

export function initSettingsPage(options: SettingsPageOptions = {}): SettingsPage {
  const doc = options.doc ?? document;
  const t = translator(options.language ?? globalThis.navigator?.language);
  const fetchJson: Fetch = options.fetch ?? ((url, init) => globalThis.fetch(url, init));
  const url = apiUrl(options.href ?? globalThis.location?.href ?? "http://127.0.0.1/settings");
  let config: NativeConfig | null = null;

  const setText = (id: string, text: string, className?: string): void => {
    const el = doc.getElementById(id);
    if (el === null) return;
    el.textContent = text;
    if (className !== undefined) el.className = className;
  };
  const el = <T extends HTMLElement>(id: string, type: new () => T): T | null => {
    const found = doc.getElementById(id);
    return found instanceof type ? found : null;
  };

  doc.title = t("optionsTitle");
  setText("title", t("optionsTitle"));
  setText("mode-legend", t("optionsModeLegend"));
  setText("mode-lead", t("optionsModeLead"));
  setText("mode-normal-label", t("optionsModeNormalLabel"));
  setText("mode-normal-hint", t("optionsModeNormalHint"));
  setText("mode-en-label", t("optionsModeEnLabel"));
  setText("mode-en-hint", t("optionsModeEnHint"));
  setText("mode-kana-label", t("optionsModeKanaLabel"));
  setText("mode-kana-hint", t("optionsModeKanaHint"));
  setText("repl-title", t("optionsReplTitle"));
  setText("repl-lead", t("optionsReplLead"));
  setText("repl-save", t("optionsReplSave"));
  setText("nc-legend", t("optionsNativeLegend"));
  setText("nc-hotkey-label", t("optionsNativeHotkey"));
  setText("nc-hotkey-hint", t("optionsNativeHotkeyHint"));
  setText("nc-icon-visible-label", t("optionsNativeIconVisible"));
  setText("nc-icon-fullscreen-label", t("optionsNativeIconFullscreen"));
  setText("nc-inject-label", t("optionsNativeInject"));
  setText("nc-inject-auto", t("optionsNativeInjectAuto"));
  setText("nc-inject-type", t("optionsNativeInjectType"));
  setText("nc-inject-paste", t("optionsNativeInjectPaste"));
  setText("nc-beside-label", t("optionsNativeBeside"));
  setText("nc-beside-hint", t("optionsNativeBesideHint"));
  setText("nc-beside-trigger-label", t("optionsNativeBesideTrigger"));
  setText("nc-trigger-focus", t("optionsNativeTriggerFocus"));
  setText("nc-trigger-hover", t("optionsNativeTriggerHover"));
  setText("nc-save", t("optionsNativeSave"));
  const replText = el("repl-text", HTMLTextAreaElement);
  if (replText !== null) replText.placeholder = t("optionsReplPlaceholder");

  const modeRadios = (): HTMLInputElement[] =>
    ["mode-normal", "mode-en", "mode-kana"]
      .map((id) => el(id, HTMLInputElement))
      .filter((r): r is HTMLInputElement => r !== null);

  function fill(c: NativeConfig): void {
    const mode: InputMode = c.inputMode ?? "normal";
    for (const radio of modeRadios()) radio.checked = radio.value === mode;
    const rules = c.replacements ?? [];
    if (replText !== null && doc.activeElement !== replText) replText.value = formatReplacementText(rules);
    setText("repl-count", t("optionsReplCount", { count: rules.length }));
    const hotkey = el("nc-hotkey", HTMLInputElement);
    if (hotkey !== null && doc.activeElement !== hotkey) hotkey.value = c.hotkey ?? "";
    const visible = el("nc-icon-visible", HTMLInputElement);
    if (visible !== null) visible.checked = c.icon.visible;
    const fullscreen = el("nc-icon-fullscreen", HTMLInputElement);
    if (fullscreen !== null) fullscreen.checked = c.icon.hideOnFullscreen;
    const inject = el("nc-inject", HTMLSelectElement);
    if (inject !== null) inject.value = c.inject;
    const beside = el("nc-beside", HTMLInputElement);
    if (beside !== null) beside.checked = c.besideField.enabled;
    const trigger = el("nc-beside-trigger", HTMLSelectElement);
    if (trigger !== null) trigger.value = c.besideField.trigger;
  }

  /** The desktop app's own settings from the form, on top of `base`. */
  function readNative(base: NativeConfig): NativeConfig {
    const hotkey = el("nc-hotkey", HTMLInputElement)?.value.trim() ?? "";
    const inject = el("nc-inject", HTMLSelectElement)?.value;
    const trigger = el("nc-beside-trigger", HTMLSelectElement)?.value;
    return {
      ...base,
      hotkey: hotkey === "" ? null : hotkey,
      icon: {
        ...base.icon,
        visible: el("nc-icon-visible", HTMLInputElement)?.checked ?? base.icon.visible,
        hideOnFullscreen: el("nc-icon-fullscreen", HTMLInputElement)?.checked ?? base.icon.hideOnFullscreen,
      },
      inject: inject === "type" || inject === "paste" || inject === "auto" ? inject : base.inject,
      besideField: {
        enabled: el("nc-beside", HTMLInputElement)?.checked ?? base.besideField.enabled,
        trigger: trigger === "hover" || trigger === "focus" ? trigger : base.besideField.trigger,
      },
    };
  }

  async function save(next: NativeConfig, done: string): Promise<boolean> {
    try {
      const res = await fetchJson(url, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(next),
      });
      const applied = res.ok ? await res.json() : null;
      if (!isNativeConfig(applied)) throw new Error("not saved");
      config = applied;
      fill(applied);
      setText("status", done, "ok");
      return true;
    } catch {
      setText("status", t("settings_failed"), "err");
      if (config !== null) fill(config);
      return false;
    }
  }

  for (const radio of modeRadios()) {
    radio.addEventListener("change", () => {
      if (!radio.checked || !isInputMode(radio.value) || config === null) return;
      void save({ ...config, inputMode: radio.value }, t("optionsSaved"));
    });
  }

  doc.getElementById("repl-save")?.addEventListener("click", () => {
    if (replText === null || config === null) return;
    const parsed = parseReplacementText(replText.value);
    const bad = doc.getElementById("repl-bad");
    if (bad !== null) {
      bad.textContent = parsed.badLines.length === 0 ? "" : t("optionsReplBadLines", { lines: parsed.badLines.join(", ") });
      bad.hidden = parsed.badLines.length === 0;
    }
    replText.value = formatReplacementText(parsed.rules);
    void save({ ...config, replacements: parsed.rules }, parsed.truncated ? t("optionsReplTruncated") : t("optionsSaved"));
  });

  doc.getElementById("nc-form")?.addEventListener("submit", (e) => {
    e.preventDefault();
    if (config === null) return;
    void save(readNative(config), t("optionsSaved"));
  });

  const loaded = (async () => {
    try {
      const res = await fetchJson(url);
      const got = res.ok ? await res.json() : null;
      if (!isNativeConfig(got)) throw new Error("unreadable");
      config = got;
      fill(got);
    } catch {
      setText("status", t("settings_failed"), "err");
    }
  })();

  return { loaded };
}

if (typeof document !== "undefined" && document.getElementById("nc-form") !== null) initSettingsPage();
