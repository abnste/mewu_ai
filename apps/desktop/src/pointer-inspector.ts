// SPDX-License-Identifier: MPL-2.0
// Pure source-coordinate geometry and a bounded latest-position request gate.
export interface PointerImageBox { x: number; y: number; width: number; height: number }
export interface PointerSampleRequest { sceneId: string; backgroundId: string; x: number; y: number }
export interface PointerSample extends PointerSampleRequest { globalX: number; globalY: number; hex: string; dataUrl: string }

/** Use the same rounded source edges as the committed screenshot rectangle. */
export function pointerSelectionDimensions(preview: PointerImageBox | undefined, image: { x: number; y: number; scale: number }, region?: { width: number; height: number; imageOverride?: { width?: number; height?: number } }): string | undefined {
  let width: number, height: number;
  if (preview) {
    if (![preview.x, preview.y, preview.width, preview.height, image.x, image.y, image.scale].every(Number.isFinite) || image.scale <= 0) return;
    width = Math.round((preview.x + preview.width - image.x) / image.scale) - Math.round((preview.x - image.x) / image.scale);
    height = Math.round((preview.y + preview.height - image.y) / image.scale) - Math.round((preview.y - image.y) / image.scale);
  } else if (region) {
    width = region.imageOverride?.width ?? region.width;
    height = region.imageOverride?.height ?? region.height;
  } else return;
  return Number.isSafeInteger(width) && Number.isSafeInteger(height) && width > 0 && height > 0 ? `${width} × ${height}` : undefined;
}

export const pointerInspectorSize = (hasDimensions: boolean) => ({ width: 90, height: hasDimensions ? 144 : 126 });

export function pointerImagePoint(clientX: number, clientY: number, box: PointerImageBox, image: { width: number; height: number }): { x: number; y: number } | undefined {
  if (![clientX, clientY, box.x, box.y, box.width, box.height, image.width, image.height].every(Number.isFinite)
    || box.width <= 0 || box.height <= 0 || !Number.isSafeInteger(image.width) || !Number.isSafeInteger(image.height)
    || image.width <= 0 || image.height <= 0) return;
  const scale = Math.min(box.width / image.width, box.height / image.height);
  if (!Number.isFinite(scale) || scale <= 0) return;
  const width = image.width * scale, height = image.height * scale;
  const left = box.x + (box.width - width) / 2, top = box.y + (box.height - height) / 2;
  if (![left, top, width, height].every(Number.isFinite)
    || clientX < left || clientY < top || clientX >= left + width || clientY >= top + height) return;
  return { x: Math.max(0, Math.min(image.width - 1, Math.round((clientX - left) / scale))),
    y: Math.max(0, Math.min(image.height - 1, Math.round((clientY - top) / scale))) };
}

export function pointerPopupPosition(x: number, y: number, viewportWidth: number, viewportHeight: number, size = { width: 90, height: 125 }): { left: number; top: number } | undefined {
  if (![x, y, viewportWidth, viewportHeight, size.width, size.height].every(Number.isFinite)
    || viewportWidth <= 0 || viewportHeight <= 0 || size.width <= 0 || size.height <= 0) return;
  const { width, height } = size, gap = 16, margin = 4;
  const left = x + gap + width <= viewportWidth - margin ? x + gap : x - gap - width;
  const top = y + gap + height <= viewportHeight - margin ? y + gap : y - gap - height;
  return { left: Math.max(margin, Math.min(Math.max(margin, viewportWidth - width - margin), left)),
    top: Math.max(margin, Math.min(Math.max(margin, viewportHeight - height - margin), top)) };
}

export class PointerSampleCanceledError extends Error {
  constructor() { super('取色位置已变化'); this.name = 'PointerSampleCanceledError'; }
}
interface Slot {
  request: PointerSampleRequest; generation: number; previewEpoch: number; status: 'waiting' | 'running' | 'done' | 'failed';
  value?: PointerSample;
  exact?: { promise: Promise<PointerSample>; resolve: (value: PointerSample) => void; reject: (error: Error) => void };
}
const same = (a: PointerSampleRequest, b: PointerSampleRequest) => a.sceneId === b.sceneId && a.backgroundId === b.backgroundId && a.x === b.x && a.y === b.y;
const sameSource = (a: PointerSampleRequest, b: PointerSampleRequest) => a.sceneId === b.sceneId && a.backgroundId === b.backgroundId;
function valid(request: PointerSampleRequest) {
  return typeof request.sceneId === 'string' && request.sceneId.length > 0 && typeof request.backgroundId === 'string' && request.backgroundId.length > 0
    && Number.isSafeInteger(request.x) && request.x >= 0 && Number.isSafeInteger(request.y) && request.y >= 0;
}
function validated(value: PointerSample, request: PointerSampleRequest): PointerSample {
  if (!value || !same(value, request) || !Number.isSafeInteger(value.globalX) || !Number.isSafeInteger(value.globalY)
    || typeof value.hex !== 'string' || !/^#[0-9a-f]{6}$/i.test(value.hex)
    || typeof value.dataUrl !== 'string' || value.dataUrl.length > 256 * 1024
    || !/^data:image\/png;base64,[A-Za-z0-9+/]+={0,2}$/.test(value.dataUrl)) throw new Error('取色结果与当前位置不一致');
  return Object.freeze({ ...value });
}

