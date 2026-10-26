// SPDX-License-Identifier: MPL-2.0
// First-party space adapter. Native code owns CLI authentication and permission.
import type { DrawingCommand, OcrTarget, Reference, Region, SceneCommand } from './contracts';
import type { TranslationLanguage } from './translation-request';

export type AppControlSceneCommand = Extract<SceneCommand, {
  type: 'rename_scene' | 'set_agent' | 'set_scene_connection' | 'remove_region' | 'remove_item';
}>;
export interface AppControlParameters {
  'capture.open': Record<string, never>;
  'region.add': { region: Region; backgroundId: string };
  'chat.setDraft': { text: string; expectedDraft?: string };
  'chat.send': { text?: string; refs?: Reference[]; expectedDraft?: string; expectedRefs?: Reference[]; regionId?: string; automationOrigin?: string };
  'chat.cancel': { expectedRunId: string };
  'blackboard.create': Record<string, never>;
  'blackboard.finish': Record<string, never>;
  'blackboard.open': { itemId: string };
  'drawing.apply': { command: DrawingCommand };
  'recording.start': { regionId: string; backgroundId?: string };
  'recording.control': { id: string; action: 'pause' | 'resume' | 'stop' | 'cancel' };
  'scroll.start': { pluginId: string; revision: number; contributionId: string; target: OcrTarget; snapshotNodeId?: number };
  'scroll.control': { id: string; action: 'stop' | 'cancel' };
  'scene.activate': { sceneId: string };
  'scene.new': Record<string, never>;
  'scene.newConversation': Record<string, never>;
  'scene.freeze': Record<string, never>;
  'scene.close': Record<string, never>;
  'scene.command': { command: AppControlSceneCommand };
  'references.set': { refs: Reference[]; expectedRefs?: Reference[] };
  'settings.open': Record<string, never>;
  'asset.import': { paths: string[] };
  'image.export': { regionId: string; variant: 'annotated' | 'original'; path: string };
  'ocr.run': { pluginId: string; revision: number; contributionId: string; target: OcrTarget };
  'translation.run': { pluginId: string; revision: number; contributionId: string; target: OcrTarget; language?: TranslationLanguage };
  'pin.create': { pluginId: string; revision: number; contributionId: string; target: OcrTarget; translationId?: string | null };
}
export type AppControlMethod = keyof AppControlParameters;
export type AppControlOperation = { [M in AppControlMethod]: { method: M; params: AppControlParameters[M] } }[AppControlMethod];
export type AppControlRequest = AppControlOperation & { id: string; nonce: string; sceneId?: string; revision?: number };
export interface AppControlCurrent {
  sceneId: string; revision: number; backgroundId?: string;
  draft?: string; refs?: Reference[]; runId?: string;
}
export type AppControlCode = 'INVALID_REQUEST' | 'UNSUPPORTED_METHOD' | 'BUSY' | 'CONFLICT'
  | 'REQUEST_EXPIRED' | 'NOT_READY' | 'OPERATION_FAILED';
export interface AppControlFailure { code: AppControlCode; message: string; executed?: false }
export type AppControlCompletion = { id: string; nonce: string } & (
  { result: unknown; error?: never } | { error: AppControlFailure; result?: never }
);
export interface AppControlActions {
  current(): AppControlCurrent | undefined;
  flush(): Promise<void>;
  claim?(request: Readonly<AppControlRequest>): Promise<void>;
  execute<M extends AppControlMethod>(method: M, params: AppControlParameters[M]): Promise<unknown>;
  complete(payload: AppControlCompletion): Promise<void>;
}
export interface AppControlOptions {
  /** Deadline before dispatch only. Once started, native code owns Unknown. */
  requestTimeoutMs?: number;
  maxQueued?: number;
  now?: () => number;
}
export class AppControlError extends Error {
  constructor(readonly code: AppControlCode, message: string, options?: ErrorOptions) { super(message, options); this.name = 'AppControlError'; }
}

