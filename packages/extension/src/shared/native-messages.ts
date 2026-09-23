// What the desktop app (packages/native) and its speech page (src/speech/) say to each other
// over the page's WebSocket (standalone plan C3; the messages were first the extension's Native
// Messaging bridge, removed in standalone plan C6). The Rust side is
// packages/native/src/protocol.rs (`ToExtension` / `FromExtension`); both test against
// packages/native/tests/fixtures/nm-messages.json, so changing one side alone fails a test.
//
// The desktop app starts and stops recognition; the page recognizes and reports back.
// Only `final` text is typed into the foreground app; `interim` is shown in a bubble.

import { isInputMode, type InputMode, type ReplacementRule } from "vtype-core";
import type { SessionEvent } from "./messages";

/** The desktop app's own settings (its config.json), shown and edited on its settings page. */
export interface NativeConfig {
  readonly hotkey: string | null;
  readonly icon: {
    readonly visible: boolean;
    readonly x: number | null;
    readonly y: number | null;
    readonly hideOnFullscreen: boolean;
    /** The floating mic's size in percent (50–200); older desktop apps leave it out. */
    readonly scale?: number;
  };
  readonly inject: "auto" | "type" | "paste";
  readonly besideField: {
    readonly enabled: boolean;
    readonly trigger: "focus" | "hover";
    /** Also in Chrome's fields; older desktop apps leave it out (they never showed it there). */
    readonly inChrome?: boolean;
  };
  readonly extraExtensionIds: readonly string[];
  /** The desktop app's own input mode and replacement table (standalone plan C5). */
  readonly inputMode?: InputMode;
  readonly replacements?: readonly ReplacementRule[];
  /** What the floating mic's send button presses (Ctrl+Enter is ⌘+Enter on macOS). */
  readonly sendKey?: "enter" | "ctrl-enter";
  /** Stop recording this many seconds after the last new words (0 = off, at most 10). */
  readonly silenceStopSec?: number;
  /** Texts put in with one click from the floating mic's top-left button. */
  readonly templates?: readonly string[];
  /** Press the send key right after a template went in. */
  readonly templateSendImmediate?: boolean;
}

export type NativeToExtension =
  | { readonly type: "hello"; readonly nativeVersion: string; readonly os: string }
  | { readonly type: "start"; readonly mode?: InputMode }
  | { readonly type: "stop" }
  | { readonly type: "set-mode"; readonly mode: InputMode }
  | { readonly type: "get-state" }
  | { readonly type: "native-config"; readonly config: NativeConfig }
  | { readonly type: "open-options" };

export type NativeSessionEvent =
  | { readonly kind: "started" }
  | { readonly kind: "interim"; readonly text: string }
  | { readonly kind: "final"; readonly text: string }
  /** soundstart / speechstart / speechend …: drives the ripple around the desktop app's mic. */
  | { readonly kind: "activity"; readonly activity: string }
  | { readonly kind: "ended"; readonly reason: string; readonly code?: string };

export type ExtensionToNative =
  | { readonly type: "hello"; readonly extensionVersion: string; readonly browser?: string }
  | { readonly type: "state"; readonly mode: InputMode; readonly recording: boolean }
  | { readonly type: "session"; readonly event: NativeSessionEvent }
  | { readonly type: "set-native-config"; readonly config: NativeConfig }
  | { readonly type: "get-native-config" }
  | { readonly type: "error"; readonly code: string }
  // The desktop app's own speech page (standalone plan C2). The Rust side reads these from C4 on.
  /**
   * The user pressed "agree and start" on the speech page, with "start vtype when you sign in"
   * ticked or not (older pages leave `autostart` out).
   */
  | { readonly type: "consent"; readonly autostart?: boolean }
  /** Where the speech page's first-run setup stands. */
  | { readonly type: "page-state"; readonly consented: boolean; readonly micGranted: boolean }
  /** The user clicked the hidden speech window's taskbar button: open the desktop app's settings. */
  | { readonly type: "open-settings" };

/**
 * A recognition session's event as the desktop app hears it: the text, final or interim, and the
 * recognizer's activity (which drives the ripple around the desktop app's mic). Used by the
 * desktop app's speech page.
 */
