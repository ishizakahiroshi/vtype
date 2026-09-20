// Buttons: the thin mic beside the field (the hover target) and the panel's toolbar
// × / send / mic, in the same order as many-ai-cli's input bar (web/src/index.html
// #input-clear-btn, #send-btn, #voice-btn; web/src/app.ts applyToolsPosition).
//
// Icons are inline SVG built with createElementNS (no innerHTML: pages that enforce Trusted
// Types would reject it, and emoji/glyph icons depend on the page's fonts).
// All class names are only meaningful inside vtype's shadow root (styles.css).

export type UiState = "idle" | "recording" | "processing";

export const UI_STATES: readonly UiState[] = ["idle", "recording", "processing"];

const SVG_NS = "http://www.w3.org/2000/svg";

interface Labels {
  trigger: string;
  /** Tooltip on the thin mic: it is both a button and something you can drag aside (C7f). */
  triggerHint: string;
  clear: string;
  send: string;
  micStart: string;
  micStop: string;
  /** C9: switch vtype off on this site, from the panel itself. */
  siteOff: string;
}

const LABELS: Record<"en" | "ja", Labels> = {
  en: {
    trigger: "vtype voice input",
    triggerHint: "Press to dictate. Drag to move it out of the way.",
    clear: "Clear the field",
    send: "Send",
    micStart: "Start voice input",
    micStop: "Stop voice input",
    siteOff: "Don't use vtype on this site",
  },
  ja: {
    trigger: "vtype 音声入力",
    triggerHint: "押すと音声入力。ドラッグでずらせます。",
    clear: "入力欄を空にする",
    send: "送信",
    micStart: "音声入力を開始",
    micStop: "音声入力を停止",
    siteOff: "このサイトでは使わない",
  },
};

/** Labels in the browser's language (not the page's): the UI belongs to the user. */
export function labelsFor(language: string | undefined): Labels {
  return language?.toLowerCase().startsWith("ja") ? LABELS.ja : LABELS.en;
}

function currentLabels(doc: Document): Labels {
  return labelsFor(doc.defaultView?.navigator.language);
}

type SvgChild = [tag: string, attrs: Record<string, string>];

function svg(doc: Document, viewBox: string, attrs: Record<string, string>, children: SvgChild[]): SVGSVGElement {
  const root = doc.createElementNS(SVG_NS, "svg");
  root.setAttribute("viewBox", viewBox);
  root.setAttribute("aria-hidden", "true");
  root.setAttribute("focusable", "false");
  for (const [k, v] of Object.entries(attrs)) root.setAttribute(k, v);
  for (const [tag, childAttrs] of children) {
    const child = doc.createElementNS(SVG_NS, tag);
    for (const [k, v] of Object.entries(childAttrs)) child.setAttribute(k, v);
    root.append(child);
  }
  return root;
}

function micIcon(doc: Document): SVGSVGElement {
  return svg(
    doc,
    "0 0 24 24",
    { fill: "none", stroke: "currentColor", "stroke-width": "2", "stroke-linecap": "round", "stroke-linejoin": "round" },
    [
      ["rect", { x: "9", y: "3", width: "6", height: "11", rx: "3" }],
      ["path", { d: "M5 11a7 7 0 0 0 14 0" }],
      ["line", { x1: "12", y1: "18", x2: "12", y2: "21" }],
    ],
  );
}

function clearIcon(doc: Document): SVGSVGElement {
  // many-ai-cli #input-clear-btn
  return svg(doc, "0 0 13 13", { fill: "none", stroke: "currentColor", "stroke-width": "2", "stroke-linecap": "round" }, [
    ["line", { x1: "2", y1: "2", x2: "11", y2: "11" }],
    ["line", { x1: "11", y1: "2", x2: "2", y2: "11" }],
  ]);
}

