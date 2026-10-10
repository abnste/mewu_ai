// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Asset, ConnectionProbe, ConnectionProfile, ConnectionTestResult, DiscoverMcpServer, DrawingCommand, DrawingEdit, MemoryEntry, MemoryPage, ProviderPreset, RunEvent, Scene, SceneCommand, Snapshot, VisualAnnotationGrant } from './contracts';
import { normalizeConnectionPath, validateConnectionProfile } from './connection-policy';
import type { ExitPreparationCanceled, ExitPreparationRequest, ExitPreparationResult } from './exit-preparation';
import { browserMosaicPreview, MosaicPreviewCache, type MosaicPreview } from './mosaic-preview';
import { emptyGeometryHistory, PreviewGeometryHistory } from './region-geometry-history';
import { IMPORT_ACCEPT, prepareImportFiles } from './asset-import';
import type { RecordingAudioMode, RecordingAudioSelection, RecordingGrant } from './recording-audio';
import type { PluginManifest } from './plugin-contracts';
import { bundledProviderPresets } from './provider-presets';

export const native = isTauri();
export async function subscribeExitPreparation(prepare: (request: ExitPreparationRequest) => void, cancel: (request: ExitPreparationCanceled) => void): Promise<() => void> {
  if (!native) return () => undefined;
  let stopped = false;
  const valid = (value: unknown): value is ExitPreparationRequest => Boolean(value && typeof value === 'object' && 'requestId' in value && typeof value.requestId === 'string' && value.requestId.length > 0);
  const stopCancel = await listen<ExitPreparationCanceled>('exit-preparation-canceled', event => { if (!stopped && valid(event.payload)) cancel(event.payload); }, { target: 'space' });
  try {
    const stopPrepare = await listen<ExitPreparationRequest>('prepare-exit', event => { if (!stopped && valid(event.payload)) prepare(event.payload); }, { target: 'space' });
    return () => { stopped = true; stopPrepare(); stopCancel(); };
  } catch (error) { stopped = true; stopCancel(); throw error; }
}
export async function finishExitPreparation(result: ExitPreparationResult): Promise<void> {
  if (!native) throw new Error('此操作需要桌面应用');
  await invoke('finish_exit_preparation', { requestId: result.requestId, success: result.success, error: result.error });
}
export async function beginExitPreparation(request: ExitPreparationRequest): Promise<void> {
  if (!native) throw new Error('此操作需要桌面应用');
  if (!request.requestId) throw new Error('退出请求无效');
  await invoke('begin_exit_preparation', { requestId: request.requestId });
}
export interface RecordingStatus {
  id: string;
  sceneId: string;
  phase: 'countdown' | 'starting' | 'recording' | 'paused' | 'stopping';
  countdown: number;
  elapsedMs: number;
  rect: { x: number; y: number; width: number; height: number };
  stopHotkey: string;
  audio: RecordingAudioMode;
}
export type RecordingAction = 'pause' | 'resume' | 'stop' | 'cancel';
interface RuntimeInfo { hotkey: string; captureBackend: string; contentOrigin: string }
let runtime: RuntimeInfo | undefined;
let runtimeRequest: Promise<RuntimeInfo> | undefined;
const uid = () => crypto.randomUUID();
const emptyScene = (agentId: string): Scene => ({
  id: uid(), title: '新会话', agentId, createdAt: Date.now(), updatedAt: Date.now(),
  frozen: false, closed: false, connectionId: null, regions: [], items: [], refs: [], draft: '', messages: [],
  geometryHistory: emptyGeometryHistory(),
});
const firstScene = emptyScene('mewu');
const previewGeometry = new PreviewGeometryHistory();
let preview: Snapshot = {
  schemaVersion: 1, activeSceneId: firstScene.id, scenes: [firstScene],
  agents: [{ id: 'mewu', name: 'Mewu', instructions: '', memory: '', memoryEnabled: true, defaultConnectionId: null }], memories: [], memoryStats: [], mcpServers: [],
  connection: { baseUrl: '', model: '', hasKey: false }, connections: [], defaultConnectionId: null,
};
const providerPackages = import.meta.glob('../plugins/official-provider-*.json', { eager: true, import: 'default' }) as Record<string, PluginManifest>;
const previewKeys = new Map<string, string>();
function previewConnectionFor(agentId: string) { return preview.agents.find(agent => agent.id === agentId)?.defaultConnectionId ?? preview.defaultConnectionId; }
function refreshPreviewConnection() {
  const profile = preview.connections.find(value => value.id === preview.defaultConnectionId);
  preview.connection = profile ? { baseUrl: profile.baseUrl, model: profile.model, hasKey: profile.hasKey } : { baseUrl: '', model: '', hasKey: false };
}
const artifactText = new Map<string, string>();
const previewUrls = new Set<string>();
const snapshotListeners = new Set<(value: Snapshot) => void>();
const previewMemories: MemoryEntry[] = [];
const previewMemoryRevisions = new Map<string, number>();
const clone = (): Snapshot => structuredClone({ ...preview, agents: preview.agents.map(agent => ({ ...agent, memoryProvider: agent.memoryProvider ?? { revision: 0, bindingId: null } })), scenes: preview.scenes.map(scene => ({ ...scene, geometryHistory: scene.geometryHistory ?? emptyGeometryHistory() })), memories: [], memoryStats: preview.agents.map(agent => ({ agentId: agent.id, count: previewMemories.filter(entry => entry.agentId === agent.id).length, revision: previewMemoryRevisions.get(agent.id) ?? 0 })) });
function publish(): Snapshot {
  refreshPreviewConnection();
  const copy = clone();
  snapshotListeners.forEach(fn => fn(copy));
  return copy;
}
function findScene(id: string): Scene {
  const scene = preview.scenes.find(s => s.id === id);
  if (!scene) throw new Error('会话不存在');
  return scene;
}
// Older stores predate managed memory. Normalize each native response, including events.
function normalizeSnapshot(snapshot: Snapshot): Snapshot {
  return { ...snapshot, scenes: snapshot.scenes.map(scene => ({ ...scene, closed: scene.closed === true, connectionId: scene.connectionId ?? null, geometryHistory: scene.geometryHistory ?? emptyGeometryHistory() })), connections: snapshot.connections ?? [], defaultConnectionId: snapshot.defaultConnectionId ?? null, memories: [], memoryStats: snapshot.memoryStats ?? [], mcpServers: snapshot.mcpServers ?? [], agents: snapshot.agents.map(agent => ({ ...agent, memoryEnabled: agent.memoryEnabled !== false, defaultConnectionId: agent.defaultConnectionId ?? null, memoryProvider: agent.memoryProvider ?? { revision: 0, bindingId: null } })) };
}
async function invokeSnapshot(command: string, args?: Record<string, unknown>): Promise<Snapshot> {
  return normalizeSnapshot(await invoke<Snapshot>(command, args));
}
export async function continueRunFromJournal(input: import('./journal-contracts').ContinueFromRecord): Promise<Snapshot> {
  if (!native) throw new Error('请在桌面版继续回答');
  return invokeSnapshot('continue_run_from_journal', { ...input });
}
function cancelPreviewAgents(agentIds: string[], reason: string) {
  for (const scene of preview.scenes) {
    if (!agentIds.includes(scene.agentId) || scene.run?.status !== 'running') continue;
    const now = Date.now();
    scene.run.status = 'canceled'; scene.run.error = reason; scene.updatedAt = now;
    for (const step of scene.run.steps ?? []) if (step.status === 'running') { step.status = 'failed'; step.finishedAt = now; step.summary = reason; }
    const message = scene.messages.find(message => message.role === 'user' && message.runId === scene.run!.id);
    if (message) message.toolSteps = structuredClone(scene.run.steps ?? []);
  }
}
function cancelPreviewConnection(id: string) {
  for (const scene of preview.scenes) {
    if (scene.connectionId !== id || scene.run?.status !== 'running') continue;
    const reason = '连接配置已更改', now = Date.now();
    scene.run.status = 'canceled'; scene.run.error = reason; scene.updatedAt = now;
    for (const step of scene.run.steps ?? []) if (step.status === 'running') { step.status = 'failed'; step.finishedAt = now; step.summary = reason; }
    const message = scene.messages.find(message => message.role === 'user' && message.runId === scene.run!.id);
    if (message) message.toolSteps = structuredClone(scene.run.steps ?? []);
  }
}

