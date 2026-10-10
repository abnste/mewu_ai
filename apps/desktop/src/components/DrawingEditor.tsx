// SPDX-License-Identifier: MPL-2.0
import type { JSX } from 'solid-js';
import type { Asset, Drawing, DrawingCommand, ManualDrawingKind, Region, Snapshot } from '../contracts';
import { applyRasterEdit } from '../raster-edit-bridge';
import { getMosaicPreview } from "../bridge";
import { mosaicBlockSize } from "../mosaic-preview";
import { DrawingShape } from "./DrawingLayer";
import { drawingDraftKey, drawingPropertyCommand } from "./drawing-properties";
import { richDocumentHistoryStep, usesDrawingDocument } from "../drawing-document";
import { richPreviewTarget, richSourceIdentity, type DrawingLayoutTarget, type DrawingTableFormat } from "../drawing-layout-preview";
import type { RegisterDrawingFlush } from "../drawing-flush";
import type { DrawingEditorPort, EditorBox, EditorDrawingAction, RegisterDrawingHistory } from '../drawing-editor-port';
import SharedDrawingEditor from './SharedDrawingEditor';
interface Props {
  sceneId:string;backgroundId:string;region:Region;box:EditorBox;scale:number;backgroundWidth:number;backgroundHeight:number;
  background?:Asset;sourceRegion?:Region;documentOnly?:boolean;tools:ManualDrawingKind[];busy:boolean;
  onCommand:(command:DrawingCommand)=>Promise<void>;onError:(message:string)=>void;onClose:()=>void;
  onExport:(copy:boolean,closeSpace:boolean)=>Promise<boolean>;onPin?:()=>Promise<boolean>;
  onRegisterFlush?:RegisterDrawingFlush;onCopyTable?:(target:DrawingLayoutTarget,format:DrawingTableFormat)=>Promise<void>;
  onRasterSnapshot?:(snapshot:Snapshot)=>void;
  blackboard?:boolean;onRegisterHistory?:RegisterDrawingHistory;
  onFinishBlackboard?:()=>Promise<boolean>;
  extraTools?:()=>JSX.Element;
}
/** Existing screenshot API is preserved; only this adapter knows native Region identity. */
export default function DrawingEditor(props:Props) {
  const target = () => ({sceneId:props.sceneId,backgroundId:props.backgroundId,regionId:props.region.id});
  const command = (action:EditorDrawingAction):DrawingCommand => {if(action.type==='move_stored')throw Error('截图对象操作无效');return {...target(),...action};};
  const richContext = () => props.background ? {sceneId:props.sceneId,background:props.background,region:props.sourceRegion??props.region} : undefined;
  const mosaicSource = () => props.region.imageOverride ? {regionId:props.region.id,source:props.region.imageOverride,drawingRevision:props.region.drawingRevision??0} : undefined;
  const mosaicContext = () => props.background ? {sceneId:props.sceneId,background:props.background,override:mosaicSource()} : undefined;
  const raster:NonNullable<DrawingEditorPort['raster']> = {apply:async(mode,points,width)=>{
    if(!props.background||!props.onRasterSnapshot||props.documentOnly)throw Error('此选区不能修补');
    const receipt=await applyRasterEdit({sceneId:props.sceneId,background:props.background,region:props.sourceRegion??props.region},mode,points,width);
    props.onRasterSnapshot(receipt.snapshot);
    return {selectedId:receipt.selectedId??undefined};
  }};
  const port:DrawingEditorPort = {
    frame:()=>({x:props.region.x,y:props.region.y,width:props.region.width,height:props.region.height}),
    sourceSize:()=>({width:props.backgroundWidth,height:props.backgroundHeight}),
    sourceIdentity:()=>props.background ? richSourceIdentity(richContext()!) : props.backgroundId,
    draftKey:()=>drawingDraftKey({...target(),sourceId:props.region.imageOverride?.id??props.backgroundId,x:props.region.x,y:props.region.y,width:props.region.width,height:props.region.height}),
    revision:()=>props.region.drawingRevision??0,drawings:()=>props.region.drawings??[],
    canSelect:drawing=>drawing.rich?.kind!=='repair'&&(!props.documentOnly||drawing.kind==='rich'),canStyle:drawing=>!props.documentOnly&&drawing.kind!=='rich',
    allows:action=>!props.documentOnly||usesDrawingDocument(props.region,command(action)),
    propertyCommand:draft=>drawingPropertyCommand(draft,target()),
    historyAvailable:redo=>{const step=(redo?props.region.drawingHistory?.redo:props.region.drawingHistory?.undo)?.at(-1);return Boolean(step&&(!props.documentOnly||richDocumentHistoryStep(step)));},
    historyStep:redo=>(redo?props.region.drawingHistory?.redo:props.region.drawingHistory?.undo)?.at(-1),
    render:(drawing,interactive,selected)=><DrawingShape drawing={drawing} interactive={interactive} selected={selected} richContext={richContext()} mosaicContext={mosaicContext()} onError={props.onError}/>,
    get mosaic(){return props.background?{blockSize:()=>mosaicBlockSize(props.background?.scaleFactor),prepare:()=>getMosaicPreview(props.sceneId,props.background!,mosaicBlockSize(props.background?.scaleFactor),mosaicSource())}:undefined;},
    get raster(){return props.background&&props.onRasterSnapshot&&!props.documentOnly?raster:undefined;},
    get copyTable(){return props.onCopyTable?(drawing:Drawing,format:DrawingTableFormat)=>{const context=richContext(),value=context&&richPreviewTarget(context,drawing);if(!value)throw Error('表格已更新');return props.onCopyTable!(value,format);}:undefined;},
  };
  return <SharedDrawingEditor extraTools={props.extraTools} onFinishBlackboard={props.onFinishBlackboard} port={port} box={props.box} tools={props.tools} busy={props.busy} documentOnly={props.documentOnly} blackboard={props.blackboard} retainOnDone={props.blackboard} onRegisterHistory={props.onRegisterHistory} onCommand={action=>props.onCommand(command(action))} onClose={props.onClose} onError={props.onError} onExport={props.onExport} onPin={props.onPin} onRegisterFlush={props.onRegisterFlush}/>;
}
