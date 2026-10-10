// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { createMemo, For, Show } from 'solid-js';
import { Play } from 'lucide-solid';
import type { Scene } from '../contracts';
import type { VideoAnswerAction } from '../video-annotation-contracts';
import { videoAnswerActions } from '../video-annotations';
import { formatTicks } from '../video-trim';
export default function VideoAnswerActions(props: { scene: Scene; runId?: string; disabled: boolean; onNavigate?: (action: VideoAnswerAction) => void }) {
  const actions = createMemo(() => videoAnswerActions(props.scene, props.runId));
  return <Show when={props.onNavigate && actions().length}><div class="video-answer-actions"><For each={actions()}>{action => <button class="quiet-button" disabled={props.disabled} title={t("播放此段")} aria-label={t("播放 {0}–{1}").replaceAll("{0}", () => String(formatTicks(action.interval.startTicks))).replaceAll("{1}", () => String(formatTicks(action.interval.endTicks)))} onClick={() => props.onNavigate?.(action)}><Play size={12} />{formatTicks(action.interval.startTicks)}–{formatTicks(action.interval.endTicks)}</button>}</For></div></Show>;
}
