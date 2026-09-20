// The mics that appear beside text fields, and the one panel they open.
//
// Invariant: nothing here writes to the page's fields. The mics and the panel live inside a
// shadow root on our own host element, appended to <html>, so we never wrap a field, add
// attributes to it or touch its style. Fields are only read (getBoundingClientRect,
// isConnected) and observed (MutationObserver on their attributes, which does not mutate
// them). The panel (C7) also reads the current field (hasText) and listens for its input
// events on that field's root node.
//
// C5: position tracking, hover open/close with separate delays, tap on no-hover devices.
// C7: the look (ui/styles.css), the thin mic (ui/toolbar.ts) and the panel (ui/panel.ts).
// C7b wires recognition through the `ui` handle (setState / onMic / ...).
// C7e: pressing a thin mic opens the panel and tells the outside (`onMicPress`), which starts
// or stops the recording; `canAutoClose` lets the outside hold the panel open while it runs.
// Hovering only opens the panel, unless the outside reacts to `onOpenChange` (the `hover`
// setting does; the default `click` setting does not).
// C7f: a mic can be dragged aside (some sites have their own button where vtype puts it).
// The offset is kept as a distance from the field, not as a position on the screen, and it is
// the same for every mic on the page.
// C7g: there can be several mics at once (`setMics`), but only one panel. It sits in the box
// of the *current* field (`target`), which is the field that was attached, hovered or pressed
// last. A mic whose field goes away keeps its box: boxes are pooled and reused, because on a
// scrolling page fields come and go all the time.

import css from "../ui/styles.css?raw";
import { createPanel, type Panel } from "../ui/panel";
import { createTrigger } from "../ui/toolbar";
import { NO_OFFSET, type MicOffset } from "../shared/settings";

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
/**
 * How far the pointer must travel with the mic held down before it counts as dragging it
 * (C7f). Under this, letting go is the click that starts or stops the recording (C7e); at or
 * over it, the same release only ends the drag.
 */
export const DRAG_THRESHOLD_PX = 4;
/** Height of the band at the top of the field the mic is centered in (first line of a textarea). */
const FIRST_LINE_BAND_PX = 36;
const VIEWPORT_MARGIN_PX = 2;

/**
 * C7h: where a mic may sit, best first. The first one is where vtype has always put it, so a
 * field with nothing beside it is placed exactly as before.
 */
export const MIC_SPOTS = ["outside-right", "inside-right", "above-right", "outside-left"] as const;
export type MicSpot = (typeof MIC_SPOTS)[number];

/**
 * C7h: what makes a spot taken. The topmost thing at the spot counts as in the way when it is
 * one of these, or sits inside one: the user would press it instead of the mic. Everything
 * else (text, a background, a decorative box) is fine to sit on top of.
 */
const PRESSABLE_SELECTOR =
  'button, a[href], input, select, textarea, summary, [role="button"], [role="link"], [role="menuitem"], [role="tab"], [role="checkbox"], [role="radio"], [role="switch"]';

export const HOST_TAG = "vtype-root";

export interface AnchorOptions {
  doc?: Document;
  /** Whether the primary pointer can hover. Default: matchMedia("(hover: hover)"). */
  hoverCapable?: () => boolean;
  /** Re-checked when a target-defining attribute of a field changes; false drops its mic. */
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
   * A drag of a thin mic finished, at this distance from where vtype would put it (C7f).
   * Only user drags report; `setOffset` does not.
   */
  onOffsetChange?: (offset: MicOffset) => void;
  /**
   * Asked before every automatic close (the pointer left). False keeps the panel open and the
   * question is asked again after another CLOSE_DELAY_MS, so the panel closes on its own once
   * the answer turns true. C7e uses it to keep the panel while a recording runs: closing it
   * would take away the waveform and the button that stops the recording.
   * Explicit closes (a tap on the mic, the field going away, detach) do not ask.
   */
  canAutoClose?: () => boolean;
  /**
   * C7g: asked before the current field changes because another mic was hovered or pressed.
   * False keeps it where it is — a running recording must not have the panel, the waveform and
   * the stop button walk off to another field.
   */
  canSwitchTarget?: () => boolean;
  /**
   * C7g: the page scrolled or the window resized, so the mics have to be re-placed — and which
   * fields are on screen may have changed too. The outside uses this to look for fields again
   * instead of adding a second pair of scroll/resize listeners.
   */
  onViewportChange?: () => void;
  /**
   * C7h: what is at this point of the page, topmost first — `document.elementsFromPoint`. The
   * anchor asks before putting a mic somewhere, so it can move to another spot when the site
   * has a button of its own there. Left out (or returning nothing) means "the page is empty
   * there", which puts every mic where vtype has always put it.
   */
  elementsAt?: (x: number, y: number) => readonly Element[];
}

