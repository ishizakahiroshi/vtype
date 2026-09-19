// Insert confirmed text at the caret of a target field (plan C6).
//
// Three rules, each from a way this goes wrong on real pages:
//
// 1. input / textarea: never `field.value = ...`. The value is written through the native
//    prototype setter and an `input` event (bubbles) follows, so frameworks that track the
//    value (React's controlled inputs) see the change as user input.
// 2. contenteditable: `document.execCommand("insertText")` first. Rich editors (webmail bodies,
//    ProseMirror, Lexical, Draft.js) keep their own document model and revert or ignore direct
//    DOM edits; insertText goes through the browser's editing pipeline, so they get the
//    `beforeinput` / `input` they listen to. Only when execCommand is missing or returns false
//    do we fall back to Selection/Range plus an `input` event.
// 3. IME: while the field is composing (compositionstart .. compositionend) nothing is
//    inserted. Requests wait and are flushed in order after compositionend.
//
// Focus is not assumed (the user may have clicked the vtype panel):
// - input / textarea: selectionStart / selectionEnd persist without focus and are used as-is.
//   The field is not focused.
// - contenteditable: execCommand only acts on the focused editing host and the current
//   selection, so the host is focused first when it is not, and if the selection is not inside
//   the host the caret is placed at the end of the host's content.
// In both cases a non-collapsed selection is replaced by the text, and the caret ends up right
// after the inserted text.

import { resolveTarget } from "./detect";

/** Which mechanism put the text in. */
export type InsertPath = "native-setter" | "execCommand" | "range";

export type InsertResult =
  | { ok: true; path: InsertPath; waitedForComposition: boolean }
  | { ok: false; reason: "not-a-target" | "disconnected" | "empty-text" | "composition-timeout" };

export interface InsertOptions {
  /**
   * Give up (without inserting) if the field is still composing after this long. The text is
   * not lost: the caller gets `composition-timeout` and keeps it (C7b leaves it in the panel).
   * Default: COMPOSITION_TIMEOUT_MS.
   */
  compositionTimeoutMs?: number;
}

/**
 * Upper bound on waiting for compositionend. Chrome ends a composition on commit, on Escape and
 * on blur, so a real wait is short; this only guards against an IME that never sends
 * compositionend, where waiting forever would silently swallow the dictated text.
 */
export const COMPOSITION_TIMEOUT_MS = 10_000;

// ---- native value access ----------------------------------------------------------------

type TextControl = HTMLInputElement | HTMLTextAreaElement;

function isTextControl(el: Element): el is TextControl {
  return el.localName === "input" || el.localName === "textarea";
}

function nativeValueDescriptor(el: TextControl): PropertyDescriptor {
  // The prototype of the element's own realm, not the instance: a framework may shadow `value`
  // on the instance (React's value tracker does), and that override must be bypassed.
  const view = (el.ownerDocument.defaultView ?? globalThis) as typeof globalThis;
  const proto = el.localName === "input" ? view.HTMLInputElement.prototype : view.HTMLTextAreaElement.prototype;
  const descriptor = Object.getOwnPropertyDescriptor(proto, "value");
  if (descriptor?.get === undefined || descriptor.set === undefined) {
    throw new Error(`no native value accessor on ${el.localName}`);
  }
  return descriptor;
}

export function readNativeValue(el: TextControl): string {
  return nativeValueDescriptor(el).get!.call(el) as string;
}

/** Write through the native setter, then announce it like user input. */
export function writeNativeValue(el: TextControl, next: string, inputType: string, data: string | null): void {
  nativeValueDescriptor(el).set!.call(el, next);
  dispatchInput(el, inputType, data);
}

export function dispatchInput(target: Element, inputType: string, data: string | null): void {
  const view = (target.ownerDocument.defaultView ?? globalThis) as typeof globalThis;
  const Ctor = typeof view.InputEvent === "function" ? view.InputEvent : view.Event;
  target.dispatchEvent(new Ctor("input", { bubbles: true, composed: true, inputType, data }));
}

// ---- IME composition tracking -----------------------------------------------------------

interface CompositionState {
  /** composedPath() of the compositionstart currently in progress, or null. */
  path: EventTarget[] | null;
  waiters: Set<() => void>;
}

const tracking = new WeakMap<Document, CompositionState>();

/**
 * Start watching composition on `doc` (idempotent). Listening happens on the document in the
 * capture phase; composition events are composed, so fields in open shadow roots are covered.
 * A composition that began before tracking started is not seen, so call this when the content
 * script starts (insertAtCursor also calls it, which covers every later composition).
 */
export function startCompositionTracking(doc: Document = document): void {
  if (tracking.has(doc)) return;
  const state: CompositionState = { path: null, waiters: new Set() };
  tracking.set(doc, state);
  doc.addEventListener(
    "compositionstart",
    (e) => {
      state.path = e.composedPath();
    },
    true,
  );
  doc.addEventListener(
    "compositionend",
    () => {
      state.path = null;
      const waiters = [...state.waiters];
      state.waiters.clear();
      for (const wake of waiters) wake();
    },
    true,
  );
}

/** Whether `field` (or something inside it) is in the middle of an IME composition. */
export function isComposing(field: Element): boolean {
  const path = tracking.get(field.ownerDocument)?.path;
  return path !== null && path !== undefined && path.includes(field);
}

