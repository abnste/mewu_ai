// SPDX-License-Identifier: MPL-2.0
import { t } from './i18n';
import { nativeSelectOwnsEscape } from './native-select-escape';
import { createMemo, createSignal, onCleanup, onMount, Show } from 'solid-js';
import { CircleAlert, LoaderCircle, Pause, Play, Square, X } from 'lucide-solid';
import { controlRecording, subscribeRecording, type RecordingAction, type RecordingStatus } from './bridge';
import './recording.css';

export function RecordingBackdrop(props: { status: RecordingStatus; maskOpacity: number }) {
  const previous = document.documentElement.dataset.surface;
  document.documentElement.dataset.surface = 'recording-live';
  onCleanup(() => {
    if (previous) document.documentElement.dataset.surface = previous;
    else delete document.documentElement.dataset.surface;
  });
  const rect = createMemo(() => {
    const value = props.status.rect;
    const x = Math.max(0, Math.min(1, value.x)), y = Math.max(0, Math.min(1, value.y));
    return { x, y, width: Math.max(0, Math.min(1 - x, value.width)), height: Math.max(0, Math.min(1 - y, value.height)) };
  });
  return <div class="recording-live-mask" aria-label={t("录屏区域")}>
    <svg viewBox="0 0 1 1" preserveAspectRatio="none" aria-hidden="true">
      <defs><mask id="recording-cutout" maskUnits="userSpaceOnUse" x="0" y="0" width="1" height="1">
        <rect width="1" height="1" fill="white" />
        <rect x={rect().x} y={rect().y} width={rect().width} height={rect().height} fill="black" />
      </mask></defs>
      <rect width="1" height="1" fill="#000" fill-opacity={props.maskOpacity} mask="url(#recording-cutout)" />
      <rect class="recording-region-outline" x={rect().x} y={rect().y} width={rect().width} height={rect().height} vector-effect="non-scaling-stroke" />
    </svg>
    <Show when={props.status.phase === 'countdown' && props.status.countdown > 0}>
      <div class="recording-countdown" style={{ left: `${(rect().x + rect().width / 2) * 100}%`, top: `${(rect().y + rect().height / 2) * 100}%` }} aria-live="polite">{props.status.countdown}</div>
    </Show>
  </div>;
}

const timeLabel = (elapsedMs: number) => {
  const seconds = Math.max(0, Math.floor(elapsedMs / 1000));
  const minutes = Math.floor(seconds / 60), remaining = seconds % 60;
  return `${minutes.toString().padStart(2, '0')}:${remaining.toString().padStart(2, '0')}`;
};

export default function RecordingSurface() {
  document.documentElement.dataset.surface = 'recording';
  const [status, setStatus] = createSignal<RecordingStatus | null>(null);
  const [pending, setPending] = createSignal(false);
  const [error, setError] = createSignal('');
  let disposed = false, stop: (() => void) | undefined;
  const label = () => ({ countdown: t('倒计时'), starting: t('准备中'), recording: t('录制中'), paused: t('已暂停'), stopping: t('处理中') }[status()?.phase || 'starting']);
  const waiting = () => status()?.phase === 'starting' || status()?.phase === 'stopping';
  const canPause = () => status()?.phase === 'recording' || status()?.phase === 'paused';
  const canceling = () => status()?.phase === 'countdown' || status()?.phase === 'starting';
  onMount(async () => {
    try {
      stop = await subscribeRecording(next => { if (!disposed) { setStatus(next); setError(''); } });
      if (disposed) stop();
    } catch (cause) { if (!disposed) setError(cause instanceof Error ? cause.message : String(cause)); }
  });
  async function act(action: RecordingAction) {
    const current = status(); if (!current || pending()) return;
    setPending(true); setError('');
    try { await controlRecording(current.id, action); }
    catch (cause) { if (!disposed) setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { if (!disposed) setPending(false); }
  }
  const stopOrCancel = () => void act(canceling() ? 'cancel' : 'stop');
  const keyboard = (event: KeyboardEvent) => {
    if (nativeSelectOwnsEscape(event)) return;
    if (event.key !== 'Escape' || !status() || status()?.phase === 'stopping') return;
    event.preventDefault(); event.stopPropagation(); stopOrCancel();
  };
  document.addEventListener('keydown', keyboard);
  onCleanup(() => { disposed = true; stop?.(); document.removeEventListener('keydown', keyboard); });
  return <div class="recording-control-window">
    <Show when={status()} fallback={<Show when={error()}><div class="recording-controls recording-control-error" role="alert"><CircleAlert size={16} /><span title={error()}>{error()}</span></div></Show>}>
      {current => <div class="recording-controls" role="toolbar" aria-label={t("录屏控制")}>
        <Show when={!error()} fallback={<CircleAlert size={13} class="recording-status-icon" />}><Show when={!waiting()} fallback={<LoaderCircle size={13} class="spin recording-status-icon" />}><span class="recording-status-dot" classList={{ paused: current().phase === 'paused' }} title={label()} /></Show></Show>
        <output class="recording-elapsed" classList={{ 'recording-control-error': Boolean(error()) }} title={error() || label()} role={error() ? 'alert' : undefined} aria-label={error() ? t('录屏错误') : t('录制时长')}>{error() || timeLabel(current().elapsedMs)}</output>
        <button class="recording-control-button" disabled={pending() || !canPause()} title={current().phase === 'paused' ? t('继续录制') : t('暂停录制')} aria-label={current().phase === 'paused' ? t('继续录制') : t('暂停录制')} onClick={() => void act(current().phase === 'paused' ? 'resume' : 'pause')}><Show when={current().phase === 'paused'} fallback={<Pause size={16} />}><Play size={16} /></Show></button>
        <button class="recording-control-button recording-stop" disabled={pending() || current().phase === 'stopping'} title={canceling() ? t('取消录屏') : t("停止录屏{0}").replaceAll("{0}", () => String(current().stopHotkey ? ` · ${current().stopHotkey}` : ''))} aria-label={canceling() ? t('取消录屏') : t('停止录屏')} onClick={stopOrCancel}><Show when={canceling()} fallback={<Square size={13} fill="currentColor" />}><X size={17} /></Show></button>
      </div>}
    </Show>
  </div>;
}
