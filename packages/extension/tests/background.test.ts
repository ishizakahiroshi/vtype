import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FakeChromeHub, FakeSpeechRecognition, flush } from "./fake-chrome";

// Background service worker over the fake Chrome bus, with the real offscreen module and the
// real vtype-core recognizer (fake SpeechRecognition underneath).

let hub: FakeChromeHub;
const inbox = new Map<string, unknown[]>();

function listen(tabId: number, frameId: number): void {
  const key = `${tabId}:${frameId}`;
  inbox.set(key, []);
  hub.contentRuntime(tabId, frameId).onMessage.addListener((m) => inbox.get(key)?.push(m));
}

function kinds(tabId: number, frameId: number): string[] {
  return (inbox.get(`${tabId}:${frameId}`) ?? []).map((m) => {
    const e = (m as { event: { kind: string; reason?: string } }).event;
    return e.kind === "ended" ? `ended:${e.reason}` : e.kind;
  });
}

async function send(tabId: number, frameId: number, message: unknown): Promise<void> {
  await hub.contentRuntime(tabId, frameId).sendMessage(message);
  await flush(vi);
}

beforeEach(() => {
  vi.useFakeTimers();
  FakeSpeechRecognition.reset();
  inbox.clear();
  hub = new FakeChromeHub();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("offscreen document lifecycle", () => {
  it("is created lazily, once, with reasons USER_MEDIA", async () => {
    hub.startBackground();
    listen(1, 0);
    expect(hub.createDocumentCalls).toHaveLength(0);
    await send(1, 0, { target: "background", type: "start", sessionId: "a" });
    expect(hub.createDocumentCalls).toEqual(["chrome-extension://synthetic-id/offscreen.html"]);
    await send(1, 0, { target: "background", type: "stop", sessionId: "a" });
    await send(1, 0, { target: "background", type: "start", sessionId: "b" });
    expect(hub.createDocumentCalls).toHaveLength(1);
  });

  it("two starts racing the creation create one document", async () => {
    const bg = hub.startBackground();
    listen(1, 0);
    listen(2, 0);
    hub.holdCreate = true;
    const p1 = bg.ensureOffscreen();
    const p2 = bg.ensureOffscreen();
    await flush(vi);
    hub.release();
    await Promise.all([p1, p2]);
    expect(hub.createDocumentCalls).toHaveLength(1);
    expect(bg.lastOffscreenCheck).toBe("awaited");
  });

  it("a restarted service worker reuses the open document (getContexts)", async () => {
    hub.startBackground();
    listen(1, 0);
    await send(1, 0, { target: "background", type: "start", sessionId: "a" });
    // Chrome stops and restarts the worker: a new background instance, same offscreen document.
    hub.backgroundListeners.length = 0;
    const bg2 = hub.startBackground();
    await bg2.ensureOffscreen();
    expect(hub.createDocumentCalls).toHaveLength(1);
    expect(bg2.lastOffscreenCheck).toBe("getContexts");
  });

  it("falls back to offscreen.hasDocument when getContexts is missing", async () => {
    hub.withGetContexts = false;
    hub.openOffscreen();
    const bg = hub.startBackground();
    await bg.ensureOffscreen();
    expect(hub.createDocumentCalls).toHaveLength(0);
    expect(bg.lastOffscreenCheck).toBe("hasDocument");
  });

  it("a restarted worker still routes results of the running session (owner travels with events)", async () => {
    hub.startBackground();
    listen(1, 0);
    await send(1, 0, { target: "background", type: "start", sessionId: "a" });
    hub.backgroundListeners.length = 0;
    hub.startBackground();
    const sr = FakeSpeechRecognition.started();
    sr.fireStart();
    sr.fireResult("still routed", true);
    await flush(vi);
    expect(kinds(1, 0)).toEqual(["started", "result"]);
  });

  it("a failed creation ends the session with an error instead of hanging", async () => {
    const chrome = hub.backgroundChrome();
    chrome.offscreen.createDocument = async () => {
      throw new Error("synthetic failure");
    };
    const { createBackground } = await import("../src/background/index");
    createBackground(chrome);
    listen(1, 0);
    await send(1, 0, { target: "background", type: "start", sessionId: "a" });
    const last = inbox.get("1:0")?.at(-1) as { event: unknown };
    expect(last.event).toEqual({ kind: "ended", reason: "error", code: "offscreen-unavailable" });
  });
});

describe("routing to the requesting frame", () => {
  it("replies to the frame that asked, not to the other frames of the tab", async () => {
    hub.startBackground();
    listen(1, 0);
    listen(1, 7);
    await send(1, 7, { target: "background", type: "start", sessionId: "a" });
    const sr = FakeSpeechRecognition.started();
    sr.fireStart();
    sr.fireResult("in the iframe", true);
    await flush(vi);
    expect(kinds(1, 7)).toEqual(["started", "result"]);
    expect(kinds(1, 0)).toEqual([]);
    expect(hub.toContent.every((d) => d.tabId === 1 && d.frameId === 7)).toBe(true);
  });

  it("content-script messages also reach the offscreen document, which ignores them", async () => {
    hub.startBackground();
    listen(1, 0);
    await send(1, 0, { target: "background", type: "start", sessionId: "a" });
    expect(FakeSpeechRecognition.instances.filter((i) => i.startCalls > 0)).toHaveLength(1);
  });
});

describe("one session at a time (plan C7b check 5)", () => {
  it("a start from another tab ends the previous tab's session first", async () => {
    hub.startBackground();
    listen(1, 0);
    listen(2, 0);
    await send(1, 0, { target: "background", type: "start", sessionId: "a" });
    const first = FakeSpeechRecognition.started();
    first.fireStart();
    await flush(vi);
    await send(2, 0, { target: "background", type: "start", sessionId: "b" });
    expect(kinds(1, 0)).toEqual(["started", "ended:superseded"]);
    expect(first.abortCalls).toBe(1);
    const second = FakeSpeechRecognition.started();
    expect(second).not.toBe(first);
    second.fireStart();
    await flush(vi);
    expect(kinds(2, 0)).toEqual(["started"]);
    // the superseded notice went out before the new session started
    const order = hub.toContent.map((d) => `${d.tabId}:${(d.message as { event: { kind: string } }).event.kind}`);
    expect(order.indexOf("1:ended")).toBeLessThan(order.indexOf("2:started"));
  });

  it("a start from another frame of the same tab also supersedes", async () => {
    hub.startBackground();
    listen(1, 0);
    listen(1, 3);
    await send(1, 0, { target: "background", type: "start", sessionId: "a" });
    await send(1, 3, { target: "background", type: "start", sessionId: "b" });
    expect(kinds(1, 0)).toEqual(["ended:superseded"]);
  });

  it("closing the owning tab stops its session", async () => {
    hub.startBackground();
    listen(1, 0);
    await send(1, 0, { target: "background", type: "start", sessionId: "a" });
    const sr = FakeSpeechRecognition.started();
    sr.fireStart();
    await flush(vi);
    hub.closeTab(1);
    await flush(vi);
    expect(sr.abortCalls).toBe(1);
    expect(hub.offscreen?.sessionId).toBeNull();
  });

  it("closing another tab does not stop the session", async () => {
    hub.startBackground();
    listen(1, 0);
    await send(1, 0, { target: "background", type: "start", sessionId: "a" });
    hub.closeTab(9);
    await flush(vi);
    expect(hub.offscreen?.sessionId).toBe("a");
  });

  it("an owner that can no longer be reached (navigated away) gets its session aborted", async () => {
    hub.startBackground();
    listen(1, 0);
    await send(1, 0, { target: "background", type: "start", sessionId: "a" });
    const sr = FakeSpeechRecognition.started();
    hub.dropContent(1);
    sr.fireStart();
    await flush(vi);
    expect(hub.offscreen?.sessionId).toBeNull();
    expect(sr.abortCalls).toBe(1);
  });
});

describe("permission page (plan C7b C2)", () => {
  it("opens on install, not on update", () => {
    hub.startBackground();
    for (const l of hub.installedListeners) l({ reason: "update" });
    expect(hub.createdTabs).toEqual([]);
    for (const l of hub.installedListeners) l({ reason: "install" });
    expect(hub.createdTabs).toEqual(["chrome-extension://synthetic-id/permission.html"]);
  });

  it("opens when a content script asks", async () => {
    hub.startBackground();
    listen(1, 0);
    await send(1, 0, { target: "background", type: "open-permission" });
    expect(hub.createdTabs).toEqual(["chrome-extension://synthetic-id/permission.html"]);
  });

  it("the page asks for the microphone once and releases it immediately", async () => {
    const { initPermissionPage } = await import("../src/permission/permission");
    document.body.innerHTML = `<h1 id="title"></h1><p id="lead"></p><button id="grant"></button><p id="status"></p><p id="after"></p>`;
    const stopped: string[] = [];
    const getUserMedia = vi.fn(async () => ({
      getTracks: () => [{ stop: () => stopped.push("track") }],
    }) as unknown as MediaStream);
    initPermissionPage({ getUserMedia, queryMicrophone: async () => "prompt", language: "en" });
    document.getElementById("grant")?.click();
    await flush(vi);
    expect(getUserMedia).toHaveBeenCalledTimes(1);
    expect(getUserMedia).toHaveBeenCalledWith({ audio: true });
    expect(stopped).toEqual(["track"]);
    expect(document.getElementById("status")?.textContent).toContain("Allowed");
    document.body.innerHTML = "";
  });

  it("the page reports a refusal", async () => {
    const { initPermissionPage } = await import("../src/permission/permission");
    document.body.innerHTML = `<button id="grant"></button><p id="status"></p>`;
    initPermissionPage({
      getUserMedia: async () => {
        throw new DOMException("denied", "NotAllowedError");
      },
      queryMicrophone: async () => "prompt",
      language: "ja",
    });
    document.getElementById("grant")?.click();
    await flush(vi);
    expect(document.getElementById("status")?.textContent).toContain("許可されませんでした");
    document.body.innerHTML = "";
  });

  it("a stop with no offscreen document is answered with ended:user", async () => {
    hub.startBackground();
    listen(1, 0);
    await send(1, 0, { target: "background", type: "stop", sessionId: "ghost" });
    expect(kinds(1, 0)).toEqual(["ended:user"]);
  });
});
