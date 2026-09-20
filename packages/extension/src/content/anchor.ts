// The mic that appears beside the focused field, and the panel it opens.
//
// Invariant: nothing here writes to the page's field. The mic and panel live inside a shadow
// root on our own host element, appended to <html>, so we never wrap the field, add attributes
// to it or touch its style. The field is only read (getBoundingClientRect, isConnected) and
// observed (MutationObserver on its attributes, which does not mutate it). The panel (C7) also
// reads it (hasText) and listens for its input events on the field's root node.
//
// C5: position tracking, hover open/close with separate delays, tap on no-hover devices.
// C7: the look (ui/styles.css), the thin mic (ui/toolbar.ts) and the panel (ui/panel.ts).
// C7b wires recognition through the `ui` handle (setState / onMic / ...).
// C7e: pressing the thin mic opens the panel and tells the outside (`onMicPress`), which starts
// or stops the recording; `canAutoClose` lets the outside hold the panel open while it runs.
// Hovering only opens the panel, unless the outside reacts to `onOpenChange` (the `hover`
// setting does; the default `click` setting does not).

import css from "../ui/styles.css?raw";
import { createPanel, type Panel } from "../ui/panel";
import { createTrigger } from "../ui/toolbar";

/** How long the pointer must rest on the mic before the panel opens (a pass-over must not open it). */
export const OPEN_DELAY_MS = 350;
/** How long after the pointer leaves the mic/panel before the panel closes (lets it cross gaps). */
export const CLOSE_DELAY_MS = 500;
/** Side of the thin mic (must match `.mic` in ui/styles.css). */
export const MIC_SIZE_PX = 24;
/** Width of the panel (must match `.panel` in ui/styles.css); decides when it grows leftwards. */
export const PANEL_WIDTH_PX = 240;
/** Rough panel height used to decide whether it opens upwards (measured when laid out). */
const PANEL_HEIGHT_ESTIMATE_PX = 90;
/** Gap between the field's right edge and the mic. */
export const MIC_GAP_PX = 6;
/** Height of the band at the top of the field the mic is centered in (first line of a textarea). */
const FIRST_LINE_BAND_PX = 36;
const VIEWPORT_MARGIN_PX = 2;

export const HOST_TAG = "vtype-root";

export interface AnchorOptions {
  doc?: Document;
  /** Whether the primary pointer can hover. Default: matchMedia("(hover: hover)"). */
  hoverCapable?: () => boolean;
  /** Re-checked when a target-defining attribute of the field changes; false detaches. */
  isStillTarget?: (field: Element) => boolean;
  /** Attributes whose change triggers `isStillTarget`. */
  observedAttributes?: readonly string[];
  /** The panel opened or closed. The `hover` setting starts a recording from this. */
  onOpenChange?: (open: boolean) => void;
  /**
   * The thin mic beside the field was pressed (a click, or a tap on a touch device). C7e: this
   * is what starts and stops a recording. It runs *before* the panel is opened, so that with
   * the `hover` setting the opening finds this recording already running (see onMicPointerUp).
   */
  onMicPress?: () => void;
  /**
   * Asked before every automatic close (the pointer left). False keeps the panel open and the
   * question is asked again after another CLOSE_DELAY_MS, so the panel closes on its own once
   * the answer turns true. C7e uses it to keep the panel while a recording runs: closing it
   * would take away the waveform and the button that stops the recording.
   * Explicit closes (a tap on the mic, the field going away, detach) do not ask.
   */
  canAutoClose?: () => boolean;
}

export interface Anchor {
  /** Our own host element on <html>. Null until the first attach. */
  readonly host: HTMLElement | null;
  /** The shadow root holding the mic and panel (closed to the page; handed to C7 through here). */
  readonly root: ShadowRoot | null;
  readonly mic: HTMLElement | null;
  /** The panel element (same as `ui.element`). */
  readonly panel: HTMLElement | null;
  /** The panel's state / hook API (C7). Null until the first attach. */
  readonly ui: Panel | null;
  /** The field the mic is attached to, or null when hidden. */
  readonly target: Element | null;
  readonly isOpen: boolean;
  /** Whether the mic is currently shown (attached and the field is visible). */
  readonly isVisible: boolean;
  attach(field: Element): void;
  detach(): void;
  /**
   * Open the panel immediately. No-op when detached. v1 has no caller (the hotkey was dropped);
   * kept as the entry point for a future keyboard path.
   */
  openPanel(): void;
  closePanel(): void;
  /** Recompute the position now instead of waiting for the next frame. */
  update(): void;
  /** Detach and remove the host from the page. */
  destroy(): void;
}