export interface Anchor {
  /** Our own host element on <html>. Null until the first mic. */
  readonly host: HTMLElement | null;
  /** The shadow root holding the mics and the panel (closed to the page). */
  readonly root: ShadowRoot | null;
  /** The current field's thin mic, or null when there is no current field. */
  readonly mic: HTMLElement | null;
  /** The panel element (same as `ui.element`). There is only ever one. */
  readonly panel: HTMLElement | null;
  /** The panel's state / hook API (C7). Null until the first mic. */
  readonly ui: Panel | null;
  /** The field the panel belongs to, or null. */
  readonly target: Element | null;
  readonly isOpen: boolean;
  /** Whether the current field's mic is shown (it exists and the field is visible). */
  readonly isVisible: boolean;
  /** How far the user dragged the mics from their default place (C7f). */
  readonly offset: MicOffset;
  /** Whether a drag of a mic is going on right now. */
  readonly isDragging: boolean;
  /** C7g: the field whose mic (or open panel) the pointer is inside, or null. */
  readonly hoveredField: Element | null;
  /** C7g: every field that has a mic right now, the current one included. */
  readonly fields: readonly Element[];
  /** C7h: which spot each mic ended up in (for tests and for looking at what happened). */
  spotOf(field: Element): MicSpot | null;
  /**
   * C7h: look at the page again and decide where each mic goes. For when the page itself
   * changed (a button appeared beside a field); scrolling does not need it, because a field and
   * the buttons around it move together.
   */
  remeasure(): void;
  /**
   * C7g: the fields that should carry a mic. The current field keeps its own mic even when it
   * is not in the list. Fields that are no longer listed lose theirs.
   */
  setMics(fields: readonly Element[]): void;
  /** Put the mics at a stored offset (or back with {x: 0, y: 0}). Does not report back. */
  setOffset(offset: MicOffset): void;
  /** Make `field` the current one (giving it a mic if it has none). */
  attach(field: Element): void;
  /** No current field any more. A mic stays only while `setMics` still asks for it. */
  detach(): void;
  /**
   * Open the panel immediately. No-op when there is no current field. v1 has no caller (the
   * hotkey was dropped); kept as the entry point for a future keyboard path.
   */
  openPanel(): void;
  closePanel(): void;
  /** Recompute the positions now instead of waiting for the next frame. */
  update(): void;
  /** Drop every mic and take the host off the page. */
  destroy(): void;
}

