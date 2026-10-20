// SPDX-License-Identifier: MPL-2.0
/** A bounded parent-document gesture keeps pointer release out of iframe gaps.
 * Coordinates are returned to the owning client; all drop semantics stay there. */
let active=false;
export interface SurfacePointerSample {type:'pointermove'|'pointerup'|'pointercancel';x:number;y:number;buttons:number}
export interface SurfacePointerGesture {
  done:Promise<{cancelled:boolean;x:number;y:number}>;
  cancel:()=>void;
  forward:(sample:SurfacePointerSample)=>void;
}
export function surfacePointerDrag(frame:HTMLIFrameElement,input?:unknown):SurfacePointerGesture {
  if(active)throw Error('已有拖动操作');
  active=true;
  const shield=document.createElement('div');
  Object.assign(shield.style,{position:'fixed',inset:'0',zIndex:'2147483647',cursor:'grabbing',touchAction:'none'});
  shield.setAttribute('aria-hidden','true');
  const detail=input as {x?:number;y?:number;preview?:{image?:string;label?:string}}|undefined;
  let ghost:HTMLElement|undefined;
  if(detail&&Number.isFinite(detail.x)&&Number.isFinite(detail.y)){
    ghost=document.createElement('div');
    Object.assign(ghost.style,{position:'fixed',left:'0',top:'0',width:'100px',padding:'6px',borderRadius:'10px',background:'#fafbffed',boxShadow:'0 6px 20px #0003',opacity:'.9',pointerEvents:'none',font:'12px system-ui',color:'#354052',overflow:'hidden',textOverflow:'ellipsis',whiteSpace:'nowrap'});
    const preview=detail.preview;
    if(typeof preview?.image==='string'&&/^data:image\/png;base64,[A-Za-z0-9+/=]+$/.test(preview.image)&&preview.image.length<=300000){
      const image=document.createElement('img');image.src=preview.image;
      Object.assign(image.style,{display:'block',width:'88px',height:'72px',objectFit:'contain'});ghost.append(image);
    }
    const label=document.createElement('span');label.textContent=typeof preview?.label==='string'?preview.label.slice(0,120):'对象';ghost.append(label);
    shield.append(ghost);
    const rect=frame.getBoundingClientRect();ghost.style.transform=`translate(${rect.left+detail.x!+12}px,${rect.top+detail.y!+12}px)`;
  }
  let finish:(cancel:boolean,x?:number,y?:number)=>void=()=>{};
  let forward:SurfacePointerGesture['forward']=()=>{};
  const done=new Promise<{cancelled:boolean;x:number;y:number}>(resolve=>{
    let ended=false;
    const move=(event:PointerEvent)=>{event.preventDefault();if(!(event.buttons&1)){finish(true);return;}if(ghost)ghost.style.transform=`translate(${event.clientX+12}px,${event.clientY+12}px)`;};
    const up=(event:PointerEvent)=>{event.preventDefault();event.stopPropagation();finish(event.type!=='pointerup',event.clientX,event.clientY);};
    const key=(event:KeyboardEvent)=>{if(event.key==='Escape'){event.preventDefault();event.stopPropagation();finish(true);}};
    // Focusing the originating iframe can blur the parent Window without
    // leaving this document tree. Only actual application focus loss cancels.
    const blur=()=>{if(!document.hasFocus())finish(true);};
    const timer=setTimeout(()=>finish(true),25000);
    forward=sample=>{
      if(ended)return;
      const rect=frame.getBoundingClientRect(),x=rect.left+sample.x,y=rect.top+sample.y;
      if(sample.type!=='pointermove'){finish(sample.type!=='pointerup',x,y);return;}
      if(!(sample.buttons&1)){finish(true);return;}
      if(ghost)ghost.style.transform=`translate(${x+12}px,${y+12}px)`;
    };
    finish=(cancelled,x=0,y=0)=>{
      if(ended)return;ended=true;active=false;clearTimeout(timer);
      const rect=frame.getBoundingClientRect();
      window.removeEventListener('blur',blur);window.removeEventListener('keydown',key,true);
      shield.remove();resolve({cancelled,x:x-rect.left,y:y-rect.top});
    };
    shield.addEventListener('pointermove',move);
    shield.addEventListener('pointerup',up);
    shield.addEventListener('pointercancel',up);
    window.addEventListener('blur',blur);window.addEventListener('keydown',key,true);
    document.body.append(shield);
  });
  return {done,cancel:()=>finish(true),forward:sample=>forward(sample)};
}
