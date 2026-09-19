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

export interface Owner {
  readonly tabId: number;
  readonly frameId: number;
}

export type SessionEvent =
  | { readonly kind: "started"; readonly recognitionId: number }
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
  | { readonly target: "offscreen"; readonly type: "start"; readonly sessionId: string; readonly owner: Owner }
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

// ---- guards ------------------------------------------------------------------------------

function record(m: unknown): Record<string, unknown> | null {
  return typeof m === "object" && m !== null ? (m as Record<string, unknown>) : null;
}

function isOwner(o: unknown): o is Owner {
  const r = record(o);
  return r !== null && typeof r.tabId === "number" && typeof r.frameId === "number";
}

function isSessionEvent(e: unknown): e is SessionEvent {
  const r = record(e);
  if (r === null) return false;
  if (r.kind === "started") return typeof r.recognitionId === "number";
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
  if (r.type === "start") return typeof r.sessionId === "string" && isOwner(r.owner);
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

export const OFFSCREEN_PATH = "offscreen.html";
export const PERMISSION_PATH = "permission.html";
