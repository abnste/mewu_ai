// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { nativeSelectOwnsEscape } from '../native-select-escape';
import { batch, createEffect, createMemo, createSignal, For, on, onCleanup, Show, untrack } from 'solid-js';
import { Check, ChevronDown, Circle, Copy, Languages, LoaderCircle, Pencil, Pin, Plus, Table2, TextSelect, Trash2, X } from 'lucide-solid';
import ScrollCaptureIcon from './ScrollCaptureIcon';
import ApplicationSnapshotIcon from './ApplicationSnapshotIcon';
import ExtensionSelectionTools from './ExtensionSelectionTools';
import { recordingGrant, recordingTrimGrant } from '../recording-audio';
import type { DrawingCommand, ImageExportVariant, OcrTarget, Reference, Region, RegionGeometry, Scene, SceneCommand, Snapshot, SpaceItem } from '../contracts';
import { geometryTarget, overlayRegion, sameGeometry, type RegionGeometryCoordinator, type GeometryView } from '../region-geometry';
import { geometryHistoryKey, type GeometryDirection } from '../region-geometry-history';
import type { PluginContribution, PluginRecord } from '../plugin-contracts';
import { assetUrl, captureBackgroundReady, exportRegion as exportNativeRegion, exportImageItem, exportTextAsset, native } from '../bridge';
import { capturePreparationPerformance } from '../capture-performance';
import { prepareCaptureBackground } from '../capture-presentation';
import { captureDisplayGeometry } from '../capture-display';
import { persistBackgroundItem, projectBackgroundItem } from '../media-item-geometry';
import ArtifactCard from './ArtifactCard';
import {isBlackboardObject} from '../blackboard-objects';
import { videoCopyKey } from '../video-copy';
import type { VideoArtifactProps } from './VideoArtifact';
import { coreDrawingGrant, coreDrawingTools, videoDrawingGrant as coreVideoDrawingGrant } from '../core-drawing';
import { isCoreReplacementPlugin } from '../plugin-core';
import DrawingLayer from './DrawingLayer';
import DrawingEditor from './DrawingEditor';
import ImageDrawingEditor from './ImageDrawingEditor';
import ImageExportButton from './ImageExportButton';
import { imageObjectBox } from '../image-annotations';
import { usesDrawingDocument, type DrawingSession } from '../drawing-document';
import type { DrawingLayoutTarget, DrawingTableFormat } from '../drawing-layout-preview';
import type { DrawingFlush, RegisterDrawingFlush } from '../drawing-flush';
import type {ObjectTrashPort} from '../object-trash';
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
import { SnapMapLoader, SnapSelectionGesture, SnapshotSelections, snapImagePoint, snapResize, snapTarget, type FrameSnapMap, type SnapNode } from '../window-snap';
import './capture.css';
import type { RegisterDrawingHistory } from '../drawing-editor-port';