interface Rect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

function defaultHoverCapable(doc: Document): () => boolean {
  return () => {
    const view = doc.defaultView;
    if (view === null || typeof view.matchMedia !== "function") return true;
    return view.matchMedia("(hover: hover)").matches;
  };
}

function intersect(a: Rect, b: Rect): Rect {
  return {
    left: Math.max(a.left, b.left),
    top: Math.max(a.top, b.top),
    right: Math.min(a.right, b.right),
    bottom: Math.min(a.bottom, b.bottom),
  };
}

function isEmpty(r: Rect): boolean {
  return r.right <= r.left || r.bottom <= r.top;
}

/** Computed `overflow-x/y` values that clip descendants (everything except `visible`). */
const CLIPPING_OVERFLOW: ReadonlySet<string> = new Set(["auto", "scroll", "hidden", "clip", "overlay"]);

function parentAcrossShadow(el: Element): Element | null {
  if (el.parentElement !== null) return el.parentElement;
  const root = el.getRootNode();
  return root instanceof ShadowRoot ? root.host : null;
}

/**
 * The part of `field` that is actually on screen: its box clipped by every ancestor that clips
 * overflow (scroll containers) and by the viewport. Empty when scrolled out of any of them.
 */
function visibleRect(field: Element, doc: Document): Rect {
  const view = doc.defaultView;
  const b = field.getBoundingClientRect();
  let visible: Rect = { left: b.left, top: b.top, right: b.right, bottom: b.bottom };
  if (b.width <= 0 || b.height <= 0) return { left: 0, top: 0, right: 0, bottom: 0 };
  if (view !== null) {
    for (let a = parentAcrossShadow(field); a !== null && a !== doc.documentElement && a !== doc.body; a = parentAcrossShadow(a)) {
      const style = view.getComputedStyle(a);
      if (CLIPPING_OVERFLOW.has(style.overflowX) || CLIPPING_OVERFLOW.has(style.overflowY)) {
        const r = a.getBoundingClientRect();
        visible = intersect(visible, { left: r.left, top: r.top, right: r.right, bottom: r.bottom });
        if (isEmpty(visible)) return visible;
      }
    }
    const vw = doc.documentElement.clientWidth || view.innerWidth;
    const vh = doc.documentElement.clientHeight || view.innerHeight;
    visible = intersect(visible, { left: 0, top: 0, right: vw, bottom: vh });
  }
  return visible;
}

