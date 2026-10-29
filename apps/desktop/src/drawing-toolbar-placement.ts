// SPDX-License-Identifier: MPL-2.0
export interface ToolbarRect { x:number; y:number; width:number; height:number }
export type ToolbarEdge = 'top' | 'bottom';
export interface ToolbarPlacement extends ToolbarRect { overlaps:boolean; edge:ToolbarEdge }
const clamp=(n:number,min:number,max:number)=>Math.max(min,Math.min(max,n));
const intersects=(a:ToolbarRect,b:ToolbarRect)=>a.x<b.x+b.width&&a.x+a.width>b.x&&a.y<b.y+b.height&&a.y+a.height>b.y;

/** Try the free sides before shifting into the viewport. Never force an
 * oversized selection's controls onto its top annotation area. */
export function placeDrawingToolbar(box:ToolbarRect,size:{width:number;height:number},viewport:{width:number;height:number},rightDocked=false,edge:ToolbarEdge='bottom'):ToolbarPlacement {
  const margin=6,gap=8;
  const x=clamp(rightDocked?box.x+box.width-size.width:box.x,margin,Math.max(margin,viewport.width-size.width-margin));
  const y=clamp(box.y,margin,Math.max(margin,viewport.height-size.height-margin));
  const rect=(x:number,y:number)=>({x,y,width:size.width,height:size.height});
  const candidates=[rect(x,box.y-size.height-gap),rect(x,box.y+box.height+gap),rect(box.x+box.width+gap,y),rect(box.x-size.width-gap,y)];
  const free=candidates.find(r=>r.x>=margin&&r.y>=margin&&r.x+r.width<=viewport.width-margin&&r.y+r.height<=viewport.height-margin&&!intersects(r,box));
  if(free)return {...free,overlaps:false,edge};
  return {...rect(x,edge==='top'?margin:Math.max(margin,viewport.height-size.height-gap)),overlaps:true,edge};
}

/** Switch only during a drawing gesture. Hovering or approaching a tool must
 * keep it clickable rather than making the toolbar run away from the mouse. */
export function avoidDrawingToolbar(current:ToolbarPlacement,point:{x:number;y:number},box:ToolbarRect,viewport:{width:number;height:number},rightDocked=false):ToolbarEdge {
  if(!current.overlaps)return current.edge;
  const near=(r:ToolbarRect)=>point.x>=r.x-20&&point.x<=r.x+r.width+20&&point.y>=r.y-20&&point.y<=r.y+r.height+20;
  if(!near(current))return current.edge;
  const edge=current.edge==='top'?'bottom':'top';
  const other=placeDrawingToolbar(box,current,viewport,rightDocked,edge);
  return near(other)?current.edge:edge;
}

/** Center a tool's bubble on its button; clamp only at viewport boundaries. */
export function placeDrawingProperties(anchor:ToolbarRect,size:{width:number;height:number},viewport:{width:number;height:number},side:'top'|'bottom'|'left'|'right') {
  const gap=8,margin=6;
  const x=side==='left'?anchor.x-size.width-gap:side==='right'?anchor.x+anchor.width+gap:anchor.x+(anchor.width-size.width)/2;
  const y=side==='top'?anchor.y-size.height-gap:side==='bottom'?anchor.y+anchor.height+gap:anchor.y+(anchor.height-size.height)/2;
  return {x:clamp(x,margin,Math.max(margin,viewport.width-size.width-margin)),y:clamp(y,margin,Math.max(margin,viewport.height-size.height-margin))};
}
