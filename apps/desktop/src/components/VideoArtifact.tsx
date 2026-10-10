// SPDX-License-Identifier: MPL-2.0
import { t } from '../i18n';
import { nativeSelectOwnsEscape } from '../native-select-escape';
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { Check, LoaderCircle, Pencil, Redo2, RefreshCw, Trash2, Undo2, X } from 'lucide-solid';
import type { Drawing, SpaceItem } from '../contracts';
import VideoDrawingEditor, { type VideoDrawingEditorProps } from './VideoDrawingEditor';
import type { VideoDrawingGrant } from '../video-drawing-contracts';
import { DrawingShape } from './DrawingLayer';
import type { EditorBox } from '../drawing-editor-port';
import { assetUrl } from '../bridge';
import type { VideoAction, VideoExportState, VideoGrant, VideoMetadata, VideoTarget } from '../video-contracts';
import { getVideoInfo, videoNative } from '../video-bridge';
import { VideoRequestLane, type RegisterVideoFlush } from '../video-requests';
import { VideoPlayback } from '../video-media';
import { placeVideoControls } from '../video-placement';
import { checkedRange, clampPosition, sameRange, TICKS_PER_SECOND, TrimCanceled, type TrimAuthority, type TrimEdit, type TrimReceipt } from '../video-trim';
import VideoTrimBar from './VideoTrimBar';
import type { RegisterVideoPlayer, VideoAnnotationAction, VideoAnnotationTarget, VideoTextContent, VideoTextLayoutRef } from '../video-annotation-contracts';
import { getVideoAnnotationPlan, getVideoAnnotationPreview, getVideoAnnotationText } from '../video-annotation-bridge';
import { VideoManualEditor, videoPoint, type VideoManualView } from '../video-manual-edit';
import { VideoAnnotationReader, type VideoAnnotationReadState } from '../video-annotation-reader';
import { intersectVideoInterval, sameVideoAnnotationTarget, videoAnnotationIdentity, videoAnnotationTarget, videoAnnotationVisible, videoSourceIdentity } from '../video-annotations';
import { VideoAnnotationPlaybackHold } from '../video-annotation-playback';