interface Rect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/** One thin mic: its box in the shadow root and the field it belongs to. */
interface MicEntry {
  /** The field, or null while the box sits in the pool waiting to be used again. */
  field: Element | null;
  readonly box: HTMLElement;
  readonly mic: HTMLElement;
  lastRect: DOMRect | null;
  dirty: boolean;
  visible: boolean;
  observer: MutationObserver | null;
  /** C7h: the spot in use. Kept until something asks for a new decision, so it cannot flap. */
  spot: number;
  /** C7h: the spot has to be decided at the next placement (a new mic, a resized field). */
  decide: boolean;
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

interface Spot {
  x: number;
  y: number;
}

/**
 * C7h: where the mic of this field could go, best first, in the order of MIC_SPOTS. The first
 * one is exactly where place() has always put it, so a field with room beside it does not move.
 */
function spotsFor(b: DOMRect, vis: Rect): Spot[] {
  // Centered on the first line (the whole box for a single-line input), kept beside the part
  // of the field that is actually on screen.
  const line = b.top + (Math.min(b.height, FIRST_LINE_BAND_PX) - MIC_SIZE_PX) / 2;
  const y = Math.min(Math.max(line, vis.top), vis.bottom - MIC_SIZE_PX);
  return [
    { x: b.right + MIC_GAP_PX, y }, // outside the right edge
    { x: b.right - MIC_SIZE_PX - VIEWPORT_MARGIN_PX, y }, // inside the right edge
    { x: b.right - MIC_SIZE_PX, y: b.top - MIC_SIZE_PX }, // above it, at its right end
    { x: b.left - MIC_SIZE_PX - MIC_GAP_PX, y }, // outside the left edge
  ];
}

/** Whether the whole mic would be inside the window at this spot. */
function fitsInView(spot: Spot, vw: number, vh: number): boolean {
  if (vw > 0 && (spot.x < VIEWPORT_MARGIN_PX || spot.x + MIC_SIZE_PX > vw - VIEWPORT_MARGIN_PX)) return false;
  if (vh > 0 && (spot.y < 0 || spot.y + MIC_SIZE_PX > vh)) return false;
  return true;
}

/** Whether `el` is something the user would press (or sits inside something they would). */
function isPressable(el: Element): boolean {
  try {
    if (typeof el.closest === "function") return el.closest(PRESSABLE_SELECTOR) !== null;
    return typeof el.matches === "function" && el.matches(PRESSABLE_SELECTOR);
  } catch {
    return false; // a selector an old engine cannot parse is not a reason to move the mic
  }
}

export function createAnchor(options: AnchorOptions = {}): Anchor {
  const doc = options.doc ?? document;
  const hoverCapable = options.hoverCapable ?? defaultHoverCapable(doc);
  const onOpenChange = options.onOpenChange;
  const onMicPress = options.onMicPress;
  const onOffsetChange = options.onOffsetChange;
  const canAutoClose = options.canAutoClose;
  const canSwitchTarget = options.canSwitchTarget;

  let host: HTMLElement | null = null;
  let root: ShadowRoot | null = null;
  let panel: HTMLElement | null = null;
  let ui: Panel | null = null;

  /** Every field that has a mic, in the order the mics were made. */
  const entries = new Map<Element, MicEntry>();
  /** Boxes whose field went away, kept for the next field instead of being thrown away. */
  const pool: MicEntry[] = [];
  /** The fields the outside asked for with setMics (C7g); the current field is extra. */
  let wanted: ReadonlySet<Element> = new Set();

  let target: Element | null = null;
  let hovered: Element | null = null;
  let open = false;
  let openTimer: ReturnType<typeof setTimeout> | null = null;
  let closeTimer: ReturnType<typeof setTimeout> | null = null;
  let frame: number | null = null;
  let tracking = false;

  // C7f: the mic can be dragged aside where a site's own button sits under it.
  let offset: MicOffset = NO_OFFSET;
  /** The pointer holding a mic down, or null. Set from the first pointerdown. */
  let dragPointer: number | null = null;
  /** True once that pointer passed DRAG_THRESHOLD_PX: the release is then not a click. */
  let dragging = false;
  let dragEntry: MicEntry | null = null;
  let dragFromX = 0;
  let dragFromY = 0;
  let dragBase: MicOffset = NO_OFFSET;

  const view = (): (Window & typeof globalThis) | null => doc.defaultView as (Window & typeof globalThis) | null;

  /** The entry of the current field (the one the panel belongs to), if it has one. */
  function current(): MicEntry | null {
    return target === null ? null : (entries.get(target) ?? null);
  }

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
    const entry = current();
    if (panel !== null) panel.hidden = !next;
    entry?.box.classList.toggle("open", next);
    entry?.mic.setAttribute("aria-expanded", next ? "true" : "false");
    // The page may have changed the field's text without an input event (e.g. after sending).
    if (next) {
      ui?.refreshHasText();
      if (entry !== null) entry.dirty = true; // re-place with the panel's real height (flip-y)
    }
    onOpenChange?.(next);
  }

  function isHoverPointer(e: PointerEvent): boolean {
    if (e.pointerType === "touch") return false;
    return hoverCapable();
  }

  /** The panel follows the pointer to another field, unless a recording says otherwise. */
  function maySwitchTo(entry: MicEntry): boolean {
    if (entry.field === null) return false;
    if (entry.field === target) return true;
    return canSwitchTarget === undefined || canSwitchTarget();
  }

