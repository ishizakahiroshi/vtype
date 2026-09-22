// The Native Messaging vocabulary, checked against the fixture the Rust side reads too.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { isExtensionToNative, isNativeToExtension } from "../src/shared/native-messages";

const fixturePath = join(
  dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
  "native",
  "tests",
  "fixtures",
  "nm-messages.json",
);
const fixture = JSON.parse(readFileSync(fixturePath, "utf8")) as Record<string, unknown[]>;

describe("the shared fixture", () => {
  it("accepts every message the desktop app may send", () => {
    expect(fixture.toExtension!.length).toBeGreaterThan(5);
    for (const m of fixture.toExtension!) expect(isNativeToExtension(m), JSON.stringify(m)).toBe(true);
  });

  it("accepts every message the extension may send", () => {
    expect(fixture.fromExtension!.length).toBeGreaterThan(5);
    for (const m of fixture.fromExtension!) expect(isExtensionToNative(m), JSON.stringify(m)).toBe(true);
  });

  it("refuses the invalid ones, in both directions", () => {
    for (const m of fixture.invalidToExtension!) expect(isNativeToExtension(m), JSON.stringify(m)).toBe(false);
    for (const m of fixture.invalidFromExtension!) expect(isExtensionToNative(m), JSON.stringify(m)).toBe(false);
  });

  it("keeps the two directions apart", () => {
    expect(isNativeToExtension({ type: "session", event: { kind: "started" } })).toBe(false);
    expect(isExtensionToNative({ type: "open-options" })).toBe(false);
  });
});
