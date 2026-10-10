// SPDX-License-Identifier: MPL-2.0
import {createEffect,createMemo,createSignal,on,onCleanup,type JSX} from 'solid-js';
import type {Drawing,Snapshot,SpaceItem} from '../contracts';
import type {VideoMetadata} from "../video-contracts";
import type {VideoAnnotationAction,VideoAnnotationTarget,VideoTextContent,VideoTextLayoutRef,VideoTextRead} from '../video-annotation-contracts';
import type {VideoDrawingAction,VideoDrawingGrant,VideoVectorLayoutRef,VideoVectorRead} from '../video-drawing-contracts';
import type {DrawingEditorPort,EditorBox,EditorDrawingAction} from '../drawing-editor-port';
import type {RegisterDrawingFlush} from "../drawing-flush";
import {draftChanged} from "./drawing-properties";
import {DrawingShape} from "./DrawingLayer";
import {getVideoAnnotationVector} from '../video-drawing-bridge';
import {drawingToVideoSource,localVectorDrawing,validateVideoVectorRead} from '../video-drawing-wire';
import {validateVideoTextRead} from '../video-manual-edit';
import {acceptVideoDrawingReceipt,movedVideoVector,sameVideoDrawingValue,videoDrawingSourceKey,videoDrawingTarget} from '../video-drawing-session';
import SharedDrawingEditor from './SharedDrawingEditor';
export interface VideoDrawingEditorProps {
  sceneId:string;item:SpaceItem;info:VideoMetadata;box:EditorBox;grant?:VideoDrawingGrant;
  busy:boolean;inputLocked:boolean;active:boolean;
  visibleAnnotationIds?:readonly string[];
  onPause:()=>void;onClose:()=>void;onError:(message:string)=>void;
  onRegisterDrawingAuthoringFlush?:RegisterDrawingFlush;
  onReadVector?:(requestId:string,target:VideoAnnotationTarget,id:string,ref:VideoVectorLayoutRef)=>Promise<VideoVectorRead>;
  onReadText:(requestId:string,target:VideoAnnotationTarget,id:string,ref:VideoTextLayoutRef)=>Promise<VideoTextRead>;
  onApplyDrawing:(requestId:string,target:VideoAnnotationTarget,grant:VideoDrawingGrant,action:VideoDrawingAction)=>Promise<Snapshot>;
  onDocument:(target:VideoAnnotationTarget,action:VideoAnnotationAction)=>Promise<Snapshot>;
  onEditText:(requestId:string,target:VideoAnnotationTarget,id:string,ref:VideoTextLayoutRef,content:VideoTextContent)=>Promise<Snapshot>;
  onSnapshot:(snapshot:Snapshot)=>void;
  /** Reuse the existing canonical native PNG layer, including integer source clipping. */
  renderStored:(id:string,interactive:boolean,selected:boolean,preview?:Drawing|EditorBox)=>JSX.Element;
  onOutput?:(copy:boolean,closeSpace:boolean)=>Promise<boolean>;
}
export default function VideoDrawingEditor(props:VideoDrawingEditorProps) {
  let disposed=false;
  const aliases=new Map<string,string>(),vectors=new Map<string,VideoVectorRead>(),texts=new Map<string,VideoTextRead>(),flights=new Map<string,Promise<Drawing|undefined>>();
  const [cacheVersion,setCacheVersion]=createSignal(0);
  const sourceKey=createMemo(()=>videoDrawingSourceKey(props.sceneId,props.item));
  const scope=createMemo(()=>JSON.stringify([sourceKey(),props.item.videoEdit??null,props.item.videoAnnotations??null,props.info]));
  const doc=()=>props.item.videoAnnotations;
  const target=()=>videoDrawingTarget(props.sceneId,props.item);
  const current=()=>!disposed&&props.active&&props.item.asset.kind==='video';
  const ready=current;
  const object=(id:string)=>doc()?.objects.find(value=>value.id===id);
  const descriptor=(id:string)=>{const value=vectors.get(id),stored=object(id);return value&&stored?.primitive.kind==='vector'&&sameVideoDrawingValue(stored.primitive.layout,value.reference)?value:undefined;};
  const report=(error:unknown)=>{if(!disposed)props.onError(error instanceof Error?error.message:String(error));};
  const bounds=(id:string):EditorBox=>{const p=object(id)?.primitive;if(!p)throw Error('视频绘制对象已变化');return p.kind==='rect'?{...p.bounds}:{...p.topLeft,width:p.layout.width,height:p.layout.height};};
  const projected=(id:string):Drawing|undefined=>{
    cacheVersion();const p=object(id)?.primitive;
    if(p?.kind==='vector'){const reply=descriptor(id);return reply&&localVectorDrawing(reply,p.topLeft);}
    if(p?.kind==='text'){const reply=texts.get(id);if(reply&&sameVideoDrawingValue(reply.reference,p.layout))return{id,kind:'text',points:[{...p.topLeft}],color:reply.content.color,strokeWidth:1,fontSize:reply.content.fontSize,text:reply.content.text};}
  };
  const values=()=>{cacheVersion();return (doc()?.objects??[]).flatMap(entry=>{const value=projected(entry.id);return value?[value]:[];});};
  const trimCache=()=>{let bytes=[...vectors.values()].reduce((sum,v)=>sum+new TextEncoder().encode(JSON.stringify(v.content)).length,0);while(vectors.size>8||bytes>2*1024*1024){const [id,value]=vectors.entries().next().value!;vectors.delete(id);bytes-=new TextEncoder().encode(JSON.stringify(value.content)).length;}while(texts.size>8)texts.delete(texts.keys().next().value!);};
  createEffect(on(sourceKey,()=>{vectors.clear();texts.clear();aliases.clear();setCacheVersion(v=>v+1);props.onPause();}));
  createEffect(on(scope,()=>{for(const [id,value]of vectors)if(object(id)?.primitive.kind!=='vector'||!sameVideoDrawingValue((object(id)!.primitive as {layout:VideoVectorLayoutRef}).layout,value.reference))vectors.delete(id);for(const [id,value]of texts)if(object(id)?.primitive.kind!=='text'||!sameVideoDrawingValue((object(id)!.primitive as {layout:VideoTextLayoutRef}).layout,value.reference))texts.delete(id);setCacheVersion(v=>v+1);}));
  async function ensureEditable(annotationId:string):Promise<Drawing|undefined>{
    const cached=projected(annotationId);if(cached)return cached;
    const entry=object(annotationId);if(!entry||entry.primitive.kind==='rect'||!current())return;
    const p=structuredClone(entry.primitive),requestTarget=target(),source=sourceKey(),key=JSON.stringify([source,annotationId,p.layout]);
    const found=flights.get(key);if(found)return found;
    const id=crypto.randomUUID(),work=(async()=>{
      if(p.kind==='vector'){
        const expected={requestId:id,target:requestTarget,annotationId,reference:p.layout},reply=validateVideoVectorRead(await(props.onReadVector??getVideoAnnotationVector)(id,requestTarget,annotationId,p.layout),expected);
        const now=object(annotationId)?.primitive;if(!current()||sourceKey()!==source||now?.kind!=='vector'||!sameVideoDrawingValue(now.layout,p.layout))return;
        vectors.delete(annotationId);vectors.set(annotationId,reply);
      }else{
        const reply=validateVideoTextRead(await props.onReadText(id,requestTarget,annotationId,p.layout),id,requestTarget,annotationId,p.layout);
        const now=object(annotationId)?.primitive;if(!current()||sourceKey()!==source||now?.kind!=='text'||!sameVideoDrawingValue(now.layout,p.layout))return;
        texts.delete(annotationId);texts.set(annotationId,reply);
      }
      trimCache();setCacheVersion(v=>v+1);return projected(annotationId);
    })().finally(()=>{if(flights.get(key)===work)flights.delete(key);});
    flights.set(key,work);return work;
  }
  const authorizes=(drawing:Drawing)=>Boolean(props.grant?.tools.includes(drawing.kind as never));
  const isTranslation=(before:Drawing,after:Drawing)=>before.kind===after.kind&&before.color===after.color&&before.strokeWidth===after.strokeWidth&&before.fontSize===after.fontSize&&before.text===after.text&&before.points.length===after.points.length&&after.points.every((p,i)=>p.x-before.points[i].x===after.points[0].x-before.points[0].x&&p.y-before.points[i].y===after.points[0].y-before.points[0].y);
  async function command(action:EditorDrawingAction):Promise<void>{
    if(!ready()||action.expectedRevision!==(doc()?.revision??0))throw Error('视频绘制对象已变化');
    props.onPause();const before=structuredClone(props.item),requestTarget=target(),grant=props.grant&&structuredClone(props.grant),sceneId=props.sceneId,source=sourceKey(),info=structuredClone(props.info);
    let receipt:Snapshot;
    if(action.type==='move_stored'){
      const actual=bounds(action.drawingId);if(!sameVideoDrawingValue(actual,action.from)||action.to.width!==actual.width||action.to.height!==actual.height)throw Error('视频绘制对象已变化');
      receipt=await props.onDocument(requestTarget,{type:'move',annotationId:action.drawingId,fromTopLeft:{x:action.from.x,y:action.from.y},toTopLeft:{x:action.to.x,y:action.to.y}});
    }else if(action.type==='remove_drawing')receipt=await props.onDocument(requestTarget,{type:'remove',annotationId:action.drawingId});
    else if(action.type==='undo_drawing'||action.type==='redo_drawing'){
      const type=action.type==='undo_drawing'?'undo':'redo',head=before.videoAnnotations?.[type].at(-1);if(!head)throw Error('视频绘制历史已变化');
      receipt=await props.onDocument(requestTarget,{type,operationId:head.id});
    }else if('drawing'in action){
      const value=action.drawing,stored=before.videoAnnotations?.objects.find(entry=>entry.id===value.id),prior=values().find(entry=>entry.id===value.id);
      if(action.type==='update_drawing'&&stored&&prior&&isTranslation(prior,value)){
        const fromTopLeft=stored.primitive.kind==='rect'?{x:stored.primitive.bounds.x,y:stored.primitive.bounds.y}:stored.primitive.topLeft;
        const toTopLeft={x:fromTopLeft.x+value.points[0].x-prior.points[0].x,y:fromTopLeft.y+value.points[0].y-prior.points[0].y};
        receipt=await props.onDocument(requestTarget,{type:'move',annotationId:stored.id,fromTopLeft,toTopLeft});
      }else if(action.type==='update_drawing'&&stored?.primitive.kind==='text'){
        const content=drawingToVideoSource(value);if(content.kind!=='text'||!sameVideoDrawingValue(content.topLeft,stored.primitive.topLeft))throw Error('视频文字已变化');
        receipt=await props.onEditText(crypto.randomUUID(),requestTarget,stored.id,stored.primitive.layout,content.content);
      }else{
        if(!grant||!authorizes(value))throw Error('视频绘制工具不可用');
        const content=drawingToVideoSource(value);
        const nativeAction:VideoDrawingAction=action.type==='add_drawing'?{type:'add',content}:stored?.primitive.kind==='vector'&&content.kind!=='text'?{type:'update',annotationId:stored.id,reference:stored.primitive.layout,content}:(()=>{throw Error('视频绘制对象已变化');})();
        receipt=await props.onApplyDrawing(crypto.randomUUID(),requestTarget,grant,nativeAction);
        const accepted=acceptVideoDrawingReceipt(receipt,sceneId,before,nativeAction,info);
        if(nativeAction.type==='add'&&sourceKey()===source)aliases.set(value.id,accepted.annotationId);
      }
    }else throw Error('视频绘制操作无效');
    // This exact successful receipt is still authoritative if its publish arrived earlier.
    // Root's snapshot reducer preserves later revisions; no late result replaces current input.
    props.onSnapshot(receipt);
  }
  const port:DrawingEditorPort={
    frame:()=>({x:0,y:0,width:props.info.width,height:props.info.height}),sourceSize:()=>({width:props.info.width,height:props.info.height}),
    sourceIdentity:()=>JSON.stringify([scope(),props.grant??null,props.visibleAnnotationIds??null]),draftKey:sourceKey,revision:()=>doc()?.revision??0,drawings:()=>values().filter(value=>!props.visibleAnnotationIds||props.visibleAnnotationIds.includes(value.id)),
    stored:()=>(doc()?.objects??[]).filter(value=>!props.visibleAnnotationIds||props.visibleAnnotationIds.includes(value.id)).map(value=>({id:value.id,kind:value.primitive.kind,bounds:bounds(value.id)})),ensureEditable,
    storedMove:(id,from,delta)=>{const p=object(id)?.primitive;if(!p)throw Error('视频绘制对象已变化');if(p.kind==='vector'){const g=p.layout.geometryBounds,x=Math.max(Math.ceil(-g.x),Math.min(Math.floor(props.info.width-g.x-g.width),Math.floor(from.x+delta.x+.5))),y=Math.max(Math.ceil(-g.y),Math.min(Math.floor(props.info.height-g.y-g.height),Math.floor(from.y+delta.y+.5)));return{...from,x,y};}return{...from,x:Math.max(0,Math.min(props.info.width-from.width,from.x+delta.x)),y:Math.max(0,Math.min(props.info.height-from.height,from.y+delta.y))};},
    renderStored:(id,interactive,selected,preview)=>props.renderStored(id,interactive,selected,preview),
    canSelect:()=>true,canStyle:drawing=>drawing.kind==='text'||Boolean(descriptor(drawing.id)&&authorizes(drawing)),
    allows:action=>current()&&(action.type!=='add_drawing'||Boolean(props.grant?.tools.includes(action.drawing.kind as never))),
    propertyCommand:draft=>{if(!draftChanged(draft))return;drawingToVideoSource(draft.drawing);return{type:draft.base?'update_drawing':'add_drawing',expectedRevision:draft.expectedRevision,drawing:draft.drawing};},
    historyAvailable:redo=>Boolean(doc()?.[redo?'redo':'undo'].length),
    persistedId:id=>aliases.get(id)??id,rejectPointOverflow:true,
    move:(original,start,end)=>{
      const entry=object(original.id),p=entry?.primitive;
      if(p?.kind==='vector')return movedVideoVector(original,p.topLeft,p.layout,start,end,{width:props.info.width,height:props.info.height});
      const dx=end.x-start.x,dy=end.y-start.y,b=p?.kind==='rect'?p.bounds:p?.kind==='text'?{...p.topLeft,width:p.layout.width,height:p.layout.height}:undefined;
      if(!b)throw Error('视频绘制对象已变化');
      const x=Math.max(-b.x,Math.min(props.info.width-b.x-b.width,dx)),y=Math.max(-b.y,Math.min(props.info.height-b.y-b.height,dy));
      return{...original,points:original.points.map(point=>({x:point.x+x,y:point.y+y}))};
    },
    render:(drawing,interactive,selected,preview)=>object(drawing.id)?props.renderStored(drawing.id,interactive,selected,preview?drawing:undefined):<DrawingShape drawing={drawing} interactive={interactive} selected={selected}/>,
  };
  const register:RegisterDrawingFlush=flush=>props.onRegisterDrawingAuthoringFlush?.(async active=>{if(!active()||!ready())throw Error('视频绘制对象已变化');await flush(active);})??(()=>{});
  onCleanup(()=>{disposed=true;});
  return <SharedDrawingEditor port={port} box={props.box} tools={props.grant?[...props.grant.tools]:[]} busy={props.busy||!ready()} inputLocked={props.inputLocked||!props.active} retainOnDone onCommand={command} onError={props.onError} onClose={props.onClose} onRegisterFlush={register} onExport={(copy,close)=>props.onOutput?.(copy,close)??Promise.resolve(false)} keyboardIgnore=".video-trim-popover,[data-run-journal]"/>;
}
