// SPDX-License-Identifier: MPL-2.0
export interface ObjectTrashPoint {x:number;y:number}
export interface ObjectTrashPort {
  begin:()=>void;move:(point:ObjectTrashPoint)=>boolean;
  finish:(point:ObjectTrashPoint)=>boolean;cancel:()=>void;
}
/** Pointer capture keeps events on the dragged object; use the target's live bounds. */
export function objectTrash(target:()=>HTMLElement|undefined,disabled:()=>boolean,changed:(dragging:boolean,over:boolean)=>void):ObjectTrashPort {
  let dragging=false;
  const hit=(point:ObjectTrashPoint)=>{const element=target(),box=element?.getBoundingClientRect();return Boolean(dragging&&!disabled()&&box&&box.width>0&&box.height>0&&Number.isFinite(point.x)&&Number.isFinite(point.y)&&point.x>=Math.max(0,box.left)&&point.x<Math.min(innerWidth,box.right)&&point.y>=Math.max(0,box.top)&&point.y<Math.min(innerHeight,box.bottom));};
  const cancel=()=>{dragging=false;changed(false,false);};
  return {begin:()=>{dragging=true;changed(true,false);},move:point=>{const over=hit(point);changed(dragging,over);return over;},finish:point=>{const over=hit(point);cancel();return over;},cancel};
}
