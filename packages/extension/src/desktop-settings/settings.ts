// The desktop app's settings page (standalone plan C5).
//
// Served by the desktop app on 127.0.0.1 next to the speech page and opened from its tray in its
// own Chrome. It reads and changes the desktop app's config.json through `api/config` (same
// directory, same per-start token): the input mode, the replacement table, and the desktop app's
// own settings (shortcut, the icon, how text goes in), and the templates the floating mic's
// top-left button puts in. The words are mostly the extension's options page's (`optionsMode*`,
// `optionsRepl*`, `optionsNative*`), so a language added once covers both; the templates are the
// desktop app's alone (`settings_templates*`).
//
// Every change sends the whole config and shows what the desktop app answers with, which is the
// config as applied (the desktop app keeps what the page does not edit, such as the icon's
// position and the consent).
//
// There is one settings window: every "open settings" starts a new one (Chrome has no way to
// bring an `--app` window back), so a window that opens tells the others over a
// BroadcastChannel, and the older ones hand it what they hold unsaved and close.

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
  /** Where settings windows meet (`SETTINGS_CHANNEL`). None by default: the page script passes one. */
  channel?: SettingsChannel | null;
  /** Closes this window. Default: window.close(). */
  close?: () => void;
  /** When this window opened, to tell the newer of two. Default: now. */
  openedAt?: number;
}

/** The BroadcastChannel settings windows find each other on (the settings profile only). */
export const SETTINGS_CHANNEL = "vtype-settings";

/** The part of a BroadcastChannel the page uses. */
export interface SettingsChannel {
  postMessage(message: unknown): void;
  addEventListener(type: "message", listener: (event: { data: unknown }) => void): void;
}

/** What a settings window holds unsaved, for the newer window that takes over. */
export interface SettingsDraft {
  replacements?: string;
  /** The desktop app's own fields (`NATIVE_FIELDS`), by element id. */
  native?: Record<string, string | boolean>;
  newTemplate?: string;
  /** A template being edited: the saved text (to find it again) and the text now. */
  editing?: { original: string; text: string };
}

/** The form's fields for the desktop app's own settings (saved with its button). */
const NATIVE_FIELDS = ["nc-hotkey", "nc-icon-visible", "nc-icon-fullscreen", "nc-inject", "nc-send-key", "nc-beside", "nc-beside-trigger"];

type ChannelMessage =
  | { type: "opened"; id: string; at: number }
  | { type: "draft"; to: string; draft: SettingsDraft };

function isChannelMessage(data: unknown): data is ChannelMessage {
  if (typeof data !== "object" || data === null) return false;
  const m = data as Record<string, unknown>;
  if (m.type === "opened") return typeof m.id === "string" && typeof m.at === "number";
  return m.type === "draft" && typeof m.to === "string" && typeof m.draft === "object" && m.draft !== null;
}

export interface SettingsPage {
  /** Resolves once the current settings were read (or could not be). */
  readonly loaded: Promise<void>;
}

/** Same limits as the desktop app's config.rs (`MAX_TEMPLATES`, `MAX_TEMPLATE_CHARS`). */
export const MAX_TEMPLATES = 100;
export const MAX_TEMPLATE_CHARS = 8000;

/** Trimmed, non-empty, cut to length, no duplicates, at most `MAX_TEMPLATES` (as config.rs). */
export function normalizeTemplates(items: readonly string[]): string[] {
  const out: string[] = [];
  for (const item of items) {
    const text = [...item.trim()].slice(0, MAX_TEMPLATE_CHARS).join("").trimEnd();
    if (text !== "" && !out.includes(text) && out.length < MAX_TEMPLATES) out.push(text);
  }
  return out;
}

/** `…/t/<token>/api/config` for a page at `…/t/<token>/settings`. */
export function apiUrl(href: string): string {
  const url = new URL("api/config", href);
  url.search = "";
  url.hash = "";
  return url.toString();
}

/**
 * The template to open for editing: the desktop app opens `…/settings#template-<index>` when the
 * user chose "Edit" in the floating mic's templates menu.
 */
export function templateFromHref(href: string): number | null {
  const m = /#template-(\d+)$/.exec(href);
  return m === null ? null : Number(m[1]);
}

/** The floating mic's size in percent: same limits as the desktop app's overlay_logic.rs. */
export const ICON_SCALE_MIN = 50;
export const ICON_SCALE_MAX = 200;
export const ICON_SCALE_DEFAULT = 100;

/** A typed size, whole and within the limits; null when it is not a number (keep what was). */
export function clampIconScale(typed: string): number | null {
  const n = Math.round(Number(typed));
  return typed.trim() === "" || !Number.isFinite(n) ? null : Math.min(ICON_SCALE_MAX, Math.max(ICON_SCALE_MIN, n));
}

