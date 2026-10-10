// SPDX-License-Identifier: MPL-2.0
import { decodeHTML } from 'entities';
import { escapeReplyText, parseReplyMarkdown } from './reply-markdown';

export interface ReplyImage { source: string; description: string }
const MAX_IMAGE_BYTES = 8 * 1024 * 1024;
const MAX_IMAGES = 16;

export function replyWebLink(value: string): string | undefined {
  if (!value || value.length > 8192 || /[\u0000-\u0020\u007f]/.test(value)) return;
  try {
    const url = new URL(value);
    if ((url.protocol === 'https:' || url.protocol === 'http:') && !url.username && !url.password) return url.href;
  } catch { /* Malformed and relative paths stay plain text. */ }
}

/** Only embedded raster data is displayed; external URLs never become img.src. */
export function replyRasterData(value: string): string | undefined {
  if (value.length > Math.ceil(MAX_IMAGE_BYTES / 3) * 4 + 40) return;
  const match = /^data:image\/(png|jpeg|gif|webp);base64,([A-Za-z0-9+/]+={0,2})$/i.exec(value);
  if (!match || match[2].length % 4 !== 0 || (match[2].length / 4 * 3 - (match[2].endsWith('==') ? 2 : match[2].endsWith('=') ? 1 : 0)) > MAX_IMAGE_BYTES) return;
  try {
    const head = atob(match[2].slice(0, 32));
    const kind = match[1].toLowerCase();
    const valid = kind === 'png' ? head.startsWith('\x89PNG\r\n\x1a\n') : kind === 'jpeg' ? head.startsWith('\xff\xd8\xff') : kind === 'gif' ? /^GIF8[79]a/.test(head) : head.startsWith('RIFF') && head.slice(8, 12) === 'WEBP';
    if (valid) return value;
  } catch { /* Invalid base64 stays a description. */ }
}

export function replyContent(text: string) {
  const images: ReplyImage[] = [];
  const parsed = parseReplyMarkdown(text, escapeReplyText, {
    link(token) {
      const body = this.parser.parseInline(token.tokens), href = replyWebLink(token.href);
      return href ? `<a href="${escapeReplyText(href)}" target="_blank" rel="noopener noreferrer"${token.title ? ` title="${escapeReplyText(token.title)}"` : ''}>${body}</a>` : body;
    },
    image(token) {
      const description = decodeHTML(token.text);
      if (images.length >= MAX_IMAGES) return escapeReplyText(description);
      const index = images.push({ source: token.href, description }) - 1;
      return `<span data-mewu-image="${index}">${escapeReplyText(description)}</span>`;
    },
  });
  return { ...parsed, images };
}

/** Match the final rendered blocks, so reference-definition changes also update. */
export function createReplyBlockPatcher() {
  const signatures = new WeakMap<Node, string>();
  return (container: HTMLElement, fragment: DocumentFragment, signature: (node: Node) => string): Node[] => {
    const before = [...container.childNodes], after = [...fragment.childNodes], keys = after.map(signature);
    let prefix = 0, suffix = 0;
    while (prefix < before.length && prefix < after.length && signatures.get(before[prefix]) === keys[prefix]) prefix++;
    while (suffix < before.length - prefix && suffix < after.length - prefix && signatures.get(before[before.length - suffix - 1]) === keys[after.length - suffix - 1]) suffix++;
    const anchor = suffix ? before[before.length - suffix] : null;
    for (let index = prefix; index < before.length - suffix; index++) before[index].remove();
    const added = after.slice(prefix, after.length - suffix);
    for (let index = prefix; index < after.length - suffix; index++) {
      signatures.set(after[index], keys[index]); container.insertBefore(after[index], anchor);
    }
    return added;
  };
}
