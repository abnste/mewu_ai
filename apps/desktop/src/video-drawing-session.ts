// SPDX-License-Identifier: MPL-2.0
import type { Drawing, Snapshot, SpaceItem } from './contracts';
import type { TimedVideoAnnotation, VideoAnnotationTarget, VideoPixelPoint } from './video-annotation-contracts';
import type { VideoDrawingAction, VideoVectorLayoutRef } from './video-drawing-contracts';
import { validVideoVectorReference, videoDrawingUuid } from './video-drawing-wire';
import {drawingDrafts,type PendingDrawingDraft} from "./components/drawing-properties";
export function listVideoDrawingDrafts(sceneId?:string):PendingDrawingDraft[]{return drawingDrafts.list(sceneId).filter(entry=>{try{const key=JSON.parse(entry.key);return Array.isArray(key)&&key[1]==='videoDrawing';}catch{return false;}});}
export function assertVideoDrawingDraftsSaved(sceneId?:string):void{if(listVideoDrawingDrafts(sceneId).length)throw Error('视频绘制尚未保存');}
const stable=(v:unknown):string=>JSON.stringify(v,(_key,value)=>value&&typeof value==='object'&&!Array.isArray(value)?Object.fromEntries(Object.keys(value).sort().map(key=>[key,value[key]])):value);
export const sameVideoDrawingValue=(a:unknown,b:unknown)=>stable(a)===stable(b);
export const videoDrawingSourceKey=(sceneId:string,item:SpaceItem)=>JSON.stringify([sceneId,'videoDrawing',item.id,item.asset]);
export const videoDrawingTarget=(sceneId:string,item:SpaceItem):VideoAnnotationTarget=>({sceneId,itemId:item.id,sourceId:item.asset.id,expectedRangeRevision:item.videoEdit?.revision??0,expectedAnnotationRevision:item.videoAnnotations?.revision??0});
export const visibleVideoVectorBounds=(topLeft:VideoPixelPoint,ref:VideoVectorLayoutRef,size:{width:number;height:number})=>{
  const x=Math.max(0,topLeft.x),y=Math.max(0,topLeft.y),right=Math.min(size.width,topLeft.x+ref.width),bottom=Math.min(size.height,topLeft.y+ref.height);
  if(!Number.isInteger(topLeft.x)||!Number.isInteger(topLeft.y)||right<=x||bottom<=y)throw Error('视频绘制对象无效');
  return{x,y,width:right-x,height:bottom-y};
};
/** Canonical integer source origin; local float points are never rounded. */
export function movedVideoVector(original:Drawing,topLeft:VideoPixelPoint,reference:VideoVectorLayoutRef,start:VideoPixelPoint,end:VideoPixelPoint,size:{width:number;height:number}):Drawing {
  if(!validVideoVectorReference(reference)||![topLeft.x,topLeft.y].every(Number.isInteger)||![start.x,start.y,end.x,end.y].every(Number.isFinite))throw Error('视频绘制对象无效');
  const g=reference.geometryBounds,loX=Math.ceil(-g.x),hiX=Math.floor(size.width-g.x-g.width),loY=Math.ceil(-g.y),hiY=Math.floor(size.height-g.y-g.height);
  if(loX>hiX||loY>hiY)throw Error('视频绘制对象越界');
  const x=Math.max(loX,Math.min(hiX,Math.floor(topLeft.x+end.x-start.x+.5))),y=Math.max(loY,Math.min(hiY,Math.floor(topLeft.y+end.y-start.y+.5)));
  return{...original,points:original.points.map(point=>({x:point.x+x-topLeft.x,y:point.y+y-topLeft.y}))};
}
function validObject(object:TimedVideoAnnotation,duration:number,size:{width:number;height:number}):boolean {
  if(!videoDrawingUuid(object.id)||object.primitive.kind!=='vector'||!validVideoVectorReference(object.primitive.layout)||object.interval.startTicks!==0||object.interval.endTicks!==duration||object.origin)return false;
  const {topLeft,layout}=object.primitive,g=layout.geometryBounds;
  return Number.isInteger(topLeft.x)&&Number.isInteger(topLeft.y)&&topLeft.x+g.x>=0&&topLeft.y+g.y>=0&&topLeft.x+g.x+g.width<=size.width&&topLeft.y+g.y+g.height<=size.height;
}
/** A publish arriving early is never a receipt. Only this exact accepted IPC snapshot maps an Add ID. */
export function acceptVideoDrawingReceipt(snapshot:Snapshot,sceneId:string,before:SpaceItem,action:VideoDrawingAction,source?:{durationTicks:number;width:number;height:number}):{item:SpaceItem;annotationId:string} {
  const scene=snapshot.scenes.find(value=>value.id===sceneId),item=scene?.items.find(value=>value.id===before.id),old=before.videoAnnotations,doc=item?.videoAnnotations;
  if(!item||item.asset.kind!=='video'||!sameVideoDrawingValue(item.asset,before.asset)||!sameVideoDrawingValue(item.videoEdit,before.videoEdit)||!doc||doc.sourceId!==before.asset.id||doc.clock!=='sourcePlaybackTicks'||doc.version!==1||old&&[doc.sourceDurationTicks,doc.sourceWidth,doc.sourceHeight].some((v,i)=>v!==[old.sourceDurationTicks,old.sourceWidth,old.sourceHeight][i]))throw Error('视频绘制回执不一致');
  if(source&&(doc.sourceDurationTicks!==source.durationTicks||doc.sourceWidth!==source.width||doc.sourceHeight!==source.height))throw Error('视频绘制源回执不一致');
  const prior=old?.objects??[],rev=old?.revision??0;
  if(action.type==='update'&&doc.revision===rev&&sameVideoDrawingValue(doc,old)){const existing=prior.find(value=>value.id===action.annotationId);if(existing?.primitive.kind!=='vector'||!sameVideoDrawingValue(existing.primitive.layout,action.reference))throw Error('视频绘制回执不一致');return{item,annotationId:action.annotationId};}
  if(doc.revision!==rev+1||doc.redo.length||!doc.undo.length)throw Error('视频绘制回执不一致');
  const last=doc.undo.at(-1)!;
  if(!sameVideoDrawingValue(last.before,prior)||!sameVideoDrawingValue(last.after,doc.objects))throw Error('视频绘制回执不一致');
  let annotationId:string;
  if(action.type==='add'){
    if(doc.objects.length!==prior.length+1||!sameVideoDrawingValue(doc.objects.slice(0,-1),prior))throw Error('视频绘制回执不一致');
    const added=doc.objects.at(-1)!;
    if(!videoDrawingUuid(added.id)||prior.some(value=>value.id===added.id)||added.origin||added.interval.startTicks!==0||added.interval.endTicks!==doc.sourceDurationTicks)throw Error('视频绘制回执不一致');
    if(action.content.kind==='text'){
      if(added.primitive.kind!=='text'||!sameVideoDrawingValue(added.primitive.topLeft,action.content.topLeft))throw Error('视频绘制回执不一致');
    }else if(!validObject(added,doc.sourceDurationTicks,{width:doc.sourceWidth,height:doc.sourceHeight}))throw Error('视频绘制回执不一致');
    annotationId=added.id;
  }else{
    if(doc.objects.length!==prior.length)throw Error('视频绘制回执不一致');
    const index=prior.findIndex(value=>value.id===action.annotationId),was=prior[index],now=doc.objects[index];
    if(index<0||was.primitive.kind!=='vector'||!sameVideoDrawingValue(was.primitive.layout,action.reference)||!now||now.id!==was.id||!sameVideoDrawingValue(now.origin,was.origin)||!sameVideoDrawingValue(now.interval,was.interval)||now.primitive.kind!=='vector'||!validVideoVectorReference(now.primitive.layout)||!doc.objects.every((value,i)=>i===index||sameVideoDrawingValue(value,prior[i])))throw Error('视频绘制回执不一致');
    annotationId=now.id;
  }
  return{item,annotationId};
}
