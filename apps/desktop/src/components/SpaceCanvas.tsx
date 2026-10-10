// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { nativeSelectOwnsEscape } from '../native-select-escape';
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show, untrack } from 'solid-js';
import { Check, ChevronDown, Circle, Copy, Download, Languages, LoaderCircle, Pencil, Pin, Plus, Table2, TextSelect, Trash2, X } from 'lucide-solid';
import ScrollCaptureIcon from './ScrollCaptureIcon';
import { recordingGrant, recordingTrimGrant } from '../recording-audio';
import type { DrawingCommand, OcrTarget, Reference, Region, RegionGeometry, Scene, Snapshot, SpaceItem } from '../contracts';
import { geometryTarget, overlayRegion, sameGeometry, type RegionGeometryCoordinator, type GeometryView } from '../region-geometry';
import { geometryHistoryKey, type GeometryDirection } from '../region-geometry-history';
import type { PluginContribution, PluginRecord } from '../plugin-contracts';
import { assetUrl, exportRegion as exportNativeRegion, exportTextAsset, native } from '../bridge';
import ArtifactCard from './ArtifactCard';
import { videoCopyKey } from '../video-copy';
import type { VideoArtifactProps } from './VideoArtifact';
import { coreDrawingGrant, coreDrawingTools, videoDrawingGrant as coreVideoDrawingGrant } from '../core-drawing';
import { isCoreReplacementPlugin } from '../plugin-core';
import DrawingLayer from './DrawingLayer';
import DrawingEditor from './DrawingEditor';
import { usesDrawingDocument, type DrawingSession } from '../drawing-document';
import type { DrawingLayoutTarget, DrawingTableFormat } from '../drawing-layout-preview';
import type { RegisterDrawingFlush } from '../drawing-flush';
import OcrTextLayer from './OcrTextLayer';
import TranslationLayer from './TranslationLayer';
import PointerInspector, { type PointerInspectorHandle } from './PointerInspector';
import SelectionCodeCard from './SelectionCodeCard';
import { codeSource } from '../code-scan';
import type { CodeAction, CodeScanView, CodeSource } from '../code-contracts';
import { pointerNative } from '../pointer-bridge';
import { pointerSelectionDimensions } from '../pointer-inspector';
import { type TranslationLanguage, type TranslationPending, type TranslationRequest } from '../translation-request';
import { OcrRequestGate, type OcrRequest } from '../ocr-request';
import { sameOcrTarget } from '../ocr-selection';
import { finishCapture } from '../capture-completion';
import { regionImageProjection } from '../region-image';
import { getWindowSnapMap } from '../window-snap-bridge';
import { SnapMapLoader, SnapSelectionGesture, snapImagePoint, snapResize, snapTarget, type FrameSnapMap } from '../window-snap';
import './capture.css';
import type { RegisterDrawingHistory } from '../drawing-editor-port';

interface Props {
  blackboard?:boolean;blackboardDrawing?:boolean;onBlackboardClose?:()=>Promise<boolean>;onOpenBlackboard?:(itemId:string)=>void;onBlackboardReplay?:(redo:boolean)=>void;
  onRegisterDrawingHistory?:RegisterDrawingHistory;
  showButtonLabels?: boolean;
  onRasterSnapshot?: (snapshot: Snapshot) => void;
  translationLanguage?: TranslationLanguage;
  video?: Omit<VideoArtifactProps, 'item' | 'active' | 'sceneId' | 'busy' | 'grant' | 'annotationBusy' | 'authoringBusy' | 'inputLocked' | 'drawingAllowed' | 'drawingGrant'>;
  authoringBusy?: boolean; inputLocked?: boolean;
  onVideoExport?: (itemId: string) => Promise<void>;
  onVideoCopy?: (itemId: string) => Promise<boolean>;
  codeView?: CodeScanView;
  codeActionBusy?: boolean;
  onCodeSource?: (source: CodeSource | undefined, paused: boolean) => void;
  onCodeAction?: (sourceToken: string, codeId: string, action: CodeAction) => Promise<boolean>;
  onRegisterDrawingFlush?: RegisterDrawingFlush;
  scene: Scene;
  refs: Reference[];
  onAddRegion: (region: Region) => void | Promise<void>;
  geometryController: RegionGeometryCoordinator;
  geometryView: GeometryView;
  onGeometryReplay: (direction: GeometryDirection) => Promise<void>;
  onRemoveRegion: (id: string) => void;
  onReference: (reference: Reference) => void;
  onUpdateItem: (item: SpaceItem) => void | Promise<void>;
  onRemoveItem: (id: string) => void;
  onError: (message: string) => void;
  onSelectionActivity?: (active: boolean) => void;
  onExportActivity?: (active: boolean) => void;
  onSelectionComplete?: (region: Region) => void;
  onFocusComposer?: () => void;
  onBeforeExport?: () => Promise<void>;
  onCopyClose?: (sceneId: string) => Promise<boolean | void>;
  onRecordRegion: (regionId: string) => void;
  onPin?: (pluginId: string, revision: number, contributionId: string, regionId: string) => Promise<boolean>;
  onScrollCapture?: (pluginId: string, revision: number, contributionId: string, target: OcrTarget) => Promise<void>;
  plugins: PluginRecord[];
  onPluginWorkflow: (pluginId: string, revision: number, contributionId: string, regionId: string) => Promise<void>;
  onDrawingCommand: (pluginId: string, pluginRevision: number, contributionId: string, command: DrawingCommand) => Promise<void>;
  onDrawingDocumentCommand: (command: DrawingCommand) => Promise<void>;
  onCopyDrawingTable: (target: DrawingLayoutTarget, format: DrawingTableFormat) => Promise<void>;
  onOcrRequest: (request: OcrRequest) => Promise<Snapshot>;
  onOcrCancel: (requestId: string) => Promise<void>;
  onOcrResult: (snapshot: Snapshot) => void;
  translationPending?: TranslationPending;
  translationNotice?: { regionId: string; text: string };
  onTranslationRequest?: (request: TranslationRequest) => void;
  onTranslationCancel?: () => void;
  onTranslationRemove?: (target: OcrTarget) => Promise<void>;
  onClearTranslationNotice?: () => void;
  busy: boolean;
  maskOpacity: number;
}
interface Box { x: number; y: number; width: number; height: number }
interface PluginEntry { plugin: PluginRecord; contribution: PluginContribution }
type Handle = 'nw' | 'n' | 'ne' | 'w' | 'e' | 'sw' | 's' | 'se';
const HANDLES: Handle[] = ['nw', 'n', 'ne', 'w', 'e', 'sw', 's', 'se'];
const TOOLBAR_WIDTH = 280, TOOLBAR_HEIGHT = 64, EDGE = 6, GAP = 8, TOLERANCE = 24;
const contains = (box: Box, x: number, y: number, padding = 0) => x >= box.x - padding && x <= box.x + box.width + padding && y >= box.y - padding && y <= box.y + box.height + padding;
const overlaps = (a: Box, b: Box) => a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y;
const clamp = (value: number, low: number, high: number) => Math.max(low, Math.min(Math.max(low, high), value));

function projectBackgroundItem(item: SpaceItem, background: Box, viewport: { width: number; height: number }): SpaceItem {
  return { ...item,
    x: (background.x + item.x * background.width) / viewport.width,
    y: (background.y + item.y * background.height) / viewport.height,
    width: item.width * background.width / viewport.width,
    height: item.height * background.height / viewport.height,
  };
}
function persistBackgroundItem(item: SpaceItem, background: Box, viewport: { width: number; height: number }): SpaceItem {
  const width = clamp(item.width * viewport.width / background.width, .000001, 1);
  const height = clamp(item.height * viewport.height / background.height, .000001, 1);
  return { ...item,
    x: clamp((item.x * viewport.width - background.x) / background.width, 0, 1 - width),
    y: clamp((item.y * viewport.height - background.y) / background.height, 0, 1 - height),
    width, height,
  };
}