export function assetUrl(asset: Asset): string {
  return native ? documentUrl(asset) : asset.path;
}
export function documentUrl(asset: Asset): string {
  if (!runtime?.contentOrigin) throw new Error('内容服务尚未就绪');
  return `${runtime.contentOrigin}/${encodeURIComponent(asset.id)}`;
}
export async function exportRegion(sceneId: string, regionId: string, clipboard: boolean): Promise<boolean> {
  if (!native) throw new Error('此操作需要桌面应用');
  const result = await invoke<boolean>('export_region', { sceneId, regionId, clipboard });
  if (typeof result !== 'boolean') throw new Error('图片导出结果无效');
  return result;
}
const mosaicPreviews = new MosaicPreviewCache();
export interface MosaicSource { regionId: string; source: Asset; drawingRevision: number }
export function getMosaicPreview(sceneId: string, background: Asset, blockSize: number, override?: MosaicSource): Promise<MosaicPreview> {
  if (!Number.isInteger(blockSize) || blockSize < 6 || blockSize > 40) return Promise.reject(new Error('马赛克块大小无效'));
  return mosaicPreviews.get(`${background.id}:${override?.source.id ?? ''}`, blockSize, async () => {
    const source = override?.source ?? background;
    const value = native ? await invoke<MosaicPreview>('get_mosaic_preview', { sceneId, backgroundId: background.id, blockSize, ...(override ? { regionId: override.regionId, sourceId: override.source.id, drawingRevision: override.drawingRevision } : {}) }) : await browserMosaicPreview(source.id, assetUrl(source), blockSize);
    if (value.backgroundId !== source.id || value.blockSize !== blockSize || value.columns !== Math.ceil(value.width / blockSize) || value.rows !== Math.ceil(value.height / blockSize) || !value.dataUrl.startsWith('data:image/png;base64,')) throw new Error('马赛克预览已失效');
    const image = new Image(); image.src = value.dataUrl; await image.decode();
    if (image.naturalWidth !== value.columns || image.naturalHeight !== value.rows) throw new Error('马赛克预览尺寸不符');
    return value;
  }, JSON.stringify([sceneId, override?.regionId ?? null, override?.drawingRevision ?? null]));
}
export async function exportTextAsset(assetId: string): Promise<void> {
  if (native) return invoke('export_text_asset', { assetId });
  const asset = preview.scenes.flatMap(scene => scene.items).map(item => item.asset).find(value => value.id === assetId && value.kind === 'text');
  if (!asset || !previewUrls.has(asset.path)) throw new Error('文本文件不存在');
  const link = document.createElement('a'); link.href = asset.path; link.download = asset.name;
  document.body.appendChild(link); link.click(); link.remove();
}
export async function startRecording(sceneId: string, regionId: string, audio: RecordingAudioSelection, grant: RecordingGrant): Promise<void> {
  if (!native) throw new Error('仅桌面版可录屏');
  return invoke('start_recording', { sceneId, regionId, audio, grant });
}
export async function controlRecording(id: string, action: RecordingAction): Promise<void> {
  if (!native) throw new Error('仅桌面版可录屏');
  return invoke('control_recording', { id, action });
}
export async function subscribeRecording(onStatus: (status: RecordingStatus | null) => void): Promise<() => void> {
  if (!native) { onStatus(null); return () => undefined; }
  let serial = 0, current: RecordingStatus | null = null, stopped = false;
  const finishedIds = new Set<string>();
  const accept = (next: RecordingStatus | null) => {
    if (stopped || (next && finishedIds.has(next.id))) return;
    if (next && current?.id === next.id) {
      if (current.phase === 'stopping' && next.phase !== 'stopping') return;
      if (current.phase !== 'countdown' && next.phase === 'countdown') return;
      if (['recording', 'paused', 'stopping'].includes(current.phase) && next.phase === 'starting') return;
      if (next.elapsedMs < current.elapsedMs) return;
      if (current.phase === 'countdown' && next.phase === 'countdown' && next.countdown > current.countdown) return;
    }
    if (current && current.id !== next?.id) finishedIds.add(current.id);
    current = next; onStatus(next);
  };
  const unlisten = await listen<RecordingStatus | null>('recording-status', event => { serial++; accept(event.payload); });
  try {
    const querySerial = serial;
    const initial = await invoke<RecordingStatus | null>('get_recording_status');
    // A delayed initial query must never overwrite a newer event (including null).
    if (querySerial === serial) accept(initial);
    return () => { stopped = true; unlisten(); };
  } catch (error) { stopped = true; unlisten(); throw error; }
}
export async function getSnapshot(): Promise<Snapshot> {
  if (!native) return clone();
  await runtimeInfo();
  return invokeSnapshot('get_snapshot');
}