export function toNativeEvent(event: SessionEvent): NativeSessionEvent | null {
  switch (event.kind) {
    case "started":
      return { kind: "started" };
    case "result":
      return event.isFinal ? { kind: "final", text: event.transcript } : { kind: "interim", text: event.transcript };
    case "activity":
      return { kind: "activity", activity: event.activity };
    case "ended":
      return event.code === undefined
        ? { kind: "ended", reason: event.reason }
        : { kind: "ended", reason: event.reason, code: event.code };
    default:
      return null;
  }
}

// ---- guards ------------------------------------------------------------------------------

function record(m: unknown): Record<string, unknown> | null {
  return typeof m === "object" && m !== null && !Array.isArray(m) ? (m as Record<string, unknown>) : null;
}

const isNullableInt = (v: unknown): boolean => v === null || (typeof v === "number" && Number.isInteger(v));

export function isNativeConfig(v: unknown): v is NativeConfig {
  const r = record(v);
  const icon = record(r?.icon);
  const beside = record(r?.besideField);
  return (
    r !== null &&
    icon !== null &&
    beside !== null &&
    (r.hotkey === null || typeof r.hotkey === "string") &&
    typeof icon.visible === "boolean" &&
    isNullableInt(icon.x) &&
    isNullableInt(icon.y) &&
    typeof icon.hideOnFullscreen === "boolean" &&
    (icon.scale === undefined || (typeof icon.scale === "number" && Number.isInteger(icon.scale))) &&
    (r.inject === "auto" || r.inject === "type" || r.inject === "paste") &&
    typeof beside.enabled === "boolean" &&
    (beside.trigger === "focus" || beside.trigger === "hover") &&
    (beside.inChrome === undefined || typeof beside.inChrome === "boolean") &&
    Array.isArray(r.extraExtensionIds) &&
    r.extraExtensionIds.every((id) => typeof id === "string") &&
    (r.inputMode === undefined || isInputMode(r.inputMode)) &&
    (r.replacements === undefined || Array.isArray(r.replacements)) &&
    (r.sendKey === undefined || r.sendKey === "enter" || r.sendKey === "ctrl-enter") &&
    (r.silenceStopSec === undefined || typeof r.silenceStopSec === "number") &&
    (r.templates === undefined || (Array.isArray(r.templates) && r.templates.every((s) => typeof s === "string"))) &&
    (r.templateSendImmediate === undefined || typeof r.templateSendImmediate === "boolean")
  );
}

export function isNativeToExtension(m: unknown): m is NativeToExtension {
  const r = record(m);
  if (r === null) return false;
  switch (r.type) {
    case "hello":
      return typeof r.nativeVersion === "string" && typeof r.os === "string";
    case "start":
      return r.mode === undefined || isInputMode(r.mode);
    case "set-mode":
      return isInputMode(r.mode);
    case "native-config":
      return isNativeConfig(r.config);
    case "stop":
    case "get-state":
    case "open-options":
      return true;
    default:
      return false;
  }
}

function isNativeSessionEvent(e: unknown): e is NativeSessionEvent {
  const r = record(e);
  if (r === null) return false;
  if (r.kind === "started") return true;
  if (r.kind === "interim" || r.kind === "final") return typeof r.text === "string";
  if (r.kind === "activity") return typeof r.activity === "string";
  if (r.kind === "ended") return typeof r.reason === "string" && (r.code === undefined || typeof r.code === "string");
  return false;
}

export function isExtensionToNative(m: unknown): m is ExtensionToNative {
  const r = record(m);
  if (r === null) return false;
  switch (r.type) {
    case "hello":
      return typeof r.extensionVersion === "string" && (r.browser === undefined || typeof r.browser === "string");
    case "state":
      return isInputMode(r.mode) && typeof r.recording === "boolean";
    case "session":
      return isNativeSessionEvent(r.event);
    case "set-native-config":
      return isNativeConfig(r.config);
    case "get-native-config":
    case "open-settings":
      return true;
    case "consent":
      return r.autostart === undefined || typeof r.autostart === "boolean";
    case "page-state":
      return typeof r.consented === "boolean" && typeof r.micGranted === "boolean";
    case "error":
      return typeof r.code === "string";
    default:
      return false;
  }
}