export function initSettingsPage(options: SettingsPageOptions = {}): SettingsPage {
  const doc = options.doc ?? document;
  const t = translator(options.language ?? globalThis.navigator?.language);
  const fetchJson: Fetch = options.fetch ?? ((url, init) => globalThis.fetch(url, init));
  const href = options.href ?? globalThis.location?.href ?? "http://127.0.0.1/settings";
  const url = apiUrl(href);
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
  setText("mode-lead", t("settings_modeLead"));
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
  setText("nc-icon-scale-label", t("settings_iconScale"));
  setText("nc-icon-scale-reset", t("settings_iconScaleReset"));
  setText("nc-icon-scale-hint", t("settings_iconScaleHint", { min: ICON_SCALE_MIN, max: ICON_SCALE_MAX }));
  setText("nc-inject-label", t("optionsNativeInject"));
  setText("nc-inject-auto", t("optionsNativeInjectAuto"));
  setText("nc-inject-type", t("optionsNativeInjectType"));
  setText("nc-inject-paste", t("optionsNativeInjectPaste"));
  setText("nc-send-key-label", t("optionsNativeSendKey"));
  setText("nc-send-enter", t("optionsNativeSendKeyEnter"));
  setText("nc-send-ctrl-enter", t("optionsNativeSendKeyCtrlEnter"));
  setText("nc-send-key-hint", t("optionsNativeSendKeyHint"));
  setText("nc-beside-label", t("optionsNativeBeside"));
  setText("nc-beside-hint", t("optionsNativeBesideHint"));
  setText("nc-beside-trigger-label", t("optionsNativeBesideTrigger"));
  setText("nc-trigger-focus", t("optionsNativeTriggerFocus"));
  setText("nc-trigger-hover", t("optionsNativeTriggerHover"));
  setText("nc-save", t("optionsNativeSave"));
  const replText = el("repl-text", HTMLTextAreaElement);
  if (replText !== null) replText.placeholder = t("optionsReplPlaceholder");
  setText("tpl-title", t("settings_templatesTitle"));
  setText("tpl-lead", t("settings_templatesLead"));
  setText("tpl-send-label", t("settings_templatesSendImmediate"));
  setText("tpl-empty", t("settings_templatesEmpty"));
  setText("tpl-add", t("settings_templatesAdd"));
  const tplNew = el("tpl-new", HTMLTextAreaElement);
  if (tplNew !== null) tplNew.placeholder = t("settings_templatesPlaceholder");
  /** The template being edited (its index), kept across a re-render. */
  let editing: number | null = null;

  function templates(): string[] {
    return [...(config?.templates ?? [])];
  }

  function saveTemplates(next: string[], done: string): void {
    if (config === null) return;
    void save({ ...config, templates: normalizeTemplates(next) }, done);
  }

  function button(label: string, onClick: () => void, quiet = true): HTMLButtonElement {
    const b = doc.createElement("button");
    b.type = "button";
    b.textContent = label;
    if (quiet) b.className = "quiet";
    b.addEventListener("click", onClick);
    return b;
  }

  function renderTemplates(list: readonly string[]): void {
    const ol = doc.getElementById("tpl-list");
    if (ol === null) return;
    ol.replaceChildren();
    list.forEach((text, i) => {
      const li = doc.createElement("li");
      const actions = doc.createElement("div");
      actions.className = "tpl-actions";
      if (editing === i) {
        const area = doc.createElement("textarea");
        area.className = "tpl-edit";
        area.rows = Math.min(8, Math.max(2, text.split("\n").length));
        area.value = text;
        li.append(area);
        actions.append(
          button(
            t("settings_templatesDone"),
            () => {
              const next = templates();
              next[i] = area.value;
              editing = null;
              saveTemplates(next, t("optionsSaved"));
            },
            false,
          ),
          button(t("settings_templatesCancel"), () => {
            editing = null;
            renderTemplates(templates());
          }),
        );
      } else {
        const body = doc.createElement("div");
        body.className = "tpl-text";
        body.textContent = text;
        li.append(body);
        const move = (to: number) => () => {
          const next = templates();
          const [item] = next.splice(i, 1);
          next.splice(to, 0, item!);
          saveTemplates(next, t("optionsSaved"));
        };
        actions.append(
          button(t("settings_templatesEdit"), () => {
            editing = i;
            renderTemplates(templates());
          }),
        );
        if (i > 0) actions.append(button(t("settings_templatesUp"), move(i - 1)));
        if (i < list.length - 1) actions.append(button(t("settings_templatesDown"), move(i + 1)));
        actions.append(
          button(t("settings_templatesDelete"), () => {
            const next = templates();
            next.splice(i, 1);
            saveTemplates(next, t("settings_templatesDeleted"));
          }),
        );
      }
      li.append(actions);
      ol.append(li);
    });
    const empty = doc.getElementById("tpl-empty");
    if (empty !== null) empty.hidden = list.length > 0;
    setText("tpl-count", t("settings_templatesCount", { count: list.length, max: MAX_TEMPLATES }));
  }

  const modeRadios = (): HTMLInputElement[] =>
    ["mode-normal", "mode-en", "mode-kana"]
      .map((id) => el(id, HTMLInputElement))
      .filter((r): r is HTMLInputElement => r !== null);

  const scaleNumber = el("nc-icon-scale", HTMLInputElement);
  const scaleRange = el("nc-icon-scale-range", HTMLInputElement);

  function fillScale(c: NativeConfig): void {
    const scale = String(c.icon.scale ?? ICON_SCALE_DEFAULT);
    if (scaleNumber !== null) scaleNumber.value = scale;
    if (scaleRange !== null) scaleRange.value = scale;
  }

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
    fillScale(c);
    const inject = el("nc-inject", HTMLSelectElement);
    if (inject !== null) inject.value = c.inject;
    const sendKey = el("nc-send-key", HTMLSelectElement);
    if (sendKey !== null) sendKey.value = c.sendKey ?? "enter";
    const beside = el("nc-beside", HTMLInputElement);
    if (beside !== null) beside.checked = c.besideField.enabled;
    const trigger = el("nc-beside-trigger", HTMLSelectElement);
    if (trigger !== null) trigger.value = c.besideField.trigger;
    const sendNow = el("tpl-send", HTMLInputElement);
    if (sendNow !== null) sendNow.checked = c.templateSendImmediate ?? false;
    renderTemplates(c.templates ?? []);
  }

  /** The desktop app's own settings from the form, on top of `base`. */
  function readNative(base: NativeConfig): NativeConfig {
    const hotkey = el("nc-hotkey", HTMLInputElement)?.value.trim() ?? "";
    const inject = el("nc-inject", HTMLSelectElement)?.value;
    const trigger = el("nc-beside-trigger", HTMLSelectElement)?.value;
    const sendKey = el("nc-send-key", HTMLSelectElement)?.value;
    return {
      ...base,
      hotkey: hotkey === "" ? null : hotkey,
      icon: {
        ...base.icon,
        visible: el("nc-icon-visible", HTMLInputElement)?.checked ?? base.icon.visible,
        hideOnFullscreen: el("nc-icon-fullscreen", HTMLInputElement)?.checked ?? base.icon.hideOnFullscreen,
        scale: clampIconScale(scaleNumber?.value ?? "") ?? base.icon.scale ?? ICON_SCALE_DEFAULT,
      },
      inject: inject === "type" || inject === "paste" || inject === "auto" ? inject : base.inject,
      sendKey: sendKey === "enter" || sendKey === "ctrl-enter" ? sendKey : (base.sendKey ?? "enter"),
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

  doc.getElementById("tpl-add")?.addEventListener("click", () => {
    if (tplNew === null || config === null) return;
    const text = normalizeTemplates([tplNew.value])[0];
    if (text === undefined) return;
    const list = templates();
    if (list.includes(text)) {
      setText("status", t("settings_templatesDuplicate"), "err");
      return;
    }
    if (list.length >= MAX_TEMPLATES) {
      setText("status", t("settings_templatesFull"), "err");
      return;
    }
    tplNew.value = "";
    saveTemplates([...list, text], t("optionsSaved"));
  });

  el("tpl-send", HTMLInputElement)?.addEventListener("change", (e) => {
    if (config === null) return;
    void save({ ...config, templateSendImmediate: (e.target as HTMLInputElement).checked }, t("optionsSaved"));
  });

  /** The mic's size goes to the desktop app at once, so the mic can be watched while choosing. */
  function saveScale(typed: string): void {
    if (config === null) return;
    const scale = clampIconScale(typed);
    if (scale === null || scale === (config.icon.scale ?? ICON_SCALE_DEFAULT)) {
      fillScale(config);
      return;
    }
    void save({ ...config, icon: { ...config.icon, scale } }, t("optionsSaved"));
  }

  scaleRange?.addEventListener("input", () => {
    if (scaleNumber !== null) scaleNumber.value = scaleRange.value;
  });
  scaleRange?.addEventListener("change", () => saveScale(scaleRange.value));
  scaleNumber?.addEventListener("change", () => saveScale(scaleNumber.value));
  doc.getElementById("nc-icon-scale-reset")?.addEventListener("click", () => saveScale(String(ICON_SCALE_DEFAULT)));

  // Ctrl+wheel over the mic resizes it (and a drag moves it) while this page is open: take that
  // in when the page is back in front, so the next save does not put the old size back. Only the
  // icon: the rest of the page may hold edits not saved yet.
  doc.defaultView?.addEventListener("focus", () => {
    if (config === null) return;
    void (async () => {
      try {
        const res = await fetchJson(url);
        const got = res.ok ? await res.json() : null;
        if (!isNativeConfig(got) || config === null) return;
        config = { ...config, icon: got.icon };
        fillScale(config);
      } catch {
        // Not reachable now; the next save says so.
      }
    })();
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
      const target = templateFromHref(href);
      if (target !== null && target < (got.templates ?? []).length) editing = target;
      fill(got);
      if (editing !== null) {
        const area = doc.querySelector<HTMLTextAreaElement>("#tpl-list textarea.tpl-edit");
        area?.scrollIntoView?.({ block: "center" });
        area?.focus();
      }
    } catch {
      setText("status", t("settings_failed"), "err");
    }
  })();

  /** What this window holds that is not saved; null when nothing. */
  function draft(): SettingsDraft | null {
    if (config === null) return null;
    const d: SettingsDraft = {};
    if (replText !== null && replText.value !== formatReplacementText(config.replacements ?? [])) {
      d.replacements = replText.value;
    }
    const form = readNative(config);
    const nativeEdited =
      form.hotkey !== (config.hotkey ?? null) ||
      form.icon.visible !== config.icon.visible ||
      form.icon.hideOnFullscreen !== config.icon.hideOnFullscreen ||
      form.inject !== config.inject ||
      form.sendKey !== (config.sendKey ?? "enter") ||
      form.besideField.enabled !== config.besideField.enabled ||
      form.besideField.trigger !== config.besideField.trigger;
    if (nativeEdited) {
      d.native = {};
      for (const id of NATIVE_FIELDS) {
        const input = el(id, HTMLInputElement);
        const select = el(id, HTMLSelectElement);
        if (input !== null) d.native[id] = input.type === "checkbox" ? input.checked : input.value;
        else if (select !== null) d.native[id] = select.value;
      }
    }
    if (tplNew !== null && tplNew.value.trim() !== "") d.newTemplate = tplNew.value;
    const original = editing === null ? undefined : templates()[editing];
    const area = doc.querySelector<HTMLTextAreaElement>("#tpl-list textarea.tpl-edit");
    if (original !== undefined && area !== null && area.value !== original) d.editing = { original, text: area.value };
    return Object.keys(d).length === 0 ? null : d;
  }

  /** Puts an older window's unsaved edits into this one. A template this window was opened to edit comes first. */
  function takeOver(d: SettingsDraft): void {
    if (config === null) return;
    if (typeof d.replacements === "string" && replText !== null) replText.value = d.replacements;
    for (const [id, value] of Object.entries(d.native ?? {})) {
      const input = el(id, HTMLInputElement);
      const select = el(id, HTMLSelectElement);
      if (input !== null && input.type === "checkbox" && typeof value === "boolean") input.checked = value;
      else if (input !== null && typeof value === "string") input.value = value;
      else if (select !== null && typeof value === "string") select.value = value;
    }
    if (typeof d.newTemplate === "string" && tplNew !== null) tplNew.value = d.newTemplate;
    const edit = d.editing;
    const index = edit === undefined ? -1 : templates().indexOf(edit.original);
    if (edit !== undefined && index >= 0 && editing === null) {
      editing = index;
      renderTemplates(templates());
      const area = doc.querySelector<HTMLTextAreaElement>("#tpl-list textarea.tpl-edit");
      if (area !== null) area.value = edit.text;
    }
  }

  const channel = options.channel ?? null;
  if (channel !== null) {
    const me = { id: globalThis.crypto?.randomUUID?.() ?? String(Math.random()), at: options.openedAt ?? Date.now() };
    const close = options.close ?? (() => doc.defaultView?.close());
    channel.addEventListener("message", (event) => {
      const m = event.data;
      if (!isChannelMessage(m)) return;
      if (m.type === "opened" && m.id !== me.id && (m.at > me.at || (m.at === me.at && m.id > me.id))) {
        const d = draft();
        if (d !== null) channel.postMessage({ type: "draft", to: m.id, draft: d });
        close();
      } else if (m.type === "draft" && m.to === me.id) {
        void loaded.then(() => takeOver(m.draft));
      }
    });
    channel.postMessage({ type: "opened", id: me.id, at: me.at });
  }

  return { loaded };
}

if (typeof document !== "undefined" && document.getElementById("nc-form") !== null) {
  initSettingsPage({ channel: typeof BroadcastChannel === "function" ? new BroadcastChannel(SETTINGS_CHANNEL) : null });
}
