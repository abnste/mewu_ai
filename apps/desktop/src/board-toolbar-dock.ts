// SPDX-License-Identifier: MPL-2.0
export type BoardDock = 'top' | 'right' | 'bottom' | 'left' | 'free';
export interface BoardToolbarPosition { edge: BoardDock; x: number; y: number }
const clamp = (n:number,min:number,max:number) => Math.max(min,Math.min(max,n));
export function dockBoardToolbar(position:BoardToolbarPosition,size:{width:number;height:number},viewport:{width:number;height:number}) {
  const x=clamp(position.x,6,Math.max(6,viewport.width-size.width-6));
  const y=clamp(position.y,6,Math.max(6,viewport.height-size.height-6));
  switch(position.edge) {
    case 'top': return {x,y:8};
    case 'bottom': return {x,y:Math.max(6,viewport.height-size.height-16)};
    case 'left': return {x:8,y};
    case 'right': return {x:Math.max(6,viewport.width-size.width-8),y};
    default: return {x,y};
  }
}
export function boardDockAt(point:{x:number;y:number},viewport:{width:number;height:number}):BoardDock {
  const edges=[['left',point.x],['right',viewport.width-point.x],['top',point.y],['bottom',viewport.height-point.y]] as const;
  const nearest=[...edges].sort((a,b)=>a[1]-b[1])[0];
  return nearest[1]<=36?nearest[0]:'free';
}

export function boardDockPreview(position:BoardToolbarPosition,size:{width:number;height:number},viewport:{width:number;height:number},edge:BoardDock,vertical:boolean) {
  if(edge==='free')return undefined;
  const nextVertical=edge==='left'||edge==='right';
  const nextSize=nextVertical===vertical?size:{width:size.height,height:size.width};
  const width=Math.min(nextSize.width,viewport.width-16),height=Math.min(nextSize.height,viewport.height-16);
  return {...dockBoardToolbar({...position,edge},{width,height},viewport),width,height,edge};
}
