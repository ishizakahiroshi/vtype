// Input modes: what the recognizer listens for, and how its transcript is shaped before it is
// inserted. Pure functions only (no DOM, no chrome.*), so the extension's offscreen document and
// the desktop app share one implementation.
//
// Order of transformTranscript: replacements -> (kana) reading -> toKatakana. Replacements run
// first so a registered proper noun (e.g. "vtype") is never turned into katakana; the replaced
// spans are passed through untouched by the later steps.

export type InputMode = 'normal' | 'en' | 'kana';

export const INPUT_MODES: readonly InputMode[] = ['normal', 'en', 'kana'];

export function isInputMode(v: unknown): v is InputMode {
  return typeof v === 'string' && (INPUT_MODES as readonly string[]).includes(v);
}

/** Recognition language for a mode. `baseLang` is what normal mode would use (e.g. navigator.language). */
export function recognitionLangFor(mode: InputMode, baseLang: string): string {
  if (mode === 'en') return 'en-US';
  if (mode === 'kana') return 'ja-JP';
  return baseLang;
}

// ---------------------------------------------------------------------------
// Replacement table
// ---------------------------------------------------------------------------

export interface ReplacementRule {
  from: string;
  to: string;
}

export const MAX_REPLACEMENT_RULES = 200;

/** Drops malformed entries, empty `from`, and duplicate `from` (ASCII case-insensitive); caps the count. */
export function normalizeReplacementRules(raw: unknown): ReplacementRule[] {
  if (!Array.isArray(raw)) return [];
  const out: ReplacementRule[] = [];
  const seen = new Set<string>();
  for (const item of raw) {
    if (out.length >= MAX_REPLACEMENT_RULES) break;
    if (!item || typeof item !== 'object') continue;
    const { from, to } = item as { from?: unknown; to?: unknown };
    if (typeof from !== 'string' || typeof to !== 'string' || from === '') continue;
    const key = asciiLower(from);
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({ from, to });
  }
  return out;
}

/** Lower-cases A-Z only, so the length never changes and non-Latin text is left alone. */
function asciiLower(s: string): string {
  return s.replace(/[A-Z]/g, (c) => String.fromCharCode(c.charCodeAt(0) + 32));
}

interface Segment {
  text: string;
  /** True for text produced by a replacement rule; later steps leave it as is. */
  replaced: boolean;
}

function replaceToSegments(text: string, rules: readonly ReplacementRule[]): Segment[] {
  if (rules.length === 0 || text === '') return [{ text, replaced: false }];
  const sorted = rules
    .map((r) => ({ ...r, key: asciiLower(r.from) }))
    .sort((a, b) => b.from.length - a.from.length);
  const lower = asciiLower(text);
  const segments: Segment[] = [];
  let plain = '';
  let i = 0;
  while (i < text.length) {
    const hit = sorted.find((r) => lower.startsWith(r.key, i));
    if (hit) {
      if (plain) segments.push({ text: plain, replaced: false });
      plain = '';
      segments.push({ text: hit.to, replaced: true });
      i += hit.from.length;
    } else {
      plain += text[i];
      i++;
    }
  }
  if (plain) segments.push({ text: plain, replaced: false });
  return segments;
}

/** Longest `from` first, left to right, non-overlapping. ASCII letters match case-insensitively. */
export function applyReplacements(text: string, rules: readonly ReplacementRule[]): string {
  if (rules.length === 0) return text;
  return replaceToSegments(text, rules)
    .map((s) => s.text)
    .join('');
}

// ---------------------------------------------------------------------------
// Katakana
// ---------------------------------------------------------------------------

// U+FF61..U+FF9D in order.
const HALF_TO_FULL =
  '。「」、・ヲァィゥェォャュョッーアイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワン';
const VOICEABLE = 'カキクケコサシスセソタチツテトハヒフヘホ';
const SEMI_VOICEABLE = 'ハヒフヘホ';

function withDakuten(c: string): string | null {
  if (VOICEABLE.includes(c)) return String.fromCharCode(c.charCodeAt(0) + 1);
  if (c === 'ウ') return 'ヴ';
  if (c === 'ワ') return 'ヷ';
  if (c === 'ヲ') return 'ヺ';
  return null;
}

/**
 * Hiragana (U+3041..U+3096, ゝ, ゞ) and half-width katakana to full-width katakana, composing
 * half-width voiced marks (ｶﾞ -> ガ, ﾊﾟ -> パ). Kanji, Latin letters and symbols are unchanged.
 */
export function toKatakana(text: string): string {
  let out = '';
  for (let i = 0; i < text.length; i++) {
    const code = text.charCodeAt(i);
    if (code >= 0x3041 && code <= 0x3096) {
      out += String.fromCharCode(code + 0x60);
    } else if (code === 0x309d || code === 0x309e) {
      out += String.fromCharCode(code + 0x60); // ゝゞ -> ヽヾ
    } else if (code >= 0xff61 && code <= 0xff9d) {
      const full = HALF_TO_FULL[code - 0xff61]!;
      const next = text.charCodeAt(i + 1);
      const voiced = next === 0xff9e ? withDakuten(full) : null;
      const semi = next === 0xff9f && SEMI_VOICEABLE.includes(full) ? String.fromCharCode(full.charCodeAt(0) + 2) : null;
      if (voiced) {
        out += voiced;
        i++;
      } else if (semi) {
        out += semi;
        i++;
      } else {
        out += full;
      }
    } else if (code === 0xff9e) {
      out += '゛';
    } else if (code === 0xff9f) {
      out += '゜';
    } else {
      out += text[i];
    }
  }
  return out;
}

// ---------------------------------------------------------------------------
// Whole transcript
// ---------------------------------------------------------------------------

/** Turns text that may contain kanji into its katakana reading. Supplied only when a dictionary ships. */
export type ReadingProvider = (text: string) => Promise<string>;

export interface TransformOptions {
  mode: InputMode;
  rules: readonly ReplacementRule[];
}

export interface AsyncTransformOptions extends TransformOptions {
  reading?: ReadingProvider;
}

/** Synchronous transform (no reading). Used for interim results. */
export function transformTranscriptSync(text: string, { mode, rules }: TransformOptions): string {
  const segments = replaceToSegments(text, rules);
  if (mode !== 'kana') return segments.map((s) => s.text).join('');
  return segments.map((s) => (s.replaced ? s.text : toKatakana(s.text))).join('');
}

/** Replacements -> (kana) reading -> toKatakana. Replaced spans are never given to `reading`. */
export async function transformTranscript(text: string, options: AsyncTransformOptions): Promise<string> {
  const { mode, rules, reading } = options;
  if (mode !== 'kana' || !reading) return transformTranscriptSync(text, { mode, rules });
  const segments = replaceToSegments(text, rules);
  const parts = await Promise.all(
    segments.map(async (s) => (s.replaced || s.text === '' ? s.text : toKatakana(await reading(s.text)))),
  );
  return parts.join('');
}
