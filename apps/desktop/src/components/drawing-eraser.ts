// SPDX-License-Identifier: MPL-2.0
import type { Drawing, DrawingPoint, DrawingStrokeErase } from '../contracts';
export interface EraserPoint { x: number; y: number }
export type EraserMode = 'pixel' | 'object';
export const pixelErasable = (drawing: Drawing) => ['pen', 'highlighter', 'line'].includes(drawing.kind);

type Interval = [number, number];
const lerp = (a: DrawingPoint,b: DrawingPoint,t: number):DrawingPoint => ({x:a.x+(b.x-a.x)*t,y:a.y+(b.y-a.y)*t});
const equalPoint = (a: DrawingPoint,b: DrawingPoint) => Math.abs(a.x-b.x)<1e-8&&Math.abs(a.y-b.y)<1e-8;
/** Exact intersections of a line segment with the swept circular brush capsule.
 * A capsule is the union of its oriented rectangle and two end disks. No
 * pointer-distance sampling is used, so fast sweeps and single dots work too.
 */
function erasedIntervals(a:DrawingPoint,b:DrawingPoint,from:EraserPoint,to:EraserPoint,radius:number):Interval[] {
  const intervals:Interval[]=[],dx=b.x-a.x,dy=b.y-a.y,length2=dx*dx+dy*dy;
  const add=(lo:number,hi:number)=>{lo=Math.max(0,lo);hi=Math.min(1,hi);if(hi>lo+1e-12)intervals.push([lo,hi]);};
  const disk=(c:EraserPoint)=>{
    const x=a.x-c.x,y=a.y-c.y;
    if(length2===0){if(x*x+y*y<=radius*radius)intervals.push([0,1]);return;}
    const h=x*dx+y*dy,d=h*h-length2*(x*x+y*y-radius*radius);
    if(d>=0){const root=Math.sqrt(d);add((-h-root)/length2,(-h+root)/length2);}
  };
  disk(from);
  const sx=to.x-from.x,sy=to.y-from.y,length=Math.hypot(sx,sy);
  if(length){
    disk(to);
    const ux=sx/length,uy=sy/length;
    let lo=0,hi=1;
    for(const [p,v,min,max] of [[(a.x-from.x)*ux+(a.y-from.y)*uy,dx*ux+dy*uy,0,length],[-(a.x-from.x)*uy+(a.y-from.y)*ux,-dx*uy+dy*ux,-radius,radius]]){
      if(v===0){if(p<min||p>max){hi=-1;break;}}
      else {const t0=(min-p)/v,t1=(max-p)/v;lo=Math.max(lo,Math.min(t0,t1));hi=Math.min(hi,Math.max(t0,t1));}
    }
    if(hi>lo)add(lo,hi);
  }
  intervals.sort((a,b)=>a[0]-b[0]);
  const merged:Interval[]=[];for(const part of intervals){const last=merged.at(-1);if(last&&part[0]<=last[1]+1e-12)last[1]=Math.max(last[1],part[1]);else merged.push(part);}
  return merged;
}
/** Cut a stroke's centerline, reserving its round-cap radius at the erased edge.
 * Retained pieces are normal Drawing geometry, shared by SVG and native export.
 */
export function eraseStroke(points:readonly DrawingPoint[],from:EraserPoint,to:EraserPoint,diameter:number,strokeWidth:number):DrawingPoint[][] {
  if(!points.length||![from.x,from.y,to.x,to.y,diameter,strokeWidth].every(Number.isFinite)||diameter<=0||strokeWidth<=0)throw Error('擦除几何无效');
  const radius=(diameter+strokeWidth)/2;
  if(points.length===1)return pointDistance(points[0],from,to)<=radius?[]:[points.map(p=>({...p}))];
  const result:DrawingPoint[][]=[];let chain:DrawingPoint[]=[];
  const flush=()=>{if(chain.length)result.push(chain);chain=[];};
  for(let i=1;i<points.length;i++){
    const a=points[i-1],b=points[i],intervals=erasedIntervals(a,b,from,to,radius);
    if(equalPoint(a,b)){if(pointDistance(a,from,to)<=radius)flush();else if(!chain.length)chain=[{...a}];continue;}
    let cursor=0;
    const keep=(lo:number,hi:number)=>{if(hi-lo<=1e-12)return;const first=lerp(a,b,lo),last=lerp(a,b,hi);if(chain.length&&!equalPoint(chain.at(-1)!,first))flush();if(!chain.length)chain.push(first);if(!equalPoint(chain.at(-1)!,last))chain.push(last);};
    for(const [lo,hi] of intervals){keep(cursor,lo);flush();cursor=hi;}
    keep(cursor,1);
  }
  flush();return result;
}

/** One gesture owns one immutable source revision and one atomic history step. */
export class PixelEraser {
  private parts=new Map<string,{original:Drawing;values:Drawing[]}>();
  private objectCount:number;
  constructor(private originals:readonly Drawing[],private nextId:()=>string){this.objectCount=originals.length;}
  sweep(from:EraserPoint,to:EraserPoint,diameter:number):boolean {
    let changed=false,count=this.objectCount;
    const pending=new Map(this.parts);
    for(const original of this.originals){
      if(!pixelErasable(original))continue;
      const previous=this.parts.get(original.id)?.values??[original],next:Drawing[]=[];
      let hit=false;
      for(const drawing of previous){
        if(!eraserStrokeHits([drawing],from,to,diameter).length){next.push(drawing);continue;}
        const fragments=eraseStroke(drawing.points,from,to,diameter,drawing.strokeWidth);
        if(fragments.length===1&&fragments[0].length===drawing.points.length&&fragments[0].every((p,i)=>equalPoint(p,drawing.points[i]))){next.push(drawing);continue;}
        hit=true;
        for(let i=0;i<fragments.length;i++){
          if(fragments[i].length>4096)throw Error('笔迹片段过多，请分次擦除');
          next.push({...drawing,id:i===0?drawing.id:this.nextId(),points:fragments[i]});
        }
      }
      if(!hit)continue;
      count=count-previous.length+next.length;
      // The first surviving piece keeps the original identity even if the old
      // first fragment was completely erased during this same drag.
      if(next.length&&next[0].id!==original.id)next[0]={...next[0],id:original.id};
      pending.set(original.id,{original,values:next});changed=true;
    }
    if(count>256)throw Error('笔迹片段过多，请分次擦除');
    this.objectCount=count;this.parts=pending;
    return changed;
  }
  preview():Map<string,Drawing[]> {return new Map([...this.parts].map(([id,value])=>[id,value.values]));}
  erasures():DrawingStrokeErase[] {return [...this.parts].map(([drawingId,value])=>({drawingId,fragments:value.values.map(d=>({id:d.id,points:d.points}))}));}
}

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