export async function applyCommand(command: SceneCommand): Promise<Snapshot> {
  if (native) return invokeSnapshot('apply_scene_command', { command });
  if (command.type === 'finish_region_geometry_edit') { previewGeometry.finish(command.sceneId, command.editId); return clone(); }
  if (command.type === 'new_scene' || command.type === 'freeze_scene') {
    const old = findScene(command.type === 'freeze_scene' ? command.sceneId : preview.activeSceneId);
    if (old.closed) throw new Error('会话已关闭');
    previewGeometry.close();
    old.frozen = true;
    const next = emptyScene(old.agentId);
    next.connectionId = previewConnectionFor(old.agentId);
    preview.scenes.push(next);
    preview.activeSceneId = next.id;
  } else if (command.type === 'new_conversation') {
    const scene = findScene(command.sceneId);
    if (scene.id !== preview.activeSceneId || scene.closed || scene.frozen || scene.run?.status === 'running') throw new Error('当前无法开始新会话');
    scene.conversationStart = scene.messages.length; scene.draft = ''; scene.run = undefined;
  } else if (command.type === 'close_scene') {
    const scene = findScene(command.sceneId);
    if (scene.closed) return publish();
    previewGeometry.close();
    const now = Date.now();
    if (scene.run?.status === 'running') {
      scene.run.status = 'canceled'; scene.run.error = '会话已关闭';
      for (const step of scene.run.steps ?? []) if (step.status === 'running') { step.status = 'failed'; step.finishedAt = now; step.summary = '会话已关闭'; }
      const message = scene.messages.find(message => message.role === 'user' && message.runId === scene.run!.id);
      if (message) message.toolSteps = structuredClone(scene.run.steps ?? []);
    }
    scene.closed = true; scene.frozen = true; scene.updatedAt = now;
    if (scene.id === preview.activeSceneId) {
      const next = emptyScene(scene.agentId);
      next.connectionId = previewConnectionFor(scene.agentId);
      preview.scenes.push(next); preview.activeSceneId = next.id;
    }
  } else if (command.type === 'set_default_connection') {
    if (command.connectionId !== null && !preview.connections.some(value => value.id === command.connectionId)) throw new Error('连接不存在');
    preview.defaultConnectionId = command.connectionId;
  } else if (command.type === 'save_agent') {
    if (!command.agent.name.trim()) throw new Error('请输入名称');
    const agent = { ...structuredClone(command.agent), name: command.agent.name.trim(), memoryEnabled: command.agent.memoryEnabled !== false };
    if (agent.defaultConnectionId && !preview.connections.some(value => value.id === agent.defaultConnectionId)) throw new Error('连接不存在');
    const index = preview.agents.findIndex(a => a.id === command.agent.id);
    // Provider selection is changed only through its own CAS, never an identity draft.
    agent.memoryProvider = structuredClone(index < 0 ? { revision: 0, bindingId: null } : preview.agents[index].memoryProvider ?? { revision: 0, bindingId: null });
    if (index >= 0 && preview.agents[index].memoryEnabled && !agent.memoryEnabled) cancelPreviewAgents([agent.id], '长期记忆已关闭');
    if (index < 0) preview.agents.push(agent);
    else preview.agents[index] = agent;
  } else if (command.type === 'save_memory' || command.type === 'delete_memory') {
    if (!preview.agents.some(agent => agent.id === command.agentId)) throw new Error('Agent 不存在');
    const index = command.id ? previewMemories.findIndex(entry => entry.id === command.id && entry.agentId === command.agentId) : -1;
    const current = previewMemories[index];
    if (command.id && !current) throw new Error('记忆已删除，请重新查看');
    if (command.type === 'delete_memory') {
      if (current.revision !== command.expectedRevision) throw new Error('记忆已更新，请重新查看后保存');
      previewMemories.splice(index, 1);
    }
    else {
      const text = command.text.trim(), length = [...text].length;
      if (!length) throw new Error('请输入记忆内容');
      if (length > 2000) throw new Error('每条记忆最多 2000 字');
      const others = previewMemories.filter(entry => entry.agentId === command.agentId && entry.id !== current?.id);
      if (current && others.some(entry => entry.text === text)) throw new Error('该 Agent 已有相同内容的记忆');
      if (current ? current.revision !== command.expectedRevision : command.expectedRevision !== undefined) throw new Error('记忆已更新，请重新查看后保存');
      if (!current && others.some(entry => entry.text === text)) return publish();
      const now = Date.now();
      const next: MemoryEntry = { id: current?.id ?? uid(), agentId: command.agentId, text, revision: (current?.revision ?? 0) + 1, createdAt: current?.createdAt ?? now, updatedAt: Math.max(now, current?.updatedAt ?? 0), origin: 'manual' };
      if (current) previewMemories[index] = next;
      else previewMemories.push(next);
    }
    previewMemoryRevisions.set(command.agentId, (previewMemoryRevisions.get(command.agentId) ?? 0) + 1);
  } else if (command.type === 'set_mcp_grants' || command.type === 'set_mcp_enabled' || command.type === 'remove_mcp_server') {
    const server = preview.mcpServers.find(server => server.id === command.serverId);
    if (!server) throw new Error('工具程序不存在');
    if (server.revision !== command.expectedRevision) throw new Error('工具配置已更新，请载入最新后重试');
    const previouslyGranted = server.grants.filter(grant => grant.toolNames.length > 0).map(grant => grant.agentId);
    if (command.type === 'set_mcp_grants') {
      if (!preview.agents.some(agent => agent.id === command.agentId)) throw new Error('Agent 不存在');
      const names = [...command.toolNames].sort();
      if (new Set(names).size !== names.length) throw new Error('授权工具重复');
      if (names.some(name => !server.tools.some(tool => tool.name === name))) throw new Error('工具不存在，请重新读取');
      const existing = [...(server.grants.find(grant => grant.agentId === command.agentId)?.toolNames ?? [])].sort();
      if (existing.length === names.length && existing.every((name, index) => name === names[index])) return publish();
      const elsewhere = preview.mcpServers.filter(other => other.id !== server.id).reduce((sum, other) => sum + (other.grants.find(grant => grant.agentId === command.agentId)?.toolNames.length ?? 0), 0);
      if (names.length + elsewhere > 60) throw new Error('每个 Agent 最多启用 60 个工具');
      server.grants = server.grants.filter(grant => grant.agentId !== command.agentId);
      if (names.length) server.grants.push({ agentId: command.agentId, toolNames: names });
      server.revision++;
      cancelPreviewAgents(previouslyGranted, '工具配置已更改');
    } else if (command.type === 'set_mcp_enabled') {
      if (server.enabled === command.enabled) return publish();
      server.enabled = command.enabled; server.revision++;
      cancelPreviewAgents(previouslyGranted, '工具配置已更改');
    } else {
      preview.mcpServers = preview.mcpServers.filter(other => other.id !== server.id);
      cancelPreviewAgents(previouslyGranted, '工具配置已更改');
    }
  } else {
    const scene = findScene(command.sceneId);
    switch (command.type) {
      case 'activate_scene':
        previewGeometry.close();
        findScene(preview.activeSceneId).frozen = true;
        scene.frozen = false;
        scene.closed = false;
        preview.activeSceneId = scene.id;
        break;
      case 'set_draft': scene.draft = command.draft; break;
      case 'rename_scene': scene.title = command.title.trim() || '新会话'; break;
      case 'set_refs': scene.refs = structuredClone(command.refs); break;
      case 'set_scene_connection':
        if (scene.connectionId === command.connectionId) return publish();
        if (scene.run?.status === 'running') throw new Error('请先停止当前请求');
        if (command.connectionId !== null && !preview.connections.some(value => value.id === command.connectionId)) throw new Error('连接不存在');
        scene.connectionId = command.connectionId;
        break;
      case 'add_region': {
        const region = structuredClone(command.region); delete region.ocr; delete region.imageOverride; delete region.translation;
        previewGeometry.invalidate(scene);
        scene.regions.push(region); break;
      }
      case 'set_region_geometry': {
        if (scene.id !== preview.activeSceneId || scene.closed || scene.frozen || scene.run?.status === 'running') throw new Error('当前无法调整区域');
        if (!previewGeometry.apply(scene, command, uid)) return clone();
        break;
      }
      case 'undo_region_geometry':
      case 'redo_region_geometry':
        if (scene.id !== preview.activeSceneId || scene.closed || scene.frozen || scene.run?.status === 'running') throw new Error('当前无法调整区域');
        previewGeometry.replay(scene, command); break;
      case 'update_region': throw new Error('请使用区域几何操作');
      case 'remove_region':
        previewGeometry.invalidate(scene, command.regionId);
        scene.regions = scene.regions.filter(r => r.id !== command.regionId);
        scene.refs = scene.refs.filter(r => !(r.kind === 'region' && r.id === command.regionId));
        break;
      case 'update_item': {
        const index = scene.items.findIndex(i => i.id === command.item.id);
        if (index < 0) throw new Error('对象不存在');
        const old = scene.items[index];
        if (JSON.stringify(old.asset) !== JSON.stringify(command.item.asset)) throw new Error('对象来源已变化');
        scene.items[index] = { ...structuredClone(command.item), videoEdit: old.videoEdit };
        break;
      }
      case 'remove_item': {
        const item = scene.items.find(i => i.id === command.itemId);
        scene.items = scene.items.filter(i => i.id !== command.itemId);
        scene.refs = scene.refs.filter(r => !(r.kind === 'item' && r.id === command.itemId));
        if (item && previewUrls.has(item.asset.path)) {
          URL.revokeObjectURL(item.asset.path);
          previewUrls.delete(item.asset.path);
          artifactText.delete(item.asset.id);
        }
        break;
      }
      case 'set_agent':
        if (!preview.agents.some(agent => agent.id === command.agentId)) throw new Error('Agent 不存在');
        if (scene.run?.status === 'running') throw new Error('请先停止当前请求');
        if (scene.messages.length > 0 && scene.agentId !== command.agentId) throw new Error('新会话中可切换 Agent');
        scene.agentId = command.agentId;
        break;
    }
    scene.updatedAt = Date.now();
  }
  return publish();
}

