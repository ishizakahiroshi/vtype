// Content-script entry. Bundled by build.mjs into a single classic script (dist/content.js).
//
// Watches focus, asks detect.ts whether the focused element is a target, and moves the anchor
// (anchor.ts) to it. The panel's mic drives controller.ts, which asks the background to run
// recognition in the offscreen document; nothing here touches the speech or media APIs.
// v1 has no keyboard shortcut (owner's decision, 2026-09-19).
// C7e: pressing the thin mic starts and stops the recording (the default `click` setting); with
// the `hover` setting the panel opening starts it by itself. Either way the panel is held open
// until the recording ends. The setting comes from chrome.storage.sync and can change while the
// page is open (shared/settings.ts).

import { createAnchor, type Anchor } from "./anchor";
import { createController, type ContentRuntime, type Controller } from "./controller";
import { TARGET_DEFINING_ATTRIBUTES, deepActiveElement, resolveTarget } from "./detect";
import { startCompositionTracking } from "./insert";
import {
  DEFAULT_TRIGGER,
  NO_OFFSET,
  extensionStorage,
  readOffsets,
  readTrigger,
  watchOffsets,
  watchTrigger,
  withOffset,
  writeOffsets,
  type MicOffsets,
  type StorageView,
  type TriggerMode,
} from "../shared/settings";

/** The only part of the extension API this file touches, typed narrowly instead of @types/chrome. */
interface ChromeRuntimeView {
  runtime?: { id?: string } & Partial<ContentRuntime>;
}

export interface ContentScript {
  readonly anchor: Anchor;
  readonly controller: Controller;
  /** What starts a recording right now (C7e). Follows the setting while the page is open. */
  readonly trigger: TriggerMode;
  /** Remove every listener this script added and take the host off the page. */
  stop(): void;
}

export interface StartOptions {
  doc?: Document;
  hoverCapable?: () => boolean;
  /** chrome.runtime by default; null runs without an extension (tests of detection only). */
  runtime?: ContentRuntime | null;
  language?: string;
  /** chrome.storage by default; null runs on the default setting and never reads storage. */
  storage?: StorageView | null;
  /** The site the dragged mic position is remembered for. Default: this page's origin. */
  origin?: string;
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
  // The anchor is built before the controller (the controller needs it), so the two callbacks
  // below reach the controller through this box, which is filled in right after.
  let recorder: Controller | null = null;
  // Until storage answers (and whenever it cannot), the default setting applies: a hover that
  // starts recording is a surprise, so it is never what an unanswered read falls back to.
  let trigger: TriggerMode = DEFAULT_TRIGGER;
  const storage = options.storage !== undefined ? options.storage : extensionStorage();
  // C7f: dragged mic positions are per site, because the button they collide with is.
  const origin = options.origin ?? doc.defaultView?.location.origin ?? "";
  let offsets: MicOffsets = {};
  const anchor = createAnchor({
    doc,
    ...(options.hoverCapable !== undefined ? { hoverCapable: options.hoverCapable } : {}),
    isStillTarget: (field) => resolveTarget(field) === field,
    observedAttributes: TARGET_DEFINING_ATTRIBUTES,
    // C7e, `click` (default): pressing the thin mic opens the panel and starts or stops the
    // recording. The panel's own mic keeps doing the same through controller.wire.
    onMicPress: () => recorder?.toggle(),
    // C7e, `hover`: opening the panel is itself the start. autoStart stays silent when it
    // cannot start, because nobody pressed anything.
    onOpenChange: (open) => {
      if (open && trigger === "hover") recorder?.autoStart();
    },
    // While a recording runs the panel must stay: it holds the button that stops it.
    canAutoClose: () => recorder === null || recorder.phase === "idle",
    // C7f: the user dragged the mic aside. Remember it for this site, not for the page.
    onOffsetChange: (offset) => {
      offsets = withOffset(offsets, origin, offset);
      void writeOffsets(storage, offsets);
    },
  });
  const controller = createController({
    anchor,
    runtime: options.runtime !== undefined ? options.runtime : extensionRuntime(),
    ...(options.language !== undefined ? { language: options.language } : {}),
  });
  recorder = controller;

  // The settings are read once and then followed: a change in the options page (or on another
  // device) reaches every open page without a reload.
  void readTrigger(storage).then((mode) => {
    trigger = mode;
  });
  const unwatchTrigger = watchTrigger(storage, (mode) => {
    trigger = mode;
  });

  function applyOffset(): void {
    anchor.setOffset(offsets[origin] ?? NO_OFFSET);
  }

  void readOffsets(storage).then((all) => {
    offsets = all;
    applyOffset();
  });
  // Reaches here when the options page resets the positions, too: the mic goes back at once.
  const unwatchOffsets = watchOffsets(storage, (all) => {
    offsets = all;
    applyOffset();
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
    const focused = path[0];
    follow(focused instanceof Element ? resolveTarget(focused) : null);
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
    get trigger() {
      return trigger;
    },
    stop(): void {
      doc.removeEventListener("focusin", onFocusIn, true);
      doc.removeEventListener("focusout", onFocusOut, true);
      setExtraRoot(null);
      unwatchTrigger();
      unwatchOffsets();
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
