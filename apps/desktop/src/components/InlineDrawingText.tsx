// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, onCleanup, onMount } from 'solid-js';
import { EditorState } from 'prosemirror-state';
import { EditorView } from 'prosemirror-view';
import type { Drawing, DrawingTextStyle } from '../contracts';
import { t } from '../i18n';
import { drawingTextLayout } from './drawing-geometry';
import { drawingTextPlugins, formatTextSelection, plainTextPaste, readTextDocument, selectionTextStyle, textDocument, type LiteralTextDocument, type TextSelectionStyle } from '../drawing-text-model';

export interface InlineDrawingTextController { focus():void; format(patch:{color?:string;textStyle?:DrawingTextStyle}):void }
/** Composition, selection and typing history share a literal, typed document. */
export default function InlineDrawingText(props: {
  drawing: Drawing; disabled: boolean;
  ref: (control:InlineDrawingTextController|undefined) => void;
  onInput:(value:LiteralTextDocument)=>void;onSelection:(style:TextSelectionStyle)=>void;
  onError:(message:string)=>void;onConfirm: () => void; onBlur: (event: FocusEvent) => void;
}) {
  let host!:HTMLDivElement,view:EditorView|undefined;
  const context = document.createElement('canvas').getContext('2d')!;
  const layout = createMemo(() => {
    const text = drawingTextLayout(props.drawing), size = text.size;
    context.font = `normal 400 ${size}px "Segoe UI", "Microsoft YaHei", sans-serif`;
    const metrics = context.measureText('Mg中'), lineHeight = size * 1.25;
    const ascent = metrics.fontBoundingBoxAscent ?? size * .8;
    const descent = metrics.fontBoundingBoxDescent ?? size * .2;
    const baseline = ascent + (lineHeight - ascent - descent) / 2;
    return {
      x: props.drawing.points[0].x, y: props.drawing.points[0].y + size - baseline,
      width: text.framed ? text.width : Math.max(size * 2, text.width + size),
      height: text.height, size, lineHeight, framed: text.framed,
    };
  });
  const selected=()=>{if(view)props.onSelection(selectionTextStyle(view.state,props.drawing.color));};
  onMount(()=>{
    let initial=EditorState.create({doc:textDocument(props.drawing),plugins:drawingTextPlugins(props.onError)});
    if(!initial.doc.textContent)initial=initial.apply(formatTextSelection(initial,{color:props.drawing.color,textStyle:props.drawing.textStyle??{}}));
    view=new EditorView(host,{
      state:initial,
      attributes:{class:'drawing-rich-text-content',role:'textbox','aria-label':t('绘制文字'),'aria-multiline':'true',spellcheck:'false'},
      editable:()=>!props.disabled,
      dispatchTransaction(transaction){
        if(!view)return;const previous=view.state.doc,state=view.state.apply(transaction);view.updateState(state);
        if(!state.doc.eq(previous))props.onInput(readTextDocument(state.doc,props.drawing));selected();
      },
      handleKeyDown(_view,event){
        if(!event.isComposing&&event.keyCode!==229&&event.key==='Enter'&&(event.ctrlKey||event.metaKey)){event.preventDefault();event.stopPropagation();props.onConfirm();return true;}return false;
      },
      handlePaste(editor,event){
        const text=event.clipboardData?.getData('text/plain');if(text===undefined)return false;
        event.preventDefault();editor.dispatch(editor.state.tr.replaceSelection(plainTextPaste(text,editor.state.storedMarks??editor.state.selection.$from.marks())));return true;
      },
      handleDOMEvents:{blur:(_view,event)=>{props.onBlur(event);return false;}},
    });
    props.ref({focus:()=>view?.focus(),format:patch=>{if(!view||props.disabled)return;view.dispatch(formatTextSelection(view.state,patch));view.focus();}});
    selected();view.focus();
  });
  createEffect(()=>{props.disabled;view?.setProps({editable:()=>!props.disabled});});
  onCleanup(()=>{view?.destroy();view=undefined;props.ref(undefined);});
  return <foreignObject class="drawing-text-editor" x={layout().x} y={layout().y} width={layout().width} height={layout().height+4}>
    <div ref={host} class="drawing-rich-text" style={{'font-size':`${layout().size}px`,'line-height':`${layout().lineHeight}px`,color:props.drawing.color,'caret-color':props.drawing.color,'white-space':layout().framed?'break-spaces':'pre','word-break':layout().framed?'break-all':'normal','line-break':layout().framed?'anywhere':'auto'}} onPointerDown={event=>event.stopPropagation()} onContextMenu={event=>event.stopPropagation()}/>
  </foreignObject>;
}
