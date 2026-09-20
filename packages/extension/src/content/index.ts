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
// C7g: fields carry a mic without being clicked. Which ones is the `micDisplay` setting: `all`
// (the default: every target field visible on screen, up to MAX_MICS) or `hover` (the field
// under the pointer and the field with the caret). Finding them is a document query, so it runs
// on DOM changes and on a timer after scrolling — never inside the per-frame position tracking.
// C9: a site the user has switched vtype off on gets no content script at all. startWhenAllowed
// is the entry point for that: it reads the excluded list first and only then starts anything,
// so on an excluded site not one element is ever put on the page.

import { createAnchor, type Anchor } from "./anchor";
import { createController, type ContentRuntime, type Controller } from "./controller";
import {
  TARGET_DEFINING_ATTRIBUTES,
  deepActiveElement,
  isFieldOnScreen,
  resolveTarget,
  visibleTargetFields,
} from "./detect";
import { startCompositionTracking } from "./insert";
import { TOGGLE_SITE_ACK, isToggleSite } from "../shared/messages";
import {
  DEFAULT_MIC_DISPLAY,
  DEFAULT_TRIGGER,
  NO_OFFSET,
  extensionStorage,
  isExcluded,
  readExcludedSites,
  readMicDisplay,
  readOffsets,
  readTrigger,
  watchExcludedSites,
  watchMicDisplay,
  watchOffsets,
  watchTrigger,
  withExcluded,
  withOffset,
  withoutExcluded,
  writeExcludedSites,
  writeOffsets,
  type ExcludedSites,
  type MicDisplay,
  type MicOffsets,
  type StorageView,
  type TriggerMode,
} from "../shared/settings";

/**
 * How many mics may be on screen at once. Two jobs: a page with dozens of fields must not turn
 * into a wall of icons, and the per-frame position tracking measures one rect per shown mic, so
 * this is what keeps that work constant however many fields the page has.
 */
export const MAX_MICS = 12;
/**
 * How long a burst of DOM changes or scrolling is collected before the fields are looked up
 * again. Scrolling fires continuously; without this the query would run on every event.
 */
export const RESCAN_DELAY_MS = 200;
/**
 * `hover`: how long a field keeps its mic after the pointer has left it, so that moving from
 * the field to its own mic (which sits outside the field) does not take the mic away first.
 */
export const HOVER_LINGER_MS = 600;

/** The only part of the extension API this file touches, typed narrowly instead of @types/chrome. */
interface ChromeRuntimeView {
  runtime?: { id?: string } & Partial<ContentRuntime>;
}

export interface ContentScript {
  readonly anchor: Anchor;
  readonly controller: Controller;
  /** What starts a recording right now (C7e). Follows the setting while the page is open. */
  readonly trigger: TriggerMode;
  /** Which fields carry a mic right now (C7g). Follows the setting while the page is open. */
  readonly micDisplay: MicDisplay;
  /** Look for the fields to put mics on now, instead of waiting for the timer (C7g). */
  rescan(): void;
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
    // C7g: and it must not walk off to another field while that recording runs either.
    canSwitchTarget: () => recorder === null || recorder.phase === "idle",
    // C7g: the anchor already listens for scroll and resize; this is that same news.
    onViewportChange: () => scheduleRescan(),
    // C7h: what the page has at a point, so a mic can step aside from the site's own buttons.
    // happy-dom (and any document without it) simply answers "nothing", which leaves every mic
    // exactly where vtype has always put it.
    elementsAt: (x, y) => {
      const fromPoint = (doc as Document & { elementsFromPoint?: (x: number, y: number) => Element[] })
        .elementsFromPoint;
      if (typeof fromPoint !== "function") return [];
      try {
        return fromPoint.call(doc, x, y);
      } catch {
        return [];
      }
    },
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

  // ---- C7g: which fields carry a mic ------------------------------------------------------

