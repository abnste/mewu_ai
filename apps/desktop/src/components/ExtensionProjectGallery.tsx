// SPDX-License-Identifier: MPL-2.0
import { createEffect, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { ArrowLeft, ArrowRight, ExternalLink, Image, LoaderCircle, X } from 'lucide-solid';
import * as api from '../extension-bridge';
import type { ExtensionCatalogEntry } from '../extension-contracts';
import { createExtensionProjectState, projectRepositoryUrl } from '../extension-project-state';
import { nativeReplyLinks, openReplyLink } from '../reply-content-bridge';
import { t } from '../i18n';

interface Props { active: boolean; entry?: ExtensionCatalogEntry; name: string }
export default function ExtensionProjectGallery(props: Props) {
  const state = createExtensionProjectState(api);
  const [viewer, setViewer] = createSignal(false), [linkError, setLinkError] = createSignal(false);
  const screenshot = () => state.project()?.screenshots[state.selected()];
  const image = () => state.images()[state.selected()];
  createEffect(() => { if (state.setEntry(props.active && api.extensionNative ? props.entry : undefined)) { setViewer(false); setLinkError(false); } });
  onCleanup(() => state.dispose());
  const repository = () => state.project()
    ? `${projectRepositoryUrl(state.project()!.sourceRepository)}/tree/${state.project()!.sourceCommit}`
    : undefined;
  const release = () => props.entry && projectRepositoryUrl(props.entry.repository)
    ? `${projectRepositoryUrl(props.entry.repository)}/releases/tag/${encodeURIComponent(props.entry.releaseTag)}` : undefined;
  async function openLink(event: MouseEvent, url: string) {
    if (!nativeReplyLinks()) return;
    event.preventDefault();
    try { await openReplyLink(url); } catch { setLinkError(true); }
  }
  function observeThumbnail(element: HTMLButtonElement, index: number) {
    if (typeof IntersectionObserver !== 'function') { state.loadImage(index); return; }
    const observer = new IntersectionObserver(values => {
      if (values.some(value => value.isIntersecting)) { observer.disconnect(); state.loadImage(index); }
    });
    observer.observe(element); onCleanup(() => observer.disconnect());
  }
  function ImageViewer() {
    let close!: HTMLButtonElement, dialog!: HTMLDivElement;
    const previous = document.activeElement;
    onMount(() => { document.addEventListener('keydown', keyboard, { capture: true }); close.focus(); });
    onCleanup(() => { document.removeEventListener('keydown', keyboard, { capture: true }); if (previous instanceof HTMLElement && previous.isConnected) previous.focus(); });
    const keyboard = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setViewer(false); }
      if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); event.stopPropagation(); state.select(state.selected() + (event.key === 'ArrowLeft' ? -1 : 1)); if (!dialog.contains(document.activeElement)) close.focus(); }
      if (event.key === 'Tab') {
        const buttons = [...dialog.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
        if (!dialog.contains(document.activeElement)) { event.preventDefault(); event.stopPropagation(); close.focus(); return; }
        if (event.shiftKey && document.activeElement === buttons[0]) { event.preventDefault(); buttons.at(-1)?.focus(); }
        else if (!event.shiftKey && document.activeElement === buttons.at(-1)) { event.preventDefault(); buttons[0]?.focus(); }
      }
    };
    return <Portal><div class="plugin-project-viewer" role="dialog" aria-modal="true" aria-label={props.name} data-settings-inner-dialog ref={dialog} onPointerDown={event => { if (event.target === event.currentTarget) setViewer(false); }}>
      <div class="plugin-project-viewer-actions"><span>{screenshot()?.caption}</span><button class="icon-button" disabled={state.selected() === 0} aria-label={t('上一张')} onClick={() => state.select(state.selected() - 1)}><ArrowLeft size={17}/></button><button class="icon-button" disabled={state.selected() + 1 >= (state.project()?.screenshots.length ?? 0)} aria-label={t('下一张')} onClick={() => state.select(state.selected() + 1)}><ArrowRight size={17}/></button><button ref={close} class="icon-button" aria-label={t('关闭')} onClick={() => setViewer(false)}><X size={18}/></button></div>
      <Show when={image()} fallback={<Show when={state.failedImages().has(state.selected())} fallback={<LoaderCircle class="spin" size={18}/>}><div class="plugin-project-error" role="alert"><span>{t('截图加载失败')}</span><button class="text-button" onClick={() => state.retry()}>{t('重试')}</button></div></Show>}><img src={image()} alt={screenshot()?.caption || props.name}/></Show>
    </div></Portal>;
  }
  return <>
    <div class="plugin-project-links"><Show when={repository()}>{url => <a href={url()} target="_blank" rel="noreferrer noopener" onClick={event => void openLink(event, url())}>{t('项目')}<ExternalLink size={12}/></a>}</Show><Show when={release()}>{url => <a href={url()} target="_blank" rel="noreferrer noopener" onClick={event => void openLink(event, url())}>{t('发行')}<ExternalLink size={12}/></a>}</Show><Show when={state.loading()}><LoaderCircle size={12} class="spin"/></Show></div>
    <Show when={linkError()}><p class="plugin-error" role="alert">{t('链接不可用')}</p></Show>
    <Show when={state.projectError() || state.failedImages().has(state.selected())}><div class="plugin-project-error" role="alert"><span>{t(state.projectError() ? '项目信息加载失败' : '截图加载失败')}</span><button class="text-button" onClick={() => state.retry()}>{t('重试')}</button></div></Show>
    <Show when={state.project()?.screenshots.length}><section class="plugin-project-gallery" aria-label={t('截图')}>
      <button class="plugin-project-main" disabled={!image()} aria-label={t('查看大图')} onClick={() => setViewer(true)}><Show when={image()} fallback={<LoaderCircle size={18} class="spin"/>}><img src={image()} alt={screenshot()?.caption || props.name}/></Show></button>
      <Show when={screenshot()?.caption}><p class="plugin-project-caption">{screenshot()?.caption}</p></Show>
      <Show when={(state.project()?.screenshots.length ?? 0) > 1}><div class="plugin-project-thumbnails"><For each={state.project()?.screenshots}>{(shot, index) => <button ref={element => observeThumbnail(element, index())} aria-label={shot.caption || `${index() + 1}`} aria-pressed={state.selected() === index()} onClick={() => state.select(index())}><Show when={state.images()[index()]} fallback={<Image size={15}/>}><img src={state.images()[index()]} alt="" loading="lazy"/></Show></button>}</For></div></Show>
    </section></Show>
    <Show when={viewer()}><ImageViewer/></Show>
  </>;
}
