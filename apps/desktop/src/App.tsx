// SPDX-License-Identifier: MPL-2.0
import { subscribeImportPins } from './import-pin-events';
import PinObjectLayer from './components/PinObjectLayer';
import { referencePinObject, subscribePinObjects, type PinObject } from './pin-objects';
import { syncNativeLanguage } from './language-bridge';
import { t } from "./i18n";
import { preferenceKey, readPreferences } from './interface-preferences';
import { updateLanguage } from './i18n';
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show } from 'solid-js';
import { CircleAlert, LoaderCircle, Pencil, Redo2, SquarePen, Undo2, X } from 'lucide-solid';
import { isBlackboard } from './blackboard';
import { coreDrawingGrant } from './core-drawing';
import type { DrawingHistoryControls, RegisterDrawingHistory } from './drawing-editor-port';
import * as bridge from './bridge';
import './settings-window.css';
import * as plugins from './plugin-bridge';
import type { PluginSnapshot } from './plugin-contracts';
import type { DrawingCommand } from './contracts';
import * as drawingLayouts from './drawing-layout-bridge';
import { usesDrawingDocument } from './drawing-document';
import type { DrawingLayoutTarget, DrawingTableFormat } from './drawing-layout-preview';
import type { AgentProfile, ConnectionProfile, DiscoverMcpServer, Reference, Region, RunEvent, SceneCommand, Snapshot, VisualAnnotationGrant } from './contracts';
import SpaceCanvas from './components/SpaceCanvas';
import Composer from './components/Composer';
import SessionDock from './components/SessionDock';
import SettingsDialog, { type Preferences, type SettingsTab } from './components/SettingsDialog';
import { RecordingBackdrop } from './RecordingSurface';
import { ScrollBackdrop } from './ScrollSurface';
import * as scrollBridge from './scroll-bridge';
import * as translationBridge from './translation-bridge';
import * as pinBridge from './pin-bridge';
import * as voiceBridge from './voice-bridge';
import * as codeBridge from './code-bridge';
import * as videoBridge from './video-bridge';
import * as recordingAudioBridge from './recording-audio-bridge';
import { RecordingAudioController, recordingAudioGrants, recordingGrant, sameAudioGrant, sameAudioSelection, validRecordingTrimGrant } from './recording-audio';
import { validVideoDrawingGrant, coreVideoDrawingTools } from './core-drawing';
import { VideoFlushRegistry, VideoPauseRegistry, VideoRequestLane, type VideoRequest } from './video-requests';
import * as videoAnnotationBridge from './video-annotation-bridge';
import * as videoDrawingBridge from './video-drawing-bridge';
import type { VideoDrawingAction, VideoDrawingGrant } from './video-drawing-contracts';
import { acceptVideoDrawingReceipt, assertVideoDrawingDraftsSaved } from './video-drawing-session';
import { currentVideoAnswer, sameVideoAnnotationTarget, videoAnnotationIdentity, videoAnnotationTarget, VideoPlayerRegistry, videoSourceIdentity } from './video-annotations';
import type { VideoAnnotationAction, VideoAnnotationTarget, VideoAnswerAction, VideoTextContent, VideoTextLayoutRef } from './video-annotation-contracts';
import { assertVideoTextDraftsSaved, movedVideoObject } from './video-manual-edit';
import { videoAnnotationBounds } from './video-annotations';
import type { VideoAction, VideoGrant, VideoTarget, VideoExportState } from './video-contracts';
import { sameRange, TrimCanceled } from './video-trim';
import type { SpaceItem } from './contracts';
import { CodeScanController, codeSource } from './code-scan';
import type { CodeAction, CodeScanView, CodeSource } from './code-contracts';
import { VoiceInput, voiceLanguages, type VoiceScope } from './voice-input';
import type { VoiceCapabilities, VoiceLanguage, VoicePending } from './voice-contracts';
import { cancelPinOnEscape, preparePin } from './pin-interaction';
import { nativeSelectOwnsEscape } from './native-select-escape';
import { TranslationRequests, translationScopeValid, type TranslationPending, type TranslationRequest } from './translation-request';
import type { OcrTarget } from './contracts';
import { ExitPreparation, pendingSceneDrafts, persistSceneDrafts, SpaceOperations } from './exit-preparation';
import { RegionGeometryCoordinator, geometryReceipt, geometryTarget, type GeometryView } from './region-geometry';
import type { RegionGeometryCommand } from './contracts';
import { submitSceneDraft } from './scene-submission';
import { annotationGrant, annotationSendIdentity, assertAnnotationSend, hasAnnotationTarget } from './visual-annotations-send';
import { geometryHistoryCommand, geometryHistoryReceipt, type GeometryDirection } from './region-geometry-history';
import { DrawingFlushRegistry } from './drawing-flush';
import DrawingDraftsDialog from './components/DrawingDraftsDialog';
import { discardUnsavedDrawingDraft, listUnsavedDrawingDrafts, type PendingDrawingDraft } from './components/drawing-properties';
import { ContinuationController } from './run-journal';
import type { ContinuationDecision, RunJournalSummary } from './journal-contracts';


function withoutScene<T>(values: Record<string, T>, sceneId: string): Record<string, T> {
  const next = { ...values }; delete next[sceneId]; return next;
}

