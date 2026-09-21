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

/** Which fields carry a mic (C7g). */
export type MicDisplay =
  /** Every target field that is visible on screen, without touching anything. */
  | "all"
  /** Only the field the pointer is on, and the field that has the caret. */
  | "hover";

export const DEFAULT_MIC_DISPLAY: MicDisplay = "all";

/**
 * Whether vtype keeps a diagnostic log (shared/diagnostics.ts). Off by default: it exists to
 * answer "why did it stop there", and nobody needs it until something looks wrong.
 */
export const DEFAULT_DIAGNOSTICS = false;

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
export const MIC_DISPLAY_KEY = "micDisplay";
export const EXCLUDED_KEY = "excludedSites";
export const DIAGNOSTICS_KEY = "diagnostics";
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
  /**
   * Only the excluded sites use this one, and only as a fallback: see EXCLUDED_AREAS. The
   * other settings are sync-only, because losing them costs the user a click, not a promise.
   */
  local?: StorageAreaView;
  onChanged?: {
    addListener(listener: StorageChangeListener): void;
    removeListener(listener: StorageChangeListener): void;
  };
}

export function isTriggerMode(value: unknown): value is TriggerMode {
  return value === "click" || value === "hover";
}

export function isMicDisplay(value: unknown): value is MicDisplay {
  return value === "all" || value === "hover";
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

/** The stored mic display, or the default when it is unset, unreadable or unknown (C7g). */
export async function readMicDisplay(storage: StorageView | null): Promise<MicDisplay> {
  const area = storage?.sync;
  if (area === undefined) return DEFAULT_MIC_DISPLAY;
  try {
    const stored = await area.get([MIC_DISPLAY_KEY]);
    const value = stored?.[MIC_DISPLAY_KEY];
    return isMicDisplay(value) ? value : DEFAULT_MIC_DISPLAY;
  } catch {
    return DEFAULT_MIC_DISPLAY;
  }
}

export async function writeMicDisplay(storage: StorageView | null, display: MicDisplay): Promise<boolean> {
  const area = storage?.sync;
  if (area === undefined) return false;
  try {
    await area.set({ [MIC_DISPLAY_KEY]: display });
    return true;
  } catch {
    return false;
  }
}

/** Whether the diagnostic log is on. Off unless it is stored as exactly `true`. */
export async function readDiagnostics(storage: StorageView | null): Promise<boolean> {
  const area = storage?.sync;
  if (area === undefined) return DEFAULT_DIAGNOSTICS;
  try {
    const stored = await area.get([DIAGNOSTICS_KEY]);
    return stored?.[DIAGNOSTICS_KEY] === true;
  } catch {
    return DEFAULT_DIAGNOSTICS;
  }
}

/** Turn the diagnostic log on or off. False when it could not be stored. */
export async function writeDiagnostics(storage: StorageView | null, on: boolean): Promise<boolean> {
  const area = storage?.sync;
  if (area === undefined) return false;
  try {
    await area.set({ [DIAGNOSTICS_KEY]: on });
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

/** The same for which fields carry a mic (C7g). */
export function watchMicDisplay(storage: StorageView | null, onChange: (display: MicDisplay) => void): () => void {
  return watchKey(storage, MIC_DISPLAY_KEY, (value) =>
    onChange(isMicDisplay(value) ? value : DEFAULT_MIC_DISPLAY),
  );
}

/** The same for the diagnostic log, so the background starts and stops recording at once. */
export function watchDiagnostics(storage: StorageView | null, onChange: (on: boolean) => void): () => void {
  return watchKey(storage, DIAGNOSTICS_KEY, (value) => onChange(value === true));
}

/** The same for the dragged positions: a reset in the options page arrives here. */
export function watchOffsets(storage: StorageView | null, onChange: (offsets: MicOffsets) => void): () => void {
  return watchKey(storage, OFFSETS_KEY, (value) => onChange(sanitizeOffsets(value)));
}

function watchKey(
  storage: StorageView | null,
  key: string,
  onValue: (value: unknown) => void,
  areas: readonly string[] = [SETTINGS_AREA],
): () => void {
  const events = storage?.onChanged;
  if (events === undefined) return () => undefined;
  const listener: StorageChangeListener = (changes, area) => {
    if (!areas.includes(area)) return;
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

// ---- C9: sites vtype stays off on -------------------------------------------------------
//
// The default is "every site". This list is what the user has taken away, so it is the one
// setting whose loss is visible as vtype doing something the user told it not to do. That is
// why it is the only one that also writes to `chrome.storage.local`: sync is refused outright
// under some enterprise policies, and "the mic came back on the site I switched it off on" is
// not an acceptable outcome of that. Both areas are written and their contents are read as one
// list, so removing an entry removes it everywhere.

/**
 * One entry of the list, in one of two shapes:
 *   `https://example.com`   exactly this origin (scheme and port included)
 *   `*.example.com`         this host and everything under it, on http and https alike
 */
export type ExcludedSites = readonly string[];

/**
 * At most this many sites. The same reason as MAX_OFFSET_ORIGINS: chrome.storage.sync allows
 * about 8 KB per item, and a list that grows without a limit starts failing to save one day,
 * on whatever site the user happens to be on. The oldest entries go first.
 */
export const MAX_EXCLUDED_SITES = 100;

/** Where the list is kept, in the order it is written and read. */
export const EXCLUDED_AREAS: readonly string[] = [SETTINGS_AREA, "local"];

function parseUrl(text: string): URL | null {
  try {
    return new URL(text);
  } catch {
    return null;
  }
}

/** The host of an origin (`https://a.example.com:8443` -> `a.example.com`), or null. */
function hostOf(origin: string): string | null {
  const url = parseUrl(origin);
  return url === null || url.hostname === "" ? null : url.hostname;
}

/** A bare host, normalised the way the URL parser would (lower case, IDN -> punycode). */
function normalizeHost(raw: string): string | null {
  const text = raw.trim().replace(/\/+$/, "");
  if (text === "" || /[\s*/]/.test(text)) return null;
  const url = parseUrl(`https://${text}`);
  // A port or a path in a wildcard entry would silently not mean what it looks like.
  if (url === null || url.hostname === "" || url.port !== "" || url.pathname !== "/") return null;
  return url.hostname;
}

/**
 * What the user typed, as an entry of the list — or null when it cannot be read as a site.
 * `example.com`, `https://example.com/inbox` and `HTTPS://Example.com` all become
 * `https://example.com`; `*.example.com` stays a wildcard. Anything else (a scheme vtype does
 * not run on, an empty string, a stray `*`) is refused, so the options page can say so instead
 * of storing something that will never match.
 */
export function normalizeExclusion(input: unknown): string | null {
  if (typeof input !== "string") return null;
  const text = input.trim();
  if (text === "") return null;
  if (text.startsWith("*.")) {
    const host = normalizeHost(text.slice(2));
    return host === null ? null : `*.${host}`;
  }
  // Everything from here on is one site, so a `*` left in it would read as a wildcard that is
  // not one: `new URL("https://*")` is accepted and would be stored as a site called `*`.
  if (/[\s*]/.test(text)) return null;
  // A bare host has no scheme, so it is tried as https:// too; `http://x` keeps its scheme.
  const url = parseUrl(text) ?? parseUrl(`https://${text}`);
  if (url === null || (url.protocol !== "http:" && url.protocol !== "https:")) return null;
  return url.origin;
}

/** Whether one entry covers `origin` (`https://example.com`). Both are matched lower case. */
export function matchesExclusion(pattern: string, origin: string): boolean {
  if (pattern === "" || origin === "") return false;
  const site = origin.toLowerCase();
  if (!pattern.startsWith("*.")) return pattern.toLowerCase() === site;
  const suffix = pattern.slice(2).toLowerCase();
  const host = hostOf(site);
  if (host === null || suffix === "") return false;
  // `example.com.evil.test` must not be caught by `*.example.com`: only a dot may precede it.
  return host === suffix || host.endsWith(`.${suffix}`);
}

/** Whether vtype stays off on this origin. An empty list means it runs everywhere. */
export function isExcluded(sites: ExcludedSites, origin: string): boolean {
  return sites.some((pattern) => matchesExclusion(pattern, origin));
}

/**
 * What was stored, with anything unusable dropped and the rest normalised: another version of
 * vtype, a hand-edited value or a half-written sync must not switch the list off wholesale.
 */
export function sanitizeExcludedSites(value: unknown): ExcludedSites {
  if (!Array.isArray(value)) return [];
  const clean: string[] = [];
  for (const entry of value) {
    const pattern = normalizeExclusion(entry);
    if (pattern !== null && !clean.includes(pattern)) clean.push(pattern);
  }
  return clean.slice(Math.max(0, clean.length - MAX_EXCLUDED_SITES));
}

/**
 * `sites` with `input` added, capped at MAX_EXCLUDED_SITES (the oldest go first). Unchanged
 * when the input cannot be read as a site or is already covered. Pure: the caller stores it.
 */
export function withExcluded(sites: ExcludedSites, input: unknown): ExcludedSites {
  const pattern = normalizeExclusion(input);
  if (pattern === null || sites.includes(pattern)) return [...sites];
  const kept = sites.slice(Math.max(0, sites.length - (MAX_EXCLUDED_SITES - 1)));
  return [...kept, pattern];
}

/**
 * `sites` with every entry that covers `origin` removed. Every one of them, because the site
 * was switched off once as far as the user is concerned: leaving a `*.example.com` behind
 * after removing `https://a.example.com` would look like the button did nothing.
 */
export function withoutExcluded(sites: ExcludedSites, origin: string): ExcludedSites {
  return sites.filter((pattern) => !matchesExclusion(pattern, origin));
}

/** `sites` with `pattern` removed exactly as it is written (the options page's list). */
export function withoutExclusionEntry(sites: ExcludedSites, pattern: string): ExcludedSites {
  return sites.filter((entry) => entry !== pattern);
}

function areasOf(storage: StorageView | null): StorageAreaView[] {
  return [storage?.sync, storage?.local].filter((area): area is StorageAreaView => area !== undefined);
}

/** Every stored entry, from both areas, as one list. Empty when nothing can be read. */
export async function readExcludedSites(storage: StorageView | null): Promise<ExcludedSites> {
  const found: unknown[] = [];
  for (const area of areasOf(storage)) {
    try {
      const stored = await area.get([EXCLUDED_KEY]);
      const value = stored?.[EXCLUDED_KEY];
      if (Array.isArray(value)) found.push(...value);
    } catch {
      // This area is unreadable (policy, no extension context): the other one may not be.
    }
  }
  return sanitizeExcludedSites(found);
}

/**
 * Store the whole list, in every area there is. True when at least one write went through;
 * false means nothing was stored and the caller must not claim the site was switched off.
 */
export async function writeExcludedSites(storage: StorageView | null, sites: ExcludedSites): Promise<boolean> {
  let stored = false;
  for (const area of areasOf(storage)) {
    try {
      await area.set({ [EXCLUDED_KEY]: [...sites] });
      stored = true;
    } catch {
      // Sync can be refused by policy or full; local is then what keeps the list.
    }
  }
  return stored;
}

/**
 * Call `onChange` when the list changes elsewhere (the options page, another tab, another
 * device), so a page switches on or off without being reloaded.
 */
export function watchExcludedSites(
  storage: StorageView | null,
  onChange: (sites: ExcludedSites) => void,
): () => void {
  // A cleared list means "vtype runs everywhere again", not "keep the old list".
  return watchKey(storage, EXCLUDED_KEY, (value) => onChange(sanitizeExcludedSites(value)), EXCLUDED_AREAS);
}