  function onBoxEnter(entry: MicEntry, e: PointerEvent): void {
    // While a mic is held the panel neither opens nor closes: a hand that slips must not
    // start a recording, and the panel must not appear under the pointer mid-drag (C7f).
    if (dragPointer !== null) return;
    if (entry.field === null) return;
    hovered = entry.field;
    if (!isHoverPointer(e)) return;
    if (entry.field !== target) {
      if (!maySwitchTo(entry)) return;
      setTarget(entry.field);
    }
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

  function onBoxLeave(entry: MicEntry, e: PointerEvent): void {
    if (dragPointer !== null) return;
    if (hovered === entry.field) hovered = null;
    if (!isHoverPointer(e)) return;
    clearOpenTimer();
    if (open) scheduleClose();
  }

  function onMicActivate(entry: MicEntry): void {
    // C7e: a press on the thin mic means the same on every device (mouse click, touch tap,
    // tap on a device that cannot hover): start or stop the recording, and show the panel.
    // It never closes the panel — while a recording runs the panel holds the only button that
    // stops it, and once idle the pointer leaving closes it as before. Where there is no hover
    // (touch), the panel goes when the field loses focus.
    //
    // The press is handed on *before* the panel opens on purpose: with the `hover` setting the
    // opening starts a recording of its own, and it must find this one already running instead
    // of starting a second one that this press would then immediately stop.
    if (entry.field === null) return;
    if (entry.field !== target) {
      // A recording is running elsewhere: the press still reaches the controller (it stops
      // that recording), but the panel stays with the field being dictated into.
      if (!maySwitchTo(entry)) {
        onMicPress?.();
        return;
      }
      setTarget(entry.field);
    }
    onMicPress?.();
    setOpen(true);
  }

  // ---- dragging the mic aside (C7f) ------------------------------------------------------
  // The pointer is captured so the mic keeps following it over the page's own elements (and
  // so the browser stops sending the box hover events that would open or close the panel).

  function onMicPointerDown(entry: MicEntry, e: PointerEvent): void {
    if (entry.field === null || dragPointer !== null) return;
    dragPointer = e.pointerId;
    dragEntry = entry;
    dragging = false;
    dragFromX = e.clientX;
    dragFromY = e.clientY;
    dragBase = offset;
    clearOpenTimer();
    clearCloseTimer();
    try {
      entry.mic.setPointerCapture(e.pointerId);
    } catch {
      // No capture (an old engine, or a pointer that is already gone): the drag still works
      // while the pointer stays over the mic, which is where it started.
    }
  }

  function onMicPointerMove(entry: MicEntry, e: PointerEvent): void {
    if (dragPointer !== e.pointerId || dragEntry !== entry) return;
    const dx = e.clientX - dragFromX;
    const dy = e.clientY - dragFromY;
    if (!dragging) {
      if (Math.hypot(dx, dy) < DRAG_THRESHOLD_PX) return; // still a click in the making
      dragging = true;
      entry.mic.classList.add("dragging"); // the cursor says the mic is moving, not pressed
    }
    // One offset for the whole page: dragging any mic moves them all (C7f 5).
    offset = { x: dragBase.x + dx, y: dragBase.y + dy };
    update(); // place them now instead of waiting for the next frame
  }

  /** Let the pointer go. True when this was a drag (and so not a click). */
  function endDrag(e: PointerEvent): boolean {
    if (dragPointer !== e.pointerId) return false;
    dragPointer = null;
    try {
      dragEntry?.mic.releasePointerCapture(e.pointerId);
    } catch {
      // Already released (the browser does it on pointerup / pointercancel by itself).
    }
    dragEntry?.mic.classList.remove("dragging");
    dragEntry = null;
    if (!dragging) return false;
    dragging = false;
    // What the user sees is what gets remembered, including a drag cut short by the system.
    onOffsetChange?.(offset);
    return true;
  }

  function cancelDrag(): void {
    dragPointer = null;
    dragging = false;
    dragEntry?.mic.classList.remove("dragging");
    dragEntry = null;
  }

  function onMicPointerUp(entry: MicEntry, e: PointerEvent): void {
    if (endDrag(e)) return; // moving the mic must not start or stop a recording
    onMicActivate(entry);
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
      ui = createPanel({ doc });
      panel = ui.element;
      root.append(style, panel);
    }
    if (!host.isConnected) doc.documentElement.append(host);
  }