export async function captureScreen(): Promise<Snapshot> {
  if (native) return invokeSnapshot('capture_screen');
  throw new Error('浏览器预览不支持截屏');
}
export async function createBlackboard(sceneId:string):Promise<Snapshot> {
  if(native)return invokeSnapshot('create_blackboard',{sceneId});
  const original=findScene(sceneId);
  if(preview.activeSceneId!==sceneId||original.closed||original.frozen||original.run?.status==='running')throw Error('会话已变化');
  const canvas=document.createElement('canvas');canvas.width=Math.max(1,Math.round(innerWidth));canvas.height=Math.max(1,Math.round(innerHeight));
  const context=canvas.getContext('2d');if(!context)throw Error('无法创建黑板');
  context.fillStyle='#33353A';context.fillRect(0,0,canvas.width,canvas.height);
  const asset:Asset={id:uid(),name:'黑板.png',kind:'image',path:canvas.toDataURL('image/png'),width:canvas.width,height:canvas.height};
  if(original.blackboardLink)throw Error('请先完成当前黑板');
  const target=emptyScene(original.agentId);target.connectionId=original.connectionId;
  const itemId=uid();target.blackboardLink={parentSceneId:original.id,itemId};
  original.items.push({id:itemId,asset:structuredClone(asset),x:.10+(original.items.length%4)*.035,y:.12+(original.items.length%4)*.035,width:.48,height:.48,state:{blackboardSceneId:target.id}});
  for(const scene of preview.scenes)scene.frozen=true;
  preview.scenes.push(target);preview.activeSceneId=target.id;
  target.title='黑板';target.background=asset;
  target.regions=[{id:uid(),x:0,y:0,width:canvas.width,height:canvas.height,drawings:[],drawingRevision:0,drawingHistory:{undo:[],redo:[]}}];
  target.refs=[{kind:'region',id:target.regions[0].id}];
  return publish();
}
export async function openBlackboard(sceneId:string,itemId:string):Promise<Snapshot>{
  if(native)return invokeSnapshot('open_blackboard',{sceneId,itemId});
  const parent=findScene(sceneId),board=preview.scenes.find(scene=>scene.blackboardLink?.parentSceneId===sceneId&&scene.blackboardLink.itemId===itemId);
  if(preview.activeSceneId!==sceneId||parent.closed||parent.frozen||parent.run?.status==='running'||!parent.items.some(item=>item.id===itemId)||!board||board.closed||board.run?.status==='running')throw Error('黑板不可编辑');
  for(const scene of preview.scenes)scene.frozen=scene.id!==board.id;
  preview.activeSceneId=board.id;return publish();
}
export async function finishBlackboard(sceneId:string):Promise<Snapshot>{
  if(native)return invokeSnapshot('finish_blackboard',{sceneId});
  const board=findScene(sceneId),link=board.blackboardLink;
  if(preview.activeSceneId!==sceneId||board.closed||board.frozen||board.run?.status==='running'||!link)throw Error('黑板已变化');
  const {blackboardPreview}=await import('./blackboard-preview');
  const original=JSON.stringify(board),path=await blackboardPreview(structuredClone(board));
  if(preview.activeSceneId!==sceneId||JSON.stringify(findScene(sceneId))!==original)throw Error('黑板已变化');
  const parent=findScene(link.parentSceneId),item=parent.items.find(item=>item.id===link.itemId);
  if(!item||parent.closed||parent.run?.status==='running')throw Error('原会话不可恢复');
  item.asset={...board.background!,id:uid(),path};
  if(!parent.refs.some(ref=>ref.kind==='item'&&ref.id===item.id))parent.refs.push({kind:'item',id:item.id});
  parent.updatedAt=Date.now();for(const scene of preview.scenes)scene.frozen=scene.id!==parent.id;
  preview.activeSceneId=parent.id;return publish();
}
/** In-memory browser preview. Desktop authoring always uses the native store. */
export async function previewDrawingCommand(command:DrawingCommand):Promise<Snapshot>{
  if(native)throw Error('预览操作不能修改桌面数据');
  const scene=findScene(command.sceneId),index=scene.regions.findIndex(region=>region.id===command.regionId);
  const original=scene.regions[index];
  if(preview.activeSceneId!==scene.id||scene.frozen||scene.closed||scene.run?.status==='running'||scene.background?.id!==command.backgroundId||!original||(original.drawingRevision??0)!==command.expectedRevision)throw Error('标注已更新');
  const region=structuredClone(original),history=region.drawingHistory??{undo:[],redo:[]},drawings=region.drawings??[];
  const apply=(edit:DrawingEdit,redo:boolean)=>{const before=redo?edit.before:edit.after,after=redo?edit.after:edit.before;
    if(before){if(drawings[edit.index]?.id!==before.id)throw Error('绘制历史已变化');if(after)drawings[edit.index]=structuredClone(after);else drawings.splice(edit.index,1);}
    else if(after)drawings.splice(edit.index,0,structuredClone(after));};
  if(command.type==='undo_drawing'||command.type==='redo_drawing'){
    const redo=command.type==='redo_drawing',source=redo?history.redo:history.undo,destination=redo?history.undo:history.redo,step=source.pop();
    if(!step)return clone();
    if('batch' in step){for(const edit of redo?step.batch:[...step.batch].reverse())apply(edit,redo);}else apply(step,redo);
    destination.push(step);
  }else if(command.type==='add_drawing'||command.type==='update_drawing'||command.type==='remove_drawing'){
    const id=command.type==='remove_drawing'?command.drawingId:command.drawing.id;
    const at=drawings.findIndex(drawing=>drawing.id===id);
    if(command.type==='add_drawing'?at>=0:at<0)throw Error('绘制对象已变化');
    const after=command.type==='remove_drawing'?null:structuredClone(command.drawing);
    if(after&&(!['pen','line','arrow','rect','ellipse','text','highlighter','number','mosaic'].includes(after.kind)||!after.points.length||after.points.length>4096||after.points.some(point=>![point.x,point.y].every(Number.isFinite))))throw Error('绘制内容无效');
    const edit:DrawingEdit={index:at<0?drawings.length:at,before:at<0?null:structuredClone(drawings[at]),after};
    apply(edit,true);history.undo.push(edit);history.redo=[];
    if(history.undo.length>128)history.undo.shift();
  }else throw Error('绘制操作无效');
  if(drawings.length>256)throw Error('绘制对象过多');
  region.drawings=drawings;region.drawingHistory=history;region.drawingRevision=command.expectedRevision+1;
  scene.regions[index]=region;return publish();
}
export async function discoverMcpServer(value: DiscoverMcpServer): Promise<Snapshot> {
  if (!native) throw new Error('仅桌面版可启动本机工具程序');
  return invokeSnapshot('discover_mcp_server', { id: value.id ?? null, expectedRevision: value.expectedRevision ?? null, label: value.label, command: value.command });
}