export default function App() {
  const drawingFlush = new DrawingFlushRegistry();
  const videoFlush = new VideoFlushRegistry();
  const videoDrawingFlush = new VideoFlushRegistry();
  const videoPlayers = new VideoPauseRegistry();
  const videoNavigation = new VideoPlayerRegistry();
  const videoLane = new VideoRequestLane(videoBridge.cancelVideoRequest);
  const videoCommits = new Set<Promise<unknown>>();
  const videoDrawingCommits = new Set<Promise<unknown>>();
  const [videoExport, setVideoExport] = createSignal<VideoExportState>();
  let exportRequest: VideoRequest<void | boolean> | undefined,
    videoOutputIntent: { id: string; canceled: boolean } | undefined, stopVideoEvents: (() => void) | undefined;
  let drawingFlushDepth = 0;
  let videoFlushDepth = 0;
  let videoDrawingFlushDepth = 0;
  let drawingDraftScope: string | undefined;
  const [pendingDrawingDrafts, setPendingDrawingDrafts] = createSignal<PendingDrawingDraft[]>([]);
  const [snapshot, setSnapshot] = createSignal<Snapshot>();
  const [pluginSnapshot, setPluginSnapshot] = createSignal<PluginSnapshot>({ revision: -1, plugins: [] });
  const [voiceCapabilities, setVoiceCapabilities] = createSignal<VoiceCapabilities>();
  const [voiceCapabilitiesRetry, setVoiceCapabilitiesRetry] = createSignal(false);
  const [voiceCapabilitiesReading, setVoiceCapabilitiesReading] = createSignal(false);
  const [voicePending, setVoicePending] = createSignal<VoicePending>();
  const [voiceLanguage, setVoiceLanguage] = createSignal<VoiceLanguage>((() => { try { const value = JSON.parse(localStorage.getItem('mewu.voice.v1') ?? '"system"'); return ['system', 'zh-CN', 'en-US'].includes(value) ? value : 'system'; } catch { return 'system'; } })());
  let voiceComposing = false, voiceSubscribed = false, stopVoiceEvents: (() => void) | undefined;
  const [drafts, setDrafts] = createSignal<Record<string, string>>({});
  const [referenceDrafts, setReferenceDrafts] = createSignal<Record<string, Reference[]>>({});
  const [streams, setStreams] = createSignal<Record<string, { runId: string; text: string; reasoning?: string }>>({});
  const [expanded, setExpanded] = createSignal<Record<string, boolean>>({});
  const [composerPositions, setComposerPositions] = createSignal<Record<string, { x: number; y: number } | undefined>>({});
  const [hotkey, setHotkey] = createSignal('');
  const [settings, setSettings] = createSignal<SettingsTab>();
  const [error, setError] = createSignal('');
  const [sendErrors, setSendErrors] = createSignal<Record<string, string>>({});
  const [busy, setBusy] = createSignal(false);
  const [exitPreparing, setExitPreparing] = createSignal(false);
  const [sending, setSending] = createSignal(false);
  const [closingScene, setClosingScene] = createSignal<string>();
  const [sessionsOpen, setSessionsOpen] = createSignal(false);
  const [selectionActive, setSelectionActive] = createSignal(false);
  const [exportActive, setExportActive] = createSignal(false);
  const [blackboardEditing, setBlackboardEditing] = createSignal<string>();
  const [drawingHistoryControls, setDrawingHistoryControls] = createSignal<DrawingHistoryControls>();
  const registerDrawingHistory:RegisterDrawingHistory=controls=>{setDrawingHistoryControls(controls);return()=>{if(drawingHistoryControls()===controls)setDrawingHistoryControls(undefined);};};
  const [focusRequest, setFocusRequest] = createSignal(0);
  const [recording, setRecording] = createSignal<bridge.RecordingStatus | null>(null);
  const [recordingReady, setRecordingReady] = createSignal(false);
  let stopRecordingAudio: (() => void) | undefined, audioRead: Promise<void> | undefined;
  const [scroll, setScroll] = createSignal<scrollBridge.ScrollStatus | null>(null);
  const [scrollReady, setScrollReady] = createSignal(false);
  const [translations, setTranslations] = createSignal<TranslationPending[]>([]);
  const [translationNotices, setTranslationNotices] = createSignal<Record<string, { regionId: string; text: string }>>({});
  const [preferences, setPreferences] = createSignal(readPreferences());
  const [geometryView, setGeometryView] = createSignal<GeometryView>({ pending: false, held: false });
  const [codeIntent, setCodeIntent] = createSignal<{ source?: CodeSource; paused: boolean }>({ paused: false });
  const [codeView, setCodeView] = createSignal<CodeScanView>();
  const [codesReady, setCodesReady] = createSignal(false);
  const [codeActionBusy, setCodeActionBusy] = createSignal(false);
  let stopCodeEvents: (() => void) | undefined;
  const scene = createMemo(() => snapshot()?.scenes.find(s => s.id === snapshot()?.activeSceneId));
  createEffect(() => { const current=scene(); if(current?.blackboardLink) setBlackboardEditing(current.id); });
  const refs = () => scene() ? referenceDrafts()[scene()!.id] ?? scene()!.refs : [];
  const draft = () => scene() ? drafts()[scene()!.id] ?? scene()!.draft : '';
  const visualAnnotationGrant = createMemo(() => annotationGrant(pluginSnapshot().plugins));
  const voiceEntry = createMemo(() => pluginSnapshot().plugins.filter(plugin => plugin.state === 'enabled' && !plugin.error).flatMap(plugin => plugin.manifest.contributions.filter(contribution => contribution.kind === 'input.speech-to-text').map(contribution => ({ plugin, contribution })))[0]);
  function voiceScope(): VoiceScope | undefined {
    const current = scene(), entry = voiceEntry();
    if (!current || !entry || disposed || busy() || sending() || exitPreparing() || recording() || scroll() || selectionActive() || settings() || sessionsOpen() || pendingDrawingDrafts().length || current.closed || current.frozen || current.run?.status === 'running' || document.visibilityState === 'hidden') return;
    return { sceneId: current.id, agentId: current.agentId, backgroundId: current.background?.id ?? null, runId: current.run?.id ?? null, pluginId: entry.plugin.manifest.id, revision: entry.plugin.revision, contributionId: entry.contribution.id };
  }
  const voice = new VoiceInput({ current: voiceScope, run: request => voiceSubscribed && voice.isActive(request.requestId) ? voiceBridge.runDictation(request) : Promise.reject(new Error('语音输入尚未就绪')), cancel: voiceBridge.cancelDictation, composing: () => voiceComposing, draft, setDraft, changed: setVoicePending, error: showError });
  function toggleVoice() {
    if (voicePending()) { void voice.cancel().catch(showError); return; }
    if (voiceCapabilitiesRetry()) { void readVoiceCapabilities(); return; }
    const capabilities = voiceCapabilities();
    if (!capabilities?.supported) { showError(capabilities?.unavailableReason || '语音输入不可用'); return; }
    if (!voiceLanguages(capabilities).some(option => option.value === voiceLanguage())) { showError('当前语音语言未安装'); return; }
    voice.start(voiceLanguage());
  }
  function chooseVoiceLanguage(value: VoiceLanguage) { setVoiceLanguage(value); try { localStorage.setItem('mewu.voice.v1', JSON.stringify(value)); } catch { /* Session choice remains valid. */ } }
  async function readVoiceCapabilities() {
    if (disposed || voiceCapabilitiesReading()) return;
    setVoiceCapabilitiesReading(true);
    try {
      if (!voiceSubscribed) {
        stopVoiceEvents = await voiceBridge.subscribeVoice(value => voice.progress(value), requestId => voice.invalidate(requestId));
        if (disposed) { stopVoiceEvents(); return; }
        voiceSubscribed = true;
      }
      const capabilities = await voiceBridge.getVoiceCapabilities();
      if (!disposed) { setVoiceCapabilities(capabilities); setVoiceCapabilitiesRetry(false); }
    } catch (error) {
      if (!disposed) { setVoiceCapabilities({ supported: false, languages: [], unavailableReason: error instanceof Error ? error.message : String(error) }); setVoiceCapabilitiesRetry(true); }
    } finally { if (!disposed) setVoiceCapabilitiesReading(false); }
  }
  let stopImportPins: (() => void) | undefined;
  const [pinObjects, setPinObjects] = createSignal<PinObject[]>([]);
  let pinObjectsSubscription: Awaited<ReturnType<typeof subscribePinObjects>> | undefined;
  createEffect(() => { scene()?.id; scene()?.background?.id; void pinObjectsSubscription?.refresh(); });
  const pinItem = (object: PinObject) => scene()?.items.find(item => item.asset.id === object.attachmentAssetId);
  const pinReferenced = (object: PinObject) => { const item=pinItem(object);return !!item && refs().some(ref=>ref.kind==='item' && ref.id===item.id); };
  async function referencePin(object: PinObject) {
    if(busy() || sending() || exitPreparing())return;
    const item=pinItem(object);
    if(pinReferenced(object) && item){toggleReference({kind:'item',id:item.id});return;}
    const sceneId=scene()?.id;if(!sceneId)return;
    setBusy(true);
    try{await flush();if(scene()?.id!==sceneId)throw Error('会话已切换');const next=await referencePinObject(object.id,sceneId);if(!next)throw Error('无法引用贴图');clearReferenceDraft(sceneId);accept(next);await pinObjectsSubscription?.refresh();}
    catch(cause){showError(cause);}finally{setBusy(false);}
  }
  const canvasScene = () => { const current=scene()!;const ids=new Set(pinObjects().map(object=>object.attachmentAssetId));return {...current,items:current.items.filter(item=>!ids.has(item.asset.id))}; };
  let stopEvents: (() => void) | undefined;
  let stopRecordingEvents: (() => void) | undefined;
  let stopScrollEvents: (() => void) | undefined;
  let stopPluginEvents: (() => void) | undefined;
  let stopExitEvents: (() => void) | undefined;
  let stopTranslationEvents: (() => void) | undefined;
  let stopPinEscape: (() => void) | undefined;
  let pendingPin: { requestId: string; canceled: boolean } | undefined;
  let disposed = false;
  function codeMatches(source: CodeSource) {
    return !disposed && !exitPreparing() && !busy() && !recording() && !scroll() && document.visibilityState !== 'hidden' && codeSource(scene(), source.request.target.regionId, pluginSnapshot().plugins)?.key === source.key;
  }
  const codes = new CodeScanController({ run: codeBridge.scanCodes, cancel: codeBridge.cancelCodeScan, changed: setCodeView, matches: codeMatches });
  function acceptCodeSource(source: CodeSource | undefined, paused: boolean) {
    setCodeIntent(previous => previous.source?.key === source?.key && previous.paused === paused ? previous : { source, paused });
  }
  createEffect(on(() => [codeIntent(), codesReady(), busy(), exitPreparing(), recording(), scroll(), settings(), sessionsOpen(), pendingDrawingDrafts().length] as const, ([intent, ready, busy, exiting, recording, scroll, settings, sessions, drafts]) => {
    codes.setSource(ready && !busy && !exiting && !recording && !scroll ? intent.source : undefined, intent.paused || Boolean(settings || sessions || drafts));
  }));
  async function codeAction(sourceToken: string, codeId: string, action: CodeAction): Promise<boolean> {
    if (codeActionBusy() || !codes.isCurrent(sourceToken)) return false;
    const finish = spaceOperations.begin(); setCodeActionBusy(true);
    try {
      if (action === 'open') await flush();
      if (!codes.isCurrent(sourceToken) || exitPreparing()) return false;
      await codeBridge.actOnCode(sourceToken, codeId, action);
      return true;
    } catch (error) { if (!disposed && !exitPreparing()) showError(error); return false; }
    finally { if (!disposed) setCodeActionBusy(false); finish(); }
  }
  const voiceControl = createMemo(() => voiceEntry() ? { capabilities: voiceCapabilities(), readingCapabilities: voiceCapabilitiesReading(), retryCapabilities: voiceCapabilitiesRetry(), language: voiceLanguage(), pending: voicePending(), disabled: !voiceScope(), onLanguage: chooseVoiceLanguage, onToggle: toggleVoice } : undefined);
  let commands: Promise<void> = Promise.resolve();
  let debounce: ReturnType<typeof setTimeout> | undefined;
  let errorTimer: ReturnType<typeof setTimeout> | undefined;
  let autosaveVersion = 0;
  let exitFailure: unknown;
  const spaceOperations = new SpaceOperations();
  const recordingAudio = new RecordingAudioController(undefined, () => {});
  recordingAudio.reconcile(recordingAudioGrants());
  async function readRecordingAudio() {
    if (audioRead || disposed) return audioRead;
    audioRead = (async () => {
      try {
        if (stopRecordingAudio) recordingAudio.readSucceeded(await recordingAudioBridge.getRecordingAudio());
        else {
          stopRecordingAudio = await recordingAudioBridge.subscribeRecordingAudio(state => recordingAudio.readSucceeded(state));
          if (disposed) stopRecordingAudio();
        }
      } catch (error) { if (!disposed) recordingAudio.readFailed(error); }
      finally { audioRead = undefined; }
    })();
    return audioRead;
  }
  const regionGeometry = new RegionGeometryCoordinator({
    current: (sceneId, regionId) => scene()?.id === sceneId ? geometryTarget(scene(), regionId) : undefined,
    commit: commitGeometry,
    finish: (sceneId, editId) => command({ type: 'finish_region_geometry_edit', sceneId, editId }),
    changed: setGeometryView,
    error: showError,
    beginOperation: () => spaceOperations.begin(),
  });
  const translationConnections = new Map<string, string>();
  const translationRequests = new TranslationRequests<Snapshot>({
    run: async request => {
      await commands;
      const current = snapshot();
      if (!translationRequests.has(request.requestId) || !current || !translationScopeValid(request, current, pluginSnapshot().plugins, translationConnections.get(request.requestId) ?? '')) throw new Error('翻译目标已变化');
      return translationBridge.runTranslation(request);
    },
    cancel: translationBridge.cancelTranslation,
    changed: values => {
      setTranslations([...values]);
      const ids = new Set(values.map(value => value.request.requestId));
      for (const id of translationConnections.keys()) if (!ids.has(id)) translationConnections.delete(id);
    },
    result: (value, request) => {
      accept(value);
      const result = value.scenes.find(value => value.id === request.target.sceneId)?.regions.find(value => value.id === request.target.regionId)?.translation;
      if (!result?.document.lines.length) setTranslationNotices(old => ({ ...old, [request.target.sceneId]: { regionId: request.target.regionId, text: '未识别到文字' } }));
    },
    error: (text, request) => setTranslationNotices(old => ({ ...old, [request.target.sceneId]: { regionId: request.target.regionId, text } })),
  });
  const exitPreparation = new ExitPreparation({
    lock: locked => {
      regionGeometry.pauseInput(locked);
      if (locked) {
        void videoLane.cancelAll().catch(showError);
        void codes.cancel().catch(showError);
        void voice.cancel().catch(showError);
        cancelPendingPin();
        translationRequests.cancelAll();
        exitFailure = undefined;
        const focused = document.activeElement;
        if (focused instanceof HTMLElement && focused.closest('.app')) focused.blur();
      }
      setExitPreparing(locked);
      if (!locked && !disposed) resumeDraftSave();
    },
    beforePrepare: async active => {
      await flushVideoDrawing(active);
      // TXT writes require RUNNING, just like accepted video authoring.
      await flushDrawing(active);
    },
    beginPreparation: bridge.beginExitPreparation,
    flush: flushForExit,
    finish: bridge.finishExitPreparation,
    error: showError,
  });
  const accept = (next: Snapshot) => {
    if (disposed) return;
    const previous = snapshot();
    if (previous?.revision !== undefined && next.revision !== undefined && next.revision < previous.revision) return;
    for (const current of next.scenes) {
      if (current.closed && previous?.scenes.some(scene => scene.id === current.id && !scene.closed)) clearSceneState(current.id);
    }
    setSnapshot(next);
    voice.reconcile();
    regionGeometry.reconcile();
  };
  const acceptPlugins = (next: PluginSnapshot) => {
    if (!disposed && next.revision >= pluginSnapshot().revision) {
      if (next.revision !== pluginSnapshot().revision) videoPlayers.pauseAll();
      setPluginSnapshot(next); voice.reconcile();
    }
  };
  function showError(value: unknown) {
    if (exitPreparing()) exitFailure ??= value;
    setError(value instanceof Error ? value.message : String(value));
    clearTimeout(errorTimer); errorTimer = setTimeout(() => setError(''), 6500);
  }
  function runEvent(event: RunEvent) {
    const target = snapshot()?.scenes.find(s => s.id === event.sceneId);
    if (!target || target.closed || target.run?.id !== event.runId) return;
    if (event.text || event.reasoning) setStreams(old => {
      const previous = old[event.sceneId]?.runId === event.runId ? old[event.sceneId] : undefined;
      return { ...old, [event.sceneId]: { runId: event.runId, text: (previous?.text ?? '') + (event.text ?? ''), reasoning: (previous?.reasoning ?? '') + (event.reasoning ?? '') } };
    });
    if (event.error) setSendErrors(old => ({ ...old, [event.sceneId]: event.error! }));
  }
  function clearReferenceDraft(sceneId: string) {
    setReferenceDrafts(old => { const next = { ...old }; delete next[sceneId]; return next; });
  }
  function clearSceneState(sceneId: string) {
    if (voicePending()?.sceneId === sceneId) void voice.cancel().catch(showError);
    translationRequests.cancelScene(sceneId);
    setTranslationNotices(old => withoutScene(old, sceneId));
    setDrafts(old => withoutScene(old, sceneId));
    setReferenceDrafts(old => withoutScene(old, sceneId));
    setStreams(old => withoutScene(old, sceneId));
    setSendErrors(old => withoutScene(old, sceneId));
    setExpanded(old => withoutScene(old, sceneId));
    setComposerPositions(old => withoutScene(old, sceneId));
  }
  function recordingEvent(next: bridge.RecordingStatus | null) {
    if (disposed) return;
    const previous = recording();
    if (next) {
      void voice.cancel().catch(showError);
      clearReferenceDraft(next.sceneId);
      setSettings(undefined); setSessionsOpen(false);
    }
    setRecording(next);
    if (previous && !next) {
      clearReferenceDraft(previous.sceneId); setSelectionActive(false); setFocusRequest(value => value + 1);
      // Completion publishes a snapshot, but querying also covers a missed event.
      void bridge.getSnapshot().then(accept).catch(showError);
    }
  }
  function scrollEvent(next: scrollBridge.ScrollStatus | null) {
    if (disposed) return;
    const previous = scroll();
    if (next) { void voice.cancel().catch(showError); clearReferenceDraft(next.sceneId); setSettings(undefined); setSessionsOpen(false); }
    setScroll(next);
    if (previous && !next) {
      clearReferenceDraft(previous.sceneId); setSelectionActive(false);
      void bridge.getSnapshot().then(value => {
        accept(value);
        if (!disposed && !exitPreparing() && !scroll() && scene()?.id === previous.sceneId) setFocusRequest(value => value + 1);
      }).catch(showError);
    }
  }
  onMount(async () => {
    try { pinObjectsSubscription = await subscribePinObjects(setPinObjects, showError); if(disposed){pinObjectsSubscription.stop();return;} } catch(cause){showError(cause);}
    await syncNativeLanguage(preferences().uiLanguage).catch(showError);
    try { stopImportPins = await subscribeImportPins(showError); if (disposed) { stopImportPins(); return; } } catch (error) { showError(error); }
    try {
      stopExitEvents = await bridge.subscribeExitPreparation(request => { void exitPreparation.prepare(request); }, request => exitPreparation.cancel(request));
      if (disposed) { stopExitEvents(); return; }
    } catch (error) { showError(error); }
    try {
      stopPinEscape = await pinBridge.subscribePinEscape(() => { if (!disposed) document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })); });
      if (disposed) { stopPinEscape(); return; }
    } catch (error) { showError(error); }
    try {
      stopTranslationEvents = await translationBridge.subscribeTranslation(value => translationRequests.progress(value));
      if (disposed) { stopTranslationEvents(); return; }
    } catch (error) { showError(error); }
    try {
      stopPluginEvents = await plugins.subscribePlugins(acceptPlugins);
      if (disposed) { stopPluginEvents(); return; }
      acceptPlugins(await plugins.getPlugins());
    } catch (error) { showError(error); }
    try {
      stopRecordingEvents = await bridge.subscribeRecording(recordingEvent);
      if (disposed) { stopRecordingEvents(); return; }
      setRecordingReady(true);
    } catch (error) { showError(error); }
    try {
      stopScrollEvents = await scrollBridge.subscribeScroll(scrollEvent);
      if (disposed) { stopScrollEvents(); return; }
      setScrollReady(true);
    } catch (error) { showError(error); }
    try {
      stopEvents = await bridge.subscribe(accept, runEvent, showError);
      if (disposed) { stopEvents(); return; }
      accept(await bridge.getSnapshot());
      try { const info = await bridge.runtimeInfo(); if (!disposed) setHotkey(info.hotkey); } catch { /* A missing status query does not block the space. */ }
    } catch (error) { showError(error); }
  });
  onMount(() => { void readVoiceCapabilities(); });
  onMount(() => { void readRecordingAudio(); });
  onCleanup(() => { recordingAudio.dispose(); stopRecordingAudio?.(); });
  onMount(() => { void videoBridge.subscribeVideoProgress(value => { setVideoExport(old => old?.requestId === value.requestId ? { ...old, percent: value.percent } : old); }).then(stop => { if (disposed) stop(); else stopVideoEvents = stop; }).catch(showError); });
  onCleanup(() => { stopVideoEvents?.(); void videoLane.dispose().catch(() => {}); });
  onMount(() => { if (codeBridge.codeNative) void codeBridge.subscribeCodeInvalidated(id => codes.invalidate(id)).then(stop => { if (disposed) stop(); else { stopCodeEvents = stop; setCodesReady(true); } }).catch(() => {}); });
  onCleanup(() => { codes.dispose(); stopCodeEvents?.(); });
  onCleanup(() => { disposed = true; pinObjectsSubscription?.stop(); stopImportPins?.(); voice.dispose(); stopVoiceEvents?.(); regionGeometry.dispose(); cancelPendingPin(); stopPinEscape?.(); translationRequests.dispose(); stopTranslationEvents?.(); exitPreparation.dispose(); stopExitEvents?.(); stopEvents?.(); stopRecordingEvents?.(); stopScrollEvents?.(); stopPluginEvents?.(); clearTimeout(debounce); clearTimeout(errorTimer); });
  createEffect(() => { voiceScope(); voice.reconcile(); });
  createEffect(on(() => scene()?.id, () => { voiceComposing = false; }));
  const voiceVisibility = () => { if (document.visibilityState === 'hidden') void voice.cancel().catch(showError); };
  document.addEventListener('visibilitychange', voiceVisibility);
  onCleanup(() => document.removeEventListener('visibilitychange', voiceVisibility));
  createEffect(() => {
    const current = snapshot(), installed = pluginSnapshot().plugins;
    if (current) translationRequests.retain(request => translationScopeValid(request, current, installed, translationConnections.get(request.requestId) ?? ''));
  });
  function startTranslation(request: TranslationRequest) {
    if (exitPreparing() || busy() || recording() || scroll()) return;
    const current = snapshot(), target = current?.scenes.find(value => value.id === request.target.sceneId);
    const connection = current?.connections.find(value => value.id === target?.connectionId);
    if (!connection) { setTranslationNotices(old => ({ ...old, [request.target.sceneId]: { regionId: request.target.regionId, text: '请先选择连接' } })); return; }
    translationRequests.cancelScene(request.target.sceneId);
    translationConnections.set(request.requestId, `${connection.id}:${connection.revision}`);
    setTranslationNotices(old => withoutScene(old, request.target.sceneId));
    void translationRequests.start(request);
  }
  async function removeTranslation(target: OcrTarget) {
    if (exitPreparing() || recording() || scroll()) throw new Error('当前无法移除译文');
    const finishOperation = spaceOperations.begin();
    translationRequests.cancelScene(target.sceneId);
    const next = commands.then(async () => accept(await translationBridge.clearTranslation(target)));
    commands = next.catch(error => { if (exitPreparing()) exitFailure ??= error; });
    try { await next; } finally { finishOperation(); }
  }
  const preferencesChanged = (event: StorageEvent) => {
    if (event.key === preferenceKey || event.key === null) { const next = readPreferences(); if (next.translationLanguage !== preferences().translationLanguage) translationRequests.cancelAll(); updateLanguage(next.uiLanguage); setPreferences(next); }
  };
  window.addEventListener('storage', preferencesChanged);
  onCleanup(() => window.removeEventListener('storage', preferencesChanged));
  function command(value: SceneCommand): Promise<void> {
    if (scroll() && value.type !== 'set_draft' && value.type !== 'set_refs' && value.type !== 'finish_region_geometry_edit') return Promise.reject(new Error('长截图进行中'));
    const stopVoice = value.type === 'set_agent' ? voice.cancel() : Promise.resolve();
    const next = commands.then(async () => { await stopVoice; accept(await bridge.applyCommand(value)); });
    commands = next.catch(error => { if (exitPreparing()) exitFailure ??= error; });
    return next;
  }
  function commitGeometry(value: RegionGeometryCommand) {
    const next = commands.then(async () => {
      const returned = await bridge.applyCommand(value);
      const receipt = geometryReceipt(returned, value);
      accept(returned);
      return receipt;
    });
    commands = next.then(() => {}, error => { if (exitPreparing()) exitFailure ??= error; });
    return next;
  }
  async function replayGeometry(direction: GeometryDirection) {
    const sceneId = scene()?.id;
    if (!sceneId || busy() || sending() || selectionActive() || exportActive() || recording() || scroll() || exitPreparing() || scene()?.run?.status === 'running') return;
    const finishOperation = spaceOperations.begin(); setBusy(true);
    try {
      await regionGeometry.flush();
      const next = commands.then(async () => {
        const current = scene(); if (!current || current.id !== sceneId) throw new Error('会话已变化');
        const value = geometryHistoryCommand(current, direction); if (!value) return;
        const entry = current.geometryHistory![direction].at(-1)!;
        const returned = await bridge.applyCommand(value);
        const receipt = geometryHistoryReceipt(returned, value, direction === 'undo' ? entry.from : entry.to);
        accept(returned);
        const saved = geometryTarget(scene(), receipt.regionId);
        if (scene()?.id !== sceneId || !saved || saved.revision !== receipt.revision || saved.backgroundId !== receipt.backgroundId || saved.sourceId !== receipt.sourceId || saved.geometry.x !== receipt.geometry.x || saved.geometry.y !== receipt.geometry.y || saved.geometry.width !== receipt.geometry.width || saved.geometry.height !== receipt.geometry.height) throw new Error('区域位置已变化');
        return receipt.regionId;
      });
      commands = next.then(() => {}, error => { if (exitPreparing()) exitFailure ??= error; });
      const regionId = await next;
      if (regionId && !disposed && scene()?.id === sceneId) focusReference({ kind: 'region', id: regionId });
    } catch (error) { showError(error); }
    finally { setBusy(false); finishOperation(); }
  }
  function drawingCommand(pluginId: string, pluginRevision: number, contributionId: string, value: DrawingCommand): Promise<void> {
    if (exitPreparing() && (!drawingFlushDepth || !['add_drawing', 'update_drawing'].includes(value.type))) return Promise.reject(new Error('正在退出'));
    if (scroll()) return Promise.reject(new Error('长截图进行中'));
    const next = commands.then(async () => { accept(await plugins.applyPluginDrawing(pluginId, pluginRevision, contributionId, value)); });
    commands = next.catch(error => { if (exitPreparing()) exitFailure ??= error; });
    return next;
  }
  function drawingDocumentCommand(value: DrawingCommand): Promise<void> {
    if (exitPreparing() && (!drawingFlushDepth || value.type !== 'update_drawing')) return Promise.reject(new Error('正在退出'));
    if (recording() || scroll()) return Promise.reject(new Error('当前无法编辑标注'));
    const next = commands.then(async () => {
      const current = scene(), region = current?.regions.find(region => region.id === value.regionId);
      if (!current || current.id !== value.sceneId || current.frozen || current.closed || current.background?.id !== value.backgroundId || !region || (region.drawingRevision ?? 0) !== value.expectedRevision || !usesDrawingDocument(region, value)) throw new Error('标注已更新');
      accept(await drawingLayouts.applyDrawingDocument(value));
    });
    commands = next.catch(error => { if (exitPreparing()) exitFailure ??= error; });
    return next;
  }
  async function copyDrawingTable(target: DrawingLayoutTarget, format: DrawingTableFormat): Promise<void> {
    if (exitPreparing() || recording() || scroll()) throw new Error('当前无法复制表格');
    const finish = spaceOperations.begin();
    try {
      const current = scene(), region = current?.regions.find(region => region.id === target.regionId);
      if (!current || current.id !== target.sceneId || current.frozen || current.closed || !region || (region.drawingRevision ?? 0) !== target.expectedRevision) throw new Error('表格已更新');
      await drawingLayouts.copyDrawingTable(target, format);
    } finally { finish(); }
  }
  async function flushVideo(active: () => boolean) {
    videoFlushDepth++;
    try {
      await videoFlush.flush(active);
      while (videoCommits.size) { await Promise.all([...videoCommits]); if (!active()) throw new TrimCanceled(); }
      assertVideoTextDraftsSaved(scene()?.id);
    } finally { videoFlushDepth--; }
  }
  function applyVideoEdit(grant: VideoGrant, target: VideoTarget, action: VideoAction): Promise<SpaceItem> {
    if (recording() || scroll()) return Promise.reject(new Error('当前无法裁剪视频'));
    const finish = spaceOperations.begin();
    const next = commands.then(async () => {
      const current = scene(), item = current?.items.find(value => value.id === target.itemId);
      if (!current || current.id !== target.sceneId || current.closed || current.frozen || !item || item.asset.id !== target.sourceId || (item.videoEdit?.revision ?? 0) !== target.expectedRevision || !sameRange(item.videoEdit?.range ?? null, action.from) || !validRecordingTrimGrant(grant)) throw new Error('视频范围或来源已更改');
      const operation = action.type === 'set' ? undefined : item.videoEdit?.[action.type].at(-1);
      if (action.type !== 'set' && operation?.id !== action.operationId) throw new Error('视频裁剪历史已变化');
      const expected = action.type === 'set' ? action.to : action.type === 'undo' ? operation!.from : operation!.to;
      const returned = await videoBridge.applyVideoEdit(grant, target, action);
      const result = returned.scenes.find(value => value.id === target.sceneId)?.items.find(value => value.id === target.itemId);
      if (!result?.videoEdit || JSON.stringify(result.asset) !== JSON.stringify(item.asset) || result.videoEdit.revision !== target.expectedRevision + 1 || !sameRange(result.videoEdit.range, expected)) throw new Error('视频范围回执不一致');
      accept(returned); return result;
    });
    commands = next.then(() => {}, error => { if (exitPreparing()) exitFailure ??= error; });
    videoCommits.add(next); void next.then(() => { videoCommits.delete(next); finish(); }, () => { videoCommits.delete(next); finish(); });
    return next;
  }
  function applyVideoAnnotation(target: VideoAnnotationTarget, action: VideoAnnotationAction, receive?: (snapshot: Snapshot) => void, authoringFlush = false): Promise<SpaceItem> {
    if ((exitPreparing() && (!(videoFlushDepth || authoringFlush) || action.type !== 'move')) || recording() || scroll()) return Promise.reject(new Error('当前无法编辑视频标注'));
    const finish = spaceOperations.begin();
    const next = commands.then(async () => {
      const current = scene(), item = current?.items.find(value => value.id === target.itemId), doc = item?.videoAnnotations;
      if ((exitPreparing() && (!(videoFlushDepth || authoringFlush) || action.type !== 'move')) || recording() || scroll() || !current || current.id !== target.sceneId || current.closed || current.frozen || current.run?.status === 'running' || !item || !doc || !sameVideoAnnotationTarget(videoAnnotationTarget(current.id, item), target)) throw new Error('视频标注已更新');
      if (action.type === 'move') {
        const object = doc.objects.find(value => value.id === action.annotationId), bounds = object && videoAnnotationBounds(object);
        if (!bounds || bounds.x !== action.fromTopLeft.x || bounds.y !== action.fromTopLeft.y || ![action.toTopLeft.x, action.toTopLeft.y].every(Number.isFinite)) throw new Error('视频标注位置已变化');
      } else if (action.type === 'remove' ? !doc.objects.some(value => value.id === action.annotationId) : doc[action.type].at(-1)?.id !== action.operationId) throw new Error('视频标注历史已变化');
      const returned = await videoAnnotationBridge.applyVideoAnnotationDocument(target, action);
      const result = returned.scenes.find(value => value.id === target.sceneId)?.items.find(value => value.id === target.itemId);
      if (!result?.videoAnnotations || result.videoAnnotations.revision !== target.expectedAnnotationRevision + 1 || JSON.stringify(result.asset) !== JSON.stringify(item.asset) || JSON.stringify(result.videoEdit) !== JSON.stringify(item.videoEdit)) throw new Error('视频标注回执不一致');
      if (action.type === 'move' && JSON.stringify(result.videoAnnotations.objects) !== JSON.stringify(doc.objects.map(value => value.id === action.annotationId ? movedVideoObject(value, action.toTopLeft) : value))) throw new Error('视频移动回执不一致');
      accept(returned); receive?.(returned); return result;
    });
    commands = next.then(() => {}, error => { if (exitPreparing()) exitFailure ??= error; });
    videoCommits.add(next); void next.then(() => { videoCommits.delete(next); finish(); }, () => { videoCommits.delete(next); finish(); });
    return next;
  }
  function applyVideoAnnotationText(requestId: string, target: VideoAnnotationTarget, annotationId: string, reference: VideoTextLayoutRef, content: VideoTextContent, receive?: (snapshot: Snapshot) => void, authoringFlush = false): Promise<SpaceItem> {
    if ((exitPreparing() && !(videoFlushDepth || authoringFlush)) || recording() || scroll()) return Promise.reject(new Error('当前无法编辑视频文字'));
    const finish = spaceOperations.begin();
    const next = commands.then(async () => {
      const current = scene(), item = current?.items.find(value => value.id === target.itemId), doc = item?.videoAnnotations, object = doc?.objects.find(value => value.id === annotationId);
      if ((exitPreparing() && !(videoFlushDepth || authoringFlush)) || recording() || scroll() || !current || current.id !== target.sceneId || current.closed || current.frozen || current.run?.status === 'running' || !item || !doc || !sameVideoAnnotationTarget(videoAnnotationTarget(current.id, item), target) || object?.primitive.kind !== 'text' || JSON.stringify(object.primitive.layout) !== JSON.stringify(reference)) throw new Error('视频文字已变化');
      const returned = await videoAnnotationBridge.editVideoAnnotationText(requestId, target, annotationId, reference, content);
      const result = returned.scenes.find(value => value.id === target.sceneId)?.items.find(value => value.id === target.itemId), updated = result?.videoAnnotations?.objects.find(value => value.id === annotationId);
      if (!result?.videoAnnotations || result.videoAnnotations.revision !== target.expectedAnnotationRevision + 1 || JSON.stringify(result.asset) !== JSON.stringify(item.asset) || JSON.stringify(result.videoEdit) !== JSON.stringify(item.videoEdit) || updated?.primitive.kind !== 'text' || JSON.stringify(updated.primitive.topLeft) !== JSON.stringify(object.primitive.topLeft) || JSON.stringify(updated.origin) !== JSON.stringify(object.origin) || JSON.stringify(updated.interval) !== JSON.stringify(object.interval) || result.videoAnnotations.objects.length !== doc.objects.length || JSON.stringify(result.videoAnnotations.objects.filter(value => value.id !== annotationId)) !== JSON.stringify(doc.objects.filter(value => value.id !== annotationId))) throw new Error('视频文字回执不一致');
      accept(returned); receive?.(returned); return result;
    });
    commands = next.then(() => {}, error => { if (exitPreparing()) exitFailure ??= error; });
    videoCommits.add(next); void next.then(() => { videoCommits.delete(next); finish(); }, () => { videoCommits.delete(next); finish(); });
    return next;
  }
  async function flushVideoDrawing(active: () => boolean, sceneId?: string) {
    videoDrawingFlushDepth++;
    try {
      await videoDrawingFlush.flush(active);
      while (videoDrawingCommits.size) {
        await Promise.all([...videoDrawingCommits]);
        if (!active()) throw new TrimCanceled();
      }
      if (!active()) throw new TrimCanceled();
      if (exitPreparing() && exitFailure !== undefined) throw exitFailure;
      assertVideoDrawingDraftsSaved(sceneId);
    } finally { videoDrawingFlushDepth--; }
  }
  function applyVideoDrawing(requestId: string, target: VideoAnnotationTarget, grant: VideoDrawingGrant, action: VideoDrawingAction): Promise<Snapshot> {
    const allowed = () => !disposed && (!exitPreparing() || videoDrawingFlushDepth > 0) && !recording() && !scroll();
    if (!allowed()) return Promise.reject(new Error('当前无法编辑视频绘制'));
    const finish = spaceOperations.begin();
    const next = commands.then(async () => {
      const current = scene(), item = current?.items.find(value => value.id === target.itemId);
      if (!allowed() || !current || current.id !== target.sceneId || current.closed || current.frozen || current.run?.status === 'running' || !item || item.asset.kind !== 'video' || !sameVideoAnnotationTarget(videoAnnotationTarget(current.id, item), target) || !validVideoDrawingGrant(grant) || !coreVideoDrawingTools.includes(action.content.kind)) throw new Error('视频绘制目标或来源已变化');
      const returned = await videoDrawingBridge.applyVideoDrawing(requestId, target, grant, action);
      acceptVideoDrawingReceipt(returned, target.sceneId, item, action);
      accept(returned); return returned;
    });
    commands = next.then(() => {}, error => { if (exitPreparing()) exitFailure ??= error; });
    videoDrawingCommits.add(next);
    void next.then(() => { videoDrawingCommits.delete(next); finish(); }, () => { videoDrawingCommits.delete(next); finish(); });
    return next;
  }
  async function applyVideoDrawingDocument(target: VideoAnnotationTarget, action: VideoAnnotationAction): Promise<Snapshot> {
    let receipt: Snapshot | undefined;
    await applyVideoAnnotation(target, action, value => { receipt = value; }, videoDrawingFlushDepth > 0);
    if (!receipt) throw new Error('视频标注回执缺失');
    return receipt;
  }
  async function applyVideoDrawingText(requestId: string, target: VideoAnnotationTarget, annotationId: string, reference: VideoTextLayoutRef, content: VideoTextContent): Promise<Snapshot> {
    let receipt: Snapshot | undefined;
    await applyVideoAnnotationText(requestId, target, annotationId, reference, content, value => { receipt = value; }, videoDrawingFlushDepth > 0);
    if (!receipt) throw new Error('视频文字回执缺失');
    return receipt;
  }
  async function jumpToVideoAnswer(action: VideoAnswerAction): Promise<void> {
    if (disposed || busy() || sending() || recording() || scroll() || exitPreparing()) return;
    const original = currentVideoAnswer(scene(), action); if (!original) return;
    const source = videoSourceIdentity(action.target.sceneId, original);
    const document = videoAnnotationIdentity(action.target.sceneId, original), pluginRevision = pluginSnapshot().revision;
    const active = () => {
      const item = currentVideoAnswer(scene(), action);
      return !disposed && !exitPreparing() && !recording() && !scroll() && !sending() && pluginSnapshot().revision === pluginRevision && !!item && videoSourceIdentity(action.target.sceneId, item) === source && videoAnnotationIdentity(action.target.sceneId, item) === document;
    };
    let prepared = false;
    const finish = spaceOperations.begin(); setBusy(true);
    try {
      await flush(); if (!active()) throw new TrimCanceled();
      focusReference({ kind: 'item', id: action.target.itemId });
      prepared = true;
    } catch (error) { if (!(error instanceof TrimCanceled)) showError(error); }
    finally { if (!disposed) setBusy(false); finish(); }
    // The busy=false effect pauses the media. Let that transition finish before
    // handing the user intent to the local player; playback itself is not busy.
    await new Promise<void>(resolve => queueMicrotask(resolve));
    if (!prepared || !active() || busy()) return;
    try { await videoNavigation.jump(action, source, () => active() && !busy()); }
    catch (error) { if (!(error instanceof TrimCanceled)) showError(error); }
  }
  async function outputVideoItem(itemId: string, kind: 'save' | 'copy'): Promise<boolean> {
    if (busy() || sending() || recording() || scroll() || exitPreparing()) return false;
    const selectedScene = scene(), selected = selectedScene?.items.find(item => item.id === itemId);
    if (!selectedScene || !selected || selected.asset.kind !== 'video') return false;
    const sceneId = selectedScene.id, source = JSON.stringify(selected.asset);
    const intent = { id: crypto.randomUUID(), canceled: false };
    const finish = spaceOperations.begin(); videoOutputIntent = intent; setBusy(true);
    setVideoExport({ requestId: intent.id, sceneId, itemId, kind, canceling: false });
    try {
      await flush(); await videoLane.cancelAll();
      const current = scene(), item = current?.items.find(value => value.id === itemId);
      if (intent.canceled || videoOutputIntent !== intent || !current || current.id !== sceneId || current.closed || current.frozen || !item || JSON.stringify(item.asset) !== source || exitPreparing()) throw new TrimCanceled();
      const target: VideoTarget = { sceneId, itemId, sourceId: item.asset.id, expectedRevision: item.videoEdit?.revision ?? 0 };
      const request = kind === 'copy' ? videoLane.request('copy', id => videoBridge.copyVideo(id, target), intent.id) : videoLane.request('export', id => videoBridge.exportVideo(id, target), intent.id);
      exportRequest = request;
      const receipt = await request.promise;
      return kind === 'copy' ? receipt === true : true;
    } catch (error) {
      if (!(error instanceof TrimCanceled)) { if (exitPreparing()) exitFailure ??= error; showError(error); }
      return false;
    } finally {
      if (videoOutputIntent === intent) { videoOutputIntent = undefined; exportRequest = undefined; setVideoExport(undefined); if (!disposed) setBusy(false); }
      finish();
    }
  }
  async function exportVideoItem(itemId: string) { await outputVideoItem(itemId, 'save'); }
  function copyVideoItem(itemId: string) { return outputVideoItem(itemId, 'copy'); }
  function cancelVideoExport() {
    if (!videoOutputIntent) return;
    videoOutputIntent.canceled = true;
    setVideoExport(old => old ? { ...old, canceling: true } : old);
    void exportRequest?.cancel().catch(showError);
  }
  async function runWorkflow(pluginId: string, pluginRevision: number, contributionId: string, regionId: string) {
    const current = scene(); if (!current || busy() || sending() || recording() || scroll() || exitPreparing()) return;
    const finishOperation = spaceOperations.begin();
    setSending(true);
    try {
      await flush();
      accept(await plugins.runPluginWorkflow(pluginId, pluginRevision, contributionId, current.id, regionId));
      setError('');
      setExpanded(old => ({ ...old, [current.id]: true }));
    } catch (error) { if (exitPreparing()) exitFailure ??= error; throw error; }
    finally { setSending(false); finishOperation(); }
  }
  function setDraft(text: string) {
    const id = scene()!.id;
    setDrafts(old => ({ ...old, [id]: text }));
    autosaveVersion++;
    clearTimeout(debounce);
    if (exitPreparing()) return;
    debounce = setTimeout(() => void command({ type: 'set_draft', sceneId: id, draft: text }).catch(showError), 350);
  }
  function toggleReference(reference: Reference) {
    if (exitPreparing() || busy()) return;
    const id = scene()!.id;
    const current = refs();
    const next = current.some(r => r.kind === reference.kind && r.id === reference.id) ? current.filter(r => r.kind !== reference.kind || r.id !== reference.id) : [...current, reference];
    setReferenceDrafts(old => ({ ...old, [id]: next }));
    void command({ type: 'set_refs', sceneId: id, refs: next }).catch(showError);
  }
  async function flush() {
    await voice.cancel();
    const sceneId = scene()?.id; if (!sceneId) return;
    await flushVideoDrawing(() => !disposed && scene()?.id === sceneId, sceneId);
    await flushVideo(() => !disposed && scene()?.id === sceneId);
    await flushDrawing(() => !disposed && scene()?.id === sceneId, sceneId);
    await regionGeometry.flush();
    const current = scene(); if (!current || current.id !== sceneId) throw new Error('会话已变化');
    const text = draft(), references = refs().map(reference => ({ ...reference }));
    autosaveVersion++; clearTimeout(debounce);
    await command({ type: 'set_draft', sceneId: current.id, draft: text });
    await command({ type: 'set_refs', sceneId: current.id, refs: references });
  }
  async function flushDrawing(active: () => boolean, sceneId?: string) {
    drawingFlushDepth++;
    try {
      await drawingFlush.flush(active);
      if (!active()) return;
      const remaining = listUnsavedDrawingDrafts(sceneId);
      if (remaining.length) {
        drawingDraftScope = sceneId;
        setPendingDrawingDrafts(remaining);
        throw new Error('还有未保存标注');
      }
    }
    finally { drawingFlushDepth--; }
  }
  function discardDrawingDraft(entry: PendingDrawingDraft) {
    discardUnsavedDrawingDraft(entry);
    setPendingDrawingDrafts(listUnsavedDrawingDrafts(drawingDraftScope));
  }
  async function flushForExit(active: () => boolean) {
    await codes.cancel();
    await voice.cancel();
    await videoLane.cancelAll();
    await flushVideo(active);
    assertVideoTextDraftsSaved();
    autosaveVersion++; clearTimeout(debounce);
    // Geometry itself is a tracked operation; drain it before the general barrier.
    await regionGeometry.flush();
    // Already-started send/capture/freeze operations can update local overlays after
    // their command queue entries settle; wait for those continuations first.
    await spaceOperations.drain(active);
    while (active()) {
      const pending = commands; await pending;
      if (pending === commands) break;
    }
    if (!active()) return;
    if (exitFailure !== undefined) throw exitFailure;
    await flushDrawing(active);
    if (!active()) return;
    accept(await bridge.getSnapshot());
    await persistSceneDrafts({ active, snapshot, drafts, references: referenceDrafts, failure: () => exitFailure, save: command });
  }
  function resumeDraftSave() {
    const version = ++autosaveVersion;
    const active = () => !disposed && !exitPreparing() && autosaveVersion === version;
    clearTimeout(debounce);
    debounce = setTimeout(() => { void (async () => {
      await spaceOperations.drain(active);
      await commands;
      const current = snapshot(); if (!active() || !current) return;
      // Queue this captured batch together, so a later Send flush remains after it.
      await Promise.all(pendingSceneDrafts(current, drafts(), referenceDrafts()).map(command));
    })().catch(showError); }, 350);
  }
  async function switchScene(value: SceneCommand) {
    if (busy() || recording() || scroll() || exitPreparing()) return;
    const finishOperation = spaceOperations.begin();
    setBusy(true);
    try { await flush(); await command(value); setError(''); setSessionsOpen(false); setSelectionActive(false); setFocusRequest(v => v + 1); }
    catch (error) { showError(error); }
    finally { setBusy(false); finishOperation(); }
  }
  async function newConversation() {
    const current = scene();
    if (!current || busy() || sending() || recording() || scroll() || exitPreparing() || current.run?.status === 'running') return;
    const sceneId = current.id;
    const oldDraft = draft();
    const finishOperation = spaceOperations.begin();
    setBusy(true);
    try {
      await flush();
      if (scene()?.id !== sceneId || disposed || exitPreparing()) throw new Error('会话已变化');
      await command({ type: 'new_conversation', sceneId });
      if (scene()?.id !== sceneId || disposed) return;
      if (draft() === oldDraft) setDrafts(old => withoutScene(old, sceneId));
      setReferenceDrafts(old => withoutScene(old, sceneId));
      setStreams(old => withoutScene(old, sceneId));
      setSendErrors(old => withoutScene(old, sceneId));
      setExpanded(old => ({ ...old, [sceneId]: false }));
      setFocusRequest(value => value + 1);
    } catch (error) { showError(error); }
    finally { setBusy(false); finishOperation(); }
  }
  async function freeze() {
    if (busy() || recording() || scroll() || exitPreparing()) return;
    const current = scene(); if (!current) return;
    const finishOperation = spaceOperations.begin();
    setBusy(true);
    try { await flush(); accept(await bridge.freezeSpace(current.id)); }
    catch (error) { showError(error); }
    finally { setBusy(false); finishOperation(); }
  }
  async function closeScene(sceneId: string): Promise<boolean> {
    if (busy() || recording() || scroll() || exitPreparing() || snapshot()?.scenes.find(scene => scene.id === sceneId)?.closed) return false;
    const finishOperation = spaceOperations.begin();
    const closingCurrent = scene()?.id === sceneId;
    setBusy(true); setClosingScene(sceneId);
    try {
      await flush();
      accept(await bridge.closeSceneWindow(sceneId));
      clearSceneState(sceneId);
      setError('');
      if (closingCurrent) { setSessionsOpen(false); setSelectionActive(false); }
      return true;
    } catch (error) { showError(error); return false; }
    finally { setClosingScene(undefined); setBusy(false); finishOperation(); }
  }
  async function addRegion(sceneId: string, region: Region) {
    if (exitPreparing()) throw new Error('正在退出');
    const finishOperation = spaceOperations.begin();
    try {
      await regionGeometry.flush();
      await command({ type: 'add_region', sceneId, region });
      const current = snapshot()!.scenes.find(s => s.id === sceneId)!;
      const next = [...(referenceDrafts()[sceneId] ?? current.refs), { kind: 'region' as const, id: region.id }];
      setReferenceDrafts(old => ({ ...old, [sceneId]: next }));
      await command({ type: 'set_refs', sceneId, refs: next });
    } finally { finishOperation(); }
  }
  function focusReference(reference: Reference) {
    document.dispatchEvent(new CustomEvent('mewu:focus-reference', { detail: reference }));
  }
  async function capture() {
    if (busy() || recording() || scroll() || exitPreparing()) return; setBusy(true);
    const finishOperation = spaceOperations.begin();
    try { await flush(); accept(await bridge.captureScreen()); }
    catch (error) { showError(error); }
    finally { setBusy(false); finishOperation(); }
  }
  async function openBlackboard() {
    const current=scene();
    if(!current||busy()||sending()||recording()||scroll()||exitPreparing()||current.run?.status==='running')return;
    if(isBlackboard(current)&&blackboardEditing()!==current.id){setBlackboardEditing(current.id);return;}
    const finish=spaceOperations.begin();setBusy(true);
    try{
      if(isBlackboard(current)){
        await flushDrawing(()=>!disposed&&scene()?.id===current.id,current.id);
        if(!disposed&&scene()?.id===current.id){const next=await bridge.finishBlackboard(current.id);setReferenceDrafts(old=>withoutScene(old,next.activeSceneId));accept(next);setBlackboardEditing(undefined);}
        return;
      }
      await flush();
      if(scene()?.id!==current.id||disposed||exitPreparing())throw Error('会话已变化');
      const next=await bridge.createBlackboard(current.id);accept(next);
      if(!disposed&&isBlackboard(scene()))setBlackboardEditing(next.activeSceneId);
    }catch(error){showError(error);}finally{setBusy(false);finish();}
  }
  async function finishBlackboard():Promise<boolean>{
    const current=scene();if(!current||!isBlackboard(current)||busy()||sending()||exitPreparing())return false;
    const finish=spaceOperations.begin();setBusy(true);
    try{
      await flushDrawing(()=>!disposed&&scene()?.id===current.id,current.id);
      if(disposed||scene()?.id!==current.id||exitPreparing())throw Error('会话已变化');
      const next=await bridge.finishBlackboard(current.id);setReferenceDrafts(old=>withoutScene(old,next.activeSceneId));accept(next);setBlackboardEditing(undefined);setFocusRequest(value=>value+1);return true;
    }catch(error){showError(error);return false;}finally{setBusy(false);finish();}
  }
  async function reopenBlackboard(itemId:string){
    const current=scene();if(!current||busy()||sending()||recording()||scroll()||exitPreparing()||current.run?.status==='running')return;
    const finish=spaceOperations.begin();setBusy(true);
    try{await flush();if(disposed||scene()?.id!==current.id||exitPreparing())throw Error('会话已变化');const next=await bridge.openBlackboard(current.id,itemId);accept(next);setBlackboardEditing(next.activeSceneId);}
    catch(error){showError(error);}finally{setBusy(false);finish();}
  }
  const canReplaySpace=(redo:boolean)=>{
    const controls=drawingHistoryControls();if(controls)return redo?controls.canRedo():controls.canUndo();
    if(selectionActive()||exportActive())return false;
    const current=scene();if(isBlackboard(current))return Boolean(current?.regions[0]?.drawingHistory?.[redo?'redo':'undo'].length);
    return redo?!geometryView().pending&&Boolean(current?.geometryHistory?.redo.length):Boolean(current?.geometryHistory?.undo.length||geometryView().pending);
  };
  function replaySpace(redo:boolean){
    const controls=drawingHistoryControls();if(controls){if(redo)controls.redo();else controls.undo();return;}
    const current=scene();
    if(isBlackboard(current)&&current?.background&&current.regions[0]){
      const region=current.regions[0];
      void drawingCommand(coreDrawingGrant.pluginId,coreDrawingGrant.revision,coreDrawingGrant.contributionId,{type:redo?'redo_drawing':'undo_drawing',sceneId:current.id,backgroundId:current.background.id,regionId:region.id,expectedRevision:region.drawingRevision??0}).catch(showError);
    }else void replayGeometry(redo?'redo':'undo');
  }
  function cancelPendingPin() { const value = pendingPin; if (value && !value.canceled) { value.canceled = true; void pinBridge.cancelPin(value.requestId).catch(() => {}); } }
  async function createPin(pluginId: string, revision: number, contributionId: string, regionId: string): Promise<boolean> {
    const current = scene();
    if (!current || busy() || recording() || scroll() || exitPreparing()) return false;
    const request = { requestId: crypto.randomUUID(), canceled: false }; pendingPin = request;
    const finishOperation = spaceOperations.begin(); setBusy(true);
    try {
      const created = await preparePin({ prepare: flush, target: () => {
        const saved = scene(), region = saved?.regions.find(value => value.id === regionId);
        if (request.canceled || disposed || exitPreparing() || saved?.id !== current.id || !saved?.background || saved.closed || saved.frozen || !region) return;
        const target = { sceneId: saved.id, backgroundId: saved.background.id, regionId, drawingRevision: region.drawingRevision ?? 0, x: region.x, y: region.y, width: region.width, height: region.height };
        return { requestId: request.requestId, pluginId, revision, contributionId, target, translationId: region.translation?.overlay.id ?? null };
      }, run: pinBridge.runPin });
      return created && !request.canceled && !disposed;
    } catch (error) { if (!request.canceled && !disposed) { if (exitPreparing()) exitFailure ??= error; throw error; } return false; }
    finally { if (pendingPin === request) pendingPin = undefined; if (!disposed) setBusy(false); finishOperation(); }
  }
  async function importFiles() {
    const current = scene();
    if (!current || busy() || recording() || scroll() || exitPreparing()) return; setBusy(true);
    const finishOperation = spaceOperations.begin();
    try {
      await flush();
      if (disposed || exitPreparing() || scene()?.id !== current.id || scene()?.closed || scene()?.frozen) return;
      const result = await bridge.importAssets(current.id);
      if (!disposed && scene()?.id === current.id) { clearReferenceDraft(current.id); accept(result); setFocusRequest(value => value + 1); }
    }
    catch (error) { showError(error); }
    finally { if (!disposed) setBusy(false); finishOperation(); }
  }
  async function startRecording(regionId: string) {
    const current = scene();
    if (!current || busy() || recording() || scroll() || exitPreparing() || current.run?.status === 'running') return;
    const grant = recordingGrant();
    if (!bridge.native) { showError('仅桌面版可录屏'); return; }
    if (!recordingReady()) { showError('录屏状态尚未就绪'); return; }
    if (!current.background || !current.regions.some(region => region.id === regionId && !region.imageOverride)) return;
    setBusy(true);
    const finishOperation = spaceOperations.begin();
    try {
      videoPlayers.pauseAll();
      const audio = await recordingAudio.prepare();
      await flush();
      if (disposed || exitPreparing() || scene()?.id !== current.id || scene()?.background?.id !== current.background.id || scene()?.closed || scene()?.frozen) throw new Error('录屏选区已变化');
      if (!sameAudioGrant(grant, recordingGrant())) throw new Error('屏幕录制已变化，请重试');
      if (!sameAudioSelection(audio, recordingAudio.selection())) throw new Error('声音设置已变化，请重新录屏');
      await bridge.startRecording(current.id, regionId, audio, grant);
      clearReferenceDraft(current.id);
    } catch (error) { showError(error); }
    finally { setBusy(false); finishOperation(); }
  }
  async function send() {
    const current = scene();
    if (!current || busy() || recording() || scroll() || sending() || exitPreparing() || current.run?.status === 'running' || (!draft().trim() && refs().length === 0)) return;
    const candidateGrant = visualAnnotationGrant();
    let grant: VisualAnnotationGrant | undefined;
    const identity = annotationSendIdentity(current);
    let sourceIdentity: string | undefined;
    const check = (text: string, references: Reference[]) => {
      if (disposed || exitPreparing() || scene()?.id !== current.id || scene()?.closed || scene()?.frozen) throw new Error('会话已变化');
      if (grant) assertAnnotationSend({ identity, sourceIdentity, grant, scene: scene(), references, draft: text, plugins: pluginSnapshot().plugins, active: !recording() && !scroll() });
    };
    const finishOperation = spaceOperations.begin();
    const stopVoice = voice.cancel();
    const id = current.id;
    setSending(true); setSendErrors(old => ({ ...old, [id]: '' }));
    try {
      await stopVoice;
      await flushVideo(() => !disposed && scene()?.id === id);
      await flushDrawing(() => !disposed && scene()?.id === id, id);
      await regionGeometry.flush();
      if (scene()?.id !== id) throw new Error('会话已变化');
      const submittedDraft = draft(), submittedRefs = refs().map(reference => ({ ...reference }));
      // Enable available tools for the committed references; the model chooses
      // whether to use them. Keep the revision captured before asynchronous work.
      grant = hasAnnotationTarget(scene(), submittedRefs) ? candidateGrant : undefined;
      // Capture committed source pixels/range/document only after our own flush.
      if (grant) sourceIdentity = annotationSendIdentity(scene()!, submittedRefs);
      check(submittedDraft, submittedRefs);
      autosaveVersion++; clearTimeout(debounce);
      // One queue entry: a later autosave belongs after begin_run, never between
      // this payload's draft/refs and the native read that creates its message.
      const submission = commands.then(() => submitSceneDraft({ sceneId: id, draft: submittedDraft, refs: submittedRefs }, {
        save: async value => { check(submittedDraft, submittedRefs); accept(await bridge.applyCommand(value)); },
        send: sceneId => { check(submittedDraft, submittedRefs); return bridge.sendMessage(sceneId, grant); },
      }));
      commands = submission.then(() => {}, error => { if (exitPreparing()) exitFailure ??= error; });
      const next = await submission;
      accept(next);
      if (snapshot()?.scenes.find(scene => scene.id === id)?.closed) return;
      setDrafts(old => old[id] !== undefined && old[id] !== submittedDraft ? old : ({ ...old, [id]: next.scenes.find(s => s.id === id)?.draft || '' }));
    } catch (error) { if (exitPreparing()) exitFailure ??= error; if (!snapshot()?.scenes.find(scene => scene.id === id)?.closed) setSendErrors(old => ({ ...old, [id]: error instanceof Error ? error.message : String(error) })); }
    finally { setSending(false); finishOperation(); }
  }
  async function startScroll(pluginId: string, revision: number, contributionId: string, target: OcrTarget) {
    const current = scene();
    if (!current || current.id !== target.sceneId || busy() || recording() || scroll() || sending() || exitPreparing() || current.run?.status === 'running') return;
    if (!bridge.native) throw new Error('请在桌面版使用长截图');
    if (!scrollReady()) throw new Error('长截图状态尚未就绪');
    if (current.regions.find(region => region.id === target.regionId)?.imageOverride) return;
    const finishOperation = spaceOperations.begin(); setBusy(true);
    try {
      await flush();
      await scrollBridge.startScrollCapture(pluginId, revision, contributionId, target);
      clearReferenceDraft(current.id);
    } catch (error) { if (exitPreparing()) exitFailure ??= error; throw error; }
    finally { setBusy(false); finishOperation(); }
  }
  async function cancel() {
    if (!scene()) return;
    try { accept(await bridge.cancelRun(scene()!.id)); }
    catch (error) { showError(error); }
  }
  async function continueJournal(summary: RunJournalSummary, decision: ContinuationDecision, selected?: string[]): Promise<boolean> {
    const current = scene(), connection = snapshot()?.connections.find(value => value.id === current?.connectionId);
    if (!current || current.id !== summary.sceneId || !connection || disposed || busy() || sending() || recording() || scroll() || exitPreparing() || current.closed || current.frozen || current.run?.status === 'running') return false;
    const id = current.id;
    const controller = new ContinuationController<Snapshot>({
      current: () => {
        const value = scene(), model = snapshot()?.connections.find(entry => entry.id === value?.connectionId);
        if (!value || !model) return;
        return { sceneId: value.id, agentId: value.agentId, viewedRunId: summary.runId, sourceRunId: decision.sourceRunId, currentRunId: value.run?.id ?? null,
          connectionId: model.id, connectionRevision: model.revision, journalRevision: decision.journalRevision, checkpointSeq: decision.checkpointSeq };
      },
      allowed: () => !disposed && !exitPreparing() && !recording() && !scroll() && !scene()?.closed && !scene()?.frozen,
      beginOperation: () => spaceOperations.begin(), flush,
      enqueue: operation => {
        const next = commands.then(operation);
        commands = next.then(() => {}, error => { if (exitPreparing()) exitFailure ??= error; });
        return next;
      },
      start: bridge.continueRunFromJournal, accept,
      pending: value => { setSending(value); setBusy(value); },
      error: value => { if (exitPreparing()) exitFailure ??= value; setSendErrors(old => ({ ...old, [id]: value })); },
    });
    setSendErrors(old => ({ ...old, [id]: '' }));
    return controller.continue(summary, decision, selected);
  }
  async function saveAgent(agent: AgentProfile) { await command({ type: 'save_agent', agent }); }
  async function saveConnectionProfile(profile: ConnectionProfile, expectedRevision?: number, apiKey?: string) { await commands; accept(await bridge.saveConnectionProfile(profile, expectedRevision, apiKey)); }
  async function deleteConnectionProfile(id: string, expectedRevision: number) { await commands; accept(await bridge.deleteConnectionProfile(id, expectedRevision)); }
  async function discoverMcpServer(value: DiscoverMcpServer) { await commands; accept(await bridge.discoverMcpServer(value)); }
  async function openConnection() {
    if (!bridge.native) { setSettings('agent'); return; }
    try { await bridge.openSettings(); } catch (error) { showError(error); }
  }
  function changePreferences(value: Preferences) {
    document.documentElement.dataset.buttonLabels = value.showButtonLabels === false ? 'hide' : 'show';
    if (value.translationLanguage !== preferences().translationLanguage) translationRequests.cancelAll();
    updateLanguage(value.uiLanguage);
    void syncNativeLanguage(value.uiLanguage).catch(showError);
    setPreferences(value);
    try { localStorage.setItem(preferenceKey, JSON.stringify(value)); } catch { /* The current session remains usable when storage is disabled. */ }
  }
  async function removeReferenceObject(kind: 'region' | 'item', id: string) {
    if (exitPreparing() || busy()) return;
    const sceneId = scene()!.id;
    const finishOperation = spaceOperations.begin(); setBusy(true);
    try {
      await flush();
      await command(kind === 'region' ? { type: 'remove_region', sceneId, regionId: id } : { type: 'remove_item', sceneId, itemId: id });
      setReferenceDrafts(old => ({ ...old, [sceneId]: (old[sceneId] ?? refs()).filter(ref => !(ref.kind === kind && ref.id === id)) }));
    } catch (error) { showError(error); }
    finally { setBusy(false); finishOperation(); }
  }
  const escape = (event: KeyboardEvent) => {
    if (nativeSelectOwnsEscape(event)) return;
    if (exitPreparing()) return;
    if (event.key !== 'Escape' || settings() || pendingDrawingDrafts().length) return;
    const activeScroll = scroll();
    if (activeScroll) {
      event.preventDefault();
      void scrollBridge.controlScrollCapture(activeScroll.id, 'cancel').catch(showError);
      return;
    }
    const activeRecording = recording();
    if (activeRecording) {
      event.preventDefault();
      if (activeRecording.phase !== 'stopping') void bridge.controlRecording(activeRecording.id, activeRecording.phase === 'countdown' || activeRecording.phase === 'starting' ? 'cancel' : 'stop').catch(showError);
      return;
    }
    if (busy()) return;
    if (sessionsOpen()) { setSessionsOpen(false); return; }
    if (scene() && expanded()[scene()!.id]) setExpanded(old => ({ ...old, [scene()!.id]: false }));
    else if (scene()) void closeScene(scene()!.id);
  };
  const pinEscape = (event: KeyboardEvent) => {
    if ((event.target as Element)?.closest?.('.video-annotation-text-editor')) return;
    if (nativeSelectOwnsEscape(event)) return;
    if (voicePending() && event.key === 'Escape' && !event.isComposing && event.keyCode !== 229 && !document.querySelector('.voice-language-menu,.send-action-menu')) {
      event.preventDefault(); event.stopImmediatePropagation(); void voice.cancel().catch(showError); return;
    }
    cancelPinOnEscape(event, Boolean(pendingPin), cancelPendingPin);
  };
  const videoExportEscape = (event: KeyboardEvent) => {
    if ((event.target as Element)?.closest?.('.video-annotation-text-editor')) return;
    if (nativeSelectOwnsEscape(event)) return;
    if (event.key !== 'Escape' || event.isComposing || event.keyCode === 229 || exitPreparing() || !videoExport()) return;
    event.preventDefault(); event.stopImmediatePropagation(); cancelVideoExport();
  };
  // Register before rendering child listeners: a pending pin must cancel even
  // while DrawingEditor owns Escape and App is otherwise busy.
  // An active export also owns Escape before video/trim target handlers consume it.
  document.addEventListener('keydown', videoExportEscape, true);
  document.addEventListener('keydown', pinEscape, true);
  document.addEventListener('keydown', escape);
  onCleanup(() => { document.removeEventListener('keydown', videoExportEscape, true); document.removeEventListener('keydown', pinEscape, true); document.removeEventListener('keydown', escape); });

  return <div class="app" inert={exitPreparing()} classList={{ 'recording-active': Boolean(recording()), 'scroll-active': Boolean(scroll()), 'reduce-motion': preferences().reduceMotion, 'text-small': preferences().textSize === 'small', 'text-large': preferences().textSize === 'large' }}>
    <Show when={!recording() && !scroll()} fallback={<><Show when={recording()}>{status => <RecordingBackdrop status={status()} maskOpacity={preferences().maskOpacity} />}</Show><Show when={scroll()}>{status => <ScrollBackdrop status={status()} maskOpacity={preferences().maskOpacity} />}</Show></>}>
    <Show when={scene()?.id} keyed fallback={<div class="startup-state"><Show when={!error()} fallback={<><CircleAlert size={19} /><span>{error()}</span><button onClick={() => location.reload()}>{t("重新载入")}</button></>}><LoaderCircle size={20} class="spin" /></Show></div>}>
      {currentId => <>
        <SpaceCanvas onBlackboardReplay={replaySpace} blackboard={isBlackboard(scene())} blackboardDrawing={blackboardEditing()===currentId} onBlackboardClose={finishBlackboard} onOpenBlackboard={id=>void reopenBlackboard(id)} onRegisterDrawingHistory={registerDrawingHistory} onRasterSnapshot={accept} showButtonLabels={preferences().showButtonLabels} translationLanguage={preferences().translationLanguage} video={{ lane: videoLane, onRegisterPause: videoPlayers.register, onRegisterPlayer: videoNavigation.register, onPauseAll: videoPlayers.pauseAll.bind(videoPlayers), onAnnotationEdit: applyVideoAnnotation, onAnnotationTextEdit: applyVideoAnnotationText, drawing: { onApplyDrawing: applyVideoDrawing, onDocument: applyVideoDrawingDocument, onEditText: applyVideoDrawingText, onSnapshot: accept, onReadVector: videoDrawingBridge.getVideoAnnotationVector, onReadText: videoAnnotationBridge.getVideoAnnotationText, onRegisterDrawingAuthoringFlush: videoDrawingFlush.register }, onRegister: videoFlush.register, onEdit: applyVideoEdit, exporting: videoExport(), onCancelExport: cancelVideoExport, onError: showError }} onVideoExport={exportVideoItem} onVideoCopy={copyVideoItem} onCodeSource={acceptCodeSource} codeView={codeView()} codeActionBusy={codeActionBusy()} onCodeAction={codeAction} onRegisterDrawingFlush={drawingFlush.register} onExportActivity={setExportActive} onGeometryReplay={replayGeometry} onPin={createPin} onTranslationRemove={removeTranslation} translationPending={translations().find(value => value.request.target.sceneId === currentId)} translationNotice={translationNotices()[currentId]} onTranslationRequest={startTranslation} onTranslationCancel={() => translationRequests.cancelScene(currentId)} onClearTranslationNotice={() => setTranslationNotices(old => withoutScene(old, currentId))} onScrollCapture={startScroll} plugins={pluginSnapshot().plugins} onPluginWorkflow={runWorkflow} onDrawingCommand={drawingCommand} onDrawingDocumentCommand={drawingDocumentCommand} onCopyDrawingTable={copyDrawingTable} onOcrRequest={request => plugins.runPluginOcr(request.requestId, request.pluginId, request.revision, request.contributionId, request.target)} onOcrCancel={plugins.cancelPluginOcr} onOcrResult={accept} scene={canvasScene()} refs={refs()} maskOpacity={preferences().maskOpacity} authoringBusy={busy() || sending()} inputLocked={exitPreparing()} busy={busy() || sending() || exitPreparing()} geometryController={regionGeometry} geometryView={geometryView()} onBeforeExport={flush} onCopyClose={closeScene} onRecordRegion={id => void startRecording(id)} onAddRegion={region => addRegion(currentId, region)} onSelectionActivity={setSelectionActive} onFocusComposer={() => setFocusRequest(v => v + 1)} onSelectionComplete={() => setFocusRequest(v => v + 1)} onRemoveRegion={id => removeReferenceObject('region', id)} onReference={toggleReference} onUpdateItem={item => command({ type: 'update_item', sceneId: currentId, item })} onRemoveItem={id => removeReferenceObject('item', id)} onError={showError} />
        <Show when={blackboardEditing()!==currentId}><PinObjectLayer objects={pinObjects()} busy={busy()||sending()||exitPreparing()} referenced={pinReferenced} onReference={object=>void referencePin(object)} onError={showError}/></Show>
        <Show when={blackboardEditing()!==currentId}><nav class="space-utilities" aria-label={t('空间操作')}>
          <button class="icon-button" aria-label={t('退出')} title={t('退出')} disabled={busy()||exitPreparing()} onClick={()=>void closeScene(currentId)}><X size={17}/></button>
          <button class="icon-button" aria-label={t('新建对话')} title={t('新建对话')} disabled={busy()||sending()||exitPreparing()||scene()?.run?.status==='running'} onClick={()=>void newConversation()}><SquarePen size={17}/></button>
          <button class="icon-button" classList={{selected:isBlackboard(scene())&&blackboardEditing()===currentId}} aria-label={t('绘制')} title={t('绘制')} aria-pressed={isBlackboard(scene())&&blackboardEditing()===currentId} disabled={busy()||sending()||exitPreparing()||scene()?.run?.status==='running'} onClick={()=>void openBlackboard()}><Pencil size={17}/></button>
          <span class="space-utility-divider"/>
          <button class="icon-button" aria-label={t('撤销')} title={t('撤销 · Ctrl+Z')} disabled={busy()||sending()||exitPreparing()||scene()?.run?.status==='running'||!canReplaySpace(false)} onClick={()=>replaySpace(false)}><Undo2 size={17}/></button>
          <button class="icon-button" aria-label={t('重做')} title={t('重做 · Ctrl+Shift+Z')} disabled={busy()||sending()||exitPreparing()||scene()?.run?.status==='running'||!canReplaySpace(true)} onClick={()=>replaySpace(true)}><Redo2 size={17}/></button>
        </nav>
        <Composer showButtonLabels={preferences().showButtonLabels} thinkingGlowEnabled={preferences().thinkingGlowEnabled} thinkingGlowColor={preferences().thinkingGlowColor} onVideoAnswer={action => void jumpToVideoAnswer(action)} onContinueJournal={continueJournal} voice={voiceControl()} onComposition={active => { if (scene()?.id !== currentId) return; voiceComposing = active; if (!active) voice.compositionEnded(); }} scene={scene()!} agents={snapshot()!.agents} connections={snapshot()!.connections} draft={draft()} refs={refs()} stream={streams()[currentId]} error={sendErrors()[currentId]} expanded={expanded()[currentId] ?? false} position={composerPositions()[currentId]} onPosition={value => setComposerPositions(old => ({ ...old, [currentId]: value }))} selectionActive={selectionActive()} focusRequest={focusRequest()} sending={sending()} closing={Boolean(closingScene())} onDraft={setDraft} onSend={() => void send()} onCancel={() => void cancel()} onImport={() => void importFiles()} onFreeze={() => void freeze()} onClose={() => void closeScene(currentId)} onNew={() => void newConversation()} onCapture={() => void capture()} onSessions={() => setSessionsOpen(true)} onFocusRef={focusReference} onRemoveRef={toggleReference} onExpanded={value => setExpanded(old => ({ ...old, [currentId]: value }))} onAgent={agentId => void command({ type: 'set_agent', sceneId: currentId, agentId }).catch(showError)} onSelectConnection={connectionId => void command({ type: 'set_scene_connection', sceneId: currentId, connectionId }).catch(showError)} onConnection={() => void openConnection()} onError={showError} /></Show>
        <Show when={sessionsOpen()}><SessionDock scenes={snapshot()!.scenes.filter(scene=>!scene.blackboardLink)} activeId={snapshot()!.activeSceneId} busy={busy()} onClose={() => setSessionsOpen(false)} onCloseScene={id => void closeScene(id)} onActivate={id => { if (id !== scene()!.id) void switchScene({ type: 'activate_scene', sceneId: id }); else setSessionsOpen(false); }} /></Show>
      </>}
    </Show>
    <Show when={settings() && snapshot()}><div class="settings-surface settings-surface-overlay"><SettingsDialog snapshot={snapshot()!} initialTab={settings()!} preferences={preferences()} hotkey={hotkey()} onPreferences={changePreferences} onSaveAgent={saveAgent} onSaveMemory={value => command({ type: 'save_memory', ...value })} onDeleteMemory={value => command({ type: 'delete_memory', ...value })} onSaveConnectionProfile={saveConnectionProfile} onDeleteConnectionProfile={deleteConnectionProfile} onSetDefaultConnection={connectionId => command({ type: 'set_default_connection', connectionId })} onDiscoverMcpServer={discoverMcpServer} onMcpCommand={command} onClose={() => setSettings(undefined)} /></div></Show>
    <Show when={!exitPreparing() && pendingDrawingDrafts().length}><DrawingDraftsDialog entries={pendingDrawingDrafts()} onDiscard={discardDrawingDraft} onClose={() => setPendingDrawingDrafts([])} /></Show>
    <Show when={error() && snapshot()}><div class="toast" role="alert"><CircleAlert size={15} /><span>{error()}</span><button class="icon-button compact" aria-label={t("关闭提示")} onClick={() => setError('')}><X size={13} /></button></div></Show>
    </Show>
  </div>;
}