const MAX_BYTES = 1024 * 1024, MAX_DEPTH = 32, MAX_NODES = 100_000;
const requestFields = new Set(['id', 'nonce', 'sceneId', 'revision', 'method', 'params']);
const fields: Record<AppControlMethod, readonly string[]> = {
  'capture.open': [], 'region.add': ['region', 'backgroundId'],
  'chat.setDraft': ['text', 'expectedDraft'], 'chat.send': ['text', 'refs', 'expectedDraft', 'expectedRefs', 'regionId', 'automationOrigin'],
  'chat.cancel': ['expectedRunId'], 'blackboard.create': [], 'blackboard.finish': [], 'blackboard.open': ['itemId'],
  'drawing.apply': ['command'], 'recording.start': ['regionId', 'backgroundId'], 'recording.control': ['id', 'action'],
  'scroll.start': ['pluginId', 'revision', 'contributionId', 'target', 'snapshotNodeId'], 'scroll.control': ['id', 'action'],
  'scene.activate': ['sceneId'], 'scene.new': [], 'scene.newConversation': [], 'scene.freeze': [], 'scene.close': [],
  'scene.command': ['command'], 'references.set': ['refs', 'expectedRefs'], 'settings.open': [],
  'asset.import': ['paths'], 'image.export': ['regionId', 'variant', 'path'],
  'ocr.run': ['pluginId', 'revision', 'contributionId', 'target'],
  'translation.run': ['pluginId', 'revision', 'contributionId', 'target', 'language'],
  'pin.create': ['pluginId', 'revision', 'contributionId', 'target', 'translationId'],
};
export const appControlMethods: readonly AppControlMethod[] = Object.freeze(Object.keys(fields) as AppControlMethod[]);

