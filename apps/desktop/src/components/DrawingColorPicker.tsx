/* SPDX-License-Identifier: MPL-2.0 */
import {createEffect,createSignal,For,Index,on,onCleanup,Show} from 'solid-js';
import {Portal} from 'solid-js/web';
import {Check,X} from 'lucide-solid';
import {t} from '../i18n';
import {colorHistoryChanged,colorHistoryKey,colorPaletteChanged,colorPaletteKey,colorHsv,colorPresets,colorRgb,hsvColor,initialColorPalette,normalizeColor,normalizeColorHistory,normalizeColorPalette,recentColors,rememberColor,rememberColorPalette,rgbColor,storedColorPalette} from '../drawing-colors';
import './drawing-color-picker.css';

export default function DrawingColorPicker(props:{value:string;disabled?:boolean;onChange:(color:string,submit:boolean)=>void}){
  let trigger!:HTMLButtonElement,panel:HTMLDivElement|undefined,plane:HTMLDivElement|undefined,pointer:number|undefined,previewBase:string|undefined,expectedValue:string|undefined;
  let editing:{index:number;original:string}|undefined;
  const buttons:HTMLButtonElement[]=[],initial=initialColorPalette(props.value);
  const [slots,setSlots]=createSignal(initial),[active,setActive]=createSignal(Math.max(0,initial.indexOf(props.value.toUpperCase())));
  const [open,setOpen]=createSignal(false),[position,setPosition]=createSignal({x:0,y:0}),[recent,setRecent]=createSignal(recentColors());
  const [hsv,setHsv]=createSignal(colorHsv(props.value)),[hex,setHex]=createSignal(props.value.toUpperCase());
  createEffect(on(()=>props.value,value=>{const next=colorHsv(value);setHex(value.toUpperCase());setHsv(old=>({...next,h:next.s===0?old.h:next.h}));const color=normalizeColor(value);if(!color)return;if(expectedValue===color){expectedValue=undefined;return;}expectedValue=undefined;close();const index=slots().indexOf(color);if(index>=0)setActive(index);else setSlots(values=>values.map((entry,index)=>index===active()?color:entry));}));
  const place=()=>{if(!trigger)return;const box=trigger.getBoundingClientRect(),width=244,height=panel?.offsetHeight||332;setPosition({x:Math.max(8,Math.min(innerWidth-width-8,box.left)),y:Math.max(8,box.top-height-8>=8?box.top-height-8:Math.min(innerHeight-height-8,box.bottom+8))});};
  const close=(focus=false)=>{if(pointer!==undefined&&plane?.hasPointerCapture(pointer))plane.releasePointerCapture(pointer);pointer=undefined;if(editing){const {index,original}=editing,duplicate=slots().findIndex((color,slot)=>slot!==index&&color===slots()[index]);if(duplicate>=0){setSlots(values=>values.map((color,slot)=>slot===index?original:color));setActive(duplicate);}}previewBase=undefined;editing=undefined;setOpen(false);if(focus)trigger?.focus({preventScroll:true});};
  const remember=(next:string,previous?:string)=>{const values=rememberColor(next,undefined,previous);setRecent(values);window.dispatchEvent(new CustomEvent(colorHistoryChanged,{detail:values}));};
  const choose=(value:string,submit:boolean)=>{
    const next=normalizeColor(value);if(!next||props.disabled)return;
    const index=editing?.index??active(),previous=previewBase??normalizeColor(props.value);
    if(!submit&&previewBase===undefined)previewBase=previous;
    const duplicate=slots().findIndex((color,slot)=>slot!==index&&color===next);
    if(submit&&duplicate>=0){const original=editing?.original;if(original)setSlots(values=>values.map((color,slot)=>slot===index?original:color));setActive(duplicate);close();}
    else{setSlots(values=>values.map((color,slot)=>slot===index?next:color));setActive(index);}
    expectedValue=next;props.onChange(next,submit);
    if(submit){previewBase=undefined;if(editing)editing.original=next;remember(next,previous);const colors=rememberColorPalette(slots());if(colors)window.dispatchEvent(new CustomEvent(colorPaletteChanged,{detail:colors}));}
  };
  const select=(index:number)=>{
    if(props.disabled)return;trigger=buttons[index];
    if(index!==active()){close();const next=slots()[index],previous=normalizeColor(props.value);setActive(index);expectedValue=next;props.onChange(next,true);remember(next,previous);return;}
    if(open()){close();return;}editing={index,original:slots()[index]};setRecent(recentColors());place();setOpen(true);queueMicrotask(()=>{if(open()){place();panel?.querySelector<HTMLButtonElement>('button')?.focus({preventScroll:true});}});
  };
  const commitHex=()=>{const next=normalizeColor(hex());if(next)choose(next,true);else setHex(props.value.toUpperCase());};
  const changeRgb=(index:number,input:HTMLInputElement,submit:boolean)=>{const value=Number(input.value);if(!input.value||!Number.isFinite(value)){if(submit)input.value=String(colorRgb(props.value)[index]);return;}const rgb=colorRgb(props.value);rgb[index]=value;choose(rgbColor(rgb),submit);};
  const sample=(event:PointerEvent,submit:boolean)=>{if(!plane||props.disabled)return;const box=plane.getBoundingClientRect(),s=Math.max(0,Math.min(1,(event.clientX-box.left)/box.width)),v=1-Math.max(0,Math.min(1,(event.clientY-box.top)/box.height));setHsv(old=>({...old,s,v}));choose(hsvColor(hsv().h,s,v),submit);};
  const outside=(event:PointerEvent)=>{if(open()&&!panel?.contains(event.target as Node)&&!buttons.some(button=>button.contains(event.target as Node)))close();};
  const key=(event:KeyboardEvent)=>{if(open()&&event.key==='Escape'){event.preventDefault();event.stopImmediatePropagation();close(true);}};
  const syncPalette=(value:unknown)=>{if(open())return;const colors=normalizeColorPalette(value);if(!colors)return;const current=normalizeColor(props.value);if(!current)return;const index=colors.indexOf(current);if(index>=0){setSlots(colors);setActive(index);}else setSlots(colors.map((color,index)=>index===active()?current:color));};
  const sync=(event:StorageEvent)=>{if(event.key===colorHistoryKey||event.key===null)setRecent(recentColors());if(event.key===colorPaletteKey)syncPalette(storedColorPalette());};
  const syncLocal=(event:Event)=>{setRecent(normalizeColorHistory((event as CustomEvent<unknown>).detail));};
  const syncLocalPalette=(event:Event)=>syncPalette((event as CustomEvent<unknown>).detail);
  document.addEventListener('pointerdown',outside,true);document.addEventListener('keydown',key,true);window.addEventListener('resize',place);window.addEventListener('storage',sync);window.addEventListener(colorHistoryChanged,syncLocal);window.addEventListener(colorPaletteChanged,syncLocalPalette);
  onCleanup(()=>{close();document.removeEventListener('pointerdown',outside,true);document.removeEventListener('keydown',key,true);window.removeEventListener('resize',place);window.removeEventListener('storage',sync);window.removeEventListener(colorHistoryChanged,syncLocal);window.removeEventListener(colorPaletteChanged,syncLocalPalette);});
  createEffect(()=>{if(props.disabled)close();});
  createEffect(()=>{if(open()){recent();queueMicrotask(()=>{if(open())place();});}});
  return <>
    <Index each={slots()}>{(value,index)=>{const label=()=>t('颜色 {0}：{1}').replace('{0}',String(index+1)).replace('{1}',value());return <button ref={element=>{buttons[index]=element;}} class="drawing-recent-color" data-caption={t('颜色')} title={label()} aria-label={label()} aria-pressed={active()===index} aria-haspopup="dialog" aria-expanded={open()&&editing?.index===index} classList={{selected:active()===index}} disabled={props.disabled} onClick={()=>select(index)}><span class="drawing-color-swatch" style={{background:value()}}/></button>;}}</Index>
    <Show when={open()}><Portal><div ref={panel} class="drawing-color-panel" role="dialog" aria-label={t('绘制颜色')} style={{left:`${position().x}px`,top:`${position().y}px`}} onPointerDown={event=>event.stopPropagation()} onKeyDown={event=>{event.stopPropagation();if(event.key==='Tab'){const nodes=[...panel!.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),[tabindex="0"]')],index=nodes.indexOf(document.activeElement as HTMLElement),next=event.shiftKey?(index<=0?nodes.length-1:index-1):(index+1)%nodes.length;event.preventDefault();nodes[next]?.focus();}}}>
      <div class="drawing-color-heading"><span>{t('颜色')}</span><button aria-label={t('关闭')} title={t('关闭')} onClick={()=>close(true)}><X size={15}/></button></div>
      <div ref={plane} class="drawing-color-plane" style={{'background-color':hsvColor(hsv().h,1,1)}} role="group" aria-label={t('色彩与明度')} onPointerDown={event=>{if(event.button!==0||props.disabled)return;event.preventDefault();pointer=event.pointerId;event.currentTarget.setPointerCapture(pointer);sample(event,false);}} onPointerMove={event=>{if(pointer===event.pointerId)sample(event,false);}} onPointerUp={event=>{if(pointer!==event.pointerId)return;sample(event,true);if(event.currentTarget.hasPointerCapture(pointer))event.currentTarget.releasePointerCapture(pointer);pointer=undefined;}} onPointerCancel={()=>{pointer=undefined;previewBase=undefined;}}><i style={{left:`${hsv().s*100}%`,top:`${(1-hsv().v)*100}%`}}/></div>
      <input class="drawing-color-hue" type="range" min="0" max="359" step="1" value={hsv().h} aria-label={t('色相')} onInput={event=>{const h=Number(event.currentTarget.value);setHsv(old=>({...old,h}));choose(hsvColor(h,hsv().s,hsv().v),false);}} onChange={()=>choose(hsvColor(hsv().h,hsv().s,hsv().v),true)}/>
      <div class="drawing-color-values"><label><span>HEX</span><input value={hex()} maxLength={7} spellcheck={false} aria-label={t('十六进制颜色')} onInput={event=>setHex(event.currentTarget.value)} onBlur={commitHex} onKeyDown={event=>{if(event.key==='Enter'){event.preventDefault();commitHex();}}}/></label><For each={['R','G','B']}>{(channel,index)=><label><span>{channel}</span><input type="number" min="0" max="255" step="1" aria-label={`RGB ${channel}`} value={colorRgb(props.value)[index()]} onInput={event=>changeRgb(index(),event.currentTarget,false)} onBlur={event=>changeRgb(index(),event.currentTarget,true)} onKeyDown={event=>{if(event.key==='Enter'){event.preventDefault();changeRgb(index(),event.currentTarget,true);}}}/></label>}</For></div>
      <div class="drawing-color-presets"><For each={colorPresets}>{value=><button style={{background:value}} classList={{selected:props.value.toUpperCase()===value}} title={value} aria-label={t('选择颜色 {0}').replace('{0}',value)} onClick={()=>choose(value,true)}><Show when={props.value.toUpperCase()===value}><Check size={13} style={{color:colorHsv(value).v>.85&&colorHsv(value).s<.35?'#26364d':'white'}}/></Show></button>}</For></div>
      <Show when={recent().length}><div class="drawing-color-recent"><span>{t('最近使用')}</span><For each={recent()}>{value=><button style={{background:value}} title={value} aria-label={t('最近颜色 {0}').replace('{0}',value)} onClick={()=>choose(value,true)}/>}</For></div></Show>
    </div></Portal></Show>
  </>;
}
