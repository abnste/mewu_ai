// SPDX-License-Identifier: MPL-2.0
import { Marked, type Token, type Tokens } from 'marked';
import { lexReplyInline, type ReplyFormula, type ReplyMathToken } from './reply-markdown';
import { tableInlineText } from './table-preview';
import type { TableDocument } from './table-contracts';

export interface ReplyCellPart { text: string; formula?: ReplyFormula }
export type ReplyTablePresentation = ReplyCellPart[][][];
const parser = new Marked({ gfm: true, breaks: true });
const bytes = (text: string) => new TextEncoder().encode(text).length;
const children = (token: Token): Token[] => token.type === 'list' ? token.items.flatMap((item: Tokens.ListItem) => item.tokens) : 'tokens' in token && Array.isArray(token.tokens) ? token.tokens : [];
const identity = (header: string[], rows: string[][], align: TableDocument['align']) => JSON.stringify([header, rows, align]);

// The host's table and its export index stay authoritative. Source AST is only
// evidence for formula styling: an entire table must match all cells/alignment.
// Identical plain tables with different code/math meanings are ambiguous and
// stay literal; no frontend index is used to guess their native identity.
export function replyTablePresentation(source: string, table: TableDocument): ReplyTablePresentation | undefined {
  if (source.length > 1024 * 1024 || bytes(source) > 1024 * 1024) return;
  const expected = identity(table.header, table.rows, table.align);
  const matches: ReplyTablePresentation[] = [];
  let tables = 0, rows = 0, cellBytes = 0, formulas = 0;
  function parts(tokens: Token[]): ReplyCellPart[] {
    return tokens.flatMap(token => {
      if (token.type === 'replyMathInline') {
        const formula = (token as ReplyMathToken).formula;
        const text = tableInlineText(parser.Lexer.lexInline(token.raw, parser.defaults));
        return [{ text, ...(formula && formulas++ < 128 ? { formula } : {}) }];
      }
      // An image's alt text is data, not a formula presentation surface.
      if (token.type !== 'image' && 'tokens' in token && Array.isArray(token.tokens)) return parts(token.tokens);
      return [{ text: tableInlineText([token]) }];
    });
  }
  function visit(tokens: Token[]) {
    for (const token of tokens) {
      if (token.type !== 'table') { visit(children(token)); continue; }
      const cells = [token.header, ...token.rows] as Tokens.TableCell[][];
      const plain = cells.map(row => row.map(cell => tableInlineText(cell.tokens)));
      tables++; rows += cells.length; cellBytes += bytes(plain.flat().join(''));
      if (tables > 12 || rows > 200 || token.header.length > 32 || cellBytes > 256 * 1024) throw new Error('presentation limit');
      const display = cells.map((row, r) => row.map((cell, c) => {
        if (!/\$|\\[([]/.test(cell.text)) return [{ text: plain[r][c] }];
        const value = parts(lexReplyInline(cell.text));
        // Parser differences, escape handling or unusual inline constructs must
        // never change the host's text. They fall back to that exact native cell.
        return value.map(part => part.text).join('') === plain[r][c] ? value : [{ text: plain[r][c] }];
      }));
      if (identity(plain[0], plain.slice(1), token.align) === expected) matches.push(display);
    }
  }
  try { visit(parser.lexer(source)); } catch { return; }
  if (!matches.length) return;
  const first = JSON.stringify(matches[0]);
  if (matches.some(value => JSON.stringify(value) !== first)) return;
  return matches[0];
}
