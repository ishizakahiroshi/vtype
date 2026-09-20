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

/** Key inside `chrome.storage.sync`. */
export const TRIGGER_KEY = "trigger";
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

/**
 * Call `onChange` whenever the setting changes elsewhere (the options page, another device),
 * so an open page switches without being reloaded. Returns the unsubscribe function; it is a
 * no-op when there is no storage to listen to.
 */
export function watchTrigger(storage: StorageView | null, onChange: (mode: TriggerMode) => void): () => void {
  const events = storage?.onChanged;
  if (events === undefined) return () => undefined;
  const listener: StorageChangeListener = (changes, area) => {
    if (area !== SETTINGS_AREA) return;
    const change = changes[TRIGGER_KEY];
    if (change === undefined) return;
    // A cleared setting means "back to the default", not "keep the old mode".
    onChange(isTriggerMode(change.newValue) ? change.newValue : DEFAULT_TRIGGER);
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
