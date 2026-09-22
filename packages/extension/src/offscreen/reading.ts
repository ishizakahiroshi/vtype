// Kanji -> katakana reading for kana mode (native plan C2), with kuromoji and the IPADIC
// dictionary shipped in dist/dict/ (about 17 MB, see docs/local plan C1's measurement).
//
// The dictionary is loaded the first time a final result in kana mode needs it, not when the
// offscreen document opens: most sessions never use kana mode, and loading takes a moment.
// A failed load is retried on the next use; the caller falls back to kana-only conversion.

import type { ReadingProvider } from "vtype-core";
// Only the tokenizer: kuromoji's index also pulls in the dictionary *builder*, which is not needed.
import TokenizerBuilder from "kuromoji/src/TokenizerBuilder.js";

interface KuromojiToken {
  readonly surface_form: string;
  readonly reading?: string;
}

interface KuromojiTokenizer {
  tokenize(text: string): KuromojiToken[];
}

/** Where build.mjs puts the dictionary, relative to the offscreen document. */
export const DICTIONARY_PATH = "dict/";

export function loadTokenizer(dicPath: string): Promise<KuromojiTokenizer> {
  return new Promise((resolve, reject) => {
    new TokenizerBuilder({ dicPath }).build((err: unknown, tokenizer: KuromojiTokenizer) => {
      if (err) reject(err instanceof Error ? err : new Error(String(err)));
      else resolve(tokenizer);
    });
  });
}

/** Each word's reading (katakana), or the word as written when the dictionary has none. */
export function readingOf(tokenizer: KuromojiTokenizer, text: string): string {
  return tokenizer
    .tokenize(text)
    .map((token) => (token.reading !== undefined && token.reading !== "" && token.reading !== "*" ? token.reading : token.surface_form))
    .join("");
}

export function createReadingProvider(dicPath: string = DICTIONARY_PATH): ReadingProvider {
  let tokenizer: Promise<KuromojiTokenizer> | null = null;
  return async (text) => {
    tokenizer ??= loadTokenizer(dicPath).catch((err: unknown) => {
      tokenizer = null;
      throw err;
    });
    return readingOf(await tokenizer, text);
  };
}
