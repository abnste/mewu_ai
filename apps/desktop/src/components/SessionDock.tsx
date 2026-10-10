// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { createSignal, For, Show, onMount } from 'solid-js';
import { Check, CircleAlert, LoaderCircle, MessageSquare, Search, X } from 'lucide-solid';
import type { Scene } from '../contracts';

// Keep this read-only fallback order aligned with frozen.rs::display_title.
function displayTitle(scene: Scene): string {
  const title = scene.title.trim();
  if (title && title !== '新场景' && title !== '新会话') return title;
  const question = scene.messages.find(message => message.role === 'user' && message.text.trim())?.text;
  const source = [question, scene.draft, scene.items[0]?.asset.name].find(value => value?.trim());
  return source ? Array.from(source.trim().replace(/\s+/gu, ' ')).slice(0, 40).join('') : scene.background ? t('截图会话') : t('新会话');
}

export default function SessionDock(props: { scenes: Scene[]; activeId: string; busy: boolean; onActivate: (id: string) => void; onCloseScene: (id: string) => void; onClose: () => void }) {
  const [query, setQuery] = createSignal('');
  let input!: HTMLInputElement;
  onMount(() => input.focus());
  const scenes = () => {
    const needle = query().trim().toLowerCase().replace(/\s+/gu, ' ');
    return [...props.scenes].sort((a, b) => b.updatedAt - a.updatedAt).filter(scene => displayTitle(scene).toLowerCase().includes(needle) || scene.messages.some(message => message.text.toLowerCase().replace(/\s+/gu, ' ').includes(needle)));
  };
  return <div class="history-backdrop" onPointerDown={e => { if (e.target === e.currentTarget) props.onClose(); }}>
    <section class="session-history" role="dialog" aria-modal="true" aria-labelledby="history-title">
      <header><h2 id="history-title">{t("历史会话")}</h2><button class="icon-button" aria-label={t("关闭历史")} onClick={props.onClose}><X size={17} /></button></header>
      <label class="history-search"><Search size={15} /><input ref={input} aria-label={t("搜索会话")} placeholder={t("搜索")} value={query()} onInput={e => setQuery(e.currentTarget.value)} /></label>
      <div class="history-entries"><For each={scenes().map(scene => scene.id)}>{id => <Show when={props.scenes.find(scene => scene.id === id)}>{scene => <div class="history-entry" classList={{ active: props.activeId === id }}>
        <button class="history-restore" disabled={props.busy} title={t("恢复：{0}").replaceAll("{0}", () => String(displayTitle(scene())))} onClick={() => props.onActivate(id)}>
          <MessageSquare size={17} /><span><strong>{displayTitle(scene())}</strong><small>{new Date(scene().updatedAt).toLocaleString([], { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</small></span>
          <Show when={scene().run?.status === 'running'}><LoaderCircle size={14} class="spin" aria-label={t("处理中")} /></Show>
          <Show when={scene().run?.status === 'completed'}><Check size={14} class="complete" aria-label={t("完成")} /></Show>
          <Show when={scene().run?.status === 'failed'}><CircleAlert size={14} class="failed" aria-label={t("失败")} /></Show>
        </button>
        <Show when={!scene().closed}><button class="icon-button compact history-close" disabled={props.busy} title={t("关闭会话")} aria-label={t("关闭会话：{0}").replaceAll("{0}", () => String(displayTitle(scene())))} onPointerDown={event => event.stopPropagation()} onClick={event => { event.stopPropagation(); props.onCloseScene(id); }}><X size={14} /></button></Show>
      </div>}</Show>}</For></div>
    </section>
  </div>;
}