  /** A new box with its own mic, wired up. Boxes are made once and then reused (see pool). */
  function createEntry(): MicEntry {
    ensureHost();
    const box = doc.createElement("div");
    box.className = "box";
    box.hidden = true;
    const mic = createTrigger(doc);
    box.append(mic);
    root?.append(box);
    const entry: MicEntry = {
      field: null,
      box,
      mic,
      lastRect: null,
      dirty: true,
      visible: false,
      observer: null,
      spot: 0,
      decide: true,
    };
    box.addEventListener("pointerenter", (e) => onBoxEnter(entry, e as PointerEvent));
    box.addEventListener("pointerleave", (e) => onBoxLeave(entry, e as PointerEvent));
    box.addEventListener("mousedown", keepFieldFocus);
    // The mic must get the whole touch gesture, or the page scrolls instead of it moving.
    mic.style.setProperty("touch-action", "none", "important");
    mic.addEventListener("pointerdown", (e) => onMicPointerDown(entry, e as PointerEvent));
    mic.addEventListener("pointermove", (e) => onMicPointerMove(entry, e as PointerEvent));
    mic.addEventListener("pointerup", (e) => onMicPointerUp(entry, e as PointerEvent));
    mic.addEventListener("pointercancel", (e) => {
      endDrag(e as PointerEvent);
    });
    return entry;
  }

  function ensureEntry(field: Element): MicEntry {
    const existing = entries.get(field);
    if (existing !== undefined) return existing;
    ensureHost();
    const entry = pool.pop() ?? createEntry();
    entry.field = field;
    entry.lastRect = null;
    entry.dirty = true;
    entry.visible = false;
    entry.spot = 0;
    entry.decide = true; // C7h: a mic that has just appeared looks for a free spot
    entries.set(field, entry);
    const isStillTarget = options.isStillTarget;
    if (isStillTarget !== undefined) {
      entry.observer = new MutationObserver(() => {
        if (entry.field !== null && !isStillTarget(entry.field)) release(entry);
      });
      const filter = options.observedAttributes;
      entry.observer.observe(
        field,
        filter === undefined ? { attributes: true } : { attributes: true, attributeFilter: [...filter] },
      );
    }
    startTracking();
    return entry;
  }

  /** This field has no mic any more. The box goes back to the pool, hidden but alive. */
  function release(entry: MicEntry): void {
    const field = entry.field;
    if (field === null) return;
    if (field === target) setTarget(null);
    if (hovered === field) hovered = null;
    if (dragEntry === entry) cancelDrag();
    entry.observer?.disconnect();
    entry.observer = null;
    entry.field = null;
    entry.visible = false;
    entry.lastRect = null;
    entry.box.hidden = true;
    entries.delete(field);
    pool.push(entry);
    stopTrackingIfIdle();
  }

  /** Move the panel to `field`'s box (or park it in the root when there is no field). */
  function setTarget(field: Element | null): void {
    if (target === field) return;
    setOpen(false);
    const previous = current();
    previous?.box.classList.remove("open");
    target = field;
    const entry = current();
    if (entry !== null && panel !== null) {
      entry.box.append(panel); // the panel always sits after the mic of the field it serves
      ui?.setTrigger(entry.mic);
      ui?.bindField(entry.field);
      entry.dirty = true;
    } else {
      if (panel !== null) root?.append(panel);
      ui?.setTrigger(null);
      ui?.bindField(null);
    }
  }

  /**
   * C7h: is the site's own furniture at this spot? Only the topmost thing counts: anything
   * under it cannot be pressed anyway. Our own mics and panel never count, and neither does
   * the field itself (or a box it sits in) — that is what the mic belongs to.
   */
  function isTaken(spot: Spot, field: Element): boolean {
    const probe = options.elementsAt;
    if (probe === undefined) return false;
    let found: readonly Element[];
    try {
      found = probe(spot.x + MIC_SIZE_PX / 2, spot.y + MIC_SIZE_PX / 2);
    } catch {
      return false; // no answer is not a reason to move the mic somewhere unexpected
    }
    for (const el of found) {
      if (host !== null && (el === host || host.contains(el))) continue;
      if (el === field || el.contains(field) || field.contains(el)) return false;
      return isPressable(el);
    }
    return false;
  }

