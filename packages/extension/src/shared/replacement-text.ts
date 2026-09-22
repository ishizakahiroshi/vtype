// The replacement table as text: `認識結果 => 入れたい形`, one rule per line. Shared by the
// extension's options page and the desktop app's settings page (standalone plan C5).

import { MAX_REPLACEMENT_RULES, type ReplacementRule } from "vtype-core";

export const REPLACEMENT_ARROW = "=>";

export interface ParsedReplacements {
  readonly rules: ReplacementRule[];
  /** 1-based numbers of the lines that could not be read. Blank lines are not counted. */
  readonly badLines: number[];
  /** More rules were written than MAX_REPLACEMENT_RULES; the rest were dropped. */
  readonly truncated: boolean;
}

/** `認識結果 => 入れたい形`, one per line. Spaces around the arrow are ignored. */
export function parseReplacementText(text: string): ParsedReplacements {
  const rules: ReplacementRule[] = [];
  const badLines: number[] = [];
  let truncated = false;
  text.split(/\r?\n/).forEach((line, index) => {
    if (line.trim() === "") return;
    const at = line.indexOf(REPLACEMENT_ARROW);
    const from = at < 0 ? "" : line.slice(0, at).trim();
    if (from === "") {
      badLines.push(index + 1);
      return;
    }
    if (rules.some((r) => r.from.toLowerCase() === from.toLowerCase())) return;
    if (rules.length >= MAX_REPLACEMENT_RULES) {
      truncated = true;
      return;
    }
    rules.push({ from, to: line.slice(at + REPLACEMENT_ARROW.length).trim() });
  });
  return { rules, badLines, truncated };
}

export function formatReplacementText(rules: readonly ReplacementRule[]): string {
  return rules.map((r) => `${r.from} ${REPLACEMENT_ARROW} ${r.to}`).join("\n");
}
