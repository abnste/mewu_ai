// SPDX-License-Identifier: MPL-2.0
import { Fragment, Schema, Slice, type Mark, type Node as TextNode } from 'prosemirror-model';
import { EditorState, Plugin, type Transaction } from 'prosemirror-state';
import { baseKeymap, toggleMark } from 'prosemirror-commands';
import { keymap } from 'prosemirror-keymap';
import { history, redo, undo } from 'prosemirror-history';
import type { Drawing, DrawingTextRun, DrawingTextStyle } from './contracts';

const flags=['bold','italic','underline','strikethrough'] as const;
export const validTextColor=(value:unknown):value is string=>typeof value==='string'&&/^#[\da-f]{6}$/i.test(value);
export const drawingTextSchema=new Schema({
  nodes:{doc:{content:'paragraph+'},paragraph:{content:'text*',group:'block',toDOM:()=>['p',0],parseDOM:[{tag:'p'}]},text:{group:'inline'}},
  marks:{bold:{toDOM:()=>['strong',0]},italic:{toDOM:()=>['em',0]},underline:{toDOM:()=>['u',0]},strikethrough:{toDOM:()=>['s',0]},color:{attrs:{value:{}},toDOM:mark=>['span',{style:`color:${mark.attrs.value}`},0]}},
});
export interface TextSelectionStyle { color:string;textStyle:DrawingTextStyle }
export interface LiteralTextDocument {text:string;textRuns?:DrawingTextRun[]}
function styleKey(run:DrawingTextRun){return JSON.stringify([run.color,...flags.map(key=>!!run.textStyle?.[key])]);}
export function readTextDocument(doc:TextNode,base:Pick<Drawing,'color'|'textStyle'>):LiteralTextDocument {
  const runs:DrawingTextRun[]=[];
  const append=(text:string,marks:readonly Mark[])=>{
    if(!text)return;const style:DrawingTextStyle={};for(const key of flags)if(marks.some(mark=>mark.type.name===key))style[key]=true;
    const color=marks.find(mark=>mark.type.name==='color')?.attrs.value??base.color;
    const run={text,color,textStyle:style},last=runs.at(-1);
    if(last&&styleKey(last)===styleKey(run))last.text+=text;else runs.push(run);
  };
  doc.forEach((paragraph,_offset,index)=>{
    if(index)append('\n',paragraph.firstChild?.marks??doc.child(index-1).lastChild?.marks??[]);
    paragraph.forEach(node=>append(node.text??'',node.marks));
  });
  const text=runs.map(run=>run.text).join(''),baseRun={text,color:base.color,textStyle:base.textStyle};
  return {text,...(runs.length&&!(runs.length===1&&styleKey(runs[0])===styleKey(baseRun))?{textRuns:runs}:{})};
}
export function textDocument(drawing:Pick<Drawing,'text'|'textRuns'|'color'|'textStyle'>):TextNode {
  const paragraphs:TextNode[]=[];let content:TextNode[]=[];
  const finish=()=>{paragraphs.push(drawingTextSchema.nodes.paragraph.create(null,content));content=[];};
  for(const run of drawing.textRuns??[{text:drawing.text??''}]){
    const style=run.textStyle??drawing.textStyle,marks:Mark[]=[];
    for(const key of flags)if(style?.[key])marks.push(drawingTextSchema.marks[key].create());
    const color=run.color??drawing.color;if(validTextColor(color))marks.push(drawingTextSchema.marks.color.create({value:color}));
    run.text.split('\n').forEach((text,index)=>{if(index)finish();if(text)content.push(drawingTextSchema.text(text,marks));});
  }
  finish();return drawingTextSchema.nodes.doc.create(null,paragraphs);
}
export function selectionTextStyle(state:EditorState,fallback:string):TextSelectionStyle {
  const selection=state.selection,sets:(readonly Mark[])[]=[];
  if(selection.empty)sets.push(state.storedMarks??selection.$from.marks());
  else state.doc.nodesBetween(selection.from,selection.to,node=>{if(node.isText)sets.push(node.marks);});
  const textStyle:DrawingTextStyle={};for(const key of flags)if(sets.length&&sets.every(marks=>marks.some(mark=>mark.type.name===key)))textStyle[key]=true;
  return {color:sets[0]?.find(mark=>mark.type.name==='color')?.attrs.value??fallback,textStyle};
}
export function formatTextSelection(state:EditorState,patch:{color?:string;textStyle?:DrawingTextStyle}):Transaction {
  let transaction=state.tr;const {from,to,empty}=state.selection,current=selectionTextStyle(state,'#000000');
  const update=(name:string,value:boolean|string)=>{
    const type=drawingTextSchema.marks[name];
    if(empty){transaction=value===false?transaction.removeStoredMark(type):transaction.addStoredMark(type.create(typeof value==='string'?{value}:undefined));}
    else {transaction=transaction.removeMark(from,to,type);if(value!==false)transaction=transaction.addMark(from,to,type.create(typeof value==='string'?{value}:undefined));}
  };
  if(patch.color!==undefined&&validTextColor(patch.color))update('color',patch.color);
  // Preserve other effects within a mixed-format selection when one toggles.
  if(patch.textStyle)for(const key of flags)if(!!patch.textStyle[key]!==!!current.textStyle[key])update(key,!!patch.textStyle[key]);
  return transaction;
}
export function drawingTextPlugins(onError:(message:string)=>void){
  return [history(),keymap({'Mod-b':toggleMark(drawingTextSchema.marks.bold),'Mod-i':toggleMark(drawingTextSchema.marks.italic),'Mod-u':toggleMark(drawingTextSchema.marks.underline),'Mod-Shift-x':toggleMark(drawingTextSchema.marks.strikethrough),'Mod-z':undo,'Shift-Mod-z':redo,'Mod-y':redo}),keymap(baseKeymap),new Plugin({filterTransaction:transaction=>{
    if(transaction.docChanged&&[...readTextDocument(transaction.doc,{color:'#000000'}).text].length>2000){onError('绘制文字不能超过 2000 字');return false;}return true;
  }})];
}
export function plainTextPaste(text:string,marks:readonly Mark[]):Slice {
  const paragraphs=text.replace(/\r\n?/g,'\n').replaceAll('\t','    ').split('\n').map(line=>drawingTextSchema.nodes.paragraph.create(null,line?drawingTextSchema.text(line,marks):undefined));
  return new Slice(Fragment.fromArray(paragraphs),1,1);
}
