// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, untrack } from 'solid-js';
import { Pause, Play } from 'lucide-solid';
import { t } from '../i18n';
import type { PinVideoMedia } from '../pin-media';
import { VideoPlayback } from '../video-media';
import { reducedVideoPreview } from '../video-preview';
import { TICKS_PER_SECOND, checkedRange } from '../video-trim';
import { videoAnnotationPreviewTicks, videoAnnotationVisible } from '../video-annotations';
import ObjectMoveGrip from './ObjectMoveGrip';
import './pin-video.css';

interface Props { source: string; media: PinVideoMedia; width: number; height: number; onReady?: () => void; onFailed?: () => void; onError: (error: unknown) => void; onDrag: (event: PointerEvent) => void }
/** Independent source-clock playback. No native mutation or scene lookup. */
export default function PinVideo(props: Props) {
  let video!: HTMLVideoElement, playback: VideoPlayback | undefined, disposed = false, ready = false, stopVisibility: (() => void) | undefined;
  let preview!: HTMLCanvasElement;
  const [canvasVisible, setCanvasVisible] = createSignal(false);
  const canvasPresentation = createMemo(() => props.media.canvasPresentation === true);
  createEffect(() => {
    const canvas = canvasPresentation();
    if (video && preview) onCleanup(reducedVideoPreview(video, preview, 100, visible => { setCanvasVisible(visible); if (visible) untrack(checkReady); }, canvas, canvas));
  });
  const [duration, setDuration] = createSignal(props.media.sourceDurationTicks ?? 0);
  const [position, setPosition] = createSignal(0), [playing, setPlaying] = createSignal(false);
  const [decoded, setDecoded] = createSignal(new Set<string>());
  const annotationIndex = createMemo(() => new Map(props.media.annotations.map(annotation => [annotation.id, annotation])));
  const range = createMemo(() => duration() > 0 ? checkedRange(duration(), props.media.range) : undefined);
  const previewTicks = () => videoAnnotationPreviewTicks(position(), range(), !playing(), video?.seeking ?? true);
  const checkReady = () => {
    if (ready || disposed || !video || video.readyState < 2 || video.seeking || decoded().size !== props.media.annotations.length || props.media.canvasPresentation && !canvasVisible()) return;
    ready = true; props.onReady?.();
  };
  function metadata() {
    if (!props.media.sourceDurationTicks) {
      const ticks = Math.round(video.duration * TICKS_PER_SECOND);
      if (!Number.isSafeInteger(ticks) || ticks <= 0) { props.onFailed?.(); return; }
      setDuration(ticks);
    }
    const retained = range(); if (retained) playback?.seekInitial(retained.startTicks);
  }
  async function toggle() {
    if (!playback || disposed) return;
    try { if (playing()) playback.pause(); else await playback.play(); }
    catch (error) { if (!disposed) props.onError(error); }
  }
  const visibility = () => { if (document.hidden) playback?.pause(); };
  onMount(() => {
    playback = new VideoPlayback(video, { identity: () => props.source, range, loop: () => !disposed && !document.hidden, position: ticks => { if (!disposed) setPosition(ticks); }, playing: value => { if (!disposed) setPlaying(value); }, error: props.onError });
    document.addEventListener('visibilitychange', visibility);
    stopVisibility = () => document.removeEventListener('visibilitychange', visibility);
    if (video.readyState >= 1) metadata();
  });
  onCleanup(() => { disposed = true; stopVisibility?.(); playback?.dispose(); });
  return <div class="pin-video-content" onContextMenu={event => event.preventDefault()} onPointerDown={event => { if (!(event.target as Element).closest('.object-move-grip')) event.stopPropagation(); }} onDblClick={event => event.stopPropagation()}>
    <canvas ref={preview} class="pin-video-preview" classList={{ visible: canvasVisible() }} aria-hidden="true" />
    <video ref={video} class="pin-video-element" src={props.source} playsinline preload="auto" draggable={false} onLoadedMetadata={metadata} onLoadedData={checkReady} onSeeked={checkReady} onError={() => props.onFailed?.()} onClick={event => { event.stopPropagation(); void toggle(); }} />
    <div class="pin-video-ink" aria-hidden="true"><For each={props.media.annotations.map(annotation => annotation.id)}>{id => { const annotation = () => annotationIndex().get(id)!; return <img src={annotation().imageUrl} alt="" draggable={false} style={{ left: `${annotation().bounds.x / props.width * 100}%`, top: `${annotation().bounds.y / props.height * 100}%`, width: `${annotation().bounds.width / props.width * 100}%`, height: `${annotation().bounds.height / props.height * 100}%`, visibility: range() && videoAnnotationVisible(annotation().interval, range()!, previewTicks()) ? 'visible' : 'hidden' }} onLoad={async event => { try { await event.currentTarget.decode(); if (!disposed) { setDecoded(old => new Set([...old, id])); checkReady(); } } catch { if (!disposed) props.onFailed?.(); } }} onError={() => props.onFailed?.()} />; }}</For></div>
    <ObjectMoveGrip label={t('移动')} onPointerDown={props.onDrag} />
    <div class="pin-video-controls"><button type="button" aria-label={playing() ? t('暂停') : t('播放')} title={playing() ? t('暂停') : t('播放')} onClick={() => void toggle()}>{playing() ? <Pause size={16} /> : <Play size={16} />}</button></div>
  </div>;
}
