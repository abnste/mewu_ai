// SPDX-License-Identifier: MPL-2.0
import type { Drawing } from '../contracts';
export interface EraserPoint { x: number; y: number }

export function clipEraserSweep(from:EraserPoint,to:EraserPoint,box:{x:number;y:number;width:number;height:number}): [EraserPoint,EraserPoint] | undefined {
  const dx=to.x-from.x,dy=to.y-from.y;let low=0,high=1;
  if(![from.x,from.y,to.x,to.y,box.x,box.y,box.width,box.height].every(Number.isFinite)||box.width<=0||box.height<=0)return;
  for(const [p,q] of [[-dx,from.x-box.x],[dx,box.x+box.width-from.x],[-dy,from.y-box.y],[dy,box.y+box.height-from.y]]){
    if(p===0){if(q<0)return;continue;}
    const ratio=q/p;if(p<0)low=Math.max(low,ratio);else high=Math.min(high,ratio);if(low>high)return;
  }
  return [{x:from.x+dx*low,y:from.y+dy*low},{x:from.x+dx*high,y:from.y+dy*high}];
}

function pointDistance(p: EraserPoint, a: EraserPoint, b: EraserPoint) {
  const dx = b.x-a.x, dy = b.y-a.y, length = dx*dx+dy*dy;
  const t = length ? Math.max(0,Math.min(1,((p.x-a.x)*dx+(p.y-a.y)*dy)/length)) : 0;
  return Math.hypot(p.x-a.x-t*dx,p.y-a.y-t*dy);
}
function segmentDistance(a: EraserPoint,b: EraserPoint,c: EraserPoint,d: EraserPoint) {
  const cross = (p:EraserPoint,q:EraserPoint,r:EraserPoint)=>(q.x-p.x)*(r.y-p.y)-(q.y-p.y)*(r.x-p.x);
  const abC=cross(a,b,c),abD=cross(a,b,d),cdA=cross(c,d,a),cdB=cross(c,d,b);
  if (((abC<0&&abD>0)||(abC>0&&abD<0)) && ((cdA<0&&cdB>0)||(cdA>0&&cdB<0))) return 0;
  return Math.min(pointDistance(a,c,d),pointDistance(b,c,d),pointDistance(c,a,b),pointDistance(d,a,b));
}
// A sweep is a capsule in drawing coordinates, including its initial disk.
// Do not sample a fast pointer path: even a one-point stroke must be hit.
export function eraserStrokeHits(drawings: readonly Drawing[], from: EraserPoint, to: EraserPoint, diameter: number): string[] {
  if (![from.x,from.y,to.x,to.y,diameter].every(Number.isFinite) || diameter<=0) return [];
  return drawings.filter(drawing=>{
    if (!['pen','highlighter','line'].includes(drawing.kind) || !drawing.points.length) return false;
    const radius=(diameter+drawing.strokeWidth)/2,points=drawing.points;
    if (points.length===1) return pointDistance(points[0],from,to)<=radius;
    for(let i=1;i<points.length;i++) if(segmentDistance(from,to,points[i-1],points[i])<=radius) return true;
    return false;
  }).map(drawing=>drawing.id);
}

export function eraserCursor(diameter: number, height = diameter) {
  // The ring is centered on the hotspot and has the actual on-screen diameter.
  const w=Math.max(5,Math.min(126,Math.ceil(diameter)+4)),h=Math.max(5,Math.min(126,Math.ceil(height)+4));
  const cx=Math.floor(w/2),cy=Math.floor(h/2),rx=Math.max(.5,Math.min(diameter,w-4)/2),ry=Math.max(.5,Math.min(height,h-4)/2);
  const svg=`<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}"><ellipse cx="${cx}" cy="${cy}" rx="${rx}" ry="${ry}" fill="none" stroke="white" stroke-width="2.5"/><ellipse cx="${cx}" cy="${cy}" rx="${rx}" ry="${ry}" fill="none" stroke="#34383f" stroke-width="1"/></svg>`;
  return `url("data:image/svg+xml,${encodeURIComponent(svg)}") ${cx} ${cy}, crosshair`;
}

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

// Hit incoming sweeps immediately, queue unique IDs (not pointer samples), and
// persist one at a time. A fast drag must not lose ink while the first IPC waits.
export class ObjectEraser {
  private active = true;
  private started = false;
  private running = false;
  private queue: string[] = [];
  private seen = new Set<string>();
  private ending = false;
  readonly finished: Promise<void>;
  private resolveFinished!: () => void;
  private last: EraserPoint;
  constructor(private initial: EraserPoint, private hooks: { current: () => boolean; hit: (point: EraserPoint, previous: EraserPoint) => string | readonly string[] | undefined; remove: (id: string) => Promise<boolean>; stopped: () => void }) { this.last = { ...initial }; this.finished=new Promise(resolve=>{this.resolveFinished=resolve;}); }
  start() { if (!this.active || this.started) return; this.started = true; this.sample(this.initial,this.initial); void this.drain(); }
  private sample(point: EraserPoint, previous: EraserPoint) {
    const hit=this.hooks.hit(point,previous),ids=typeof hit==='string'?[hit]:hit??[];
    for(const id of ids) if(!this.seen.has(id)){this.seen.add(id);this.queue.push(id);}
  }
  move(point: EraserPoint) {
    if (!this.active || this.ending || !this.hooks.current()) { if(!this.ending)this.stop(); return; }
    if (!Number.isFinite(point.x) || !Number.isFinite(point.y) || (point.x===this.last.x&&point.y===this.last.y)) return;
    // Advance before hit/removal: native pointer-capture/DOM resync can reenter.
    const previous=this.last; this.last = { ...point };
    if(this.started)this.sample(point,previous);
    if (this.started) void this.drain();
  }
  finish() { if(!this.active)return; this.ending=true; void this.drain(); }
  stop() { if (!this.active) return; this.active = false; this.queue=[]; this.hooks.stopped(); if(!this.running)this.resolveFinished(); }
  private async drain() {
    if (this.running || !this.active || !this.started) return;
    this.running = true;
    try {
      while (this.active && this.queue.length) {
        if (!this.hooks.current()) { this.stop(); break; }
        const id = this.queue.shift()!;
        if (!await this.hooks.remove(id)) { this.stop(); break; }
        if (!this.hooks.current()) { this.stop(); break; }
      }
    } catch { this.stop(); } // The command owner reports persistence failures.
    finally { this.running = false; if(this.ending)this.stop(); if(!this.active)this.resolveFinished(); }
  }
}
