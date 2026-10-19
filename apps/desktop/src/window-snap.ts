// SPDX-License-Identifier: MPL-2.0
export interface SnapBounds { x: number; y: number; width: number; height: number }
export interface SnapNode { id: number; bounds: SnapBounds; selectable: boolean; snapshot?: boolean; children: SnapNode[] }
export interface FrameSnapMap { version: 1; backgroundId: string; width: number; height: number; roots: SnapNode[] }
export interface SnapMapRequest { sceneId: string; backgroundId: string; width: number; height: number }
export type SnapHandle = 'nw' | 'n' | 'ne' | 'w' | 'e' | 'sw' | 's' | 'se';
interface Point { x: number; y: number }
const integer = (value: unknown): value is number => typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 0xffff_ffff;
const record = (value: unknown): value is Record<string, unknown> => Boolean(value && typeof value === 'object' && !Array.isArray(value));
const keys = (value: Record<string, unknown>, expected: string[]) => Object.keys(value).length === expected.length && expected.every(key => Object.hasOwn(value, key));
export const snapContains = (box: SnapBounds, x: number, y: number) => x >= box.x && y >= box.y && x < box.x + box.width && y < box.y + box.height;
const within = (box: SnapBounds, parent: SnapBounds) => box.x >= parent.x && box.y >= parent.y && box.x + box.width <= parent.x + parent.width && box.y + box.height <= parent.y + parent.height;

/** Reject the entire advisory map on malformed data; manual selection remains available. */
export function validateSnapMap(value: unknown, request: SnapMapRequest): FrameSnapMap | undefined {
  if (!record(value) || !keys(value, ['version', 'backgroundId', 'width', 'height', 'roots']) || value.version !== 1 || value.backgroundId !== request.backgroundId || value.width !== request.width || value.height !== request.height || !integer(value.width) || !integer(value.height) || !value.width || !value.height || value.width > 16384 || value.height > 16384 || value.width * value.height > 32 * 1024 * 1024 || !Array.isArray(value.roots) || value.roots.length > 256) return;
  const seen = new Set<number>(), background = { x: 0, y: 0, width: value.width, height: value.height };
  let count = 0;
  const parse = (nodes: unknown[], parent: SnapBounds, depth: number): SnapNode[] | undefined => {
    if (depth > 12 || nodes.length > 2048) return;
    const result: SnapNode[] = [];
    for (const node of nodes) {
      if (++count > 2048 || !record(node) || !(keys(node, ['id', 'bounds', 'selectable', 'children']) || keys(node, ['id', 'bounds', 'selectable', 'snapshot', 'children'])) || !integer(node.id) || seen.has(node.id) || typeof node.selectable !== 'boolean' || (node.snapshot !== undefined && (typeof node.snapshot !== 'boolean' || node.snapshot && !node.selectable)) || !Array.isArray(node.children) || !record(node.bounds) || !keys(node.bounds, ['x', 'y', 'width', 'height'])) return;
      const b = node.bounds;
      if (![b.x, b.y, b.width, b.height].every(integer) || !b.width || !b.height) return;
      const bounds = { x: b.x, y: b.y, width: b.width, height: b.height } as SnapBounds;
      if (!within(bounds, parent)) return;
      if (depth === 1 && !node.selectable && node.children.length) return;
      seen.add(node.id);
      const children = node.children.length ? parse(node.children, bounds, depth + 1) : [];
      if (!children) return;
      result.push({ id: node.id, bounds, selectable: node.selectable, ...(node.snapshot === undefined ? {} : { snapshot: node.snapshot as boolean }), children });
    }
    return result;
  };
  const roots = parse(value.roots, background, 1);
  if (!roots) return;
  const map: FrameSnapMap = { version: 1, backgroundId: request.backgroundId, width: value.width, height: value.height, roots };
  if (new TextEncoder().encode(JSON.stringify(map)).byteLength > 256 * 1024) return;
  return map;
}

/** The first occluding branch owns the point, even when it provides no candidate. */
export function snapTarget(map: FrameSnapMap | undefined, x: number, y: number): SnapNode | undefined {
  if (!map || !Number.isFinite(x) || !Number.isFinite(y)) return;
  let node = map.roots.find(value => snapContains(value.bounds, x, y));
  if (!node?.selectable) return;
  let selected: SnapNode = node;
  while (node) {
    const child: SnapNode | undefined = node.children.find(value => snapContains(value.bounds, x, y));
    if (!child) break;
    if (child.selectable) selected = child;
    node = child;
  }
  return selected;
}

const sameBounds = (a: SnapBounds, b: SnapBounds) => a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
/** This is an advisory UI association, never a native window capability. */
export class SnapshotSelections {
  private identity = '';
  private selected = new Map<string, { node: number; bounds: SnapBounds }>();
  sync(sceneId: string, backgroundId: string | undefined, regions: { id: string }[]) {
    const identity = JSON.stringify([sceneId, backgroundId]);
    if (identity !== this.identity) { this.identity = identity; this.selected.clear(); }
    const ids = new Set(regions.map(region => region.id));
    for (const id of this.selected.keys()) if (!ids.has(id)) this.selected.delete(id);
  }
  bind(region: SnapBounds & { id: string }, node: SnapNode | undefined) {
    if (node?.snapshot && node.selectable && sameBounds(region, node.bounds) && this.selected.size < 2048) this.selected.set(region.id, { node: node.id, bounds: { ...node.bounds } });
  }
  get(region: SnapBounds & { id: string; imageOverride?: unknown }, map: FrameSnapMap | undefined): number | undefined {
    const saved = this.selected.get(region.id);
    if (!saved || !map || region.imageOverride || !sameBounds(region, saved.bounds)) return;
    const find = (nodes: SnapNode[]): SnapNode | undefined => { for (const node of nodes) { if (node.id === saved.node) return node; const found = find(node.children); if (found) return found; } };
    const current = find(map.roots);
    return current?.snapshot && current.selectable && sameBounds(current.bounds, saved.bounds) ? current.id : undefined;
  }
}