export async function memoryPage(value: { agentId: string; query?: string; cursor?: string; limit?: number }): Promise<MemoryPage> {
  if (native) return invoke('memory_page', { agentId: value.agentId, query: value.query ?? '', cursor: value.cursor ?? null, limit: value.limit ?? 25 });
  if (!preview.agents.some(agent => agent.id === value.agentId)) throw new Error('Agent 不存在');
  const query = (value.query ?? '').trim(), limit = value.limit ?? 25;
  if ([...query].length > 200 || query.includes('\0')) throw new Error('搜索内容最多 200 字，不能包含空字符');
  if (!Number.isInteger(limit) || limit < 1 || limit > 25) throw new Error('每页应为 1 至 25 条');
  const revision = previewMemoryRevisions.get(value.agentId) ?? 0;
  let offset = 0;
  if (value.cursor) {
    let cursor: { agentId?: unknown; query?: unknown; revision?: unknown; offset?: unknown };
    try { cursor = JSON.parse(value.cursor); } catch { throw new Error('分页已失效，请重新查看'); }
    if (!cursor || cursor.agentId !== value.agentId || cursor.query !== query || cursor.revision !== revision || !Number.isInteger(cursor.offset) || Number(cursor.offset) < 0) throw new Error('记忆已更新，请重新查看');
    offset = Number(cursor.offset);
  }
  const fold = (text: string) => text.replace(/[A-Z]/g, character => character.toLowerCase());
  const entries = previewMemories.filter(entry => entry.agentId === value.agentId && fold(entry.text).includes(fold(query)))
    .sort((left, right) => right.updatedAt - left.updatedAt || (left.id < right.id ? -1 : left.id > right.id ? 1 : 0));
  return structuredClone({ entries: entries.slice(offset, offset + limit), total: entries.length, revision,
    nextCursor: offset + limit < entries.length ? JSON.stringify({ agentId: value.agentId, query, revision, offset: offset + limit }) : undefined });
}

