// Microphone permission page (plan C7b C2).
//
// The offscreen document cannot show a permission prompt: without a prior grant its
// recognition fails with `not-allowed` at once (C1 spike, real Chrome). Granting getUserMedia
// once on this page, which has the same chrome-extension:// origin, lets the offscreen document
// recognise speech on every site with no dialog. This page is opened on install and from the
// panel after a `not-allowed` error. The stream is stopped as soon as it is granted.
//
// Text comes from `_locales/` through shared/i18n (one file per language).

import { translator } from "../shared/i18n";

type GetUserMedia = (constraints: MediaStreamConstraints) => Promise<MediaStream>;

export interface PermissionPageOptions {
  doc?: Document;
  getUserMedia?: GetUserMedia;
  queryMicrophone?: () => Promise<string>;
  language?: string;
}

function setText(doc: Document, id: string, text: string, className?: string): void {
  const el = doc.getElementById(id);
  if (el === null) return;
  el.textContent = text;
  if (className !== undefined) el.className = className;
}

export function initPermissionPage(options: PermissionPageOptions = {}): void {
  const doc = options.doc ?? document;
  const nav = globalThis.navigator;
  const t = translator(options.language ?? nav?.language);
  const getUserMedia: GetUserMedia | undefined =
    options.getUserMedia ?? (nav?.mediaDevices ? (c) => nav.mediaDevices.getUserMedia(c) : undefined);
  const queryMicrophone =
    options.queryMicrophone ??
    (async () => (await nav.permissions.query({ name: "microphone" as PermissionName })).state);

  setText(doc, "title", t("permissionTitle"));
  setText(doc, "lead", t("permissionLead"));
  setText(doc, "grant", t("permissionButton"));
  setText(doc, "after", t("permissionAfter"));

  void queryMicrophone()
    .then((state) => {
      if (state === "granted") setText(doc, "status", t("permissionAlready"), "ok");
    })
    .catch(() => undefined);

  doc.getElementById("grant")?.addEventListener("click", async () => {
    if (getUserMedia === undefined) {
      setText(doc, "status", t("permissionDenied"), "err");
      return;
    }
    setText(doc, "status", t("permissionRequesting"), "");
    try {
      const stream = await getUserMedia({ audio: true });
      for (const track of stream.getTracks()) track.stop();
      setText(doc, "status", t("permissionGranted"), "ok");
    } catch {
      setText(doc, "status", t("permissionDenied"), "err");
    }
  });
}

if (typeof document !== "undefined" && document.getElementById("grant") !== null) initPermissionPage();
