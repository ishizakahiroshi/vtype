// Submitting the page from the panel's send button (plan C8).
//
// The safe default is the important part: when the destination cannot be identified, nothing
// is sent. Sending into the wrong place on a posting site cannot be taken back, so "probably
// this one" is not good enough.
//
// Two paths, in this order:
// 1. The field belongs to a <form>: form.requestSubmit(). Never form.submit(), which skips the
//    `submit` event and with it the site's own validation and single-page-app handlers.
//    Evidence that it went through: the `submit` event fired (a site that calls
//    preventDefault() and posts by itself still counts; a form blocked by HTML validation does
//    not fire it, so that counts as "not submitted").
// 2. No form: send an Enter key sequence to the field, as a user would press it. Evidence that
//    the page took it: the keydown was preventDefault()ed by a handler. Synthetic key events
//    perform no default action of their own, so nothing happens when nobody listens.
// Anything else: no submit, and the caller tells the user.

import { resolveTarget } from "./detect";

export type SubmitPath = "requestSubmit" | "enter-key" | "none";

export interface SubmitResult {
  /** Whether the page actually took the submit (see the evidence rules above). */
  readonly submitted: boolean;
  readonly path: SubmitPath;
}

function formOf(field: Element): HTMLFormElement | null {
  const owner = (field as { form?: HTMLFormElement | null }).form;
  // input / textarea know their form (including the `form="id"` attribute); other editable
  // elements only through the tree.
  if (owner !== undefined) return owner;
  return field.closest("form");
}

function pressEnter(field: Element): boolean {
  const view = field.ownerDocument.defaultView;
  const Ctor = view?.KeyboardEvent ?? KeyboardEvent;
  let consumed = false;
  for (const type of ["keydown", "keypress", "keyup"] as const) {
    const event = new Ctor(type, {
      key: "Enter",
      code: "Enter",
      bubbles: true,
      cancelable: true,
      composed: true,
    });
    // Legacy handlers still read keyCode / which; KeyboardEventInit cannot set them.
    for (const name of ["keyCode", "which"]) {
      Object.defineProperty(event, name, { get: () => 13, configurable: true });
    }
    const notCancelled = field.dispatchEvent(event);
    if (type === "keydown" && !notCancelled) consumed = true;
  }
  return consumed;
}

/**
 * Submit the page through `field`. Returns whether the page took it and which path was used.
 * Refuses fields vtype does not work on (password / readonly / disabled).
 */
export function submitFrom(field: Element): SubmitResult {
  if (resolveTarget(field) !== field || !field.isConnected) return { submitted: false, path: "none" };

  const form = formOf(field);
  if (form !== null) {
    const view = (form.ownerDocument.defaultView ?? globalThis) as typeof globalThis;
    const formProto = view.HTMLFormElement?.prototype;
    const eventProto = view.EventTarget?.prototype;
    const requestSubmit =
      typeof form.requestSubmit === "function" ? form.requestSubmit : formProto?.requestSubmit;
    const addListener =
      typeof form.addEventListener === "function" ? form.addEventListener : eventProto?.addEventListener;
    const removeListener =
      typeof form.removeEventListener === "function" ? form.removeEventListener : eventProto?.removeEventListener;

    if (typeof requestSubmit === "function") {
      let submitEventSeen = false;
      const onSubmit = (): void => {
        submitEventSeen = true;
      };
      try {
        addListener.call(form, "submit", onSubmit, { capture: true });
        requestSubmit.call(form);
      } catch {
        // e.g. a detached form; treated as "not submitted" below
      } finally {
        try {
          removeListener.call(form, "submit", onSubmit, { capture: true });
        } catch {
          // ignore
        }
      }
      if (submitEventSeen) return { submitted: true, path: "requestSubmit" };
      // The form exists but refused (HTML validation): do not fall through to Enter, which
      // would try to submit the same form behind the validation.
      return { submitted: false, path: "none" };
    }
  }

  return pressEnter(field) ? { submitted: true, path: "enter-key" } : { submitted: false, path: "none" };
}