export async function memoryEntry(value: { agentId: string; id: string }): Promise<MemoryEntry | null> {
  if (native) return invoke('memory_entry', value);
  if (!preview.agents.some(agent => agent.id === value.agentId)) throw new Error('Agent 不存在');
  return structuredClone(previewMemories.find(entry => entry.agentId === value.agentId && entry.id === value.id) ?? null);
}

function pickFiles(): Promise<File[]> {
  return new Promise(resolve => {
    const input = document.createElement('input');
    input.type = 'file'; input.multiple = true;
    input.accept = IMPORT_ACCEPT;
    input.hidden = true;
    let settled = false;
    const finish = (files: File[]) => { if (settled) return; settled = true; input.remove(); resolve(files); };
    input.onchange = () => finish(Array.from(input.files ?? []));
    input.addEventListener('cancel', () => finish([]), { once: true });
    document.body.appendChild(input);
    input.click();
  });
}

export async function importAssets(sceneId: string): Promise<Snapshot> {
  if (native) return invokeSnapshot('import_assets', { sceneId });
  const initial = findScene(sceneId);
  const owner = { agentId: initial.agentId, background: JSON.stringify(initial.background ?? null) };
  const checkScene = () => {
    const scene = findScene(sceneId);
    if (preview.activeSceneId !== sceneId || scene.closed || scene.frozen || scene.agentId !== owner.agentId || JSON.stringify(scene.background ?? null) !== owner.background) throw new Error('导入会话已变化');
    return scene;
  };
  checkScene();
  const files = await pickFiles();
  if (!files.length) return clone();
  checkScene();
  const prepared = await prepareImportFiles(files, {
    id: uid, createUrl: file => URL.createObjectURL(file), revokeUrl: url => URL.revokeObjectURL(url),
    imageSize: async file => { const bitmap = await createImageBitmap(file); try { return { width: bitmap.width, height: bitmap.height }; } finally { bitmap.close(); } },
  });
  let scene: Scene;
  try { scene = checkScene(); }
  catch (error) { for (const entry of prepared) URL.revokeObjectURL(entry.asset.path); throw error; }
  const items = prepared.map((entry, index) => ({ id: uid(), asset: entry.asset, x: .12 + ((scene.items.length + index) % 4) * .05, y: .15 + ((scene.items.length + index) % 4) * .045, width: .34, height: entry.asset.kind === 'image' ? .35 : .43 }));
  for (const entry of prepared) {
    previewUrls.add(entry.asset.path);
    if (entry.text !== undefined) artifactText.set(entry.asset.id, entry.text);
  }
  scene.items.push(...items);
  scene.refs.push(...items.map(item => ({ kind: 'item' as const, id: item.id })));
  scene.updatedAt = Date.now();
  return publish();
}

