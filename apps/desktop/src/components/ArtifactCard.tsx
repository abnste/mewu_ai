// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { createEffect, createMemo, createResource, createSignal, For, onCleanup, Show, untrack } from 'solid-js';
import { Code2, Copy, Download, File, Image, Link2, Maximize2, Move, Pencil, RotateCcw, Video, X } from 'lucide-solid';
import type {ObjectTrashPort} from '../object-trash';
import type {DrawingFlush,RegisterDrawingFlush} from '../drawing-flush';
import {isBlackboardText} from '../blackboard-objects';
import {resizeBoardObject,type ResizeCorner} from '../blackboard-text';
import {moveImageObject,zoomImageObject} from '../image-object-geometry';
import BlackboardText from './BlackboardText';
import type { SpaceItem } from '../contracts';
import { assetUrl, documentUrl, isolatedDocument, native, readArtifact } from '../bridge';
import './video.css';
import TextArtifact from './TextArtifact';
import './image-object.css';
import VideoArtifact, { type VideoArtifactProps } from './VideoArtifact';

interface Props {
  item: SpaceItem;
  active?: boolean;
  onActivate?: () => void;
  referenced: boolean;
  onReference: () => void;
  onRemove: () => void | Promise<void>;
  onUpdate: (item: SpaceItem) => void | Promise<void>;
  onExport?: () => void;
  onCopy?: () => void;
  video?: Omit<VideoArtifactProps, 'item' | 'active'>;
  busy?: boolean;
  onOpenBlackboard?: () => void;
  blackboard?:boolean;interactive?:boolean;hovered?:boolean;trash?:ObjectTrashPort;onRegisterFlush?:RegisterDrawingFlush;
  boardSceneId?:string;onError?:(message:string)=>void;
  onRegisterWheel?:(wheel:(event:WheelEvent)=>void)=>()=>void;
}