export default function SpaceCanvas(props: Props) {
  const videoGrant = createMemo(() => {
    if (props.scene.run?.status === 'running') return;
    return recordingTrimGrant();
  });
  const videoDrawingGrant = coreVideoDrawingGrant;
  const captureGrant = recordingGrant;
  let canvas!: HTMLElement;
  let toolbar: HTMLDivElement | undefined;
  let toolbarObserver: ResizeObserver | undefined;
  let hideTimer: ReturnType<typeof setTimeout> | undefined;
  let flashTimer: ReturnType<typeof setTimeout> | undefined;
  let cancelGesture: (() => void) | undefined;
  let lastSceneId: string | undefined;
  let pointerInspector: PointerInspectorHandle | undefined;
  let interactionVersion = 0;
  let disposed = false;
  let cancelDeferredPointer: (() => void) | undefined;
  let forceManualSelection: (() => void) | undefined;
  let snapPointer: { x: number; y: number; shiftKey: boolean } | undefined;
  let snapIdentity = '';
  const heldArrows = new Set<string>();
  const [settling, setSettling] = createSignal(false);
  const [viewport, setViewport] = createSignal({ width: innerWidth, height: innerHeight });
  const [natural, setNatural] = createSignal({ width: 0, height: 0 });
  const [selection, setSelection] = createSignal<Box>();
  const [snapMap, setSnapMap] = createSignal<FrameSnapMap>();
  const [snapPreview, setSnapPreview] = createSignal<Box>();
  const [active, setActive] = createSignal('');
  const [activeItemId, setActiveItemId] = createSignal('');
  const [activity, setActivity] = createSignal(false);
  const [forceNew, setForceNew] = createSignal(false);
  const [toolbarVisible, setToolbarVisible] = createSignal(false);
  const [toolbarBox, setToolbarBox] = createSignal<Box>({ x: 0, y: 0, width: TOOLBAR_WIDTH, height: TOOLBAR_HEIGHT });
  const [backgroundError, setBackgroundError] = createSignal(false);
  const [flashedRegion, setFlashedRegion] = createSignal('');
  const [drawingSession, setDrawingSession] = createSignal<DrawingSession>();
  const [pluginMenu, setPluginMenu] = createSignal<'workflow' | 'ocr' | 'scroll' | 'translation' | 'pin'>();
  const language = () => props.translationLanguage ?? 'zh-Hans';
  const [textMode, setTextMode] = createSignal<Record<string, 'ocr' | 'translation' | 'none'>>({});
  const [ocrPending, setOcrPending] = createSignal<OcrRequest>();
  const [ocrNotice, setOcrNotice] = createSignal<{ regionId: string; text: string }>();
  const [hiddenOcr, setHiddenOcr] = createSignal<Record<string, boolean>>({});
  const [exporting, setExporting] = createSignal(false);
  const [codeFocused, setCodeFocused] = createSignal(document.hasFocus() && !document.hidden);
  const [codeModal, setCodeModal] = createSignal(false);
  const snapLoader = new SnapMapLoader(getWindowSnapMap, value => { if (!disposed) { setSnapMap(value); if (snapPointer) updateSnapPreview(snapPointer); } });
  const ocrRequests = new OcrRequestGate<Snapshot>({
    run: request => props.onOcrRequest(request), cancel: id => props.onOcrCancel(id), pending: setOcrPending,
    result: (snapshot, request) => {
      props.onOcrResult(snapshot);
      const result = snapshot.scenes.find(scene => scene.id === request.target.sceneId)?.regions.find(region => region.id === request.target.regionId)?.ocr;
      if (result && !result.document.lines.length) setOcrNotice({ regionId: request.target.regionId, text: '未识别到文字' });
    },
    error: (text, request) => setOcrNotice({ regionId: request.target.regionId, text }),
  });
  const pluginEntries = createMemo(() => props.plugins.filter(plugin => plugin.state === 'enabled' && !plugin.error && !isCoreReplacementPlugin(plugin.manifest.id)).flatMap(plugin => plugin.manifest.contributions.map(contribution => ({ plugin, contribution }))));
  const workflowEntries = createMemo(() => pluginEntries().filter(value => value.contribution.kind === 'selection.workflow'));
  const ocrEntries = createMemo(() => pluginEntries().filter(value => value.contribution.kind === 'selection.ocr'));
  const scrollEntries = createMemo(() => pluginEntries().filter(value => value.contribution.kind === 'selection.scroll'));
  const translationEntries = createMemo(() => pluginEntries().filter(value => value.contribution.kind === 'selection.translation'));
  const pinEntries = createMemo(() => pluginEntries().filter(value => value.contribution.kind === 'selection.pin'));
  const toolbarWidth = () => (props.showButtonLabels === false ? 321 : 419) + (props.showButtonLabels === false ? 40 : 54) * [pinEntries().length, workflowEntries().length, ocrEntries().length || activeRegion()?.ocr, scrollEntries().length, translationEntries().length || activeRegion()?.translation].filter(Boolean).length;
  const drawingTools = () => [...coreDrawingTools];
  const geometry = createMemo(() => {
    const width = props.scene.background?.width || natural().width;
    const height = props.scene.background?.height || natural().height;
    if (!width || !height) return { x: 0, y: 0, width: 0, height: 0, scale: 1 };
    const scale = Math.min(viewport().width / width, viewport().height / height);
    return { x: (viewport().width - width * scale) / 2, y: (viewport().height - height * scale) / 2, width: width * scale, height: height * scale, scale };
  });
  const regionById = (id: string) => {
    const saved = props.scene.regions.find(region => region.id === id);
    return saved && overlayRegion(props.scene, saved, props.geometryView.overlay);
  };
  const cropChanged = (region: Region) => !region.imageOverride && props.geometryView.overlay?.regionId === region.id && !sameGeometry(region, props.scene.regions.find(value => value.id === region.id)!);
  const backgroundAnchored = (item: SpaceItem) => item.asset.kind === 'video' && item.state?.coordinateSpace === 'background' && item.state.backgroundId === props.scene.background?.id && geometry().width > 0 && geometry().height > 0;
  const displayedItem = (id: string) => {
    const item = props.scene.items.find(item => item.id === id)!;
    return backgroundAnchored(item) ? projectBackgroundItem(item, geometry(), viewport()) : item;
  };
  const updateItem = async (item: SpaceItem) => {
    try { await props.onUpdateItem(backgroundAnchored(item) ? persistBackgroundItem(item, geometry(), viewport()) : item); }
    catch (error) { props.onError(error instanceof Error ? error.message : String(error)); throw error; }
  };
  const activeRegion = createMemo(() => regionById(active()));
  const ocrTarget = (region: Region): OcrTarget => ({ sceneId: props.scene.id, regionId: region.id, backgroundId: props.scene.background!.id, drawingRevision: region.drawingRevision ?? 0, x: region.x, y: region.y, width: region.width, height: region.height });
  const savedOcr = (region: Region) => region.ocr?.backgroundId === props.scene.background?.id && region.ocr?.drawingRevision === (region.drawingRevision ?? 0) && !cropChanged(region) ? region.ocr : undefined;
  const savedTranslation = (region: Region) => region.translation?.backgroundId === props.scene.background?.id && region.translation?.sourceId === (region.imageOverride?.id ?? props.scene.background?.id) && region.translation?.drawingRevision === (region.drawingRevision ?? 0) && !cropChanged(region) ? region.translation : undefined;
  const selectedTextMode = (region: Region) => textMode()[region.id] ?? (savedTranslation(region) ? 'translation' : 'ocr');
  const ocrVisible = (region: Region) => active() === region.id && selectedTextMode(region) === 'ocr' && !hiddenOcr()[region.id] && !drawingSession() && !forceNew() && Boolean(savedOcr(region)?.document.lines.length);
  const translationVisible = (region: Region) => active() === region.id && selectedTextMode(region) === 'translation' && !drawingSession() && !forceNew() && Boolean(savedTranslation(region)?.selection.lines.length);
  const display = (region: Box): Box => ({ x: geometry().x + region.x * geometry().scale, y: geometry().y + region.y * geometry().scale, width: region.width * geometry().scale, height: region.height * geometry().scale });
  const imageProjection = (region: Region) => regionImageProjection(region, display(region), props.scene.background!, geometry().scale);
  const regions = () => props.scene.regions.map(region => ({ region, box: display(regionById(region.id)!) }));
  const point = (event: PointerEvent) => ({ x: clamp(event.clientX, geometry().x, geometry().x + geometry().width), y: clamp(event.clientY, geometry().y, geometry().y + geometry().height) });
  const referenced = (id: string) => props.refs.some(ref => ref.kind === 'region' && ref.id === id);
  const blocked = () => props.busy || exporting() || settling() || props.scene.run?.status === 'running';
  const scanSource = createMemo(() => {
    if (!props.onCodeSource || !codeFocused() || props.busy || drawingSession() || selection() || forceNew() || backgroundError() || props.geometryView.pending || props.geometryView.held) return;
    return codeSource(props.scene, active(), props.plugins);
  }, undefined, { equals: (a, b) => a?.key === b?.key });
  const codesPaused = () => Boolean(activity() || exporting() || settling() || pluginMenu() || ocrPending() || props.translationPending || codeModal());
  const visibleCodes = createMemo(() => { const view = props.codeView; return view?.key === scanSource()?.key && view?.result.codes.length ? view : undefined; });
  createEffect(on(() => [scanSource(), codesPaused()] as const, ([source, paused]) => props.onCodeSource?.(source, paused)));
  const snapUnavailable = () => blocked() || activity() || forceNew() || drawingSession() || pluginMenu() || ocrPending() || props.translationPending || props.geometryView.pending || props.geometryView.held || props.scene.frozen || props.scene.closed || backgroundError();
  function clearSnapPreview() { snapPointer = undefined; setSnapPreview(undefined); }
  function candidateAt(x: number, y: number) {
    const map = snapMap(), bg = props.scene.background, box = geometry();
    if (!bg || map?.backgroundId !== bg.id) return;
    const pixel = snapImagePoint(x, y, box, box.scale);
    return pixel && snapTarget(map, pixel.x, pixel.y);
  }
  function previewCandidateAt(pointer: { x: number; y: number; shiftKey: boolean }) {
    const target = document.elementFromPoint(pointer.x, pointer.y), composer = composerBox();
    if (snapUnavailable() || pointer.shiftKey || !document.hasFocus() || !target?.closest('.capture-surface') || target.closest('button,a,input,textarea,select,[contenteditable]:not([contenteditable="false"]),.artifact-card,.capture-region,.capture-toolbar,.selection-code-card,.ocr-text-layer,.drawing-editor') || document.querySelector('.modal-backdrop,.history-backdrop,.channel-popover,.reply-table-menu,[role="menu"]') || (toolbarVisible() && contains(toolbarBox(), pointer.x, pointer.y, TOLERANCE)) || (composer && contains(composer, pointer.x, pointer.y)) || regions().some(entry => contains(entry.box, pointer.x, pointer.y, 5))) return;
    return candidateAt(pointer.x, pointer.y);
  }
  function updateSnapPreview(pointer: { x: number; y: number; shiftKey: boolean }) {
    snapPointer = pointer;
    const candidate = previewCandidateAt(pointer);
    setSnapPreview(candidate ? display(candidate.bounds) : undefined);
  }
  function endNudge(flush = true) { heldArrows.clear(); props.geometryController.endKeys(flush); }
  async function settledRegion(region: Region, action: (saved: Region) => void | Promise<void>) {
    if (settling()) return;
    const sceneId = props.scene.id, backgroundId = props.scene.background?.id;
    setSettling(true); endNudge(false);
    try {
      await props.geometryController.flush();
      const saved = props.scene.regions.find(value => value.id === region.id);
      if (!disposed && !props.busy && !props.scene.closed && !props.scene.frozen && props.scene.id === sceneId && props.scene.background?.id === backgroundId && saved) await action(saved);
    } catch (error) { if (!disposed) props.onError(error instanceof Error ? error.message : String(error)); }
    finally { if (!disposed) setSettling(false); }
  }
  function deferGeometryPointer(event: PointerEvent, start: () => void) {
    if (!props.geometryView.pending) return false;
    event.preventDefault(); event.stopPropagation(); cancelDeferredPointer?.(); endNudge(false);
    const sceneId = props.scene.id, backgroundId = props.scene.background?.id;
    let canceled = false;
    const released = (value: PointerEvent) => { if (value.pointerId === event.pointerId) cancel(); };
    const cancel = () => { canceled = true; window.removeEventListener('pointerup', released, true); window.removeEventListener('pointercancel', released, true); if (cancelDeferredPointer === cancel) cancelDeferredPointer = undefined; };
    cancelDeferredPointer = cancel;
    window.addEventListener('pointerup', released, true); window.addEventListener('pointercancel', released, true);
    void props.geometryController.flush().then(() => {
      const valid = !canceled && !disposed && !blocked() && props.scene.id === sceneId && props.scene.background?.id === backgroundId;
      cancel(); if (valid) start();
    }).catch(() => cancel());
    return true;
  }
  function clearHideTimer() { if (hideTimer !== undefined) clearTimeout(hideTimer); hideTimer = undefined; }
  function hideToolbar() { clearHideTimer(); setToolbarVisible(false); setPluginMenu(undefined); }
  function scheduleHide() {
    if (!toolbarVisible() || hideTimer !== undefined) return;
    // One timer per departure. Continuous movement outside must not reset it.
    hideTimer = setTimeout(() => { hideTimer = undefined; setToolbarVisible(false); }, 300);
  }
  function composerBox(): Box | undefined {
    const element = document.querySelector<HTMLElement>('.composer');
    if (!element || !element.getClientRects().length || getComputedStyle(element).visibility === 'hidden') return;
    const rect = element.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0 ? { x: rect.x, y: rect.y, width: rect.width, height: rect.height } : undefined;
  }
  function placeToolbar(region: Region) {
    const box = display(region);
    const width = Math.min(toolbar?.offsetWidth || toolbarWidth(), viewport().width - EDGE * 2);
    const height = toolbar?.offsetHeight || TOOLBAR_HEIGHT;
    const x = clamp(box.x, EDGE, viewport().width - width - EDGE);
    const above = box.y - height - GAP, below = box.y + box.height + GAP;
    const composer = composerBox();
    let y = above;
    if (above < EDGE) {
      const candidate = { x, y: below, width, height };
      y = below + height <= viewport().height - EDGE && (!composer || !overlaps(candidate, composer)) ? below : clamp(above, EDGE, viewport().height - height - EDGE);
    }
    setToolbarBox({ x, y, width, height });
  }
  function bindToolbar(element: HTMLDivElement) {
    toolbar = element; toolbarObserver?.disconnect();
    const place = () => { if (!disposed && toolbar === element && element.isConnected && toolbarVisible()) { const region = activeRegion(); if (region) untrack(() => placeToolbar(region)); } };
    toolbarObserver = new ResizeObserver(place); toolbarObserver.observe(element); queueMicrotask(place);
  }
  function revealToolbar(region: Region) {
    if (activity() || forceNew()) return;
    clearHideTimer(); setActive(region.id); placeToolbar(region); setToolbarVisible(true);
  }
  function hover(event: PointerEvent) {
    if (event.target instanceof Element && event.target.closest('.selection-code-card')) { clearSnapPreview(); clearHideTimer(); return; }
    updateSnapPreview({ x: event.clientX, y: event.clientY, shiftKey: event.shiftKey });
    if (activity() || forceNew() || exporting() || props.geometryView.held || props.geometryView.pending) return;
    const target = event.target instanceof Element ? event.target : undefined;
    if ((pluginMenu()) && target?.closest('.capture-toolbar')) { clearHideTimer(); return; }
    const composer = composerBox();
    if (!target?.closest('.capture-surface') || target.closest('.artifact-card') || (composer && contains(composer, event.clientX, event.clientY))) { scheduleHide(); return; }
    if (toolbarVisible() && contains(toolbarBox(), event.clientX, event.clientY, TOLERANCE)) { clearHideTimer(); return; }
    // Last drawn wins for overlapping regions, matching the capture surface.
    const hovered = [...props.scene.regions].reverse().find(region => contains(display(regionById(region.id)!), event.clientX, event.clientY, 5));
    if (hovered) revealToolbar(regionById(hovered.id)!); else scheduleHide();
  }
  function setSelectionActivity(value: boolean) {
    if (activity() === value) return;
    setActivity(value); props.onSelectionActivity?.(value);
  }
  function closeDrawing() { if(props.blackboard){void props.onBlackboardClose?.();return;} setDrawingSession(undefined); setSelectionActivity(false); const region = activeRegion(); if (region) revealToolbar(region); }
  function beginDrawing(region: Region) {
    if (blocked() || !props.scene.background) return;
    void settledRegion(region, region => {
    ocrRequests.cancel(); props.onTranslationCancel?.(); cancelGesture?.(); hideToolbar(); setForceNew(false); setActiveItemId(''); setActive(region.id);
    const target = { sceneId: props.scene.id, regionId: region.id, backgroundId: props.scene.background!.id, sourceId: region.imageOverride?.id ?? props.scene.background!.id };
    setDrawingSession({ ...target, kind: 'core' });
    setSelectionActivity(true);
    });
  }
  function runWorkflow(entry: PluginEntry, region: Region) {
    setPluginMenu(undefined); if (blocked()) return;
    void props.onPluginWorkflow(entry.plugin.manifest.id, entry.plugin.revision, entry.contribution.id, region.id).catch(error => props.onError(error instanceof Error ? error.message : String(error)));
  }
  async function runPin(entry: PluginEntry, region: Region): Promise<boolean> {
    if (blocked() || !props.scene.background || entry.contribution.kind !== 'selection.pin' || !props.onPin) return false;
    setPluginMenu(undefined);
    try { return await props.onPin(entry.plugin.manifest.id, entry.plugin.revision, entry.contribution.id, region.id); }
    catch (error) { props.onError(error instanceof Error ? error.message : String(error)); return false; }
  }
  function runScroll(entry: PluginEntry, region: Region) {
    if (blocked() || region.imageOverride || !props.scene.background || entry.contribution.kind !== 'selection.scroll' || !props.onScrollCapture) return;
    setPluginMenu(undefined); ocrRequests.cancel(); props.onTranslationCancel?.();
    void settledRegion(region, saved => props.onScrollCapture!(entry.plugin.manifest.id, entry.plugin.revision, entry.contribution.id, ocrTarget(saved)));
  }
  function runOcr(entry: PluginEntry, region: Region) {
    if (blocked() || !props.scene.background || entry.contribution.kind !== 'selection.ocr') return;
    void settledRegion(region, region => {
    props.onTranslationCancel?.(); props.onClearTranslationNotice?.(); setTextMode(old => ({ ...old, [region.id]: 'ocr' }));
    setPluginMenu(undefined); setOcrNotice(undefined); setHiddenOcr(old => ({ ...old, [region.id]: false }));
    void ocrRequests.start({ requestId: crypto.randomUUID(), pluginId: entry.plugin.manifest.id, revision: entry.plugin.revision, contributionId: entry.contribution.id, target: ocrTarget(region) });
    });
  }
  function selectOcr(region: Region) {
    if (savedOcr(region)?.document.lines.length) { const visible = ocrVisible(region); setTextMode(old => ({ ...old, [region.id]: 'ocr' })); setHiddenOcr(old => ({ ...old, [region.id]: visible })); return; }
    if (ocrEntries().length === 1) runOcr(ocrEntries()[0], region);
    else setPluginMenu(pluginMenu() === 'ocr' ? undefined : 'ocr');
  }
  function runTranslation(entry: PluginEntry, region: Region) {
    if (blocked() || !props.scene.background || entry.contribution.kind !== 'selection.translation') return;
    void settledRegion(region, region => {
    ocrRequests.cancel(); setOcrNotice(undefined); setPluginMenu(undefined);
    setTextMode(old => ({ ...old, [region.id]: 'translation' }));
    props.onTranslationRequest?.({ requestId: crypto.randomUUID(), pluginId: entry.plugin.manifest.id, revision: entry.plugin.revision, contributionId: entry.contribution.id, target: ocrTarget(region), language: language() });
    });
  }
  function translate(region: Region, repeat = false) {
    if (!repeat && savedTranslation(region)?.selection.lines.length) { const visible = translationVisible(region); setTextMode(old => ({ ...old, [region.id]: visible ? 'none' : 'translation' })); return; }
    if (translationEntries().length === 1) runTranslation(translationEntries()[0], region);
    else if (translationEntries().length) { setPluginMenu('translation'); }
  }
  function trackPointer(event: PointerEvent, move: (event: PointerEvent) => void, finish: (committed: boolean) => void) {
    ocrRequests.cancel(); props.onTranslationCancel?.(); cancelGesture?.(); event.preventDefault(); event.stopPropagation();
    interactionVersion++;
    clearSnapPreview();
    window.getSelection()?.removeAllRanges(); hideToolbar(); setSelectionActivity(true);
    const pointerId = event.pointerId;
    let ended = false;
    const cleanup = () => {
      canvas.removeEventListener('pointermove', moved); canvas.removeEventListener('pointerup', released);
      canvas.removeEventListener('pointercancel', canceled); canvas.removeEventListener('lostpointercapture', canceled);
      cancelGesture = undefined;
      if (canvas.hasPointerCapture(pointerId)) canvas.releasePointerCapture(pointerId);
    };
    const end = (committed: boolean) => { if (ended) return; ended = true; cleanup(); setSelectionActivity(false); finish(committed); };
    const moved = (next: PointerEvent) => { if (next.pointerId === pointerId) { next.preventDefault(); move(next); } };
    const released = (next: PointerEvent) => { if (next.pointerId === pointerId) { moved(next); end(true); } };
    const canceled = (next: PointerEvent) => { if (next.pointerId === pointerId) end(false); };
    cancelGesture = () => end(false);
    canvas.addEventListener('pointermove', moved); canvas.addEventListener('pointerup', released);
    canvas.addEventListener('pointercancel', canceled); canvas.addEventListener('lostpointercapture', canceled);
    try { canvas.setPointerCapture(pointerId); } catch { end(false); }
  }
  function begin(event: PointerEvent) {
    if (props.blackboard) return;
    if (event.button !== 0 || activity() || !props.scene.background || backgroundError() || !geometry().width || blocked()) return;
    const target = event.target as Element;
    if (target.closest('button,.artifact-card,.capture-toolbar,.selection-code-card')) return;
    if (target.closest('.capture-region') && !event.shiftKey && !forceNew()) return;
    if (deferGeometryPointer(event, () => begin(event))) return;
    setActiveItemId('');
    endNudge();
    const start = point(event), scale = geometry().scale, offset = { x: geometry().x, y: geometry().y };
    const sceneId = props.scene.id, backgroundId = props.scene.background.id;
    const automatic = previewCandidateAt({ x: event.clientX, y: event.clientY, shiftKey: event.shiftKey });
    const gesture = new SnapSelectionGesture(start, automatic && display(automatic.bounds));
    let lastPoint = start;
    setForceNew(false); setSelection(gesture.move(start));
    forceManualSelection = () => { gesture.forceManual(); setSelection(gesture.move(lastPoint)); };
    trackPointer(event, next => {
      const end = point(next);
      lastPoint = end; setSelection(gesture.move(end, next.shiftKey, { x: next.clientX, y: next.clientY }));
    }, committed => {
      forceManualSelection = undefined;
      const selected = selection(); setSelection(undefined);
      if (!committed || disposed || props.scene.id !== sceneId || props.scene.background?.id !== backgroundId || !selected || (!gesture.isAutomatic && (selected.width < 8 || selected.height < 8))) return;
      const left = Math.round((selected.x - offset.x) / scale), top = Math.round((selected.y - offset.y) / scale);
      const right = Math.round((selected.x + selected.width - offset.x) / scale), bottom = Math.round((selected.y + selected.height - offset.y) / scale);
      const region: Region = { id: crypto.randomUUID(), x: left, y: top, width: right - left, height: bottom - top };
      const version = interactionVersion;
      setActive(region.id);
      void Promise.resolve().then(() => props.onAddRegion(region)).then(() => {
        if (props.scene.id !== sceneId || interactionVersion !== version) return;
        revealToolbar(region); props.onSelectionComplete?.(region);
      }).catch(error => props.onError(error instanceof Error ? error.message : '无法添加区域'));
    });
  }
  function editRegion(event: PointerEvent, region: Region, handle?: Handle) {
    if (event.shiftKey || forceNew()) { begin(event); return; }
    if (event.button !== 0 || activity() || blocked()) return;
    if (deferGeometryPointer(event, () => { const saved = regionById(region.id); if (saved) editRegion(event, saved, handle); })) return;
    endNudge();
    const target = geometryTarget(props.scene, region.id);
    if (!target || !props.geometryController.beginPointer(target)) return;
    setActiveItemId('');
    canvas.focus({ preventScroll: true });
    const original = target.geometry, scale = geometry().scale;
    const width = target.width, height = target.height, minimum = Math.max(1, Math.round(8 / scale));
    const startX = event.clientX, startY = event.clientY, sceneId = props.scene.id;
    setActive(region.id);
    trackPointer(event, next => {
      const dx = Math.round((next.clientX - startX) / scale), dy = Math.round((next.clientY - startY) / scale);
      let updated: RegionGeometry;
      if (!handle) updated = { ...original, x: clamp(original.x + dx, 0, width - original.width), y: clamp(original.y + dy, 0, height - original.height) };
      else {
        let left = original.x, top = original.y, right = original.x + original.width, bottom = original.y + original.height;
        if (handle.includes('w')) left = clamp(left + dx, 0, right - minimum);
        if (handle.includes('e')) right = clamp(right + dx, left + minimum, width);
        if (handle.includes('n')) top = clamp(top + dy, 0, bottom - minimum);
        if (handle.includes('s')) bottom = clamp(bottom + dy, top + minimum, height);
        updated = { ...original, x: left, y: top, width: right - left, height: bottom - top };
        updated = snapResize(updated, handle, candidateAt(next.clientX, next.clientY)?.bounds, scale, width, height, minimum);
      }
      props.geometryController.previewPointer(updated);
    }, committed => {
      void props.geometryController.endPointer(committed && props.scene.id === sceneId).then(() => {
        const saved = regionById(region.id); if (!disposed && props.scene.id === sceneId && saved) revealToolbar(saved);
      }).catch(() => {});
    });
  }
  const resize = () => { clearSnapPreview(); cancelDeferredPointer?.(); cancelGesture?.(); endNudge(); setViewport({ width: innerWidth, height: innerHeight }); const region = activeRegion(); if (region && toolbarVisible()) placeToolbar(region); };
  const leave = () => { clearSnapPreview(); scheduleHide(); };
  const blur = () => { setCodeFocused(false); clearSnapPreview(); cancelDeferredPointer?.(); cancelGesture?.(); endNudge(); hideToolbar(); };
  const codeFocus = () => setCodeFocused(!document.hidden);
  const snapFocus = (event: FocusEvent) => { if (event.target instanceof Element && event.target.closest('button,a,input,textarea,select,[contenteditable]:not([contenteditable="false"]),.composer,.video-trim-popover,.artifact-card,.selection-code-card,.ocr-text-layer')) clearSnapPreview(); };
  const snapVisibility = () => { setCodeFocused(!document.hidden && document.hasFocus()); if (document.hidden) clearSnapPreview(); };
  const snapMutations = new MutationObserver(() => { const modal = Boolean(document.querySelector('.modal-backdrop,.history-backdrop,.channel-popover,.reply-table-menu,[role="menu"]')); setCodeModal(modal); if (snapPreview() && modal) clearSnapPreview(); });
  snapMutations.observe(document.body, { childList: true, subtree: true });
  const recordKey = (event: KeyboardEvent) => {
    if(props.blackboard){
      const replay=geometryHistoryKey(event),target=event.target instanceof Element?event.target:document.activeElement;
      if(replay&&!event.defaultPrevented&&document.hasFocus()&&!drawingSession()&&!blocked()&&props.scene.regions[0]?.drawingHistory?.[replay].length&&window.getSelection()?.isCollapsed&&!target?.closest('input,textarea,select,[contenteditable]:not([contenteditable="false"]),.composer,.artifact-card,iframe')&&!document.querySelector('.modal-backdrop,.history-backdrop,.channel-popover,[role="menu"]')){
        event.preventDefault();event.stopImmediatePropagation();props.onBlackboardReplay?.(replay==='redo');
      }
      return;
    }
    if (nativeSelectOwnsEscape(event)) return;
    const videoItem = props.scene.items.find(item => item.id === activeItemId() && item.asset.kind === 'video');
    if (props.onVideoCopy && videoCopyKey(event, { active: Boolean(videoItem), sceneId: props.scene.id, itemId: videoItem?.id ?? '', focused: document.hasFocus(), selectedText: Boolean(window.getSelection()?.toString()), target: event.target instanceof Element ? event.target : document.activeElement, blocked: activity() || Boolean(drawingSession()) || exporting() || forceNew() || blocked() || Boolean(pluginMenu() || document.querySelector('.modal-backdrop,.history-backdrop,.channel-popover,.reply-table-menu,[role="menu"]')) })) {
      event.preventDefault(); event.stopImmediatePropagation(); void props.onVideoCopy(videoItem!.id).catch(error => props.onError(error instanceof Error ? error.message : String(error))); return;
    }
    if ((event.target instanceof Element ? event.target : document.activeElement)?.closest('.selection-code-card,.video-trim-popover,.artifact-video,[data-run-journal]')) return;
    if (event.key === 'Shift') { setSnapPreview(undefined); if (snapPointer) snapPointer.shiftKey = true; forceManualSelection?.(); }
    if (event.key === 'Escape' && cancelGesture) { cancelGesture(); event.preventDefault(); event.stopImmediatePropagation(); return; }
    if (event.key === 'Escape' && pluginMenu()) { setPluginMenu(undefined); event.preventDefault(); event.stopImmediatePropagation(); return; }
    if (event.key === 'Escape' && ocrPending()) { ocrRequests.cancel(); event.preventDefault(); event.stopImmediatePropagation(); return; }
    if (event.key === 'Escape' && props.translationPending) { props.onTranslationCancel?.(); event.preventDefault(); event.stopImmediatePropagation(); return; }
    const replay = geometryHistoryKey(event);
    if (replay) {
      const target = event.target instanceof Element ? event.target : document.activeElement;
      if (event.defaultPrevented || !document.hasFocus() || activity() || drawingSession() || forceNew() || blocked() || pluginMenu() || !window.getSelection()?.isCollapsed || target?.closest('input,textarea,select,[contenteditable]:not([contenteditable="false"]),[role="slider"],[role="menu"],.composer,.video-trim-popover,.artifact-card,.ocr-text-layer,video,iframe') || document.querySelector('.modal-backdrop,.history-backdrop,.channel-popover,.reply-table-menu')) return;
      if (!props.scene.geometryHistory?.[replay].length && !(replay === 'undo' && props.geometryView.pending)) return;
      event.preventDefault(); event.stopImmediatePropagation(); endNudge(false); void props.onGeometryReplay(replay);
      return;
    }
    if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) {
      const target = event.target instanceof Element ? event.target : document.activeElement;
      const region = activeRegion(), selected = window.getSelection();
      if (event.defaultPrevented || event.isComposing || event.ctrlKey || event.metaKey || event.altKey || !document.hasFocus() || activity() || drawingSession() || forceNew() || blocked() || pluginMenu() || !region || !selected?.isCollapsed || backgroundError() || target?.closest('input,textarea,select,button,a,[contenteditable]:not([contenteditable="false"]),[role="slider"],[role="menu"],.composer,.video-trim-popover,.artifact-card,.ocr-text-layer,video,iframe') || document.querySelector('.modal-backdrop,.history-backdrop,.channel-popover,.reply-table-menu')) return;
      const current = geometryTarget(props.scene, region.id); if (!current) return;
      const fresh = !event.repeat && !heldArrows.has(event.key);
      heldArrows.add(event.key);
      event.preventDefault(); event.stopImmediatePropagation();
      const step = event.shiftKey ? 10 : 1;
      if (props.geometryController.nudge(current, event.key === 'ArrowLeft' ? -step : event.key === 'ArrowRight' ? step : 0, event.key === 'ArrowUp' ? -step : event.key === 'ArrowDown' ? step : 0, fresh)) {
        ocrRequests.cancel(); props.onTranslationCancel?.(); revealToolbar(regionById(region.id)!);
      }
      return;
    }
    if (event.key.toLowerCase() === 'c' && !event.defaultPrevented && !event.isComposing && !event.repeat && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey) {
      const target = event.target instanceof Element ? event.target : document.activeElement;
      if (!target?.closest('input,textarea,select,.ocr-text-layer,[contenteditable]:not([contenteditable="false"])') && !window.getSelection()?.anchorNode?.parentElement?.closest('.ocr-text-layer') && pointerInspector?.copy()) { event.preventDefault(); event.stopImmediatePropagation(); return; }
    }
    if (event.defaultPrevented || event.isComposing || event.repeat || !['r', 'd', 'o', 't', 'p', 'c', 's', 'enter'].includes(event.key.toLowerCase()) || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey || activity() || drawingSession() || exporting() || forceNew() || blocked()) return;
    const target = event.target instanceof Element ? event.target : document.activeElement;
    if (target?.closest('input,textarea,select,.artifact-card,.ocr-text-layer,[contenteditable]:not([contenteditable="false"])') || (event.key === 'Enter' && target?.closest('button,a')) || document.querySelector('.modal-backdrop,.history-backdrop,.channel-popover,.reply-table-menu') || window.getSelection()?.anchorNode?.parentElement?.closest('.ocr-text-layer,.text-artifact')) return;
    const region = activeRegion();
    if (!region || !props.scene.background || backgroundError()) return;
    if (event.key.toLowerCase() === 'd') { event.preventDefault(); beginDrawing(region); }
    else if (event.key.toLowerCase() === 'o') { const entry = ocrEntries()[0]; if (entry) { event.preventDefault(); runOcr(entry, region); } }
    else if (event.key.toLowerCase() === 't') { if (translationEntries().length || savedTranslation(region)) { event.preventDefault(); event.stopImmediatePropagation(); translate(region); } }
    else if (event.key.toLowerCase() === 'p') { const entry = pinEntries()[0]; if (entry) { event.preventDefault(); event.stopImmediatePropagation(); void runPin(entry, region); } }
    else if (['c', 's', 'enter'].includes(event.key.toLowerCase())) { event.preventDefault(); event.stopImmediatePropagation(); void exportRegion(region, event.key.toLowerCase() !== 's', event.key === 'Enter'); }
    else if (!region.imageOverride && captureGrant()) { event.preventDefault(); props.onRecordRegion(region.id); }
  };
  const releaseKey = (event: KeyboardEvent) => { if (event.key === 'Shift' && snapPointer) updateSnapPreview({ ...snapPointer, shiftKey: false }); if (heldArrows.delete(event.key) && !heldArrows.size) props.geometryController.endKeys(); };
  const focusReference = (event: Event) => {
    const reference = (event as CustomEvent<Reference>).detail;
    if (reference?.kind === 'item') {
      if (props.scene.items.some(item => item.id === reference.id)) setActiveItemId(reference.id);
      return;
    }
    if (reference?.kind !== 'region' || activity()) return;
    const region = regionById(reference.id);
    if (!region) return;
    setForceNew(false); revealToolbar(region); setFlashedRegion(region.id);
    if (flashTimer !== undefined) clearTimeout(flashTimer);
    flashTimer = setTimeout(() => { setFlashedRegion(''); flashTimer = undefined; }, 900);
  };
  window.addEventListener('resize', resize); window.addEventListener('pointermove', hover); window.addEventListener('blur', blur);
  window.addEventListener('focus', codeFocus);
  document.documentElement.addEventListener('pointerleave', leave);
  document.addEventListener('mewu:focus-reference', focusReference);
  document.addEventListener('keydown', recordKey, true);
  document.addEventListener('keyup', releaseKey, true);
  document.addEventListener('focusin', snapFocus); document.addEventListener('visibilitychange', snapVisibility);
  onCleanup(() => {
    disposed = true;
    toolbarObserver?.disconnect();
    props.onCodeSource?.(undefined, false); window.removeEventListener('focus', codeFocus);
    snapLoader.dispose();
    snapMutations.disconnect(); document.removeEventListener('focusin', snapFocus); document.removeEventListener('visibilitychange', snapVisibility);
    props.onExportActivity?.(false);
    ocrRequests.dispose();
    cancelDeferredPointer?.(); cancelGesture?.(); endNudge(); clearHideTimer(); if (activity()) props.onSelectionActivity?.(false);
    if (flashTimer !== undefined) clearTimeout(flashTimer);
    window.removeEventListener('resize', resize); window.removeEventListener('pointermove', hover); window.removeEventListener('blur', blur);
    document.documentElement.removeEventListener('pointerleave', leave);
    document.removeEventListener('mewu:focus-reference', focusReference);
    document.removeEventListener('keydown', recordKey, true);
    document.removeEventListener('keyup', releaseKey, true);
  });
  createEffect(() => {
    const background = props.scene.background, id = props.scene.id;
    const request = !props.blackboard && background?.width && background.height ? { sceneId: id, backgroundId: background.id, width: background.width, height: background.height } : undefined;
    const identity = request ? `${id}:${request.backgroundId}:${request.width}:${request.height}` : '';
    if (identity === snapIdentity) return;
    snapIdentity = identity;
    untrack(() => { clearSnapPreview(); if (request) snapLoader.request(request); else snapLoader.cancel(); });
  });
  createEffect(() => { if (snapUnavailable()) untrack(clearSnapPreview); });
  createEffect(() => {
    const id = props.scene.id;
    if (lastSceneId === id) return;
    lastSceneId = id;
    untrack(() => { ocrRequests.cancel(); setOcrNotice(undefined); setHiddenOcr({}); setTextMode({}); interactionVersion++; cancelDeferredPointer?.(); cancelGesture?.(); endNudge(); setDrawingSession(undefined); setSelectionActivity(false); hideToolbar(); setActive(''); setActiveItemId(''); setFlashedRegion(''); setForceNew(false); setBackgroundError(false); });
  });
  createEffect(() => {
    const disabled = props.busy || exporting() || settling() || props.scene.frozen || props.scene.closed || props.scene.run?.status === 'running';
    if (disabled) untrack(() => { cancelDeferredPointer?.(); cancelGesture?.(); endNudge(false); });
  });
  createEffect(() => {
    const value = props.geometryView, region = activeRegion();
    if (value.mode === 'keyboard' && (value.held || value.pending) && region) untrack(() => { clearHideTimer(); placeToolbar(region); setToolbarVisible(true); });
  });
  createEffect(() => {
    const ids = props.scene.regions.map(region => region.id);
    untrack(() => { if (active() && !ids.includes(active())) hideToolbar(); });
  });
  createEffect(() => {
    const ids = props.scene.items.map(item => item.id);
    untrack(() => { if (activeItemId() && !ids.includes(activeItemId())) setActiveItemId(''); });
  });
  let lastSource: string | undefined;
  createEffect(() => {
    const source = `${props.scene.id}:${props.scene.background?.id ?? ''}:${props.scene.regions.map(value => `${value.id}:${value.imageOverride?.id ?? ''}`).join('|')}`;
    if (source === lastSource) return;
    lastSource = source;
    untrack(() => { cancelDeferredPointer?.(); cancelGesture?.(); endNudge(); });
  });
  createEffect(() => {
    const session = drawingSession();
    if (session && (session.sceneId !== props.scene.id || session.backgroundId !== props.scene.background?.id || !props.scene.regions.some(region => region.id === session.regionId && (region.imageOverride?.id ?? props.scene.background?.id) === session.sourceId) || props.scene.frozen || props.scene.closed || props.scene.run?.status === 'running')) untrack(() => { setDrawingSession(undefined); setSelectionActivity(false); hideToolbar(); });
  });
  createEffect(() => {
    const enabled=props.blackboard&&props.blackboardDrawing, background=props.scene.background, region=props.scene.regions[0];
    const locked=props.busy||props.scene.frozen||props.scene.closed||props.scene.run?.status==='running';
    untrack(()=>{
      if(!enabled){if(props.blackboard&&drawingSession()){setDrawingSession(undefined);setSelectionActivity(false);hideToolbar();}return;}
      if(locked||!background||!region||drawingSession())return;
      hideToolbar();setActive(region.id);
      setDrawingSession({sceneId:props.scene.id,backgroundId:background.id,regionId:region.id,sourceId:background.id,kind:'core'});
      setSelectionActivity(true);
    });
  });
  createEffect(() => {
    const request = ocrPending(), region = activeRegion();
    if (!request) return;
    const entry = ocrEntries().find(value => value.plugin.manifest.id === request.pluginId && value.plugin.revision === request.revision && value.contribution.id === request.contributionId);
    if (!entry || !region || !props.scene.background || props.scene.frozen || props.scene.closed || props.busy || forceNew() || !sameOcrTarget(request.target, ocrTarget(region))) untrack(() => ocrRequests.cancel());
  });
  createEffect(() => {
    const request = props.translationPending?.request, region = activeRegion();
    // Restoring a frozen scene begins with no hovered region; that is not cancellation.
    if (request && !props.scene.frozen && ((region && request.target.regionId !== region.id) || forceNew())) untrack(() => props.onTranslationCancel?.());
  });
  async function crop(region: Region): Promise<Blob> {
    if (!props.scene.background) throw new Error('没有截图');
    if (region.drawings?.length || region.translation) throw new Error('请在桌面版导出带标注的图片');
    const source = region.imageOverride;
    const img = new window.Image(); img.crossOrigin = 'anonymous'; img.src = assetUrl(source ?? props.scene.background);
    await img.decode();
    const imageCanvas = document.createElement('canvas'); imageCanvas.width = source?.width ?? region.width; imageCanvas.height = source?.height ?? region.height;
    const context = imageCanvas.getContext('2d'); if (!context) throw new Error('无法处理截图');
    if (source) context.drawImage(img, 0, 0, imageCanvas.width, imageCanvas.height);
    else context.drawImage(img, region.x, region.y, region.width, region.height, 0, 0, region.width, region.height);
    return new Promise((resolve, reject) => imageCanvas.toBlob(blob => blob ? resolve(blob) : reject(new Error('无法处理截图')), 'image/png'));
  }
  async function exportRegion(region: Region, copy: boolean, closeSpace = false): Promise<boolean> {
    if (exporting() || props.busy) return false;
    const sceneId = props.scene.id, backgroundId = props.scene.background?.id;
    let fence: string | undefined;
    const stamp = (value: Region) => `${value.x}:${value.y}:${value.width}:${value.height}:${value.drawingRevision ?? 0}:${value.imageOverride?.id ?? ''}:${value.translation?.overlay.id ?? ''}`;
    const current = () => {
      const saved = props.scene.regions.find(value => value.id === region.id);
      return !disposed && props.scene.id === sceneId && !props.scene.closed && !props.scene.frozen && props.scene.background?.id === backgroundId && Boolean(saved) && (fence === undefined || stamp(saved!) === fence);
    };
    setExporting(true); props.onExportActivity?.(true);
    try {
      return await finishCapture({ prepare: async () => { await props.onBeforeExport?.(); const saved = props.scene.regions.find(value => value.id === region.id); if (saved) fence = stamp(saved); }, current, export: async () => {
        const savedRegion = props.scene.regions.find(value => value.id === region.id)!;
        if (native) return exportNativeRegion(sceneId, region.id, copy);
        else {
          const blob = await crop(savedRegion);
          if (!current()) return false;
          if (copy) await navigator.clipboard.write([new ClipboardItem({ 'image/png': blob })]);
          else { const url = URL.createObjectURL(blob), link = document.createElement('a'); link.href = url; link.download = `截图_${new Date().toISOString().replace(/[:.]/g, '-')}.png`; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000); }
          return true;
        }
      }, close: copy && closeSpace ? () => props.onCopyClose?.(sceneId) ?? Promise.resolve(false) : undefined });
    } catch (error) { if (!disposed) props.onError(error instanceof Error ? error.message : '操作失败'); return false; }
    finally { if (!disposed) { setExporting(false); props.onExportActivity?.(false); } }
  }
  return <main ref={canvas} tabIndex={-1} class="space-canvas capture-surface" classList={{ 'capture-blackboard':props.blackboard, 'can-select': !props.blackboard&&Boolean(props.scene.background), 'capture-dragging': activity(), 'capture-add-mode': forceNew() }} onPointerDown={begin} aria-label={t("AI 空间")}>
    <Show when={visibleCodes()}>{view => <SelectionCodeCard result={view().result} region={display(props.scene.regions.find(region => region.id === view().result.regionId)!)} toolbar={toolbarVisible() ? toolbarBox() : undefined} hidden={codesPaused()} disabled={Boolean(props.codeActionBusy)} onAction={(token, id, action) => props.onCodeAction?.(token, id, action) ?? Promise.resolve(false)} />}</Show>
    <Show when={!props.blackboard && pointerNative && props.scene.background}>{background => <PointerInspector sceneId={props.scene.id} background={background()} imageBox={geometry()} dimensions={pointerSelectionDimensions(selection(), geometry(), activeRegion())} disabled={props.busy || exporting() || props.scene.frozen || props.scene.closed || backgroundError() || Boolean(drawingSession() || pluginMenu() || ocrPending() || props.translationPending) || props.scene.regions.some(region => ocrVisible(region) || translationVisible(region))} blockedAt={(x, y) => toolbarVisible() && contains(toolbarBox(), x, y, TOLERANCE)} onReady={value => { pointerInspector = value; }} onError={props.onError} />}</Show>
    <Show when={props.scene.background?.id} keyed>{id => <img class="screen-background capture-background" data-asset={id} src={assetUrl(props.scene.background!)} alt={t("屏幕截图")} draggable={false} onLoad={event => { setNatural({ width: event.currentTarget.naturalWidth, height: event.currentTarget.naturalHeight }); setBackgroundError(false); }} onError={() => { setBackgroundError(true); props.onError('无法读取屏幕截图'); }} />}</Show>
    <svg class="capture-mask" width="100%" height="100%" aria-hidden="true">
      <defs><mask id="region-cutouts" maskUnits="userSpaceOnUse"><rect width="100%" height="100%" fill="white" />
        <For each={regions()}>{entry => <rect x={entry.box.x} y={entry.box.y} width={entry.box.width} height={entry.box.height} fill="black" />}</For>
        <Show when={selection()}>{box => <rect x={box().x} y={box().y} width={box().width} height={box().height} fill="black" />}</Show>
      </mask></defs>
      <rect width="100%" height="100%" fill="#000" fill-opacity={!props.blackboard&&props.scene.background ? props.maskOpacity : '0'} mask="url(#region-cutouts)" />
    </svg>
    <Show when={snapPreview()}>{box => <div class="capture-snap-preview" aria-hidden="true" style={{ left: `${box().x}px`, top: `${box().y}px`, width: `${box().width}px`, height: `${box().height}px` }} />}</Show>
    <For each={props.scene.regions.map(region => region.id)}>{(id, index) => {
      const region = () => regionById(id)!;
      const box = () => display(region());
      const image = () => imageProjection(region());
      return <section class="capture-region" classList={{ 'capture-active': active() === id, 'capture-located': flashedRegion() === id, 'capture-image-override': Boolean(region().imageOverride) }} style={{ left: `${box().x}px`, top: `${box().y}px`, width: `${box().width}px`, height: `${box().height}px` }} onPointerDown={event => editRegion(event, region())} aria-label={`区域 ${index() + 1}`}>
        <div class="capture-region-content" style={{ left: `${image().box.x - box().x}px`, top: `${image().box.y - box().y}px`, width: `${image().box.width}px`, height: `${image().box.height}px` }}>
          <Show when={region().imageOverride?.id} keyed>{assetId => <img data-asset={assetId} class="capture-override-image" src={assetUrl(region().imageOverride!)} alt={t("长截图")} draggable={false} onError={() => props.onError('无法读取长截图')} />}</Show>
          <Show when={savedTranslation(region())}>{value => <TranslationLayer value={value()} active={active() === id && !drawingSession()} disabled={blocked()} onRemove={props.onTranslationRemove ? () => settledRegion(region(), saved => props.onTranslationRemove!(ocrTarget(saved))) : undefined} onError={props.onError} />}</Show>
          <Show when={drawingSession()?.regionId !== id}><DrawingLayer region={image().region} sourceRegion={props.scene.regions.find(value => value.id === id)} sceneId={props.scene.id} background={props.scene.background} /></Show>
          <Show when={translationVisible(region())}><OcrTextLayer document={savedTranslation(region())!.selection} label={t("译文")} repeatLabel={t("重新翻译")} active={active() === id} onClose={() => setTextMode(old => ({ ...old, [id]: 'none' }))} onRepeat={translationEntries().length ? () => translate(region(), true) : undefined} onError={props.onError} onSelecting={setSelectionActivity} /></Show>
          <Show when={ocrVisible(region())}><OcrTextLayer document={savedOcr(region())!.document} active={active() === id} onClose={() => { ocrRequests.cancel(); setHiddenOcr(old => ({ ...old, [id]: true })); }} onRepeat={ocrEntries().length ? () => runOcr(ocrEntries()[0], region()) : undefined} onError={props.onError} onSelecting={setSelectionActivity} /></Show>
        </div>
        <Show when={ocrPending()?.target.regionId === id || ocrNotice()?.regionId === id}><div class="ocr-status" role="status" onPointerDown={event => event.stopPropagation()}><Show when={ocrPending()?.target.regionId === id}><LoaderCircle size={13} class="spin" /></Show><span title={ocrNotice()?.regionId === id ? ocrNotice()!.text : undefined}>{ocrPending()?.target.regionId === id ? t("正在识别") : ocrNotice()?.text}</span><button title={ocrPending()?.target.regionId === id ? t("取消识别") : t("关闭")} aria-label={ocrPending()?.target.regionId === id ? t("取消识别") : t("关闭识别提示")} onClick={() => { ocrRequests.cancel(); setOcrNotice(undefined); }}><X size={13} /></button></div></Show>
        <span class="capture-region-number">{index() + 1}</span>
        <Show when={props.translationPending?.request.target.regionId === id || props.translationNotice?.regionId === id}><div class="ocr-status" role="status" onPointerDown={event => event.stopPropagation()}><Show when={props.translationPending?.request.target.regionId === id}><LoaderCircle size={13} class="spin" /></Show><span>{props.translationPending?.request.target.regionId === id ? props.translationPending.progress?.phase === 'rendering' ? t("正在排版") : props.translationPending.progress?.phase === 'translating' ? `正在翻译${props.translationPending.progress.total ? ` ${props.translationPending.progress.completed}/${props.translationPending.progress.total}` : ''}` : t("正在识别") : props.translationNotice?.text}</span><button title={props.translationPending ? t("取消翻译") : t("关闭")} aria-label={props.translationPending ? t("取消翻译") : t("关闭翻译提示")} onClick={() => { props.onTranslationCancel?.(); props.onClearTranslationNotice?.(); }}><X size={13} /></button></div></Show>
        <Show when={active() === id && !forceNew() && !drawingSession()}>
          <For each={HANDLES}>{handle => <i class={`capture-handle capture-handle-${handle}`} onPointerDown={event => editRegion(event, region(), handle)} aria-hidden="true" />}</For>
        </Show>
      </section>;
    }}</For>
    <Show when={selection()}>{box => <div class="capture-selection" style={{ left: `${box().x}px`, top: `${box().y}px`, width: `${box().width}px`, height: `${box().height}px` }} />}</Show>
    <Show when={toolbarVisible() && !activity() && !forceNew() && activeRegion()}>{region => <div ref={bindToolbar} class="capture-toolbar" role="toolbar" aria-label={t("截图工具")} style={{ left: `${toolbarBox().x}px`, top: `${toolbarBox().y}px`, width: `${toolbarWidth()}px` }} onPointerDown={event => { event.stopPropagation(); if (window.getSelection()?.anchorNode?.parentElement?.closest('.ocr-text-layer')) event.preventDefault(); }} onPointerEnter={clearHideTimer}>
      <button data-caption={t("引用")} class="capture-reference" classList={{ 'capture-referenced': referenced(region().id) }} title={referenced(region().id) ? t("已引用") : t("引用当前区域")} aria-label={t("引用当前区域")} disabled={blocked()} onClick={() => { if (!referenced(region().id)) props.onReference({ kind: 'region', id: region().id }); props.onFocusComposer?.(); }}>@</button>
      <button data-caption={t("加选区")} title={t("添加区域 · Shift + 拖动")} aria-label={t("添加截图区域")} disabled={blocked()} onClick={() => { setForceNew(true); hideToolbar(); }}><Plus size={19} /></button>
      <button data-caption={t("删除")} class="capture-delete" title={t("删除区域")} aria-label={t("删除当前区域")} disabled={blocked()} onClick={() => { const id = region().id; hideToolbar(); setActive(''); props.onRemoveRegion(id); }}><Trash2 size={17} /></button>
      <span class="capture-separator" />
      <button data-caption={t("绘制")} class="capture-plugin" title={t("绘制 · D")} aria-label={t("绘制")} disabled={blocked()} onClick={() => beginDrawing(region())}><Pencil size={18} /></button>
      <Show when={workflowEntries().length}><button data-caption={workflowEntries().length === 1 ? workflowEntries()[0].contribution.title : t("图像处理")} class="capture-plugin" title={workflowEntries().length === 1 ? workflowEntries()[0].contribution.title : t("图像处理")} aria-label={workflowEntries().length === 1 ? workflowEntries()[0].contribution.title : t("图像处理")} disabled={blocked()} onClick={() => workflowEntries().length === 1 ? runWorkflow(workflowEntries()[0], region()) : setPluginMenu(pluginMenu() === 'workflow' ? undefined : 'workflow')}><Table2 size={18} /></button></Show>
      <Show when={ocrEntries().length || savedOcr(region())?.document.lines.length}><button data-caption={t("文字")} class="capture-plugin" classList={{ 'capture-ocr-active': ocrVisible(region()) }} title={t("文字识别 · O")} aria-label={t("文字识别")} disabled={props.busy || exporting()} onClick={() => selectOcr(region())}><TextSelect size={18} /></button></Show>
      <Show when={translationEntries().length || savedTranslation(region())}><button data-caption={t("翻译")} class="capture-plugin" classList={{ 'capture-ocr-active': translationVisible(region()) }} title={t("翻译 · T")} aria-label={t("翻译")} disabled={blocked()} onClick={() => translate(region())}><Languages size={18} /></button></Show>
      <span class="capture-separator" />
      <Show when={scrollEntries().length}><button data-caption={t("长截图")} class="capture-plugin" title={t("长截图")} aria-label={t("长截图")} disabled={blocked() || Boolean(region().imageOverride)} onClick={() => scrollEntries().length === 1 ? runScroll(scrollEntries()[0], region()) : setPluginMenu(pluginMenu() === 'scroll' ? undefined : 'scroll')}><ScrollCaptureIcon /></button></Show>
      <button data-caption={t("录屏")} class="capture-record" title={t("区域录屏 · R")} aria-label={t("录制当前区域")} disabled={blocked() || Boolean(region().imageOverride)} onClick={() => props.onRecordRegion(region().id)}><Circle size={18} fill="currentColor" /></button>
      <span class="capture-separator" />
      <button data-caption={t("复制")} title={t("复制 · C")} aria-label={t("复制区域")} disabled={props.busy || exporting()} onClick={() => void exportRegion(region(), true)}><Copy size={18} /></button>
      <button data-caption={t("保存")} title={t("保存 · S")} aria-label={t("保存区域")} disabled={props.busy || exporting()} onClick={() => void exportRegion(region(), false)}><Download size={18} /></button>
      <Show when={pinEntries().length}><button data-caption={t("贴图")} title={t("贴图 · P")} aria-label={t("贴图")} disabled={blocked()} onClick={() => pinEntries().length === 1 ? void runPin(pinEntries()[0], region()) : setPluginMenu(pluginMenu() === 'pin' ? undefined : 'pin')}><Pin size={18} /></button></Show>
      <Show when={pluginMenu()}><div class="capture-plugin-menu" classList={{ above: toolbarBox().y + TOOLBAR_HEIGHT + 228 > viewport().height }} role="menu"><For each={pluginMenu() === 'ocr' ? ocrEntries() : pluginMenu() === 'scroll' ? scrollEntries() : pluginMenu() === 'translation' ? translationEntries() : pluginMenu() === 'pin' ? pinEntries() : workflowEntries()}>{entry => <button role="menuitem" disabled={blocked()} onClick={() => entry.contribution.kind === 'selection.ocr' ? runOcr(entry, region()) : entry.contribution.kind === 'selection.scroll' ? runScroll(entry, region()) : entry.contribution.kind === 'selection.translation' ? runTranslation(entry, region()) : entry.contribution.kind === 'selection.pin' ? void runPin(entry, region()) : runWorkflow(entry, region())}>{entry.contribution.title}</button>}</For></div></Show>
    </div>}</Show>
    <Show when={drawingSession() && regionById(drawingSession()!.regionId)}>{region => <DrawingEditor extraTools={props.blackboard ? () => <For each={workflowEntries()}>{entry => <button data-caption={entry.contribution.title} title={entry.contribution.title} aria-label={entry.contribution.title} disabled={blocked()} onClick={() => runWorkflow(entry,region())}><Table2 size={18}/></button>}</For> : undefined} onFinishBlackboard={props.onBlackboardClose} blackboard={props.blackboard} onRegisterHistory={props.onRegisterDrawingHistory} onRasterSnapshot={props.onRasterSnapshot} onRegisterFlush={props.onRegisterDrawingFlush} sceneId={drawingSession()!.sceneId} backgroundId={drawingSession()!.backgroundId} background={props.scene.background} region={imageProjection(region()).region} sourceRegion={props.scene.regions.find(value => value.id === region().id)} documentOnly={drawingSession()!.kind === 'document'} box={imageProjection(region()).box} scale={imageProjection(region()).scale} backgroundWidth={imageProjection(region()).width} backgroundHeight={imageProjection(region()).height} tools={drawingTools()} busy={props.busy} onPin={pinEntries().length ? () => runPin(pinEntries()[0], region()) : undefined} onCommand={command => { const session = drawingSession(); if (!session) return Promise.reject(Error('标注编辑已切换')); if (session.kind === 'document' || usesDrawingDocument(region(), command)) return props.onDrawingDocumentCommand(command); return props.onDrawingCommand(coreDrawingGrant.pluginId, coreDrawingGrant.revision, coreDrawingGrant.contributionId, command); }} onCopyTable={props.onCopyDrawingTable} onError={props.onError} onExport={(copy, closeSpace) => exportRegion(region(), copy, closeSpace)} onClose={closeDrawing} />}</Show>
    <div class="artifact-layer"><For each={props.scene.items.map(item => item.id)}>{id => <ArtifactCard onOpenBlackboard={typeof displayedItem(id).state?.blackboardSceneId === 'string' && props.onOpenBlackboard ? () => props.onOpenBlackboard!(id) : undefined} busy={props.busy} video={props.video ? { ...props.video, sceneId: props.scene.id, busy: props.busy, annotationBusy: props.scene.closed || props.scene.frozen || props.scene.run?.status === 'running', authoringBusy: props.authoringBusy, inputLocked: props.inputLocked, drawingAllowed: !drawingSession(), drawingGrant: videoDrawingGrant(), grant: videoGrant() } : undefined} item={displayedItem(id)} active={activeItemId() === id} onActivate={() => setActiveItemId(id)} referenced={props.refs.some(ref => ref.kind === 'item' && ref.id === id)} onReference={() => props.onReference({ kind: 'item', id })} onRemove={() => props.onRemoveItem(id)} onUpdate={updateItem} onCopy={props.onVideoCopy ? () => { void props.onVideoCopy!(id).catch(error => props.onError(error instanceof Error ? error.message : String(error))); } : undefined} onExport={() => { const item = props.scene.items.find(item => item.id === id); const saving = item?.asset.kind === 'text' ? exportTextAsset(item.asset.id) : item?.asset.kind === 'video' ? props.onVideoExport?.(id) : undefined; void saving?.catch(error => props.onError(error instanceof Error ? error.message : String(error))); }} />}</For></div>
  </main>;
}