export async function readArtifact(assetId: string): Promise<string> {
  if (native) return invoke('read_artifact', { assetId });
  const text = artifactText.get(assetId);
  if (text === undefined) throw new Error('无法读取文件');
  return text;
}

export async function getProviderPresets(): Promise<ProviderPreset[]> {
  return native ? invoke('get_provider_presets') : bundledProviderPresets(Object.values(providerPackages));
}
export async function discoverConnectionModels(value: ConnectionProbe): Promise<{ models: string[] }> {
  if (!native) throw new Error('请在桌面版获取模型，可手动输入模型 ID');
  return invoke('discover_connection_models', { ...value, expectedRevision: value.expectedRevision ?? null, apiKey: value.apiKey ?? null });
}
export async function testConnection(value: ConnectionProbe): Promise<ConnectionTestResult> {
  if (!native) throw new Error('请在桌面版测试连接');
  return invoke('test_connection', { ...value, expectedRevision: value.expectedRevision ?? null, apiKey: value.apiKey ?? null });
}
export async function cancelConnectionProbe(requestId: string): Promise<void> {
  if (native) await invoke('cancel_connection_probe', { requestId });
}
export async function saveConnectionProfile(profile: ConnectionProfile, expectedRevision?: number, apiKey?: string): Promise<Snapshot> {
  if (native) return invokeSnapshot('save_connection_profile', { profile, expectedRevision: expectedRevision ?? null, apiKey: apiKey ?? null });
  validateConnectionProfile(profile);
  const old = preview.connections.find(value => value.id === profile.id);
  if (old ? old.revision !== expectedRevision : expectedRevision !== undefined) throw new Error('连接已更改，请载入最新后重试');
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(profile.id)) throw new Error('连接身份无效');
  if (apiKey !== undefined && /[\r\n\0]/.test(apiKey)) throw new Error('API Key 包含无效字符');
  const hasKey = apiKey === undefined ? old?.hasKey ?? false : Boolean(apiKey);
  const next: ConnectionProfile = { ...structuredClone(profile), name: profile.name.trim(), baseUrl: profile.baseUrl.trim(), model: profile.model.trim(), advanced: { ...structuredClone(profile.advanced), requestPath: normalizeConnectionPath(profile.advanced.requestPath) }, revision: (old?.revision ?? 0) + 1, hasKey, credentialId: hasKey ? old?.credentialId ?? uid() : null };
  if (apiKey !== undefined) { if (apiKey) previewKeys.set(profile.id, apiKey); else previewKeys.delete(profile.id); }
  cancelPreviewConnection(profile.id);
  const index = preview.connections.findIndex(value => value.id === profile.id);
  if (index < 0) preview.connections.push(next); else preview.connections[index] = next;
  return publish();
}
export async function deleteConnectionProfile(id: string, expectedRevision: number): Promise<Snapshot> {
  if (native) return invokeSnapshot('delete_connection_profile', { id, expectedRevision });
  const value = preview.connections.find(value => value.id === id);
  if (!value || value.revision !== expectedRevision) throw new Error('连接已更改，请载入最新后重试');
  cancelPreviewConnection(id);
  preview.connections = preview.connections.filter(value => value.id !== id); previewKeys.delete(id);
  if (preview.defaultConnectionId === id) preview.defaultConnectionId = null;
  for (const scene of preview.scenes) if (scene.connectionId === id) scene.connectionId = null;
  for (const agent of preview.agents) if (agent.defaultConnectionId === id) agent.defaultConnectionId = null;
  return publish();
}