export default function ArtifactCard(props: Props) {
  const [position, setPosition] = createSignal({ x: props.item.x, y: props.item.y, width: props.item.width, height: props.item.height });
  const [failed, setFailed] = createSignal(false);
  createEffect(() => { props.item.asset.id; setFailed(false); });
  const [reload, setReload] = createSignal(0);
  const [moving, setMoving] = createSignal(false);
  const [viewport, setViewport] = createSignal({ width: innerWidth, height: innerHeight });
  const resized = () => setViewport({ width: innerWidth, height: innerHeight });
  window.addEventListener('resize', resized);
  let shell!: HTMLElement;
  let gestureToken = 0;
  let disposed = false;
  let disposeGesture: (() => void) | undefined;
  let gestureFlight:Promise<void>|undefined;
  let zoomTimer:ReturnType<typeof setTimeout>|undefined,zoomAsset:string|undefined;
  let textFlush:DrawingFlush|undefined;
  const unregisterFlush=props.onRegisterFlush?.(async active=>{disposeGesture?.();await flushZoom();if(gestureFlight)await gestureFlight;if(textFlush)await textFlush(active);if(!active())throw Error('素材编辑已切换');});
  const unregisterWheel=props.onRegisterWheel?.(wheel);
  const activate = () => props.onActivate?.();
  // An isolated iframe does not bubble its pointer events into this document.
  // Its focus can still select this card without accessing the child document.
  const frameFocused = () => queueMicrotask(() => {
    if (!disposed && document.activeElement instanceof HTMLIFrameElement && shell?.contains(document.activeElement)) activate();
  });
  window.addEventListener('blur', frameFocused);
  onCleanup(() => {
    disposed = true;
    window.removeEventListener('resize', resized);
    window.removeEventListener('blur', frameFocused);
    shell?.removeEventListener('pointerdown', activate, true);
    shell?.removeEventListener('focusin', activate);
    gestureToken += 1;
    disposeGesture?.();
    clearTimeout(zoomTimer);
    unregisterFlush?.();unregisterWheel?.();
  });
  const savedPosition = () => ({ x: props.item.x, y: props.item.y, width: props.item.width, height: props.item.height });
  const persistedPosition = createMemo(() => `${props.item.x}:${props.item.y}:${props.item.width}:${props.item.height}`);
  createEffect(() => {
    persistedPosition();
    if (!untrack(moving)) setPosition(untrack(savedPosition));
  });
  const isDocument = () => props.item.asset.kind === 'html' || props.item.asset.kind === 'svg';
  const isVideo = () => props.item.asset.kind === 'video';
  const isImage = () => props.item.asset.kind === 'image';
  const imageBox = createMemo(() => {
    const p = position(), vp = viewport(), ratio = (props.item.asset.width ?? 1) / (props.item.asset.height ?? 1);
    const width = Math.min(p.width * vp.width, p.height * vp.height * ratio);
    return { width, height: width / ratio };
  });
  const controlPosition=()=>{const p=position(),vp=viewport(),left=p.x*vp.width,top=p.y*vp.height,width=isImage()?imageBox().width:p.width*vp.width;return{right:'auto',left:`${Math.max(4,Math.min(vp.width-32,left+width-32))-left}px`,top:`${Math.max(4,Math.min(vp.height-32,top+4))-top}px`};};
  const [source] = createResource(() => isDocument() && !native ? `${props.item.asset.id}:${reload()}` : false, () => readArtifact(props.item.asset.id));

  async function flushZoom() {
    clearTimeout(zoomTimer);zoomTimer=undefined;
    if(gestureFlight)await gestureFlight;
    if(!zoomAsset)return;
    const assetId=zoomAsset;zoomAsset=undefined;
    if(disposed||props.item.asset.id!==assetId){setMoving(false);return;}
    const request=Promise.resolve().then(()=>props.onUpdate({...props.item,...position()}));gestureFlight=request;
    try {await request;} catch(error){props.onError?.(error instanceof Error?error.message:String(error));throw error;}
    finally{if(gestureFlight===request)gestureFlight=undefined;if(!disposed){setPosition(savedPosition());setMoving(false);}}
  }
  function wheel(event:WheelEvent) {
    if(!isImage()||props.busy||gestureFlight||disposeGesture||disposed)return;
    const p={...position(),width:imageBox().width/innerWidth,height:imageBox().height/innerHeight};
    const delta=event.deltaY*(event.deltaMode===1?16:event.deltaMode===2?innerHeight:1);
    const next=zoomImageObject(p,{x:event.clientX/innerWidth,y:event.clientY/innerHeight},delta,innerWidth,innerHeight);
    if(!next)return;event.preventDefault();event.stopPropagation();
    zoomAsset=props.item.asset.id;setMoving(true);setPosition(next);clearTimeout(zoomTimer);
    zoomTimer=setTimeout(()=>void flushZoom().catch(()=>{}),140);
  }

  const gesture = (event: PointerEvent, resize = false, control = false, corner?:ResizeCorner) => {
    if (props.busy || gestureFlight || zoomAsset || event.button !== 0 || (!resize && !control && (event.target as HTMLElement).closest('button'))) return;
    event.preventDefault(); event.stopPropagation();
    disposeGesture?.();
    const token = ++gestureToken;
    const target = event.currentTarget as HTMLElement;
    const start = isImage() ? { ...position(), width: imageBox().width / innerWidth, height: imageBox().height / innerHeight } : position();
    const startX = event.clientX, startY = event.clientY;
    const vw = window.innerWidth, vh = window.innerHeight;
    const videoMode = isVideo();
    let changed = false;
    if(props.blackboard&&!resize)props.trash?.begin();
    try{target.setPointerCapture(event.pointerId);}catch{props.trash?.cancel();return;}
    const move = (e: PointerEvent) => {
      if (!changed && Math.hypot(e.clientX - startX, e.clientY - startY) < 3) return;
      changed = true;
      setMoving(true);
      if(props.blackboard&&!resize)props.trash?.move({x:e.clientX,y:e.clientY});
      const dx = (e.clientX - startX) / vw, dy = (e.clientY - startY) / vh;
      if(!resize&&(isImage()||props.blackboard)){setPosition(moveImageObject(start,dx,dy,vw,vh));return;}
      if(resize&&corner){setPosition(resizeBoardObject(start,dx,dy,corner,vw,vh));return;}
      const minWidth = videoMode ? Math.min(64 / vw, 1 - start.x) : Math.min(240 / vw, .85);
      const minHeight = videoMode ? Math.min(48 / vh, 1 - start.y) : Math.min(160 / vh, .8);
      if (resize && isImage()) {
        const ratio = (props.item.asset.width ?? 1) / (props.item.asset.height ?? 1);
        const width = Math.min((1 - start.x) * vw, (1 - start.y) * vh * ratio, Math.max(48, start.width * vw + e.clientX - startX));
        setPosition({ ...start, width: width / vw, height: width / ratio / vh });
        return;
      }
      setPosition(resize ? {
        ...start,
        width: Math.max(minWidth, Math.min(1 - start.x - (videoMode ? 0 : .008), start.width + dx)),
        height: Math.max(minHeight, Math.min((videoMode ? 1 : .98) - start.y, start.height + dy)),
      } : {
        ...start,
        x: Math.max(videoMode ? 0 : .008, Math.min(1 - start.width - (videoMode ? 0 : .008), start.x + dx)),
        y: Math.max(videoMode ? 0 : .04, Math.min((videoMode ? 1 : .98) - start.height, start.y + dy)),
      });
    };
    const cleanup = () => {
      target.removeEventListener('pointermove', move);
      target.removeEventListener('pointerup', end);
      target.removeEventListener('pointercancel', end);
      target.removeEventListener('lostpointercapture', canceled);
      if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
      if (disposeGesture === canceled) disposeGesture = undefined;
    };
    const end = async (e: PointerEvent) => {
      cleanup();
      if (disposed || token !== gestureToken) return;
      const remove=Boolean(props.blackboard&&!resize&&changed&&e.type==='pointerup'&&props.trash?.finish({x:e.clientX,y:e.clientY}));props.trash?.cancel();
      if (changed && e.type !== 'pointercancel') {
        const request=Promise.resolve().then(()=>remove?props.onRemove():props.onUpdate({ ...props.item, ...position() }));
        // Removal owns the app's full flush; registering it here would wait on itself.
        if(!remove)gestureFlight=request;
        try { await request; }
        catch { /* The caller presents the error; the saved geometry remains authoritative. */ }
        finally{if(gestureFlight===request)gestureFlight=undefined;}
      }
      if (disposed || token !== gestureToken) return;
      // Clamping can commit the same geometry, leaving persistedPosition's memo
      // unchanged. Always reconcile after the commit, including rejected writes.
      setPosition(savedPosition());
      setMoving(false);
    };
    const canceled=()=>{cleanup();props.trash?.cancel();setPosition(savedPosition());setMoving(false);};
    disposeGesture = canceled;
    target.addEventListener('pointermove', move);
    target.addEventListener('pointerup', end);
    target.addEventListener('pointercancel', end);
    target.addEventListener('lostpointercapture', canceled);
  };

  return <section ref={node => { shell = node; node.addEventListener('pointerdown', activate, true); node.addEventListener('focusin', activate); }} class="artifact-card" classList={{ active: props.active, moving: moving(), 'video-artifact': props.item.asset.kind === 'video', 'image-object': isImage(), 'document-object':isDocument(), 'blackboard-object':props.blackboard,'blackboard-interactive':props.interactive,'blackboard-object-hovered':props.hovered }} tabindex={isImage() ? 0 : undefined}
    style={{ left: `${position().x * 100}%`, top: `${position().y * 100}%`, width: isImage() ? `${imageBox().width}px` : `${position().width * 100}%`, height: isImage() ? `${imageBox().height}px` : `${position().height * 100}%`, 'z-index': props.active ? 3 : undefined }}
    data-video-item={isVideo() ? props.item.id : undefined} data-video-scene={isVideo() ? props.video?.sceneId : undefined}
    aria-label={props.item.asset.name} onWheel={wheel} onPointerDown={e => { e.stopPropagation(); if (isImage()&&!props.blackboard) gesture(e); }} onDblClick={event => { if (isImage() && props.onOpenBlackboard && !props.busy && !(event.target as Element).closest('button')) {event.preventDefault();event.stopPropagation();props.onOpenBlackboard();} }}>
    <Show when={props.blackboard}><div class="blackboard-object-control" role="toolbar" aria-label={t('对象工具')} style={controlPosition()}><button aria-label={t('移动对象')} title={t('移动对象')} disabled={props.busy} onPointerDown={event=>gesture(event,false,true)}><Move size={18} strokeWidth={1.7}/></button></div></Show>
    <Show when={props.blackboard&&isBlackboardText(props.item.asset)}><For each={['nw','ne','sw','se'] as ResizeCorner[]}>{corner=><div class={`blackboard-text-corner ${corner}`} aria-hidden="true" onPointerDown={event=>gesture(event,true,true,corner)} />}</For></Show>
    <Show when={!isImage()&&!props.blackboard}>
    <header class="artifact-header" onPointerDown={e => gesture(e)}>
      <Show when={isDocument()} fallback={<Show when={props.item.asset.kind === 'image'} fallback={<Show when={props.item.asset.kind === 'video'} fallback={<File size={14} />}><Video size={14} /></Show>}><Image size={14} /></Show>}><Code2 size={14} /></Show>
      <span class="artifact-title" title={props.item.asset.name}>{props.item.asset.name}</span>
      <Show when={props.onOpenBlackboard}><button class="icon-button compact" title={t("编辑黑板")} aria-label={t("编辑黑板")} disabled={props.busy} onClick={() => props.onOpenBlackboard?.()}><Pencil size={13} /></button></Show>
      <Show when={isDocument()}><button class="icon-button compact" title={t("重新载入")} aria-label={t("重新载入内容")} onClick={() => setReload(value => value + 1)}><RotateCcw size={13} /></button></Show>
      <button class="icon-button compact" title={t("移除")} aria-label={t("移除 {0}").replaceAll("{0}", () => String(props.item.asset.name))} disabled={props.busy} onClick={props.onRemove}><X size={14} /></button>
    </header>
    </Show>
    <Show when={isImage()&&!props.blackboard}><div class="image-object-tools" role="toolbar" aria-label={t("图片工具")} onPointerDown={event => event.stopPropagation()}>
      <button class="icon-button compact" classList={{ selected: props.referenced }} title={props.referenced ? t('取消引用') : t('引用')} aria-label={props.referenced ? t('取消引用') : t('引用')} disabled={props.busy} onClick={props.onReference}><Link2 size={15} /></button>
      <Show when={props.onOpenBlackboard}><button class="icon-button compact" title={t('编辑黑板')} aria-label={t('编辑黑板')} disabled={props.busy} onClick={() => props.onOpenBlackboard?.()}><Pencil size={15} /></button></Show>
      <button class="icon-button compact" title={t('调整尺寸')} aria-label={t('调整对象尺寸')} disabled={props.busy} onPointerDown={event => gesture(event, true)}><Maximize2 size={15} /></button>
      <button class="icon-button compact" title={t('移除')} aria-label={t('移除 {0}').replaceAll('{0}', () => props.item.asset.name)} disabled={props.busy} onClick={props.onRemove}><X size={15} /></button>
    </div></Show>
    <div class="artifact-content">
      <Show when={props.item.asset.kind === 'image'}>
        <Show when={!failed()} fallback={<span class="inline-error">{t("无法打开图片")}</span>}>
          <img src={assetUrl(props.item.asset)} alt={props.item.asset.name} draggable={false} onError={() => setFailed(true)} />
        </Show>
      </Show>
      <Show when={props.item.asset.kind === 'video'}>
        <Show when={props.video}>{value => <VideoArtifact {...value()} item={props.item} blackboard={props.blackboard} active={Boolean(props.active && (!props.blackboard || props.interactive && !moving()))} onActivate={props.onActivate} />}</Show>
      </Show>
      <Show when={isDocument()}>
        <Show when={native} fallback={<Show when={!source.error} fallback={<span class="inline-error">{t("无法读取内容")}</span>}>
          <Show when={source()} fallback={<span class="quiet-state">{t("载入中")}</span>}>
            <iframe title={props.item.asset.name} srcdoc={isolatedDocument(source()!)} sandbox="allow-scripts" referrerpolicy="no-referrer" allow="" />
          </Show>
        </Show>}>
          <Show when={reload() + 1} keyed>{_revision => <iframe title={props.item.asset.name} src={documentUrl(props.item.asset)} sandbox="allow-scripts" referrerpolicy="no-referrer" allow="" />}</Show>
        </Show>
      </Show>
      <Show when={props.item.asset.kind === 'file'&&!props.blackboard}><div class="file-object"><File size={30} /><span>{props.item.asset.name}</span></div></Show>
      <Show when={props.item.asset.kind === 'text'}><Show when={props.blackboard&&isBlackboardText(props.item.asset)&&props.boardSceneId} fallback={<Show when={!props.blackboard}><TextArtifact assetId={props.item.asset.id} name={props.item.asset.name} /></Show>}><BlackboardText sceneId={props.boardSceneId!} itemId={props.item.id} assetId={props.item.asset.id} name={props.item.asset.name} busy={Boolean(props.busy)} onFlush={flush=>{textFlush=flush;return()=>{if(textFlush===flush)textFlush=undefined;};}} onError={message=>props.onError?.(message)} /></Show></Show>
      <Show when={moving()}><div class="iframe-drag-cover" /></Show>
    </div>
    <Show when={!isImage()&&!props.blackboard}><footer class="artifact-footer">
      <button class="quiet-button" classList={{ selected: props.referenced }} onClick={props.onReference}><Link2 size={13} />{props.referenced ? t('已引用') : t('引用')}</button>
      <Show when={isVideo() && props.onCopy}><button class="quiet-button" title={t("复制视频文件 (C)")} aria-label={t("复制视频文件")} disabled={props.busy} onClick={() => props.onCopy?.()}><Copy size={13} />{t("复制")}</button></Show>
      <Show when={['video', 'text'].includes(props.item.asset.kind) && props.onExport}><button class="quiet-button" disabled={props.busy} onClick={() => props.onExport?.()}><Download size={13} />{t("保存")}</button></Show>
      <span class="artifact-kind">{props.item.asset.kind.toUpperCase()}</span>
      <button class="resize-handle" aria-label={t("调整对象尺寸")} title={t("调整尺寸")} onPointerDown={e => gesture(e, true)}><Maximize2 size={12} /></button>
    </footer></Show>
  </section>;
}
