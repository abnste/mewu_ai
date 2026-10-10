/* SPDX-License-Identifier: MPL-2.0 */
import {createEffect,createSignal,For,onCleanup,Show} from 'solid-js';
import {Portal} from 'solid-js/web';
import {Check,X} from 'lucide-solid';
import {t} from '../i18n';
import {colorHistoryKey,colorHsv,colorPresets,colorRgb,hsvColor,normalizeColor,recentColors,rememberColor,rgbColor} from '../drawing-colors';
import './drawing-color-picker.css';

export default function DrawingColorPicker(props:{value:string;disabled?:boolean;onChange:(color:string,submit:boolean)=>void}){
  let trigger!:HTMLButtonElement,panel:HTMLDivElement|undefined,plane:HTMLDivElement|undefined,pointer:number|undefined;
  const [open,setOpen]=createSignal(false),[position,setPosition]=createSignal({x:0,y:0}),[recent,setRecent]=createSignal(recentColors());
  const [hsv,setHsv]=createSignal(colorHsv(props.value)),[hex,setHex]=createSignal(props.value.toUpperCase());
  createEffect(()=>{const value=props.value,next=colorHsv(value);setHex(value.toUpperCase());setHsv(old=>({...next,h:next.s===0?old.h:next.h}));});
  const place=()=>{const box=trigger.getBoundingClientRect(),width=244,height=panel?.offsetHeight||332;setPosition({x:Math.max(8,Math.min(innerWidth-width-8,box.left)),y:Math.max(8,box.top-height-8>=8?box.top-height-8:Math.min(innerHeight-height-8,box.bottom+8))});};
  const close=(focus=false)=>{if(pointer!==undefined&&plane?.hasPointerCapture(pointer))plane.releasePointerCapture(pointer);pointer=undefined;setOpen(false);if(focus)trigger.focus({preventScroll:true});};
  const choose=(value:string,submit:boolean)=>{const next=normalizeColor(value);if(!next||props.disabled)return;props.onChange(next,submit);if(submit)setRecent(rememberColor(next));};
  const commitHex=()=>{const next=normalizeColor(hex());if(next)choose(next,true);else setHex(props.value.toUpperCase());};
  const changeRgb=(index:number,input:HTMLInputElement,submit:boolean)=>{const value=Number(input.value);if(!input.value||!Number.isFinite(value)){if(submit)input.value=String(colorRgb(props.value)[index]);return;}const rgb=colorRgb(props.value);rgb[index]=value;choose(rgbColor(rgb),submit);};
  const sample=(event:PointerEvent,submit:boolean)=>{if(!plane||props.disabled)return;const box=plane.getBoundingClientRect(),s=Math.max(0,Math.min(1,(event.clientX-box.left)/box.width)),v=1-Math.max(0,Math.min(1,(event.clientY-box.top)/box.height));setHsv(old=>({...old,s,v}));choose(hsvColor(hsv().h,s,v),submit);};
  const outside=(event:PointerEvent)=>{if(open()&&!panel?.contains(event.target as Node)&&!trigger.contains(event.target as Node))close();};
  const key=(event:KeyboardEvent)=>{if(open()&&event.key==='Escape'){event.preventDefault();event.stopImmediatePropagation();close(true);}};
  const sync=(event:StorageEvent)=>{if(event.key===colorHistoryKey)setRecent(recentColors());};
  document.addEventListener('pointerdown',outside,true);document.addEventListener('keydown',key,true);window.addEventListener('resize',place);window.addEventListener('storage',sync);
  onCleanup(()=>{close();document.removeEventListener('pointerdown',outside,true);document.removeEventListener('keydown',key,true);window.removeEventListener('resize',place);window.removeEventListener('storage',sync);});
  createEffect(()=>{if(props.disabled)close();});
  createEffect(()=>{if(open()){recent();queueMicrotask(()=>{if(open())place();});}});
  return <>
    <button ref={trigger} class="drawing-color-trigger" data-caption={t('颜色')} title={t('颜色')} aria-label={t('绘制颜色')} aria-haspopup="dialog" aria-expanded={open()} disabled={props.disabled} onClick={()=>{if(open()){close();return;}setRecent(recentColors());place();setOpen(true);queueMicrotask(()=>{place();panel?.querySelector<HTMLButtonElement>('button')?.focus({preventScroll:true});});}}><span class="drawing-color-swatch" style={{background:props.value}}/></button>
    <Show when={open()}><Portal><div ref={panel} class="drawing-color-panel" role="dialog" aria-label={t('绘制颜色')} style={{left:`${position().x}px`,top:`${position().y}px`}} onPointerDown={event=>event.stopPropagation()} onKeyDown={event=>{event.stopPropagation();if(event.key==='Tab'){const nodes=[...panel!.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),[tabindex="0"]')],index=nodes.indexOf(document.activeElement as HTMLElement),next=event.shiftKey?(index<=0?nodes.length-1:index-1):(index+1)%nodes.length;event.preventDefault();nodes[next]?.focus();}}}>
      <div class="drawing-color-heading"><span>{t('颜色')}</span><button aria-label={t('关闭')} title={t('关闭')} onClick={()=>close(true)}><X size={15}/></button></div>
      <div ref={plane} class="drawing-color-plane" style={{'background-color':hsvColor(hsv().h,1,1)}} role="group" aria-label={t('色彩与明度')} onPointerDown={event=>{if(event.button!==0||props.disabled)return;event.preventDefault();pointer=event.pointerId;event.currentTarget.setPointerCapture(pointer);sample(event,false);}} onPointerMove={event=>{if(pointer===event.pointerId)sample(event,false);}} onPointerUp={event=>{if(pointer!==event.pointerId)return;sample(event,true);if(event.currentTarget.hasPointerCapture(pointer))event.currentTarget.releasePointerCapture(pointer);pointer=undefined;}} onPointerCancel={()=>{pointer=undefined;}}><i style={{left:`${hsv().s*100}%`,top:`${(1-hsv().v)*100}%`}}/></div>
      <input class="drawing-color-hue" type="range" min="0" max="359" step="1" value={hsv().h} aria-label={t('色相')} onInput={event=>{const h=Number(event.currentTarget.value);setHsv(old=>({...old,h}));choose(hsvColor(h,hsv().s,hsv().v),false);}} onChange={()=>choose(hsvColor(hsv().h,hsv().s,hsv().v),true)}/>
      <div class="drawing-color-values"><label><span>HEX</span><input value={hex()} maxLength={7} spellcheck={false} aria-label={t('十六进制颜色')} onInput={event=>setHex(event.currentTarget.value)} onBlur={commitHex} onKeyDown={event=>{if(event.key==='Enter'){event.preventDefault();commitHex();}}}/></label><For each={['R','G','B']}>{(channel,index)=><label><span>{channel}</span><input type="number" min="0" max="255" step="1" aria-label={`RGB ${channel}`} value={colorRgb(props.value)[index()]} onInput={event=>changeRgb(index(),event.currentTarget,false)} onBlur={event=>changeRgb(index(),event.currentTarget,true)} onKeyDown={event=>{if(event.key==='Enter'){event.preventDefault();changeRgb(index(),event.currentTarget,true);}}}/></label>}</For></div>
      <div class="drawing-color-presets"><For each={colorPresets}>{value=><button style={{background:value}} classList={{selected:props.value.toUpperCase()===value}} title={value} aria-label={t('选择颜色 {0}').replace('{0}',value)} onClick={()=>choose(value,true)}><Show when={props.value.toUpperCase()===value}><Check size={13} style={{color:colorHsv(value).v>.85&&colorHsv(value).s<.35?'#26364d':'white'}}/></Show></button>}</For></div>
      <Show when={recent().length}><div class="drawing-color-recent"><span>{t('最近使用')}</span><For each={recent()}>{value=><button style={{background:value}} title={value} aria-label={t('最近颜色 {0}').replace('{0}',value)} onClick={()=>choose(value,true)}/>}</For></div></Show>
    </div></Portal></Show>
  </>;
}