export async function sendMessage(sceneId: string, visualAnnotations?: VisualAnnotationGrant): Promise<Snapshot> {
  if (native) return invokeSnapshot('send_message', { sceneId, visualAnnotations: visualAnnotations ?? null });
  throw new Error('浏览器预览不连接模型，请在桌面应用发送');
}
export async function cancelRun(sceneId: string): Promise<Snapshot> {
  if (native) return invokeSnapshot('cancel_run', { sceneId });
  const scene = findScene(sceneId);
  if (scene.run?.status === 'running') scene.run.status = 'canceled';
  return publish();
}
export async function hideSpace(): Promise<void> {
  if (native) return invoke('hide_space');
}
export async function openSettings(): Promise<void> {
  if (native) return invoke('open_settings');
}
export async function closeSceneWindow(sceneId: string): Promise<Snapshot> {
  if (native) return invokeSnapshot('close_scene_window', { sceneId });
  return applyCommand({ type: 'close_scene', sceneId });
}
export async function freezeSpace(sceneId: string): Promise<Snapshot> {
  if (native) return invokeSnapshot('freeze_space', { sceneId });
  return applyCommand({ type: 'freeze_scene', sceneId });
}
export async function runtimeInfo(): Promise<RuntimeInfo> {
  if (!native) return { hotkey: '', captureBackend: '', contentOrigin: '' };
  if (runtime) return runtime;
  if (!runtimeRequest) {
    runtimeRequest = invoke<RuntimeInfo>('runtime_info').then(info => {
      const origin = new URL(info.contentOrigin);
      if (origin.protocol !== 'http:' || origin.hostname !== '127.0.0.1' || !origin.port || origin.username || origin.password || origin.pathname !== '/' || origin.search || origin.hash) throw new Error('内容服务地址无效');
      runtime = { ...info, contentOrigin: origin.origin };
      return runtime;
    }).catch(error => {
      runtimeRequest = undefined;
      throw error;
    });
  }
  return runtimeRequest;
}
export async function exportSnapshot(): Promise<void> {
  if (native) return invoke('export_snapshot');
  const current = clone();
  const data = { schemaVersion: current.schemaVersion, agents: current.agents, memories: structuredClone(previewMemories), memoryStats: current.memoryStats, mcpServers: current.mcpServers, scenes: current.scenes };
  const url = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' }));
  const link = document.createElement('a'); link.href = url; link.download = 'mewu-space.json'; link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export async function subscribe(onSnapshot: (snapshot: Snapshot) => void, onRun: (event: RunEvent) => void, onError: (message: string) => void): Promise<() => void> {
  if (!native) { snapshotListeners.add(onSnapshot); return () => snapshotListeners.delete(onSnapshot); }
  // Events can arrive before getSnapshot resolves; prepare document URLs first.
  await runtimeInfo();
  const stopSnapshot = await listen<Snapshot>('snapshot', event => onSnapshot(normalizeSnapshot(event.payload)));
  try {
    const stopRun = await listen<RunEvent>('run-event', event => onRun(event.payload));
    try {
      const stopError = await listen<string>('host-error', event => onError(event.payload));
      return () => { stopSnapshot(); stopRun(); stopError(); };
    } catch (error) { stopRun(); throw error; }
  } catch (error) { stopSnapshot(); throw error; }
}

export function isolatedDocument(source: string): string {
  const policy = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; media-src data: blob:; font-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
  return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${policy}"><meta name="viewport" content="width=device-width, initial-scale=1"><style>html,body{margin:0;min-height:100%;box-sizing:border-box}body{font-family:system-ui,sans-serif;overflow:auto}svg{max-width:100%;height:auto}*{box-sizing:border-box}</style></head><body>${source}</body></html>`;
}
