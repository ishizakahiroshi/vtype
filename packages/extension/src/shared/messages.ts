// Every message that crosses an extension boundary, in one place (plan C7b C1).
//
//   content  --runtime.sendMessage-->  background   (start / stop / open-permission)
//   background --runtime.sendMessage--> offscreen   (start / stop / abort)
//   offscreen --runtime.sendMessage-->  background  (session-event)
//   background --tabs.sendMessage(tabId, msg, {frameId})--> content (session-event)
//
// runtime.sendMessage reaches every extension page that listens (background, offscreen, and
// also the permission page), and a content script's runtime.sendMessage reaches the offscreen
// document too. Every message therefore names its `target`, and each receiver drops the rest.
//
// A session is keyed by `sessionId`, chosen by the content script when the user presses the
// mic (it plays the role of a request id). The offscreen document owns the session state,
// including the owner (tab + frame) it reports back to, so the background service worker can be
// restarted by Chrome at any time without losing track of where results go.

import { isInputMode, type InputMode, type ReplacementRule } from "vtype-core";

export type EndReason =
  /** The user pressed the mic again: insert the text. */
  | "user"
  /** Nothing was heard for several recognition cycles in a row. */
  | "silence"
  /** Recognition failed (`code` says why). */
  | "error"
  /** Voice input was started in another tab or frame. */
  | "superseded"
  /** Stopped because the owning tab closed or could no longer be reached. */
  | "aborted";

/**
 * Who a session reports to: a frame of a tab (the mic beside a web field), or the desktop app's
 * own speech page (src/speech/, standalone plan C2), which has no tab at all.
 */
export type Owner = TabOwner | NativeOwner;

export interface TabOwner {
  readonly kind: "tab";
  readonly tabId: number;
  readonly frameId: number;
}

export interface NativeOwner {
  readonly kind: "native";
}

export const NATIVE_OWNER: NativeOwner = { kind: "native" };

export type SessionEvent =
  | { readonly kind: "started"; readonly recognitionId: number }
  /** vtype-core `activity` (audiostart / soundstart / speechstart / ...): drives the waveform. */
  | { readonly kind: "activity"; readonly recognitionId: number; readonly activity: string }
  | {
      readonly kind: "result";
      readonly recognitionId: number;
      readonly isCurrent: boolean;
      readonly transcript: string;
      readonly isFinal: boolean;
    }
  | { readonly kind: "ended"; readonly reason: EndReason; readonly code?: string };

// ---- content -> background ---------------------------------------------------------------

export type ContentToBackground =
  | { readonly target: "background"; readonly type: "start"; readonly sessionId: string }
  | { readonly target: "background"; readonly type: "stop"; readonly sessionId: string }
  | { readonly target: "background"; readonly type: "open-permission" };

// ---- background -> offscreen -------------------------------------------------------------

export type BackgroundToOffscreen =
  | {
      readonly target: "offscreen";
      readonly type: "start";
      readonly sessionId: string;
      readonly owner: Owner;
      /** Input mode and replacement table for this session (the background reads them from storage). */
      readonly mode: InputMode;
      readonly rules: readonly ReplacementRule[];
    }
  | { readonly target: "offscreen"; readonly type: "stop"; readonly sessionId: string }
  | {
      readonly target: "offscreen";
      readonly type: "abort";
      /** Abort this session, or (when absent) whatever session belongs to `tabId`. */
      readonly sessionId?: string;
      readonly tabId?: number;
    };

// ---- offscreen -> background -------------------------------------------------------------

export interface OffscreenToBackground {
  readonly target: "background";
  readonly type: "session-event";
  readonly sessionId: string;
  readonly owner: Owner;
  readonly event: SessionEvent;
}

// ---- background -> content ---------------------------------------------------------------

export interface BackgroundToContent {
  readonly target: "content";
  readonly type: "session-event";
  readonly sessionId: string;
  readonly event: SessionEvent;
}

/**
 * C9: the toolbar icon was pressed. The background does not know what site the tab is on
 * (vtype asks for no host permissions and no `tabs` permission), and it does not need to: the
 * page knows its own origin, so the switching on and off happens there.
 */
export interface BackgroundToContentToggleSite {
  readonly target: "content";
  readonly type: "toggle-site";
}

/**
 * What the page answers a `toggle-site` with, so the background knows the press was taken.
 * Without an answer, Chrome closes the port and the send looks like a failure even though it
 * arrived — which would send the user to the options page on every press.
 */
export const TOGGLE_SITE_ACK = "vtype:toggle-site-taken";

// ---- guards ------------------------------------------------------------------------------

function record(m: unknown): Record<string, unknown> | null {
  return typeof m === "object" && m !== null ? (m as Record<string, unknown>) : null;
}

function isOwner(o: unknown): o is Owner {
  const r = record(o);
  if (r === null) return false;
  if (r.kind === "native") return true;
  return r.kind === "tab" && typeof r.tabId === "number" && typeof r.frameId === "number";
}

function isSessionEvent(e: unknown): e is SessionEvent {
  const r = record(e);
  if (r === null) return false;
  if (r.kind === "started") return typeof r.recognitionId === "number";
  if (r.kind === "activity") return typeof r.recognitionId === "number" && typeof r.activity === "string";
  if (r.kind === "result") {
    return (
      typeof r.recognitionId === "number" &&
      typeof r.isCurrent === "boolean" &&
      typeof r.transcript === "string" &&
      typeof r.isFinal === "boolean"
    );
  }
  if (r.kind === "ended") return typeof r.reason === "string";
  return false;
}

export function isContentToBackground(m: unknown): m is ContentToBackground {
  const r = record(m);
  if (r === null || r.target !== "background") return false;
  if (r.type === "start" || r.type === "stop") return typeof r.sessionId === "string";
  return r.type === "open-permission";
}

export function isOffscreenToBackground(m: unknown): m is OffscreenToBackground {
  const r = record(m);
  return (
    r !== null &&
    r.target === "background" &&
    r.type === "session-event" &&
    typeof r.sessionId === "string" &&
    isOwner(r.owner) &&
    isSessionEvent(r.event)
  );
}

export function isBackgroundToOffscreen(m: unknown): m is BackgroundToOffscreen {
  const r = record(m);
  if (r === null || r.target !== "offscreen") return false;
  if (r.type === "start") {
    return typeof r.sessionId === "string" && isOwner(r.owner) && isInputMode(r.mode) && Array.isArray(r.rules);
  }
  if (r.type === "stop") return typeof r.sessionId === "string";
  return r.type === "abort";
}

export function isBackgroundToContent(m: unknown): m is BackgroundToContent {
  const r = record(m);
  return (
    r !== null &&
    r.target === "content" &&
    r.type === "session-event" &&
    typeof r.sessionId === "string" &&
    isSessionEvent(r.event)
  );
}

export function isToggleSite(m: unknown): m is BackgroundToContentToggleSite {
  const r = record(m);
  return r !== null && r.target === "content" && r.type === "toggle-site";
}

export const OFFSCREEN_PATH = "offscreen.html";
export const PERMISSION_PATH = "permission.html";
