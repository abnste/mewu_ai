// SPDX-License-Identifier: MPL-2.0
import type { Region, RegionGeometry, RegionGeometryCommand, Scene, Snapshot } from './contracts';

export interface GeometryTarget { sceneId: string; regionId: string; backgroundId: string; sourceId: string; revision: number; geometry: RegionGeometry; width: number; height: number }
export interface GeometryReceipt { sceneId: string; regionId: string; backgroundId: string; sourceId: string; revision: number; geometry: RegionGeometry }
export interface GeometryView { overlay?: GeometryReceipt; pending: boolean; held: boolean; mode?: 'pointer' | 'keyboard' }
export const regionGeometry = (value: RegionGeometry): RegionGeometry => ({ x: value.x, y: value.y, width: value.width, height: value.height });
export const sameGeometry = (a: RegionGeometry, b: RegionGeometry) => a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
const sameIdentity = (a: GeometryReceipt, b: GeometryReceipt) => a.sceneId === b.sceneId && a.regionId === b.regionId && a.backgroundId === b.backgroundId && a.sourceId === b.sourceId;
export function geometryTarget(scene: Scene | undefined, regionId: string): GeometryTarget | undefined {
  const region = scene?.regions.find(value => value.id === regionId), background = scene?.background;
  if (!scene || scene.closed || scene.frozen || scene.run?.status === 'running' || !region || !background?.width || !background.height) return;
  return { sceneId: scene.id, regionId, backgroundId: background.id, sourceId: region.imageOverride?.id ?? background.id, revision: region.drawingRevision ?? 0, geometry: regionGeometry(region), width: background.width, height: background.height };
}
export function geometryReceipt(snapshot: Snapshot, command: RegionGeometryCommand): GeometryReceipt {
  const scene = snapshot.scenes.find(value => value.id === command.sceneId), target = geometryTarget(scene, command.regionId);
  if (!target || target.backgroundId !== command.backgroundId || target.sourceId !== command.sourceId || !sameGeometry(target.geometry, command.to) || target.revision !== command.expectedRevision + (sameGeometry(command.from, command.to) ? 0 : 1)) throw new Error('区域位置已变化');
  return target;
}
export function overlayRegion(scene: Scene, region: Region, overlay?: GeometryReceipt): Region {
  if (!overlay || overlay.sceneId !== scene.id || overlay.regionId !== region.id || overlay.backgroundId !== scene.background?.id || overlay.sourceId !== (region.imageOverride?.id ?? scene.background?.id)) return region;
  return { ...region, ...overlay.geometry };
}
export function nudgedGeometry(value: RegionGeometry, dx: number, dy: number, width: number, height: number): RegionGeometry {
  return { ...value, x: Math.max(0, Math.min(width - value.width, value.x + dx)), y: Math.max(0, Math.min(height - value.height, value.y + dy)) };
}
interface Session { target: GeometryTarget; latest: RegionGeometry; editId: string; mode: 'pointer' | 'keyboard'; held: boolean; attempted: boolean }
/** One native transaction; each of at most 50 gesture boundaries retains only its latest target. */
export class RegionGeometryCoordinator {
  private queue: Session[] = [];
  private flight?: Promise<void>;
  private cleanup?: { sceneId: string; editId: string };
  private failure?: { error: Error; held: boolean };
  private timer?: ReturnType<typeof setTimeout>;
  private endOperation?: () => void;
  private waiters = new Set<() => void>();
  private disposed = false;
  private paused = false;
  private epoch = 0;
  private capacityWarning = false;
  private failureSerial = 0;
  private lastFailure?: Error;
  constructor(private hooks: {
    current: (sceneId: string, regionId: string) => GeometryTarget | undefined;
    commit: (command: RegionGeometryCommand) => Promise<GeometryReceipt>;
    finish: (sceneId: string, editId: string) => Promise<void>;
    changed: (view: GeometryView) => void;
    error: (error: Error) => void;
    beginOperation?: () => () => void;
    delay?: number;
    newId?: () => string;
  }) {}
  private dirty(value: Session) { return !sameGeometry(value.target.geometry, value.latest); }
  private work() { return Boolean(this.cleanup || this.queue.some(value => value.attempted || this.dirty(value))); }
  private wake() { for (const resolve of this.waiters) resolve(); this.waiters.clear(); }
  private publish() {
    if (this.disposed) return;
    const value = this.queue.at(-1), head = this.queue[0];
    this.hooks.changed({ overlay: value && (this.flight || this.work()) ? { ...value.target, geometry: { ...value.latest } } : undefined, pending: Boolean(this.flight || this.work()), held: value?.held ?? false, mode: value?.mode ?? head?.mode });
    if (!this.flight && !this.work()) { this.endOperation?.(); this.endOperation = undefined; }
    if (this.queue.length < 50) this.capacityWarning = false;
    this.wake();
  }
  private operation() { this.endOperation ??= this.hooks.beginOperation?.(); }
  private clearTimer() { clearTimeout(this.timer); this.timer = undefined; }
  private fail(error: unknown, cleanup?: { sceneId: string; editId: string }) {
    if (this.failure) return;
    this.failure = { error: error instanceof Error ? error : new Error(String(error)), held: this.queue.at(-1)?.held ?? false };
    this.lastFailure = this.failure.error; this.failureSerial++;
    this.queue = []; this.cleanup = cleanup; this.clearTimer(); this.publish(); this.hooks.error(this.failure.error);
  }
  private start(target: GeometryTarget, mode: Session['mode']): Session | undefined {
    if (this.disposed || this.paused || this.cleanup || this.failure?.held) return;
    if (this.queue.length >= 50) {
      if (!this.capacityWarning) { this.capacityWarning = true; this.hooks.error(new Error('正在保存区域，请稍候')); }
      return;
    }
    const previous = this.queue.at(-1);
    if (previous?.held || (previous && (!sameIdentity(previous.target, target) || mode === 'pointer'))) return;
    if (!previous && this.flight) return;
    const current = this.hooks.current(target.sceneId, target.regionId);
    if (!current || !sameIdentity(current, target) || (!previous && (current.revision !== target.revision || !sameGeometry(current.geometry, target.geometry)))) return;
    this.failure = undefined;
    const base = previous ? { ...previous.target, geometry: { ...previous.latest } } : { ...target, geometry: { ...target.geometry } };
    const value: Session = { target: base, latest: { ...base.geometry }, editId: this.hooks.newId?.() ?? crypto.randomUUID(), mode, held: true, attempted: false };
    this.queue.push(value); this.publish(); return value;
  }
  beginPointer(target: GeometryTarget) { return Boolean(this.start(target, 'pointer')); }
  previewPointer(to: RegionGeometry) {
    const value = this.queue.at(-1); if (!value || value.mode !== 'pointer' || !value.held || this.failure || this.paused) return;
    value.latest = regionGeometry(to); this.operation(); this.publish();
  }
  async endPointer(commit: boolean) {
    const value = this.queue.at(-1); if (!value || value.mode !== 'pointer') return;
    value.held = false;
    if (!commit) value.latest = { ...value.target.geometry };
    this.prune(); this.publish();
    await this.flush();
  }
  nudge(target: GeometryTarget, dx: number, dy: number, fresh: boolean): boolean {
    if (this.disposed || this.paused || this.failure?.held) return false;
    let value = this.queue.at(-1);
    if (!value || value.mode !== 'keyboard' || !sameIdentity(value.target, target) || !value.held) {
      if (!fresh) return false;
      const base = value && sameIdentity(value.target, target) ? value.latest : target.geometry;
      if (sameGeometry(base, nudgedGeometry(base, dx, dy, target.width, target.height))) return true;
      value = this.start(target, 'keyboard'); if (!value) return false;
    }
    value.latest = nudgedGeometry(value.latest, dx, dy, value.target.width, value.target.height);
    if (this.dirty(value)) { this.operation(); this.schedule(); } this.publish(); return true;
  }
  endKeys(flush = true) {
    const value = this.queue.at(-1);
    if (value?.mode === 'keyboard') value.held = false;
    if (this.failure) this.failure.held = false;
    this.prune(); this.publish(); if (flush) void this.flush().catch(() => {});
  }
  private prune() {
    if (this.flight) return;
    // A boundary that never reached native and returned to its own start is empty.
    while (this.queue.length && !this.queue[0].held && !this.queue[0].attempted && !this.dirty(this.queue[0])) {
      const previous = this.queue.shift()!, next = this.queue[0];
      if (next) next.target.revision = previous.target.revision;
    }
  }
  private schedule() {
    const value = this.queue[0];
    if (this.timer || this.flight || this.disposed || (!this.cleanup && (!value || (value.held && !this.dirty(value))))) return;
    this.timer = setTimeout(() => { this.timer = undefined; void this.submit(); }, this.cleanup || !value?.held ? 0 : this.hooks.delay ?? 75);
  }
  private submit(): Promise<void> {
    if (this.flight) return this.flight;
    this.prune();
    const value = this.queue[0], orphan = this.cleanup;
    if (!orphan && (!value || (value.held && !this.dirty(value)))) return Promise.resolve();
    const token = this.epoch, writing = Boolean(!orphan && value && this.dirty(value));
    const command: RegionGeometryCommand | undefined = writing ? { type: 'set_region_geometry', sceneId: value.target.sceneId, regionId: value.target.regionId, backgroundId: value.target.backgroundId, sourceId: value.target.sourceId, expectedRevision: value.target.revision, from: { ...value.target.geometry }, to: { ...value.latest }, editId: value.editId } : undefined;
    this.clearTimer();
    const job = Promise.resolve().then(async () => {
      if (orphan) { await this.hooks.finish(orphan.sceneId, orphan.editId); if (this.cleanup === orphan) this.cleanup = undefined; return; }
      const before = this.hooks.current(value.target.sceneId, value.target.regionId);
      if (!before || !sameIdentity(before, value.target) || before.revision !== value.target.revision || !sameGeometry(before.geometry, value.target.geometry)) throw new Error('区域位置已变化');
      if (command) {
        value.attempted = true;
        const receipt = await this.hooks.commit(command);
        if (this.disposed || token !== this.epoch || this.queue[0] !== value) return;
        const current = this.hooks.current(receipt.sceneId, receipt.regionId);
        if (!current || !sameIdentity(current, receipt) || current.revision !== receipt.revision || !sameGeometry(current.geometry, receipt.geometry)) throw new Error('区域位置已变化');
        value.target = { ...value.target, revision: receipt.revision, geometry: { ...receipt.geometry } };
      } else {
        await this.hooks.finish(value.target.sceneId, value.editId);
        if (this.disposed || token !== this.epoch || this.queue[0] !== value) return;
        const current = this.hooks.current(value.target.sceneId, value.target.regionId);
        if (!current || !sameIdentity(current, value.target) || current.revision !== value.target.revision || !sameGeometry(current.geometry, value.target.geometry)) throw new Error('区域位置已变化');
        this.queue.shift(); const next = this.queue[0]; if (next) next.target.revision = value.target.revision;
      }
    }).catch(error => {
      if (orphan && this.cleanup === orphan) this.cleanup = undefined;
      if (!this.disposed && token === this.epoch) this.fail(error, command && value.attempted ? { sceneId: value.target.sceneId, editId: value.editId } : undefined);
    }).finally(() => {
      if (this.flight === job) this.flight = undefined;
      this.publish(); if (!this.disposed) this.schedule();
    });
    this.flight = job; this.publish(); return job;
  }
  async flush(): Promise<void> {
    const failureSerial = this.failureSerial, existingFailure = this.failure?.error;
    this.clearTimer();
    const input = this.queue.at(-1);
    if (input?.mode === 'pointer' && input.held) throw new Error('请先完成区域调整');
    if (input) input.held = false;
    while (!this.disposed) {
      if (this.flight) { await this.flight; continue; }
      if (this.cleanup) { await this.submit(); continue; }
      if (this.failure || existingFailure || this.failureSerial !== failureSerial) {
        const error = this.failure?.error ?? this.lastFailure ?? existingFailure!;
        if (!this.failure?.held) { this.failure = undefined; this.publish(); }
        throw error;
      }
      this.prune();
      const value = this.queue[0];
      if (!value) { this.publish(); return; }
      if (value.held && !this.dirty(value)) await new Promise<void>(resolve => this.waiters.add(resolve));
      else await this.submit();
    }
  }
  pauseInput(paused: boolean) {
    this.paused = paused;
    if (paused) {
      const value = this.queue.at(-1);
      if (value?.mode === 'pointer' && value.held) value.latest = { ...value.target.geometry };
      if (value) value.held = false;
      if (this.failure) this.failure.held = false;
      this.prune(); this.publish();
    }
  }
  reconcile() {
    const value = this.queue[0]; if (!value || this.disposed) return;
    const current = this.hooks.current(value.target.sceneId, value.target.regionId);
    if (!current || !sameIdentity(value.target, current)) { this.invalidate(); return; }
    if (!this.flight && (current.revision !== value.target.revision || !sameGeometry(current.geometry, value.target.geometry))) {
      this.fail(new Error('区域位置已变化'), value.attempted ? { sceneId: value.target.sceneId, editId: value.editId } : undefined);
      this.schedule();
    }
  }
  invalidate() { const value = this.queue[0]; if (value?.attempted) this.cleanup = { sceneId: value.target.sceneId, editId: value.editId }; this.epoch++; this.clearTimer(); this.queue = []; this.failure = undefined; this.publish(); this.schedule(); }
  dispose() { this.disposed = true; this.epoch++; this.clearTimer(); this.queue = []; this.cleanup = undefined; this.wake(); this.endOperation?.(); this.endOperation = undefined; }
}
