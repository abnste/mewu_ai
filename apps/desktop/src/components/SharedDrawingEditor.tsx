// SPDX-License-Identifier: MPL-2.0
import { t } from '../i18n';
import { nativeSelectOwnsEscape } from '../native-select-escape';
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { ArrowUpRight, BrushCleaning, Check, Circle, Copy, Eraser, Grid2X2, Hash, Highlighter, LoaderCircle, Minus, MousePointer2, Pencil, Pin, Redo2, RotateCcw, Scissors, Square, Trash2, Type, Undo2, X } from 'lucide-solid';
import type { Drawing, DrawingKind, DrawingPoint, ManualDrawingKind } from '../contracts';
import { constrainedEnd, drawingBounds, drawingMoveDelta, drawingOrder, drawingResizeHandles, resizedDrawingPoints, validNextNumber } from "./drawing-geometry";
import { finishDrawing, outputDrawing } from "../capture-completion";
import { eraserHit, ObjectEraser } from "./drawing-eraser";
import { changeProperty, drawingDraftKey, drawingDrafts, drawingPropertyCommand, propertyDraft, settleDrawingEdit, strokeKinds, type DrawingPropertyDraft } from "./drawing-properties";
import { drawingFrame, drawingPointerSamples, liveDrawing } from './drawing-pointer';
import type { RegisterDrawingFlush } from "../drawing-flush";
import type { DrawingTableFormat } from "../drawing-layout-preview";
import "./drawing.css";

