// SPDX-License-Identifier: MPL-2.0
export interface EraserPoint { x: number; y: number }

// Capture retargets pointer events to the SVG. Ask the browser for the actual
// painted hit stack instead, preserving vector/mosaic order and clip paths.
export function eraserHit(svg: SVGSVGElement, point: EraserPoint, allowed: ReadonlySet<string>, elements?: Element[]): string | undefined {
  const box = svg.getBoundingClientRect();
  if (!Number.isFinite(point.x) || !Number.isFinite(point.y) || point.x < box.left || point.y < box.top || point.x >= box.right || point.y >= box.bottom) return;
  elements ??= document.elementsFromPoint(point.x, point.y);
  if (!elements.length || (elements[0] !== svg && !svg.contains(elements[0]))) return;
  for (const element of elements) {
    if (element === svg) break;
    if (!svg.contains(element) || element.closest('defs,clipPath,mask')) continue;
    const group = element.closest('[data-drawing-id]');
    const id = group?.getAttribute('data-drawing-id');
    if (group && svg.contains(group) && id && allowed.has(id)) return id;
  }
}

// One deletion per sampled CSS point, at most one IPC and one latest movement.
// Never re-hit merely because removing the top object changed the DOM below it.
export class ObjectEraser {
  private active = true;
  private started = false;
  private running = false;
  private latest?: EraserPoint;
  private last: EraserPoint;
  constructor(private initial: EraserPoint, private hooks: { current: () => boolean; hit: (point: EraserPoint) => string | undefined; remove: (id: string) => Promise<boolean>; stopped: () => void }) { this.last = { ...initial }; }
  start() { if (!this.active || this.started) return; this.started = true; this.latest = { ...this.initial }; void this.drain(); }
  move(point: EraserPoint) {
    if (!this.active || !this.hooks.current()) { this.stop(); return; }
    if (!Number.isFinite(point.x) || !Number.isFinite(point.y) || Math.hypot(point.x - this.last.x, point.y - this.last.y) < 12) return;
    // Advance before hit/removal: native pointer-capture/DOM resync can reenter.
    this.last = { ...point }; this.latest = { ...point };
    if (this.started) void this.drain();
  }
  stop() { if (!this.active) return; this.active = false; this.latest = undefined; this.hooks.stopped(); }
  private async drain() {
    if (this.running || !this.active || !this.started) return;
    this.running = true;
    try {
      while (this.active && this.latest) {
        if (!this.hooks.current()) { this.stop(); break; }
        const point = this.latest; this.latest = undefined;
        const id = this.hooks.hit(point);
        if (!id) continue;
        if (!await this.hooks.remove(id)) { this.stop(); break; }
        if (!this.hooks.current()) { this.stop(); break; }
      }
    } catch { this.stop(); } // The command owner reports persistence failures.
    finally { this.running = false; }
  }
}
