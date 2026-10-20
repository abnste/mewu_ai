// SPDX-License-Identifier: MPL-2.0
export type ObjectSource = { kind:'item';sceneId:string;itemId:string }
  | { kind:'region';sceneId:string;regionId:string } | { kind:'pin';id:string }
  | ({kind:'drawings';drawingIds:string[]} & DrawingSource);
export type DrawingSource = {sceneId:string;regionId:string;itemId?:never} | {sceneId:string;itemId:string;regionId?:never};
export interface ObjectPoint { x:number;y:number }
export interface ObjectDragSource { source:ObjectSource;remove:()=>void|Promise<void> }
export interface ObjectDrag { move(point:ObjectPoint):void;finish(point:ObjectPoint):boolean;cancel():void }
interface Receiver { begin(source:ObjectDragSource):ObjectDrag }
const receivers=new Set<Receiver>();
export function registerObjectDrop(receiver:Receiver):()=>void {
  receivers.add(receiver);return ()=>receivers.delete(receiver);
}
/** Registration order is stacking order. Only the top hit consumes a drop. */
export function beginObjectDrag(source:ObjectDragSource):ObjectDrag|undefined {
  const active=[...receivers].reverse().map(receiver=>({receiver,drag:receiver.begin(source)}));
  if(!active.length)return;
  let ended=false;
  return {
    move(point){if(!ended)for(const {receiver,drag} of active){if(receivers.has(receiver))drag.move(point);else drag.cancel();}},
    finish(point){
      if(ended)return false;ended=true;let consumed=false;
      for(const {receiver,drag} of active){
        if(!consumed&&receivers.has(receiver))consumed=drag.finish(point);
        else drag.cancel();
      }
      return consumed;
    },
    cancel(){if(!ended){ended=true;for(const {drag} of active)drag.cancel();}}
  };
}
