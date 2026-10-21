// SPDX-License-Identifier: MPL-2.0
import {createResource,createSignal,onCleanup,Show} from 'solid-js';
import {documentUrl,native,readArtifact} from '../bridge';
import type {Asset} from '../contracts';
import {documentBounds,type DocumentBounds} from '../document-layout';
import runtime from '../../../../packages/sdk/document-runtime.js?raw';
import {t} from '../i18n';
function documentFrame(source:string){
  const policy="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; media-src data: blob:; font-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
  return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${policy}"><meta name="viewport" content="width=device-width,initial-scale=1"></head><body>${source}<script>${runtime};mewuDocumentRuntime();</script></body></html>`;
}
export default function DocumentArtifact(props:{asset:Asset;layoutWidth:number;layoutHeight:number;box?:DocumentBounds;scale:number;onBounds:(box:DocumentBounds)=>void}) {
  let frame!:HTMLIFrameElement,port:MessagePort|undefined,timer:ReturnType<typeof setTimeout>|undefined;
  const [failed,setFailed]=createSignal(false);
  const [source]=createResource(()=>!native&&props.asset.id,()=>readArtifact(props.asset.id).catch(()=>{setFailed(true);return undefined;}));
  function connect(){
    port?.close();clearTimeout(timer);if(!frame.contentWindow)return;
    const channel=new MessageChannel();port=channel.port1;
    timer=setTimeout(()=>{port?.close();setFailed(true);},5000);
    port.onmessage=event=>{if(event.data?.type==='layout'){const box=documentBounds(event.data);if(box){clearTimeout(timer);setFailed(false);props.onBounds(box);}}};
    port.start();frame.contentWindow.postMessage({type:'mewu-document-connect'},'*',[channel.port2]);
  }
  onCleanup(()=>{port?.close();clearTimeout(timer);});
  const style=()=>({width:`${props.box?.viewportWidth?props.layoutWidth:Math.max(props.layoutWidth,props.box?.width??0)}px`,height:`${props.box?.viewportHeight?props.layoutHeight:Math.max(props.layoutHeight,props.box?.height??0)}px`,transform:`scale(${props.scale})`,'transform-origin':'0 0'});
  return <Show when={!failed()} fallback={<span>{t('内容不可用')}</span>}><Show when={native||source()}><iframe ref={frame} title={props.asset.name} src={native?documentUrl(props.asset):undefined} srcdoc={!native&&source()?documentFrame(source()!):undefined} style={style()} sandbox="allow-scripts" referrerpolicy="no-referrer" allow="" onLoad={connect}/></Show></Show>;
}
