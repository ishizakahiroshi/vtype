// The extension's settings: what they are, where they live, and how to read them safely.
//
// Shared by the content script (which obeys the setting) and the options page (which writes
// it), so the key, the default and the shape are written down once. It lives next to
// messages.ts for the same reason: both sides of a boundary have to agree on it.
//
// Everything here is fail-safe. `chrome.storage` is missing in a plain page, can be disabled
// by policy, and rejects when the sync quota is exhausted; none of that may stop voice input
// from working, so every failure falls back to the default (`click`).

/** What starts a recording (C7e). */
export type TriggerMode =
  /** Press the thin mic beside the field. Hovering only opens the panel. */
  | "click"
  /** The panel opening (hover) starts it by itself. The thin mic still starts and stops. */
  | "hover";

export const DEFAULT_TRIGGER: TriggerMode = "click";

/**
 * How far the thin mic was dragged from where vtype puts it, in CSS pixels (C7f). Kept as a
 * distance from the field, never as an absolute position: the field moves with the page.
 */
export interface MicOffset {
  readonly x: number;
  readonly y: number;
}

export const NO_OFFSET: MicOffset = { x: 0, y: 0 };

/** Dragged positions, one per origin (`https://example.test`). */
export type MicOffsets = Readonly<Record<string, MicOffset>>;

/**
 * At most this many origins are remembered. chrome.storage.sync is small (about 100 KB, 8 KB
 * per item): without a cap the whole write would start failing one day, on the site the user
 * happens to be on. The oldest entries go first.
 */
export const MAX_OFFSET_ORIGINS = 50;

/** Key inside `chrome.storage.sync`. */
export const TRIGGER_KEY = "trigger";
export const OFFSETS_KEY = "micOffsets";
/** The area the setting lives in (it follows the user's Chrome profile). */
export const SETTINGS_AREA = "sync";

export interface StorageAreaView {
  get(keys: string | string[] | null): Promise<Record<string, unknown>> | void;
  set(items: Record<string, unknown>): Promise<void> | void;
}

export interface StorageChange {
  readonly oldValue?: unknown;
  readonly newValue?: unknown;
}

export type StorageChangeListener = (changes: Record<string, StorageChange>, area: string) => void;

/** The part of `chrome.storage` this extension touches, typed narrowly. */
export interface StorageView {
  sync?: StorageAreaView;
  onChanged?: {
    addListener(listener: StorageChangeListener): void;
    removeListener(listener: StorageChangeListener): void;
  };
}

export function isTriggerMode(value: unknown): value is TriggerMode {
  return value === "click" || value === "hover";
}