export class PointerInspector {
  private current?: Slot;
  private running?: Slot;
  private generation = 0;
  private previewEpoch = 0;
  private disposed = false;
  constructor(private readonly fetch: (request: PointerSampleRequest) => Promise<PointerSample>,
    private readonly onSample: (sample: PointerSample | undefined) => void,
    private readonly onError: (error: string) => void = () => {}) {}

  request(request: PointerSampleRequest): void {
    if (this.disposed) return;
    if (!valid(request)) { this.cancel(); this.onError('取色位置无效'); return; }
    this.select(request);
  }

  sampleExact(request: PointerSampleRequest): Promise<PointerSample> {
    if (this.disposed) return Promise.reject(new PointerSampleCanceledError());
    if (!valid(request)) { this.cancel(); return Promise.reject(new Error('取色位置无效')); }
    const slot = this.select(request);
    if (!this.active(slot)) return Promise.reject(new PointerSampleCanceledError());
    if (slot.value) return Promise.resolve(slot.value);
    if (!slot.exact) {
      let resolve!: (value: PointerSample) => void, reject!: (error: Error) => void;
      const promise = new Promise<PointerSample>((yes, no) => { resolve = yes; reject = no; });
      slot.exact = { promise, resolve, reject };
    }
    return slot.exact.promise;
  }

  cancel(): void {
    this.generation++; this.previewEpoch++;
    const old = this.current; this.current = undefined;
    old?.exact?.reject(new PointerSampleCanceledError());
    this.onSample(undefined);
    // An already-started native invocation retains its one slot until it ends.
  }
  dispose(): void { if (!this.disposed) { this.disposed = true; this.cancel(); } }

  private select(request: PointerSampleRequest): Slot {
    if (this.current && this.current.status !== 'failed' && same(this.current.request, request)) return this.current;
    const previous = this.current;
    const sourceChanged = !previous || !sameSource(previous.request, request);
    if (sourceChanged) this.previewEpoch++;
    const slot: Slot = { request: Object.freeze({ ...request }), generation: ++this.generation, previewEpoch: this.previewEpoch, status: 'waiting' };
    this.current = slot;
    previous?.exact?.reject(new PointerSampleCanceledError());
    // Keep the last completed preview while the pointer moves on this source.
    // Exact-copy still waits for the current slot's own coordinates.
    if (sourceChanged) this.onSample(undefined);
    this.pump();
    return slot;
  }
  private active(slot: Slot) { return !this.disposed && this.current === slot && slot.generation === this.generation; }
  private previewable(slot: Slot) {
    return !this.disposed && this.current && slot.previewEpoch === this.previewEpoch && sameSource(slot.request, this.current.request);
  }
  private pump(): void {
    const slot = this.current;
    if (this.disposed || this.running || !slot || slot.status !== 'waiting') return;
    this.running = slot; slot.status = 'running';
    let result: Promise<PointerSample>;
    try { result = this.fetch(slot.request); } catch (error) { result = Promise.reject(error); }
    void Promise.resolve(result).then(value => this.complete(slot, value), error => this.complete(slot, undefined, error));
  }
  private complete(slot: Slot, value?: PointerSample, failure?: unknown): void {
    if (this.running === slot) this.running = undefined;
    if (this.previewable(slot)) {
      try {
        if (failure !== undefined) throw failure;
        const sample = validated(value!, slot.request);
        slot.value = sample; slot.status = 'done';
        if (this.active(slot)) slot.exact?.resolve(sample);
        // A completed sample is useful during continuous movement. Only one
        // fetch runs; the next fetch always uses the latest queued position.
        this.onSample(sample);
      } catch (error) {
        slot.status = 'failed'; slot.value = undefined;
        const failure = error instanceof Error ? error : new Error(String(error));
        if (this.active(slot)) {
          slot.exact?.reject(failure);
          this.onSample(undefined); this.onError(failure.message);
        }
      }
    }
    this.pump();
  }
}
