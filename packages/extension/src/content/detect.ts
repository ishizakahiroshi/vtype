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

function safeGetAttribute(el: Element, name: string): string | null {
  try {
    return Element.prototype.getAttribute.call(el, name);
  } catch {
    return null;
  }
}

function hasPasswordAutocomplete(el: Element): boolean {
  const value = safeGetAttribute(el, "autocomplete");
  if (value === null) return false;
  return value
    .toLowerCase()
    .split(/\s+/)
    .some((token) => PASSWORD_AUTOCOMPLETE_TOKENS.has(token));
}

function isTextSecurityObscured(el: Element): boolean {
  try {
    const rawStyle = safeGetAttribute(el, "style");
    if (rawStyle !== null && /-webkit-text-security\s*:\s*(disc|circle|square)/i.test(rawStyle)) {
      return true;
    }
    const inline = (el as HTMLElement).style?.getPropertyValue?.("-webkit-text-security");
    if (inline === "disc" || inline === "circle" || inline === "square") return true;

    const win = el.ownerDocument.defaultView;
    if (!win || typeof win.getComputedStyle !== "function") return false;
    const style = win.getComputedStyle(el);
    const sec =
      style.getPropertyValue?.("-webkit-text-security") ||
      (style as unknown as { webkitTextSecurity?: string }).webkitTextSecurity;
    return sec === "disc" || sec === "circle" || sec === "square";
  } catch {
    return false;
  }
}

function isTextInput(el: HTMLInputElement): boolean {
  // `.type` is the normalized IDL value: a missing or unknown type attribute reads as "text".
  if (!TARGET_INPUT_TYPES.has(el.type.toLowerCase())) return false;
  if (el.readOnly || el.disabled) return false;
  if (hasPasswordAutocomplete(el)) return false;
  if (isTextSecurityObscured(el)) return false;
  return true;
}

function isWritableTextArea(el: HTMLTextAreaElement): boolean {
  if (el.readOnly || el.disabled) return false;
  if (hasPasswordAutocomplete(el)) return false;
  if (isTextSecurityObscured(el)) return false;
  return true;
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
    const value = safeGetAttribute(node, "contenteditable");
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
  const host = editingHost(el);
  if (host === null) return null;
  if (hasPasswordAutocomplete(host) || hasPasswordAutocomplete(el)) return null;
  if (isTextSecurityObscured(host) || isTextSecurityObscured(el)) return null;
  return host;
}

export function isTargetField(el: Element | null): el is TargetField {
  return el !== null && resolveTarget(el) === el;
}

// ---- finding the fields on the page (C7g) -------------------------------------------------

/**
 * Elements worth asking `resolveTarget` about. Kept as one selector so the scan is a single
 * query: it runs on a timer and on DOM changes, never per frame.
 */
export const TARGET_SELECTOR = "input, textarea, [contenteditable]";

/**
 * A mic is only shown on a field big enough to have one sitting next to it. Below this a field
 * is a filter box in a toolbar, a one-character cell in a grid or a hidden 1x1 input, and an
 * icon beside it would be noise. Chosen as: narrower than about eight characters, or shorter
 * than a single line of text.
 */
export const MIN_FIELD_WIDTH_PX = 60;
export const MIN_FIELD_HEIGHT_PX = 18;

/** Whether the field is big enough and at least partly inside the window right now. */
export function isFieldOnScreen(field: Element, doc: Document): boolean {
  const r = field.getBoundingClientRect();
  if (r.width < MIN_FIELD_WIDTH_PX || r.height < MIN_FIELD_HEIGHT_PX) return false;
  const view = doc.defaultView;
  const vw = doc.documentElement.clientWidth || (view?.innerWidth ?? 0);
  const vh = doc.documentElement.clientHeight || (view?.innerHeight ?? 0);
  if (vw <= 0 || vh <= 0) return true; // no measurable window: do not hide everything
  return r.right > 0 && r.bottom > 0 && r.left < vw && r.top < vh;
}

/**
 * The fields that should carry a mic: targets (so no password field, no read-only field), big
 * enough, on screen, in document order, at most `limit` of them. The limit is what keeps a
 * page with fifty fields from becoming a wall of icons — and what keeps the per-frame position
 * tracking constant, since only the mics that exist are measured.
 *
 * Fields inside an open shadow root are not found here (a query does not cross that boundary);
 * they still get a mic when they are focused or hovered, which is how they got one before C7g.
 */
export function visibleTargetFields(doc: Document, limit: number): TargetField[] {
  const found: TargetField[] = [];
  if (limit <= 0) return found;
  const seen = new Set<Element>();
  for (const el of doc.querySelectorAll(TARGET_SELECTOR)) {
    const field = resolveTarget(el);
    if (field === null || seen.has(field)) continue;
    seen.add(field);
    if (!isFieldOnScreen(field, doc)) continue;
    found.push(field);
    if (found.length >= limit) break;
  }
  return found;
}

/** `document.activeElement`, descending into open shadow roots. */
export function deepActiveElement(doc: Document): Element | null {
  let active: Element | null = doc.activeElement;
  while (active !== null && active.shadowRoot !== null && active.shadowRoot.activeElement !== null) {
    active = active.shadowRoot.activeElement;
  }
  return active;
}