  let micDisplay: MicDisplay = DEFAULT_MIC_DISPLAY;
  /** `hover`: the field the pointer is on, and the one it just left (it keeps its mic a while). */
  let overField: Element | null = null;
  let leavingField: Element | null = null;
  let lingerTimer: ReturnType<typeof setTimeout> | null = null;
  let rescanTimer: ReturnType<typeof setTimeout> | null = null;
  let fieldObserver: MutationObserver | null = null;
  /** C7h: the next rescan follows a change to the page itself, so the spots are decided again. */
  let pageChanged = false;

  function keepAlive(field: Element | null, into: Element[]): void {
    if (field === null || into.includes(field)) return;
    if (!field.isConnected || resolveTarget(field) !== field) return;
    if (!isFieldOnScreen(field, doc)) return;
    into.push(field);
  }

  /**
   * The fields the user is actually dealing with. They come first in both settings: with
   * `hover` they are the whole list, and with `all` they are what makes the cap bearable — the
   * field under the pointer gets a mic even on a page with more fields than mics.
   */
  function priorityFields(): Element[] {
    const keep: Element[] = [];
    keepAlive(overField, keep);
    keepAlive(leavingField, keep);
    keepAlive(anchor.hoveredField, keep); // the pointer is on this field's own mic or panel
    keepAlive(resolveTarget(deepActiveElement(doc)), keep);
    // Never take the mic away from a panel that is open or a recording that is running.
    if (anchor.isOpen || controller.phase !== "idle") keepAlive(anchor.target, keep);
    return keep;
  }

  function fieldsForMics(): Element[] {
    const keep = priorityFields();
    if (micDisplay === "all") {
      // Document order for the rest, so the page reads the same way from the top every time.
      for (const field of visibleTargetFields(doc, MAX_MICS)) {
        if (keep.length >= MAX_MICS) break;
        keepAlive(field, keep);
      }
    }
    return keep.slice(0, MAX_MICS);
  }

  function rescan(): void {
    if (rescanTimer !== null) {
      clearTimeout(rescanTimer);
      rescanTimer = null;
    }
    if (pageChanged) {
      // C7h: the page itself is different, so a button may have appeared (or gone) where a mic
      // sits. Scrolling never gets here: a field and the buttons around it move together.
      pageChanged = false;
      anchor.remeasure();
    }
    anchor.setMics(fieldsForMics());
    wireUi();
  }

  /**
   * Scrolling and DOM changes arrive in bursts; the fields are looked up once per burst. This
   * is the only place the whole document is queried, and it is never on the frame path.
   */
  function scheduleRescan(fromPageChange = false): void {
    if (fromPageChange) pageChanged = true;
    if (rescanTimer !== null) return;
    rescanTimer = setTimeout(() => {
      rescanTimer = null;
      rescan();
    }, RESCAN_DELAY_MS);
  }

  /**
   * Scrolling asks for a rescan and nothing more: handed straight to `scheduleRescan`, the event
   * would arrive as `fromPageChange` and make every scroll remeasure the spots. Named, because
   * removing a listener needs the function that was added.
   */
  function onWindowScroll(): void {
    scheduleRescan();
  }

  function setOverField(field: Element | null): void {
    if (field === overField) return;
    if (overField !== null) {
      // Let the field it left keep its mic for a moment: the mic sits outside the field, so
      // reaching for it means leaving the field first.
      leavingField = overField;
      if (lingerTimer !== null) clearTimeout(lingerTimer);
      lingerTimer = setTimeout(() => {
        lingerTimer = null;
        leavingField = null;
        rescan();
      }, HOVER_LINGER_MS);
    }
    overField = field;
    rescan();
  }

  function onPointerOver(e: Event): void {
    const path = e.composedPath();
    if (isOurs(path)) return; // our own mic or panel; the anchor knows the pointer is there
    const under = path[0];
    setOverField(under instanceof Element ? resolveTarget(under) : null);
  }

