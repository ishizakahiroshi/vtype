// Content-script entry. Bundled by build.mjs into a single classic script (dist/content.js).
//
// Watches focus, asks detect.ts whether the focused element is a target, and moves the anchor
// (anchor.ts) to it. The panel's mic drives controller.ts, which asks the background to run
// recognition in the offscreen document; nothing here touches the speech or media APIs.
// v1 has no keyboard shortcut (owner's decision, 2026-09-19).

import { createAnchor, type Anchor } from "./anchor";
import { createController, type ContentRuntime, type Controller } from "./controller";
import { TARGET_DEFINING_ATTRIBUTES, deepActiveElement, resolveTarget } from "./detect";
import { startCompositionTracking } from "./insert";

/** The only part of the extension API this file touches, typed narrowly instead of @types/chrome. */
interface ChromeRuntimeView {
  runtime?: { id?: string } & Partial<ContentRuntime>;
}

export interface ContentScript {
  readonly anchor: Anchor;
  readonly controller: Controller;
  /** Remove every listener this script added and take the host off the page. */
  stop(): void;
}

export interface StartOptions {
  doc?: Document;
  hoverCapable?: () => boolean;
  /** chrome.runtime by default; null runs without an extension (tests of detection only). */
  runtime?: ContentRuntime | null;
  language?: string;
}

function extensionRuntime(): ContentRuntime | null {
  const runtime = (globalThis as { chrome?: ChromeRuntimeView }).chrome?.runtime;
  if (runtime?.id === undefined || runtime.sendMessage === undefined || runtime.onMessage === undefined) return null;
  return runtime as ContentRuntime;
}

export function startContentScript(options: StartOptions = {}): ContentScript {
  const doc = options.doc ?? document;
  // Before anything can be inserted: a composition that starts before tracking is invisible,
  // and insertAtCursor would then write into the middle of it (C6).
  startCompositionTracking(doc);
  const anchor = createAnchor({
    doc,
    ...(options.hoverCapable !== undefined ? { hoverCapable: options.hoverCapable } : {}),
    isStillTarget: (field) => resolveTarget(field) === field,
    observedAttributes: TARGET_DEFINING_ATTRIBUTES,
  });
  const controller = createController({
    anchor,
    runtime: options.runtime !== undefined ? options.runtime : extensionRuntime(),
    ...(options.language !== undefined ? { language: options.language } : {}),
  });

  // Focus changes between two elements of the same shadow root are not visible from the
  // document (the event stops at the shadow boundary once target and relatedTarget retarget
  // to the same host), so while attached inside a shadow root we also listen on that root.
  let extraRoot: ShadowRoot | null = null;

  function isOurs(path: EventTarget[]): boolean {
    const host = anchor.host;
    return host !== null && path.includes(host);
  }

  function follow(field: Element | null): void {
    if (field === null) {
      anchor.detach();
      setExtraRoot(null);
      return;
    }
    anchor.attach(field);
    // The panel is created on the first attach; hook its buttons up to the controller.
    if (anchor.ui !== null) controller.wire(anchor.ui);
    const root = field.getRootNode();
    setExtraRoot(root instanceof ShadowRoot ? root : null);
  }

  function onFocusIn(e: Event): void {
    const path = e.composedPath();
    if (isOurs(path)) return;
    const origin = path[0];
    follow(origin instanceof Element ? resolveTarget(origin) : null);
  }

  function onFocusOut(): void {
    // Decide after focus has settled: the window losing focus keeps activeElement on the
    // field (keep the mic), clicking empty page space moves it to <body> (hide it).
    setTimeout(() => {
      const current = anchor.target;
      if (current === null) return;
      const active = deepActiveElement(doc);
      if (active !== null && anchor.host !== null && active === anchor.host) return;
      if (active !== null && resolveTarget(active) === current) return;
      follow(null);
    }, 0);
  }

  function setExtraRoot(root: ShadowRoot | null): void {
    if (extraRoot === root) return;
    if (extraRoot !== null) {
      extraRoot.removeEventListener("focusin", onFocusIn, true);
      extraRoot.removeEventListener("focusout", onFocusOut, true);
    }
    extraRoot = root;
    if (root !== null) {
      root.addEventListener("focusin", onFocusIn, true);
      root.addEventListener("focusout", onFocusOut, true);
    }
  }

  doc.addEventListener("focusin", onFocusIn, true);
  doc.addEventListener("focusout", onFocusOut, true);

  // The script may be injected after a field already has focus (document_idle).
  follow(resolveTarget(deepActiveElement(doc)));

  return {
    anchor,
    controller,
    stop(): void {
      doc.removeEventListener("focusin", onFocusIn, true);
      doc.removeEventListener("focusout", onFocusOut, true);
      setExtraRoot(null);
      controller.dispose();
      anchor.destroy();
    },
  };
}

// Auto-start only inside an extension context; tests import this module and call
// startContentScript() themselves.
if (extensionRuntime() !== null) {
  startContentScript();
}
