// What the extension and the desktop app (packages/native) say to each other over Chrome
// Native Messaging (native plan C4). The Rust side is packages/native/src/protocol.rs
// (`ToExtension` / `FromExtension`); both test against
// packages/native/tests/fixtures/nm-messages.json, so changing one side alone fails a test.
//
// The desktop app starts and stops recognition; the extension recognizes and reports back.
// Only `final` text is typed into the foreground app; `interim` is shown in a bubble.

import { isInputMode, type InputMode } from "vtype-core";

export const NATIVE_HOST = "com.ishizakahiroshi.vtype";

/** The desktop app's own settings (its config.json), shown and edited on the options page. */
export interface NativeConfig {
  readonly hotkey: string | null;
  readonly icon: {
    readonly visible: boolean;
    readonly x: number | null;
    readonly y: number | null;
    readonly hideOnFullscreen: boolean;
  };
  readonly inject: "auto" | "type" | "paste";
  readonly besideField: { readonly enabled: boolean; readonly trigger: "focus" | "hover" };
  readonly extraExtensionIds: readonly string[];
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
  | { readonly kind: "ended"; readonly reason: string; readonly code?: string };

export type ExtensionToNative =
  | { readonly type: "hello"; readonly extensionVersion: string; readonly browser?: string }
  | { readonly type: "state"; readonly mode: InputMode; readonly recording: boolean }
  | { readonly type: "session"; readonly event: NativeSessionEvent }
  | { readonly type: "set-native-config"; readonly config: NativeConfig }
  | { readonly type: "get-native-config" }
  | { readonly type: "error"; readonly code: string };

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
    (r.inject === "auto" || r.inject === "type" || r.inject === "paste") &&
    typeof beside.enabled === "boolean" &&
    (beside.trigger === "focus" || beside.trigger === "hover") &&
    Array.isArray(r.extraExtensionIds) &&
    r.extraExtensionIds.every((id) => typeof id === "string")
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
      return true;
    case "error":
      return typeof r.code === "string";
    default:
      return false;
  }
}