/** Resolves true once `field` is not composing, false on timeout. */
function waitForCompositionEnd(field: Element, timeoutMs: number): Promise<boolean> {
  const state = tracking.get(field.ownerDocument);
  if (state === undefined || !isComposing(field)) return Promise.resolve(true);
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      state.waiters.delete(wake);
      resolve(false);
    }, timeoutMs);
    function wake(): void {
      clearTimeout(timer);
      // Let the browser finish committing the composed text before we touch the field.
      setTimeout(() => resolve(true), 0);
    }
    state.waiters.add(wake);
  });
}

// ---- insertion --------------------------------------------------------------------------

/** Per-field queue: requests run one after another in call order, each exactly once. */
const queues = new WeakMap<Element, Promise<unknown>>();

/**
 * Insert `text` at the caret of `field`. Resolves after any IME wait. `field` must be a vtype
 * target (detect.ts): password, readonly and disabled fields are refused with `not-a-target`.
 */
export function insertAtCursor(field: Element, text: string, options: InsertOptions = {}): Promise<InsertResult> {
  startCompositionTracking(field.ownerDocument);
  const timeoutMs = options.compositionTimeoutMs ?? COMPOSITION_TIMEOUT_MS;
  const previous = queues.get(field) ?? Promise.resolve();
  const run = previous.then(async (): Promise<InsertResult> => {
    if (text === "") return { ok: false, reason: "empty-text" };
    let waited = false;
    if (isComposing(field)) {
      waited = true;
      if (!(await waitForCompositionEnd(field, timeoutMs))) return { ok: false, reason: "composition-timeout" };
    }
    if (!field.isConnected) return { ok: false, reason: "disconnected" };
    // Re-checked at insertion time: the page may have turned the field into a password field
    // (or made it readonly) while we waited.
    if (resolveTarget(field) !== field) return { ok: false, reason: "not-a-target" };
    const path = isTextControl(field) ? insertIntoTextControl(field, text) : insertIntoEditable(field as HTMLElement, text);
    return { ok: true, path, waitedForComposition: waited };
  });
  queues.set(
    field,
    run.catch(() => undefined),
  );
  return run;
}

function insertIntoTextControl(el: TextControl, text: string): InsertPath {
  const value = readNativeValue(el);
  // input type=email has no selection API (selectionStart is null): append at the end.
  const start = el.selectionStart ?? value.length;
  const end = el.selectionEnd ?? start;
  writeNativeValue(el, value.slice(0, start) + text + value.slice(end), "insertText", text);
  const caret = start + text.length;
  try {
    el.setSelectionRange(caret, caret);
  } catch {
    // types without a selection API
  }
  return "native-setter";
}

function selectionFor(host: HTMLElement): Selection | null {
  const root = host.getRootNode() as Document | (ShadowRoot & { getSelection?: () => Selection | null });
  // Chromium exposes a shadow root's own selection; elsewhere the document selection is used.
  if ("getSelection" in root && typeof root.getSelection === "function" && root !== host.ownerDocument) {
    const own = root.getSelection();
    if (own !== null) return own;
  }
  return host.ownerDocument.getSelection();
}

function selectionInside(host: HTMLElement, sel: Selection | null): boolean {
  if (sel === null || sel.rangeCount === 0) return false;
  const range = sel.getRangeAt(0);
  return host.contains(range.startContainer) && host.contains(range.endContainer);
}

function caretAtEnd(host: HTMLElement, sel: Selection): void {
  const range = host.ownerDocument.createRange();
  range.selectNodeContents(host);
  range.collapse(false);
  sel.removeAllRanges();
  sel.addRange(range);
}

/** Focus the editing host (if needed) and make sure the selection is inside it. */
export function prepareEditable(host: HTMLElement): Selection | null {
  const doc = host.ownerDocument;
  let active: Element | null = doc.activeElement;
  while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
  if (active !== host) host.focus({ preventScroll: true });
  const sel = selectionFor(host);
  if (sel !== null && !selectionInside(host, sel)) caretAtEnd(host, sel);
  return sel;
}

/** execCommand, or false when the document has none or it throws. */
export function tryExecCommand(doc: Document, command: string, value?: string): boolean {
  const exec = (doc as { execCommand?: Document["execCommand"] }).execCommand;
  if (typeof exec !== "function") return false;
  try {
    return exec.call(doc, command, false, value) === true;
  } catch {
    return false;
  }
}

function insertIntoEditable(host: HTMLElement, text: string): InsertPath {
  const sel = prepareEditable(host);
  if (tryExecCommand(host.ownerDocument, "insertText", text)) return "execCommand";
  insertWithRange(host, sel, text);
  return "range";
}

function insertWithRange(host: HTMLElement, sel: Selection | null, text: string): void {
  const doc = host.ownerDocument;
  let range: Range;
  if (sel !== null && selectionInside(host, sel)) {
    range = sel.getRangeAt(0);
  } else {
    range = doc.createRange();
    range.selectNodeContents(host);
    range.collapse(false);
  }
  range.deleteContents();
  const node = doc.createTextNode(text);
  range.insertNode(node);
  const after = doc.createRange();
  after.setStartAfter(node);
  after.collapse(true);
  if (sel !== null) {
    sel.removeAllRanges();
    sel.addRange(after);
  }
  dispatchInput(host, "insertText", text);
}
