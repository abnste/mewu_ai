// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, onCleanup, Show } from 'solid-js';
import DOMPurify from 'dompurify';
import { formulaAttributes, formulaCopyText, formulaTags, renderReplyFormula, replyTags, type ReplyFormula } from '../reply-markdown';
import { createReplyBlockPatcher, replyContent, replyRasterData, replyWebLink } from '../reply-content';
import { nativeReplyLinks, openReplyLink } from '../reply-content-bridge';
import { t } from '../i18n';
import 'katex/dist/katex.min.css';
import './reply-math.css';
import './reply-content.css';

function safeFormula(value: ReplyFormula): string | undefined {
  const rendered = renderReplyFormula(value);
  return rendered ? DOMPurify.sanitize(rendered, { ALLOWED_TAGS: formulaTags, ALLOWED_ATTR: formulaAttributes, ALLOW_DATA_ATTR: false }) : undefined;
}

// Only generated KaTeX reaches innerHTML. Original table text stays a Solid text
// node on failure and the exact host spelling is retained for selection-copy.
export function ReplyFormulaView(props: { formula: ReplyFormula; text: string }) {
  const rendered = createMemo(() => safeFormula(props.formula));
  return <Show when={rendered()} fallback={props.text}>{html => <span class={props.formula.display ? 'reply-formula reply-formula-display' : 'reply-formula'} data-formula-source={props.text} innerHTML={html()} />}</Show>;
}

export default function Markdown(props: { text: string }) {
  let element!: HTMLDivElement;
  const patch = createReplyBlockPatcher(), timers = new Set<ReturnType<typeof setTimeout>>();
  const descendants = (root: Node, selector: string) => root instanceof HTMLElement ? [...(root.matches(selector) ? [root] : []), ...root.querySelectorAll<HTMLElement>(selector)] : [];
  onCleanup(() => { for (const timer of timers) clearTimeout(timer); timers.clear(); });
  createEffect(() => {
    const result = replyContent(props.text), template = document.createElement('template');
    template.innerHTML = DOMPurify.sanitize(result.html, { ALLOWED_TAGS: [...replyTags, 'h5', 'h6', 'span'], ALLOWED_ATTR: ['title', 'href', 'target', 'rel', 'class', 'start', 'align', 'data-mewu-formula', 'data-mewu-image'], ALLOW_DATA_ATTR: false });
    // No image src exists in this HTML. Public external images remain links;
    // the only displayed images are bounded, validated embedded raster data.
    const added = patch(element, template.content, node => (node instanceof Element ? node.outerHTML : node.textContent ?? '') + JSON.stringify(descendants(node, '[data-mewu-image]').map(marker => result.images[Number(marker.dataset.mewuImage)])));
    for (const node of added.flatMap(root => descendants(root, '[data-mewu-formula]'))) {
      const value = result.formulas[Number(node.dataset.mewuFormula)]; node.removeAttribute('data-mewu-formula');
      if (!value) continue;
      if (node.closest('pre,code')) { node.textContent = value.source; continue; }
      node.className = value.display ? 'reply-formula reply-formula-display' : 'reply-formula';
      node.dataset.formulaSource = value.source;
      const rendered = safeFormula(value);
      if (rendered) node.innerHTML = rendered;
      else node.textContent = value.source;
    }
    for (const node of added.flatMap(root => descendants(root, '[data-mewu-image]'))) {
      const image = result.images[Number(node.dataset.mewuImage)]; node.removeAttribute('data-mewu-image');
      if (!image) continue;
      node.className = 'reply-image-slot';
      const data = replyRasterData(image.source), href = replyWebLink(image.source);
      if (data) {
        const img = document.createElement('img'); img.src = data; img.alt = image.description; img.loading = 'lazy'; img.dataset.replyImageDescription = image.description || t('图片');
        img.addEventListener('error', () => { if (img.parentNode === node) { node.replaceChildren(document.createTextNode(image.description || t('图片'))); node.title = t('图片不可用'); } }, { once: true });
        node.replaceChildren(img);
      }
      else if (href) { const link = document.createElement('a'); link.href = href; link.target = '_blank'; link.rel = 'noopener noreferrer'; link.textContent = image.description || t('图片'); if (!image.description) link.dataset.replyImageDefault = ''; node.replaceChildren(link); }
      else { node.textContent = image.description || t('图片'); node.title = t('图片不可用'); if (!image.description) node.dataset.replyImageDefault = ''; }
    }
    for (const pre of added.flatMap(root => descendants(root, 'pre'))) {
      const code = pre.querySelector('code'); if (!code) continue;
      const language = [...code.classList].find(value => /^language-[a-z0-9_+.-]{1,32}$/i.test(value))?.slice(9) || '';
      pre.classList.add('reply-code-block');
      const header = document.createElement('div'), label = document.createElement('span'), copy = document.createElement('button');
      header.className = 'reply-code-heading'; header.dataset.replyCopyExclude = ''; label.textContent = language;
      copy.type = 'button'; copy.className = 'reply-code-copy'; copy.dataset.replyCodeCopy = ''; copy.textContent = t('复制'); copy.setAttribute('aria-label', t('复制代码')); copy.title = t('复制代码');
      header.append(label, copy); pre.prepend(header);
    }
  });
  createEffect(() => {
    const copy = t('复制'), label = t('复制代码'), image = t('图片');
    for (const button of element.querySelectorAll<HTMLElement>('[data-reply-code-copy]')) { button.textContent = copy; button.title = label; button.setAttribute('aria-label', label); }
    for (const node of element.querySelectorAll<HTMLElement>('[data-reply-image-default]')) node.textContent = image;
  });
  // Interactive HTML/SVG still use the isolated content host, never this view.
  return <div ref={element} class="message-markdown" onClick={event => {
    const target = event.target instanceof Element ? event.target : undefined;
    const link = target?.closest<HTMLAnchorElement>('a[href]');
    if (link && element.contains(link) && nativeReplyLinks()) { event.preventDefault(); void openReplyLink(link.href).catch(() => { link.title = t('链接不可用'); }); return; }
    const button = target?.closest<HTMLButtonElement>('[data-reply-code-copy]');
    if (!button || !element.contains(button)) return;
    event.preventDefault(); event.stopPropagation();
    const code = button.closest('pre')?.querySelector('code'); if (!code) return;
    void (navigator.clipboard ? navigator.clipboard.writeText(code.textContent ?? '') : Promise.reject(new Error('Clipboard unavailable'))).then(() => {
      if (!button.isConnected) return; button.textContent = t('已复制');
      const timer = setTimeout(() => { timers.delete(timer); if (button.isConnected) button.textContent = t('复制'); }, 1200); timers.add(timer);
    }, () => { if (button.isConnected) button.title = t('复制失败'); });
  }} onCopy={event => { const selection = window.getSelection(); if (!selection?.rangeCount || selection.isCollapsed || !event.clipboardData) return; const text = formulaCopyText(element, selection.getRangeAt(0)); if (text === undefined) return; event.preventDefault(); event.clipboardData.setData('text/plain', text); }} />;
}