  /**
   * C7h: the first spot that is inside the window and not taken. `checkTaken` is false when the
   * user dragged the mic themselves (C7f): their position wins and nothing is looked up.
   */
  function chooseSpot(spots: readonly Spot[], field: Element, checkTaken: boolean, vw: number, vh: number): number {
    let firstFitting = -1;
    for (let i = 0; i < spots.length; i++) {
      const spot = spots[i];
      if (spot === undefined || !fitsInView(spot, vw, vh)) continue;
      if (firstFitting < 0) firstFitting = i;
      if (!checkTaken) return i;
      if (!isTaken(spot, field)) return i;
    }
    // Every spot is taken (or none fits): back to the best one. A mic that overlaps something
    // can still be dragged aside; a mic that is not there at all cannot.
    return firstFitting < 0 ? 0 : firstFitting;
  }

  /** A field that changed size may have grown into (or away from) the thing beside it. */
  function noteRect(entry: MicEntry, b: DOMRect): void {
    const before = entry.lastRect;
    if (before !== null && (before.width !== b.width || before.height !== b.height)) entry.decide = true;
    entry.lastRect = b;
  }

  function hide(entry: MicEntry): void {
    entry.visible = false;
    entry.box.hidden = true;
    if (entry.field === target) setOpen(false);
  }

  function place(entry: MicEntry): void {
    const field = entry.field;
    if (field === null) return;
    if (!field.isConnected) {
      release(entry);
      return;
    }
    const b = field.getBoundingClientRect();
    const vis = visibleRect(field, doc);
    if (isEmpty(vis)) {
      hide(entry);
      return;
    }
    const w = view();
    const vw = doc.documentElement.clientWidth || (w?.innerWidth ?? 0);
    const vh = doc.documentElement.clientHeight || (w?.innerHeight ?? 0);
    // C7h: the spots this field offers, and which one this mic is in. The decision is only
    // made when something asked for it (a new mic, a field that changed size, remeasure), so
    // scrolling moves the mic without ever looking at the page again — and the spot in use
    // cannot flap from frame to frame.
    const spots = spotsFor(b, vis);
    const manual = offset.x !== 0 || offset.y !== 0;
    if (entry.decide || manual) {
      // C7f: on a site where the user dragged the mic, their position wins and the page is
      // not consulted at all — the choice they made by hand is the answer.
      entry.spot = chooseSpot(spots, field, !manual, vw, vh);
      entry.decide = false;
    }
    const chosen = spots[entry.spot] ?? spots[0] ?? { x: b.right + MIC_GAP_PX, y: b.top };
    // C7f: where the user dragged it to, measured from the spot above, so it keeps its
    // distance while the field moves. The viewport still has the last word: an offset from
    // another window size must not leave the mic somewhere it cannot be reached.
    let x = chosen.x + offset.x;
    let y = chosen.y + offset.y;
    if (vw > 0) x = Math.min(Math.max(x, VIEWPORT_MARGIN_PX), vw - MIC_SIZE_PX - VIEWPORT_MARGIN_PX);
    if (vh > 0) y = Math.min(Math.max(y, 0), vh - MIC_SIZE_PX);
    entry.box.style.left = `${Math.round(x)}px`;
    entry.box.style.top = `${Math.round(y)}px`;
    // The panel opens below-right of the mic; flip it when that would leave the viewport.
    const onPanel = entry.field === target && panel !== null && !panel.hidden;
    const panelHeight = onPanel && panel !== null ? panel.getBoundingClientRect().height : 0;
    entry.box.classList.toggle("flip-x", vw > 0 && x + PANEL_WIDTH_PX > vw - VIEWPORT_MARGIN_PX);
    entry.box.classList.toggle(
      "flip-y",
      vh > 0 && y + MIC_SIZE_PX + (panelHeight || PANEL_HEIGHT_ESTIMATE_PX) > vh - VIEWPORT_MARGIN_PX,
    );
    entry.box.hidden = false;
    entry.visible = true;
  }

  function sameBox(a: DOMRect | null, b: DOMRect): boolean {
    return a !== null && a.left === b.left && a.top === b.top && a.width === b.width && a.height === b.height;
  }

