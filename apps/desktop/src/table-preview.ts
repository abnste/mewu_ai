// SPDX-License-Identifier: MPL-2.0
import { Marked, type Token, type Tokens } from 'marked';
import { decodeHTML } from 'entities';
import type { TableDocument, TableMessage } from './table-contracts';

const parser = new Marked({ gfm: true, breaks: true });
export function tableInlineText(tokens: Token[]): string {
  return tokens.map(token => {
    if (token.type === 'br') return '\n';
    if (token.type === 'html') return /^<br\s*\/?\s*>$/i.test(token.raw.trim()) ? '\n' : '';
    if (token.type === 'codespan') return token.text;
    if ('tokens' in token && Array.isArray(token.tokens)) return tableInlineText(token.tokens);
    // Marked resolves some numeric references in text but leaves named ones.
    // Decode the text token's original spelling once, never its partially decoded value.
    if (token.type === 'text') return decodeHTML(token.raw);
    if (token.type === 'escape') return token.text;
    if ('text' in token && typeof token.text === 'string') return decodeHTML(token.text);
    return '';
  }).join('');
}
const children = (token: Token): Token[] => token.type === 'list' ? token.items.flatMap((item: Tokens.ListItem) => item.tokens) : 'tokens' in token && Array.isArray(token.tokens) ? token.tokens : [];
const hasTable = (tokens: Token[]): boolean => tokens.some(token => token.type === 'table' || hasTable(children(token)));

// Only browser preview uses Marked's AST. Desktop displays the host's authoritative cells/ranges.
export function previewMessageTables(text: string, sourceHash: string): TableMessage {
  if (new TextEncoder().encode(text).length > 1024 * 1024) throw new Error('回答过大，无法提取表格');
  const tokens = parser.lexer(text), tables: TableDocument[] = [], blocks: TableMessage['blocks'] = [];
  let rows = 0, bytes = 0;
  function markdown(text: string) {
    if (!text) return;
    const previous = blocks.at(-1);
    if (previous?.kind === 'markdown') previous.text += text; else blocks.push({ kind: 'markdown', text });
  }
  function collect(tokens: Token[]) {
    for (const token of tokens) {
      if (token.type === 'table') {
        const header = token.header.map((cell: Tokens.TableCell) => tableInlineText(cell.tokens));
        const body = token.rows.map((row: Tokens.TableCell[]) => row.map(cell => tableInlineText(cell.tokens)));
        if ([...header, ...body.flat()].some(cell => /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/u.test(cell))) throw new Error('表格包含无效控制字符');
        rows += body.length + 1;
        bytes += new TextEncoder().encode([...header, ...body.flat()].join('')).length;
        if (tables.length >= 12 || rows > 200 || header.length > 32 || bytes > 256 * 1024) throw new Error('表格内容超出可导出范围');
        const index = tables.length;
        tables.push({ index, header, rows: body, align: token.align }); blocks.push({ kind: 'table', tableIndex: index });
      } else {
        const nested = children(token);
        if (nested.length && hasTable(nested)) collect(nested); else markdown(token.raw);
      }
    }
  }
  collect(tokens);
  return { sourceHash, blocks: tables.length ? blocks : [{ kind: 'markdown', text }], tables };
}
export function tablePlainText(table: TableDocument, format: 'markdown' | 'csv' | 'tsv') {
  const rows = [table.header, ...table.rows].map(row => table.header.map((_, index) => row[index] ?? ''));
  if (format === 'markdown') {
    const escape = (value: string) => [...value].map(char => {
      if (char === '\n') return '<br>';
      const code = char.charCodeAt(0);
      return code === 9 || code === 13 || code === 32 || (code >= 33 && code <= 47) || (code >= 58 && code <= 64) || (code >= 91 && code <= 96) || (code >= 123 && code <= 126) ? `&#${code};` : char;
    }).join('');
    const line = (row: string[]) => `| ${row.map(escape).join(' | ')} |`;
    const separator = table.header.map((_, index) => table.align[index] === 'center' ? ':---:' : table.align[index] === 'right' ? '---:' : table.align[index] === 'left' ? ':---' : '---');
    return [line(rows[0]), `| ${separator.join(' | ')} |`, ...rows.slice(1).map(line)].join('\n');
  }
  const separator = format === 'csv' ? ',' : '\t';
  return rows.map(row => row.map(value => `"${value.replaceAll('"', '""')}"`).join(separator)).join('\r\n');
}
