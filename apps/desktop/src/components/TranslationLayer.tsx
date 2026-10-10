// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { createEffect, createSignal, on, onCleanup, Show } from 'solid-js';
import { LoaderCircle, X } from 'lucide-solid';
import type { SavedTranslation } from '../contracts';
import { assetUrl } from '../bridge';
import './translation.css';

export default function TranslationLayer(props: { value: SavedTranslation; active: boolean; disabled: boolean; onRemove?: () => Promise<void>; onError: (message: string) => void }) {
  const [failed, setFailed] = createSignal(false);
  const [removing, setRemoving] = createSignal(false);
  let disposed = false;
  onCleanup(() => { disposed = true; });
  createEffect(on(() => props.value.overlay.id, () => setFailed(false)));
  return <><Show when={!failed()}><img class="translation-overlay" data-translation={props.value.overlay.id} src={assetUrl(props.value.overlay)} alt="" draggable={false} onError={() => { setFailed(true); props.onError('无法读取译文图层'); }} /></Show>
    <Show when={props.active && props.onRemove}><div class="translation-actions" onPointerDown={event => { event.preventDefault(); event.stopPropagation(); }}><button title={t("移除译文")} aria-label={t("移除译文")} disabled={props.disabled || removing()} onClick={() => { setRemoving(true); void props.onRemove?.().catch(error => { if (!disposed) props.onError(error instanceof Error ? error.message : String(error)); }).finally(() => { if (!disposed) setRemoving(false); }); }}><Show when={removing()} fallback={<X size={14} />}><LoaderCircle size={14} class="spin" /></Show></button></div></Show>
    <Show when={failed()}><button class="translation-retry" onPointerDown={event => event.stopPropagation()} onClick={() => setFailed(false)}>{t("重试译文图层")}</button></Show></>;
}