  function tick(): void {
    frame = null;
    // Polling the rects every frame catches fields moving for reasons no event reports
    // (content inserted above them, a sidebar opening, CSS transitions). The work is one
    // getBoundingClientRect per *shown* mic, and the caller caps how many that is (C7g):
    // it never grows with the number of fields on the page.
    for (const entry of [...entries.values()]) {
      const field = entry.field;
      if (field === null) continue;
      const b = field.getBoundingClientRect();
      if (entry.dirty || !sameBox(entry.lastRect, b) || !field.isConnected) {
        entry.dirty = false;
        noteRect(entry, b);
        place(entry);
      }
    }
    if (entries.size > 0) scheduleFrame();
  }

  function scheduleFrame(): void {
    if (frame === null) frame = requestAnimationFrame(tick);
  }

  function markDirty(): void {
    for (const entry of entries.values()) entry.dirty = true;
    options.onViewportChange?.();
  }

  function startTracking(): void {
    if (tracking) return;
    tracking = true;
    const w = view();
    // Capture phase on the document sees scroll events from every scroll container, not only
    // the page itself (scroll does not bubble).
    doc.addEventListener("scroll", markDirty, { capture: true, passive: true });
    w?.addEventListener("resize", markDirty, { passive: true });
    scheduleFrame();
  }

  function stopTrackingIfIdle(): void {
    if (!tracking || entries.size > 0) return;
    tracking = false;
    const w = view();
    doc.removeEventListener("scroll", markDirty, { capture: true });
    w?.removeEventListener("resize", markDirty);
    if (frame !== null) cancelAnimationFrame(frame);
    frame = null;
  }

  function update(): void {
    for (const entry of [...entries.values()]) {
      if (entry.field === null) continue;
      entry.dirty = false;
      noteRect(entry, entry.field.getBoundingClientRect());
      place(entry);
    }
  }

  function remeasure(): void {
    for (const entry of entries.values()) entry.decide = true;
  }

  function attach(field: Element): void {
    if (target === field) {
      update();
      return;
    }
    const previous = target;
    // The previous field gives its box back first, so a page with one field at a time keeps
    // reusing the same box instead of collecting one per field ever focused.
    if (previous !== null) {
      setTarget(null);
      const entry = entries.get(previous);
      if (entry !== undefined && !wanted.has(previous)) release(entry);
    }
    ensureEntry(field);
    setTarget(field);
    update();
    scheduleFrame();
  }

  function detach(): void {
    if (target === null) return;
    const previous = target;
    cancelDrag();
    setTarget(null);
    const entry = entries.get(previous);
    if (entry !== undefined && !wanted.has(previous)) release(entry);
  }

  function setMics(fields: readonly Element[]): void {
    wanted = new Set(fields);
    for (const [field, entry] of [...entries]) {
      if (!wanted.has(field) && field !== target) release(entry);
    }
    for (const field of fields) ensureEntry(field);
    update();
    if (entries.size > 0) scheduleFrame();
  }

  return {
    get host() {
      return host;
    },
    get root() {
      return root;
    },
    get mic() {
      return current()?.mic ?? null;
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
      return current()?.visible ?? false;
    },
    get offset() {
      return offset;
    },
    get isDragging() {
      return dragging;
    },
    get hoveredField() {
      return hovered;
    },
    get fields() {
      return [...entries.keys()];
    },
    spotOf(field: Element): MicSpot | null {
      const entry = entries.get(field);
      return entry === undefined ? null : (MIC_SPOTS[entry.spot] ?? MIC_SPOTS[0]);
    },
    remeasure,
    setMics,
    setOffset(next: MicOffset): void {
      if (dragging) return; // the hand on the mic wins over a value arriving from storage
      if (next.x === offset.x && next.y === offset.y) return;
      offset = { x: next.x, y: next.y };
      update();
    },
    attach,
    detach,
    openPanel(): void {
      const entry = current();
      if (entry !== null && entry.visible) setOpen(true);
    },
    closePanel(): void {
      setOpen(false);
    },
    update,
    destroy(): void {
      detach();
      for (const entry of [...entries.values()]) release(entry);
      wanted = new Set();
      stopTrackingIfIdle();
      ui?.destroy();
      host?.remove();
    },
  };
}
