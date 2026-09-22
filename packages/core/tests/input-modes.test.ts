import { describe, expect, it, vi } from 'vitest';
import {
  MAX_REPLACEMENT_RULES,
  applyReplacements,
  isInputMode,
  normalizeReplacementRules,
  recognitionLangFor,
  toKatakana,
  transformTranscript,
  transformTranscriptSync,
} from '../src/index.js';

describe('recognitionLangFor', () => {
  it('keeps the base language in normal mode and fixes it in en / kana', () => {
    expect(recognitionLangFor('normal', 'ja')).toBe('ja');
    expect(recognitionLangFor('normal', 'fr-FR')).toBe('fr-FR');
    expect(recognitionLangFor('en', 'ja')).toBe('en-US');
    expect(recognitionLangFor('kana', 'en-US')).toBe('ja-JP');
  });

  it('recognizes only the three modes', () => {
    expect(isInputMode('normal')).toBe(true);
    expect(isInputMode('en')).toBe(true);
    expect(isInputMode('kana')).toBe(true);
    expect(isInputMode('katakana')).toBe(false);
    expect(isInputMode(undefined)).toBe(false);
  });
});

describe('toKatakana', () => {
  it('turns hiragana into katakana, including small kana and iteration marks', () => {
    expect(toKatakana('ありがとう')).toBe('アリガトウ');
    expect(toKatakana('きゃっぷ')).toBe('キャップ');
    expect(toKatakana('ゔ')).toBe('ヴ');
    expect(toKatakana('ゝゞ')).toBe('ヽヾ');
  });

  it('leaves full-width katakana as it is', () => {
    expect(toKatakana('カタカナ')).toBe('カタカナ');
  });

  it('widens half-width katakana and composes voiced marks', () => {
    expect(toKatakana('ｶﾀｶﾅ')).toBe('カタカナ');
    expect(toKatakana('ｶﾞｷﾞ')).toBe('ガギ');
    expect(toKatakana('ﾊﾟﾋﾟﾌﾟ')).toBe('パピプ');
    expect(toKatakana('ｳﾞ')).toBe('ヴ');
    expect(toKatakana('ｱﾞ')).toBe('ア゛');
    expect(toKatakana('ｰ｡')).toBe('ー。');
  });

  it('does not touch kanji, Latin letters, digits or symbols', () => {
    expect(toKatakana('東京 abc 123 !?')).toBe('東京 abc 123 !?');
    expect(toKatakana('今日はvtypeで')).toBe('今日ハvtypeデ');
  });
});

describe('applyReplacements', () => {
  it('returns the text unchanged for an empty table', () => {
    expect(applyReplacements('hello', [])).toBe('hello');
  });

  it('prefers the longest match', () => {
    const rules = [
      { from: 'ブイ', to: 'V' },
      { from: 'ブイタイプ', to: 'vtype' },
    ];
    expect(applyReplacements('ブイタイプとブイ', rules)).toBe('vtypeとV');
  });

  it('matches ASCII letters case-insensitively', () => {
    expect(applyReplacements('I use VType and vtype', [{ from: 'vtype', to: 'vtype' }])).toBe('I use vtype and vtype');
  });

  it('never overlaps: scanning resumes after the replaced span', () => {
    expect(applyReplacements('aaa', [{ from: 'aa', to: 'b' }])).toBe('ba');
    expect(applyReplacements('abc', [{ from: 'ab', to: 'X' }, { from: 'bc', to: 'Y' }])).toBe('Xc');
  });
});

describe('normalizeReplacementRules', () => {
  it('drops malformed items, empty from, and duplicates', () => {
    const raw = [
      { from: 'a', to: 'A' },
      { from: '', to: 'x' },
      { from: 'b' },
      'junk',
      null,
      { from: 'A', to: 'dup' },
      { from: 'c', to: '' },
    ];
    expect(normalizeReplacementRules(raw)).toEqual([
      { from: 'a', to: 'A' },
      { from: 'c', to: '' },
    ]);
    expect(normalizeReplacementRules('nope')).toEqual([]);
  });

  it('caps the table at 200 rules', () => {
    const raw = Array.from({ length: 250 }, (_, i) => ({ from: `w${i}`, to: `W${i}` }));
    const rules = normalizeReplacementRules(raw);
    expect(MAX_REPLACEMENT_RULES).toBe(200);
    expect(rules).toHaveLength(200);
    expect(rules.at(-1)).toEqual({ from: 'w199', to: 'W199' });
  });
});

describe('transformTranscript', () => {
  const rules = [{ from: 'ぶいたいぷ', to: 'vtype' }];

  it('leaves normal and en text alone except for replacements', () => {
    expect(transformTranscriptSync('ぶいたいぷです', { mode: 'normal', rules })).toBe('vtypeです');
    expect(transformTranscriptSync('hello', { mode: 'en', rules: [] })).toBe('hello');
  });

  it('keeps a replaced proper noun as registered in kana mode', () => {
    expect(transformTranscriptSync('ぶいたいぷをつかう', { mode: 'kana', rules })).toBe('vtypeヲツカウ');
  });

  it('keeps an English word registered in the table as English in kana mode', () => {
    const r = [{ from: 'vtype', to: 'vtype' }];
    expect(transformTranscriptSync('VTYPEでにゅうりょく', { mode: 'kana', rules: r })).toBe('vtypeデニュウリョク');
  });

  it('async without reading equals the sync version', async () => {
    expect(await transformTranscript('ぶいたいぷをつかう', { mode: 'kana', rules })).toBe('vtypeヲツカウ');
    expect(await transformTranscript('東京', { mode: 'normal', rules: [] })).toBe('東京');
  });

  it('runs replacements, then reading, then toKatakana, and never gives replaced spans to reading', async () => {
    const reading = vi.fn(async (s: string) => s.replace('東京', 'とうきょう').replace('行く', 'イク'));
    const out = await transformTranscript('ぶいたいぷで東京に行く', { mode: 'kana', rules, reading });
    expect(out).toBe('vtypeデトウキョウニイク');
    expect(reading).toHaveBeenCalledTimes(1);
    expect(reading).toHaveBeenCalledWith('で東京に行く');
  });

  it('does not call reading outside kana mode', async () => {
    const reading = vi.fn(async (s: string) => s);
    expect(await transformTranscript('東京', { mode: 'en', rules: [], reading })).toBe('東京');
    expect(reading).not.toHaveBeenCalled();
  });
});
