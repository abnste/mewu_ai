// SPDX-License-Identifier: MPL-2.0
import { createEffect, createSignal, createUniqueId, For, onCleanup, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { Download } from 'lucide-solid';
import type { ImageExportVariant } from '../contracts';
import { t } from '../i18n';
import './image-export-button.css';

export interface ImageExportButtonProps {
  scope: string; disabled?: boolean; compact?: boolean; label?: string; title?: string; layoutId?: string;
  onSelect: (variant: ImageExportVariant) => void;
  onOpenChange?: (open: boolean) => void;
  onRegister?: (open: () => void) => (() => void);
}

/** A single save trigger; the file picker opens only after an explicit image choice. */
export default function ImageExportButton(props: ImageExportButtonProps) {
  let trigger!: HTMLButtonElement, menu: HTMLDivElement | undefined, disposed = false, openedScope: string | undefined;
  const id = `image-export-${createUniqueId()}`, choices = ['annotated', 'original'] as const;
  const buttons: HTMLButtonElement[] = [];
  const [open, setOpen] = createSignal(false), [position, setPosition] = createSignal({ x: 0, y: 0 });
  const [focused, setFocused] = createSignal(0);
  const close = (restoreFocus = false) => {
    if (!open()) return;
    setOpen(false); openedScope = undefined; props.onOpenChange?.(false);
    if (restoreFocus && !disposed && trigger?.isConnected) trigger.focus({ preventScroll: true });
  };
  const place = () => {
    const box = trigger.getBoundingClientRect(), width = menu?.offsetWidth || 160, height = menu?.offsetHeight || 86;
    setPosition({ x: Math.max(8, Math.min(innerWidth - width - 8, box.left)), y: Math.max(8, box.bottom + height + 6 <= innerHeight - 8 ? box.bottom + 6 : box.top - height - 6) });
  };
  const focusItem = (index: number) => { const next = (index + choices.length) % choices.length; setFocused(next); buttons[next]?.focus({ preventScroll: true }); };
  const show = () => {
    if (disposed || props.disabled || !trigger?.isConnected) return;
    openedScope = props.scope; setFocused(0); setOpen(true); props.onOpenChange?.(true); place();
    queueMicrotask(() => { if (!disposed && open() && openedScope === props.scope) { place(); focusItem(0); } });
  };
  const choose = (variant: ImageExportVariant) => {
    if (!open() || props.disabled || openedScope !== props.scope) { close(); return; }
    close(true); props.onSelect(variant);
  };
  const key = (event: KeyboardEvent) => {
    if (!open() || event.isComposing || event.keyCode === 229) return;
    if (event.key === 'Tab') { close(); return; }
    event.stopImmediatePropagation();
    if (event.key === 'Escape') { event.preventDefault(); close(true); }
    else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); focusItem(focused() + (event.key === 'ArrowDown' ? 1 : -1)); }
    else if (event.key === 'Home' || event.key === 'End') { event.preventDefault(); focusItem(event.key === 'Home' ? 0 : 1); }
    else if ((event.key === 'Enter' || event.key === ' ') && !event.repeat) { event.preventDefault(); choose(choices[focused()]); }
  };
  const outside = (event: PointerEvent) => { if (!trigger.contains(event.target as Node) && !menu?.contains(event.target as Node)) close(); };
  createEffect(() => { if (open() && (props.disabled || props.scope !== openedScope)) close(); });
  createEffect(() => {
    if (!open()) return;
    const blurred = () => close();
    window.addEventListener('keydown', key, true); window.addEventListener('pointerdown', outside, true); window.addEventListener('blur', blurred); window.addEventListener('resize', blurred);
    onCleanup(() => { window.removeEventListener('keydown', key, true); window.removeEventListener('pointerdown', outside, true); window.removeEventListener('blur', blurred); window.removeEventListener('resize', blurred); });
  });
  const unregister = props.onRegister?.(show);
  onCleanup(() => { disposed = true; close(); unregister?.(); });
  return <>
    <button data-layout-id={props.layoutId} ref={trigger} class="image-export-trigger" classList={{ 'icon-button': props.compact, compact: props.compact }} data-caption={props.compact ? undefined : t('保存')} title={props.title ?? t('保存 · S')} aria-label={props.label ?? t('保存图片')} aria-haspopup="menu" aria-expanded={open()} aria-controls={open() ? id : undefined} disabled={props.disabled} onPointerDown={event => event.stopPropagation()} onClick={event => { event.stopPropagation(); if (open()) close(); else show(); }} onKeyDown={event => { if (!open() && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) { event.preventDefault(); event.stopPropagation(); show(); if (event.key === 'ArrowUp') queueMicrotask(() => { if (open()) focusItem(1); }); } }}><Download size={props.compact ? 15 : 20} /></button>
    <Show when={open()}><Portal><div ref={menu} id={id} class="image-export-menu" role="menu" aria-label={t('保存图片')} style={{ left: `${position().x}px`, top: `${position().y}px` }} onPointerDown={event => event.stopPropagation()}>
      <For each={choices}>{(variant, index) => <button ref={element => { buttons[index()] = element; }} type="button" role="menuitem" tabIndex={focused() === index() ? 0 : -1} onFocus={() => setFocused(index())} onClick={() => choose(variant)}>{t(variant === 'annotated' ? '带标注图' : '原图')}</button>}</For>
    </div></Portal></Show>
  </>;
}
