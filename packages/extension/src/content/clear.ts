// Empty a target field for the panel's × button (plan C6).
//
// Same write paths as insert.ts: input / textarea through the native value setter plus an
// `input` event; contenteditable through execCommand("delete") on a selection covering the
// whole host, falling back to Range.deleteContents plus an `input` event.
//
// "Only when there is text" lives here (hasText); showing or hiding the × is C7's job.

import { resolveTarget } from "./detect";
import { dispatchInput, prepareEditable, readNativeValue, tryExecCommand, writeNativeValue } from "./insert";

/** Characters editors leave in an "empty" contenteditable (caret anchors), not user text. */
const INVISIBLE = /[​﻿]/g;

function isTextControl(el: Element): el is HTMLInputElement | HTMLTextAreaElement {
  return el.localName === "input" || el.localName === "textarea";
}

/** Whether the field holds any text, i.e. whether × should be usable. */
export function hasText(field: Element): boolean {
  if (isTextControl(field)) return readNativeValue(field).length > 0;
  return (field.textContent ?? "").replace(INVISIBLE, "").length > 0;
}

/**
 * Empty `field`. Returns false, without firing any event, when the field is already empty or
 * is not a vtype target (password / readonly / disabled / not editable). Returns true after
 * clearing and firing `input`.
 */
export function clearField(field: Element): boolean {
  if (resolveTarget(field) !== field) return false;
  if (!hasText(field)) return false;
  if (isTextControl(field)) {
    writeNativeValue(field, "", "deleteContent", null);
    return true;
  }
  const host = field as HTMLElement;
  const sel = prepareEditable(host);
  const all = host.ownerDocument.createRange();
  all.selectNodeContents(host);
  if (sel !== null) {
    sel.removeAllRanges();
    sel.addRange(all);
  }
  if (tryExecCommand(host.ownerDocument, "delete")) return true;
  all.deleteContents();
  if (sel !== null) {
    const caret = host.ownerDocument.createRange();
    caret.selectNodeContents(host);
    caret.collapse(true);
    sel.removeAllRanges();
    sel.addRange(caret);
  }
  dispatchInput(host, "deleteContent", null);
  return true;
}
