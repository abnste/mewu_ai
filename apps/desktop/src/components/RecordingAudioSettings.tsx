// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { createEffect, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { LoaderCircle } from 'lucide-solid';
import { getRecordingAudio, setRecordingAudio, subscribeRecordingAudio } from '../recording-audio-bridge';
import { audioLabels, audioModes, recordingAudioGrants, RecordingAudioController, type RecordingAudioMode, type RecordingAudioView } from '../recording-audio';
import { Row } from './SettingControls';

export default function RecordingAudioSettings(props: { active: boolean }) {
  const [view, setView] = createSignal<RecordingAudioView>({ mode: 'system', pending: false, error: '' });
  const grants = recordingAudioGrants(), grant = grants[0];
  const [loading, setLoading] = createSignal(false);
  const controller = new RecordingAudioController(setRecordingAudio, setView);
  controller.reconcile(grants);
  let started = false, disposed = false, stops: (() => void)[] = [];
  async function initialize() {
    if (disposed || loading()) return;
    setLoading(true);
    const acquired: (() => void)[] = [];
    try {
      if (!stops.length) {
        acquired.push(await subscribeRecordingAudio(state => controller.readSucceeded(state)));
        if (!disposed) stops = acquired.splice(0);
      } else {
        if (!disposed) controller.readSucceeded(await getRecordingAudio());
      }
    } catch (error) { if (!disposed) controller.readFailed(error); }
    finally { for (const stop of acquired) stop(); if (!disposed) setLoading(false); }
  }
  async function retry() {
    const draft = view().draft;
    if (draft) await controller.choose(draft.mode, draft.grant); else await initialize();
  }
  createEffect(on(() => props.active, active => { if (active && !started) { started = true; void initialize(); } }));
  onCleanup(() => { disposed = true; stops.forEach(stop => stop()); controller.dispose(); });
  return <>
    <Row title={t("声音")}><Show when={!loading()} fallback={<LoaderCircle size={15} class="spin" aria-label={t("加载录屏声音")} />}><select aria-label={t("录屏声音")} disabled={view().pending || !view().state?.editable} value={view().mode} onChange={event => void controller.choose(event.currentTarget.value as RecordingAudioMode, grant)}><For each={audioModes}>{mode => <option value={mode}>{t(audioLabels[mode])}</option>}</For></select></Show></Row>
    <Show when={view().error || view().state?.message}><p class="settings-error" role="alert">{view().error || view().state?.message}</p></Show>
    <Show when={view().error}><div class="settings-actions"><Show when={view().draft}><button class="secondary-button" disabled={view().pending} onClick={() => controller.discard()}>{t("取消")}</button></Show><button class="secondary-button" disabled={view().pending || loading()} onClick={() => void retry()}>{t("重试")}</button></div></Show>
  </>;
}