  function onPageChanged(records: MutationRecord[]): void {
    const host = anchor.host;
    // Our own host being added to <html> is not a reason to look at the page again.
    if (host !== null && records.every((r) => r.target === host || host.contains(r.target))) return;
    scheduleRescan();
  }

  function applyMicDisplay(next: MicDisplay): void {
    micDisplay = next;
    rescan();
  }

  void readMicDisplay(storage).then(applyMicDisplay);
  const unwatchMicDisplay = watchMicDisplay(storage, applyMicDisplay);

  // Focus changes between two elements of the same shadow root are not visible from the
  // document (the event stops at the shadow boundary once target and relatedTarget retarget
  // to the same host), so while attached inside a shadow root we also listen on that root.
  let extraRoot: ShadowRoot | null = null;

  function isOurs(path: EventTarget[]): boolean {
    const host = anchor.host;
    return host !== null && path.includes(host);
  }

  /** The panel is created with the first mic; hook its buttons up to the controller. */
  function wireUi(): void {
    const ui = anchor.ui;
    if (ui === null) return;
    controller.wire(ui);
    // C9: the panel offers the one-press way off this site, but only where the answer can be
    // remembered. What happens next is not this script's business: it writes the list, and
    // whoever started it (startWhenAllowed) sees the change and takes the mics away.
    ui.showSiteOff(storage !== null && origin !== "");
    ui.onSiteOff = () => {
      void readExcludedSites(storage).then((sites) => writeExcludedSites(storage, withExcluded(sites, origin)));
    };
  }

  function follow(field: Element | null): void {
    if (field === null) {
      anchor.detach();
      setExtraRoot(null);
      // C7g: the field that lost the caret may still deserve a mic (or may not).
      if (micDisplay === "hover") rescan();
      return;
    }
    anchor.attach(field);
    wireUi();
    const root = field.getRootNode();
    setExtraRoot(root instanceof ShadowRoot ? root : null);
    if (micDisplay === "hover") rescan();
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
  // C7g: where the pointer is decides which field has a mic with the `hover` setting, and with
  // `all` it is what gives a mic to a field that did not fit under the cap.
  doc.addEventListener("pointerover", onPointerOver, true);

  // C7g: fields appear and disappear (a dialog opens, a list loads). Watching the page is how
  // the mics follow that without the frame loop ever looking for fields itself.
  fieldObserver = new MutationObserver(onPageChanged);
  fieldObserver.observe(doc.documentElement, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: [...TARGET_DEFINING_ATTRIBUTES],
  });
  // Scrolling and resizing change which fields are on screen. While mics exist the anchor is
  // already listening for both and says so through onViewportChange, so the only listener
  // added here is for the case where there are no mics yet (every field below the fold):
  // capture on the window sees scrolling in any container, the same as capture on the document.
  doc.defaultView?.addEventListener("scroll", onWindowScroll, { capture: true, passive: true });

  // The script may be injected after a field already has focus (document_idle).
  follow(resolveTarget(deepActiveElement(doc)));
  // ... and the visible fields get their mics without anyone touching them (C7g).
  rescan();

  return {
    anchor,
    controller,
    get trigger() {
      return trigger;
    },
    get micDisplay() {
      return micDisplay;
    },
    rescan,
    stop(): void {
      doc.removeEventListener("focusin", onFocusIn, true);
      doc.removeEventListener("focusout", onFocusOut, true);
      doc.removeEventListener("pointerover", onPointerOver, true);
      doc.defaultView?.removeEventListener("scroll", onWindowScroll, { capture: true });
      fieldObserver?.disconnect();
      fieldObserver = null;
      if (rescanTimer !== null) clearTimeout(rescanTimer);
      if (lingerTimer !== null) clearTimeout(lingerTimer);
      rescanTimer = null;
      lingerTimer = null;
      setExtraRoot(null);
      unwatchTrigger();
      unwatchOffsets();
      unwatchMicDisplay();
      controller.dispose();
      anchor.destroy();
    },
  };
}