function isFinitePixel(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

export function isMicOffset(value: unknown): value is MicOffset {
  if (typeof value !== "object" || value === null) return false;
  const o = value as { x?: unknown; y?: unknown };
  return isFinitePixel(o.x) && isFinitePixel(o.y);
}

/**
 * What was stored, with anything unusable dropped: another version of vtype, a half-written
 * sync or a hand-edited value must not stop the rest from working.
 */
export function sanitizeOffsets(value: unknown): MicOffsets {
  if (typeof value !== "object" || value === null) return {};
  const clean: Record<string, MicOffset> = {};
  for (const [origin, offset] of Object.entries(value as Record<string, unknown>)) {
    if (origin !== "" && isMicOffset(offset)) clean[origin] = { x: offset.x, y: offset.y };
  }
  return clean;
}

/**
 * `offsets` with `origin` set to `offset`, capped at MAX_OFFSET_ORIGINS (the oldest origins
 * are dropped). A zero offset removes the entry instead of storing "no change".
 * Pure: the caller decides whether to store the result.
 */
export function withOffset(offsets: MicOffsets, origin: string, offset: MicOffset): MicOffsets {
  const next: Record<string, MicOffset> = { ...offsets };
  if (origin === "") return next;
  delete next[origin]; // re-inserted below, so the origins in use stay the newest
  if (offset.x === 0 && offset.y === 0) return next;
  const entries = Object.entries(next);
  const kept = entries.slice(Math.max(0, entries.length - (MAX_OFFSET_ORIGINS - 1)));
  return { ...Object.fromEntries(kept), [origin]: { x: offset.x, y: offset.y } };
}

/** `chrome.storage` when there is one, else null (a page outside the extension, or a test). */
export function extensionStorage(): StorageView | null {
  const storage = (globalThis as { chrome?: { storage?: StorageView } }).chrome?.storage;
  return storage ?? null;
}

/** The stored mode, or the default when it is unset, unreadable or not one of the two values. */
export async function readTrigger(storage: StorageView | null): Promise<TriggerMode> {
  const area = storage?.sync;
  if (area === undefined) return DEFAULT_TRIGGER;
  try {
    const stored = await area.get([TRIGGER_KEY]);
    const value = stored?.[TRIGGER_KEY];
    return isTriggerMode(value) ? value : DEFAULT_TRIGGER;
  } catch {
    return DEFAULT_TRIGGER;
  }
}

/** Store the mode. False when it could not be stored (the caller decides what to say). */
export async function writeTrigger(storage: StorageView | null, mode: TriggerMode): Promise<boolean> {
  const area = storage?.sync;
  if (area === undefined) return false;
  try {
    await area.set({ [TRIGGER_KEY]: mode });
    return true;
  } catch {
    return false;
  }
}

/** The stored offsets, or none at all when they cannot be read. */
export async function readOffsets(storage: StorageView | null): Promise<MicOffsets> {
  const area = storage?.sync;
  if (area === undefined) return {};
  try {
    const stored = await area.get([OFFSETS_KEY]);
    return sanitizeOffsets(stored?.[OFFSETS_KEY]);
  } catch {
    return {};
  }
}

/** Store the whole map. False when it could not be stored (the position is then not kept). */
export async function writeOffsets(storage: StorageView | null, offsets: MicOffsets): Promise<boolean> {
  const area = storage?.sync;
  if (area === undefined) return false;
  try {
    await area.set({ [OFFSETS_KEY]: offsets });
    return true;
  } catch {
    return false;
  }
}

/** Forget every dragged position. Returns how many origins were forgotten (0 when none). */
export async function clearOffsets(storage: StorageView | null): Promise<number> {
  const area = storage?.sync;
  if (area === undefined) return 0;
  try {
    const before = Object.keys(await readOffsets(storage)).length;
    if (before === 0) return 0;
    await area.set({ [OFFSETS_KEY]: {} });
    return before;
  } catch {
    return 0;
  }
}

/**
 * Call `onChange` whenever the setting changes elsewhere (the options page, another device),
 * so an open page switches without being reloaded. Returns the unsubscribe function; it is a
 * no-op when there is no storage to listen to.
 */
export function watchTrigger(storage: StorageView | null, onChange: (mode: TriggerMode) => void): () => void {
  // A cleared setting means "back to the default", not "keep the old mode".
  return watchKey(storage, TRIGGER_KEY, (value) => onChange(isTriggerMode(value) ? value : DEFAULT_TRIGGER));
}

/** The same for the dragged positions: a reset in the options page arrives here. */
export function watchOffsets(storage: StorageView | null, onChange: (offsets: MicOffsets) => void): () => void {
  return watchKey(storage, OFFSETS_KEY, (value) => onChange(sanitizeOffsets(value)));
}

function watchKey(storage: StorageView | null, key: string, onValue: (value: unknown) => void): () => void {
  const events = storage?.onChanged;
  if (events === undefined) return () => undefined;
  const listener: StorageChangeListener = (changes, area) => {
    if (area !== SETTINGS_AREA) return;
    const change = changes[key];
    if (change === undefined) return;
    onValue(change.newValue);
  };
  try {
    events.addListener(listener);
  } catch {
    return () => undefined;
  }
  return () => {
    try {
      events.removeListener(listener);
    } catch {
      // Nothing to undo: the extension context is gone, and so is the listener.
    }
  };
}