function invalid(): never { throw new AppControlError('INVALID_REQUEST', '控制请求格式无效'); }
function conflict(): never { throw new AppControlError('CONFLICT', '操作目标已变化'); }
function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}
function keys(value: Record<string, unknown>, allowed: readonly string[]): void {
  if (Object.keys(value).some(key => !allowed.includes(key))) invalid();
}
function identifier(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= 256
    && !/[\u0000-\u001f\u007f]/u.test(value);
}
function revision(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}
function references(value: unknown): value is Reference[] {
  if (!Array.isArray(value) || value.length > 256) return false;
  const seen = new Set<string>();
  return value.every(entry => {
    if (!object(entry) || Object.keys(entry).some(key => key !== 'kind' && key !== 'id')
      || (entry.kind !== 'region' && entry.kind !== 'item') || !identifier(entry.id)) return false;
    const key = `${entry.kind}:${entry.id}`;
    if (seen.has(key)) return false;
    seen.add(key); return true;
  });
}
function geometry(value: Record<string, unknown>): void {
  for (const key of ['x', 'y', 'width', 'height']) {
    if (typeof value[key] !== 'number' || !Number.isFinite(value[key])) invalid();
  }
  if ((value.x as number) < 0 || (value.y as number) < 0 || (value.width as number) <= 0 || (value.height as number) <= 0) invalid();
}
function path(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= 32767 && !/[\u0000-\u001f\u007f]/u.test(value);
}
function selectionPlugin(params: Record<string, unknown>): void {
  if (!identifier(params.pluginId) || !revision(params.revision) || !identifier(params.contributionId) || !object(params.target)) invalid();
  keys(params.target, ['sceneId', 'regionId', 'backgroundId', 'drawingRevision', 'x', 'y', 'width', 'height']);
  if (!identifier(params.target.sceneId) || !identifier(params.target.regionId) || !identifier(params.target.backgroundId) || !revision(params.target.drawingRevision)) invalid();
  geometry(params.target);
}
function drawing(value: unknown): void {
  if (!object(value) || !identifier(value.sceneId) || !identifier(value.regionId)
    || !identifier(value.backgroundId) || !revision(value.expectedRevision)) invalid();
  const common = ['type', 'sceneId', 'regionId', 'backgroundId', 'expectedRevision'];
  switch (value.type) {
    case 'add_drawing': case 'update_drawing':
      keys(value, [...common, 'drawing']); if (!object(value.drawing)) invalid(); break;
    case 'update_drawings':
      keys(value, [...common, 'drawings']); if (!Array.isArray(value.drawings) || !value.drawings.length || !value.drawings.every(object)) invalid(); break;
    case 'erase_drawing_strokes':
      keys(value, [...common, 'erasures']); if (!Array.isArray(value.erasures) || !value.erasures.length || !value.erasures.every(object)) invalid(); break;
    case 'remove_drawing':
      keys(value, [...common, 'drawingId']); if (!identifier(value.drawingId)) invalid(); break;
    case 'remove_drawings':
      keys(value, [...common, 'drawingIds']); if (!Array.isArray(value.drawingIds) || !value.drawingIds.length || !value.drawingIds.every(identifier)) invalid(); break;
    case 'undo_drawing': case 'redo_drawing': keys(value, common); break;
    default: invalid();
  }
  // Rich layout/source proofs and exact drawing schemas remain native authority.
}
function sceneCommand(value: unknown): void {
  if (!object(value) || !identifier(value.sceneId)) invalid();
  switch (value.type) {
    case 'rename_scene': keys(value, ['type', 'sceneId', 'title']); if (typeof value.title !== 'string') invalid(); break;
    case 'set_agent': keys(value, ['type', 'sceneId', 'agentId']); if (!identifier(value.agentId)) invalid(); break;
    case 'set_scene_connection': keys(value, ['type', 'sceneId', 'connectionId']); if (value.connectionId !== null && !identifier(value.connectionId)) invalid(); break;
    case 'remove_region': keys(value, ['type', 'sceneId', 'regionId']); if (!identifier(value.regionId)) invalid(); break;
    case 'remove_item': keys(value, ['type', 'sceneId', 'itemId']); if (!identifier(value.itemId)) invalid(); break;
    default: invalid();
  }
}
function validateParams(method: AppControlMethod, params: Record<string, unknown>): void {
  keys(params, fields[method]);
  switch (method) {
    case 'region.add':
      if (!identifier(params.backgroundId) || !object(params.region)) invalid();
      keys(params.region, ['id', 'x', 'y', 'width', 'height']);
      if (!identifier(params.region.id)) invalid(); geometry(params.region); break;
    case 'chat.setDraft':
      if (typeof params.text !== 'string') invalid();
      if (params.expectedDraft !== undefined && typeof params.expectedDraft !== 'string') invalid(); break;
    case 'chat.send':
      for (const key of ['text', 'expectedDraft']) if (params[key] !== undefined && typeof params[key] !== 'string') invalid();
      for (const key of ['refs', 'expectedRefs']) if (params[key] !== undefined && !references(params[key])) invalid();
      if (params.regionId !== undefined && !identifier(params.regionId)) invalid();
      if (params.automationOrigin !== undefined && (typeof params.automationOrigin !== 'string' || !/^[0-9a-f]{8}-[0-9a-f-]{27}$/.test(params.automationOrigin))) invalid(); break;
    case 'chat.cancel': if (!identifier(params.expectedRunId)) invalid(); break;
    case 'blackboard.open': if (!identifier(params.itemId)) invalid(); break;
    case 'drawing.apply': drawing(params.command); break;
    case 'recording.start':
      if (!identifier(params.regionId) || (params.backgroundId !== undefined && !identifier(params.backgroundId))) invalid(); break;
    case 'recording.control':
      if (!identifier(params.id) || !['pause', 'resume', 'stop', 'cancel'].includes(params.action as string)) invalid(); break;
    case 'scroll.start':
      selectionPlugin(params);
      if (params.snapshotNodeId !== undefined && (!revision(params.snapshotNodeId) || params.snapshotNodeId < 1 || params.snapshotNodeId > 0xffff_ffff)) invalid(); break;
    case 'scroll.control':
      if (!identifier(params.id) || !['stop', 'cancel'].includes(params.action as string)) invalid(); break;
    case 'scene.activate': if (!identifier(params.sceneId)) invalid(); break;
    case 'scene.command': sceneCommand(params.command); break;
    case 'references.set':
      if (!references(params.refs) || (params.expectedRefs !== undefined && !references(params.expectedRefs))) invalid(); break;
    case 'asset.import':
      if (!Array.isArray(params.paths) || !params.paths.length || params.paths.length > 32 || !params.paths.every(path) || new Set(params.paths).size !== params.paths.length) invalid(); break;
    case 'image.export':
      if (!identifier(params.regionId) || !['annotated', 'original'].includes(params.variant as string) || !path(params.path)) invalid(); break;
    case 'ocr.run': selectionPlugin(params); break;
    case 'translation.run':
      selectionPlugin(params);
      if (params.language !== undefined && !['zh-Hans', 'en', 'ja', 'ko', 'de', 'fr', 'es'].includes(params.language as string)) invalid(); break;
    case 'pin.create':
      selectionPlugin(params);
      if (params.translationId !== undefined && params.translationId !== null && !identifier(params.translationId)) invalid(); break;
  }
}
/** Decode only a bounded plain JSON tree. Never retain a mutable event object. */
export function parseAppControlRequest(value: unknown): AppControlRequest {
  let nodes = 0;
  function visit(entry: unknown, depth: number): void {
    if (++nodes > MAX_NODES || depth > MAX_DEPTH) invalid();
    if (entry === null || typeof entry === 'string' || typeof entry === 'boolean') return;
    if (typeof entry === 'number') { if (!Number.isFinite(entry)) invalid(); return; }
    if (Array.isArray(entry)) {
      if (Object.getPrototypeOf(entry) !== Array.prototype || entry.length > MAX_NODES
        || Reflect.ownKeys(entry).length !== entry.length + 1) invalid();
      for (let index = 0; index < entry.length; index++) {
        const descriptor = Object.getOwnPropertyDescriptor(entry, index);
        if (!descriptor || !('value' in descriptor) || !descriptor.enumerable) invalid();
        visit(descriptor.value, depth + 1);
      }
      return;
    }
    if (!object(entry) || ![Object.prototype, null].includes(Object.getPrototypeOf(entry))) invalid();
    for (const [key, descriptor] of Object.entries(Object.getOwnPropertyDescriptors(entry))) {
      if (key === '__proto__' || key === 'constructor' || key === 'prototype' || !('value' in descriptor) || !descriptor.enumerable) invalid();
      visit(descriptor.value, depth + 1);
    }
    if (Reflect.ownKeys(entry).length !== Object.keys(entry).length) invalid();
  }
  visit(value, 0);
  const encoded = JSON.stringify(value);
  if (encoded.length > MAX_BYTES || new TextEncoder().encode(encoded).byteLength > MAX_BYTES) invalid();
  const request: unknown = JSON.parse(encoded);
  if (!object(request) || Object.keys(request).some(key => !requestFields.has(key))
    || !identifier(request.id) || !identifier(request.nonce) || typeof request.method !== 'string'
    || !object(request.params)) invalid();
  if (!Object.hasOwn(fields, request.method)) throw new AppControlError('UNSUPPORTED_METHOD', '不支持此控制操作');
  if (request.sceneId !== undefined && !identifier(request.sceneId)) invalid();
  if (request.revision !== undefined && !revision(request.revision)) invalid();
  validateParams(request.method as AppControlMethod, request.params);
  return request as AppControlRequest;
}

