// SPDX-License-Identifier: MPL-2.0
/** Tracks only public reasoning text; provider continuation blobs never enter here. */
export class ReasoningDisclosure {
  private identity: string | undefined;
  private running = false;
  private seen = false;
  private expanded = false;
  update(identity: string, running: boolean, hasText: boolean): boolean {
    if (identity !== this.identity) { this.identity = identity; this.running = running; this.seen = false; this.expanded = false; }
    if (!this.seen && hasText) { this.seen = true; this.expanded = running; }
    if (this.running && !running) this.expanded = false;
    this.running = running;
    return this.expanded;
  }
  toggle() { this.expanded = !this.expanded; return this.expanded; }
}
export function reasoningDisplayText(text: string): string {
  return text;
}

/** Only leading provider envelopes are control tags. Answer prose and code are literal. */
export function splitReplyReasoning(source: string, publicReasoning = '', streaming = false): { text: string; reasoning: string } {
  const thoughts: string[] = [];
  const opening = /<(think|thinking|reasoning)>/iy;
  const tags = ['<think>', '<thinking>', '<reasoning>'];
  let offset = 0, text = '';
  while (offset < source.length) {
    const start = offset;
    while (offset < source.length && /[\s\u200b]/u.test(source[offset])) offset++;
    opening.lastIndex = offset;
    const match = opening.exec(source);
    if (!match) {
      const tail = source.slice(offset).toLowerCase();
      text = streaming && tail.length > 0 && tags.some(tag => tag.startsWith(tail)) ? '' : source.slice(start);
      break;
    }
    offset = opening.lastIndex;
    const closeTag = `</${match[1].toLowerCase()}>`;
    const closing = new RegExp(closeTag, 'ig');
    closing.lastIndex = offset;
    const end = closing.exec(source);
    if (!end) {
      let tail = source.slice(offset);
      if (streaming) {
        for (let size = Math.min(closeTag.length - 1, tail.length); size > 0; size--) {
          if (closeTag.startsWith(tail.slice(-size).toLowerCase())) { tail = tail.slice(0, -size); break; }
        }
      }
      thoughts.push(tail);
      break;
    }
    thoughts.push(source.slice(offset, end.index));
    offset = closing.lastIndex;
  }
  const inline = thoughts.join('\n\n');
  const reasoning = inline && publicReasoning && inline.trim() !== publicReasoning.trim() ? `${publicReasoning}\n\n${inline}` : publicReasoning || inline;
  return { text, reasoning };
}
