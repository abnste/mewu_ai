// SPDX-License-Identifier: MPL-2.0
import type { JSX } from 'solid-js';
import type { Drawing, DrawingPoint, DrawingStep, ManualDrawingKind } from './contracts';
import type { DrawingPropertyDraft } from "./components/drawing-properties";
import type { RegisterDrawingFlush } from "./drawing-flush";
import type { DrawingTableFormat } from "./drawing-layout-preview";
import type { ObjectTrashPort } from './object-trash';
export interface EditorBox { x:number;y:number;width:number;height:number }
export type EditorDrawingAction = {expectedRevision:number} & (
  | {type:'add_drawing'|'update_drawing';drawing:Drawing}
  | {type:'remove_drawing';drawingId:string}
  | {type:'undo_drawing'|'redo_drawing'}
  | {type:'move_stored';drawingId:string;from:EditorBox;to:EditorBox}
);
/** Geometry and gesture UI have no Region/Asset/native command identity. */
export interface DrawingEditorPort {
  frame:()=>EditorBox;
  sourceSize:()=>{width:number;height:number};
  sourceIdentity:()=>string;
  draftKey:()=>string;
  revision:()=>number;
  drawings:()=>Drawing[];
  stored?:()=>{id:string;kind:'rect'|'text'|'vector';bounds:EditorBox}[];
  ensureEditable?:(id:string)=>Promise<Drawing|undefined>;
  storedMove?:(id:string,from:EditorBox,delta:{x:number;y:number})=>EditorBox;
  renderStored?:(id:string,interactive:boolean,selected:boolean,preview?:EditorBox)=>JSX.Element;
  canSelect:(drawing:Drawing)=>boolean;
  canStyle:(drawing:Drawing)=>boolean;
  allows:(action:EditorDrawingAction)=>boolean;
  propertyCommand:(draft:DrawingPropertyDraft)=>EditorDrawingAction|undefined;
  historyAvailable:(redo:boolean)=>boolean;
  historyStep?:(redo:boolean)=>DrawingStep|undefined;
  render:(drawing:Drawing,interactive:boolean,selected:boolean,preview:boolean)=>JSX.Element;
  move?:(original:Drawing,start:{x:number;y:number},end:{x:number;y:number})=>Drawing;
  persistedId?:(temporaryId:string)=>string;
  rejectPointOverflow?:boolean;
  mosaic?:{blockSize:()=>number;prepare:()=>Promise<unknown>};
  raster?:{apply:(mode:'extract'|'heal',points:DrawingPoint[],width:number)=>Promise<{selectedId?:string}>};
  copyTable?:(drawing:Drawing,format:DrawingTableFormat)=>Promise<void>;
}
export interface SharedDrawingEditorProps {
  port:DrawingEditorPort;box:EditorBox;tools:ManualDrawingKind[];
  documentOnly?:boolean;busy:boolean;inputLocked?:boolean;retainOnDone?:boolean;keyboardIgnore?:string;
  onCommand:(action:EditorDrawingAction)=>Promise<void>;
  onError:(message:string)=>void;onClose:()=>void;
  onExport:(copy:boolean,closeSpace:boolean)=>Promise<boolean>;
  onPin?:()=>Promise<boolean>;onRegisterFlush?:RegisterDrawingFlush;
  blackboard?:boolean;
  onFinishBlackboard?:()=>Promise<boolean>;
  extraTools?:()=>JSX.Element;
  objects?:(interactive:()=>boolean)=>JSX.Element;
  objectsFlush?:(active:()=>boolean)=>Promise<void>;
  onObjectPointerMove?:(event:PointerEvent)=>void;
  onObjectPointerLeave?:()=>void;
  onRegisterObjectTrash?:(port:ObjectTrashPort)=>()=>void;
  onRegisterHistory?:RegisterDrawingHistory;
}
export interface DrawingHistoryControls { undo:()=>void;redo:()=>void;canUndo:()=>boolean;canRedo:()=>boolean }
export type RegisterDrawingHistory = (controls:DrawingHistoryControls)=>()=>void;