function interrupt(request: AppControlRequest): boolean {
  return request.method === 'chat.cancel' || request.method === 'scroll.control'
    || (request.method === 'recording.control' && ['stop', 'cancel'].includes(request.params.action));
}
export function sameControlReferences(left: Reference[] | undefined, right: Reference[]): boolean {
  return left !== undefined && left.length === right.length
    && left.every((entry, index) => entry.kind === right[index].kind && entry.id === right[index].id);
}
function checkTarget(request: AppControlRequest, current: AppControlCurrent): void {
  const params = request.params as Record<string, unknown>;
  if ('expectedDraft' in params && current.draft !== params.expectedDraft) conflict();
  if ('expectedRefs' in params && !sameControlReferences(current.refs, params.expectedRefs as Reference[])) conflict();
  if (request.method === 'chat.cancel' && current.runId !== request.params.expectedRunId) conflict();
  if ('backgroundId' in params && current.backgroundId !== params.backgroundId) conflict();
  if (request.method === 'drawing.apply' || request.method === 'scene.command') {
    if (request.params.command.sceneId !== current.sceneId) conflict();
    if (request.method === 'drawing.apply' && request.params.command.backgroundId !== current.backgroundId) conflict();
  }
  if (['scroll.start', 'ocr.run', 'translation.run', 'pin.create'].includes(request.method)
    && ('target' in request.params)
    && (request.params.target.sceneId !== current.sceneId || request.params.target.backgroundId !== current.backgroundId)) conflict();
}
function failure(error: unknown, started: boolean): AppControlFailure {
  return {
    code: error instanceof AppControlError ? error.code : 'OPERATION_FAILED',
    message: error instanceof AppControlError ? error.message : '操作未完成',
    ...(!started ? { executed: false as const } : {}),
  };
}

