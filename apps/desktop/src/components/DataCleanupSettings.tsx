// SPDX-License-Identifier: MPL-2.0
import { For, onCleanup, onMount, Show } from 'solid-js';
import { LoaderCircle, RefreshCw } from 'lucide-solid';
import { t } from '../i18n';
import { formatStorageBytes, type createDataCleanup } from '../data-cleanup';
import type { DataBucket, DataCategory } from '../settings-contracts';
import { Row } from './SettingControls';

type Controller = ReturnType<typeof createDataCleanup>;
const titles: Record<DataCategory, string> = { files: '历史文件', screenshots: '历史截图', conversations: '历史会话' };
function Review(props: { bucket: DataBucket; data: Controller }) {
  let root!: HTMLElement, cancel!: HTMLButtonElement;
  const previous = document.activeElement as HTMLElement | null;
  onMount(() => cancel.focus());
  onCleanup(() => { if (previous?.isConnected) previous.focus(); });
  function keydown(event: KeyboardEvent) {
    event.stopPropagation();
    if (event.key === 'Escape') { event.preventDefault(); props.data.cancel(); }
    if (event.key === 'Tab') {
      const controls = [...root.querySelectorAll<HTMLElement>('button:not(:disabled)')];
      if (!controls.length) { event.preventDefault(); return; }
      if (event.shiftKey && document.activeElement === controls[0]) { event.preventDefault(); controls.at(-1)?.focus(); }
      else if (!event.shiftKey && document.activeElement === controls.at(-1)) { event.preventDefault(); controls[0]?.focus(); }
    }
  }
  return <div class="settings-document-backdrop" onPointerDown={e => { if (e.target === e.currentTarget) props.data.cancel(); }}>
    <section ref={root} class="settings-document settings-cleanup-review" data-settings-inner-dialog role="dialog" aria-modal="true" aria-label={t('确认清理')} tabIndex={-1} on:keydown={keydown} aria-busy={props.data.pending()}>
      <h2>{t('清理 {0}').replace('{0}', t(titles[props.bucket.category]))}</h2>
      <p class="settings-cleanup-amount">{props.bucket.cleanableCount} {t('项')} · {formatStorageBytes(props.bucket.cleanableBytes)}<Show when={props.bucket.category === 'conversations'}> <small>{t('逻辑占用')}</small></Show></p>
      <p>{t(props.bucket.category === 'conversations' ? '仅删除已关闭且未被记忆引用的会话，保留当前会话和冻结会话。附件可随后单独清理。' : '仅删除未被引用且已保存超过一天的副本，保留会话、黑板和置顶正在使用的素材。')}</p>
      <p>{t('清理后无法恢复。')}</p>
      <div class="settings-actions"><button ref={cancel} class="secondary-button" disabled={props.data.pending()} onClick={() => props.data.cancel()}>{t('取消')}</button><button class="primary-button" disabled={props.data.pending()} onClick={() => void props.data.confirm()}><Show when={props.data.pending()}><LoaderCircle size={14} class="spin" /></Show>{t(props.data.pending() ? '清理中' : '确认清理')}</button></div>
    </section>
  </div>;
}
export function DataCleanupSettings(props: { data: Controller; disabled: boolean }) {
  return <section class="settings-storage-section" aria-busy={props.data.pending()}><div class="settings-section-heading"><span>{t('存储占用')}</span><button class="icon-button compact" aria-label={t('刷新占用')} title={t('刷新占用')} disabled={props.disabled || props.data.pending()} onClick={() => void props.data.load()}><RefreshCw size={14} classList={{ spin: props.data.pending() }} /></button></div>
    <Show when={props.data.usage()} fallback={<Show when={props.data.pending()}><LoaderCircle size={16} class="spin" aria-label={t('统计占用')} /></Show>}>{usage => <>
      <For each={['files', 'screenshots', 'conversations'] as DataCategory[]}>{category => <Row title={t(titles[category])} caption={`${usage()[category].count} ${t('项')} · ${formatStorageBytes(usage()[category].bytes)}${category === 'conversations' ? ` · ${t('逻辑占用')}` : ''}`}>
        <div class="settings-storage-actions"><span>{t('可清理')} {formatStorageBytes(usage()[category].cleanableBytes)}</span><button class="secondary-button" disabled={props.disabled || props.data.pending() || !usage().canClean || !usage()[category].cleanableCount} onClick={() => props.data.choose(category)}>{t('清理')}</button></div>
      </Row>}</For>
      <Row title={t('数据库')}><span class="settings-value">{formatStorageBytes(usage().databaseBytes)}</span></Row>
    </>}</Show>
    <Show when={props.data.error()}><p class="settings-error" role="alert">{props.data.error()}</p></Show>
    <Show when={props.data.receipt()}>{receipt => <p class="settings-cleanup-result" role="status">{t('已清理')} {receipt().removedCount} {t('项')} · {t('释放')} {formatStorageBytes(receipt().removedBytes)}<Show when={receipt().failedCount}> · {receipt().failedCount} {t('项未清理')}</Show><Show when={!receipt().compacted}> · {t('数据库压缩未完成')}</Show></p>}</Show>
    <Show when={props.data.review()}>{bucket => <Review bucket={bucket()} data={props.data} />}</Show>
  </section>;
}
