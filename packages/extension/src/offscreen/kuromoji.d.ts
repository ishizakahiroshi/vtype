// kuromoji ships no types. Only the tokenizer builder is used (src/offscreen/reading.ts).
declare module "kuromoji/src/TokenizerBuilder.js" {
  export default class TokenizerBuilder {
    constructor(options: { dicPath: string });
    build(callback: (err: unknown, tokenizer: never) => void): void;
  }
}
