// SPDX-License-Identifier: MPL-2.0
/** A bounded parent-document gesture keeps pointer release out of iframe gaps.
 * Coordinates are returned to the owning client; all drop semantics stay there. */
let active=false;
export function surfacePointerDrag(frame:HTMLIFrameElement) {
  if(active)throw Error('已有拖动操作');
  active=true;
  const shield=document.createElement('div');
  Object.assign(shield.style,{position:'fixed',inset:'0',zIndex:'2147483647',cursor:'grabbing',touchAction:'none'});
  shield.setAttribute('aria-hidden','true');
  let finish:(cancel:boolean,x?:number,y?:number)=>void=()=>{};
  const done=new Promise<{cancelled:boolean;x:number;y:number}>(resolve=>{
    let ended=false;
    const move=(event:PointerEvent)=>{event.preventDefault();if(!(event.buttons&1))finish(true);};
    const up=(event:PointerEvent)=>{event.preventDefault();event.stopPropagation();finish(event.type!=='pointerup',event.clientX,event.clientY);};
    const key=(event:KeyboardEvent)=>{if(event.key==='Escape'){event.preventDefault();event.stopPropagation();finish(true);}};
    const blur=()=>finish(true);
    const timer=setTimeout(blur,30000);
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
  return {done,cancel:()=>finish(true)};
}
