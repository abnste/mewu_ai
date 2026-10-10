// SPDX-License-Identifier: MPL-2.0
import type { Reference, SceneCommand, Snapshot } from './contracts';

export interface ExitPreparationRequest { requestId: string }
export interface ExitPreparationCanceled extends ExitPreparationRequest { error?: string }
export interface ExitPreparationResult extends ExitPreparationRequest { success: boolean; error: string | null }

/** Track only already-started space actions. No setting is saved by this barrier. */
export class SpaceOperations {
  private pending = new Set<Promise<void>>();
  begin(): () => void {
    let resolve!: () => void;
    const operation = new Promise<void>(done => { resolve = done; });
    this.pending.add(operation);
    return () => { this.pending.delete(operation); resolve(); };
  }
  async drain(active: () => boolean): Promise<void> {
    while (active() && this.pending.size) await Promise.all([...this.pending]);
  }
}

function sameReferences(left: Reference[], right: Reference[]) {
  return left.length === right.length && left.every((value, index) => value.kind === right[index].kind && value.id === right[index].id);
}

/** Local maps are overlays, not a full replacement of the host's scene collection. */
export function pendingSceneDrafts(snapshot: Snapshot, drafts: Record<string, string>, references: Record<string, Reference[]>): SceneCommand[] {
  const result: SceneCommand[] = [];
  const ids = new Set([...Object.keys(drafts), ...Object.keys(references)]);
  for (const id of ids) {
    const scene = snapshot.scenes.find(value => value.id === id);
    if (!scene) throw new Error('无法找到待保存草稿的会话');
    if (Object.hasOwn(drafts, id) && drafts[id] !== scene.draft) result.push({ type: 'set_draft', sceneId: id, draft: drafts[id] });
    if (Object.hasOwn(references, id) && !sameReferences(references[id], scene.refs)) result.push({ type: 'set_refs', sceneId: id, refs: references[id].map(value => ({ ...value })) });
  }
  return result;
}

export async function persistSceneDrafts(hooks: {
  active: () => boolean;
  snapshot: () => Snapshot | undefined;
  drafts: () => Record<string, string>;
  references: () => Record<string, Reference[]>;
  failure: () => unknown;
  save: (command: SceneCommand) => Promise<void>;
}) {
  while (hooks.active()) {
    const failure = hooks.failure();
    if (failure !== undefined) throw failure;
    const current = hooks.snapshot();
    if (!current) throw new Error('会话尚未就绪，无法保存草稿');
    const pending = pendingSceneDrafts(current, hooks.drafts(), hooks.references());
    if (!pending.length) return;
    for (const value of pending) {
      if (!hooks.active()) return;
      await hooks.save(value);
    }
    // Re-read overlays: a final IME input can arrive while an earlier save awaits IPC.
  }
}

export class ExitPreparation {
  private current?: { requestId: string };
  private disposed = false;
  private canceled = new Set<string>();
  constructor(private hooks: {
    lock: (locked: boolean) => void;
    beforePrepare?: (active: () => boolean) => Promise<void>;
    beginPreparation?: (request: ExitPreparationRequest) => Promise<void>;
    flush: (active: () => boolean) => Promise<void>;
    finish: (result: ExitPreparationResult) => Promise<void>;
    error: (message: string) => void;
  }) {}
  async prepare(request: ExitPreparationRequest): Promise<void> {
    if (this.disposed || !request.requestId || this.canceled.has(request.requestId) || this.current?.requestId === request.requestId) return;
    if (this.current) {
      await this.hooks.finish({ requestId: request.requestId, success: false, error: '正在准备退出' }).catch(() => undefined);
      return;
    }
    const token = { requestId: request.requestId };
    this.current = token; this.hooks.lock(true);
    const active = () => !this.disposed && this.current === token;
    try {
      if (this.hooks.beforePrepare) {
        await this.hooks.beforePrepare(active);
        if (!active()) return;
      }
      if (this.hooks.beginPreparation) {
        await this.hooks.beginPreparation({ requestId: token.requestId });
        if (!active()) return;
      }
      await this.hooks.flush(active);
      if (active()) await this.hooks.finish({ requestId: token.requestId, success: true, error: null });
      // Keep editing locked after success until the host exits or explicitly cancels.
    } catch (cause) {
      if (!active()) return;
      const message = cause instanceof Error ? cause.message : String(cause);
      this.hooks.error(message);
      await this.hooks.finish({ requestId: token.requestId, success: false, error: message }).catch(() => undefined);
      if (active()) { this.current = undefined; this.hooks.lock(false); }
    }
  }
  cancel(request: ExitPreparationCanceled) {
    if (this.disposed) return;
    this.canceled.add(request.requestId);
    if (this.canceled.size > 32) this.canceled.delete(this.canceled.values().next().value!);
    if (this.current?.requestId !== request.requestId) return;
    this.current = undefined; this.hooks.lock(false);
    if (request.error) this.hooks.error(request.error);
  }
  dispose() { this.disposed = true; this.current = undefined; }
}