function sendIcon(doc: Document): SVGSVGElement {
  // many-ai-cli #send-btn shows "➤"; drawn here so it does not depend on the page's fonts.
  return svg(doc, "0 0 16 16", { fill: "currentColor" }, [["path", { d: "M2 2.5 14.5 8 2 13.5 4.2 8Z" }]]);
}

function button(doc: Document, className: string, label: string, icon: SVGSVGElement): HTMLButtonElement {
  const b = doc.createElement("button");
  b.type = "button";
  b.className = `btn ${className}`;
  b.setAttribute("aria-label", label);
  b.title = label;
  b.append(icon);
  return b;
}

/** Put exactly one state class (`recording` / `processing`, none for idle) on `el`. */
export function applyStateClass(el: Element, state: UiState): void {
  el.classList.toggle("recording", state === "recording");
  el.classList.toggle("processing", state === "processing");
}

/**
 * The thin mic shown beside the focused field. Hovering opens the panel; pressing starts and
 * stops the recording (C7e) and dragging moves it aside (C7f), which the tooltip says because
 * nothing else shows it. It is kept out of the tab order: v1 has no keyboard path to it (the
 * hotkey was dropped).
 */
export function createTrigger(doc: Document): HTMLButtonElement {
  const labels = currentLabels(doc);
  const b = doc.createElement("button");
  b.type = "button";
  b.className = "mic";
  b.tabIndex = -1;
  b.setAttribute("aria-label", labels.trigger);
  b.title = labels.triggerHint;
  b.setAttribute("aria-haspopup", "true");
  b.setAttribute("aria-expanded", "false");
  b.append(micIcon(doc));
  return b;
}

/**
 * C9: the one line in the panel that switches vtype off on this site. Written as a small text
 * button rather than a fourth icon: the toolbar row is what the user reaches for while
 * dictating, and this is the opposite of that — pressed once, and then never again on this
 * site. The way back is the toolbar icon of the extension (the panel is gone by then).
 */
export function createSiteOffButton(doc: Document): HTMLButtonElement {
  const labels = currentLabels(doc);
  const b = doc.createElement("button");
  b.type = "button";
  b.className = "site-off";
  b.textContent = labels.siteOff;
  b.title = labels.siteOff;
  return b;
}

export interface Toolbar {
  readonly element: HTMLElement;
  readonly clearButton: HTMLButtonElement;
  readonly sendButton: HTMLButtonElement;
  readonly micButton: HTMLButtonElement;
  /** × is usable only while the field has text. */
  setHasText(hasText: boolean): void;
  setState(state: UiState): void;
}

export function createToolbar(doc: Document): Toolbar {
  const labels = currentLabels(doc);
  const element = doc.createElement("div");
  element.className = "toolbar";
  const clearButton = button(doc, "clear", labels.clear, clearIcon(doc));
  const sendButton = button(doc, "send", labels.send, sendIcon(doc));
  const micButton = button(doc, "record", labels.micStart, micIcon(doc));
  micButton.setAttribute("aria-pressed", "false");
  element.append(clearButton, sendButton, micButton);

  function setHasText(hasText: boolean): void {
    clearButton.classList.toggle("has-text", hasText);
    // Hidden by opacity (keeps the row from shifting, as in many-ai-cli), so also take it out
    // of the accessibility tree and make it unclickable while there is nothing to clear.
    clearButton.disabled = !hasText;
    clearButton.setAttribute("aria-hidden", hasText ? "false" : "true");
    clearButton.tabIndex = hasText ? 0 : -1;
  }

  function setState(state: UiState): void {
    applyStateClass(micButton, state);
    const recording = state === "recording";
    micButton.setAttribute("aria-pressed", recording ? "true" : "false");
    const label = recording ? labels.micStop : labels.micStart;
    micButton.setAttribute("aria-label", label);
    micButton.title = label;
  }

  setHasText(false);
  setState("idle");
  return { element, clearButton, sendButton, micButton, setHasText, setState };
}
