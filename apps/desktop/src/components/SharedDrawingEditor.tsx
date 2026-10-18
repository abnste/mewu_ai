// SPDX-License-Identifier: MPL-2.0
import { t } from '../i18n';
import { nativeSelectOwnsEscape } from '../native-select-escape';
import { batch, createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { ArrowUpRight, BrushCleaning, Check, Circle, Copy, Eraser, Grid2X2, Hash, Highlighter, LoaderCircle, Minus, MousePointer2, Pencil, Pin, Redo2, RotateCcw, Scissors, Square, Trash2, Type, Undo2, X } from 'lucide-solid';
import type { Drawing, DrawingKind, DrawingPoint, DrawingTextStyle, ImageExportVariant, ManualDrawingKind } from '../contracts';
import { constrainedEnd, drawingBounds, drawingMoveDelta, drawingOrder, drawingResizeHandles, resizedDrawingPoints, validNextNumber } from "./drawing-geometry";
import { finishDrawing, outputDrawing } from "../capture-completion";
import { clipEraserSweep, eraserCursor, eraserHit, eraserStrokeHits, ObjectEraser, PixelEraser, type EraserMode } from "./drawing-eraser";
import { changeProperty, drawingDraftKey, drawingDrafts, drawingPropertyCommand, propertyDraft, settleDrawingEdit, strokeKinds, type DrawingPropertyDraft } from "./drawing-properties";
import { DrawingAddQueue, drawingFrame, drawingPointerSamples, liveDrawing } from './drawing-pointer';
import { DrawingHistoryLane, projectDrawingHistory, type DrawingHistoryState } from './drawing-history';
import type { DrawingFlush, RegisterDrawingFlush } from "../drawing-flush";
import type { DrawingTableFormat } from "../drawing-layout-preview";
import "./drawing.css";
import DrawingColorPicker from './DrawingColorPicker';
import BlackboardImageControls from './BlackboardImageControls';
import {blackboardImage} from '../blackboard-image';
import {objectTrash} from '../object-trash';
import DrawingWidthSlider from './DrawingWidthSlider';
import InlineDrawingText from './InlineDrawingText';
import DrawingTextStylePicker from './DrawingTextStylePicker';
import ImageExportButton from './ImageExportButton';
import {selectionBox,marqueeDrawings,movedDrawings,linkedRepairPreview} from './drawing-selection';

import type { SharedDrawingEditorProps as Props, EditorDrawingAction } from '../drawing-editor-port';
const labels: Record<ManualDrawingKind, string> = { pen: '画笔', line: '直线', arrow: '箭头', rect: '矩形', ellipse: '椭圆', text: '文字', highlighter: '荧光笔', mark: '重点高亮', number: '序号', mosaic: '马赛克' };
const MarkIcon = (props: { size: number }) => <svg width={props.size} height={props.size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="5" width="18" height="14" rx="3" fill="currentColor" fill-opacity=".16" /><path d="M8 15l8-6m-4 0h4v4" /></svg>;
const icons = { pen: Pencil, line: Minus, arrow: ArrowUpRight, rect: Square, ellipse: Circle, text: Type, highlighter: Highlighter, mark: MarkIcon, number: Hash, mosaic: Grid2X2 };
const clamp = (value: number, low: number, high: number) => Math.max(low, Math.min(high, value));
export default function SharedDrawingEditor(props: Props) {
  let openImageExport: (() => void) | undefined;
  let editorShell!:HTMLDivElement;
  const objectWheel=(event:WheelEvent)=>{if(props.onObjectWheel?.(event)){event.preventDefault();event.stopImmediatePropagation();}};
  onMount(()=>{editorShell.addEventListener('wheel',objectWheel,{capture:true,passive:false});});
  onCleanup(()=>editorShell.removeEventListener('wheel',objectWheel,true));
  let svg!: SVGSVGElement, toolbar!: HTMLDivElement, toolsRow!: HTMLDivElement, trashButton!:HTMLButtonElement, textarea: HTMLTextAreaElement | undefined;
  let cancelGesture: (() => void) | undefined, refreshConstraint: ((shift: boolean) => void) | undefined, disposed = false, serial = 0, numberPreference = 0;
  let flight: Promise<boolean> | undefined;
  let tableCopy: Promise<void> | undefined;
  let flushImages: DrawingFlush | undefined;
  const [imageZooming, setImageZooming] = createSignal(false);
  let eraseSource: (()=>boolean) | undefined;
  const [erasing,setErasing]=createSignal(false);
  const [eraserMode,setEraserMode]=createSignal<EraserMode>('pixel');
  const [pixelDraft,setPixelDraft]=createSignal<Map<string,Drawing[]>|undefined>();
  const pixelIndex=createMemo(()=>new Map([...(pixelDraft()?.values()??[])].flat().map(d=>[d.id,d])));
  const [wideBrush,setWideBrush]=createSignal(props.tools[0]==='highlighter');
  let eraseFlight: Promise<void> | undefined;
  const numberChanges = new Map<string, { previous: number; next: number; preference: number }>();
  type Tool = ManualDrawingKind | 'select' | 'eraser' | 'extract' | 'heal';
  const [tool, setTool] = createSignal<Tool>(props.tools[0] ?? 'select');
  const [rasterPreview, setRasterPreview] = createSignal<{mode:'extract'|'heal';points:DrawingPoint[];width:number}>();
  const [healWidth, setHealWidth] = createSignal(32);
  const [tableFormat, setTableFormat] = createSignal<DrawingTableFormat>('table');
  const [color, setColor] = createSignal(props.blackboard ? '#E8ECF4' : '#ee4848'), [width, setWidth] = createSignal(4), [fontSize, setFontSize] = createSignal(20);
  const [textStyle, setTextStyle] = createSignal<DrawingTextStyle>({});
  const firstNumber = Math.min(9999, Math.max(0, ...(props.port.drawings()).filter(value => value.kind === 'number').map(value => validNextNumber(value.text ?? '') ?? 0)) + 1);
  const [highlightWidth, setHighlightWidth] = createSignal(18), [nextNumber, setNextNumber] = createSignal(String(firstNumber)), [numberDiameter, setNumberDiameter] = createSignal(28);
  const [mosaicReady, setMosaicReady] = createSignal(false), [mosaicFailed, setMosaicFailed] = createSignal(false), [mosaicRetry, setMosaicRetry] = createSignal(0), [finishing, setFinishing] = createSignal(false);
  const [highlightReady, setHighlightReady] = createSignal(false), [highlightFailed, setHighlightFailed] = createSignal(false), [highlightRetry, setHighlightRetry] = createSignal(0);
  const [selected, setSelected] = createSignal(''), [draft, setDraft] = createSignal<Drawing>();
  const [selectedIds,setSelectedIds]=createSignal<string[]>([]),[marquee,setMarquee]=createSignal<import('../drawing-editor-port').EditorBox>(),[groupDraft,setGroupDraft]=createSignal<Drawing[]>([]);
  createEffect(on(selected,id=>{if(!id)setSelectedIds([]);else if(!selectedIds().includes(id))setSelectedIds([id]);}));
  const [storedDraft,setStoredDraft]=createSignal<{id:string;bounds:{x:number;y:number;width:number;height:number}}>();
  let selectionSerial=0; const [selectionReading,setSelectionReading]=createSignal(false);
  const [pending, setPending] = createSignal(false), [edit, setEdit] = createSignal<DrawingPropertyDraft>();
  const [queuedDrawings, setQueuedDrawings] = createSignal<Drawing[]>([]);
  const [recoveredDrawings, setRecoveredDrawings] = createSignal<Drawing[]>([]);
  const [strokeVersion, setStrokeVersion] = createSignal(0);
  let strokeQueue: DrawingAddQueue | undefined, editCacheKey: string | undefined;
  const [historyPreview, setHistoryPreview] = createSignal<Drawing[]>();
  const [historyPending, setHistoryPending] = createSignal(false);
  let historyLane: DrawingHistoryLane | undefined, historyPreparation: Promise<void> | undefined;
  let historyIntents: boolean[] = [], preparingHistory: DrawingHistoryState | undefined;
  const activeDraftId = createMemo(() => draft()?.id), activeEditId = createMemo(() => edit()?.drawing.id), storedPreviewId = createMemo(() => storedDraft()?.id);
  const textEditor = () => edit()?.mode === 'text' ? edit() : undefined;
  const cacheKey = () => props.port.draftKey();
  const [viewport, setViewport] = createSignal({ width: innerWidth, height: innerHeight });
  const [toolbarSize, setToolbarSize] = createSignal({ width: 680, height: 118 });
  const [toolsWidth, setToolsWidth] = createSignal(680);
  // Dock from the stable tool bubble, not the changing property bubble.
  const rightDocked = () => !props.blackboard && props.box.x + toolsWidth() + 6 > viewport().width;
  const [surfaceSize, setSurfaceSize] = createSignal({ width: props.box.width, height: props.box.height });
  const drawings = () => props.port.drawings();
  const visibleDrawings = () => historyPreview() ?? drawings();
  const drawingIndex = createMemo(() => new Map(visibleDrawings().map(value => [value.id, value])));
  const storedIndex = createMemo(() => new Map((props.port.stored?.() ?? []).map(value => [value.id, value])));
  const selectedDrawing = () => drawingIndex().get(selected());
  const propertyTarget = () => selectedIds().length>1?undefined:edit()?.drawing ?? (tool() === 'select' && selectedDrawing() && props.port.canStyle(selectedDrawing()!) ? selectedDrawing() : undefined);
  const propertyKind = () => propertyTarget()?.kind ?? tool();
  const propertyColor = () => propertyTarget()?.color ?? color();
  const brushWidth = () => tool()==='highlighter'||(tool()==='eraser'&&wideBrush()) ? highlightWidth() : width();
  const propertyWidth = () => propertyTarget()?.strokeWidth ?? brushWidth();
  const eraseWidth = () => wideBrush() ? highlightWidth() : width();
  const eraseCursor = () => eraserCursor(eraseWidth()*surfaceSize().width/props.port.frame().width,eraseWidth()*surfaceSize().height/props.port.frame().height);
  const propertySize = () => propertyTarget()?.fontSize ?? (tool() === 'number' ? numberDiameter() : fontSize());
  const propertyTextStyle = () => { const drawing = propertyTarget(); return drawing ? drawing.textStyle ?? {} : textStyle(); };
  const options = (defaults: number[], current: number) => [...new Set([...defaults, current])].sort((a, b) => a - b);
  const controlsLocked = () => props.busy || props.inputLocked || finishing() || imageZooming();
  const disabled = () => pending() || controlsLocked() || historyPending();
  const historyDisabled = () => pending() || controlsLocked();
  const [objectDragging,setObjectDragging]=createSignal(false),[trashOver,setTrashOver]=createSignal(false);
  const trash=objectTrash(()=>trashButton,()=>Boolean(disabled()||edit()),(dragging,over)=>{setObjectDragging(dragging);setTrashOver(over);});
  const unregisterTrash=props.blackboard?props.onRegisterObjectTrash?.(trash):undefined;
  onCleanup(()=>{unregisterTrash?.();trash.cancel();});
  const blockSize = () => props.port.mosaic?.blockSize() ?? 8;
  const editableIds = () => new Set([...drawings().filter(value => props.port.canSelect(value)).map(value => value.id),...(props.port.stored?.().map(value=>value.id)??[])]);
  const historyAvailable = (redo: boolean) => {
    historyPreview();
    return preparingHistory ? (redo ? preparingHistory.redo : preparingHistory.undo).length > 0 : historyLane?.pending() ? historyLane.available(redo) : props.port.historyAvailable(redo);
  };
  const previewDrawing = () => draft() ?? edit()?.drawing;
  const previews=createMemo(()=>new Map([...recoveredDrawings(),...queuedDrawings(),...groupDraft(),...(previewDrawing()?[previewDrawing()!]:[])].map(d=>[d.id,d])));
  const addedPreviewId = createMemo(() => {
    const preview = previewDrawing();
    return preview && !drawingIndex().has(preview.id) && !storedIndex().has(preview.id) && (preview.kind !== 'text' || preview.text?.trim()) ? preview.id : undefined;
  });
  const addedPreviewMosaic = createMemo(() => previewDrawing()?.kind === 'mosaic');
  const drawingIds = createMemo(() => {
    const ordered = drawingOrder(visibleDrawings().flatMap(value=>pixelDraft()?.get(value.id)??[value])).filter(value => !storedIndex().has(value.id));
    const ids = [...storedIndex().keys(), ...ordered.map(value => value.id)];
    const addedId = addedPreviewId();
    for (const value of [...recoveredDrawings(), ...(historyPreview() === undefined ? queuedDrawings() : [])]) if (!ids.includes(value.id)) ids.push(value.id);
    if (addedId) {
      if (addedPreviewMosaic()) ids.splice(storedIndex().size + ordered.filter(value => value.kind === 'mosaic').length, 0, addedId); else ids.push(addedId);
    }
    return ids;
  }, undefined, { equals: (a, b) => a.length === b.length && a.every((id, i) => id === b[i]) });
  const resizeDrawing = () => {
    if(selectedIds().length>1)return;
    if (props.documentOnly || tool() !== 'select' || disabled() || edit()) return;
    const drawing = previewDrawing()?.id === selected() ? previewDrawing() : selectedDrawing();
    return drawing && props.port.canStyle(drawing) ? drawing : undefined;
  };
  const resizeHandles = () => { const drawing = resizeDrawing(), size = surfaceSize(); return drawing && size.width > 0 && size.height > 0 ? drawingResizeHandles(drawing) : []; };
  const handleSize = () => ({ x: 10 * props.port.frame().width / surfaceSize().width, y: 10 * props.port.frame().height / surfaceSize().height });
  const strokeSource = () => JSON.stringify([cacheKey(), props.port.sourceIdentity(), props.port.sourceSize(), props.port.frame(), props.box]);
  const continuousStrokes = () => !props.documentOnly && !props.port.stored;
  const gestureIdentity = () => { strokeVersion(); return JSON.stringify([cacheKey(), props.port.sourceIdentity(), props.port.sourceSize(), strokeQueue?.revisionIdentity() ?? props.port.revision(), props.box]); };
  const target = (expectedRevision = props.port.revision()) => ({ expectedRevision });
  const position = createMemo(() => {
    const size = toolbarSize(), box = props.box, vp = viewport();
    if (props.blackboard) return { left: `${Math.max(6, (vp.width - size.width) / 2)}px`, top: `${Math.max(6, vp.height - size.height - 16)}px` };
    const x = rightDocked() ? clamp(box.x + box.width - size.width, 6, Math.max(6, vp.width - size.width - 6)) : clamp(box.x, 6, Math.max(6, vp.width - size.width - 6));
    const y = box.y >= size.height + 14 ? box.y - size.height - 8 : box.y + box.height + size.height + 14 <= vp.height ? box.y + box.height + 8 : 6;
    return { left: `${x}px`, top: `${y}px` };
  });
  const point = (event: PointerEvent, clip = true, box = svg.getBoundingClientRect(), frame = props.port.frame()): DrawingPoint => {
    const value = { x: frame.x + (event.clientX - box.left) / box.width * frame.width, y: frame.y + (event.clientY - box.top) / box.height * frame.height };
    return clip ? { x: clamp(value.x, frame.x, frame.x + frame.width), y: clamp(value.y, frame.y, frame.y + frame.height) } : value;
  };
  const report = (error: unknown) => props.onError(error instanceof Error ? error.message : String(error));
  function cachedStrokes() {
    const base = cacheKey(), source = strokeSource();
    return drawingDrafts.list().filter(entry => {
      try {
        const key = JSON.parse(entry.key);
        if (!(key.length === 5 && key[1] === 'queued-stroke' && key[2] === base && key[3] === source)) return false;
        const saved = drawings().find(value => value.id === entry.draft.drawing.id);
        if (saved && JSON.stringify(saved) === JSON.stringify(entry.draft.drawing)) { drawingDrafts.delete(entry.key, entry.draft); return false; }
        return true;
      } catch { return false; }
    });
  }
  function restoreStrokeDraft() {
    const entries = cachedStrokes();
    setRecoveredDrawings(entries.map(entry => entry.draft.drawing));
    const first = entries[0]; editCacheKey = first?.key; setEdit(first?.draft);
    if (first) { setSelected(first.draft.drawing.id); setTool('select'); }
  }
  function newStrokeQueue() {
    const base = cacheKey(), source = strokeSource();
    const key = (id: string) => strokeCacheKey(id, base, source);
    const queue = new DrawingAddQueue({
      read: () => ({ source: strokeSource(), revision: props.port.revision(), drawings: drawings() }),
      active: () => !disposed && source === strokeSource(),
      save: (drawing, expectedRevision) => {
        const command = { type: 'add_drawing' as const, drawing, expectedRevision };
        if (!props.tools.includes(drawing.kind as ManualDrawingKind) || !props.port.allows(command)) throw Error('绘制工具已变化');
        return props.onCommand(command);
      },
      remember: (drawing, revision) => drawingDrafts.set(key(drawing.id), propertyDraft(drawing, revision, 'style', false)),
      forget: drawing => { const stored = drawingDrafts.get(key(drawing.id)); if (stored?.drawing.id === drawing.id) drawingDrafts.delete(key(drawing.id), stored); },
      changed: values => { if (!disposed && strokeQueue === queue) batch(() => { setQueuedDrawings(values); setStrokeVersion(value => value + 1); }); },
      failed: error => {
        if (!disposed && strokeQueue === queue && source === strokeSource()) {
          cancelGesture?.();
          for (const entry of cachedStrokes()) {
            try { drawingDrafts.set(entry.key, { ...entry.draft, error: error instanceof Error ? error.message : String(error) }); }
            catch { /* Keep the already-reserved user geometry even if diagnostic text exceeds the cache budget. */ }
          }
          restoreStrokeDraft(); report(error);
        }
      },
    });
    return queue;
  }
  function strokeCacheKey(id: string, base = cacheKey(), source = strokeSource()) {
    const parts = JSON.parse(base), sceneId = parts[0] === 'image-item' ? parts[1] : parts[0];
    return JSON.stringify([sceneId, 'queued-stroke', base, source, id]);
  }
  function releaseFailedStrokes() {
    if (!strokeQueue?.failed()) return;
    strokeQueue.stop(); strokeQueue = newStrokeQueue(); setQueuedDrawings([]); setStrokeVersion(value => value + 1);
  }
  function storeEdit(value: DrawingPropertyDraft): boolean {
    try { drawingDrafts.remember(editCacheKey ?? cacheKey(), value); setEdit(value); return true; }
    catch (error) { report(error); return false; }
  }
  function cancelEdit() { const value = edit(); if (value) drawingDrafts.delete(editCacheKey ?? cacheKey(), value); editCacheKey = undefined; setEdit(undefined); setDraft(undefined); }
  function cancelUserEdit(){if(!disabled()){releaseFailedStrokes();cancelEdit();restoreStrokeDraft();}}
  const setText = (value: string) => { if(disabled())return; const current = textEditor(); if (current) storeEdit(changeProperty(current, { text: value })); };
  function commit(command: EditorDrawingAction, controlled = false, active: () => boolean = () => true, persisted: () => void = () => {}): Promise<boolean> {
    if (strokeQueue?.pending() || pending() || (!controlled && (props.busy || props.inputLocked)) || !active()) return Promise.resolve(false);
    if (!props.port.allows(command)) return Promise.resolve(false);
    setPending(true); const token = ++serial, origin = identity();
    const request = settleDrawingEdit(() => props.onCommand(command), () => !disposed && token === serial && origin === identity() && active(), persisted)
      .catch(error => { if (!disposed && origin === identity()) report(error); throw error; })
      .finally(() => { if (flight === request) flight = undefined; if (!disposed && token === serial) { setPending(false); setDraft(undefined); } });
    // Callers of drawing gestures retain their existing boolean failure contract.
    const handled = request.catch(() => false); flight = handled;
    void handled.finally(() => { if (flight === handled) flight = undefined; });
    return handled;
  }
  async function choose(next: Tool) {
    if (disabled()) return; const origin = identity(); cancelGesture?.();
    setFinishing(true);
    try {
      await strokeQueue?.drain();
      await drainHistory();
      if (!(await saveText()) || disposed || origin !== identity()) return;
      setDraft(undefined); if (next !== 'select') setSelected('');
      if(strokeKinds.includes(next as DrawingKind))setWideBrush(next==='highlighter');
      if (next === 'mosaic' && mosaicFailed()) setMosaicRetry(value => value + 1); setTool(next);
      if (['highlighter', 'mark'].includes(next) && highlightFailed()) setHighlightRetry(value => value + 1);
    } catch (error) { report(error); }
    finally { if (!disposed) setFinishing(false); }
  }
  function openText(value: Drawing, existing: boolean) {
    if (disabled() || edit()) return;
    if (!storeEdit(propertyDraft(value, props.port.revision(), 'text', existing))) return;
    setSelected(value.id); setTool('text');
    queueMicrotask(() => { if (!disposed && textEditor()?.drawing.id === value.id) textarea?.focus(); });
  }
  function loadLatest() {
    const value = edit(); if (!value || disabled()) return;
    if (editCacheKey && !value.base) {
      releaseFailedStrokes(); storeEdit({ ...value, expectedRevision: props.port.revision(), error: undefined }); return;
    }
    const latest = drawings().find(drawing => drawing.id === value.drawing.id);
    if (!latest) { props.onError('标注已删除'); return; }
    cancelEdit();
    if (value.mode === 'text') openText(latest, true); else setSelected(latest.id);
  }
  function changeStyle(patch: { color?: string; strokeWidth?: number; fontSize?: number; textStyle?: DrawingTextStyle }, submit: boolean) {
    if (disabled() || cancelGesture) return;
    const selectedObject = propertyTarget();
    if (selectedObject && !props.port.canStyle(selectedObject)) return;
    if (patch.textStyle !== undefined) setTextStyle({ ...patch.textStyle });
    if (selectedObject) {
      const value = edit() ?? propertyDraft(selectedObject, props.port.revision());
      if (storeEdit(changeProperty(value, patch)) && submit && value.mode !== 'text') void saveText();
    } else {
      if (patch.color !== undefined) setColor(patch.color);
      if (patch.strokeWidth !== undefined) (tool() === 'highlighter'||(tool()==='eraser'&&wideBrush()) ? setHighlightWidth : setWidth)(patch.strokeWidth);
      if (patch.fontSize !== undefined) (tool() === 'number' ? setNumberDiameter : setFontSize)(patch.fontSize);
    }
  }
  function editNumber(value: string) { if(disabled())return; numberPreference++; setNextNumber(value); }
  function beginPixelErase(event: PointerEvent, secondary: boolean) {
    const pointerId=event.pointerId,origin=identity(),revision=props.port.revision(),originalTool=tool();
    const initial=svg.getBoundingClientRect(),frame=props.port.frame(),diameter=eraseWidth();
    if(initial.width<=0||initial.height<=0||props.documentOnly)return;
    const source=JSON.stringify([props.port.sourceIdentity(),props.port.sourceSize(),frame,props.box]);
    const sourceCurrent=()=>!disposed&&identity()===origin&&originalTool===tool()&&eraserMode()==='pixel'&&source===JSON.stringify([props.port.sourceIdentity(),props.port.sourceSize(),props.port.frame(),props.box]);
    const current=()=>sourceCurrent()&&props.port.revision()===revision&&!props.busy&&!props.inputLocked&&!finishing();
    const pointAt=(p:{clientX:number;clientY:number})=>({x:frame.x+(p.clientX-initial.left)*frame.width/initial.width,y:frame.y+(p.clientY-initial.top)*frame.height/initial.height});
    const eraser=new PixelEraser(drawings(),()=>crypto.randomUUID());
    let last=pointAt(event),ended=false,released=false;
    const visual=drawingFrame(()=>{if(!ended)setPixelDraft(eraser.preview());});
    eraseSource=sourceCurrent;setErasing(true);
    const detach=()=>{svg.removeEventListener('pointermove',moved);svg.removeEventListener('pointerup',up);svg.removeEventListener('pointercancel',canceled);svg.removeEventListener('lostpointercapture',canceled);if(svg.hasPointerCapture(pointerId))svg.releasePointerCapture(pointerId);};
    const clear=()=>{setPixelDraft(undefined);if(eraseSource===sourceCurrent)eraseSource=undefined;cancelGesture=undefined;};
    const finish=(save:boolean)=>{
      if(ended)return;const valid=current();ended=true;released=save&&valid;visual.cancel();detach();setErasing(false);
      const erasures=eraser.erasures();
      if(!released||!erasures.length){clear();return;}
      setPixelDraft(eraser.preview());
      // Flush/close must wait for this one real CAS. A released pointer cannot
      // cancel already accepted persistence when input is subsequently locked.
      cancelGesture=()=>{};
      const completion=commit({...target(revision),type:'erase_drawing_strokes',erasures},true,sourceCurrent).then(saved=>{if(!saved)throw Error('擦除尚未保存');}).finally(()=>{clear();if(eraseFlight===completion)eraseFlight=undefined;});
      eraseFlight=completion;void completion.catch(()=>{});
    };
    const sample=(value:PointerEvent)=>{
      if(ended)return;
      if(!current()){finish(false);return;}
      const next=pointAt(value),previous=last;last=next;
      // Do not cut through a toolbar or an interactive media object's controls.
      const hit=document.elementFromPoint(value.clientX,value.clientY);
      if(hit!==svg&&!(hit&&svg.contains(hit)))return;
      const sweep=clipEraserSweep(previous,next,frame);if(!sweep)return;
      try{if(eraser.sweep(sweep[0],sweep[1],diameter))visual.request();}catch(error){finish(false);report(error);}
    };
    const moved=(value:PointerEvent)=>{if(value.pointerId!==pointerId||ended)return;value.preventDefault();if(!(value.buttons&(secondary?2:1))){finish(true);return;}for(const p of drawingPointerSamples(value))sample(p);};
    const up=(value:PointerEvent)=>{if(value.pointerId!==pointerId||ended)return;sample(value);finish(true);};
    const canceled=(value:PointerEvent)=>{if(value.pointerId===pointerId)finish(false);};
    cancelGesture=()=>finish(false);
    svg.addEventListener('pointermove',moved);svg.addEventListener('pointerup',up);svg.addEventListener('pointercancel',canceled);svg.addEventListener('lostpointercapture',canceled);
    try{svg.setPointerCapture(pointerId);sample(event);}catch(error){finish(false);report(error);}
  }
  function beginErase(event: PointerEvent, secondary = false) {
    if(props.port.pixelEraser&&eraserMode()==='pixel'){beginPixelErase(event,secondary);return;}
    const pointerId = event.pointerId, origin = identity();
    const initialBox=svg.getBoundingClientRect(),frame=props.port.frame(),diameter=eraseWidth();
    if(initialBox.width<=0||initialBox.height<=0)return;
    const inkPoint=(p:{x:number;y:number})=>({x:frame.x+(p.x-initialBox.left)*frame.width/initialBox.width,y:frame.y+(p.y-initialBox.top)*frame.height/initialBox.height});
    setErasing(true);
    const originalTool = tool(), source = JSON.stringify([props.port.sourceIdentity(),props.port.sourceSize(),props.port.frame(),props.box]);
    eraseSource = () => identity() === origin && tool() === originalTool && source === JSON.stringify([props.port.sourceIdentity(),props.port.sourceSize(),props.port.frame(),props.box]);
    let stopped = false, releasedPointer = false;
    const detachPointer = () => {
      svg.removeEventListener('pointermove', moved); svg.removeEventListener('pointerup', released);
      svg.removeEventListener('pointercancel', canceled); svg.removeEventListener('lostpointercapture', canceled);
      if (svg.hasPointerCapture(pointerId)) svg.releasePointerCapture(pointerId);
    };
    const cleanup = () => {
      if (stopped) return; stopped = true;
      detachPointer();
      cancelGesture = undefined;
      eraseSource = undefined;
      setErasing(false);
    };
    const eraser = new ObjectEraser({ x: event.clientX, y: event.clientY }, {
      current: () => !disposed && !stopped && Boolean(eraseSource?.()) && (secondary || tool() === 'eraser') && (releasedPointer || (!props.busy && !props.inputLocked)) && !finishing(),
      hit: (point,previous) => {
        const protectedIds=new Set(drawings().filter(blackboardImage).map(value=>value.id));
        const allowed=new Set([...editableIds()].filter(id=>!protectedIds.has(id)));
        const hits=new Set<string>();
        // Toolbar/media controls cover the surface; never erase through them.
        const top=document.elementFromPoint(point.x,point.y);
        if(!top||(top!==svg&&!svg.contains(top)))return [];
        const ink=drawings().filter(value=>allowed.has(value.id)&&['pen','highlighter','line'].includes(value.kind));
        const sweep=clipEraserSweep(inkPoint(previous),inkPoint(point),frame);
        if(sweep)for(const id of eraserStrokeHits(ink,sweep[0],sweep[1],diameter))hits.add(id);
        const inkIds=new Set(ink.map(value=>value.id));
        const others=new Set([...allowed].filter(id=>!inkIds.has(id)));
        if(others.size){
          // Non-ink (including compact native video layouts) uses painted SVG hits.
          const steps=Math.max(1,Math.ceil(Math.hypot(point.x-previous.x,point.y-previous.y)/2));
          for(let i=1;i<=steps;i++){
            const p={x:previous.x+(point.x-previous.x)*i/steps,y:previous.y+(point.y-previous.y)*i/steps};
            const id=eraserHit(svg,p,others);if(id)hits.add(id);
          }
        }
        return [...hits];
      },
      remove: id => commit({ ...target(), type: 'remove_drawing', drawingId: id },releasedPointer,()=>!disposed&&Boolean(eraseSource?.())),
      stopped: cleanup,
    });
    const finishErase = () => {releasedPointer=true;setErasing(false);detachPointer();eraser.finish();};
    const moved = (next: PointerEvent) => { if (next.pointerId === pointerId) { next.preventDefault(); if (!(next.buttons & (secondary ? 2 : 1))) finishErase(); else for(const sample of drawingPointerSamples(next))eraser.move({ x: sample.clientX, y: sample.clientY }); } };
    const released = (next: PointerEvent) => { if (next.pointerId === pointerId) {eraser.move({x:next.clientX,y:next.clientY});finishErase();} };
    const canceled = (next: PointerEvent) => { if (next.pointerId === pointerId) eraser.stop(); };
    // A completed stroke's accepted IDs must drain before the exit flush returns.
    cancelGesture = () => {if(!releasedPointer)eraser.stop();};
    const completion=eraser.finished;eraseFlight=completion;
    void completion.finally(()=>{if(eraseFlight===completion)eraseFlight=undefined;});
    svg.addEventListener('pointermove', moved); svg.addEventListener('pointerup', released);
    svg.addEventListener('pointercancel', canceled); svg.addEventListener('lostpointercapture', canceled);
    // ObjectEraser initialized its CSS-space step before capture can resync input.
    try { svg.setPointerCapture(pointerId); eraser.start(); } catch { eraser.stop(); }
  }
  function beginResize(event: PointerEvent, handle: number) {
    event.stopPropagation();
    if (event.button !== 0 || disabled() || edit() || cancelGesture || props.documentOnly || tool() !== 'select') return;
    const original = selectedDrawing();
    if (!original || !drawingResizeHandles(original)[handle]) return;
    event.preventDefault(); svg.focus({ preventScroll: true });
    const revision = props.port.revision(), origin = gestureIdentity(), pointerId = event.pointerId;
    const initialBox = svg.getBoundingClientRect();
    if (initialBox.width <= 0 || initialBox.height <= 0) return;
    let value: Drawing = { ...original, points: original.points.map(value => ({ ...value })), ...(original.origin ? { origin: { ...original.origin } } : {}) };
    let ended = false, started = false, last = point(event, false), changed = false;
    const current = () => !disposed && origin === gestureIdentity() && !disabled() && !edit() && tool() === 'select' && selected() === original.id && (() => { const box = svg.getBoundingClientRect(); return box.left === initialBox.left && box.top === initialBox.top && box.width === initialBox.width && box.height === initialBox.height; })();
    const cleanup = () => {
      window.removeEventListener('pointermove', moved); window.removeEventListener('pointerup', released); window.removeEventListener('pointercancel', canceled); svg.removeEventListener('lostpointercapture', canceled);
      cancelGesture = undefined; refreshConstraint = undefined;
      if (svg.hasPointerCapture(pointerId)) svg.releasePointerCapture(pointerId);
    };
    const finish = (save: boolean) => {
      if (ended) return;
      const valid = current(); ended = true; cleanup();
      if (!save || !valid || !changed) { setDraft(undefined); return; }
      void commit({ ...target(revision), type: 'update_drawing', drawing: value });
    };
    const update = (end: DrawingPoint, shift: boolean) => {
      if (!current()) { finish(false); return; }
      last = end;
      const points = resizedDrawingPoints(original, handle, end, shift, props.port.frame());
      if (!points) { finish(false); return; }
      value = { ...value, points };
      changed = points.some((point, index) => point.x !== original.points[index].x || point.y !== original.points[index].y);
      setDraft(changed ? value : undefined);
    };
    const moved = (next: PointerEvent) => {
      if (next.pointerId !== pointerId) return;
      if (!current()) { finish(false); return; }
      if (next.type === 'pointermove' && !(next.buttons & 1)) { finish(false); return; }
      if (!started && Math.hypot(next.clientX - event.clientX, next.clientY - event.clientY) <= 1) return;
      started = true; next.preventDefault(); update(point(next, false), next.shiftKey);
    };
    const released = (next: PointerEvent) => { if (next.pointerId === pointerId) { moved(next); finish(true); } };
    const canceled = (next: PointerEvent) => { if (next.pointerId === pointerId) finish(false); };
    cancelGesture = () => finish(false);
    refreshConstraint = shift => { if (started) update(last, shift); };
    window.addEventListener('pointermove', moved); window.addEventListener('pointerup', released); window.addEventListener('pointercancel', canceled); svg.addEventListener('lostpointercapture', canceled);
    try { svg.setPointerCapture(pointerId); } catch { finish(false); }
  }
  function beginStored(event:PointerEvent,id:string) {
    const original=props.port.stored?.().find(value=>value.id===id);
    if(!original||!props.port.storedMove||disabled()||edit()||cancelGesture)return;
    const revision=props.port.revision(),origin=gestureIdentity(),pointerId=event.pointerId,start=point(event),initial=svg.getBoundingClientRect();
    let value=original.bounds,changed=false,ended=false,started=false;
    const current=()=>!disposed&&origin===gestureIdentity()&&!disabled()&&!edit()&&tool()==='select'&&selected()===id&&(()=>{const box=svg.getBoundingClientRect();return box.left===initial.left&&box.top===initial.top&&box.width===initial.width&&box.height===initial.height;})();
    const cleanup=()=>{window.removeEventListener('pointermove',moved);window.removeEventListener('pointerup',released);window.removeEventListener('pointercancel',canceled);svg.removeEventListener('lostpointercapture',canceled);cancelGesture=undefined;if(svg.hasPointerCapture(pointerId))svg.releasePointerCapture(pointerId);};
    const finish=(save:boolean)=>{if(ended)return;const valid=current();ended=true;cleanup();setStoredDraft(undefined);if(save&&valid&&changed)void commit({type:'move_stored',expectedRevision:revision,drawingId:id,from:original.bounds,to:value});};
    const moved=(next:PointerEvent)=>{if(next.pointerId!==pointerId)return;if(!current()||next.type==='pointermove'&&!(next.buttons&1)){finish(false);return;}if(!started&&Math.hypot(next.clientX-event.clientX,next.clientY-event.clientY)<=4)return;started=true;next.preventDefault();const end=point(next,false);try{value=props.port.storedMove!(id,original.bounds,{x:end.x-start.x,y:end.y-start.y});changed=value.x!==original.bounds.x||value.y!==original.bounds.y;setStoredDraft(changed?{id,bounds:value}:undefined);}catch(error){finish(false);report(error);}};
    const released=(next:PointerEvent)=>{if(next.pointerId===pointerId){moved(next);finish(true);}},canceled=(next:PointerEvent)=>{if(next.pointerId===pointerId)finish(false);};
    cancelGesture=()=>finish(false);window.addEventListener('pointermove',moved);window.addEventListener('pointerup',released);window.addEventListener('pointercancel',canceled);svg.addEventListener('lostpointercapture',canceled);
    try{svg.setPointerCapture(pointerId);}catch{finish(false);}
  }

  async function ensureSelection(id:string):Promise<Drawing|undefined>{
    if(!props.port.ensureEditable)return drawings().find(value=>value.id===id);
    const token=++selectionSerial,origin=identity();setSelectionReading(true);
    try{const value=await props.port.ensureEditable(id);if(!disposed&&token===selectionSerial&&origin===identity()&&selected()===id)return value;}catch(error){if(!disposed&&token===selectionSerial&&origin===identity())report(error);}finally{if(!disposed&&token===selectionSerial)setSelectionReading(false);}
  }
  function selectionGesture(event:PointerEvent,update:(point:DrawingPoint)=>void,finish:(save:boolean)=>void){
    const origin=gestureIdentity(),pointerId=event.pointerId;let ended=false;
    const valid=()=>!disposed&&origin===gestureIdentity()&&!disabled()&&tool()==='select'&&!edit();
    const cleanup=()=>{window.removeEventListener('pointermove',move);window.removeEventListener('pointerup',up);window.removeEventListener('pointercancel',cancel);svg.removeEventListener('lostpointercapture',cancel);cancelGesture=undefined;if(svg.hasPointerCapture(pointerId))svg.releasePointerCapture(pointerId);};
    const stop=(save:boolean)=>{if(ended)return;const current=valid();ended=true;cleanup();finish(save&&current);};
    const move=(e:PointerEvent)=>{if(e.pointerId!==pointerId)return;if(!valid()||!(e.buttons&1)){stop(false);return;}e.preventDefault();update(point(e,false));};
    const up=(e:PointerEvent)=>{if(e.pointerId!==pointerId)return;if(valid())update(point(e,false));stop(true);};
    const cancel=(e:PointerEvent)=>{if(e.pointerId===pointerId)stop(false);};
    cancelGesture=()=>stop(false);window.addEventListener('pointermove',move);window.addEventListener('pointerup',up);window.addEventListener('pointercancel',cancel);svg.addEventListener('lostpointercapture',cancel);
    try{svg.setPointerCapture(pointerId);}catch{stop(false);}
  }
  function beginMarquee(event:PointerEvent){
    const start=point(event),previous=event.shiftKey?selectedIds():[];let box=selectionBox(start,start);
    setMarquee(box);
    selectionGesture(event,end=>{box=selectionBox(start,end);setMarquee(box);},save=>{
      setMarquee(undefined);if(!save)return;
      const ids=box.width+box.height<2?previous:[...new Set([...previous,...marqueeDrawings(drawings(),box,d=>props.port.canSelect(d))])];
      setSelected(ids[0]??'');setSelectedIds(ids);
    });
  }
  function beginGroup(event:PointerEvent){
    const ids=selectedIds(),originals=drawings().filter(d=>ids.includes(d.id)&&props.port.canSelect(d)),start=point(event,false),revision=props.port.revision();let values=originals;
    selectionGesture(event,end=>{values=movedDrawings(originals,end.x-start.x,end.y-start.y,props.port.frame(),props.port.sourceSize());setGroupDraft(values);},save=>{
      if(!save||values.every((d,i)=>d.points.every((p,j)=>p.x===originals[i].points[j].x&&p.y===originals[i].points[j].y))){setGroupDraft([]);return;}
      void commit({...target(revision),type:'update_drawings',drawings:values}).finally(()=>{if(!disposed)setGroupDraft([]);});
    });
  }
  function beginRaster(event:PointerEvent, mode:'extract'|'heal') {
    const raster=props.port.raster;
    if (!raster || disabled() || edit() || cancelGesture) return;
    const initial=svg.getBoundingClientRect();
    if (initial.width<=0 || initial.height<=0) return;
    const origin=gestureIdentity(), pointerId=event.pointerId, start=point(event), brush=healWidth();
    let points=[start], ended=false, frame:number|undefined;
    const current=()=>!disposed && origin===gestureIdentity() && props.port.raster===raster && tool()===mode && !disabled() && !edit() && (()=>{const box=svg.getBoundingClientRect();return box.left===initial.left&&box.top===initial.top&&box.width===initial.width&&box.height===initial.height;})();
    const publish=()=>{frame=undefined;if(!ended&&current())setRasterPreview({mode,points:points.map(p=>({...p})),width:brush});};
    const cleanup=()=>{if(frame!==undefined)cancelAnimationFrame(frame);window.removeEventListener('pointermove',moved);window.removeEventListener('pointerup',released);window.removeEventListener('pointercancel',canceled);svg.removeEventListener('lostpointercapture',canceled);cancelGesture=undefined;if(svg.hasPointerCapture(pointerId))svg.releasePointerCapture(pointerId);};
    const finish=(save:boolean)=>{
      if(ended)return; const valid=current();ended=true;cleanup();setRasterPreview(undefined);
      if(!save||!valid||mode==='extract'&&(points.length!==2||Math.abs(points[1].x-start.x)<2||Math.abs(points[1].y-start.y)<2))return;
      setPending(true);const token=++serial, identityAtStart=identity();
      const request=raster.apply(mode,points.map(p=>({...p})),brush).then(value=>{
        if(!disposed&&token===serial&&identityAtStart===identity()&&props.port.raster===raster){if(value.selectedId){setSelected(value.selectedId);setTool('select');}return true;}return false;
      }).catch(error=>{if(!disposed&&identityAtStart===identity())report(error);return false;}).finally(()=>{if(flight===request)flight=undefined;if(!disposed&&token===serial)setPending(false);});
      flight=request;
    };
    const moved=(next:PointerEvent)=>{
      if(next.pointerId!==pointerId)return;
      if(!current()||next.type==='pointermove'&&!(next.buttons&1)){finish(false);return;}
      next.preventDefault();
      for(const sample of drawingPointerSamples(next)){
        const end=point(sample);
        if(mode==='extract')points=[start,end];
        else if(points.length>=4096){finish(false);props.onError('笔迹最多 4096 个点');return;}
        else if(Math.hypot(end.x-points[points.length-1].x,end.y-points[points.length-1].y)>=.5 || next.type==='pointerup')points.push(end);
      }
      if(frame===undefined)frame=requestAnimationFrame(publish);
    };
    const released=(next:PointerEvent)=>{if(next.pointerId===pointerId){moved(next);finish(true);}},canceled=(next:PointerEvent)=>{if(next.pointerId===pointerId)finish(false);};
    cancelGesture=()=>finish(false);setRasterPreview({mode,points,width:brush});
    window.addEventListener('pointermove',moved);window.addEventListener('pointerup',released);window.addEventListener('pointercancel',canceled);svg.addEventListener('lostpointercapture',canceled);
    try{svg.setPointerCapture(pointerId);}catch{finish(false);}
  }
  function begin(event: PointerEvent, forcedImage?: Drawing) {
    event.stopPropagation();
    const queueable = event.button === 0 && !forcedImage && continuousStrokes() && strokeKinds.includes(tool() as DrawingKind);
    if (strokeQueue?.pending() && !queueable) return;
    if (queueable && strokeQueue && !strokeQueue.capacity()) { props.onError('请先保存未完成的笔迹'); return; }
    if (!forcedImage && event.button === 2) {
      event.preventDefault();
      if (!disabled() && !textEditor() && !edit() && !cancelGesture) { svg.focus({preventScroll:true}); beginErase(event,true); }
      return;
    }
    if (event.button !== 0 || disabled() || cancelGesture) return;
    event.preventDefault();
    if (textEditor()) { void finishTextAt(event); return; }
    if (edit()) { void saveText(); return; }
    svg.focus({ preventScroll: true });
    if (!forcedImage && tool() === 'eraser') { beginErase(event); return; }
    if (!forcedImage && (tool() === 'extract' || tool() === 'heal')) { beginRaster(event,tool() as 'extract'|'heal'); return; }
    const initialBox = svg.getBoundingClientRect();
    if (initialBox.width <= 0 || initialBox.height <= 0) return;
    const initialFrame = props.port.frame();
    const start = point(event, true, initialBox, initialFrame), originalTool = tool(), kind = forcedImage ? 'select' : originalTool, revision = props.port.revision(), origin = gestureSource();
    const strokeRevision = strokeQueue?.revisionIdentity() ?? revision;
    const hitId = event.target instanceof Element ? event.target.closest('[data-drawing-id]')?.getAttribute('data-drawing-id') : undefined;
    const hit = forcedImage ?? (hitId && editableIds().has(hitId) ? drawings().find(value => value.id === hitId) : undefined);
    if (kind === 'mosaic' && !mosaicReady()) return;
    if (['highlighter', 'mark'].includes(kind) && props.port.highlight && !highlightReady()) return;
    const stored=props.port.stored?.().find(value=>value.id===hitId);
    if(kind==='select'&&stored){setSelected(stored.id);void ensureSelection(stored.id);beginStored(event,stored.id);return;}
    if(kind==='text'&&stored?.kind==='text'){setSelected(stored.id);void ensureSelection(stored.id).then(value=>{if(value)openText(value,true);});return;}
    if (kind === 'text') {
      if (hit?.kind === 'text') openText(hit, true);
      else openText({ id: crypto.randomUUID(), kind, color: color(), strokeWidth: width(), points: [start], fontSize: fontSize(), ...(Object.keys(textStyle()).length ? { textStyle: { ...textStyle() } } : {}), text: '' }, false);
      return;
    }
    if (kind === 'number') {
      const number = validNextNumber(nextNumber()); if (number === undefined) { props.onError('序号应为 1–9999'); return; }
      const diameter = numberDiameter(), preference = numberPreference;
      const position = { x: clamp(start.x - diameter / 2, props.port.frame().x, Math.max(props.port.frame().x, props.port.frame().x + props.port.frame().width - diameter)), y: clamp(start.y - diameter / 2, props.port.frame().y, Math.max(props.port.frame().y, props.port.frame().y + props.port.frame().height - diameter)) };
      const drawing: Drawing = { id: crypto.randomUUID(), kind, color: color(), strokeWidth: width(), points: [position], fontSize: diameter, text: String(number) };
      setDraft(drawing);
      void commit({ ...target(revision), type: 'add_drawing', drawing }).then(ok => {
        if (!ok) return;
        const next = Math.min(9999, number + 1); numberChanges.set(props.port.persistedId?.(drawing.id) ?? drawing.id, { previous: number, next, preference });
        if (numberChanges.size > 128) numberChanges.delete(numberChanges.keys().next().value!);
        if (numberPreference === preference) setNextNumber(String(next));
      }); return;
    }
    const original = kind === 'select' ? hit : undefined;
    const objectMove=Boolean(props.blackboard&&original&&blackboardImage(original));
    if(kind==='select'&&!original){beginMarquee(event);return;}
    if(kind==='select'&&original&&!forcedImage&&selectedIds().length>1&&selectedIds().includes(original.id)&&props.port.allows({...target(),type:'update_drawings',drawings:drawings().filter(d=>selectedIds().includes(d.id))})){beginGroup(event);return;}
    if (kind === 'select') { setSelectedIds(original?[original.id]:[]);setSelected(original?.id ?? ''); if (!original) return; }
    else setSelected('');
    if(objectMove)trash.begin();
    let value: Drawing = original ? { ...original, points: original.points.map(value => ({ ...value })) } : { id: crypto.randomUUID(), kind: kind as DrawingKind, color: color(), strokeWidth: kind === 'mosaic' ? blockSize() : kind === 'highlighter' ? highlightWidth() : kind === 'mark' ? 1 : width(), points: kind === 'pen' || kind === 'highlighter' ? [start] : [start, start] };
    let releaseStrokeReservation: (() => void) | undefined;
    const strokeKey = queueable && strokeQueue ? strokeCacheKey(value.id) : undefined;
    if (strokeKey) {
      try {
        // A JSON number uses at most 25 characters; 4096 two-coordinate points
        // fit comfortably within 512 KiB, including all vector style fields.
        const bytes = new TextEncoder().encode(JSON.stringify(strokeKey)).length + 512 * 1024;
        releaseStrokeReservation = drawingDrafts.reserve(strokeKey, bytes);
      } catch (error) { report(error); return; }
    }
    // Keep sampled ink separate from the straight preview so releasing Shift restores it.
    const strokePoints = !original && (kind === 'pen' || kind === 'highlighter') ? value.points : undefined;
    let changed = false, ended = false, dragStarted = !original;
    const pointerId = event.pointerId;
    if (!original) setDraft(value);
    let lastEnd = start;
    let latest: { end: DrawingPoint; shift: boolean } | undefined;
    // An old long stroke's bounds are invariant during a move; compute them once.
    const originalBounds = original ? (() => {
      if (original.kind === 'number') return drawingBounds(original);
      let x = Infinity, y = Infinity, right = -Infinity, bottom = -Infinity;
      for (const point of original.points) { x = Math.min(x, point.x); y = Math.min(y, point.y); right = Math.max(right, point.x); bottom = Math.max(bottom, point.y); }
      return { x, y, width: right - x, height: bottom - y };
    })() : undefined;
    const current = () => !disposed && origin === gestureSource() && !disabled() && !edit() && tool() === originalTool
      && (original ? selected() === original.id : props.tools.includes(kind as ManualDrawingKind))
      && props.port.allows({ ...target(revision), type: original ? 'update_drawing' : 'add_drawing', drawing: value });
    const surfaceCurrent = () => { const box = svg.getBoundingClientRect(); return box.left === initialBox.left && box.top === initialBox.top && box.width === initialBox.width && box.height === initialBox.height; };
    const update = (end: DrawingPoint, shift: boolean, terminal = false) => {
      lastEnd = end;
      if (original) {
        const movedByPort = props.port.move?.(original, start, end);
        if (movedByPort) { value = movedByPort; changed = value.points.some((point, i) => point.x !== original.points[i].x || point.y !== original.points[i].y); setDraft(changed ? value : undefined); return; }
        const bounds = originalBounds!;
        if (Math.hypot(end.x - start.x, end.y - start.y) < .1) { value = original; changed = false; setDraft(undefined); return; }
        const dx = blackboardImage(original)?end.x-start.x:drawingMoveDelta(end.x - start.x, bounds.x, bounds.x + bounds.width, props.port.frame().x, props.port.frame().x + props.port.frame().width, props.port.sourceSize().width);
        const dy = blackboardImage(original)?end.y-start.y:drawingMoveDelta(end.y - start.y, bounds.y, bounds.y + bounds.height, props.port.frame().y, props.port.frame().y + props.port.frame().height, props.port.sourceSize().height);
        value = { ...original, points: original.points.map(point => ({ ...point, x: point.x + dx, y: point.y + dy })) }; changed = Math.abs(dx) + Math.abs(dy) > .1;
      } else if (kind === 'pen' || kind === 'highlighter') {
        const points = strokePoints!, last = points[points.length - 1];
        if (end.x !== last.x || end.y !== last.y) {
          if (points.length >= 4096) {
            if (props.port.rejectPointOverflow) { finish(false); props.onError('笔迹最多 4096 个点'); return; }
            // Preserve the release endpoint under the existing screenshot point bound.
            if (terminal) points[points.length - 1] = end;
          } else points.push(end);
        }
        if (kind === 'highlighter') value = { ...value, points: shift ? [start, constrainedEnd('line', start, end, true, initialFrame)] : points };
        return;
      } else value = { ...value, points: [start, constrainedEnd(kind as DrawingKind, start, end, shift, props.port.frame())] };
      setDraft(value);
    };
    const publish = () => {
      if (ended) return;
      if (!current() || !surfaceCurrent()) { finish(false); return; }
      if (latest) { const next = latest; latest = undefined; update(next.end, next.shift); }
      if (!ended && !original && (kind === 'pen' || kind === 'highlighter')) setDraft({ ...value });
    };
    const previewFrame = drawingFrame(publish);
    const moved = (next: PointerEvent) => {
      if (next.pointerId !== pointerId) return;
      if (!current() || (next.type === 'pointermove' && !(next.buttons & 1))) { finish(false); return; }
      if(objectMove)trash.move({x:next.clientX,y:next.clientY});
      if (!dragStarted) {
        if (Math.abs(next.clientX - event.clientX) <= 4 && Math.abs(next.clientY - event.clientY) <= 4) return;
        dragStarted = true;
        if (next.buttons & 1) try { svg.setPointerCapture(pointerId); } catch { /* Window listeners still finish this gesture. */ }
      }
      next.preventDefault();
      if (!original && (kind === 'pen' || kind === 'highlighter')) {
        latest = undefined;
        for (const sample of drawingPointerSamples(next)) { if (sample.pointerId !== pointerId) continue; update(point(sample, true, initialBox, initialFrame), next.shiftKey, next.type === 'pointerup'); if (ended) return; }
      } else latest = { end: point(next, !original||!blackboardImage(original), initialBox, initialFrame), shift: next.shiftKey };
      if (next.type === 'pointerup') publish(); else previewFrame.request();
    };
    refreshConstraint = shift => { if (!original && ['line', 'arrow', 'rect', 'ellipse', 'highlighter', 'mark'].includes(kind)) { latest = { end: latest?.end ?? lastEnd, shift }; previewFrame.request(); } };
    const cleanup = () => {
      previewFrame.cancel(); latest = undefined;
      window.removeEventListener('pointermove', moved); window.removeEventListener('pointerup', released);
      window.removeEventListener('pointercancel', canceled); svg.removeEventListener('lostpointercapture', canceled);
      cancelGesture = undefined; refreshConstraint = undefined; if (svg.hasPointerCapture(pointerId)) svg.releasePointerCapture(pointerId);
    };
    const finish = (save: boolean) => {
      if (ended) return;
      const valid = current() && surfaceCurrent(); ended = true; cleanup();
      try {
      const removeObject=objectMove&&save&&valid&&dragStarted&&trashOver();if(objectMove)trash.cancel();
      if(removeObject){setDraft(undefined);void commit({...target(revision),type:'remove_drawing',drawingId:original!.id}).finally(()=>{if(!disposed)setSelected('');});return;}
      if (!save || !valid || (original && !changed)) { setDraft(undefined); return; }
      const [a, b] = value.points;
      if (!original && (((kind === 'arrow' || kind === 'line') && Math.hypot(b.x - a.x, b.y - a.y) < 1) || (['rect', 'ellipse', 'mosaic', 'mark'].includes(kind) && (Math.abs(b.x - a.x) < 1 || Math.abs(b.y - a.y) < 1)))) { setDraft(undefined); return; }
      // A receipt owns an immutable snapshot; late input cannot mutate its point array.
      value = { ...value, points: value.points.map(point => ({ ...point })) };
      if (!original && queueable && strokeQueue) {
        if (strokeQueue.enqueue(value, strokeRevision)) setDraft(undefined);
        else { const saved = { ...propertyDraft(value, props.port.revision(), 'style', false), error: '笔迹尚未保存' }; drawingDrafts.set(strokeKey!, saved); editCacheKey = strokeKey; setEdit(saved); }
        return;
      }
      void commit({ ...target(revision), type: original ? 'update_drawing' : 'add_drawing', drawing: value }).finally(()=>{if(forcedImage&&!disposed)setSelected('');});
      } finally { releaseStrokeReservation?.(); }
    };
    const released = (next: PointerEvent) => { if (next.pointerId === pointerId) { moved(next); finish(true); } };
    const canceled = (next: PointerEvent) => { if (next.pointerId === pointerId) finish(false); };
    cancelGesture = () => finish(false);
    window.addEventListener('pointermove', moved); window.addEventListener('pointerup', released); window.addEventListener('pointercancel', canceled); svg.addEventListener('lostpointercapture', canceled);
    if (!original) try { svg.setPointerCapture(pointerId); } catch { finish(false); }
  }
  async function finishTextAt(event: PointerEvent) {
    // A click commits at the current position, then starts the next inline text.
    // Freeze only source/geometry identity: the successful save advances revision.
    const source = () => JSON.stringify([identity(),props.port.sourceIdentity(),props.port.frame(),props.box]);
    const origin = source();
    if (await saveText() && !disposed && origin === source() && !disabled() && tool() === 'text') begin(event);
  }
  function textBlur(event: FocusEvent) {
    if (!event.relatedTarget || event.relatedTarget instanceof Node && toolbar.contains(event.relatedTarget) || event.relatedTarget instanceof Element && event.relatedTarget.closest('.drawing-color-panel')) return;
    void saveText();
  }
  function confirmText() {
    releaseFailedStrokes();
    void saveText().then(saved => { if (saved && !disposed && !textEditor()) { restoreStrokeDraft(); svg.focus({preventScroll:true}); } });
  }
  async function saveText(controlled = false, active: () => boolean = () => true): Promise<boolean> {
    const value = edit(); if (!value) return true;
    if (pending() || (!controlled && props.busy) || !active()) return false;
    if (strokeQueue?.pending() || strokeQueue?.failed()) return false;
    const key = editCacheKey ?? cacheKey(), origin = identity();
    try {
      const command: EditorDrawingAction | undefined = value.mode === 'text' && !value.drawing.text?.trim()
        ? value.base ? {...target(value.expectedRevision),type:'remove_drawing',drawingId:value.base.id} : undefined
        : props.port.propertyCommand(value);
      if (!command) { cancelEdit(); return true; }
      let persisted = false;
      const saved = await commit(command, controlled, active, () => { persisted = true; drawingDrafts.delete(key, value); });
      if (persisted && !disposed && origin === identity() && edit() === value) { editCacheKey = undefined; setEdit(undefined); setRecoveredDrawings(cachedStrokes().map(entry => entry.draft.drawing)); setSelected(value.mode === 'text' ? '' : props.port.persistedId?.(value.drawing.id) ?? value.drawing.id); }
      if (saved) {
        return true;
      }
      if (!persisted && !disposed && origin === identity() && edit() === value) storeEdit({ ...value, error: '保存失败' });
    } catch (error) {
      if (!disposed && origin === identity() && edit() === value) { storeEdit({ ...value, error: error instanceof Error ? error.message : String(error) }); report(error); }
    }
    return false;
  }
  function doubleClick(event: MouseEvent) {
    if (props.documentOnly) return;
    if (!['select', 'text'].includes(tool()) || disabled() || edit()) return;
    const id = eraserHit(svg, { x: event.clientX, y: event.clientY }, editableIds());
    const value = drawings().find(drawing => drawing.id === id);
    if(props.port.stored?.().find(value=>value.id===id)?.kind==='text'){event.preventDefault();event.stopPropagation();cancelGesture?.();setSelected(id!);void ensureSelection(id!).then(value=>{if(value)openText(value,true);});return;}
    if (value?.kind !== 'text') return;
    event.preventDefault(); event.stopPropagation(); cancelGesture?.(); openText(value, true);
  }
  function copySelectedTable() {
    const drawing = selectedDrawing();
    if (disabled() || edit() || cancelGesture || !props.port.copyTable || drawing?.rich?.kind !== 'table') return;
    const origin = identity(), source = props.port.sourceIdentity(), format = tableFormat(); setFinishing(true);
    const request = Promise.resolve().then(() => { if (disposed || origin !== identity() || source !== props.port.sourceIdentity() || selected() !== drawing.id) throw Error('表格已更新'); return props.port.copyTable!(drawing, format); });
    tableCopy = request;
    void request.catch(error => { if (!disposed && origin === identity()) report(error); }).finally(() => { if (tableCopy === request) tableCopy = undefined; if (!disposed && origin === identity()) setFinishing(false); });
  }
  async function done(closeSpace = false, copy = true, exitDrawing = true, variant?: ImageExportVariant) {
    const closingDisabledBoard = Boolean(props.blackboard && props.inputLocked && !props.busy && !pending() && !finishing());
    if ((historyDisabled() && !closingDisabledBoard) || cancelGesture) return;
    const currentIdentity = identity(); setFinishing(true);
    try {
      await strokeQueue?.drain();
      await drainHistory();
      if (props.blackboard && exitDrawing && props.onFinishBlackboard) {
        if (closingDisabledBoard) { await props.onFinishBlackboard(); return; }
        if (await saveText() && !disposed && currentIdentity === identity()) await props.onFinishBlackboard();
        return;
      }
      await finishDrawing({ saveText, current: () => !disposed && currentIdentity === identity(), copy: () => exitDrawing && props.retainOnDone ? Promise.resolve(true) : props.onExport(copy, closeSpace, variant), close: () => { if (exitDrawing) props.onClose(); } });
    }
    catch (error) { if (!disposed) report(error); }
    finally { if (!disposed) setFinishing(false); }
  }
  const remove = () => { const value = selectedDrawing() ?? props.port.stored?.().find(value=>value.id===selected()); if (value && !(props.blackboard&&'rich' in value&&blackboardImage(value as Drawing)) && !disabled() && !cancelGesture && !edit()) void commit({ ...target(), type: 'remove_drawing', drawingId: value.id }).then(ok => { if (ok) setSelected(''); }); };
  async function pin() {
    if (disabled() || cancelGesture || !props.onPin) return;
    const currentIdentity = identity(); setFinishing(true);
    try { await strokeQueue?.drain(); await drainHistory(); await outputDrawing({ saveText, current: () => !disposed && currentIdentity === identity(), output: () => props.onPin?.() ?? Promise.resolve(false) }); }
    catch (error) { if (!disposed) report(error); }
    finally { if (!disposed) setFinishing(false); }
  }
  async function closeEditor() {
    if (historyDisabled() || cancelGesture) return;
    const source = strokeSource(); setFinishing(true);
    try { await strokeQueue?.drain(); await drainHistory(); if (!disposed && source === strokeSource()) props.onClose(); }
    catch (error) { if (!disposed) report(error); }
    finally { if (!disposed) setFinishing(false); }
  }
  function readHistory(): DrawingHistoryState {
    const value = props.port.history!();
    return { source: strokeSource(), revision: props.port.revision(), drawings: drawings(), undo: value.undo, redo: value.redo };
  }
  function historyAcknowledged(redo: boolean, entry: import('../contracts').DrawingStep) {
    const number = !('batch' in entry) && !entry.before && entry.after?.kind === 'number' ? numberChanges.get(entry.after.id) : undefined;
    if (number && number.preference === numberPreference && validNextNumber(nextNumber()) === (redo ? number.previous : number.next)) setNextNumber(String(redo ? number.next : number.previous));
  }
  function createHistoryLane() {
    const source = strokeSource();
    const lane = new DrawingHistoryLane({
      read: readHistory, active: () => !disposed && source === strokeSource(),
      save: (redo, expectedRevision) => {
        const action = { type: redo ? 'redo_drawing' as const : 'undo_drawing' as const, expectedRevision };
        if (!props.port.historyAvailable(redo) || !props.port.allows(action)) throw Error('绘制历史或工具已变化');
        return props.onCommand(action);
      },
      changed: preview => { if (!disposed && historyLane === lane && source === strokeSource()) batch(() => { setHistoryPreview(preview); setHistoryPending(preview !== undefined); }); },
      completed: historyAcknowledged,
      failed: error => { if (!disposed && historyLane === lane && source === strokeSource()) report(error); },
    });
    return lane;
  }
  async function drainHistory(active: () => boolean = () => true) {
    if (historyPreparation) await historyPreparation;
    await historyLane?.drain(active);
    if (!active()) throw Error('绘制编辑已切换');
  }
  function beginHistoryAfterStrokes() {
    const source = strokeSource(), queue = strokeQueue;
    const preparation = (async () => {
      await queue?.drain();
      if (disposed || source !== strokeSource()) throw Error('绘制编辑已切换');
      // The provisional view never creates native history. Re-read the real
      // history after every accepted add is durable before issuing any CAS.
      let proof = readHistory();
      for (const redo of historyIntents) { const after = projectDrawingHistory(proof, redo); if (!after) throw Error('绘制历史已变化'); proof = after; }
      historyLane = createHistoryLane();
      for (const redo of historyIntents) if (!historyLane.enqueue(redo)) throw Error('绘制历史已变化');
      historyIntents = []; preparingHistory = undefined;
      await historyLane.drain();
    })().catch(error => {
      if (!disposed && source === strokeSource()) { batch(() => { setHistoryPreview(undefined); setHistoryPending(false); }); report(error); }
      throw error;
    }).finally(() => { if (historyPreparation === preparation) { historyPreparation = undefined; historyIntents = []; preparingHistory = undefined; } });
    historyPreparation = preparation; void preparation.catch(() => {});
  }
  async function history(redo: boolean) {
    if (historyDisabled() || edit() || cancelGesture) return;
    if (props.port.history && !props.port.stored) {
      try {
        if (preparingHistory || strokeQueue?.pending()) {
          if (historyIntents.length >= 16) return;
          if (!preparingHistory) {
            preparingHistory = readHistory();
            for (const drawing of queuedDrawings()) if (!preparingHistory.drawings.some(value => value.id === drawing.id)) {
              const step: import('../contracts').DrawingStep = { index: preparingHistory.drawings.length, before: null, after: drawing };
              preparingHistory = { ...preparingHistory, drawings: [...preparingHistory.drawings, drawing], undo: [...preparingHistory.undo, step], redo: [] };
            }
          }
          const next = projectDrawingHistory(preparingHistory, redo); if (!next) return;
          preparingHistory = next; historyIntents.push(redo);
          batch(() => { setHistoryPending(true); setHistoryPreview(next.drawings); });
          if (!historyPreparation) beginHistoryAfterStrokes();
        } else { historyLane ??= createHistoryLane(); historyLane.enqueue(redo); }
      } catch (error) { report(error); }
      return;
    }
    if (disabled()) return;
    const source = strokeSource(); setFinishing(true);
    try { await strokeQueue?.drain(); } catch (error) { report(error); return; }
    finally { if (!disposed) setFinishing(false); }
    if (disposed || source !== strokeSource() || disabled() || edit() || cancelGesture) return;
    if (!historyAvailable(redo)) return;
    const entry = props.port.historyStep?.(redo);
    const number = entry && !('batch' in entry) && !entry.before && entry.after?.kind === 'number' ? numberChanges.get(entry.after.id) : undefined;
    void commit({ ...target(), type: redo ? 'redo_drawing' : 'undo_drawing' }).then(ok => {
      if (ok && number && number.preference === numberPreference && validNextNumber(nextNumber()) === (redo ? number.previous : number.next)) setNextNumber(String(redo ? number.next : number.previous));
    });
  }
  function keyboard(event: KeyboardEvent) {
    if ((event.target instanceof Element ? event.target : document.activeElement)?.closest('.drawing-color-panel')) return;
    if ((event.target instanceof Element ? event.target : document.activeElement)?.closest(props.keyboardIgnore ?? '.video-trim-popover,.artifact-video,.blackboard-object,[data-run-journal]')) return;
    if (props.inputLocked) {
      if (props.blackboard && event.type === 'keydown' && event.key === 'Escape' && !event.isComposing && event.keyCode !== 229) { event.preventDefault(); event.stopImmediatePropagation(); void done(); }
      return;
    }
    if (document.querySelector('.drawing-drafts-dialog')) return;
    if (event.isComposing || event.keyCode === 229) return;
    if (event.key === 'Shift') { refreshConstraint?.(event.shiftKey); return; }
    if (event.type === 'keyup') return;
    if (nativeSelectOwnsEscape(event)) return;
    if (event.key === 'Escape') {
      event.preventDefault(); event.stopImmediatePropagation();
      if (cancelGesture) cancelGesture(); else if (!disabled() && textEditor()) confirmText(); else if (!disabled() && edit()) cancelUserEdit(); else if (!historyDisabled()) { if (props.blackboard) void done(); else void closeEditor(); }
      return;
    }
    if (event.target instanceof Element && event.target.closest('input,textarea,select,[contenteditable="true"]')) return;
    if ((event.ctrlKey || event.metaKey) && ['z', 'y'].includes(event.key.toLowerCase())) { event.preventDefault(); event.stopImmediatePropagation(); void history(event.shiftKey || event.key.toLowerCase() === 'y'); return; }
    if (event.repeat) return;
    if (event.key.toLowerCase() === 'p' && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey && props.onPin) { event.preventDefault(); event.stopImmediatePropagation(); void pin(); return; }
    if (event.key === 'Enter' && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey && !(event.target instanceof Element && event.target.closest('button'))) { event.preventDefault(); event.stopImmediatePropagation(); void done(true); return; }
    if (['c', 's'].includes(event.key.toLowerCase()) && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey) { event.preventDefault(); event.stopImmediatePropagation(); if (event.key.toLowerCase() === 's' && props.imageExport !== false) { if (!disabled() && !cancelGesture) openImageExport?.(); } else void done(false, event.key.toLowerCase() === 'c', false); return; }
    if (event.key === 'Delete' || event.key === 'Backspace') { event.preventDefault(); event.stopImmediatePropagation(); remove(); }
  }
  const resize = () => { cancelGesture?.(); setViewport({ width: innerWidth, height: innerHeight }); };
  const blur = () => cancelGesture?.();
  const identity = createMemo(cacheKey);
  const queueSource = createMemo(strokeSource);
  createEffect(on(queueSource, () => {
    historyLane?.stop(); historyLane = undefined; historyPreparation = undefined; preparingHistory = undefined; historyIntents = []; setHistoryPreview(undefined); setHistoryPending(false);
    strokeQueue?.stop(); strokeQueue = continuousStrokes() ? newStrokeQueue() : undefined; setQueuedDrawings([]); setRecoveredDrawings([]); editCacheKey = undefined;
  }));
  createEffect(on(identity, () => {
    cancelGesture?.(); setDraft(undefined);
    const saved = props.documentOnly ? undefined : drawingDrafts.get(cacheKey()); setEdit(saved);
    if (saved) { setSelected(saved.drawing.id); setTool(saved.mode === 'text' ? 'text' : 'select'); }
    else { setSelected(''); restoreStrokeDraft(); }
  }));
  createEffect(() => { if (props.busy || props.inputLocked) cancelGesture?.(); });
  const gestureSource = createMemo(gestureIdentity);
  createEffect(on(gestureSource, () => { if (!eraseSource?.()) cancelGesture?.(); }));
  createEffect(() => { if (svg) { const observer = new ResizeObserver(() => { const box = svg.getBoundingClientRect(); const before = surfaceSize(); if (before.width !== box.width || before.height !== box.height) { cancelGesture?.(); setSurfaceSize({ width: box.width, height: box.height }); } }); observer.observe(svg); onCleanup(() => observer.disconnect()); } });
  createEffect(() => { if (toolbar) { const observer = new ResizeObserver(() => { setToolbarSize({ width: toolbar.offsetWidth, height: toolbar.offsetHeight }); setToolsWidth(toolsRow.offsetWidth); }); observer.observe(toolbar); observer.observe(toolsRow); onCleanup(() => observer.disconnect()); } });
  const mosaicIdentity = createMemo(() => tool() === 'mosaic' && props.port.mosaic ? JSON.stringify([props.port.sourceIdentity(), blockSize(), mosaicRetry()]) : '');
  const highlightIdentity = createMemo(() => ['highlighter', 'mark'].includes(tool()) && props.port.highlight ? JSON.stringify([props.port.highlight.identity?.() ?? props.port.sourceIdentity(), highlightRetry()]) : '');
  createEffect(on(highlightIdentity, key => {
    setHighlightReady(false); setHighlightFailed(false); let alive = true;
    if (key && props.port.highlight) void props.port.highlight.prepare().then(() => { if (alive) setHighlightReady(true); }).catch(error => { if (alive) { setHighlightFailed(true); report(error); } });
    onCleanup(() => { alive = false; });
  }));
  createEffect(on(mosaicIdentity, key => {
    setMosaicReady(false); setMosaicFailed(false); let alive = true;
    if (key && props.port.mosaic) void props.port.mosaic.prepare().then(() => { if (alive) setMosaicReady(true); }).catch(error => { if (alive) { setMosaicFailed(true); report(error); } });
    onCleanup(() => { alive = false; });
  }));
  document.addEventListener('keydown', keyboard, true); document.addEventListener('keyup', keyboard, true); window.addEventListener('resize', resize); window.addEventListener('blur', blur);
  const unregisterFlush = props.onRegisterFlush?.(async active => {
    const origin = identity(); cancelGesture?.();
    await strokeQueue?.drain(active);
    await drainHistory(active);
    if(eraseFlight)await eraseFlight;
    if (tableCopy) await tableCopy;
    if (flight && !(await flight)) throw new Error('标注尚未保存');
    await flushImages?.(active);
    await props.objectsFlush?.(active);
    if (!active() || disposed || origin !== identity()) throw new Error('标注编辑已切换');
    if (!(await saveText(true, () => active() && !disposed && origin === identity()))) throw new Error('标注尚未保存');
  });
  const unregisterHistory = props.onRegisterHistory?.({
    undo:()=>history(false),redo:()=>history(true),
    canUndo:()=>!historyDisabled()&&!edit()&&!erasing()&&!cancelGesture&&historyAvailable(false),
    canRedo:()=>!historyDisabled()&&!edit()&&!erasing()&&!cancelGesture&&historyAvailable(true),
  });
  onCleanup(()=>unregisterHistory?.());
  onCleanup(() => { unregisterFlush?.(); disposed = true; strokeQueue?.stop(); historyLane?.stop(); serial++; cancelGesture?.(); document.removeEventListener('keydown', keyboard, true); document.removeEventListener('keyup', keyboard, true); window.removeEventListener('resize', resize); window.removeEventListener('blur', blur); });
  function DrawingItem(item: { id: string }) {
    const isDraft = createMemo(() => activeDraftId() === item.id), isEdit = createMemo(() => activeEditId() === item.id);
    const value = createMemo(() => {const saved=drawingIndex().get(item.id),preview=pixelIndex().get(item.id)??previews().get(item.id);return preview??(saved?linkedRepairPreview(saved,drawingIndex(),previews()):undefined);});
    const drawing = liveDrawing(item.id, () => value()!);
    const isPreview = createMemo(() => isDraft() || isEdit() || queuedDrawings().some(value => value.id === item.id) || recoveredDrawings().some(value => value.id === item.id));
    const isSelected = createMemo(() => selectedIds().includes(item.id) && !(props.blackboard && value()?.rich?.kind === 'extracted'&&selectedIds().length===1));
    const interactive = createMemo(() => (props.blackboard || ['select', 'eraser', 'text'].includes(tool())) && Boolean(value() && props.port.canSelect(drawing)));
    const render = () => <>{props.port.render(drawing, interactive(), isSelected(), isPreview())}</>;
    const storedBounds = createMemo(() => storedPreviewId() === item.id ? storedDraft()?.bounds : undefined);
    const stored = createMemo(() => storedIndex().has(item.id) ? props.port.renderStored?.(item.id, ['select', 'eraser', 'text'].includes(tool()), isSelected(), storedBounds()) : undefined);
    return <Show when={storedIndex().has(item.id)} fallback={<Show when={value()}>{_ => render()}</Show>}>
      {_ => <Show when={isPreview()} fallback={stored()}>{_ => render()}</Show>}
    </Show>;
  }
  return <div ref={editorShell} class="drawing-editor" classList={{'drawing-blackboard':props.blackboard}} onPointerMove={props.onObjectPointerMove} onPointerLeave={props.onObjectPointerLeave} onContextMenu={event=>{if(props.blackboard){event.preventDefault();event.stopPropagation();}}} onPointerDown={event => event.stopPropagation()}>
    {props.objects?.(()=>tool()==='select')}
    <Show when={props.blackboard&&tool()==='select'}><div class="drawing-selection-background" style={{left:`${props.box.x}px`,top:`${props.box.y}px`,width:`${props.box.width}px`,height:`${props.box.height}px`}} onPointerDown={begin}/></Show>
    <svg ref={svg} tabindex={0} class="drawing-edit-surface" classList={{ 'drawing-select-mode': tool() === 'select', 'drawing-text-mode': tool() === 'text', 'drawing-eraser-mode': tool() === 'eraser'||erasing() }} style={{ left: `${props.box.x}px`, top: `${props.box.y}px`, width: `${props.box.width}px`, height: `${props.box.height}px`,'pointer-events':props.blackboard&&tool()==='select'?'none':undefined,cursor:tool()==='eraser'||erasing()?eraseCursor():undefined }} viewBox={`${props.port.frame().x} ${props.port.frame().y} ${props.port.frame().width} ${props.port.frame().height}`} preserveAspectRatio="none" onPointerDown={begin} onContextMenu={event=>event.preventDefault()} onDblClick={doubleClick} aria-label={t("绘制区域")}>
      <For each={drawingIds()}>{id => <DrawingItem id={id} />}</For>
      <Show when={textEditor()}>{value => <InlineDrawingText drawing={value().drawing} disabled={pending() || !!props.inputLocked} ref={element => {textarea=element;}} onInput={setText} onConfirm={confirmText} onBlur={textBlur}/>}</Show>
      <Show when={marquee()}>{box=><rect class="drawing-marquee" {...box()} fill="#348bf1" fill-opacity=".07" stroke="#348bf1" stroke-width="1" vector-effect="non-scaling-stroke" pointer-events="none"/>}</Show>
      <Show when={rasterPreview()}>{value=> <Show when={value().mode==='extract'} fallback={<polyline points={value().points.map(p=>`${p.x},${p.y}`).join(' ')} fill="none" stroke="#428fff" stroke-opacity=".38" stroke-width={value().width} stroke-linecap="round" stroke-linejoin="round" pointer-events="none" />}><rect x={Math.min(value().points[0].x,value().points.at(-1)!.x)} y={Math.min(value().points[0].y,value().points.at(-1)!.y)} width={Math.abs(value().points.at(-1)!.x-value().points[0].x)} height={Math.abs(value().points.at(-1)!.y-value().points[0].y)} fill="none" stroke="#428fff" stroke-width="1" vector-effect="non-scaling-stroke" pointer-events="none" /></Show>}</Show>
      <For each={resizeHandles()}>{(point, index) => <ellipse class="drawing-resize-handle" cx={point.x} cy={point.y} rx={handleSize().x / 2} ry={handleSize().y / 2} vector-effect="non-scaling-stroke" style={{ cursor: resizeDrawing()?.kind === 'line' || resizeDrawing()?.kind === 'arrow' ? 'move' : index() % 2 ? 'nesw-resize' : 'nwse-resize' }} aria-label={resizeDrawing()?.kind === 'line' || resizeDrawing()?.kind === 'arrow' ? t('调整端点') : t('缩放图形')} onPointerDown={event => beginResize(event, index())} />}</For>
    </svg>
    <div ref={toolbar} class="drawing-toolbar" data-controls-locked={controlsLocked()} classList={{ 'drawing-dock-right': rightDocked() }} role="toolbar" aria-label={t("绘制工具")} style={position()}>
      <div ref={toolsRow} class="drawing-tools-row">
      <button data-caption={t("选择")} title={t("选择并移动")} aria-label={t("选择并移动绘制对象")} classList={{ selected: tool() === 'select' }} data-unavailable={controlsLocked()} disabled={disabled()} onClick={() => choose('select')}><MousePointer2 size={17} /></button>
      <button data-caption={t("橡皮")} title={t("橡皮")} aria-label={t("橡皮")} classList={{ selected: tool() === 'eraser' }} data-unavailable={controlsLocked()} disabled={disabled()} onClick={() => choose('eraser')}><Eraser size={17} /></button>
      <For each={props.tools}>{kind => { const Icon = icons[kind]; return <button data-caption={t(labels[kind])} title={t(labels[kind])} aria-label={t(labels[kind])} classList={{ selected: tool() === kind }} data-unavailable={controlsLocked()} disabled={disabled()} onClick={() => choose(kind)}><Icon size={18} /></button>; }}</For>
      <Show when={props.port.raster}><span class="drawing-divider" /><button data-caption={t("无痕提取")} title={t("无痕提取")} aria-label={t("无痕提取")} classList={{selected:tool()==='extract'}} disabled={disabled()} onClick={()=>choose('extract')}><Scissors size={18}/></button><button data-caption={t("涂抹消除")} title={t("涂抹消除")} aria-label={t("涂抹消除")} classList={{selected:tool()==='heal'}} disabled={disabled()} onClick={()=>choose('heal')}><BrushCleaning size={18}/></button></Show>
      <Show when={selectionReading()}><LoaderCircle size={15} class="spin" /></Show>
      {props.extraTools?.()}
      </div>
      <div class="drawing-properties-row">
      <Show when={tool()==='eraser'&&props.port.pixelEraser}>
        <button data-caption={t('像素橡皮')} title={t('局部擦除画笔、直线和荧光笔')} aria-label={t('像素橡皮')} aria-pressed={eraserMode()==='pixel'} classList={{selected:eraserMode()==='pixel'}} disabled={disabled()} onClick={()=>{cancelGesture?.();setEraserMode('pixel');}}><Eraser size={20}/></button>
        <button data-caption={t('对象橡皮')} title={t('整项擦除绘制对象')} aria-label={t('对象橡皮')} aria-pressed={eraserMode()==='object'} classList={{selected:eraserMode()==='object'}} disabled={disabled()} onClick={()=>{cancelGesture?.();setEraserMode('object');}}><Square size={20}/></button>
      </Show>
      <div class="drawing-property-controls">
      <Show when={!['mosaic', 'eraser', 'select', 'rich','extract','heal'].includes(propertyKind())}><DrawingColorPicker value={propertyColor()} disabled={disabled()} onChange={(color,submit)=>changeStyle({color},submit)}/></Show>
      <Show when={tool()==='heal'}><DrawingWidthSlider label={t('画笔大小')} value={healWidth()} min={1} max={64} disabled={disabled()} onChange={value=>{if(!disabled())setHealWidth(value);}}/></Show>
      <Show when={strokeKinds.includes(propertyKind() as DrawingKind)||tool()==='eraser'}><DrawingWidthSlider label={t('线宽')} value={propertyWidth()} min={1} max={64} disabled={disabled()} onChange={(value,submit)=>changeStyle({strokeWidth:value},submit)}/></Show>
      <Show when={tool() === 'mosaic' && !mosaicReady() && !mosaicFailed()}><LoaderCircle size={15} class="spin" /></Show>
      <Show when={['highlighter', 'mark'].includes(tool()) && props.port.highlight && !highlightReady() && !highlightFailed()}><LoaderCircle size={15} class="spin" /></Show>
      <Show when={tool() === 'number'}><input class="drawing-number" type="number" aria-label={t("下个序号")} title={t("下个序号")} min="1" max="9999" step="1" value={nextNumber()} disabled={disabled()} onInput={event => editNumber(event.currentTarget.value)} /></Show>
      <Show when={propertyKind() === 'number'}><select aria-label={t("序号大小")} title={t("序号大小")} value={propertySize()} disabled={disabled()} onChange={event => changeStyle({ fontSize: Number(event.currentTarget.value) }, true)}><For each={options([22, 28, 36, 48, 64], propertySize())}>{value => <option value={value} selected={value === propertySize()}>{value}</option>}</For></select></Show>
      <Show when={propertyKind() === 'text'}><select aria-label={t("字号")} title={t("字号")} value={propertySize()} disabled={disabled()} onChange={event => changeStyle({ fontSize: Number(event.currentTarget.value) }, true)}><For each={options([16, 20, 28, 36, 48], propertySize())}>{value => <option value={value} selected={value === propertySize()}>{value}</option>}</For></select></Show>
      <Show when={propertyKind() === 'text'}><DrawingTextStylePicker value={propertyTextStyle()} disabled={disabled()} preserveFocus={!!textEditor()} onChange={value => changeStyle({ textStyle: value }, true)} /></Show>
      <Show when={edit()?.error || (strokeVersion(), strokeQueue?.failed())}><button title={t("重试保存")} aria-label={t("重试保存标注")} disabled={disabled()} onClick={confirmText}><Check size={16} /></button><button title={t("载入最新")} aria-label={t("载入最新标注")} disabled={disabled()} onClick={loadLatest}><RotateCcw size={16} /></button><button title={t("取消修改")} aria-label={t("取消属性修改")} disabled={disabled()} onClick={cancelUserEdit}><X size={16} /></button></Show>
      <span class="drawing-divider" />
      <Show when={selectedDrawing()?.rich?.kind === 'table' && props.port.copyTable}><button title={t("复制表格")} aria-label={t("复制表格")} disabled={disabled() || !!edit()} onClick={copySelectedTable}><Copy size={16} /></button><select aria-label={t("表格复制格式")} value={tableFormat()} disabled={disabled() || !!edit()} onChange={event => setTableFormat(event.currentTarget.value as DrawingTableFormat)}><For each={['table', 'markdown', 'csv', 'tsv', 'png'] as DrawingTableFormat[]}>{format => <option value={format} selected={tableFormat() === format}>{format === 'table' ? 'Excel' : format === 'markdown' ? 'Markdown' : format.toUpperCase()}</option>}</For></select></Show>
      </div>
      <div class="drawing-history-actions">
      <button data-caption={t("撤销")} title={t("撤销 · Ctrl + Z")} aria-label={t("撤销绘制")} data-unavailable={controlsLocked() || !!edit() || !historyAvailable(false)} disabled={historyDisabled() || !!edit() || !historyAvailable(false)} onClick={() => history(false)}><Undo2 size={17} /></button>
      <button data-caption={t("重做")} title={t("重做 · Ctrl + Shift + Z")} aria-label={t("重做绘制")} data-unavailable={controlsLocked() || !!edit() || !historyAvailable(true)} disabled={historyDisabled() || !!edit() || !historyAvailable(true)} onClick={() => history(true)}><Redo2 size={17} /></button>
      <button ref={trashButton} class="drawing-trash" classList={{'object-dragging':objectDragging(),'object-trash-over':trashOver()}} data-unavailable={!!edit() || (!objectDragging()&&(!(selectedDrawing() || props.port.stored?.().some(value=>value.id===selected()))||Boolean(props.blackboard&&selectedDrawing()&&blackboardImage(selectedDrawing()!))))} data-caption={t("删除")} title={props.blackboard?t('拖入删除'):t("删除对象 · Delete")} aria-label={props.blackboard?t('删除对象'):t("删除绘制对象")} disabled={disabled() || !!edit() || (!objectDragging()&&(!(selectedDrawing() || props.port.stored?.().some(value=>value.id===selected()))||Boolean(props.blackboard&&selectedDrawing()&&blackboardImage(selectedDrawing()!))))} onClick={remove}><Trash2 size={16} /></button>
      <Show when={props.imageExport !== false}><ImageExportButton scope={strokeSource()} disabled={disabled()} onRegister={open => { openImageExport = open; return () => { if (openImageExport === open) openImageExport = undefined; }; }} onSelect={variant => void done(false, false, false, variant)} /></Show>
      <Show when={props.onPin}><button data-caption={t("贴图")} title={t("贴图 · P")} aria-label={t("贴图")} disabled={disabled()} onClick={() => void pin()}><Pin size={17} /></button></Show>
      <button data-caption={t("完成")} class="drawing-done" title={props.retainOnDone ? t("完成") : t("完成并复制")} aria-label={props.retainOnDone ? t("完成") : t("完成并复制")} disabled={props.blackboard && props.inputLocked ? props.busy || pending() || finishing() : disabled()} onClick={() => void done()}><Check size={18} /></button>
      </div>
      </div>
    </div>
    <Show when={props.blackboard}><BlackboardImageControls surface={()=>svg} box={props.box} frame={props.port.frame} drawings={drawings} preview={previewDrawing} revision={props.port.revision} source={()=>JSON.stringify([identity(),props.port.sourceIdentity(),props.port.sourceSize(),props.port.frame(),props.box])} disabled={()=>pending()||props.busy||!!props.inputLocked||finishing()||!!edit()} gestureActive={()=>!!cancelGesture} onMove={(event,drawing)=>begin(event,drawing)} onUpdate={(drawing,revision,active)=>commit({expectedRevision:revision,type:'update_drawing',drawing},true,active)} onPreview={setDraft} onZooming={setImageZooming} onRegisterFlush={flush=>{flushImages=flush;return()=>{if(flushImages===flush)flushImages=undefined;};}} onError={props.onError}/></Show>
  </div>;
}
