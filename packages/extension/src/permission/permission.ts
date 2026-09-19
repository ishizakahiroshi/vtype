// Microphone permission page (plan C7b C2).
//
// The offscreen document cannot show a permission prompt: without a prior grant its
// recognition fails with `not-allowed` at once (C1 spike, real Chrome). Granting getUserMedia
// once on this page, which has the same chrome-extension:// origin, lets the offscreen document
// recognise speech on every site with no dialog. This page is opened on install and from the
// panel after a `not-allowed` error. The stream is stopped as soon as it is granted.

interface Texts {
  title: string;
  lead: string;
  button: string;
  requesting: string;
  granted: string;
  denied: string;
  after: string;
  already: string;
}

const TEXTS: Record<"en" | "ja", Texts> = {
  en: {
    title: "vtype needs the microphone",
    lead: "vtype turns your speech into text in web text fields. Allow the microphone once here; after that no site will ask again.",
    button: "Allow the microphone",
    requesting: "Waiting for your answer in the browser dialog...",
    granted: "Allowed. You can close this tab.",
    denied: "Not allowed. Allow the microphone for this extension in the address bar's site settings, then press the button again.",
    after: "To use vtype: focus a text field, rest the mouse on the small mic next to it, and press the mic in the panel.",
    already: "The microphone is already allowed. You can close this tab.",
  },
  ja: {
    title: "vtype にマイクの使用を許可してください",
    lead: "vtype は話した言葉を Web の入力欄に文字で入れます。ここで一度だけマイクを許可すると、以降はどのサイトでも確認は出ません。",
    button: "マイクを許可する",
    requesting: "ブラウザの確認ダイアログで許可してください…",
    granted: "許可されました。このタブは閉じてかまいません。",
    denied: "許可されませんでした。アドレスバーのサイト設定でこの拡張のマイクを許可してから、もう一度ボタンを押してください。",
    after: "使い方: 入力欄をクリックし、横に出る小さなマイクにマウスを乗せて、パネルのマイクを押します。",
    already: "マイクはすでに許可されています。このタブは閉じてかまいません。",
  },
};

export function textsFor(language: string | undefined): Texts {
  return language?.toLowerCase().startsWith("ja") ? TEXTS.ja : TEXTS.en;
}

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
  const t = textsFor(options.language ?? nav?.language);
  const getUserMedia: GetUserMedia | undefined =
    options.getUserMedia ?? (nav?.mediaDevices ? (c) => nav.mediaDevices.getUserMedia(c) : undefined);
  const queryMicrophone =
    options.queryMicrophone ??
    (async () => (await nav.permissions.query({ name: "microphone" as PermissionName })).state);

  setText(doc, "title", t.title);
  setText(doc, "lead", t.lead);
  setText(doc, "grant", t.button);
  setText(doc, "after", t.after);

  void queryMicrophone()
    .then((state) => {
      if (state === "granted") setText(doc, "status", t.already, "ok");
    })
    .catch(() => undefined);

  doc.getElementById("grant")?.addEventListener("click", async () => {
    if (getUserMedia === undefined) {
      setText(doc, "status", t.denied, "err");
      return;
    }
    setText(doc, "status", t.requesting, "");
    try {
      const stream = await getUserMedia({ audio: true });
      for (const track of stream.getTracks()) track.stop();
      setText(doc, "status", t.granted, "ok");
    } catch {
      setText(doc, "status", t.denied, "err");
    }
  });
}

if (typeof document !== "undefined" && document.getElementById("grant") !== null) initPermissionPage();