// ---- C9: the sites vtype stays off on ---------------------------------------------------

export interface SiteGate {
  /** The site this page is on, as the excluded list spells it (`https://example.test`). */
  readonly origin: string;
  /** Whether vtype is switched off here. Unknown (false) until the list has been read. */
  readonly excluded: boolean;
  /** The running content script, or null while the site is excluded. */
  readonly script: ContentScript | null;
  /** Switch vtype off here, or back on. What the toolbar icon of the extension does. */
  toggle(): Promise<boolean>;
  /** Stop the content script (if any) and stop following the list. */
  stop(): void;
}

/**
 * Start the content script unless this site is on the excluded list, and follow that list for
 * as long as the page lives.
 *
 * The list is read *before* anything is started, which is the whole point: an excluded site
 * must not get a mic that is then taken away again. Until the read answers, nothing exists —
 * that is a microtask on a page that has just loaded, and the alternative is a flash of
 * something the user has said they do not want.
 *
 * Switching off later (the panel's own line, the options page, another device) stops the
 * script and takes the host element off the page; switching back on starts a fresh one.
 */
export function startWhenAllowed(options: StartOptions = {}): SiteGate {
  const doc = options.doc ?? document;
  const storage = options.storage !== undefined ? options.storage : extensionStorage();
  const runtime = options.runtime !== undefined ? options.runtime : extensionRuntime();
  const origin = options.origin ?? doc.defaultView?.location.origin ?? "";
  let excluded = false;
  let script: ContentScript | null = null;
  let stopped = false;

  // The first read and the watch race each other: a change that lands while the first read is
  // still in flight would otherwise be undone by that older answer arriving last, and the site
  // the user has just switched off would get its mics back until the next change.
  let followedAChange = false;

  function apply(sites: ExcludedSites, fromChange = false): void {
    if (stopped) return;
    if (fromChange) followedAChange = true;
    else if (followedAChange) return;
    excluded = isExcluded(sites, origin);
    if (excluded) {
      script?.stop();
      script = null;
      return;
    }
    if (script === null) script = startContentScript({ ...options, storage, origin });
  }

  void readExcludedSites(storage).then((sites) => apply(sites));
  const unwatch = watchExcludedSites(storage, (sites) => apply(sites, true));

  /** The toolbar icon: off here if it is on, on again if it is off. */
  async function toggle(): Promise<boolean> {
    if (origin === "") return false;
    const sites = await readExcludedSites(storage);
    const next = isExcluded(sites, origin) ? withoutExcluded(sites, origin) : withExcluded(sites, origin);
    return writeExcludedSites(storage, next);
  }

  // The background cannot see what site a tab is on (vtype asks for no host permissions), so
  // the toolbar icon arrives here as a message and the page answers for itself. This listener
  // is the gate's, not the content script's: on an excluded site there is no content script,
  // and that is exactly when the user needs the way back.
  // The answer matters as much as the switching: Chrome closes the port when a listener says
  // nothing, and the background reads that as "no content script here" and opens the options
  // page. Answering right away (the switching itself carries on in the background) is what
  // tells it apart from a page vtype does not run on.
  const onMessage = (message: unknown, _sender?: unknown, sendResponse?: (response?: unknown) => void): void => {
    if (!isToggleSite(message)) return;
    void toggle();
    sendResponse?.(TOGGLE_SITE_ACK);
  };
  runtime?.onMessage.addListener(onMessage);

  return {
    origin,
    get excluded() {
      return excluded;
    },
    get script() {
      return script;
    },
    toggle,
    stop(): void {
      stopped = true;
      runtime?.onMessage.removeListener(onMessage);
      unwatch();
      script?.stop();
      script = null;
    },
  };
}

// Auto-start only inside an extension context; tests import this module and call
// startContentScript() / startWhenAllowed() themselves.
if (extensionRuntime() !== null) {
  startWhenAllowed();
}
