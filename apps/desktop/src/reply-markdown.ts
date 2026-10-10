// SPDX-License-Identifier: MPL-2.0
import { Marked, type RendererObject, type Token, type Tokens } from 'marked';
import katex from 'katex';

export interface ReplyFormula { source: string; tex: string; display: boolean }
export const MAX_FORMULA_LENGTH = 2048;
const MAX_FORMULAS = 128;
export const replyTags = ['p', 'br', 'strong', 'em', 'del', 'blockquote', 'pre', 'code', 'ul', 'ol', 'li', 'h1', 'h2', 'h3', 'h4', 'hr', 'table', 'thead', 'tbody', 'tr', 'th', 'td', 'a'];
export const formulaTags = ['span', 'math', 'semantics', 'annotation', 'mrow', 'mi', 'mo', 'mn', 'mspace', 'mtext', 'mfrac', 'msqrt', 'mroot', 'msup', 'msub', 'msubsup', 'mover', 'munder', 'munderover', 'mtable', 'mtr', 'mtd', 'menclose', 'mstyle', 'mpadded', 'mphantom', 'mmultiscripts', 'mprescripts', 'none', 'svg', 'path', 'line', 'rect', 'g'];
export const formulaAttributes = ['class', 'style', 'aria-hidden', 'xmlns', 'encoding', 'display', 'mathvariant', 'scriptlevel', 'displaystyle', 'stretchy', 'fence', 'separator', 'accent', 'accentunder', 'lspace', 'rspace', 'minsize', 'maxsize', 'width', 'height', 'depth', 'voffset', 'columnalign', 'columnspacing', 'rowspacing', 'rowalign', 'linethickness', 'notation', 'viewBox', 'preserveAspectRatio', 'd', 'x', 'y', 'x1', 'x2', 'y1', 'y2', 'fill', 'stroke', 'stroke-width'];
export const escapeReplyText = (value: string) => value.replace(/[&<>"']/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char]!);

export interface ReplyMathToken extends Tokens.Generic { type: string; raw: string; formula?: ReplyFormula }
function escaped(text: string, index: number) { let count = 0; while (index > 0 && text[--index] === '\\') count++; return count % 2 === 1; }

function mathToken(source: string, block: boolean): ReplyMathToken | undefined {
  const offset = block ? source.match(/^ {0,3}/)![0].length : 0;
  const text = source.slice(offset);
  const opening = text.startsWith('$$') ? '$$' : text.startsWith('\\[') ? '\\[' : !block && text.startsWith('\\(') ? '\\(' : !block && text[0] === '$' ? '$' : undefined;
  if (!opening) return;
  const display = opening === '$$' || opening === '\\[';
  const closing = opening === '\\[' ? '\\]' : opening === '\\(' ? '\\)' : opening;
  const single = opening === '$';
  if (single && (!text[1] || /\s|\$/.test(text[1]))) return;
  const limit = Math.min(text.length, MAX_FORMULA_LENGTH + opening.length + closing.length);
  for (let end = opening.length; end < limit; end++) {
    if (single && /[\r\n]/.test(text[end])) return;
    // Do not consume a later dollar from another Markdown construct. In
    // particular, a price followed by a code span must leave that span to Marked.
    if (single && !escaped(text, end) && (text[end] === '`' || /^<(?:[A-Za-z/!]|https?:\/\/)/.test(text.slice(end)) || /^!?\[[^\]\n]*\]\(/.test(text.slice(end)))) return;
    if (!text.startsWith(closing, end) || escaped(text, end)) continue;
    // Pandoc-style dollar boundaries avoid amounts such as "$5 and $10".
    if (single && (/\s/.test(text[end - 1]) || /[\d$]/.test(text[end + 1] ?? ''))) return;
    const content = text.slice(opening.length, end);
    if (!content.trim()) return;
    const length = offset + end + closing.length;
    if (block && !/^[ \t]*(?:\r?\n|$)/.test(source.slice(length))) return;
    const raw = source.slice(0, length);
    return { type: block ? 'replyMathBlock' : 'replyMathInline', raw, formula: { source: raw, tex: content, display } };
  }
  // Preserve bracket delimiters in a partial streaming reply. Marked's normal
  // escape tokenizer would otherwise remove their backslashes.
  if (!block && opening.startsWith('\\')) return { type: 'replyMathInline', raw: text.slice(0, text.indexOf('\n') < 0 ? text.length : text.indexOf('\n')) };
}

export function parseReplyMarkdown(text: string, sanitizeRawHtml: (html: string) => string, renderer: RendererObject = {}) {
  const formulas: ReplyFormula[] = [];
  const renderMath = (token: Tokens.Generic) => {
    const value = token as ReplyMathToken;
    if (!value.formula || formulas.length >= MAX_FORMULAS) return escapeReplyText(value.raw);
    const index = formulas.push(value.formula) - 1;
    return `<span data-mewu-formula="${index}">${escapeReplyText(value.raw)}</span>${value.type === 'replyMathBlock' ? '\n' : ''}`;
  };
  return { html: replyParser(renderMath, sanitizeRawHtml, renderer).parse(text) as string, formulas };
}

function replyParser(renderMath: (token: Tokens.Generic) => string, sanitizeRawHtml: (html: string) => string, renderer: RendererObject = {}) {
  // This instance cannot change table parsing or another request's tokenizer.
  return new Marked({ async: false, breaks: true, renderer: { ...renderer, html: token => sanitizeRawHtml(token.text) }, extensions: [
    { name: 'replyMathBlock', level: 'block', start: source => { const match = /(?:^|\n) {0,3}(?:\$\$|\\\[)/.exec(source); return match?.index; }, tokenizer: source => mathToken(source, true), renderer: renderMath },
    { name: 'replyMathInline', level: 'inline', start: source => { const match = /\$|\\[([]/.exec(source); return match?.index; }, tokenizer(source) { if (!this.lexer.state.inRawBlock) return mathToken(source, false); }, renderer: renderMath },
    // 0.7.3 vision replies sometimes contain a second escaped newline before
    // numbered/list items. This is an inline extension inside Marked, so code,
    // raw HTML and recognized math never receive global string substitutions.
    { name: 'replyProviderBreak', level: 'inline', start: source => /\\n(?=\s*(?:\(?\d+[.)]|[（(]|[-•*]))/.exec(source)?.index,
      tokenizer(source) { if (!this.lexer.state.inRawBlock && /^\\n(?=\s*(?:\(?\d+[.)]|[（(]|[-•*]))/.test(source)) return { type: 'replyProviderBreak', raw: '\\n', text: '\n' }; }, renderer: () => '<br>' },
  ] });
}

// Presentation-only tokens for a source cell. Code/raw-code and dollar boundary
// rules are the same as ordinary replies; this never parses a flattened host cell.
export function lexReplyInline(source: string): Token[] {
  const parser = replyParser(token => escapeReplyText(token.raw), escapeReplyText);
  return parser.Lexer.lexInline(source, parser.defaults);
}

export function renderReplyFormula(value: ReplyFormula): string | undefined {
  if (!value.tex.trim() || value.tex.length > MAX_FORMULA_LENGTH) return;
  try {
    // Original 0.7.3 accepts these provider/Markdown display escapes. Only the
    // recognized formula is adapted; source/copy, code and aligned row breaks
    // retain their exact spelling. KaTeX already supports the original geometry,
    // shorthand fraction and binomial commands without degrading their meaning.
    const tex = value.tex.replaceAll('\\_', '_').replaceAll('\\\\,', '\\,');
    return katex.renderToString(tex, { displayMode: value.display, output: 'htmlAndMathml', trust: false, throwOnError: true, strict: 'error', maxExpand: 200, maxSize: 20, macros: {}, globalGroup: false });
  } catch { return; } // Original TeX remains visible and copyable; errors are not HTML.
}

const blockTags = new Set(['P', 'DIV', 'PRE', 'BLOCKQUOTE', 'LI', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'TR']);
function selectedText(node: Node): string {
  if (node.nodeType === Node.TEXT_NODE) return node.textContent ?? '';
  if (node instanceof Element && node.tagName === 'BR') return '\n';
  // A native table selection must keep its column boundaries when replacing
  // KaTeX's duplicated visual/MathML text with the authoritative cell text.
  if (node instanceof Element && node.tagName === 'TR') return [...node.childNodes].filter(child => child instanceof Element && (child.tagName === 'TD' || child.tagName === 'TH')).map(selectedText).join('\t') + '\n';
  // A range within a single row clones its TD/TH children without their TR.
  const children = [...node.childNodes];
  if (children.length && children.every(child => child instanceof Element && (child.tagName === 'TD' || child.tagName === 'TH'))) return children.map(selectedText).join('\t');
  const text = children.map(selectedText).join('');
  return text + (node instanceof Element && blockTags.has(node.tagName) ? '\n' : '');
}
export function formulaCopyText(container: HTMLElement, range: Range): string | undefined {
  if (!container.contains(range.commonAncestorContainer)) return;
  const formulas = [...container.querySelectorAll<HTMLElement>('[data-formula-source]')].filter(node => range.intersectsNode(node));
  const controls = [...container.querySelectorAll<HTMLElement>('[data-reply-copy-exclude]')].filter(node => range.intersectsNode(node));
  const images = [...container.querySelectorAll<HTMLElement>('[data-reply-image-description]')].filter(node => range.intersectsNode(node));
  if (!formulas.length && !controls.length && !images.length) return;
  const copy = range.cloneRange();
  for (const node of formulas) {
    if (node.contains(copy.startContainer)) copy.setStartBefore(node);
    if (node.contains(copy.endContainer)) copy.setEndAfter(node);
  }
  const content = copy.cloneContents();
  for (const node of content.querySelectorAll<HTMLElement>('[data-reply-copy-exclude]')) node.remove();
  for (const node of content.querySelectorAll<HTMLElement>('[data-reply-image-description]')) node.replaceWith(document.createTextNode(node.dataset.replyImageDescription ?? ''));
  for (const node of content.querySelectorAll<HTMLElement>('[data-formula-source]')) node.replaceWith(document.createTextNode(node.dataset.formulaSource ?? ''));
  return selectedText(content).replace(/\n$/, '');
}
