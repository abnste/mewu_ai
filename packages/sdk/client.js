// SPDX-License-Identifier: MPL-2.0
// Installed before plugin HTML in an opaque-origin sandbox. No Tauri API.
(() => {
  let port, closed = false, closing = false, serial = 0;
  const beforeClose=new Set();
  async function requestClose(){
    if(closing||closed)return;closing=true;
    try{for(const callback of beforeClose)await callback();port?.postMessage({type:'escape'});}
    catch(error){port?.postMessage({type:'close-error',error:String(error?.message||'保存失败')});}
    finally{closing=false;}
  }
  let press, activeDrag;
  function relayPointer(event) {
    if (!activeDrag || !event.isTrusted || event.pointerId !== activeDrag.pointerId) return;
    const {id}=activeDrag;
    if (!pending.has(id)) { activeDrag=undefined; return; }
    port.postMessage({type:'pointer-drag-event',id,event:{type:event.type,x:event.clientX,y:event.clientY,buttons:event.buttons}});
    if(event.type!=='pointermove')activeDrag=undefined;
  }
  addEventListener('pointerdown',event=>{if(event.isTrusted&&event.button===0)press=event;},true);
  addEventListener('pointermove',relayPointer,true);
  addEventListener('pointerup',event=>{press=undefined;relayPointer(event);},true);
  addEventListener('pointercancel',event=>{press=undefined;relayPointer(event);},true);
  // Keys in an opaque iframe never bubble to the containing space.
  addEventListener('keydown',event=>{
    if(event.key!=='Escape'||event.isComposing||event.keyCode===229||event.repeat||event.target instanceof HTMLSelectElement)return;
    event.preventDefault();event.stopPropagation();void requestClose();
  });
  const pending = new Map(), listeners = new Map();
  function request(type,input) {
    if (closed || !port || pending.size >= 16) throw new Error('Surface unavailable');
    const id = ++serial;
    return new Promise((resolve,reject) => {
      const timer=setTimeout(() => {pending.delete(id);if(activeDrag?.id===id)activeDrag=undefined;reject(new Error('Plugin request timed out'));}, 30000);
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
          if(activeDrag?.id===data.id)activeDrag=undefined;
          data.ok ? entry.resolve(data.value) : entry.reject(new Error(data.error || 'Plugin request failed'));
        } else if (data.type === 'event') {
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
    closed = true; activeDrag=undefined; port?.close();
    for (const entry of pending.values()) {clearTimeout(entry.timer);entry.reject(new Error('Surface closed'));}
    pending.clear(); listeners.clear(); beforeClose.clear();
  }
  addEventListener('pagehide', dispose, {once:true});
  Object.defineProperty(globalThis, 'mewu', {value: Object.freeze({
    ready,
    onBeforeClose(callback){beforeClose.add(callback);return ()=>beforeClose.delete(callback);},
    async call(input) {
      await ready;
      return request('call',input);
    },
    drag(event, preview) {
      if (press!==event || !(event instanceof PointerEvent) || !event.isTrusted || event.type!=='pointerdown' || event.button!==0 || !(event.buttons&1)) {
        return Promise.reject(new Error('Drag requires a user pointer press'));
      }
      press=undefined;
      event.preventDefault();
      const target=event.target;
      if(target instanceof Element && target.hasPointerCapture(event.pointerId))target.releasePointerCapture(event.pointerId);
      const id=serial+1;
      const result=request('pointer-drag',{x:event.clientX,y:event.clientY,preview});
      // WebKit can keep dispatching a pressed pointer to its original iframe
      // even after the host installs an overlay. Relay only this live gesture.
      activeDrag={id,pointerId:event.pointerId};
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