import type { SharedDrawingEditorProps as Props, EditorDrawingAction } from '../drawing-editor-port';
const labels: Record<ManualDrawingKind, string> = { pen: '画笔', line: '直线', arrow: '箭头', rect: '矩形', ellipse: '椭圆', text: '文字', highlighter: '荧光笔', number: '序号', mosaic: '马赛克' };
const icons = { pen: Pencil, line: Minus, arrow: ArrowUpRight, rect: Square, ellipse: Circle, text: Type, highlighter: Highlighter, number: Hash, mosaic: Grid2X2 };
const clamp = (value: number, low: number, high: number) => Math.max(low, Math.min(high, value));
export default function SharedDrawingEditor(props: Props) {
  let svg!: SVGSVGElement, toolbar!: HTMLDivElement, toolsRow!: HTMLDivElement, textarea: HTMLTextAreaElement | undefined;
  let cancelGesture: (() => void) | undefined, refreshConstraint: ((shift: boolean) => void) | undefined, disposed = false, serial = 0, numberPreference = 0;
  let flight: Promise<boolean> | undefined;
  let tableCopy: Promise<void> | undefined;
  let eraseSource: (()=>boolean) | undefined;
  const [erasing,setErasing]=createSignal(false);
  const numberChanges = new Map<string, { previous: number; next: number; preference: number }>();
  type Tool = ManualDrawingKind | 'select' | 'eraser' | 'extract' | 'heal';
  const [tool, setTool] = createSignal<Tool>(props.tools[0] ?? 'select');
  const [rasterPreview, setRasterPreview] = createSignal<{mode:'extract'|'heal';points:DrawingPoint[];width:number}>();
  const [healWidth, setHealWidth] = createSignal(32);
  const [tableFormat, setTableFormat] = createSignal<DrawingTableFormat>('table');
  const [color, setColor] = createSignal(props.blackboard ? '#E8ECF4' : '#ee4848'), [width, setWidth] = createSignal(4), [fontSize, setFontSize] = createSignal(20);
  const firstNumber = Math.min(9999, Math.max(0, ...(props.port.drawings()).filter(value => value.kind === 'number').map(value => validNextNumber(value.text ?? '') ?? 0)) + 1);
  const [highlightWidth, setHighlightWidth] = createSignal(18), [nextNumber, setNextNumber] = createSignal(String(firstNumber)), [numberDiameter, setNumberDiameter] = createSignal(28);
  const [mosaicReady, setMosaicReady] = createSignal(false), [mosaicFailed, setMosaicFailed] = createSignal(false), [mosaicRetry, setMosaicRetry] = createSignal(0), [finishing, setFinishing] = createSignal(false);
  const [selected, setSelected] = createSignal(''), [draft, setDraft] = createSignal<Drawing>();
  const [storedDraft,setStoredDraft]=createSignal<{id:string;bounds:{x:number;y:number;width:number;height:number}}>();
  let selectionSerial=0; const [selectionReading,setSelectionReading]=createSignal(false);
  const [pending, setPending] = createSignal(false), [edit, setEdit] = createSignal<DrawingPropertyDraft>();
  const activeDraftId = createMemo(() => draft()?.id), activeEditId = createMemo(() => edit()?.drawing.id), storedPreviewId = createMemo(() => storedDraft()?.id);
  const textEditor = () => edit()?.mode === 'text' ? edit() : undefined;
  const text = () => textEditor()?.drawing.text ?? '';
  const cacheKey = () => props.port.draftKey();
  const [viewport, setViewport] = createSignal({ width: innerWidth, height: innerHeight });
  const [toolbarSize, setToolbarSize] = createSignal({ width: 680, height: 118 });
  const [toolsWidth, setToolsWidth] = createSignal(680);
  // Dock from the stable tool bubble, not the changing property bubble.
  const rightDocked = () => !props.blackboard && props.box.x + toolsWidth() + 6 > viewport().width;
  const [surfaceSize, setSurfaceSize] = createSignal({ width: props.box.width, height: props.box.height });
  const drawings = () => props.port.drawings();
  const drawingIndex = createMemo(() => new Map(drawings().map(value => [value.id, value])));
  const storedIndex = createMemo(() => new Map((props.port.stored?.() ?? []).map(value => [value.id, value])));
  const selectedDrawing = () => drawingIndex().get(selected());
  const propertyTarget = () => edit()?.drawing ?? (tool() === 'select' && selectedDrawing() && props.port.canStyle(selectedDrawing()!) ? selectedDrawing() : undefined);
  const propertyKind = () => propertyTarget()?.kind ?? tool();
  const propertyColor = () => propertyTarget()?.color ?? color();
  const propertyWidth = () => propertyTarget()?.strokeWidth ?? (tool() === 'highlighter' ? highlightWidth() : width());
  const propertySize = () => propertyTarget()?.fontSize ?? (tool() === 'number' ? numberDiameter() : fontSize());
  const options = (defaults: number[], current: number) => [...new Set([...defaults, current])].sort((a, b) => a - b);
  const disabled = () => pending() || props.busy || props.inputLocked || finishing();
  const blockSize = () => props.port.mosaic?.blockSize() ?? 8;
  const editableIds = () => new Set([...drawings().filter(value => props.port.canSelect(value)).map(value => value.id),...(props.port.stored?.().map(value=>value.id)??[])]);
  const historyAvailable = (redo: boolean) => props.port.historyAvailable(redo);
  const previewDrawing = () => draft() ?? edit()?.drawing;
  const addedPreviewId = createMemo(() => {
    const preview = previewDrawing();
    return preview && !drawingIndex().has(preview.id) && !storedIndex().has(preview.id) && (preview.kind !== 'text' || preview.text?.trim()) ? preview.id : undefined;
  });
  const addedPreviewMosaic = createMemo(() => previewDrawing()?.kind === 'mosaic');
  const drawingIds = createMemo(() => {
    const ordered = drawingOrder(drawings()).filter(value => !storedIndex().has(value.id));
    const ids = [...storedIndex().keys(), ...ordered.map(value => value.id)];
    const addedId = addedPreviewId();
    if (addedId) {
      if (addedPreviewMosaic()) ids.splice(storedIndex().size + ordered.filter(value => value.kind === 'mosaic').length, 0, addedId); else ids.push(addedId);
    }
    return ids;
  }, undefined, { equals: (a, b) => a.length === b.length && a.every((id, i) => id === b[i]) });
  const resizeDrawing = () => {
    if (props.documentOnly || tool() !== 'select' || disabled() || edit()) return;
    const drawing = previewDrawing()?.id === selected() ? previewDrawing() : selectedDrawing();
    return drawing && props.port.canStyle(drawing) ? drawing : undefined;
  };
  const resizeHandles = () => { const drawing = resizeDrawing(), size = surfaceSize(); return drawing && size.width > 0 && size.height > 0 ? drawingResizeHandles(drawing) : []; };
  const handleSize = () => ({ x: 10 * props.port.frame().width / surfaceSize().width, y: 10 * props.port.frame().height / surfaceSize().height });
  const gestureIdentity = () => JSON.stringify([cacheKey(), props.port.sourceIdentity(), props.port.sourceSize(), props.port.revision(), props.box]);
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
  function storeEdit(value: DrawingPropertyDraft): boolean {
    try { drawingDrafts.remember(cacheKey(), value); setEdit(value); return true; }
    catch (error) { report(error); return false; }
  }
  function cancelEdit() { const value = edit(); if (value) drawingDrafts.delete(cacheKey(), value); setEdit(undefined); setDraft(undefined); }
  function cancelUserEdit(){if(!disabled())cancelEdit();}
  const setText = (value: string) => { if(disabled())return; const current = textEditor(); if (current) storeEdit(changeProperty(current, { text: value })); };
  function commit(command: EditorDrawingAction, controlled = false, active: () => boolean = () => true, persisted: () => void = () => {}): Promise<boolean> {
    if (pending() || (!controlled && (props.busy || props.inputLocked)) || !active()) return Promise.resolve(false);
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
    if (!(await saveText()) || disposed || origin !== identity()) return;
    setDraft(undefined); if (next !== 'select') setSelected('');
    if (next === 'mosaic' && mosaicFailed()) setMosaicRetry(value => value + 1); setTool(next);
  }
  function openText(value: Drawing, existing: boolean) {
    if (disabled() || edit()) return;
    if (!storeEdit(propertyDraft(value, props.port.revision(), 'text', existing))) return;
    setSelected(value.id); setTool('text');
    queueMicrotask(() => { if (!disposed && textEditor()?.drawing.id === value.id) textarea?.focus(); });
  }
  function loadLatest() {
    const value = edit(); if (!value || disabled()) return;
    const latest = drawings().find(drawing => drawing.id === value.drawing.id);
    if (!latest) { props.onError('标注已删除'); return; }
    cancelEdit();
    if (value.mode === 'text') openText(latest, true); else setSelected(latest.id);
  }
  function changeStyle(patch: { color?: string; strokeWidth?: number; fontSize?: number }, submit: boolean) {
    if (disabled() || cancelGesture) return;
    const selectedObject = propertyTarget();
    if (selectedObject && !props.port.canStyle(selectedObject)) return;
    if (selectedObject) {
      const value = edit() ?? propertyDraft(selectedObject, props.port.revision());
      if (storeEdit(changeProperty(value, patch)) && submit && value.mode !== 'text') void saveText();
    } else {
      if (patch.color !== undefined) setColor(patch.color);
      if (patch.strokeWidth !== undefined) (tool() === 'highlighter' ? setHighlightWidth : setWidth)(patch.strokeWidth);
      if (patch.fontSize !== undefined) (tool() === 'number' ? setNumberDiameter : setFontSize)(patch.fontSize);
    }
  }
  function editNumber(value: string) { if(disabled())return; numberPreference++; setNextNumber(value); }
  function beginErase(event: PointerEvent, secondary = false) {
    const pointerId = event.pointerId, origin = identity();
    setErasing(true);
    const originalTool = tool(), source = JSON.stringify([props.port.sourceIdentity(),props.port.sourceSize(),props.port.frame(),props.box]);
    eraseSource = () => identity() === origin && tool() === originalTool && source === JSON.stringify([props.port.sourceIdentity(),props.port.sourceSize(),props.port.frame(),props.box]);
    let stopped = false;
    const cleanup = () => {
      if (stopped) return; stopped = true;
      svg.removeEventListener('pointermove', moved); svg.removeEventListener('pointerup', released);
      svg.removeEventListener('pointercancel', canceled); svg.removeEventListener('lostpointercapture', canceled);
      cancelGesture = undefined;
      eraseSource = undefined;
      setErasing(false);
      if (svg.hasPointerCapture(pointerId)) svg.releasePointerCapture(pointerId);
    };
    const eraser = new ObjectEraser({ x: event.clientX, y: event.clientY }, {
      current: () => !disposed && !stopped && Boolean(eraseSource?.()) && (secondary ? props.blackboard === true : tool() === 'eraser') && !props.busy && !props.inputLocked && !finishing(),
      hit: point => eraserHit(svg, point, editableIds()),
      remove: id => commit({ ...target(), type: 'remove_drawing', drawingId: id }),
      stopped: cleanup,
    });
    const moved = (next: PointerEvent) => { if (next.pointerId === pointerId) { next.preventDefault(); if (!(next.buttons & (secondary ? 2 : 1))) eraser.stop(); else eraser.move({ x: next.clientX, y: next.clientY }); } };
    const released = (next: PointerEvent) => { if (next.pointerId === pointerId) eraser.stop(); };
    const canceled = (next: PointerEvent) => { if (next.pointerId === pointerId) eraser.stop(); };
    cancelGesture = () => eraser.stop();
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
  function begin(event: PointerEvent) {
    event.stopPropagation();
    if (props.blackboard && event.button === 2) {
      event.preventDefault();
      if (!disabled() && !textEditor() && !edit() && !cancelGesture) { svg.focus({preventScroll:true}); beginErase(event,true); }
      return;
    }
    if (event.button !== 0 || disabled() || textEditor() || cancelGesture) return;
    event.preventDefault();
    if (edit()) { void saveText(); return; }
    svg.focus({ preventScroll: true });
    if (tool() === 'eraser') { beginErase(event); return; }
    if (tool() === 'extract' || tool() === 'heal') { beginRaster(event,tool() as 'extract'|'heal'); return; }
    const initialBox = svg.getBoundingClientRect();
    if (initialBox.width <= 0 || initialBox.height <= 0) return;
    const initialFrame = props.port.frame();
    const start = point(event, true, initialBox, initialFrame), kind = tool(), revision = props.port.revision(), origin = gestureSource();
    const hitId = event.target instanceof Element ? event.target.closest('[data-drawing-id]')?.getAttribute('data-drawing-id') : undefined;
    const hit = hitId && editableIds().has(hitId) ? drawings().find(value => value.id === hitId) : undefined;
    if (kind === 'mosaic' && !mosaicReady()) return;
    const stored=props.port.stored?.().find(value=>value.id===hitId);
    if(kind==='select'&&stored){setSelected(stored.id);void ensureSelection(stored.id);beginStored(event,stored.id);return;}
    if(kind==='text'&&stored?.kind==='text'){setSelected(stored.id);void ensureSelection(stored.id).then(value=>{if(value)openText(value,true);});return;}
    if (kind === 'text') {
      if (hit?.kind === 'text') openText(hit, true);
      else openText({ id: crypto.randomUUID(), kind, color: color(), strokeWidth: width(), points: [start], fontSize: fontSize(), text: '' }, false);
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
    if (kind === 'select') { setSelected(original?.id ?? ''); if (!original) return; }
    else setSelected('');
    let value: Drawing = original ? { ...original, points: original.points.map(value => ({ ...value })) } : { id: crypto.randomUUID(), kind: kind as DrawingKind, color: color(), strokeWidth: kind === 'mosaic' ? blockSize() : kind === 'highlighter' ? highlightWidth() : width(), points: kind === 'pen' || kind === 'highlighter' ? [start] : [start, start] };
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
    const current = () => !disposed && origin === gestureSource() && !disabled() && !edit() && tool() === kind
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
        const dx = drawingMoveDelta(end.x - start.x, bounds.x, bounds.x + bounds.width, props.port.frame().x, props.port.frame().x + props.port.frame().width, props.port.sourceSize().width);
        const dy = drawingMoveDelta(end.y - start.y, bounds.y, bounds.y + bounds.height, props.port.frame().y, props.port.frame().y + props.port.frame().height, props.port.sourceSize().height);
        value = { ...original, points: original.points.map(point => ({ ...point, x: point.x + dx, y: point.y + dy })) }; changed = Math.abs(dx) + Math.abs(dy) > .1;
      } else if (kind === 'pen' || kind === 'highlighter') {
        const last = value.points[value.points.length - 1];
        if (end.x !== last.x || end.y !== last.y) {
          if (value.points.length >= 4096) {
            if (props.port.rejectPointOverflow) { finish(false); props.onError('笔迹最多 4096 个点'); return; }
            // Preserve the release endpoint under the existing screenshot point bound.
            if (terminal) value.points[value.points.length - 1] = end;
          } else value.points.push(end);
        }
        return;
      } else value = { ...value, points: [start, constrainedEnd(kind as DrawingKind, start, end, shift, props.port.frame())] };
      setDraft(value);
    };
    const publish = () => {
      if (ended) return;
      if (!current() || !surfaceCurrent()) { finish(false); return; }
      if (latest) { const next = latest; latest = undefined; update(next.end, next.shift); }
      else if (!original && (kind === 'pen' || kind === 'highlighter')) setDraft({ ...value });
    };
    const previewFrame = drawingFrame(publish);
    const moved = (next: PointerEvent) => {
      if (next.pointerId !== pointerId) return;
      if (!current() || (next.type === 'pointermove' && !(next.buttons & 1))) { finish(false); return; }
      if (!dragStarted) {
        if (Math.abs(next.clientX - event.clientX) <= 4 && Math.abs(next.clientY - event.clientY) <= 4) return;
        dragStarted = true;
        if (next.buttons & 1) try { svg.setPointerCapture(pointerId); } catch { /* Window listeners still finish this gesture. */ }
      }
      next.preventDefault();
      if (!original && (kind === 'pen' || kind === 'highlighter')) {
        for (const sample of drawingPointerSamples(next)) { if (sample.pointerId !== pointerId) continue; update(point(sample, true, initialBox, initialFrame), sample.shiftKey, next.type === 'pointerup'); if (ended) return; }
      } else latest = { end: point(next, true, initialBox, initialFrame), shift: next.shiftKey };
      if (next.type === 'pointerup') publish(); else previewFrame.request();
    };
    refreshConstraint = shift => { if (!original && ['line', 'arrow', 'rect', 'ellipse'].includes(kind)) { latest = { end: latest?.end ?? lastEnd, shift }; previewFrame.request(); } };
    const cleanup = () => {
      previewFrame.cancel(); latest = undefined;
      window.removeEventListener('pointermove', moved); window.removeEventListener('pointerup', released);
      window.removeEventListener('pointercancel', canceled); svg.removeEventListener('lostpointercapture', canceled);
      cancelGesture = undefined; refreshConstraint = undefined; if (svg.hasPointerCapture(pointerId)) svg.releasePointerCapture(pointerId);
    };
    const finish = (save: boolean) => {
      if (ended) return;
      const valid = current() && surfaceCurrent(); ended = true; cleanup();
      if (!save || !valid || (original && !changed)) { setDraft(undefined); return; }
      const [a, b] = value.points;
      if (!original && (((kind === 'arrow' || kind === 'line') && Math.hypot(b.x - a.x, b.y - a.y) < 1) || (['rect', 'ellipse', 'mosaic'].includes(kind) && (Math.abs(b.x - a.x) < 1 || Math.abs(b.y - a.y) < 1)))) { setDraft(undefined); return; }
      // A receipt owns an immutable snapshot; late input cannot mutate its point array.
      value = { ...value, points: value.points.map(point => ({ ...point })) };
      void commit({ ...target(revision), type: original ? 'update_drawing' : 'add_drawing', drawing: value });
    };
    const released = (next: PointerEvent) => { if (next.pointerId === pointerId) { moved(next); finish(true); } };
    const canceled = (next: PointerEvent) => { if (next.pointerId === pointerId) finish(false); };
    cancelGesture = () => finish(false);
    window.addEventListener('pointermove', moved); window.addEventListener('pointerup', released); window.addEventListener('pointercancel', canceled); svg.addEventListener('lostpointercapture', canceled);
    if (!original) try { svg.setPointerCapture(pointerId); } catch { finish(false); }
  }
  async function saveText(controlled = false, active: () => boolean = () => true): Promise<boolean> {
    const value = edit(); if (!value) return true;
    if (pending() || (!controlled && props.busy) || !active()) return false;
    const key = cacheKey(), origin = identity();
    try {
      const command = props.port.propertyCommand(value);
      if (!command) { cancelEdit(); return true; }
      let persisted = false;
      const saved = await commit(command, controlled, active, () => { persisted = true; drawingDrafts.delete(key, value); });
      if (persisted && !disposed && origin === identity() && edit() === value) { setEdit(undefined); setSelected(props.port.persistedId?.(value.drawing.id) ?? value.drawing.id); }
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
  async function done(closeSpace = false, copy = true, exitDrawing = true) {
    if (disabled() || cancelGesture) return;
    const currentIdentity = identity(); setFinishing(true);
    try {
      if (props.blackboard && exitDrawing && props.onFinishBlackboard) {
        if (await saveText() && !disposed && currentIdentity === identity()) await props.onFinishBlackboard();
        return;
      }
      await finishDrawing({ saveText, current: () => !disposed && currentIdentity === identity(), copy: () => exitDrawing && props.retainOnDone ? Promise.resolve(true) : props.onExport(copy, closeSpace), close: () => { if (exitDrawing) props.onClose(); } });
    }
    catch (error) { if (!disposed) report(error); }
    finally { if (!disposed) setFinishing(false); }
  }
  const remove = () => { const value = selectedDrawing() ?? props.port.stored?.().find(value=>value.id===selected()); if (value && !disabled() && !cancelGesture && !edit()) void commit({ ...target(), type: 'remove_drawing', drawingId: value.id }).then(ok => { if (ok) setSelected(''); }); };
  async function pin() {
    if (disabled() || cancelGesture || !props.onPin) return;
    const currentIdentity = identity(); setFinishing(true);
    try { await outputDrawing({ saveText, current: () => !disposed && currentIdentity === identity(), output: () => props.onPin?.() ?? Promise.resolve(false) }); }
    catch (error) { if (!disposed) report(error); }
    finally { if (!disposed) setFinishing(false); }
  }
  function history(redo: boolean) {
    if (disabled() || edit() || cancelGesture) return;
    if (!historyAvailable(redo)) return;
    const entry = props.port.historyStep?.(redo);
    const number = entry && !('batch' in entry) && !entry.before && entry.after?.kind === 'number' ? numberChanges.get(entry.after.id) : undefined;
    void commit({ ...target(), type: redo ? 'redo_drawing' : 'undo_drawing' }).then(ok => {
      if (ok && number && number.preference === numberPreference && validNextNumber(nextNumber()) === (redo ? number.previous : number.next)) setNextNumber(String(redo ? number.next : number.previous));
    });
  }
  function keyboard(event: KeyboardEvent) {
    if ((event.target instanceof Element ? event.target : document.activeElement)?.closest(props.keyboardIgnore ?? '.video-trim-popover,.artifact-video,[data-run-journal]')) return;
    if (props.inputLocked) return;
    if (document.querySelector('.drawing-drafts-dialog')) return;
    if (event.isComposing) return;
    if (event.key === 'Shift') { refreshConstraint?.(event.shiftKey); return; }
    if (event.type === 'keyup') return;
    if (nativeSelectOwnsEscape(event)) return;
    if (event.key === 'Escape') {
      event.preventDefault(); event.stopImmediatePropagation();
      if (cancelGesture) cancelGesture(); else if (!disabled() && edit()) cancelEdit(); else if (!disabled()) { if (props.blackboard) void done(); else props.onClose(); }
      return;
    }
    if (event.target instanceof Element && event.target.closest('input,textarea,select,[contenteditable="true"]')) return;
    if (event.repeat) return;
    if (event.key.toLowerCase() === 'p' && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey && props.onPin) { event.preventDefault(); event.stopImmediatePropagation(); void pin(); return; }
    if (event.key === 'Enter' && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey && !(event.target instanceof Element && event.target.closest('button'))) { event.preventDefault(); event.stopImmediatePropagation(); void done(true); return; }
    if (['c', 's'].includes(event.key.toLowerCase()) && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey) { event.preventDefault(); event.stopImmediatePropagation(); void done(false, event.key.toLowerCase() === 'c', false); return; }
    if ((event.ctrlKey || event.metaKey) && ['z', 'y'].includes(event.key.toLowerCase())) { event.preventDefault(); event.stopImmediatePropagation(); history(event.shiftKey || event.key.toLowerCase() === 'y'); }
    else if (event.key === 'Delete' || event.key === 'Backspace') { event.preventDefault(); event.stopImmediatePropagation(); remove(); }
  }
  const resize = () => { cancelGesture?.(); setViewport({ width: innerWidth, height: innerHeight }); };
  const blur = () => cancelGesture?.();
  const identity = createMemo(cacheKey);
  createEffect(on(identity, () => {
    cancelGesture?.(); setDraft(undefined);
    const saved = props.documentOnly ? undefined : drawingDrafts.get(cacheKey()); setEdit(saved);
    if (saved) { setSelected(saved.drawing.id); setTool(saved.mode === 'text' ? 'text' : 'select'); }
    else setSelected('');
  }));
  createEffect(() => { if (props.busy || props.inputLocked) cancelGesture?.(); });
  const gestureSource = createMemo(gestureIdentity);
  createEffect(on(gestureSource, () => { if (!eraseSource?.()) cancelGesture?.(); }));
  createEffect(() => { if (svg) { const observer = new ResizeObserver(() => { const box = svg.getBoundingClientRect(); const before = surfaceSize(); if (before.width !== box.width || before.height !== box.height) { cancelGesture?.(); setSurfaceSize({ width: box.width, height: box.height }); } }); observer.observe(svg); onCleanup(() => observer.disconnect()); } });
  createEffect(() => { if (toolbar) { const observer = new ResizeObserver(() => { setToolbarSize({ width: toolbar.offsetWidth, height: toolbar.offsetHeight }); setToolsWidth(toolsRow.offsetWidth); }); observer.observe(toolbar); observer.observe(toolsRow); onCleanup(() => observer.disconnect()); } });
  const mosaicIdentity = createMemo(() => tool() === 'mosaic' && props.port.mosaic ? JSON.stringify([props.port.sourceIdentity(), blockSize(), mosaicRetry()]) : '');
  createEffect(on(mosaicIdentity, key => {
    setMosaicReady(false); setMosaicFailed(false); let alive = true;
    if (key && props.port.mosaic) void props.port.mosaic.prepare().then(() => { if (alive) setMosaicReady(true); }).catch(error => { if (alive) { setMosaicFailed(true); report(error); } });
    onCleanup(() => { alive = false; });
  }));
  document.addEventListener('keydown', keyboard, true); document.addEventListener('keyup', keyboard, true); window.addEventListener('resize', resize); window.addEventListener('blur', blur);
  const unregisterFlush = props.onRegisterFlush?.(async active => {
    const origin = identity(); cancelGesture?.();
    if (tableCopy) await tableCopy;
    if (flight && !(await flight)) throw new Error('标注尚未保存');
    if (!active() || disposed || origin !== identity()) throw new Error('标注编辑已切换');
    if (!(await saveText(true, () => active() && !disposed && origin === identity()))) throw new Error('标注尚未保存');
  });
  const unregisterHistory = props.onRegisterHistory?.({
    undo:()=>history(false),redo:()=>history(true),
    canUndo:()=>!disabled()&&!edit()&&!erasing()&&!cancelGesture&&historyAvailable(false),
    canRedo:()=>!disabled()&&!edit()&&!erasing()&&!cancelGesture&&historyAvailable(true),
  });
  onCleanup(()=>unregisterHistory?.());
  onCleanup(() => { unregisterFlush?.(); disposed = true; serial++; cancelGesture?.(); document.removeEventListener('keydown', keyboard, true); document.removeEventListener('keyup', keyboard, true); window.removeEventListener('resize', resize); window.removeEventListener('blur', blur); });
  function DrawingItem(item: { id: string }) {
    const isDraft = createMemo(() => activeDraftId() === item.id), isEdit = createMemo(() => activeEditId() === item.id);
    const value = createMemo(() => isDraft() ? draft() : isEdit() ? edit()?.drawing : drawingIndex().get(item.id));
    const drawing = liveDrawing(item.id, () => value()!);
    const isPreview = createMemo(() => isDraft() || isEdit());
    const isSelected = createMemo(() => selected() === item.id);
    const interactive = createMemo(() => (props.blackboard || ['select', 'eraser', 'text'].includes(tool())) && Boolean(value() && props.port.canSelect(drawing)));
    const render = () => <>{props.port.render(drawing, interactive(), isSelected(), isPreview())}</>;
    const storedBounds = createMemo(() => storedPreviewId() === item.id ? storedDraft()?.bounds : undefined);
    const stored = createMemo(() => storedIndex().has(item.id) ? props.port.renderStored?.(item.id, ['select', 'eraser', 'text'].includes(tool()), isSelected(), storedBounds()) : undefined);
    return <Show when={storedIndex().has(item.id)} fallback={<Show when={value()}>{_ => render()}</Show>}>
      {_ => <Show when={isPreview()} fallback={stored()}>{_ => render()}</Show>}
    </Show>;
  }
  return <div class="drawing-editor" classList={{'drawing-blackboard':props.blackboard}} onContextMenu={event=>{if(props.blackboard){event.preventDefault();event.stopPropagation();}}} onPointerDown={event => event.stopPropagation()}>
    <svg ref={svg} tabindex={0} class="drawing-edit-surface" classList={{ 'drawing-select-mode': tool() === 'select', 'drawing-text-mode': tool() === 'text', 'drawing-eraser-mode': tool() === 'eraser' }} style={{ left: `${props.box.x}px`, top: `${props.box.y}px`, width: `${props.box.width}px`, height: `${props.box.height}px` }} viewBox={`${props.port.frame().x} ${props.port.frame().y} ${props.port.frame().width} ${props.port.frame().height}`} preserveAspectRatio="none" onPointerDown={begin} onDblClick={doubleClick} aria-label={t("绘制区域")}>
      <For each={drawingIds()}>{id => <DrawingItem id={id} />}</For>
      <Show when={rasterPreview()}>{value=> <Show when={value().mode==='extract'} fallback={<polyline points={value().points.map(p=>`${p.x},${p.y}`).join(' ')} fill="none" stroke="#428fff" stroke-opacity=".38" stroke-width={value().width} stroke-linecap="round" stroke-linejoin="round" pointer-events="none" />}><rect x={Math.min(value().points[0].x,value().points.at(-1)!.x)} y={Math.min(value().points[0].y,value().points.at(-1)!.y)} width={Math.abs(value().points.at(-1)!.x-value().points[0].x)} height={Math.abs(value().points.at(-1)!.y-value().points[0].y)} fill="none" stroke="#428fff" stroke-width="1" vector-effect="non-scaling-stroke" pointer-events="none" /></Show>}</Show>
      <For each={resizeHandles()}>{(point, index) => <ellipse class="drawing-resize-handle" cx={point.x} cy={point.y} rx={handleSize().x / 2} ry={handleSize().y / 2} vector-effect="non-scaling-stroke" style={{ cursor: resizeDrawing()?.kind === 'line' || resizeDrawing()?.kind === 'arrow' ? 'move' : index() % 2 ? 'nesw-resize' : 'nwse-resize' }} aria-label={resizeDrawing()?.kind === 'line' || resizeDrawing()?.kind === 'arrow' ? t('调整端点') : t('缩放图形')} onPointerDown={event => beginResize(event, index())} />}</For>
    </svg>
    <div ref={toolbar} class="drawing-toolbar" classList={{ 'drawing-dock-right': rightDocked() }} role="toolbar" aria-label={t("绘制工具")} style={position()}>
      <div ref={toolsRow} class="drawing-tools-row">
      <button data-caption={t("选择")} title={t("选择并移动")} aria-label={t("选择并移动绘制对象")} classList={{ selected: tool() === 'select' }} disabled={disabled()} onClick={() => choose('select')}><MousePointer2 size={17} /></button>
      <button data-caption={t("橡皮")} title={t("橡皮")} aria-label={t("橡皮")} classList={{ selected: tool() === 'eraser' }} disabled={disabled()} onClick={() => choose('eraser')}><Eraser size={17} /></button>
      <For each={props.tools}>{kind => { const Icon = icons[kind]; return <button data-caption={t(labels[kind])} title={t(labels[kind])} aria-label={t(labels[kind])} classList={{ selected: tool() === kind }} disabled={disabled()} onClick={() => choose(kind)}><Icon size={18} /></button>; }}</For>
      <Show when={props.port.raster}><span class="drawing-divider" /><button data-caption={t("无痕提取")} title={t("无痕提取")} aria-label={t("无痕提取")} classList={{selected:tool()==='extract'}} disabled={disabled()} onClick={()=>choose('extract')}><Scissors size={18}/></button><button data-caption={t("涂抹消除")} title={t("涂抹消除")} aria-label={t("涂抹消除")} classList={{selected:tool()==='heal'}} disabled={disabled()} onClick={()=>choose('heal')}><BrushCleaning size={18}/></button></Show>
      <Show when={selectionReading()}><LoaderCircle size={15} class="spin" /></Show>
      {props.extraTools?.()}
      </div>
      <div class="drawing-properties-row">
      <Show when={!['mosaic', 'eraser', 'select', 'rich','extract','heal'].includes(propertyKind())}><label class="drawing-color" title={t("颜色")}><input type="color" aria-label={t("绘制颜色")} value={propertyColor()} disabled={disabled()} onInput={event => changeStyle({ color: event.currentTarget.value }, false)} onChange={event => changeStyle({ color: event.currentTarget.value }, true)} /></label></Show>
      <Show when={tool()==='heal'}><select aria-label={t("画笔大小")} title={t("画笔大小")} value={healWidth()} disabled={disabled()} onChange={event=>setHealWidth(Number(event.currentTarget.value))}><For each={[16,24,32,48,64]}>{size=><option value={size} selected={size===healWidth()}>{size}px</option>}</For></select></Show>
      <Show when={strokeKinds.includes(propertyKind() as DrawingKind)}><select aria-label={t("线宽")} title={t("线宽")} value={propertyWidth()} disabled={disabled()} onChange={event => changeStyle({ strokeWidth: Number(event.currentTarget.value) }, true)}><For each={options(propertyKind() === 'highlighter' ? [12, 18, 24, 32] : [2, 4, 8, 12], propertyWidth())}>{value => <option value={value} selected={value === propertyWidth()}>{value}px</option>}</For></select></Show>
      <Show when={tool() === 'mosaic' && !mosaicReady() && !mosaicFailed()}><LoaderCircle size={15} class="spin" /></Show>
      <Show when={tool() === 'number'}><input class="drawing-number" type="number" aria-label={t("下个序号")} title={t("下个序号")} min="1" max="9999" step="1" value={nextNumber()} disabled={disabled()} onInput={event => editNumber(event.currentTarget.value)} /></Show>
      <Show when={propertyKind() === 'number'}><select aria-label={t("序号大小")} title={t("序号大小")} value={propertySize()} disabled={disabled()} onChange={event => changeStyle({ fontSize: Number(event.currentTarget.value) }, true)}><For each={options([22, 28, 36, 48, 64], propertySize())}>{value => <option value={value} selected={value === propertySize()}>{value}</option>}</For></select></Show>
      <Show when={propertyKind() === 'text'}><select aria-label={t("字号")} title={t("字号")} value={propertySize()} disabled={disabled()} onChange={event => changeStyle({ fontSize: Number(event.currentTarget.value) }, true)}><For each={options([16, 20, 28, 36, 48], propertySize())}>{value => <option value={value} selected={value === propertySize()}>{value}</option>}</For></select></Show>
      <Show when={edit()?.mode === 'style' && edit()?.error}><button title={t("重试保存")} aria-label={t("重试保存标注")} disabled={disabled()} onClick={() => void saveText()}><Check size={16} /></button><button title={t("载入最新")} aria-label={t("载入最新标注")} disabled={disabled()} onClick={loadLatest}><RotateCcw size={16} /></button><button title={t("取消修改")} aria-label={t("取消属性修改")} disabled={disabled()} onClick={cancelUserEdit}><X size={16} /></button></Show>
      <span class="drawing-divider" />
      <Show when={selectedDrawing()?.rich?.kind === 'table' && props.port.copyTable}><button title={t("复制表格")} aria-label={t("复制表格")} disabled={disabled() || !!edit()} onClick={copySelectedTable}><Copy size={16} /></button><select aria-label={t("表格复制格式")} value={tableFormat()} disabled={disabled() || !!edit()} onChange={event => setTableFormat(event.currentTarget.value as DrawingTableFormat)}><For each={['table', 'markdown', 'csv', 'tsv', 'png'] as DrawingTableFormat[]}>{format => <option value={format} selected={tableFormat() === format}>{format === 'table' ? 'Excel' : format === 'markdown' ? 'Markdown' : format.toUpperCase()}</option>}</For></select></Show>
      <button data-caption={t("撤销")} title={t("撤销 · Ctrl + Z")} aria-label={t("撤销绘制")} disabled={disabled() || !!edit() || !historyAvailable(false)} onClick={() => history(false)}><Undo2 size={17} /></button>
      <button data-caption={t("重做")} title={t("重做 · Ctrl + Shift + Z")} aria-label={t("重做绘制")} disabled={disabled() || !!edit() || !historyAvailable(true)} onClick={() => history(true)}><Redo2 size={17} /></button>
      <button data-caption={t("删除")} title={t("删除对象 · Delete")} aria-label={t("删除绘制对象")} disabled={disabled() || !!edit() || !(selectedDrawing() || props.port.stored?.().some(value=>value.id===selected()))} onClick={remove}><Trash2 size={16} /></button>
      <Show when={props.onPin}><button data-caption={t("贴图")} title={t("贴图 · P")} aria-label={t("贴图")} disabled={disabled()} onClick={() => void pin()}><Pin size={17} /></button></Show>
      <button data-caption={t("完成")} class="drawing-done" title={props.retainOnDone ? t("完成") : t("完成并复制")} aria-label={props.retainOnDone ? t("完成") : t("完成并复制")} disabled={disabled()} onClick={() => void done()}><Check size={18} /></button>
      </div>
    </div>
    <Show when={textEditor()}>{value => <div class="drawing-text-editor" style={{ left: `${clamp(props.box.x + (value().drawing.points[0].x - props.port.frame().x) * props.box.width / props.port.frame().width, 6, Math.max(6, viewport().width - 294))}px`, top: `${clamp(props.box.y + (value().drawing.points[0].y - props.port.frame().y) * props.box.height / props.port.frame().height, 6, Math.max(6, viewport().height - 160))}px` }}>
      <textarea ref={textarea} aria-label={t("绘制文字")} placeholder={t("输入文字")} value={text()} disabled={pending() || props.inputLocked} onInput={event => setText(event.currentTarget.value)} onKeyDown={event => { if (!event.isComposing && event.keyCode !== 229 && event.key === 'Enter' && (event.ctrlKey || event.metaKey)) { event.preventDefault(); void saveText(); } }} />
      <div><Show when={value().error}><span class="drawing-edit-error" role="status">{value().error}</span><Show when={value().base}><button class="icon-button compact" title={t("载入最新")} aria-label={t("载入最新标注")} disabled={disabled()} onClick={loadLatest}><RotateCcw size={15} /></button></Show></Show><button class="icon-button compact" title={t("取消")} aria-label={t("取消文字")} disabled={disabled()} onClick={cancelUserEdit}><X size={15} /></button><button class="primary-button" title={`${value().base ? t('保存') : t('添加')} · Ctrl + Enter`} aria-label={value().base ? t('保存文字') : t('添加文字')} disabled={disabled() || !text().trim()} onClick={() => void saveText()}><Check size={15} /></button></div>
    </div>}</Show>
  </div>;
}
