// SPDX-License-Identifier: MPL-2.0
// Installed before plugin HTML in an opaque-origin sandbox. No Tauri API.
(() => {
  let port, closed = false, closing = false, serial = 0, pinned = false;
  const beforeClose=new Set();
  const previews=new Set();
  async function previewRaster(source){
    const image=new Image();image.src=source;await image.decode();
    const canvas=document.createElement('canvas');canvas.width=240;canvas.height=180;
    const context=canvas.getContext('2d');if(!context)return source;
    context.drawImage(image,0,0,240,180);
    try{return canvas.toDataURL('image/png');}catch{return source;}
  }
  async function requestClose(){
    if(closing||closed)return;closing=true;
    try{for(const callback of beforeClose)await callback();port?.postMessage({type:'escape'});}
    catch(error){port?.postMessage({type:'close-error',error:String(error?.message||'保存失败')});}
    finally{closing=false;}
  }
  let press, activeDrag;
  function releaseDrag() {
    const drag=activeDrag;activeDrag=undefined;
    if(drag?.capture){
      try{if(drag.capture.hasPointerCapture(drag.pointerId))drag.capture.releasePointerCapture(drag.pointerId);}catch{/* the surface may already be detached */}
    }
  }
  function relayPointer(event) {
    if (!activeDrag || !event.isTrusted || event.pointerId !== activeDrag.pointerId) return;
    const {id}=activeDrag;
    if (!pending.has(id)) { releaseDrag(); return; }
    port.postMessage({type:'pointer-drag-event',id,event:{type:event.type,x:event.clientX,y:event.clientY,buttons:event.buttons}});
    if(event.type!=='pointermove')releaseDrag();
  }
  addEventListener('pointerdown',event=>{if(event.isTrusted&&event.button===0)press=event;},true);
  addEventListener('pointermove',relayPointer,true);
  addEventListener('pointerup',event=>{press=undefined;relayPointer(event);},true);
  addEventListener('pointercancel',event=>{press=undefined;relayPointer(event);},true);
  // Keys in an opaque iframe never bubble to the containing space.
  addEventListener('keydown',event=>{
    if(event.key!=='Escape'||event.isComposing||event.keyCode===229||event.repeat||event.target instanceof HTMLSelectElement)return;
    event.preventDefault();event.stopPropagation();
    if(pinned)port?.postMessage({type:'escape-key'});else void requestClose();
  });
  const pending = new Map(), listeners = new Map();
  function request(type,input) {
    if (closed || !port || pending.size >= 16) throw new Error('Surface unavailable');
    const id = ++serial;
    return new Promise((resolve,reject) => {
      const timer=setTimeout(() => {pending.delete(id);if(activeDrag?.id===id)releaseDrag();reject(new Error('Plugin request timed out'));}, 30000);
      pending.set(id,{resolve,reject,timer});
      port.postMessage({type,id,input});
    });
  }
  const ready = new Promise(resolve => {
    const connect = event => {
      if (event.source !== parent || event.data?.type !== 'mewu-connect' || !event.ports[0] || port) return;
      port = event.ports[0];
      removeEventListener('message', connect);
      port.onmessage = ({ data }) => {
        if (!data || typeof data !== 'object') return;
        if (data.type === 'response') {
          const entry = pending.get(data.id);
          if (!entry) return;
          pending.delete(data.id); clearTimeout(entry.timer);
          if(activeDrag?.id===data.id)releaseDrag();
          data.ok ? entry.resolve(data.value) : entry.reject(new Error(data.error || 'Plugin request failed'));
        } else if (data.type === 'event') {
          if(data.name==='window-state')pinned=data.value?.pinned===true;
          if(data.name==='close-request'){void requestClose();return;}
          for (const callback of listeners.get(data.name) || []) {
            try { callback(data.value); } catch { /* one listener cannot break delivery */ }
          }
        }
      };
      port.start(); resolve();
    };
    addEventListener('message', connect);
  });
  function dispose() {
    closed = true; releaseDrag(); port?.close();
    for(const cancel of previews)cancel();previews.clear();
    for (const entry of pending.values()) {clearTimeout(entry.timer);entry.reject(new Error('Surface closed'));}
    pending.clear(); listeners.clear(); beforeClose.clear();
  }
  addEventListener('pagehide', dispose, {once:true});
  Object.defineProperty(globalThis, 'mewu', {value: Object.freeze({
    ready,
    async previewDocument(input) {
      if(closed||typeof input?.html!=='string'||input.html.length>2*1024*1024)throw Error('Invalid preview document');
      const width=Math.min(1600,Math.max(80,Number(input.width)||640)),height=Math.min(1200,Math.max(40,Number(input.height)||480));
      const policy="default-src 'none';script-src 'unsafe-inline';style-src 'unsafe-inline';img-src data: blob:;font-src data:;media-src data: blob:;connect-src 'none';frame-src 'none';object-src 'none';base-uri 'none';form-action 'none'";
      const html='<!doctype html><meta http-equiv="Content-Security-Policy" content="'+policy+'"><body>'+input.html+'<script>'+mewuDocumentRuntime.toString()+';mewuDocumentRuntime();<\/script>';
      const frame=document.createElement('iframe');frame.sandbox='allow-scripts';frame.setAttribute('aria-hidden','true');frame.style.cssText='position:fixed;left:-10000px;top:0;pointer-events:none;border:0;width:'+width+'px;height:'+height+'px';
      const channel=new MessageChannel();
      return new Promise((resolve,reject)=>{
        let done=false,timer,requested=false;
        const cancel=()=>finish(null,new Error('Surface closed'));
        const finish=(image,error)=>{if(done)return;done=true;previews.delete(cancel);clearTimeout(timer);channel.port1.close();frame.remove();error?reject(error):resolve(image);};
        previews.add(cancel);
        timer=setTimeout(()=>finish(null,new Error('Preview timed out')),5000);
        channel.port1.onmessage=event=>{
          if(event.data?.type==='layout'&&!requested){requested=true;channel.port1.postMessage({type:'preview'});}
          if(event.data?.type==='preview'){
            const image=typeof event.data.image==='string'&&event.data.image.length<2*1024*1024&&event.data.image.startsWith('data:image/svg+xml;base64,')?event.data.image:null;
            if(!image){finish(null);return;}
            void previewRaster(image).then(value=>finish(value),()=>finish(image));
          }
        };
        frame.onload=()=>{if(!done)frame.contentWindow.postMessage({type:'mewu-document-connect'},'*',[channel.port2]);};frame.srcdoc=html;document.body.append(frame);channel.port1.start();
      });
    },
    onBeforeClose(callback){beforeClose.add(callback);return ()=>beforeClose.delete(callback);},
    async call(input) {
      await ready;
      return request('call',input);
    },
    drag(event, preview) {
      // WebKit on macOS can report buttons=0 for a fast native primary press.
      // The captured, unconsumed trusted pointerdown is the gesture authority;
      // a later pointerup/cancel still revokes it before any request can start.
      if (press!==event || !(event instanceof PointerEvent) || !event.isTrusted || event.type!=='pointerdown' || event.button!==0 || (!(event.buttons&1)&&event.buttons!==0)) {
        return Promise.reject(new Error('Drag requires a user pointer press'));
      }
      if(activeDrag)return Promise.reject(new Error('A drag is already active'));
      press=undefined;
      event.preventDefault();
      const target=event.target;
      let capture;
      if(target instanceof Element){
        // Keep the real gesture in its originating document across iframe edges.
        // The host still owns the preview and validates every forwarded sample.
        try{target.setPointerCapture(event.pointerId);capture=target;}catch{/* the parent shield remains a fallback */}
      }
      const id=serial+1;
      activeDrag={id,pointerId:event.pointerId,capture};
      let result;
      try{result=request('pointer-drag',{x:event.clientX,y:event.clientY,preview});}
      catch(error){releaseDrag();return Promise.reject(error);}
      // WebKit can keep dispatching a pressed pointer to its original iframe
      // even after the host installs an overlay. Relay only this live gesture.
      return result;
    },
    async setDropTargets(targets) {await ready;if(!closed)port.postMessage({type:'drop-targets',targets});},
    on(name, callback) {
      const callbacks=listeners.get(name)||new Set();callbacks.add(callback);listeners.set(name,callbacks);
      return ()=>callbacks.delete(callback);
    },
    async viewportPoint(x,y) {
      await ready;
      return this.call({type:'viewport-point',x,y});
    },
    dispose
  }), writable:false, configurable:false});
})();