export function createAnchor(options: AnchorOptions = {}): Anchor {
  const doc = options.doc ?? document;
  const hoverCapable = options.hoverCapable ?? defaultHoverCapable(doc);
  const onOpenChange = options.onOpenChange;
  const onMicPress = options.onMicPress;
  const canAutoClose = options.canAutoClose;

  let host: HTMLElement | null = null;
  let root: ShadowRoot | null = null;
  let box: HTMLElement | null = null;
  let mic: HTMLElement | null = null;
  let panel: HTMLElement | null = null;
  let ui: Panel | null = null;

  let target: Element | null = null;
  let open = false;
  let visible = false;
  let openTimer: ReturnType<typeof setTimeout> | null = null;
  let closeTimer: ReturnType<typeof setTimeout> | null = null;
  let frame: number | null = null;
  let dirty = true;
  let lastBox: DOMRect | null = null;
  let attributeObserver: MutationObserver | null = null;

  const view = (): (Window & typeof globalThis) | null => doc.defaultView as (Window & typeof globalThis) | null;

  function clearOpenTimer(): void {
    if (openTimer !== null) {
      clearTimeout(openTimer);
      openTimer = null;
    }
  }

  function clearCloseTimer(): void {
    if (closeTimer !== null) {
      clearTimeout(closeTimer);
      closeTimer = null;
    }
  }

  function setOpen(next: boolean): void {
    clearOpenTimer();
    clearCloseTimer();
    if (open === next) return;
    open = next;
    if (panel !== null) panel.hidden = !next;
    box?.classList.toggle("open", next);
    mic?.setAttribute("aria-expanded", next ? "true" : "false");
    // The page may have changed the field's text without an input event (e.g. after sending).
    if (next) {
      ui?.refreshHasText();
      dirty = true; // re-place on the next frame with the panel's real height (flip-y)
    }
    onOpenChange?.(next);
  }

  function isHoverPointer(e: PointerEvent): boolean {
    if (e.pointerType === "touch") return false;
    return hoverCapable();
  }

  function onBoxEnter(e: PointerEvent): void {
    if (!isHoverPointer(e) || target === null) return;
    clearCloseTimer();
    if (!open && openTimer === null) {
      openTimer = setTimeout(() => {
        openTimer = null;
        if (target !== null) setOpen(true);
      }, OPEN_DELAY_MS);
    }
  }

  function scheduleClose(): void {
    if (closeTimer !== null) return;
    closeTimer = setTimeout(() => {
      closeTimer = null;
      if (!open) return;
      // Not allowed to close yet (a recording is running): keep the panel and ask again.
      if (canAutoClose !== undefined && !canAutoClose()) {
        scheduleClose();
        return;
      }
      setOpen(false);
    }, CLOSE_DELAY_MS);
  }

  function onBoxLeave(e: PointerEvent): void {
    if (!isHoverPointer(e)) return;
    clearOpenTimer();
    if (open) scheduleClose();
  }

  function onMicActivate(): void {
    // C7e: a press on the thin mic means the same on every device (mouse click, touch tap,
    // tap on a device that cannot hover): start or stop the recording, and show the panel.
    // It never closes the panel — while a recording runs the panel holds the only button that
    // stops it, and once idle the pointer leaving closes it as before. Where there is no hover
    // (touch), the panel goes when the field loses focus.
    //
    // The press is handed on *before* the panel opens on purpose: with the `hover` setting the
    // opening starts a recording of its own, and it must find this one already running instead
    // of starting a second one that this press would then immediately stop.
    if (target === null) return;
    onMicPress?.();
    setOpen(true);
  }

  function keepFieldFocus(e: Event): void {
    // A press on the mic or panel must not move focus away from the field (that would hide us
    // and lose the caret). Cancelling mousedown keeps focus where it is; for touch, the
    // compatibility mousedown fired after the tap is cancelled the same way.
    e.preventDefault();
  }

  function ensureHost(): void {
    if (host === null) {
      host = doc.createElement(HOST_TAG);
      // Inline, !important: page stylesheets must not be able to size, hide or shift the host.
      for (const [prop, value] of [
        ["all", "initial"],
        ["position", "fixed"],
        ["top", "0"],
        ["left", "0"],
        ["width", "0"],
        ["height", "0"],
        ["overflow", "visible"],
        ["z-index", "2147483647"],
        ["display", "block"],
      ] as const) {
        host.style.setProperty(prop, value, "important");
      }
      root = host.attachShadow({ mode: "closed" });
      const style = doc.createElement("style");
      style.textContent = css;
      box = doc.createElement("div");
      box.className = "box";
      box.hidden = true;
      mic = createTrigger(doc);
      ui = createPanel({ doc, trigger: mic });
      panel = ui.element;
      box.append(mic, panel);
      root.append(style, box);
      box.addEventListener("pointerenter", onBoxEnter);
      box.addEventListener("pointerleave", onBoxLeave);
      box.addEventListener("mousedown", keepFieldFocus);
      mic.addEventListener("pointerup", onMicActivate);
    }
    if (!host.isConnected) doc.documentElement.append(host);
  }

  function hide(): void {
    visible = false;
    if (box !== null) box.hidden = true;
    setOpen(false);
  }

  function place(): void {
    if (target === null || box === null) return;
    if (!target.isConnected) {
      detach();
      return;
    }
    const b = target.getBoundingClientRect();
    const vis = visibleRect(target, doc);
    if (isEmpty(vis)) {
      hide();
      return;
    }
    const w = view();
    const vw = doc.documentElement.clientWidth || (w?.innerWidth ?? 0);
    const vh = doc.documentElement.clientHeight || (w?.innerHeight ?? 0);
    // Outside the right edge; if that would leave the viewport, tuck it inside the field's
    // right edge instead so it stays reachable.
    let x = b.right + MIC_GAP_PX;
    if (vw > 0 && x + MIC_SIZE_PX > vw - VIEWPORT_MARGIN_PX) x = b.right - MIC_SIZE_PX - VIEWPORT_MARGIN_PX;
    // Centered on the first line (the whole box for a single-line input).
    let y = b.top + (Math.min(b.height, FIRST_LINE_BAND_PX) - MIC_SIZE_PX) / 2;
    y = Math.max(y, vis.top);
    y = Math.min(y, vis.bottom - MIC_SIZE_PX);
    if (vh > 0) y = Math.min(Math.max(y, 0), vh - MIC_SIZE_PX);
    box.style.left = `${Math.round(x)}px`;
    box.style.top = `${Math.round(y)}px`;
    // The panel opens below-right of the mic; flip it when that would leave the viewport.
    const panelHeight = panel !== null && !panel.hidden ? panel.getBoundingClientRect().height : 0;
    box.classList.toggle("flip-x", vw > 0 && x + PANEL_WIDTH_PX > vw - VIEWPORT_MARGIN_PX);
    box.classList.toggle(
      "flip-y",
      vh > 0 && y + MIC_SIZE_PX + (panelHeight || PANEL_HEIGHT_ESTIMATE_PX) > vh - VIEWPORT_MARGIN_PX,
    );
    box.hidden = false;
    visible = true;
  }

  function sameBox(a: DOMRect | null, b: DOMRect): boolean {
    return a !== null && a.left === b.left && a.top === b.top && a.width === b.width && a.height === b.height;
  }

  function tick(): void {
    frame = null;
    if (target === null) return;
    // Polling the rect every frame while attached catches the field moving for reasons no
    // event reports (content inserted above it, a sidebar opening, CSS transitions).
    const b = target.getBoundingClientRect();
    if (dirty || !sameBox(lastBox, b) || !target.isConnected) {
      dirty = false;
      lastBox = b;
      place();
    }
    if (target !== null) scheduleFrame();
  }

  function scheduleFrame(): void {
    if (frame === null) frame = requestAnimationFrame(tick);
  }

  function markDirty(): void {
    dirty = true;
  }

  function addTrackingListeners(): void {
    const w = view();
    // Capture phase on the document sees scroll events from every scroll container, not only
    // the page itself (scroll does not bubble).
    doc.addEventListener("scroll", markDirty, { capture: true, passive: true });
    w?.addEventListener("resize", markDirty, { passive: true });
  }

  function removeTrackingListeners(): void {
    const w = view();
    doc.removeEventListener("scroll", markDirty, { capture: true });
    w?.removeEventListener("resize", markDirty);
  }

  function attach(field: Element): void {
    if (target === field) {
      update();
      return;
    }
    if (target !== null) detach();
    ensureHost();
    target = field;
    ui?.bindField(field);
    dirty = true;
    lastBox = null;
    addTrackingListeners();
    const isStillTarget = options.isStillTarget;
    if (isStillTarget !== undefined) {
      attributeObserver = new MutationObserver(() => {
        if (target !== null && !isStillTarget(target)) detach();
      });
      const filter = options.observedAttributes;
      attributeObserver.observe(
        field,
        filter === undefined ? { attributes: true } : { attributes: true, attributeFilter: [...filter] },
      );
    }
    update();
    scheduleFrame();
  }

  function detach(): void {
    if (target === null) return;
    target = null;
    removeTrackingListeners();
    attributeObserver?.disconnect();
    attributeObserver = null;
    if (frame !== null) cancelAnimationFrame(frame);
    frame = null;
    lastBox = null;
    hide();
    ui?.bindField(null);
  }

  function update(): void {
    if (target === null) return;
    dirty = false;
    lastBox = target.getBoundingClientRect();
    place();
  }

  return {
    get host() {
      return host;
    },
    get root() {
      return root;
    },
    get mic() {
      return mic;
    },
    get panel() {
      return panel;
    },
    get ui() {
      return ui;
    },
    get target() {
      return target;
    },
    get isOpen() {
      return open;
    },
    get isVisible() {
      return visible;
    },
    attach,
    detach,
    openPanel(): void {
      if (target !== null && visible) setOpen(true);
    },
    closePanel(): void {
      setOpen(false);
    },
    update,
    destroy(): void {
      detach();
      ui?.destroy();
      host?.remove();
    },
  };
}