export function createAppControl(actions: AppControlActions, options: AppControlOptions = {}) {
  const now = options.now ?? Date.now, timeout = options.requestTimeoutMs ?? 30_000, maxQueued = options.maxQueued ?? 32;
  if (!Number.isSafeInteger(timeout) || timeout <= 0 || timeout > 120_000 || !Number.isSafeInteger(maxQueued) || maxQueued < 1 || maxQueued > 128) throw Error('Invalid app control limits');
  let disposed = false, queued = 0, lane = Promise.resolve();
  const pending = new Map<string, { nonce: string; completion: Promise<void> }>();
  const recent = new Map<string, string>();
  async function run(request: AppControlRequest, deadline: number): Promise<void> {
    let started = false;
    let completion: AppControlCompletion;
    try {
      const check = () => {
        if (disposed) throw new AppControlError('NOT_READY', '空间控制已停止');
        if (now() >= deadline) throw new AppControlError('REQUEST_EXPIRED', '控制请求已过期');
        const current = actions.current();
        if (!current || !identifier(current.sceneId) || !revision(current.revision)) throw new AppControlError('NOT_READY', '空间尚未就绪');
        if (request.sceneId !== undefined && current.sceneId !== request.sceneId) conflict();
        checkTarget(request, current); return { ...current };
      };
      const before = check();
      if (request.revision !== undefined && before.revision !== request.revision) conflict();
      if (!interrupt(request) && request.method !== 'recording.control') {
        await actions.flush();
        const after = check();
        if (before.sceneId !== after.sceneId) conflict();
      }
      const beforeClaim = check();
      await actions.claim?.(request);
      // Claim can await native permission/revision checks. Input may have
      // changed meanwhile, and a claimed request must not target its successor.
      const claimed = check();
      if (before.sceneId !== claimed.sceneId) conflict();
      if (request.revision !== undefined && beforeClaim.revision !== claimed.revision) conflict();
      started = true;
      const result = await actions.execute(request.method, request.params);
      // A dispatched operation may have committed. Never label its timeout or
      // disappearing renderer as NotSent, and never automatically replay it.
      completion = { id: request.id, nonce: request.nonce, result: result ?? null };
    } catch (error) {
      completion = { id: request.id, nonce: request.nonce, error: failure(error, started) };
    }
    await actions.complete(completion);
  }
  async function rejectKnown(request: { id: string; nonce: string }, error: unknown): Promise<void> {
    await actions.complete({ id: request.id, nonce: request.nonce, error: failure(error, false) });
  }
  function handle(value: unknown): Promise<void> {
    let request: AppControlRequest;
    try { request = parseAppControlRequest(value); }
    catch (error) {
      // Do not invoke accessors merely to recover IDs from a malformed event.
      if (object(value)) {
        const id = Object.getOwnPropertyDescriptor(value, 'id'), nonce = Object.getOwnPropertyDescriptor(value, 'nonce');
        if (id && nonce && 'value' in id && 'value' in nonce && identifier(id.value) && identifier(nonce.value)) return rejectKnown({ id: id.value, nonce: nonce.value }, error);
      }
      return Promise.resolve();
    }
    const active = pending.get(request.id);
    if (active) return active.nonce === request.nonce ? active.completion : rejectKnown(request, new AppControlError('CONFLICT', '控制请求标识已使用'));
    const completed = recent.get(request.id);
    if (completed !== undefined) return completed === request.nonce ? Promise.resolve() : rejectKnown(request, new AppControlError('CONFLICT', '控制请求标识已使用'));
    if (disposed) return rejectKnown(request, new AppControlError('NOT_READY', '空间控制已停止'));
    const urgent = interrupt(request);
    if ((!urgent && queued >= maxQueued) || pending.size >= maxQueued + 8) return rejectKnown(request, new AppControlError('BUSY', '空间控制队列已满'));
    const deadline = now() + timeout;
    if (!urgent) queued++;
    const work = urgent ? Promise.resolve().then(() => run(request, deadline)) : lane.then(() => run(request, deadline));
    const completion = work.finally(() => {
      if (!urgent) queued--;
      pending.delete(request.id); recent.set(request.id, request.nonce);
      // Native code has already retired old nonces before this replay cache is
      // pruned. The cache is only duplicate-event protection, never authority.
      if (recent.size > 128) recent.delete(recent.keys().next().value!);
    });
    pending.set(request.id, { nonce: request.nonce, completion });
    if (!urgent) lane = completion.catch(() => {});
    return completion;
  }
  return { handle, dispose() { disposed = true; } };
}

export type AppControlListen = (name: 'mewu-control-request', handler: (event: { payload: unknown }) => void,
  options: { target: 'space' }) => Promise<() => void>;
/** Install only in the first-party space. There is no browser/demo control path. */
export async function attachAppControl(actions: AppControlActions, injectedListen?: AppControlListen,
  options?: AppControlOptions): Promise<() => void> {
  const adapter = createAppControl(actions, options);
  const listen: AppControlListen = injectedListen ?? (await import('@tauri-apps/api/event')).listen;
  let stop: (() => void) | undefined;
  try {
    stop = await listen('mewu-control-request', event => {
      // A lost completion transport is not a reason to repeat an operation.
      void adapter.handle(event.payload).catch(() => {});
    }, { target: 'space' });
  } catch (error) { adapter.dispose(); throw error; }
  return () => { adapter.dispose(); stop?.(); stop = undefined; };
}
