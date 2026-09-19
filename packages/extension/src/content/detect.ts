// Which elements on a page are voice-input targets.
//
// Everything here is read-only: it inspects tag names, properties and attributes and never
// writes to the page. The anchor (anchor.ts) lives in its own shadow host and only reads the
// field's geometry.

/**
 * The only `<input>` types that are targets. This is an allowlist on purpose: `password`
 * (and `hidden`, `number`, `date`, ...) is excluded because it is not listed, not because a
 * denylist remembered it. A new input type added to HTML later stays excluded by default.
 */
export const TARGET_INPUT_TYPES: ReadonlySet<string> = new Set(["text", "search", "email", "url", "tel"]);

/**
 * `autocomplete` tokens that mark a field as a password even when its `type` is not
 * `password`. Sites with a "show password" toggle switch the field to `type="text"` while it
 * is revealed; the autocomplete token stays and keeps the field excluded.
 */
const PASSWORD_AUTOCOMPLETE_TOKENS: ReadonlySet<string> = new Set(["current-password", "new-password"]);

/** Attributes whose change can turn a target into a non-target (or back). */
export const TARGET_DEFINING_ATTRIBUTES: readonly string[] = [
  "type",
  "readonly",
  "disabled",
  "contenteditable",
  "autocomplete",
];

export type TargetField = HTMLInputElement | HTMLTextAreaElement | HTMLElement;

function isInput(el: Element): el is HTMLInputElement {
  return el.localName === "input" && el.namespaceURI === "http://www.w3.org/1999/xhtml";
}

function isTextArea(el: Element): el is HTMLTextAreaElement {
  return el.localName === "textarea" && el.namespaceURI === "http://www.w3.org/1999/xhtml";
}

function hasPasswordAutocomplete(el: Element): boolean {
  const value = el.getAttribute("autocomplete");
  if (value === null) return false;
  return value
    .toLowerCase()
    .split(/\s+/)
    .some((token) => PASSWORD_AUTOCOMPLETE_TOKENS.has(token));
}

function isTextInput(el: HTMLInputElement): boolean {
  // `.type` is the normalized IDL value: a missing or unknown type attribute reads as "text".
  if (!TARGET_INPUT_TYPES.has(el.type.toLowerCase())) return false;
  if (el.readOnly || el.disabled) return false;
  if (hasPasswordAutocomplete(el)) return false;
  return true;
}

function isWritableTextArea(el: HTMLTextAreaElement): boolean {
  return !el.readOnly && !el.disabled;
}

/**
 * Whether `el` is editable through the `contenteditable` attribute, following the HTML
 * inheritance rule: the nearest ancestor-or-self with the attribute decides; "" / "true" /
 * "plaintext-only" mean editable, "false" means not, invalid values inherit. Implemented from
 * attributes rather than `isContentEditable` so the result does not depend on how complete the
 * DOM implementation is. The walk stops at a shadow root (editability does not cross it).
 */
export function isContentEditableElement(el: Element): boolean {
  for (let node: Element | null = el; node !== null; node = node.parentElement) {
    const value = node.getAttribute("contenteditable");
    if (value === null) continue;
    const v = value.toLowerCase();
    if (v === "" || v === "true" || v === "plaintext-only") return true;
    if (v === "false") return false;
  }
  return false;
}

/** The outermost contenteditable element containing `el` (the element that holds focus). */
function editingHost(el: Element): HTMLElement | null {
  if (!isContentEditableElement(el)) return null;
  let host: Element = el;
  while (host.parentElement !== null && isContentEditableElement(host.parentElement)) {
    host = host.parentElement;
  }
  return host instanceof HTMLElement ? host : null;
}

/**
 * Map a focused (or event-origin) element to the field the mic should attach to, or null.
 * `input` and `textarea` are decided by their own type/state and never fall through to the
 * contenteditable rule, so an `<input type="password">` inside an editable region is still
 * excluded.
 */
export function resolveTarget(el: Element | null): TargetField | null {
  if (el === null) return null;
  if (isInput(el)) return isTextInput(el) ? el : null;
  if (isTextArea(el)) return isWritableTextArea(el) ? el : null;
  return editingHost(el);
}

export function isTargetField(el: Element | null): el is TargetField {
  return el !== null && resolveTarget(el) === el;
}

/** `document.activeElement`, descending into open shadow roots. */
export function deepActiveElement(doc: Document): Element | null {
  let active: Element | null = doc.activeElement;
  while (active !== null && active.shadowRoot !== null && active.shadowRoot.activeElement !== null) {
    active = active.shadowRoot.activeElement;
  }
  return active;
}