const metadataCache = new Map<string, VideoMetadata>();
export interface VideoArtifactProps {
  sceneId: string; item: SpaceItem; active: boolean; busy: boolean; grant?: VideoGrant;
  blackboard?: boolean;
  lane: VideoRequestLane; onRegister: RegisterVideoFlush;
  onActivate?: () => void;
  onRegisterPause?: (pause: () => void) => () => void;
  onPauseAll?: () => void;
  onRegisterPlayer?: RegisterVideoPlayer;
  annotationBusy?: boolean;
  authoringBusy?: boolean; inputLocked?: boolean; drawingAllowed?: boolean; drawingGrant?: VideoDrawingGrant;
  drawing?: Omit<VideoDrawingEditorProps, 'sceneId' | 'item' | 'info' | 'box' | 'grant' | 'busy' | 'inputLocked' | 'active' | 'onPause' | 'onClose' | 'onError' | 'renderStored'>;
  onAnnotationEdit?: (target: VideoAnnotationTarget, action: VideoAnnotationAction) => Promise<SpaceItem>;
  onAnnotationTextEdit?: (requestId: string, target: VideoAnnotationTarget, annotationId: string, reference: VideoTextLayoutRef, content: VideoTextContent) => Promise<SpaceItem>;
  onEdit: (grant: VideoGrant, target: VideoTarget, action: VideoAction) => Promise<SpaceItem>;
  exporting?: VideoExportState; onCancelExport: () => void;
  onError: (error: unknown) => void;
}
export default function VideoArtifact(props: VideoArtifactProps) {
  let video!: HTMLVideoElement, popover: HTMLDivElement | undefined, playback: VideoPlayback | undefined, disposed = false;
  let popoverResize: ResizeObserver | undefined;
  let textEditor: HTMLDivElement | undefined, textResize: ResizeObserver | undefined;
  let annotationGeneration = 0;
  const [metadata, setMetadata] = createSignal<VideoMetadata>();
  const [mediaDuration, setMediaDuration] = createSignal(0);
  const [position, setPosition] = createSignal(0), [playing, setPlaying] = createSignal(false);
  const [reading, setReading] = createSignal(false), [readError, setReadError] = createSignal('');
  const [failed, setFailed] = createSignal(false), [retry, setRetry] = createSignal(0);
  const [annotationState, setAnnotationState] = createSignal<VideoAnnotationReadState>({ loading: false });
  const [annotationPosition, setAnnotationPosition] = createSignal<number>();
  const [selectedAnnotation, setSelectedAnnotation] = createSignal<string>();
  const [annotationEditing, setAnnotationEditing] = createSignal(false);
  const [annotationRetry, setAnnotationRetry] = createSignal(0);
  const [decodedImages, setDecodedImages] = createSignal<ReadonlySet<string>>(new Set());
  const [placement, setPlacement] = createSignal({ left: 8, top: 8, width: 300 });
  const [manualView, setManualView] = createSignal<VideoManualView>({ reading: false, pending: false });
  const [textPlacement, setTextPlacement] = createSignal({ left: 8, top: 8 });
  const [drawingOpen, setDrawingOpen] = createSignal(false);
  const [drawingBox, setDrawingBox] = createSignal({ x: 0, y: 0, width: 0, height: 0 });
  let stopAnnotationPointer: (() => void) | undefined;
  const sourceKey = createMemo(() => videoSourceIdentity(props.sceneId, props.item));
  const annotationKey = createMemo(() => videoAnnotationIdentity(props.sceneId, props.item));
  const target = (): VideoTarget => ({ sceneId: props.sceneId, itemId: props.item.id, sourceId: props.item.asset.id, expectedRevision: props.item.videoEdit?.revision ?? 0 });
  const duration = () => props.item.videoEdit?.sourceDurationTicks ?? props.item.videoAnnotations?.sourceDurationTicks ?? metadata()?.durationTicks ?? (!videoNative ? mediaDuration() : 0);
  const range = () => duration() > 0 ? checkedRange(duration(), props.item.videoEdit?.range ?? null) : undefined;
  const identity = createMemo(() => JSON.stringify([sourceKey(), props.grant ?? null]));
  const authority = createMemo<TrimAuthority | undefined>(() => {
    const total = duration(); if (!total) return;
    return { identity: identity(), durationTicks: total, revision: props.item.videoEdit?.revision ?? 0, range: props.item.videoEdit?.range ?? null, editable: Boolean(props.grant && metadata() && videoNative) };
  });
  const report = (error: unknown) => { if (!disposed && !(error instanceof TrimCanceled)) props.onError(error); };
  const previewHold = new VideoAnnotationPlaybackHold();
  const pausePlayback = () => { previewHold.invalidate(); playback?.pause(); };
  const annotationReader = new VideoAnnotationReader(getVideoAnnotationPlan, getVideoAnnotationPreview, setAnnotationState);
  createEffect(on(() => [annotationKey(), annotationRetry()] as const, () => {
    annotationGeneration++; setSelectedAnnotation(undefined); setDecodedImages(new Set<string>()); pausePlayback();
    annotationReader.select(props.sceneId, props.item, sourceKey());
  }));
  createEffect(on(() => props.active, active => { if (!active) setSelectedAnnotation(undefined); }));
  createEffect(on(sourceKey, () => setDrawingOpen(false), { defer: true }));
  const visibleAnnotations = createMemo(() => {
    const state = annotationState(), plan = state.plan;
    if (!plan || !sameVideoAnnotationTarget(plan.target, videoAnnotationTarget(props.sceneId, props.item))) return [];
    return plan.entries.filter(entry => videoAnnotationVisible(entry.interval, plan.range, annotationPosition()));
  });
  const visiblePixels = createMemo(() => visibleAnnotations().reduce((sum, entry) => sum + entry.reference.width * entry.reference.height, 0));
  const imageIdentity = (id: string, sha: string) => `${annotationGeneration}:${id}:${sha}`;
  const rasterLoading = () => annotationState().loading || visibleAnnotations().some(entry => !annotationState().images?.has(entry.annotationId) || !decodedImages().has(imageIdentity(entry.annotationId, entry.reference.pngSha256)));
  const annotationUnavailable = () => rasterLoading() || Boolean(annotationState().error) || visiblePixels() > 32 * 1024 * 1024;
  const manual = new VideoManualEditor({
    authority: () => ({ key: annotationKey(), target: videoAnnotationTarget(props.sceneId, props.item), item: props.item, editable: !disposed && videoNative && props.active && !props.busy && !props.exporting && !props.annotationBusy && !annotationEditing() && !!props.onAnnotationEdit && !!props.onAnnotationTextEdit && !annotationUnavailable() }),
    move: (target, action) => props.onAnnotationEdit!(target, action),
    read: (target, id, reference) => props.lane.request('text-read', requestId => getVideoAnnotationText(requestId, target, id, reference)),
    edit: draft => props.lane.request('text-edit', requestId => props.onAnnotationTextEdit!(requestId, draft.target, draft.annotationId, draft.reference, draft.content)),
    pause: () => { props.onPauseAll?.(); pausePlayback(); }, publish: setManualView,
  });
  const stopManualFlush = props.onRegister(async active => { stopAnnotationPointer?.(); pausePlayback(); await manual.flush(active); });
  const annotationDisabled = () => props.busy || props.annotationBusy || drawingOpen() || annotationEditing() || manualView().pending || manualView().reading || Boolean(manualView().text) || !props.onAnnotationEdit;
  createEffect(on(() => [annotationKey(), props.active, props.busy, props.annotationBusy, Boolean(props.exporting), props.item.x, props.item.y, props.item.width, props.item.height] as const, () => { stopAnnotationPointer?.(); manual.cancelMove(); manual.reconcile(); }));
  createEffect(() => {
    const state = annotationState();
    previewHold.update({ source: annotationKey(), intent: playback?.playIntent(), loading: rasterLoading(), failed: Boolean(state.error) || visiblePixels() > 32 * 1024 * 1024, playing: playing(), allowed: !disposed && props.active && !props.busy && !props.exporting && !props.annotationBusy }, () => playback?.holdForRaster(), () => playback?.resumeCurrent() ?? Promise.resolve(), report);
    if (state.error || visiblePixels() > 32 * 1024 * 1024) pausePlayback();
  });
  const ownedExport = () => props.exporting?.sceneId === props.sceneId && props.exporting.itemId === props.item.id ? props.exporting : undefined;
  function place() {
    if (!video || disposed) return;
    const box = video.getBoundingClientRect(), card = video.closest('.artifact-card');
    const editorBox = { x: box.left, y: box.top, width: box.width, height: box.height };
    setDrawingBox(previous => Object.keys(editorBox).every(key => previous[key as keyof typeof previous] === editorBox[key as keyof typeof editorBox]) ? previous : editorBox);
    const header = card?.querySelector('.artifact-header')?.getBoundingClientRect(), footer = card?.querySelector('.artifact-footer')?.getBoundingClientRect();
    const controls = props.blackboard ? document.querySelector('.drawing-blackboard .drawing-toolbar')?.getBoundingClientRect() : header && footer ? { left: Math.min(header.left, footer.left), top: Math.min(header.top, footer.top), width: Math.max(header.right, footer.right) - Math.min(header.left, footer.left), height: Math.max(header.bottom, footer.bottom) - Math.min(header.top, footer.top) } : header ?? footer;
    const composer = document.querySelector('.composer:not(.composer-hidden)')?.getBoundingClientRect();
    const next = placeVideoControls(box, { width: window.innerWidth, height: window.innerHeight }, popover?.getBoundingClientRect().height ?? 0, composer, controls);
    setPlacement(previous => previous.left === next.left && previous.top === next.top && previous.width === next.width ? previous : next);
    const draft = manualView().text, object = props.item.videoAnnotations?.objects.find(value => value.id === draft?.annotationId);
    if (object?.primitive.kind === 'text') {
      const editorBox = textEditor?.getBoundingClientRect();
      const width = editorBox?.width ?? Math.min(300, Math.max(0, window.innerWidth - 16)), height = editorBox?.height ?? 0;
      setTextPlacement({ left: Math.max(8, Math.min(window.innerWidth - 8 - width, box.left + object.primitive.topLeft.x / props.item.videoAnnotations!.sourceWidth * box.width)), top: Math.max(8, Math.min(window.innerHeight - 8 - height, box.top + object.primitive.topLeft.y / props.item.videoAnnotations!.sourceHeight * box.height + 12)) });
    }
  }
  function observePopover(node: HTMLDivElement) {
    popoverResize?.disconnect(); popover = node;
    popoverResize = new ResizeObserver(place); popoverResize.observe(node);
    queueMicrotask(place);
  }
  function observeTextEditor(node: HTMLDivElement) { textResize?.disconnect(); textEditor = node; textResize = new ResizeObserver(place); textResize.observe(node); queueMicrotask(place); }
  createEffect(() => { if (!props.active || !manualView().text) { textResize?.disconnect(); textResize = undefined; textEditor = undefined; } });
  createEffect(on(() => props.active, active => { if (!active) { popoverResize?.disconnect(); popoverResize = undefined; popover = undefined; } }));
  createEffect(on(sourceKey, () => { playback?.invalidate(); setMetadata(metadataCache.get(sourceKey())); setFailed(false); setReadError(''); setMediaDuration(0); setPosition(0); }));
  createEffect(on(() => [sourceKey(), props.active && !props.busy && !props.exporting, retry()] as const, ([key, active]) => {
    if (!active || !videoNative || metadataCache.has(key)) return;
    let valid = true; setReading(true); setReadError(''); const requestTarget = target();
    const request = props.lane.request('info', id => getVideoInfo(id, requestTarget));
    void request.promise.then(value => {
      if (!valid || disposed || sourceKey() !== key) return;
      metadataCache.delete(key); metadataCache.set(key, value.metadata); while (metadataCache.size > 8) metadataCache.delete(metadataCache.keys().next().value!);
      setMetadata(value.metadata);
    }, error => { if (valid && !disposed && !(error instanceof TrimCanceled)) setReadError(error instanceof Error ? error.message : String(error)); }).finally(() => { if (valid && !disposed) setReading(false); });
    onCleanup(() => { valid = false; setReading(false); void request.cancel().catch(report); });
  }));
  const playbackScope = createMemo(() => JSON.stringify([annotationKey(), props.active, props.busy, Boolean(props.exporting), Boolean(props.annotationBusy)]));
  createEffect(on(playbackScope, () => {
    pausePlayback();
    const saved = range(), actual = position();
    // An acknowledged boundary seek can still be decoding. Do not overwrite it
    // with the last presented position when that position is already in range.
    if (saved && clampPosition(actual, saved) !== actual) playback?.seek(clampPosition(actual, saved));
  }));
  createEffect(on(() => [props.item.x, props.item.y, props.item.width, props.item.height, props.active, metadata()] as const, () => queueMicrotask(place)));
  onMount(() => {
    playback = new VideoPlayback(video, { identity: sourceKey, range, position: setPosition, playing: setPlaying, observation: value => { setAnnotationPosition(value.sourcePlaybackTicks); annotationReader.at(value.sourcePlaybackTicks); }, error: report });
    const unregister = props.onRegisterPause?.(() => { stopAnnotationPointer?.(); manual.interrupt(); pausePlayback(); });
    onCleanup(() => unregister?.());
    const resize = new ResizeObserver(place); resize.observe(video);
    const mutation = new MutationObserver(place); const composer = document.querySelector('.composer'); if (composer) { mutation.observe(composer, { attributes: true, attributeFilter: ['style', 'class'] }); resize.observe(composer); }
    window.addEventListener('resize', place);
    const blur = () => { stopAnnotationPointer?.(); manual.interrupt(); pausePlayback(); }; window.addEventListener('blur', blur);
    const visible = () => { if (document.hidden) { stopAnnotationPointer?.(); manual.interrupt(); previewHold.invalidate(); playback?.invalidate(); } }; document.addEventListener('visibilitychange', visible);
    const escapeMove = (event: KeyboardEvent) => { if (nativeSelectOwnsEscape(event)) return; if (event.key === 'Escape' && !event.isComposing && event.keyCode !== 229 && manualView().move && !(event.target as Element)?.closest('.video-annotation-text-editor')) { event.preventDefault(); event.stopImmediatePropagation(); stopAnnotationPointer?.(); manual.cancelMove(); } }; window.addEventListener('keydown', escapeMove, true);
    onCleanup(() => { resize.disconnect(); mutation.disconnect(); window.removeEventListener('resize', place); window.removeEventListener('blur', blur); document.removeEventListener('visibilitychange', visible); window.removeEventListener('keydown', escapeMove, true); });
  });
  createEffect(on(sourceKey, key => {
    const unregister = props.onRegisterPlayer?.(props.sceneId, props.item.id, { sourceIdentity: key, jump: async (action, active) => {
      const document = annotationKey();
      let previewSha: string | undefined;
      const current = () => !disposed && active() && props.active && !props.busy && !props.exporting && !props.annotationBusy && sourceKey() === key && annotationKey() === document && (!previewSha || annotationState().plan?.documentSha256 === previewSha) && sameVideoAnnotationTarget(videoAnnotationTarget(props.sceneId, props.item), action.target);
      const object = props.item.videoAnnotations?.objects.find(value => value.id === action.annotationId);
      if (!playback || !current() || !object || object.origin?.runId !== action.runId || object.origin.toolEventId !== action.toolEventId) throw new TrimCanceled();
      const saved = range(), interval = saved && intersectVideoInterval(object.interval, saved);
      if (!interval || interval.startTicks !== action.interval.startTicks || interval.endTicks !== action.interval.endTicks) throw new TrimCanceled();
      pausePlayback();
      setDrawingOpen(false);
      setSelectedAnnotation(action.annotationId);
      await playback.playInterval(interval, current, () => {
        const plan = annotationState().plan;
        if (!plan || !sameVideoAnnotationTarget(plan.target, action.target) || annotationUnavailable()) return false;
        previewSha ??= plan.documentSha256;
        return plan.documentSha256 === previewSha;
      });
    } });
    onCleanup(() => unregister?.());
  }));
  createEffect(() => { manualView().text; queueMicrotask(place); });
  onCleanup(() => { disposed = true; stopAnnotationPointer?.(); stopManualFlush(); manual.dispose(); previewHold.invalidate(); annotationReader.dispose(); popoverResize?.disconnect(); textResize?.disconnect(); playback?.dispose(); });
  const displayedBounds = (entry: { annotationId: string; bounds: { x: number; y: number; width: number; height: number } }) => {
    const movement = manualView().move;
    if (movement?.annotationId !== entry.annotationId) return entry.bounds;
    const primitive = props.item.videoAnnotations?.objects.find(value => value.id === entry.annotationId)?.primitive;
    return primitive?.kind === 'vector' ? { ...entry.bounds, x: entry.bounds.x + movement.bounds.x - primitive.topLeft.x, y: entry.bounds.y + movement.bounds.y - primitive.topLeft.y } : movement.bounds;
  };
  function annotationPointer(event: PointerEvent, annotationId: string) {
    if (event.button !== 0 || props.busy) return;
    props.onActivate?.();
    event.preventDefault(); event.stopPropagation(); pausePlayback(); setSelectedAnnotation(annotationId); video.focus({ preventScroll: true });
    const target = event.currentTarget as SVGRectElement, layer = target.ownerSVGElement!, box = layer.getBoundingClientRect(), doc = props.item.videoAnnotations!;
    const sample = (value: PointerEvent) => videoPoint({ x: value.clientX, y: value.clientY }, box, { width: doc.sourceWidth, height: doc.sourceHeight });
    if (!manual.begin(annotationId, sample(event))) return;
    const id = event.pointerId;
    const release = () => { target.removeEventListener('pointermove', moved); target.removeEventListener('pointerup', ended); target.removeEventListener('pointercancel', canceled); target.removeEventListener('lostpointercapture', canceled); if (stopAnnotationPointer === release) stopAnnotationPointer = undefined; if (target.hasPointerCapture(id)) target.releasePointerCapture(id); };
    const moved = (next: PointerEvent) => { if (next.pointerId === id) { next.preventDefault(); next.stopPropagation(); manual.move(sample(next)); } };
    const ended = (next: PointerEvent) => { if (next.pointerId !== id) return; moved(next); release(); void manual.completeMove().catch(report); };
    const canceled = (next: PointerEvent) => { if (next.pointerId !== id) return; release(); manual.cancelMove(); };
    stopAnnotationPointer = release;
    target.addEventListener('pointermove', moved); target.addEventListener('pointerup', ended); target.addEventListener('pointercancel', canceled); target.addEventListener('lostpointercapture', canceled);
    try { target.setPointerCapture(id); } catch { release(); manual.cancelMove(); }
  }
  async function editAnnotation(action: VideoAnnotationAction) {
    if (annotationDisabled()) return;
    pausePlayback(); setAnnotationEditing(true);
    try { await props.onAnnotationEdit!(videoAnnotationTarget(props.sceneId, props.item), action); }
    catch (error) { report(error); }
    finally { if (!disposed) setAnnotationEditing(false); }
  }
  function replayAnnotations(direction: 'undo' | 'redo') {
    const entry = props.item.videoAnnotations?.[direction].at(-1);
    if (entry) void editAnnotation({ type: direction, operationId: entry.id });
  }
  async function commit(edit: TrimEdit): Promise<TrimReceipt> {
    const grant = props.grant, current = authority();
    if (!grant || !current || current.identity !== edit.identity || current.revision !== edit.expectedRevision || !sameRange(current.range, edit.from)) throw new TrimCanceled();
    const result = await props.onEdit(grant, target(), { type: 'set', from: edit.from, to: edit.to });
    if (!result.videoEdit || result.asset.id !== props.item.asset.id) throw new Error('视频范围回执不一致');
    return { identity: edit.identity, durationTicks: result.videoEdit.sourceDurationTicks, revision: result.videoEdit.revision, range: result.videoEdit.range };
  }
  async function replay(direction: 'undo' | 'redo') {
    const grant = props.grant, edit = props.item.videoEdit, head = edit?.[direction].at(-1);
    if (!grant || !edit || !head) return;
    pausePlayback(); await props.onEdit(grant, target(), { type: direction, from: edit.range, operationId: head.id });
  }
  const drawingAvailable = () => videoNative && !!metadata() && !!props.drawing && (Boolean(props.drawingGrant) || Boolean(props.item.videoAnnotations));
  async function openDrawing() {
    if (!drawingAvailable() || props.busy || props.annotationBusy || props.drawingAllowed === false || props.inputLocked) return;
    const identity = sourceKey(); props.onPauseAll?.(); pausePlayback();
    try {
      await manual.flush(() => !disposed && identity === sourceKey());
      if (disposed || !props.active || props.busy || props.inputLocked || identity !== sourceKey()) return;
      setDrawingOpen(true); queueMicrotask(place);
    } catch (error) { report(error); }
  }
  function renderStoredDrawing(id: string, interactive: boolean, selected: boolean, preview?: Drawing | EditorBox) {
    const entry = visibleAnnotations().find(value => value.annotationId === id), object = props.item.videoAnnotations?.objects.find(value => value.id === id);
    const image = entry && annotationState().images?.get(id);
    if (!entry || !object || !image) return null;
    if (preview && 'kind' in preview && object.primitive.kind === 'vector') return <DrawingShape drawing={preview} />;
    let x = entry.bounds.x, y = entry.bounds.y;
    if (preview && !('kind' in preview)) {
      const origin = object.primitive.kind === 'rect' ? object.primitive.bounds : object.primitive.topLeft;
      x += preview.x - origin.x; y += preview.y - origin.y;
    } else if (preview && object.primitive.kind === 'text') { x += preview.points[0].x - object.primitive.topLeft.x; y += preview.points[0].y - object.primitive.topLeft.y; }
    return <g data-drawing-id={interactive ? id : undefined}>
      <image x={x} y={y} width={entry.bounds.width} height={entry.bounds.height} preserveAspectRatio="none" href={image} pointer-events="none" />
      <Show when={interactive}><rect x={x} y={y} width={entry.bounds.width} height={entry.bounds.height} fill="transparent" stroke={selected ? 'var(--accent, #2563eb)' : 'transparent'} stroke-width="1.5" stroke-dasharray="4 3" vector-effect="non-scaling-stroke" pointer-events="all" /></Show>
    </g>;
  }
  const toggle = () => { if (playback?.hasIntervalIntent()) { pausePlayback(); return; } previewHold.invalidate(); if (props.busy || drawingOpen() || manualView().text || manualView().pending || manualView().reading || annotationUnavailable() || !range()) return; if (playing()) pausePlayback(); else void playback?.play().catch(report); };
  return <>
    <video ref={video} class="artifact-video" src={assetUrl(props.item.asset)} preload="metadata" playsinline tabIndex={0} aria-label={props.item.asset.name} onContextMenu={event => { event.preventDefault(); event.stopPropagation(); }} onClick={() => { video.focus(); toggle(); }} on:keydown={event => { if (event.isComposing || event.ctrlKey || event.altKey || event.metaKey) return; if (event.key.toLowerCase() === 'd') { event.preventDefault(); event.stopPropagation(); void openDrawing(); } else if (event.key === ' ' || event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); if (event.key === ' ') toggle(); else pausePlayback(); } }} onError={() => setFailed(true)} onLoadedMetadata={() => { if (Number.isFinite(video.duration)) setMediaDuration(Math.round(video.duration * TICKS_PER_SECOND)); const saved = range(); if (saved) playback?.seek(saved.startTicks); }} />
    <Show when={!drawingOpen() && annotationState().plan && !annotationState().error && visiblePixels() <= 32 * 1024 * 1024}><svg class="video-annotation-layer" viewBox={`0 0 ${annotationState().plan!.sourceWidth} ${annotationState().plan!.sourceHeight}`} preserveAspectRatio="none" aria-label={t("视频标注")}>
      <For each={visibleAnnotations()}>{entry => <g>
        <Show when={annotationState().images?.get(entry.annotationId)} keyed>{dataUrl => {
          const identity = imageIdentity(entry.annotationId, entry.reference.pngSha256);
          onCleanup(() => { if (!disposed) setDecodedImages(previous => { const next = new Set(previous); next.delete(identity); return next; }); });
          return <image x={displayedBounds(entry).x} y={displayedBounds(entry).y} width={entry.bounds.width} height={entry.bounds.height} preserveAspectRatio="none" href={dataUrl} onLoad={() => {
            if (disposed || identity !== imageIdentity(entry.annotationId, entry.reference.pngSha256) || annotationState().images?.get(entry.annotationId) !== dataUrl) return;
            setDecodedImages(previous => new Set([...previous, identity]));
          }} onError={() => annotationReader.imageFailed(entry.annotationId, dataUrl)} />;
        }}</Show>
        <rect class="video-annotation-hit" classList={{ selected: selectedAnnotation() === entry.annotationId && props.active, moving: manualView().move?.annotationId === entry.annotationId }} x={displayedBounds(entry).x} y={displayedBounds(entry).y} width={entry.bounds.width} height={entry.bounds.height} vector-effect="non-scaling-stroke" onPointerDown={event => annotationPointer(event, entry.annotationId)} onDblClick={event => { event.preventDefault(); event.stopPropagation(); props.onActivate?.(); void manual.open(entry.annotationId).catch(report); }} />
      </g>}</For>
    </svg></Show>
    <Show when={failed()}><span class="video-playback-error inline-error">{t("无法播放视频")}</span></Show>
    <Show when={props.active && manualView().text}><Portal><div ref={observeTextEditor} class="video-annotation-text-editor" inert={props.busy || Boolean(props.annotationBusy) || Boolean(props.exporting)} style={{ left: `${textPlacement().left}px`, top: `${textPlacement().top}px` }} onPointerDown={event => event.stopPropagation()} onWheel={event => event.stopPropagation()} on:keydown={event => {
      event.stopPropagation(); if (event.isComposing || event.keyCode === 229) return;
      if (event.key === 'Escape') { event.preventDefault(); void manual.cancelText().catch(report); }
      else if (event.key === 'Enter' && !event.shiftKey && !event.ctrlKey && !event.altKey && !event.metaKey) { event.preventDefault(); void manual.save().catch(report); }
    }}><textarea aria-label={t("视频标注文字")} maxLength={1000} ref={node => queueMicrotask(() => { if (!disposed && node.isConnected) { node.focus(); node.select(); } })} value={manualView().text?.content.text ?? ''} disabled={manualView().pending} onInput={event => { try { manual.input(event.currentTarget.value); } catch (error) { report(error); } }} /><div class="video-annotation-text-actions"><Show when={manualView().text?.error}><span class="inline-error">{manualView().text?.error}</span></Show><Show when={manualView().pending}><LoaderCircle class="spin" size={14} /></Show><button class="icon-button compact" title={t("确认")} aria-label={t("保存视频标注文字")} disabled={manualView().pending} onClick={() => void manual.save().catch(report)}><Check size={15} /></button><button class="icon-button compact" title={t("取消")} aria-label={t("取消编辑视频文字")} onClick={() => void manual.cancelText().catch(report)}><X size={15} /></button></div></div></Portal></Show>
    <Show when={props.active}><Portal><div ref={observePopover} class="video-trim-popover" data-video-item={props.item.id} data-video-scene={props.sceneId} style={{ left: `${placement().left}px`, top: `${placement().top}px`, width: `${placement().width}px` }} onPointerDown={event => event.stopPropagation()}>
      <Show when={authority()} fallback={<div class="video-trim-loading"><Show when={reading()} fallback={<><span>{readError() || t('无法读取视频')}</span><button class="icon-button compact" title={t("重试")} aria-label={t("重新读取视频")} onClick={() => setRetry(value => value + 1)}><RefreshCw size={15} /></button></>}><LoaderCircle class="spin" size={16} /></Show></div>}>
        <VideoTrimBar authority={authority()} positionTicks={position()} playing={playing()} busy={props.busy || drawingOpen() || Boolean(props.exporting) || manualView().pending || Boolean(manualView().text)} onPause={pausePlayback} onSeek={ticks => { pausePlayback(); playback?.seek(ticks); }} onResume={ticks => { previewHold.invalidate(); if (!annotationUnavailable()) void playback?.play(ticks).catch(report); }} onTogglePlayback={toggle} onCommit={commit} onError={report} canUndo={Boolean(props.item.videoEdit?.undo.length)} canRedo={Boolean(props.item.videoEdit?.redo.length)} onReplay={replay} onRegister={flush => props.onRegister(async active => { pausePlayback(); await flush(active); })} />
      </Show>
      <Show when={drawingAvailable() || props.item.videoAnnotations}><div class="video-annotation-tools" on:keydown={event => { if (nativeSelectOwnsEscape(event)) return; if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setSelectedAnnotation(undefined); video.focus(); } }}>
        <Show when={drawingAvailable()}><button class="icon-button compact" title={t("绘制 · D")} aria-label={t("绘制视频标注")} disabled={props.busy || props.annotationBusy || props.inputLocked || props.drawingAllowed === false || drawingOpen()} onClick={() => void openDrawing()}><Pencil size={15} /></button></Show>
        <button class="icon-button compact" title={t("撤销标注")} aria-label={t("撤销视频标注")} disabled={annotationDisabled() || !props.item.videoAnnotations?.undo.length} onClick={() => replayAnnotations('undo')}><Undo2 size={15} /></button>
        <button class="icon-button compact" title={t("重做标注")} aria-label={t("重做视频标注")} disabled={annotationDisabled() || !props.item.videoAnnotations?.redo.length} onClick={() => replayAnnotations('redo')}><Redo2 size={15} /></button>
        <button class="icon-button compact" title={t("删除标注")} aria-label={t("删除视频标注")} disabled={annotationDisabled() || !selectedAnnotation()} onClick={() => { const id = selectedAnnotation(); if (id) void editAnnotation({ type: 'remove', annotationId: id }); }}><Trash2 size={15} /></button>
        <Show when={annotationState().loading || annotationEditing()}><LoaderCircle class="spin" size={14} /></Show>
        <Show when={annotationState().error || visiblePixels() > 32 * 1024 * 1024}><span class="inline-error">{annotationState().error || t('视频标注预览过大')}</span><button class="icon-button compact" title={t("重试")} aria-label={t("重新读取视频标注")} disabled={props.busy} onClick={() => setAnnotationRetry(value => value + 1)}><RefreshCw size={14} /></button></Show>
      </div></Show>
      <Show when={ownedExport()}>{value => <div class="video-export-state"><LoaderCircle class="spin" size={13} /><span>{value().canceling ? t('取消中') : value().percent === undefined ? (value().kind === 'copy' ? t('复制中') : t('保存中')) : `${Math.round(value().percent!)}%`}</span><button class="icon-button compact" aria-label={value().kind === 'copy' ? t('取消复制视频') : t('取消保存视频')} title={t("取消")} disabled={value().canceling} onClick={props.onCancelExport}><X size={14} /></button></div>}</Show>
    </div></Portal></Show>
    <Show when={drawingOpen() && props.active && metadata() && props.drawing && drawingBox().width > 0}><Portal><VideoDrawingEditor {...props.drawing!} sceneId={props.sceneId} item={props.item} info={metadata()!} box={drawingBox()} grant={props.drawingGrant} busy={Boolean(props.authoringBusy || props.annotationBusy || props.exporting)} inputLocked={Boolean(props.inputLocked)} active={props.active} onPause={() => { props.onPauseAll?.(); pausePlayback(); }} onClose={() => setDrawingOpen(false)} onError={props.onError} renderStored={renderStoredDrawing} visibleAnnotationIds={visibleAnnotations().map(entry => entry.annotationId)} /></Portal></Show>
  </>;
}
