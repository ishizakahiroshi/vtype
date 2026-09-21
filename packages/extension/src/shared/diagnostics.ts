// The diagnostic log: opt-in, off by default, and deliberately text-free.
//
// Why it exists: "it stopped in the middle of a breath" cannot be chased without knowing how
// many times Chrome restarted recognition, when each result arrived and why the session ended.
// Reading that from the browser console means opening DevTools, so vtype keeps a small log of
// its own that the settings page can show, copy and clear.
//
// **What it must never hold is the recognised text.** The privacy policy and the store
// submission both say transcripts are not stored. An entry keeps the time, the cycle number,
// the kind of event, how many characters arrived and why a session ended — never the words.
// `describeSessionEvent` is the only place that reads an event, and it takes `.length`.
//
// It lives in `chrome.storage.local`: it is about this browser on this machine, it would be
// pointless on another device, and sync is small enough that a debug log has no business in it.

import type { SessionEvent } from "./messages";
import type { StorageView } from "./settings";

/** Key inside `chrome.storage.local`. */
export const DIAG_LOG_KEY = "diagLog";

/** The area the log lives in. Not `sync`: this is about one browser, and sync is small. */
export const DIAG_LOG_AREA = "local";

/**
 * At most this many entries are kept, oldest dropped first. A recording produces roughly ten
 * entries, so this holds the last couple of dozen sessions: enough to find the one that went
 * wrong, small enough that it can never grow into a storage problem.
 */
export const MAX_DIAG_ENTRIES = 300;

export interface DiagEntry {
  /** `Date.now()` when the line was recorded. */
  readonly t: number;
  /** Already-formatted, text-free description. See describeSessionEvent. */
  readonly line: string;
}

function isDiagEntry(value: unknown): value is DiagEntry {
  if (typeof value !== "object" || value === null) return false;
  const e = value as { t?: unknown; line?: unknown };
  return typeof e.t === "number" && Number.isFinite(e.t) && typeof e.line === "string";
}

/** What was stored, with anything unusable dropped and the cap applied. */
export function sanitizeDiagLog(value: unknown): DiagEntry[] {
  if (!Array.isArray(value)) return [];
  const clean = value.filter(isDiagEntry).map((e) => ({ t: e.t, line: e.line }));
  return clean.slice(Math.max(0, clean.length - MAX_DIAG_ENTRIES));
}

/**
 * One session event as a line. Nothing here reads the transcript itself: a result contributes
 * its length and its flags, which is what a timing question needs.
 */
export function describeSessionEvent(event: SessionEvent): string {
  if (event.kind === "started") return `id=${event.recognitionId} started`;
  if (event.kind === "activity") return `id=${event.recognitionId} activity ${event.activity}`;
  if (event.kind === "result") {
    return `id=${event.recognitionId} result final=${event.isFinal} current=${event.isCurrent} len=${event.transcript.length}`;
  }
  return `ended reason=${event.reason}${event.code === undefined ? "" : ` code=${event.code}`}`;
}

/** The stored log, oldest first, or none at all when it cannot be read. */
export async function readDiagLog(storage: StorageView | null): Promise<DiagEntry[]> {
  const area = storage?.local;
  if (area === undefined) return [];
  try {
    const stored = await area.get([DIAG_LOG_KEY]);
    return sanitizeDiagLog(stored?.[DIAG_LOG_KEY]);
  } catch {
    return [];
  }
}

// Appends are read-modify-write, so two of them in flight would lose one. The background is a
// single context, so serialising them here is enough.
let pending: Promise<void> = Promise.resolve();

/**
 * Add one line. Never throws and never blocks the caller's own work: a full disk or a disabled
 * storage means the log is lost, which is the right trade for something only used to debug.
 */
export function appendDiag(storage: StorageView | null, line: string, now: number = Date.now()): Promise<void> {
  const area = storage?.local;
  if (area === undefined) return Promise.resolve();
  pending = pending.then(async () => {
    try {
      const entries = await readDiagLog(storage);
      entries.push({ t: now, line });
      await area.set({ [DIAG_LOG_KEY]: entries.slice(Math.max(0, entries.length - MAX_DIAG_ENTRIES)) });
    } catch {
      // ignored on purpose: see above
    }
  });
  return pending;
}

/** Forget the log. Returns how many entries were forgotten (0 when there were none). */
export async function clearDiagLog(storage: StorageView | null): Promise<number> {
  const area = storage?.local;
  if (area === undefined) return 0;
  try {
    const before = (await readDiagLog(storage)).length;
    if (before === 0) return 0;
    await area.set({ [DIAG_LOG_KEY]: [] });
    return before;
  } catch {
    return 0;
  }
}

/**
 * Call `onChange` whenever the log changes, so the settings page follows a recording made on
 * another tab without being reopened. Returns the unsubscribe function.
 */
export function watchDiagLog(storage: StorageView | null, onChange: (entries: DiagEntry[]) => void): () => void {
  const changed = storage?.onChanged;
  if (changed === undefined) return () => undefined;
  const listener = (changes: Record<string, { readonly newValue?: unknown }>, area: string): void => {
    if (area !== DIAG_LOG_AREA || !(DIAG_LOG_KEY in changes)) return;
    onChange(sanitizeDiagLog(changes[DIAG_LOG_KEY]?.newValue));
  };
  try {
    changed.addListener(listener);
  } catch {
    return () => undefined;
  }
  return () => {
    try {
      changed.removeListener(listener);
    } catch {
      // the page is going away anyway
    }
  };
}

function two(n: number): string {
  return String(n).padStart(2, "0");
}

function three(n: number): string {
  return String(n).padStart(3, "0");
}

/**
 * The log as text, one entry per line: wall clock, the gap since the previous entry, then the
 * description. The gap is what a question like "how long a pause ends it" is answered with.
 */
export function formatDiagLog(entries: readonly DiagEntry[]): string {
  let previous: number | null = null;
  return entries
    .map((entry) => {
      const d = new Date(entry.t);
      const clock = `${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())}.${three(d.getMilliseconds())}`;
      const gap = previous === null ? "" : `+${String(entry.t - previous).padStart(5, " ")}ms`;
      previous = entry.t;
      return `${clock} ${gap.padStart(9, " ")}  ${entry.line}`;
    })
    .join("\n");
}