interface Props {
  aiAvailable?: boolean;
  blackboardEnabled?: boolean;
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
  onRegisterObjectFlush?: RegisterDrawingFlush;
  scene: Scene;
  refs: Reference[];
  onAddRegion: (region: Region) => void | Promise<void>;
  geometryController: RegionGeometryCoordinator;
  geometryView: GeometryView;
  onGeometryReplay: (direction: GeometryDirection) => Promise<void>;
  onRemoveRegion: (id: string) => void;
  onReference: (reference: Reference) => void;
  onUpdateItem: (item: SpaceItem) => void | Promise<void>;
  onRemoveItem: (id: string) => void | Promise<void>;
  onError: (message: string) => void;
  onSelectionActivity?: (active: boolean) => void;
  onExportActivity?: (active: boolean) => void;
  onSelectionComplete?: (region: Region) => void;
  onActiveRegionChange?: (sceneId: string, regionId: string | undefined) => void;
  onFocusComposer?: () => void;
  onBeforeExport?: () => Promise<void>;
  onCopyClose?: (sceneId: string) => Promise<boolean | void>;
  onRecordRegion: (regionId: string) => void;
  onPin?: (pluginId: string, revision: number, contributionId: string, regionId: string) => Promise<boolean>;
  onPinItem?: (itemId: string) => Promise<boolean | void>;
  onScrollCapture?: (pluginId: string, revision: number, contributionId: string, target: OcrTarget, snapshotNodeId?: number) => Promise<void>;
  plugins: PluginRecord[];
  onPluginWorkflow: (pluginId: string, revision: number, contributionId: string, regionId: string) => Promise<void>;
  onDrawingCommand: (pluginId: string, pluginRevision: number, contributionId: string, command: DrawingCommand) => Promise<void>;
  onDrawingDocumentCommand: (command: DrawingCommand) => Promise<void>;
  onImageDrawingCommand?: (command: SceneCommand) => Promise<void>;
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
interface PendingSelection { sceneId: string; backgroundId: string; region: Region; version: number; automatic?: SnapNode; complete: boolean; acknowledged?: boolean }
interface PluginEntry { plugin: PluginRecord; contribution: PluginContribution }
type Handle = 'nw' | 'n' | 'ne' | 'w' | 'e' | 'sw' | 's' | 'se';
const CORNERS: Handle[] = ['nw', 'ne', 'sw', 'se'];
const EDGES: Handle[] = ['n', 'w', 'e', 's'];
const TOOLBAR_WIDTH = 405, TOOLBAR_HEIGHT = 60, EDGE = 6, GAP = 8, TOLERANCE = 24;
const contains = (box: Box, x: number, y: number, padding = 0) => x >= box.x - padding && x <= box.x + box.width + padding && y >= box.y - padding && y <= box.y + box.height + padding;
const overlaps = (a: Box, b: Box) => a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y;
const clamp = (value: number, low: number, high: number) => Math.max(low, Math.min(Math.max(low, high), value));

export default function SpaceCanvas(props: Props) {
  function backgroundLoaded(image: HTMLImageElement, backgroundId: string) {
    setNatural({ width: image.naturalWidth, height: image.naturalHeight });
    setBackgroundError(false);
    backgroundReady(image, backgroundId);
  }
  function backgroundReady(image: HTMLImageElement, backgroundId: string) {
    if (!native) return;
    const sceneId = props.scene.id, background = props.scene.background;
    if (background?.id !== backgroundId || !background.width || !background.height) return;
    const source = image.getAttribute('src');
    void prepareCaptureBackground(image, {
      sceneId, backgroundId, width: background.width, height: background.height,
    }, () => !disposed && !props.scene.closed && !props.scene.frozen
      && props.scene.id === sceneId && props.scene.background?.id === backgroundId
      && image.getAttribute('src') === source, captureBackgroundReady,
      () => capturePreparationPerformance({ sceneId, backgroundId, width: background.width!, height: background.height! })).catch(() => {
      // The capture owner reports a failed/canceled IPC. A stale callback cannot
      // wake it, show a window, or add another notification to a later scene.
    });
  }
  function backgroundAttached(image: HTMLImageElement, backgroundId: string) {
    // Solid sets the remaining attributes and connects this actual element in
    // the current turn. Start decode then, without waiting for the load event.
    queueMicrotask(() => backgroundReady(image, backgroundId));
  }
  function backgroundFailed(image: HTMLImageElement, backgroundId: string) {
    setBackgroundError(true); props.onError('无法读取屏幕截图');
    backgroundReady(image, backgroundId);
  }
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
  let newSelectionSource: (() => boolean) | undefined;
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
  const [viewport, setViewport] = createSignal({ width: innerWidth, height: innerHeight, deviceScale: devicePixelRatio });
  const [natural, setNatural] = createSignal({ width: 0, height: 0 });
  const [selection, setSelection] = createSignal<Box>();
  const [pendingSelection, setPendingSelection] = createSignal<PendingSelection>();
  const [snapMap, setSnapMap] = createSignal<FrameSnapMap>();
  const snapshotSelections = new SnapshotSelections();
  const [snapshotVersion, setSnapshotVersion] = createSignal(0);
  createEffect(() => { snapshotSelections.sync(props.scene.id, props.scene.background?.id, props.scene.regions); setSnapshotVersion(value => value + 1); });
  const snapshotNode = (region: Region) => { snapshotVersion(); return snapMap()?.backgroundId === props.scene.background?.id ? snapshotSelections.get(region, snapMap()) : undefined; };
  const [snapPreview, setSnapPreview] = createSignal<Box>();
  const [active, setActive] = createSignal('');
  const [activeItemId, setActiveItemId] = createSignal('');
  const [activity, setActivity] = createSignal(false);
  const [forceNew, setForceNew] = createSignal(false);
  const [toolbarVisible, setToolbarVisible] = createSignal(false);
  const [imageExportOpen, setImageExportOpen] = createSignal(false);
  let openRegionImageExport: (() => void) | undefined;
  const [toolbarBox, setToolbarBox] = createSignal<Box>({ x: 0, y: 0, width: TOOLBAR_WIDTH, height: TOOLBAR_HEIGHT });
  const [backgroundError, setBackgroundError] = createSignal(false);
  const [flashedRegion, setFlashedRegion] = createSignal('');
  const [drawingSession, setDrawingSession] = createSignal<DrawingSession>();
  const [imageDrawing, setImageDrawing] = createSignal<{sceneId:string;itemId:string;assetId:string}>();
  const imageDrawingItem = createMemo(() => { const target=imageDrawing(); return target?.sceneId===props.scene.id ? props.scene.items.find(item=>item.id===target.itemId&&item.asset.id===target.assetId&&item.asset.kind==='image') : undefined; });
  const closeImageDrawing = () => { setImageDrawing(undefined); setSelectionActivity(false); props.onFocusComposer?.(); };
  const beginImageDrawing = (id:string) => { const item=props.scene.items.find(item=>item.id===id); if(!item||blocked()||drawingSession()||props.scene.run?.status==='running'||!props.onImageDrawingCommand)return; hideToolbar();setActiveItemId(id);setImageDrawing({sceneId:props.scene.id,itemId:id,assetId:item.asset.id});setSelectionActivity(true); };
  const [pluginMenu, setPluginMenu] = createSignal<'workflow' | 'ocr' | 'scroll' | 'translation' | 'pin'>();
  const [extensionButtonCount, setExtensionButtonCount] = createSignal(0);
  const [extensionInteracting, setExtensionInteracting] = createSignal(false), [extensionRunning, setExtensionRunning] = createSignal(false);
  const [scrollMenuManual, setScrollMenuManual] = createSignal(false);
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
  const workflowEntries = createMemo(() => props.aiAvailable === false ? [] : pluginEntries().filter(value => value.contribution.kind === 'selection.workflow'));
  const ocrEntries = createMemo(() => pluginEntries().filter(value => value.contribution.kind === 'selection.ocr'));
  const scrollEntries = createMemo(() => pluginEntries().filter(value => value.contribution.kind === 'selection.scroll'));
  const translationEntries = createMemo(() => props.aiAvailable === false ? [] : pluginEntries().filter(value => value.contribution.kind === 'selection.translation'));
  const pinEntries = createMemo(() => pluginEntries().filter(value => value.contribution.kind === 'selection.pin'));
  const toolbarWidth = () => TOOLBAR_WIDTH - (props.aiAvailable === false ? 52 : 0) + 52 * ([pinEntries().length, workflowEntries().length, ocrEntries().length || activeRegion()?.ocr, scrollEntries().length, translationEntries().length || activeRegion()?.translation].filter(Boolean).length + extensionButtonCount());
  const drawingTools = () => [...coreDrawingTools];
  const geometry = createMemo(() => {
    const width = props.scene.background?.width || natural().width;
    const height = props.scene.background?.height || natural().height;
    return captureDisplayGeometry(props.scene.background, width, height, viewport(), viewport().deviceScale);
  });
  // One index per saved snapshot. Pointer/geometry previews must not run a
  // linear search for every property of every region on each move.
  const savedRegions = createMemo(() => new Map(props.scene.regions.map(region => [region.id, region])));
  const regionIds = createMemo(() => props.scene.regions.map(region => region.id));
  const regionById = (id: string) => {
    const saved = savedRegions().get(id);
    return saved && overlayRegion(props.scene, saved, props.geometryView.overlay);
  };
  const cropChanged = (region: Region) => !region.imageOverride && props.geometryView.overlay?.regionId === region.id && !sameGeometry(region, savedRegions().get(region.id)!);
  const backgroundAnchored = (item: SpaceItem) => item.asset.kind === 'video' && item.state?.coordinateSpace === 'background' && item.state.backgroundId === props.scene.background?.id && geometry().width > 0 && geometry().height > 0;
  const displayedItem = (id: string) => {
    const item = props.scene.items.find(item => item.id === id)!;
    return backgroundAnchored(item) ? projectBackgroundItem(item, geometry(), viewport()) : item;
  };
  const updateItem = async (item: SpaceItem) => {
    try { await props.onUpdateItem(backgroundAnchored(item) ? persistBackgroundItem(item, geometry(), viewport()) : item); }
    catch (error) { props.onError(error instanceof Error ? error.message : String(error)); throw error; }
  };
  const [objectTrash,setObjectTrash]=createSignal<ObjectTrashPort>(),[hoveredObject,setHoveredObject]=createSignal('');
  const objectFlushes=new Map<string,DrawingFlush>();
  const objectWheels=new Map<string,(event:WheelEvent)=>void>();
  const wheelObject=(event:WheelEvent)=>{if((event.target as Element).closest('button,input,textarea,.drawing-toolbar'))return false;const vp=viewport();const item=[...props.scene.items].reverse().find(item=>{const p=displayedItem(item.id);return (item.asset.kind==='image'||item.asset.kind==='video')&&event.clientX>=p.x*vp.width&&event.clientY>=p.y*vp.height&&event.clientX<(p.x+p.width)*vp.width&&event.clientY<(p.y+p.height)*vp.height;});if(!item)return false;objectWheels.get(item.id)?.(event);return event.defaultPrevented;};
  const registerObjectTrash=(port:ObjectTrashPort)=>{setObjectTrash(port);return()=>{if(objectTrash()===port)setObjectTrash(undefined);};};
  const flushObjects=async(active:()=>boolean)=>{for(const flush of [...objectFlushes.values()]){if(!active())throw Error('素材编辑已切换');await flush(active);}};
  // Board editors already compose their object flushes into DrawingFlushRegistry.
  // Plain capture objects need their own owner without displacing that editor.
  const unregisterObjects = props.onRegisterObjectFlush?.(async active => { if (!props.blackboard) await flushObjects(active); });
  onCleanup(() => unregisterObjects?.());
  const hoverObject=(event:PointerEvent)=>{if(event.buttons)return;const vp=viewport();setHoveredObject([...props.scene.items].reverse().find(item=>{const p=displayedItem(item.id);return event.clientX>=p.x*vp.width&&event.clientY>=p.y*vp.height&&event.clientX<(p.x+p.width)*vp.width&&event.clientY<(p.y+p.height)*vp.height;})?.id??'');};
  // A Solid component owns the list once; a reactive helper invocation
  // would remount every video/image when a scene receipt updates its props.
  const ArtifactItems=(itemProps:{board:boolean;interactive?:()=>boolean})=><For each={props.scene.items.filter(item=>!itemProps.board||isBlackboardObject(item.asset)).map(item=>item.id)}>{id=>{
    // Disposal invokes media observers before this For owner is fully gone.
    // Keep its last owned item available while removal cleans those observers.
    const item=createMemo<SpaceItem>(previous=>props.scene.items.some(value=>value.id===id)?displayedItem(id):previous,untrack(()=>displayedItem(id)));
    return <ArtifactCard aiAvailable={props.aiAvailable}
      sceneId={props.scene.id} imageEditing={imageDrawing()?.itemId===id} onEditImage={!itemProps.board&&!Object.hasOwn(item().state??{},'blackboardSceneId')&&props.onImageDrawingCommand?()=>beginImageDrawing(id):undefined}
      onRegisterWheel={itemProps.board?wheel=>{objectWheels.set(id,wheel);return()=>{if(objectWheels.get(id)===wheel)objectWheels.delete(id);};}:undefined}
      boardSceneId={itemProps.board?props.scene.id:undefined} onError={props.onError} blackboard={itemProps.board} interactive={itemProps.interactive?.()??true} hovered={hoveredObject()===id} trash={objectTrash()}
      onRegisterFlush={flush=>{objectFlushes.set(id,flush);return()=>{if(objectFlushes.get(id)===flush)objectFlushes.delete(id);};}}
      onOpenBlackboard={!itemProps.board&&typeof item().state?.blackboardSceneId==='string'&&props.onOpenBlackboard?()=>props.onOpenBlackboard!(id):undefined}
      onPin={props.onPinItem&&(item().asset.kind==='image'||item().asset.kind==='video')?()=>props.onPinItem!(id):undefined}
      busy={props.busy||(itemProps.board&&props.blackboardEnabled===false)}
      video={props.video?{...props.video,sceneId:props.scene.id,busy:props.busy,annotationBusy:props.scene.closed||props.scene.frozen||props.scene.run?.status==='running',authoringBusy:props.authoringBusy,inputLocked:props.inputLocked,drawingAllowed:!drawingSession(),drawingGrant:videoDrawingGrant(),grant:videoGrant()}:undefined}
      item={item()} active={activeItemId()===id} onActivate={()=>setActiveItemId(id)} referenced={props.refs.some(ref=>ref.kind==='item'&&ref.id===id)} onReference={()=>props.onReference({kind:'item',id})} onRemove={()=>props.onRemoveItem(id)} onUpdate={updateItem}
      onCopy={props.onVideoCopy?()=>{void props.onVideoCopy!(id).catch(error=>props.onError(error instanceof Error?error.message:String(error)));}:undefined}
      onExportImage={variant=>{const current=props.scene.items.find(value=>value.id===id);if(current?.asset.kind==='image')void exportImage(current,false,false,variant);}}
      onExport={()=>{const item=props.scene.items.find(item=>item.id===id);const saving=item?.asset.kind==='text'?exportTextAsset(item.asset.id):item?.asset.kind==='video'?props.onVideoExport?.(id):undefined;void saving?.catch(error=>props.onError(error instanceof Error?error.message:String(error)));}}/>;}}</For>;
  const activeRegion = createMemo(() => regionById(active()));
  const activeRegionIdentity = createMemo(() => [props.scene.id, activeRegion()?.id] as const, undefined,
    { equals: (previous, next) => previous[0] === next[0] && previous[1] === next[1] });
  createEffect(on(activeRegionIdentity, ([sceneId, regionId]) => {
    props.onActiveRegionChange?.(sceneId, regionId);
  }));
  onCleanup(() => props.onActiveRegionChange?.(props.scene.id, undefined));
  const ocrTarget = (region: Region): OcrTarget => ({ sceneId: props.scene.id, regionId: region.id, backgroundId: props.scene.background!.id, drawingRevision: region.drawingRevision ?? 0, x: region.x, y: region.y, width: region.width, height: region.height });
  const savedOcr = (region: Region) => region.ocr?.backgroundId === props.scene.background?.id && region.ocr?.drawingRevision === (region.drawingRevision ?? 0) && !cropChanged(region) ? region.ocr : undefined;
  const savedTranslation = (region: Region) => region.translation?.backgroundId === props.scene.background?.id && region.translation?.sourceId === (region.imageOverride?.id ?? props.scene.background?.id) && region.translation?.drawingRevision === (region.drawingRevision ?? 0) && !cropChanged(region) ? region.translation : undefined;
  const selectedTextMode = (region: Region) => textMode()[region.id] ?? (savedTranslation(region) ? 'translation' : 'ocr');
  const ocrVisible = (region: Region) => active() === region.id && selectedTextMode(region) === 'ocr' && !hiddenOcr()[region.id] && !drawingSession() && !forceNew() && Boolean(savedOcr(region)?.document.lines.length);
  const translationVisible = (region: Region) => active() === region.id && selectedTextMode(region) === 'translation' && !drawingSession() && !forceNew() && Boolean(savedTranslation(region)?.selection.lines.length);
  const display = (region: Box): Box => ({ x: geometry().x + region.x * geometry().scale, y: geometry().y + region.y * geometry().scale, width: region.width * geometry().scale, height: region.height * geometry().scale });
  const imageProjection = (region: Region) => regionImageProjection(region, display(region), props.scene.background!, geometry().scale);
  // Hit testing is paused during a pointer gesture; do not project all saved
  // boxes just to maintain an unused hover cache on every captured move.
  const regions = createMemo(() => activity() ? [] : props.scene.regions.map(region => ({ region, box: display(overlayRegion(props.scene, region, props.geometryView.overlay)) })));
  // Presentation only: pending geometry never enters the saved region/tool lists.
  const selectionPreview = createMemo(() => {
    if (selection()) return selection();
    const pending = pendingSelection();
    if (!pending || !currentPendingSelection(pending) || props.scene.regions.some(region => region.id === pending.region.id)) return;
    return display(pending.region);
  });
  const point = (event: PointerEvent) => ({ x: clamp(event.clientX, geometry().x, geometry().x + geometry().width), y: clamp(event.clientY, geometry().y, geometry().y + geometry().height) });
  const referenced = (id: string) => props.refs.some(ref => ref.kind === 'region' && ref.id === id);
  const blocked = () => props.busy || exporting() || settling() || extensionRunning() || props.scene.run?.status === 'running';
  const scanSource = createMemo(() => {
    if (!props.onCodeSource || !codeFocused() || props.busy || drawingSession() || selection() || pendingSelection() || forceNew() || backgroundError() || props.geometryView.pending || props.geometryView.held) return;
    return codeSource(props.scene, active(), props.plugins);
  }, undefined, { equals: (a, b) => a?.key === b?.key });
  const codesPaused = () => Boolean(activity() || exporting() || settling() || pluginMenu() || ocrPending() || props.translationPending || codeModal());
  const visibleCodes = createMemo(() => { const view = props.codeView; return view?.key === scanSource()?.key && view?.result.codes.length ? view : undefined; });
  createEffect(on(() => [scanSource(), codesPaused()] as const, ([source, paused]) => props.onCodeSource?.(source, paused)));
  const snapUnavailable = () => blocked() || activity() || pendingSelection() || forceNew() || drawingSession() || pluginMenu() || ocrPending() || props.translationPending || props.geometryView.pending || props.geometryView.held || props.scene.frozen || props.scene.closed || backgroundError();
  function clearSnapPreview() { snapPointer = undefined; setSnapPreview(undefined); }
  function candidateAt(x: number, y: number) {
    const map = snapMap(), bg = props.scene.background, box = geometry();
    if (!bg || map?.backgroundId !== bg.id) return;
    const pixel = snapImagePoint(x, y, box, box.scale);
    return pixel && snapTarget(map, pixel.x, pixel.y);
  }
  function previewCandidateAt(pointer: { x: number; y: number; shiftKey: boolean }) {
    if (snapUnavailable() || pointer.shiftKey || !document.hasFocus()) return;
    const target = document.elementFromPoint(pointer.x, pointer.y), composer = composerBox();
    if (!target?.closest('.capture-surface') || target.closest('button,a,input,textarea,select,[contenteditable]:not([contenteditable="false"]),.artifact-card,.capture-region,.capture-toolbar,.selection-code-card,.ocr-text-layer,.drawing-editor') || document.querySelector('.modal-backdrop,.history-backdrop,.channel-popover,.reply-table-menu,[role="menu"]') || (toolbarVisible() && contains(toolbarBox(), pointer.x, pointer.y, TOLERANCE)) || (composer && contains(composer, pointer.x, pointer.y)) || regions().some(entry => contains(entry.box, pointer.x, pointer.y, 5))) return;
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
  function hideToolbar() { clearHideTimer(); if(imageExportOpen())return; setToolbarVisible(false); setPluginMenu(undefined); }
  function scheduleHide() {
    if (!toolbarVisible() || extensionInteracting() || imageExportOpen() || hideTimer !== undefined) return;
    // One timer per departure. Continuous movement outside must not reset it.
    hideTimer = setTimeout(() => { hideTimer = undefined; if (!extensionInteracting() && !imageExportOpen()) setToolbarVisible(false); }, 300);
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
    if (pendingSelection() && pendingSelection()!.region.id !== region.id) clearPendingSelection();
    clearHideTimer(); setActive(region.id); placeToolbar(region); setToolbarVisible(true);
  }
  function hover(event: PointerEvent) {
    if (extensionInteracting()) { clearHideTimer(); return; }
    if (event.target instanceof Element && event.target.closest('.selection-code-card')) { clearSnapPreview(); clearHideTimer(); return; }
    updateSnapPreview({ x: event.clientX, y: event.clientY, shiftKey: event.shiftKey });
    if (activity() || forceNew() || exporting() || props.geometryView.held || props.geometryView.pending) return;
    const target = event.target instanceof Element ? event.target : undefined;
    if ((pluginMenu()) && target?.closest('.capture-toolbar')) { clearHideTimer(); return; }
    const composer = composerBox();
    if (!target?.closest('.capture-surface') || target.closest('.artifact-card') || (composer && contains(composer, event.clientX, event.clientY))) { scheduleHide(); return; }
    if (toolbarVisible() && contains(toolbarBox(), event.clientX, event.clientY, TOLERANCE)) { clearHideTimer(); return; }
    // Last drawn wins for overlapping regions, matching the capture surface.
    const candidates = regions();
    for (let index = candidates.length - 1; index >= 0; index--) {
      if (contains(candidates[index].box, event.clientX, event.clientY, 5)) {
        revealToolbar(regionById(candidates[index].region.id)!); return;
      }
    }
    scheduleHide();
  }
  function setSelectionActivity(value: boolean) {
    if (activity() === value) return;
    setActivity(value); props.onSelectionActivity?.(value);
  }
  function closeDrawing() { if(props.blackboard){void props.onBlackboardClose?.();return;} setDrawingSession(undefined); setSelectionActivity(false); const region = activeRegion(); if (region) revealToolbar(region); props.onFocusComposer?.(); }
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
  function runScroll(entry: PluginEntry, region: Region, manual = false) {
    if (blocked() || region.imageOverride || !props.scene.background || entry.contribution.kind !== 'selection.scroll' || !props.onScrollCapture) return;
    setPluginMenu(undefined); ocrRequests.cancel(); props.onTranslationCancel?.();
    void settledRegion(region, saved => props.onScrollCapture!(entry.plugin.manifest.id, entry.plugin.revision, entry.contribution.id, ocrTarget(saved), manual ? undefined : snapshotNode(saved)));
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
  function trackPointer(event: PointerEvent, move: (event: PointerEvent) => void, finish: (committed: boolean) => void, sourceCurrent?: () => boolean) {
    ocrRequests.cancel(); props.onTranslationCancel?.(); cancelGesture?.(); event.preventDefault(); event.stopPropagation();
    interactionVersion++;
    clearPendingSelection();
    clearSnapPreview();
    window.getSelection()?.removeAllRanges(); hideToolbar(); setSelectionActivity(true);
    const pointerId = event.pointerId;
    newSelectionSource = sourceCurrent;
    let ended = false;
    const cleanup = () => {
      canvas.removeEventListener('pointermove', moved); canvas.removeEventListener('pointerup', released);
      canvas.removeEventListener('pointercancel', canceled); canvas.removeEventListener('lostpointercapture', canceled);
      cancelGesture = undefined;
      newSelectionSource = undefined;
      if (canvas.hasPointerCapture(pointerId)) canvas.releasePointerCapture(pointerId);
    };
    const end = (committed: boolean) => { if (ended) return; ended = true; cleanup(); batch(() => { setSelectionActivity(false); finish(committed); }); };
    const moved = (next: PointerEvent) => { if (next.pointerId === pointerId) { if (sourceCurrent && !sourceCurrent()) { end(false); return; } next.preventDefault(); move(next); } };
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
    clearPendingSelection();
    setActiveItemId('');
    endNudge();
    const start = point(event), scale = geometry().scale, offset = { x: geometry().x, y: geometry().y };
    const sourceGeometry = { ...geometry() }, sourceViewport = { ...viewport() };
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
      const pending: PendingSelection = { sceneId, backgroundId, region, version, automatic: gesture.isAutomatic ? automatic : undefined, complete: false };
      setPendingSelection(pending);
      setActive(region.id);
      void Promise.resolve().then(() => {
        if (currentPendingSelection(pending)) return props.onAddRegion(region);
      }).then(() => {
        if (currentPendingSelection(pending)) setPendingSelection(current => current && { ...current, complete: true });
      }).catch(error => {
        if (!currentPendingSelection(pending)) return;
        clearPendingSelection(); props.onError(error instanceof Error ? error.message : '无法添加区域');
      });
    }, () => !disposed && !document.hidden && !props.inputLocked && !props.blackboard && !props.scene.closed && !props.scene.frozen && !blocked()
      && props.scene.id === sceneId && props.scene.background?.id === backgroundId && sameGeometry(geometry(), sourceGeometry) && geometry().scale === scale
      && viewport().width === sourceViewport.width && viewport().height === sourceViewport.height && viewport().deviceScale === sourceViewport.deviceScale);
  }
  function currentPendingSelection(pending: PendingSelection) {
    return !disposed && !document.hidden && !props.inputLocked && !props.blackboard && !props.scene.closed && !props.scene.frozen
      && props.scene.run?.status !== 'running' && props.scene.id === pending.sceneId && props.scene.background?.id === pending.backgroundId
      && interactionVersion === pending.version && pendingSelection()?.region.id === pending.region.id;
  }
  function clearPendingSelection() {
    const pending = pendingSelection();
    if (pending && active() === pending.region.id) setActive('');
    setPendingSelection(undefined);
  }
  function reconcilePendingSelection() {
    const pending = pendingSelection();
    if (!pending) return;
    const current = currentPendingSelection(pending), saved = props.scene.regions.find(region => region.id === pending.region.id);
    untrack(() => {
      if (!current || saved && (!sameGeometry(saved, pending.region) || saved.imageOverride)) { clearPendingSelection(); return; }
      if (!saved) { if (pending.acknowledged) clearPendingSelection(); return; }
      if (!pending.complete) { if (!pending.acknowledged) setPendingSelection({ ...pending, acknowledged: true }); return; }
      setPendingSelection(undefined);
      if (pending.automatic) { snapshotSelections.bind(saved, pending.automatic); setSnapshotVersion(value => value + 1); }
      revealToolbar(saved); props.onSelectionComplete?.(saved);
    });
  }
  createEffect(reconcilePendingSelection);
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
    clearPendingSelection();
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
  const resize = () => { clearPendingSelection(); clearSnapPreview(); cancelDeferredPointer?.(); cancelGesture?.(); endNudge(); setViewport({ width: innerWidth, height: innerHeight, deviceScale: devicePixelRatio }); const region = activeRegion(); if (region && toolbarVisible()) placeToolbar(region); };
  const leave = () => { clearSnapPreview(); scheduleHide(); };
  const blur = () => { clearPendingSelection(); setCodeFocused(false); clearSnapPreview(); cancelDeferredPointer?.(); cancelGesture?.(); endNudge(); hideToolbar(); };
  const codeFocus = () => setCodeFocused(!document.hidden);
  const snapFocus = (event: FocusEvent) => { if (event.target instanceof Element && event.target.closest('button,a,input,textarea,select,[contenteditable]:not([contenteditable="false"]),.composer,.video-trim-popover,.artifact-card,.selection-code-card,.ocr-text-layer')) clearSnapPreview(); };
  const snapVisibility = () => { setCodeFocused(!document.hidden && document.hasFocus()); if (document.hidden) { clearPendingSelection(); clearSnapPreview(); cancelDeferredPointer?.(); cancelGesture?.(); } };
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
    if ((event.target instanceof Element ? event.target : document.activeElement)?.closest('.selection-code-card,.video-trim-popover,.artifact-video,[data-run-journal],.extension-task-popover,.extension-command-menu')) return;
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
    else if (['c', 's', 'enter'].includes(event.key.toLowerCase())) { event.preventDefault(); event.stopImmediatePropagation(); if(event.key.toLowerCase()==='s'){const sceneId=props.scene.id,regionId=region.id;revealToolbar(region);queueMicrotask(()=>{if(!disposed&&props.scene.id===sceneId&&activeRegion()?.id===regionId&&!blocked()&&!exporting())openRegionImageExport?.();});}else void exportRegion(region, true, event.key === 'Enter'); }
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
    clearPendingSelection();
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
    untrack(() => { ocrRequests.cancel(); setOcrNotice(undefined); setHiddenOcr({}); setTextMode({}); interactionVersion++; cancelDeferredPointer?.(); cancelGesture?.(); endNudge(); setDrawingSession(undefined); setImageDrawing(undefined); setSelectionActivity(false); hideToolbar(); setActive(''); setActiveItemId(''); setFlashedRegion(''); setForceNew(false); setBackgroundError(false); });
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
    // A previous region's save does not change a new selection's pixel source.
    // Existing-region move/resize gestures retain the strict list/source fence.
    untrack(() => { cancelDeferredPointer?.(); if (!newSelectionSource?.()) cancelGesture?.(); endNudge(); });
  });
  createEffect(() => {
    const session = drawingSession();
    if (session && (session.sceneId !== props.scene.id || session.backgroundId !== props.scene.background?.id || !props.scene.regions.some(region => region.id === session.regionId && (region.imageOverride?.id ?? props.scene.background?.id) === session.sourceId) || props.scene.frozen || props.scene.closed || props.scene.run?.status === 'running')) untrack(() => { setDrawingSession(undefined); setSelectionActivity(false); hideToolbar(); });
  });
  createEffect(() => {
    if(imageDrawing()&&(!imageDrawingItem()||props.scene.frozen||props.scene.closed||props.scene.run?.status==='running'))untrack(closeImageDrawing);
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
  async function crop(region: Region, variant: ImageExportVariant = 'annotated'): Promise<Blob> {
    if (!props.scene.background) throw new Error('没有截图');
    if (variant === 'annotated' && (region.drawings?.length || region.translation)) throw new Error('请在桌面版导出带标注的图片');
    const source = region.imageOverride;
    const img = new window.Image(); img.crossOrigin = 'anonymous'; img.src = assetUrl(source ?? props.scene.background);
    await img.decode();
    const imageCanvas = document.createElement('canvas'); imageCanvas.width = source?.width ?? region.width; imageCanvas.height = source?.height ?? region.height;
    const context = imageCanvas.getContext('2d'); if (!context) throw new Error('无法处理截图');
    if (source) context.drawImage(img, 0, 0, imageCanvas.width, imageCanvas.height);
    else context.drawImage(img, region.x, region.y, region.width, region.height, 0, 0, region.width, region.height);
    return new Promise((resolve, reject) => imageCanvas.toBlob(blob => blob ? resolve(blob) : reject(new Error('无法处理截图')), 'image/png'));
  }
  function imageEditorBox(item:SpaceItem):Box {
    return imageObjectBox(item,viewport());
  }
  async function exportImage(item:SpaceItem,copy:boolean,closeSpace=false,variant:ImageExportVariant='annotated'):Promise<boolean> {
    if(exporting()||props.busy)return false;
    const sceneId=props.scene.id,itemId=item.id,assetId=item.asset.id;let fence:string|undefined;
    const saved=()=>props.scene.items.find(value=>value.id===itemId&&value.asset.id===assetId);
    const current=()=>!disposed&&props.scene.id===sceneId&&!props.scene.closed&&!props.scene.frozen&&Boolean(saved())&&(fence===undefined||JSON.stringify(saved())===fence);
    setExporting(true);props.onExportActivity?.(true);
    try{return await finishCapture({prepare:async()=>{await props.onBeforeExport?.();fence=JSON.stringify(saved());},current,export:()=>exportImageItem(sceneId,itemId,assetId,copy,copy?'annotated':variant),close:copy&&closeSpace?()=>props.onCopyClose?.(sceneId)??Promise.resolve(false):undefined});}
    catch(error){if(!disposed)props.onError(error instanceof Error?error.message:String(error));return false;}
    finally{if(!disposed){setExporting(false);props.onExportActivity?.(false);}}
  }
  async function exportRegion(region: Region, copy: boolean, closeSpace = false, variant: ImageExportVariant = 'annotated'): Promise<boolean> {
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
        if (native) return exportNativeRegion(sceneId, region.id, copy, copy ? 'annotated' : variant);
        else {
          const blob = await crop(savedRegion, copy ? 'annotated' : variant);
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
    <Show when={!props.blackboard && pointerNative && props.scene.background}>{background => <PointerInspector sceneId={props.scene.id} background={background()} imageBox={geometry()} dimensions={pointerSelectionDimensions(selection(), geometry(), activeRegion())} disabled={props.busy || exporting() || props.scene.frozen || props.scene.closed || backgroundError() || Boolean(drawingSession() || imageDrawing() || pluginMenu() || ocrPending() || props.translationPending) || props.scene.regions.some(region => ocrVisible(region) || translationVisible(region))} blockedAt={(x, y) => toolbarVisible() && contains(toolbarBox(), x, y, TOLERANCE)} onReady={value => { pointerInspector = value; }} onError={props.onError} />}</Show>
    <Show when={props.scene.background?.id} keyed>{id => <img ref={image => backgroundAttached(image, id)} class="screen-background capture-background" data-asset={id} src={assetUrl(props.scene.background!)} style={geometry().nativePixels ? { left: '0px', top: '0px', width: `${geometry().width}px`, height: `${geometry().height}px` } : undefined} alt={t("屏幕截图")} draggable={false} decoding="sync" onLoad={event => backgroundLoaded(event.currentTarget, id)} onError={event => backgroundFailed(event.currentTarget, id)} />}</Show>
    <svg class="capture-mask" width="100%" height="100%" aria-hidden="true">
      <defs><mask id="region-cutouts" maskUnits="userSpaceOnUse"><rect width="100%" height="100%" fill="white" />
        <For each={regionIds()}>{id => {
          const region = createMemo(() => regionById(id)!);
          const box = createMemo(() => display(region()), undefined, { equals: sameGeometry });
          return <rect x={box().x} y={box().y} width={box().width} height={box().height} fill="black" />;
        }}</For>
        <Show when={selectionPreview()}>{box => <rect x={box().x} y={box().y} width={box().width} height={box().height} fill="black" />}</Show>
      </mask></defs>
      <rect width="100%" height="100%" fill="#000" fill-opacity={!props.blackboard&&props.scene.background ? props.maskOpacity : '0'} mask="url(#region-cutouts)" />
    </svg>
    <Show when={snapPreview()}>{box => <div class="capture-snap-preview" aria-hidden="true" style={{ left: `${box().x}px`, top: `${box().y}px`, width: `${box().width}px`, height: `${box().height}px` }} />}</Show>
    <For each={regionIds()}>{(id, index) => {
      const region = createMemo(() => regionById(id)!);
      const box = createMemo(() => display(region()), undefined, { equals: sameGeometry });
      const image = createMemo(() => imageProjection(region()));
      return <section class="capture-region" classList={{ 'capture-active': active() === id, 'capture-located': flashedRegion() === id, 'capture-image-override': Boolean(region().imageOverride) }} style={{ left: `${box().x}px`, top: `${box().y}px`, width: `${box().width}px`, height: `${box().height}px` }} onPointerDown={event => editRegion(event, region())} aria-label={`区域 ${index() + 1}`}>
        <div class="capture-region-content" style={{ left: `${image().box.x - box().x}px`, top: `${image().box.y - box().y}px`, width: `${image().box.width}px`, height: `${image().box.height}px` }}>
          <Show when={region().imageOverride?.id} keyed>{assetId => <img data-asset={assetId} class="capture-override-image" src={assetUrl(region().imageOverride!)} alt={t("长截图")} draggable={false} onError={() => props.onError('无法读取长截图')} />}</Show>
          <Show when={savedTranslation(region())}>{value => <TranslationLayer value={value()} active={active() === id && !drawingSession()} disabled={blocked()} onRemove={props.onTranslationRemove ? () => settledRegion(region(), saved => props.onTranslationRemove!(ocrTarget(saved))) : undefined} onError={props.onError} />}</Show>
          <Show when={translationVisible(region())}><OcrTextLayer document={savedTranslation(region())!.selection} label={t("译文")} repeatLabel={t("重新翻译")} active={active() === id} onClose={() => setTextMode(old => ({ ...old, [id]: 'none' }))} onRepeat={translationEntries().length ? () => translate(region(), true) : undefined} onError={props.onError} onSelecting={setSelectionActivity} /></Show>
          <Show when={ocrVisible(region())}><OcrTextLayer document={savedOcr(region())!.document} active={active() === id} onClose={() => { ocrRequests.cancel(); setHiddenOcr(old => ({ ...old, [id]: true })); }} onRepeat={ocrEntries().length ? () => runOcr(ocrEntries()[0], region()) : undefined} onError={props.onError} onSelecting={setSelectionActivity} /></Show>
        </div>
        <Show when={ocrPending()?.target.regionId === id || ocrNotice()?.regionId === id}><div class="ocr-status" role="status" onPointerDown={event => event.stopPropagation()}><Show when={ocrPending()?.target.regionId === id}><LoaderCircle size={13} class="spin" /></Show><span title={ocrNotice()?.regionId === id ? ocrNotice()!.text : undefined}>{ocrPending()?.target.regionId === id ? t("正在识别") : ocrNotice()?.text}</span><button title={ocrPending()?.target.regionId === id ? t("取消识别") : t("关闭")} aria-label={ocrPending()?.target.regionId === id ? t("取消识别") : t("关闭识别提示")} onClick={() => { ocrRequests.cancel(); setOcrNotice(undefined); }}><X size={13} /></button></div></Show>
        <span class="capture-region-number">{index() + 1}</span>
        <Show when={props.translationPending?.request.target.regionId === id || props.translationNotice?.regionId === id}><div class="ocr-status" role="status" onPointerDown={event => event.stopPropagation()}><Show when={props.translationPending?.request.target.regionId === id}><LoaderCircle size={13} class="spin" /></Show><span>{props.translationPending?.request.target.regionId === id ? props.translationPending.progress?.phase === 'rendering' ? t("正在排版") : props.translationPending.progress?.phase === 'translating' ? `正在翻译${props.translationPending.progress.total ? ` ${props.translationPending.progress.completed}/${props.translationPending.progress.total}` : ''}` : t("正在识别") : props.translationNotice?.text}</span><button title={props.translationPending ? t("取消翻译") : t("关闭")} aria-label={props.translationPending ? t("取消翻译") : t("关闭翻译提示")} onClick={() => { props.onTranslationCancel?.(); props.onClearTranslationNotice?.(); }}><X size={13} /></button></div></Show>
        <Show when={active() === id && !forceNew() && !drawingSession()}>
          <For each={EDGES}>{edge => <i class={`capture-edge capture-edge-${edge}`} onPointerDown={event => editRegion(event, region(), edge)} aria-hidden="true" />}</For>
          <For each={CORNERS}>{corner => <i class={`capture-handle capture-handle-${corner}`} onPointerDown={event => editRegion(event, region(), corner)} aria-hidden="true" />}</For>
        </Show>
      </section>;
    }}</For>
    <div class="capture-saved-drawings" aria-hidden="true">
      <For each={regionIds()}>{id => {
        const region = createMemo(() => regionById(id)!);
        const image = createMemo(() => imageProjection(region()));
        return <div class="capture-region-content" style={{ left: `${image().box.x}px`, top: `${image().box.y}px`, width: `${image().box.width}px`, height: `${image().box.height}px` }}>
          <DrawingLayer hidden={drawingSession()?.regionId === id} region={image().region} sourceRegion={savedRegions().get(id)} sceneId={props.scene.id} background={props.scene.background} />
        </div>;
      }}</For>
    </div>
    <Show when={selectionPreview()}>{box => <div class="capture-selection" style={{ left: `${box().x}px`, top: `${box().y}px`, width: `${box().width}px`, height: `${box().height}px` }} />}</Show>
    {/* Native region observation temporarily hides this window and causes blur; keep the invocation owner mounted. */}
    <Show when={(toolbarVisible() || extensionInteracting()) && !drawingSession() && !imageDrawing() && !activity() && !forceNew() && activeRegion()}>{region => <div ref={bindToolbar} class="capture-toolbar" role="toolbar" aria-label={t("截图工具")} style={{ left: `${toolbarBox().x}px`, top: `${toolbarBox().y}px` }} onPointerDown={event => { event.stopPropagation(); if (window.getSelection()?.anchorNode?.parentElement?.closest('.ocr-text-layer')) event.preventDefault(); }} onPointerEnter={clearHideTimer}>
      <Show when={props.aiAvailable !== false}><button data-caption={t("引用")} class="capture-reference" classList={{ 'capture-referenced': referenced(region().id) }} title={referenced(region().id) ? t("已引用") : t("引用当前区域")} aria-label={t("引用当前区域")} disabled={blocked()} onClick={() => { if (!referenced(region().id)) props.onReference({ kind: 'region', id: region().id }); props.onFocusComposer?.(); }}>@</button></Show>
      <button data-caption={t("加选区")} title={t("添加区域 · Shift + 拖动")} aria-label={t("添加截图区域")} disabled={blocked()} onClick={() => { setForceNew(true); hideToolbar(); }}><Plus size={19} /></button>
      <button data-caption={t("删除")} class="capture-delete" title={t("删除区域")} aria-label={t("删除当前区域")} disabled={blocked()} onClick={() => { const id = region().id; hideToolbar(); setActive(''); props.onRemoveRegion(id); }}><Trash2 size={17} /></button>
      <span class="capture-separator" />
      <button data-caption={t("绘制")} class="capture-plugin" title={t("绘制 · D")} aria-label={t("绘制")} disabled={blocked()} onClick={() => beginDrawing(region())}><Pencil size={18} /></button>
      <Show when={workflowEntries().length}><button data-caption={workflowEntries().length === 1 ? workflowEntries()[0].contribution.title : t("图像处理")} class="capture-plugin" title={workflowEntries().length === 1 ? workflowEntries()[0].contribution.title : t("图像处理")} aria-label={workflowEntries().length === 1 ? workflowEntries()[0].contribution.title : t("图像处理")} disabled={blocked()} onClick={() => workflowEntries().length === 1 ? runWorkflow(workflowEntries()[0], region()) : setPluginMenu(pluginMenu() === 'workflow' ? undefined : 'workflow')}><Table2 size={18} /></button></Show>
      <ExtensionSelectionTools sceneId={props.scene.id} regionId={region().id} draft={props.scene.draft} disabled={blocked() || Boolean(region().imageOverride)} onCountChange={setExtensionButtonCount} onInteractionChange={setExtensionInteracting} onRunningChange={setExtensionRunning} onError={props.onError}/>
      <Show when={ocrEntries().length || savedOcr(region())?.document.lines.length}><button data-caption={t("文字")} class="capture-plugin" classList={{ 'capture-ocr-active': ocrVisible(region()) }} title={t("文字识别 · O")} aria-label={t("文字识别")} disabled={props.busy || exporting()} onClick={() => selectOcr(region())}><TextSelect size={18} /></button></Show>
      <Show when={translationEntries().length || savedTranslation(region())}><button data-caption={t("翻译")} class="capture-plugin" classList={{ 'capture-ocr-active': translationVisible(region()) }} title={t("翻译 · T")} aria-label={t("翻译")} disabled={blocked()} onClick={() => translate(region())}><Languages size={18} /></button></Show>
      <span class="capture-separator" />
      <Show when={scrollEntries().length}><button data-caption={t(snapshotNode(region()) ? '快照' : '长截图')} class="capture-plugin" title={t(snapshotNode(region()) ? '快照（Shift：手动长截图）' : '长截图')} aria-label={t(snapshotNode(region()) ? '快照' : '长截图')} disabled={blocked() || Boolean(region().imageOverride)} onClick={event => { if (scrollEntries().length === 1) runScroll(scrollEntries()[0], region(), event.shiftKey); else { setScrollMenuManual(event.shiftKey); setPluginMenu(pluginMenu() === 'scroll' ? undefined : 'scroll'); } }}><Show when={snapshotNode(region())} fallback={<ScrollCaptureIcon />}><ApplicationSnapshotIcon /></Show></button></Show>
      <button data-caption={t("录屏")} class="capture-record" title={t("区域录屏 · R")} aria-label={t("录制当前区域")} disabled={blocked() || Boolean(region().imageOverride)} onClick={() => props.onRecordRegion(region().id)}><Circle size={18} fill="currentColor" /></button>
      <span class="capture-separator" />
      <button data-caption={t("复制")} title={t("复制 · C")} aria-label={t("复制区域")} disabled={props.busy || exporting()} onClick={() => void exportRegion(region(), true)}><Copy size={18} /></button>
      <ImageExportButton scope={JSON.stringify([props.scene.id,props.scene.background?.id,region().id,region().x,region().y,region().width,region().height,region().imageOverride?.id])} label={t('保存区域')} disabled={props.busy || exporting()} onRegister={open=>{openRegionImageExport=open;return()=>{if(openRegionImageExport===open)openRegionImageExport=undefined;};}} onOpenChange={value=>{setImageExportOpen(value);if(value){setPluginMenu(undefined);clearHideTimer();}else scheduleHide();}} onSelect={variant=>void exportRegion(region(),false,false,variant)} />
      <Show when={pinEntries().length}><button data-caption={t("贴图")} title={t("贴图 · P")} aria-label={t("贴图")} disabled={blocked()} onClick={() => pinEntries().length === 1 ? void runPin(pinEntries()[0], region()) : setPluginMenu(pluginMenu() === 'pin' ? undefined : 'pin')}><Pin size={18} /></button></Show>
      <Show when={pluginMenu()}><div class="capture-plugin-menu" classList={{ above: toolbarBox().y + toolbarBox().height + 228 > viewport().height }} role="menu"><For each={pluginMenu() === 'ocr' ? ocrEntries() : pluginMenu() === 'scroll' ? scrollEntries() : pluginMenu() === 'translation' ? translationEntries() : pluginMenu() === 'pin' ? pinEntries() : workflowEntries()}>{entry => <button role="menuitem" disabled={blocked()} onClick={event => entry.contribution.kind === 'selection.ocr' ? runOcr(entry, region()) : entry.contribution.kind === 'selection.scroll' ? runScroll(entry, region(), scrollMenuManual() || event.shiftKey) : entry.contribution.kind === 'selection.translation' ? runTranslation(entry, region()) : entry.contribution.kind === 'selection.pin' ? void runPin(entry, region()) : runWorkflow(entry, region())}>{entry.contribution.title}</button>}</For></div></Show>
    </div>}</Show>
    <Show when={drawingSession() && regionById(drawingSession()!.regionId)}>{region => <DrawingEditor inputLocked={props.blackboard&&props.blackboardEnabled===false} onObjectWheel={props.blackboard?wheelObject:undefined} onObjectPointerMove={hoverObject} onObjectPointerLeave={()=>setHoveredObject('')} objects={props.blackboard?interactive=><div class="blackboard-artifact-layer"><ArtifactItems board interactive={interactive}/></div>:undefined} objectsFlush={props.blackboard?flushObjects:undefined} onRegisterObjectTrash={props.blackboard?registerObjectTrash:undefined} extraTools={props.blackboard ? () => <For each={workflowEntries()}>{entry => <button data-caption={entry.contribution.title} title={entry.contribution.title} aria-label={entry.contribution.title} disabled={blocked()} onClick={() => runWorkflow(entry,region())}><Table2 size={18}/></button>}</For> : undefined} onFinishBlackboard={props.onBlackboardClose} blackboard={props.blackboard} onRegisterHistory={props.onRegisterDrawingHistory} onRasterSnapshot={props.onRasterSnapshot} onRegisterFlush={props.onRegisterDrawingFlush} sceneId={drawingSession()!.sceneId} backgroundId={drawingSession()!.backgroundId} background={props.scene.background} region={imageProjection(region()).region} sourceRegion={props.scene.regions.find(value => value.id === region().id)} documentOnly={drawingSession()!.kind === 'document'} box={imageProjection(region()).box} scale={imageProjection(region()).scale} backgroundWidth={imageProjection(region()).width} backgroundHeight={imageProjection(region()).height} tools={drawingTools()} busy={props.busy} onPin={pinEntries().length ? () => runPin(pinEntries()[0], region()) : undefined} onCommand={command => { const session = drawingSession(); if (!session) return Promise.reject(Error('标注编辑已切换')); if (session.kind === 'document' || usesDrawingDocument(region(), command)) return props.onDrawingDocumentCommand(command); return props.onDrawingCommand(coreDrawingGrant.pluginId, coreDrawingGrant.revision, coreDrawingGrant.contributionId, command); }} onCopyTable={props.onCopyDrawingTable} onError={props.onError} onExport={(copy, closeSpace, variant) => exportRegion(region(), copy, closeSpace, variant)} onClose={closeDrawing} />}</Show>
    <Show when={imageDrawingItem()}>{item => <ImageDrawingEditor sceneId={props.scene.id} item={item()} box={imageEditorBox(item())} busy={props.busy} inputLocked={props.inputLocked} onRegisterFlush={props.onRegisterDrawingFlush} onRegisterHistory={props.onRegisterDrawingHistory} onCopyTable={props.onCopyDrawingTable} onCommand={value=>props.onImageDrawingCommand?.(value)??Promise.reject(Error('图片编辑不可用'))} onClose={closeImageDrawing} onError={props.onError} onExport={(copy,closeSpace,variant)=>exportImage(item(),copy,closeSpace,variant)} onPin={props.onPinItem?async()=>await props.onPinItem!(item().id)!==false:undefined} />}</Show>
    <Show when={!props.blackboard||!drawingSession()}><div class="artifact-layer"><ArtifactItems board={Boolean(props.blackboard)}/></div></Show>
  </main>;
}
