// Kana mode's kanji reading with the real kuromoji dictionary (read from node_modules here; the
// build ships the same files in dist/dict/).

import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { describe, expect, it } from "vitest";
import { transformTranscript } from "vtype-core";
import { createReadingProvider } from "../src/offscreen/reading";

const dicPath = join(dirname(createRequire(import.meta.url).resolve("kuromoji/package.json")), "dict") + "/";

describe("the kanji reading", () => {
  const reading = createReadingProvider(dicPath);

  it("turns a sentence with kanji into katakana in kana mode", async () => {
    expect(await transformTranscript("東京に行く", { mode: "kana", rules: [], reading })).toBe("トウキョウニイク");
  }, 60_000);

  it("keeps words the dictionary has no reading for, and replaced words, as written", async () => {
    const out = await transformTranscript("vtypeで入力する", {
      mode: "kana",
      rules: [{ from: "vtype", to: "vtype" }],
      reading,
    });
    expect(out).toBe("vtypeデニュウリョクスル");
  }, 60_000);

  it("fails, and can be retried, when the dictionary is not there", async () => {
    const broken = createReadingProvider(join(dicPath, "nowhere") + "/");
    await expect(broken("東京")).rejects.toBeDefined();
    await expect(broken("東京")).rejects.toBeDefined();
  }, 60_000);
});