export function snapImagePoint(x: number, y: number, image: SnapBounds, scale: number): Point | undefined {
  if (!Number.isFinite(scale) || scale <= 0 || !snapContains(image, x, y)) return;
  return { x: (x - image.x) / scale, y: (y - image.y) / scale };
}

/** Only moved edges snap. The source bounds and opposite edges remain authoritative. */
export function snapResize(value: SnapBounds, handle: SnapHandle, target: SnapBounds | undefined, scale: number, width: number, height: number, minimum: number): SnapBounds {
  if (!target || !Number.isFinite(scale) || scale <= 0) return value;
  const threshold = 9 / scale;
  const nearest = (value: number, first: number, second: number) => Math.abs(value - first) <= threshold && Math.abs(value - first) <= Math.abs(value - second) ? first : Math.abs(value - second) <= threshold ? second : value;
  let left = value.x, top = value.y, right = value.x + value.width, bottom = value.y + value.height;
  if (handle.includes('w')) left = nearest(left, target.x, target.x + target.width);
  if (handle.includes('e')) right = nearest(right, target.x, target.x + target.width);
  if (handle.includes('n')) top = nearest(top, target.y, target.y + target.height);
  if (handle.includes('s')) bottom = nearest(bottom, target.y, target.y + target.height);
  if (right - left < minimum || bottom - top < minimum) return value;
  if (handle.includes('w')) left = Math.max(0, Math.min(left, right - minimum));
  if (handle.includes('e')) right = Math.min(width, Math.max(right, left + minimum));
  if (handle.includes('n')) top = Math.max(0, Math.min(top, bottom - minimum));
  if (handle.includes('s')) bottom = Math.min(height, Math.max(bottom, top + minimum));
  return { x: left, y: top, width: right - left, height: bottom - top };
}

/** Keep the down candidate advisory until release; dragging never flashes its full bounds. */
export class SnapSelectionGesture {
  private candidate?: SnapBounds;
  private automatic = false;
  constructor(private readonly start: Point, candidate?: SnapBounds) { this.candidate = candidate && { ...candidate }; }
  get isAutomatic() { return this.automatic; }
  forceManual() { this.candidate = undefined; this.automatic = false; }
  move(point: Point, manual = false, rawPoint: Point = point): SnapBounds {
    if (manual || Math.abs(rawPoint.x - this.start.x) > 4 || Math.abs(rawPoint.y - this.start.y) > 4) this.forceManual();
    return { x: Math.min(this.start.x, point.x), y: Math.min(this.start.y, point.y), width: Math.abs(point.x - this.start.x), height: Math.abs(point.y - this.start.y) };
  }
  finish(point: Point): SnapBounds {
    const manual = this.move(point);
    this.automatic = Boolean(this.candidate);
    return this.candidate ?? manual;
  }
}

const sameRequest = (a: SnapMapRequest, b: SnapMapRequest) => a.sceneId === b.sceneId && a.backgroundId === b.backgroundId && a.width === b.width && a.height === b.height;
interface Slot { request: SnapMapRequest; generation: number; status: 'waiting' | 'running' | 'done' }
type MapObservation = (outcome: 0 | 1 | 2 | 3, count: number) => void;
/** One real invocation plus one latest identity; cancellation never frees an active slot early. */
export class SnapMapLoader {
  private generation = 0;
  private current?: Slot;
  private running?: Slot;
  private disposed = false;
  constructor(private readonly fetch: (request: SnapMapRequest) => Promise<unknown>, private readonly changed: (value: FrameSnapMap | undefined) => void,
    private readonly observation?: () => MapObservation | undefined) {}
  request(request: SnapMapRequest) {
    if (this.disposed || this.current && sameRequest(this.current.request, request)) return;
    this.current = { request: { ...request }, generation: ++this.generation, status: 'waiting' };
    this.changed(undefined); this.pump();
  }
  cancel() { this.generation++; this.current = undefined; this.changed(undefined); }
  dispose() { this.disposed = true; this.cancel(); }
  private pump() {
    const slot = this.current;
    if (this.disposed || this.running || !slot || slot.status !== 'waiting') return;
    this.running = slot; slot.status = 'running';
    let promise: Promise<unknown>;
    let failed = false;
    try { promise = this.fetch(slot.request); } catch { failed = true; promise = Promise.resolve(null); }
    void Promise.resolve(promise).then(value => this.complete(slot, value, failed), () => this.complete(slot, null, true));
  }
  private complete(slot: Slot, value: unknown, failed: boolean) {
    if (this.running === slot) this.running = undefined;
    if (!this.disposed && this.current === slot && slot.generation === this.generation) {
      slot.status = 'done';
      const map = validateSnapMap(value, slot.request);
      try { this.observation?.()?.(failed ? 3 : map ? 0 : value == null ? 1 : 2, map?.roots.length ?? 0); }
      catch { /* Advisory observation must not replace a map or its failure. */ }
      this.changed(map);
    }
    this.pump();
  }
}
