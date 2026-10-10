// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { createEffect, createMemo, createResource, createSignal, on, Show } from 'solid-js';
import { RotateCcw } from 'lucide-solid';
import { readArtifact } from '../bridge';
import { textPageBoundaries } from '../asset-import';
import './text-artifact.css';

export default function TextArtifact(props: { assetId: string; name: string }) {
  const [reload, setReload] = createSignal(0);
  const [page, setPage] = createSignal(0);
  const identity = createMemo(() => props.assetId);
  const requestKey = createMemo(() => `${identity()}:${reload()}`);
  const [content] = createResource(requestKey, () => readArtifact(identity()));
  const boundaries = createMemo(() => textPageBoundaries(content.error ? '' : content() ?? ''));
  createEffect(on(requestKey, () => setPage(0)));
  const pageCount = () => boundaries().length - 1;
  const visibleText = () => (content() ?? '').slice(boundaries()[page()], boundaries()[page() + 1]);
  return <div class="text-artifact">
    <Show when={!content.error} fallback={<div class="text-artifact-error"><span class="inline-error">{t("无法读取文本")}</span><button class="icon-button compact" title={t("重试")} aria-label={t("重新读取文本")} onClick={() => setReload(value => value + 1)}><RotateCcw size={14} /></button></div>}>
      <Show when={!content.loading} fallback={<span class="quiet-state">{t("读取中")}</span>}>
        <pre class="text-artifact-page" tabIndex={0} role="textbox" aria-readonly="true" aria-label={props.name}>{visibleText()}</pre>
        <Show when={pageCount() > 1}><nav class="text-artifact-pages" aria-label={t("文本分页")}>
          <button class="quiet-button" disabled={page() === 0} onClick={() => setPage(value => value - 1)}>{t("上一页")}</button>
          <span>{page() + 1} / {pageCount()}</span>
          <button class="quiet-button" disabled={page() + 1 === pageCount()} onClick={() => setPage(value => value + 1)}>{t("下一页")}</button>
        </nav></Show>
      </Show>
    </Show>
  </div>;
}
