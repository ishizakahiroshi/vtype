// The panel that opens from the thin mic: transcript line, message line, and the toolbar
// × / send / mic. It renders state; it does not recognise speech or send anything itself.
//
// Hooks for later steps (assign a function; null means "not wired yet", pressing does nothing):
//   panel.onMic   ... C7b: start / stop recognition
//   panel.onSend  ... C8: submit the page's form
//   panel.onClear ... notified after × has cleared the field (× itself calls C6's clearField)
//   panel.onSiteOff ... C9: "don't use vtype on this site" was pressed
// State API for C7b: setState("idle" | "recording" | "processing"), setTranscript, setMessage.
//
// The field is only read (hasText) and listened to (input events, on its root node). It is
// never written here except through clearField when the user presses ×.

import { clearField, hasText } from "../content/clear";
import { applyStateClass, createSiteOffButton, createToolbar, type Toolbar, type UiState } from "./toolbar";
import { createWaveform, type Waveform, type WaveformActivity } from "./waveform";

export type { UiState } from "./toolbar";

export interface MessageAction {
  readonly label: string;
  readonly run: () => void;
}

export interface PanelOptions {
  doc?: Document;
  /** The thin mic beside the field; mirrors the recording / processing state. */
  trigger?: HTMLElement | null;
}

export interface Panel {
  readonly element: HTMLElement;
  readonly toolbar: Toolbar;
  /** The bars shown while recording (C7c). */
  readonly waveform: Waveform;
  readonly state: UiState;
  /** The field × clears and whose text decides whether × is shown. */
  readonly field: Element | null;
  onMic: (() => void) | null;
  onSend: (() => void) | null;
  onClear: (() => void) | null;
  /** C9: the user asked for vtype to stay off on this site. */
  onSiteOff: (() => void) | null;
  setState(state: UiState): void;
  /** A recognition activity event arrived (C7c): moves the bars. */
  setActivity(kind: WaveformActivity): void;
  /** Confirmed text and the not-yet-final tail (shown dimmed). Empty strings hide the line. */
  setTranscript(finalText: string, interimText?: string): void;
  /**
   * A short message (errors). Null or "" hides it. `action` adds one small button after the
   * text (C7b: "allow the microphone" after a not-allowed error).
   */
  setMessage(text: string | null, action?: MessageAction): void;
  bindField(field: Element | null): void;
  /**
   * C7g: the thin mic that mirrors the state. There is one panel and several mics now, so it
   * changes when the panel moves to another field. The mic it leaves goes back to idle.
   */
  setTrigger(next: HTMLElement | null): void;
  /**
   * C9: whether the "don't use vtype on this site" line is offered. Hidden where pressing it
   * could not be honoured (no chrome.storage to remember it in), so it never lies.
   */
  showSiteOff(show: boolean): void;
  /** Re-read whether the field has text (the anchor calls this when the panel opens). */
  refreshHasText(): void;
  /** Remove listeners on the page (on the bound field's root). */
  destroy(): void;
}

export function createPanel(options: PanelOptions = {}): Panel {
  const doc = options.doc ?? document;
  let trigger = options.trigger ?? null;

  const element = doc.createElement("div");
  element.className = "panel";
  element.setAttribute("role", "group");
  element.hidden = true;

  const transcript = doc.createElement("p");
  transcript.className = "transcript";
  transcript.setAttribute("aria-live", "polite");
  transcript.hidden = true;
  const finalSpan = doc.createElement("span");
  finalSpan.className = "final";
  const interimSpan = doc.createElement("span");
  interimSpan.className = "interim";
  transcript.append(finalSpan, interimSpan);

  const message = doc.createElement("p");
  message.className = "message";
  message.setAttribute("role", "status");
  message.hidden = true;

  const waveform = createWaveform({ doc });
  waveform.element.hidden = true;
  const toolbar = createToolbar(doc);
  // C9: last, and hidden until the content script says it can be honoured.
  const siteOff = createSiteOffButton(doc);
  siteOff.hidden = true;
  element.append(waveform.element, transcript, message, toolbar.element, siteOff);

  let state: UiState = "idle";
  let field: Element | null = null;
  let listenRoot: Node | null = null;

  const panel: Panel = {
    element,
    toolbar,
    waveform,
    get state() {
      return state;
    },
    get field() {
      return field;
    },
    onMic: null,
    onSend: null,
    onClear: null,
    onSiteOff: null,
    setState,
    setActivity(kind: WaveformActivity): void {
      waveform.setActivity(kind);
    },
    setTranscript,
    setMessage,
    bindField,
    setTrigger,
    showSiteOff(show: boolean): void {
      siteOff.hidden = !show;
    },
    refreshHasText,
    destroy(): void {
      bindField(null);
      waveform.stop();
    },
  };

  function setState(next: UiState): void {
    const previous = state;
    state = next;
    toolbar.setState(next);
    applyStateClass(element, next);
    if (trigger !== null) applyStateClass(trigger, next);
    // The bars run while the microphone is open and while the last result is awaited; they
    // disappear when the panel goes idle.
    if (next === "recording" && previous !== "recording") waveform.start();
    else if (next === "idle") waveform.stop();
  }

  function setTrigger(next: HTMLElement | null): void {
    if (trigger === next) return;
    // The mic left behind must not keep glowing as if it were still the one recording.
    if (trigger !== null) applyStateClass(trigger, "idle");
    trigger = next;
    if (trigger !== null) applyStateClass(trigger, state);
  }

  function setTranscript(finalText: string, interimText = ""): void {
    finalSpan.textContent = finalText;
    interimSpan.textContent = interimText;
    transcript.hidden = finalText === "" && interimText === "";
  }

  function setMessage(text: string | null, action?: MessageAction): void {
    message.replaceChildren();
    const empty = text === null || text === "";
    if (!empty) {
      const span = doc.createElement("span");
      span.textContent = text;
      message.append(span);
      if (action !== undefined) {
        const b = doc.createElement("button");
        b.type = "button";
        b.className = "message-action";
        b.textContent = action.label;
        b.addEventListener("click", () => action.run());
        message.append(" ", b);
      }
    }
    message.hidden = empty;
  }

  function refreshHasText(): void {
    toolbar.setHasText(field !== null && field.isConnected && hasText(field));
  }

  function onFieldInput(e: Event): void {
    if (field !== null && e.composedPath().includes(field)) refreshHasText();
  }

  function bindField(next: Element | null): void {
    if (listenRoot !== null) {
      listenRoot.removeEventListener("input", onFieldInput, true);
      listenRoot = null;
    }
    field = next;
    if (next !== null) {
      // Listen on the field's root (document or shadow root), not on the field itself.
      listenRoot = next.getRootNode();
      listenRoot.addEventListener("input", onFieldInput, true);
    }
    refreshHasText();
  }

  toolbar.clearButton.addEventListener("click", () => {
    if (field === null) return;
    const cleared = clearField(field);
    refreshHasText();
    if (cleared) panel.onClear?.();
  });
  toolbar.sendButton.addEventListener("click", () => {
    panel.onSend?.();
  });
  toolbar.micButton.addEventListener("click", () => {
    panel.onMic?.();
  });
  siteOff.addEventListener("click", () => {
    panel.onSiteOff?.();
  });

  setState("idle");
  return panel;
}
