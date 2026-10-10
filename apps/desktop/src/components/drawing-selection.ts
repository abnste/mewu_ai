// SPDX-License-Identifier: MPL-2.0
import type {Drawing,DrawingPoint} from '../contracts';
import {drawingBounds,drawingMoveDelta} from './drawing-geometry';
import type {EditorBox} from '../drawing-editor-port';
export const selectionBox=(a:DrawingPoint,b:DrawingPoint):EditorBox=>({x:Math.min(a.x,b.x),y:Math.min(a.y,b.y),width:Math.abs(b.x-a.x),height:Math.abs(b.y-a.y)});
export function marqueeDrawings(drawings:Drawing[],box:EditorBox,canSelect:(d:Drawing)=>boolean):string[]{
  return drawings.filter(d=>{if(!canSelect(d))return false;const b=drawingBounds(d);return b.x<=box.x+box.width&&b.y<=box.y+box.height&&b.x+b.width>=box.x&&b.y+b.height>=box.y;}).map(d=>d.id);
}
export function movedDrawings(drawings:Drawing[],dx:number,dy:number,frame:EditorBox,source:{width:number;height:number}):Drawing[]{
  const ink=drawings.filter(d=>d.rich?.kind!=='extracted');
  if(ink.length){const boxes=ink.map(drawingBounds),x=Math.min(...boxes.map(b=>b.x)),y=Math.min(...boxes.map(b=>b.y)),right=Math.max(...boxes.map(b=>b.x+b.width)),bottom=Math.max(...boxes.map(b=>b.y+b.height));dx=drawingMoveDelta(dx,x,right,frame.x,frame.x+frame.width,source.width);dy=drawingMoveDelta(dy,y,bottom,frame.y,frame.y+frame.height,source.height);}
  return drawings.map(d=>({...d,points:d.points.map(p=>({x:p.x+dx,y:p.y+dy}))}));
}
/** Repair previews follow the very same transform that core saves in one step. */
export function linkedRepairPreview(d:Drawing,drawings:Map<string,Drawing>,preview:Map<string,Drawing>):Drawing {
  const parentId=d.rich?.parentId;if(!parentId)return d;
  const old=drawings.get(parentId),next=preview.get(parentId);if(!old||!next)return d;
  const [a,b]=old.points,[c,e]=next.points;if(b.x<=a.x||b.y<=a.y)return d;
  return {...d,points:d.points.map(p=>({x:c.x+(p.x-a.x)*(e.x-c.x)/(b.x-a.x),y:c.y+(p.y-a.y)*(e.y-c.y)/(b.y-a.y)}))};
}
