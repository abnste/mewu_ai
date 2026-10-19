// SPDX-License-Identifier: MPL-2.0
import { t } from './i18n';
import { nativeSelectOwnsEscape } from './native-select-escape';
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show } from 'solid-js';
import { CircleAlert, LoaderCircle, Pause, Play, Square, X } from 'lucide-solid';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { controlRecording, recordingSurfaceReady, subscribeRecording, type RecordingAction, type RecordingStatus } from './bridge';
import { readPreferences } from './interface-preferences';
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
  const parameters = new URLSearchParams(location.search);
  const nativeLinux = parameters.get('native-platform') === 'linux', nativeCompound = parameters.get('compound') === '1';
  const previousCompound = document.documentElement.dataset.recordingCompound;
  if (nativeCompound) document.documentElement.dataset.recordingCompound = 'true';
  if (nativeLinux) document.documentElement.dataset.recordingPlatform = 'linux';
  const [status, setStatus] = createSignal<RecordingStatus | null>(null);
  const [pending, setPending] = createSignal(false);
  const [error, setError] = createSignal('');
  let disposed = false, stop: (() => void) | undefined;
  let surface: HTMLDivElement | undefined, controls: HTMLDivElement | undefined;
  const maskOpacity = readPreferences().maskOpacity;
  createEffect(() => {
    if (nativeCompound || status()?.hud) document.documentElement.dataset.recordingCompound = 'true';
    else delete document.documentElement.dataset.recordingCompound;
  });
  const readyKey = createMemo(() => {
    const current = status(), hud = current?.hud;
    if (!current || !hud || (hud.stage === 'layout' ? !['starting', 'countdown'].includes(current.phase) : hud.stage !== 'mask' || current.phase !== 'starting')) return undefined;
    return JSON.stringify([current.id, hud.stage, hud.rect]);
  });
  createEffect(on(readyKey, key => {
    if (!key) return;
    const current = status()!, id = current.id, stage = current.hud!.stage as 'layout' | 'mask';
    let canceled = false, sent = false, frame = 0;
    const owns = () => !canceled && !disposed && readyKey() === key;
    const laidOut = () => {
      if (!surface?.isConnected || (current.hud!.rect && !controls?.isConnected)) return false;
      const boxes = [surface, ...(current.hud!.rect ? [controls!] : [])].map(element => element.getBoundingClientRect());
      return boxes.every(box => Number.isFinite(box.width) && Number.isFinite(box.height) && box.width > 0 && box.height > 0);
    };
    const cancelFrame = () => { if (frame) cancelAnimationFrame(frame); frame = 0; };
    const schedule = () => {
      cancelFrame();
      if (!owns() || sent || document.hidden) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        if (!owns() || document.hidden || !laidOut()) return;
        frame = requestAnimationFrame(() => {
          frame = 0;
          if (!owns() || sent || document.hidden || !laidOut()) return;
          sent = true;
          // Visible layout is necessary readiness, not an optical first-frame claim.
          void recordingSurfaceReady(id, stage).catch(cause => {
            if (owns()) setError(cause instanceof Error ? cause.message : String(cause));
          });
        });
      });
    };
    const timer = setTimeout(() => { canceled = true; cancelFrame(); }, 10000);
    document.addEventListener('visibilitychange', schedule);
    window.addEventListener('resize', schedule);
    schedule();
    onCleanup(() => {
      canceled = true; cancelFrame(); clearTimeout(timer);
      document.removeEventListener('visibilitychange', schedule); window.removeEventListener('resize', schedule);
    });
  }));
  const label = () => ({ countdown: t('倒计时'), starting: t('准备中'), recording: t('录制中'), paused: t('已暂停'), stopping: t('处理中') }[status()?.phase || 'starting']);
  const waiting = () => status()?.phase === 'starting' || status()?.phase === 'stopping';
  const canPause = () => status()?.phase === 'recording' || status()?.phase === 'paused';
  const canceling = () => status()?.phase === 'countdown' || status()?.phase === 'starting';
  onMount(async () => {
    try {
      stop = await subscribeRecording(next => { if (!disposed) { setStatus(next); setError(''); } }, 'recording-controls');
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
  function beginDrag(event: PointerEvent) {
    if (!nativeLinux || disposed || !status() || event.button !== 0 || event.pointerType !== 'mouse') return;
    event.preventDefault();
    void getCurrentWindow().startDragging().catch(cause => {
      if (!disposed) setError(cause instanceof Error ? cause.message : String(cause));
    });
  }
  const keyboard = (event: KeyboardEvent) => {
    if (nativeSelectOwnsEscape(event)) return;
    if (event.key !== 'Escape' || !status() || status()?.phase === 'stopping') return;
    event.preventDefault(); event.stopPropagation(); stopOrCancel();
  };
  document.addEventListener('keydown', keyboard);
  onCleanup(() => { disposed = true; stop?.(); document.removeEventListener('keydown', keyboard); if (nativeLinux) delete document.documentElement.dataset.recordingPlatform;
    if (previousCompound) document.documentElement.dataset.recordingCompound = previousCompound;
    else delete document.documentElement.dataset.recordingCompound;
  });
  return <div ref={element => { surface = element; }} class="recording-control-window" classList={{ 'recording-compound-window': Boolean(nativeCompound || status()?.hud) }}>
    <Show when={status()} fallback={<Show when={!nativeCompound && error()}><div class="recording-controls recording-control-error" role="alert"><CircleAlert size={16} /><span title={error()}>{error()}</span></div></Show>}>
      {current => <><Show when={current().hud && current().hud!.stage !== 'layout'}><RecordingBackdrop status={current()} maskOpacity={maskOpacity} /></Show>
      <Show when={(!nativeCompound && !current().hud) || current().hud?.rect}><div ref={element => { controls = element; }} class="recording-controls-slot" style={current().hud?.rect ? {
        left: `${current().hud!.rect!.x * 100}%`, top: `${current().hud!.rect!.y * 100}%`, width: `${current().hud!.rect!.width * 100}%`, height: `${current().hud!.rect!.height * 100}%`,
      } : undefined}><div class="recording-controls" role="toolbar" aria-label={t("录屏控制")}>
        <Show when={!error()} fallback={<CircleAlert size={13} class="recording-status-icon" />}><Show when={!waiting()} fallback={<LoaderCircle size={13} class="spin recording-status-icon" />}><span class="recording-status-dot" classList={{ paused: current().phase === 'paused' }} title={label()} /></Show></Show>
        <output class="recording-elapsed" classList={{ 'recording-control-error': Boolean(error()), 'recording-control-draggable': nativeLinux }} onPointerDown={beginDrag} title={error() || label()} role={error() ? 'alert' : undefined} aria-label={error() ? t('录屏错误') : t('录制时长')}>{error() || timeLabel(current().elapsedMs)}</output>
        <button class="recording-control-button" disabled={pending() || !canPause()} title={current().phase === 'paused' ? t('继续录制') : t('暂停录制')} aria-label={current().phase === 'paused' ? t('继续录制') : t('暂停录制')} onClick={() => void act(current().phase === 'paused' ? 'resume' : 'pause')}><Show when={current().phase === 'paused'} fallback={<Pause size={16} />}><Play size={16} /></Show></button>
        <button class="recording-control-button recording-stop" disabled={pending() || current().phase === 'stopping'} title={canceling() ? t('取消录屏') : t("停止录屏{0}").replaceAll("{0}", () => String(current().stopHotkey ? ` · ${current().stopHotkey}` : ''))} aria-label={canceling() ? t('取消录屏') : t('停止录屏')} onClick={stopOrCancel}><Show when={canceling()} fallback={<Square size={13} fill="currentColor" />}><X size={17} /></Show></button>
      </div></div></Show></>}
    </Show>
  </div>;
}
