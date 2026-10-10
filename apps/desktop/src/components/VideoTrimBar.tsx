// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { nativeSelectOwnsEscape } from '../native-select-escape';
import { createEffect, createMemo, createSignal, on, onCleanup, Show } from 'solid-js';
import { ArrowLeftToLine, ArrowRightToLine, Pause, Play, RotateCcw, Undo2, Redo2 } from 'lucide-solid';
import { VideoTrimGesture, checkedRange, clampPosition, formatTicks, positionFromClient, type TrimAuthority, type TrimEdit, type TrimPart, type TrimReceipt, type TrimView } from '../video-trim';
import './video-trim.css';
export interface VideoTrimBarProps {
  authority?: TrimAuthority;
  positionTicks: number;
  playing: boolean;
  busy: boolean;
  onPause: () => void;
  onSeek: (ticks: number) => void;
  onResume: (ticks: number) => void;
  onTogglePlayback: () => void;
  onCommit: (edit: TrimEdit) => Promise<TrimReceipt>;
  onError: (error: unknown) => void;
  canUndo?: boolean;
  canRedo?: boolean;
  onReplay?: (direction: 'undo' | 'redo') => Promise<void>;
  onRegister?: (flush: (active: () => boolean) => Promise<void>) => () => void;
}
export default function VideoTrimBar(props: VideoTrimBarProps) {
  let rail!: HTMLDivElement, root!: HTMLDivElement, stopPointer: (() => void) | undefined, disposed = false;
  const [view, setView] = createSignal<TrimView>();
  const [replaying, setReplaying] = createSignal(false);
  const state = createMemo(() => props.authority);
  const controller = new VideoTrimGesture({ current: state, position: () => props.positionTicks, playing: () => props.playing, pause: () => props.onPause(), seek: ticks => props.onSeek(ticks), resume: ticks => props.onResume(ticks), commit: edit => props.onCommit(edit), changed: setView, error: props.onError });
  const range = () => view()?.range ?? (state() ? checkedRange(state()!.durationTicks, state()!.range) : { startTicks: 0, endTicks: 0 });
  const position = () => view()?.positionTicks ?? 0;
  const disabled = () => props.busy || view()?.pending || replaying() || !state();
  const canEdit = () => Boolean(state()?.editable && !disabled());
  const percent = (ticks: number) => `${Math.min(100, Math.max(0, ticks / (state()?.durationTicks || 1) * 100))}%`;
  const stop = props.onRegister?.(async active => { stopPointer?.(); await controller.flush(active); });
  createEffect(on(() => [state()?.identity, state()?.durationTicks, state()?.revision, state()?.range?.startTicks, state()?.range?.endTicks, state()?.editable, props.positionTicks, props.playing] as const, () => { controller.reconcile(); if (!controller.isInteracting()) stopPointer?.(); }));
  createEffect(on(() => props.busy, value => { if (value) { stopPointer?.(); controller.cancel(false); } }));
  function cancelPointer(restore = true) { stopPointer?.(); controller.cancel(restore); }
  function pointerDown(event: PointerEvent, part: TrimPart) {
    if (event.button !== 0 || disabled() || (part !== 'seek' && !canEdit())) return;
    event.preventDefault(); event.stopPropagation();
    if (!controller.begin(part)) return;
    const target = event.currentTarget as HTMLElement, id = event.pointerId;
    const box = rail.getBoundingClientRect();
    // Preserve pointer-to-thumb offset so a grab at the edge never jumps its boundary.
    const original = part === 'start' ? range().startTicks : part === 'end' ? range().endTicks : position();
    const offset = target === rail ? 0 : event.clientX - (box.left + original / state()!.durationTicks * box.width);
    const update = (value: PointerEvent) => { if (value.pointerId === id && state()) controller.move(positionFromClient(value.clientX - offset, box.left, box.width, state()!.durationTicks)); };
    const release = () => {
      target.removeEventListener('pointermove', moved); target.removeEventListener('pointerup', ended); target.removeEventListener('pointercancel', canceled); target.removeEventListener('lostpointercapture', canceled);
      if (target.hasPointerCapture(id)) target.releasePointerCapture(id); if (stopPointer === release) stopPointer = undefined;
    };
    const moved = (value: PointerEvent) => { if (value.pointerId !== id) return; value.preventDefault(); value.stopPropagation(); update(value); };
    const ended = (value: PointerEvent) => { if (value.pointerId !== id) return; value.preventDefault(); value.stopPropagation(); update(value); release(); void controller.complete(); };
    const canceled = (value: PointerEvent) => { if (value.pointerId !== id) return; release(); controller.cancel(true); };
    stopPointer = release; target.focus(); target.setPointerCapture(id);
    target.addEventListener('pointermove', moved); target.addEventListener('pointerup', ended); target.addEventListener('pointercancel', canceled); target.addEventListener('lostpointercapture', canceled);
    if (target === rail) update(event);
  }
  async function replay(direction: 'undo' | 'redo') {
    if (!canEdit() || !props.onReplay || !(direction === 'undo' ? props.canUndo : props.canRedo)) return;
    cancelPointer(false); setReplaying(true);
    try { await controller.flush(() => !disposed); if (!disposed) await props.onReplay(direction); }
    catch (error) { if (!disposed) props.onError(error); }
    finally { if (!disposed) setReplaying(false); }
  }
  function keyDown(event: KeyboardEvent) {
    if (nativeSelectOwnsEscape(event)) return;
    if (event.isComposing || event.keyCode === 229 || (event.target as Element)?.closest('input,textarea,select,[contenteditable="true"]')) return;
    if (event.key === 'Escape') { event.preventDefault(); event.stopImmediatePropagation(); cancelPointer(true); return; }
    const lower = event.key.toLowerCase();
    if ((event.ctrlKey || event.metaKey) && !event.altKey && (lower === 'z' || lower === 'y')) { event.preventDefault(); event.stopImmediatePropagation(); void replay(lower === 'y' || event.shiftKey ? 'redo' : 'undo'); return; }
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    if (event.key === ' ' && !(event.target as Element)?.closest('button')) { event.preventDefault(); event.stopImmediatePropagation(); if (!disabled() && !controller.isInteracting()) props.onTogglePlayback(); return; }
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault(); event.stopImmediatePropagation();
    if (disabled() || (event.target as Element)?.closest('button')) return;
    const part = (event.target as HTMLElement).dataset.trimPart as TrimPart | undefined;
    controller.keyDown(part ?? 'seek', event.key, event.shiftKey);
  }
  const keyUp = (event: KeyboardEvent) => { const finished = controller.keyUp(event.key); if (finished) { event.preventDefault(); event.stopImmediatePropagation(); void finished; } };
  const blur = () => cancelPointer(false);
  window.addEventListener('keyup', keyUp, true); window.addEventListener('blur', blur);
  onCleanup(() => { disposed = true; stop?.(); stopPointer?.(); controller.dispose(); window.removeEventListener('keyup', keyUp, true); window.removeEventListener('blur', blur); });
  return <Show when={state()}><div ref={root} class="video-trim-bar" data-trim-interacting={(view(), controller.isInteracting())} role="group" aria-label={t("视频进度与裁切")} onPointerDown={event => event.stopPropagation()} onWheel={event => event.stopPropagation()} on:keydown={keyDown}>
    <button class="icon-button" disabled={Boolean(disabled())} aria-label={props.playing ? t('暂停视频') : t('播放视频')} title={props.playing ? t('暂停视频') : t('播放视频')} onClick={props.onTogglePlayback}><Show when={props.playing} fallback={<Play size={18} />}><Pause size={18} /></Show></button>
    <div class="video-trim-track-wrap"><div ref={rail} class="video-trim-track" tabIndex={0} aria-label={t("视频进度")} onPointerDown={event => pointerDown(event, 'seek')}>
      <span class="video-trim-rail" /><span class="video-trim-retained" style={{ left: percent(range().startTicks), width: percent(range().endTicks - range().startTicks) }} />
      <Show when={state()?.editable}><div class="video-trim-thumb start" role="slider" tabIndex={0} data-trim-part="start" aria-label={t("裁切起点")} aria-valuemin={0} aria-valuemax={range().endTicks - Math.min(1_000_000, state()!.durationTicks)} aria-valuenow={range().startTicks} aria-valuetext={formatTicks(range().startTicks)} aria-disabled={!canEdit()} style={{ left: percent(range().startTicks) }} onPointerDown={event => pointerDown(event, 'start')} /><div class="video-trim-thumb end" role="slider" tabIndex={0} data-trim-part="end" aria-label={t("裁切终点")} aria-valuemin={range().startTicks + Math.min(1_000_000, state()!.durationTicks)} aria-valuemax={state()!.durationTicks} aria-valuenow={range().endTicks} aria-valuetext={formatTicks(range().endTicks)} aria-disabled={!canEdit()} style={{ left: percent(range().endTicks) }} onPointerDown={event => pointerDown(event, 'end')} /></Show>
      <div class="video-trim-thumb position" role="slider" tabIndex={0} data-trim-part="seek" aria-label={t("播放位置")} aria-valuemin={range().startTicks} aria-valuemax={range().endTicks} aria-valuenow={clampPosition(position(), range())} aria-valuetext={formatTicks(position())} aria-disabled={Boolean(disabled())} style={{ left: percent(position()) }} onPointerDown={event => pointerDown(event, 'seek')} />
    </div><span class="video-trim-range-time">{formatTicks(range().startTicks)} – {formatTicks(range().endTicks)}</span></div>
    <div class="video-trim-clock"><span>{formatTicks(position())} / {formatTicks(state()!.durationTicks)}</span><small>{t("保留")}{formatTicks(range().endTicks - range().startTicks)}</small></div>
    <Show when={state()?.editable}><div class="video-trim-actions"><button class="icon-button" aria-label={t("将当前位置设为起点")} title={t("设为起点")} disabled={!canEdit()} onClick={() => void controller.setBoundary('start')}><ArrowLeftToLine size={17} /></button><button class="icon-button" aria-label={t("将当前位置设为终点")} title={t("设为终点")} disabled={!canEdit()} onClick={() => void controller.setBoundary('end')}><ArrowRightToLine size={17} /></button><button class="icon-button" aria-label={t("恢复完整视频")} title={t("恢复整段")} disabled={!canEdit()} onClick={() => void controller.reset()}><RotateCcw size={17} /></button><button class="icon-button" aria-label={t("撤销裁切")} title={t("撤销")} disabled={!canEdit() || !props.canUndo} onClick={() => void replay('undo')}><Undo2 size={17} /></button><button class="icon-button" aria-label={t("重做裁切")} title={t("重做")} disabled={!canEdit() || !props.canRedo} onClick={() => void replay('redo')}><Redo2 size={17} /></button></div></Show>
  </div></Show>;
}
