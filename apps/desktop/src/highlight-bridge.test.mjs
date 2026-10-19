// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {SourceTextModule,SyntheticModule,createContext} from 'node:vm';
import {webcrypto} from 'node:crypto';
import {loadProviderPresets,loadAssetImport,loadGeometryHistory,loadDrawingDiagnostics, loadHighlightSource} from './geometry-test-module.mjs';
const read=async name=>stripTypeScriptTypes(await readFile(new URL(name,import.meta.url),'utf8'),{mode:'transform'});
test('actual highlight bridge coalesces unchanged ink, sends exact raster targets, rejects bad mapping and releases a failed cache entry',async()=>{
  const context=createContext({crypto:webcrypto,structuredClone,Date,Map,Set,Blob,URL,setTimeout,clearTimeout,
    Image:class{naturalWidth=120;naturalHeight=60;decode(){return Promise.resolve();}}});
  const synthetic=values=>new SyntheticModule(Object.keys(values),function(){for(const[key,value]of Object.entries(values))this.setExport(key,value);},{context});
  const calls=[],waits=[];let badMapping=false;
  const core=synthetic({isTauri:()=>true,invoke:async(command,args)=>{calls.push({command,args});await new Promise(resolve=>waits.push(resolve));return{backgroundId:args.backgroundId,x:badMapping?21:20,y:10,width:120,height:60,dataUrl:'data:image/png;base64,synthetic'};}});
  const connection=new SourceTextModule(await read('./connection-policy.ts'),{context});await connection.link(()=>{throw Error('Unexpected');});
  const mosaic=new SourceTextModule(await read('./mosaic-preview.ts'),{context});await mosaic.link(()=>{throw Error('Unexpected');});
  const assets=await loadAssetImport(context),history=await loadGeometryHistory(context),foreground=await loadHighlightSource(context),providers=await loadProviderPresets(context);
  const module=new SourceTextModule(await read('./bridge.ts'),{context,initializeImportMeta:meta=>{meta.glob=()=>({});}});
  await module.link(async name=>name==='./drawing-diagnostics'?await loadDrawingDiagnostics(context):name==='./highlight-source'?foreground:name==='./asset-import'?assets:name==='./region-geometry-history'?history:name==='./provider-presets'?providers:name==='./connection-policy'?connection:name==='./mosaic-preview'?mosaic:name.endsWith('/core')?core:synthetic({listen:async()=>()=>{}}));await module.evaluate();
  const background={id:'owned',path:'owned.png',kind:'image',name:'owned',width:180,height:100};
  const ink={id:'ink',kind:'pen',color:'#ff0000',strokeWidth:2,points:[{x:30,y:25}]};
  const redaction={...ink,id:'redaction',kind:'mosaic',strokeWidth:6,points:[{x:30,y:25},{x:40,y:35}]};
  const region={id:'region',x:20,y:10,width:120,height:60,drawings:[ink,redaction],drawingRevision:2};
  const api=module.namespace,tick=()=>new Promise(resolve=>setImmediate(resolve));
  const first=api.getHighlightPreview('scene',background,region);await tick();
  const same=api.getHighlightPreview('scene',background,{...region,drawingRevision:3,drawings:[{...ink,color:'#000000'},redaction]});assert.equal(first,same);assert.equal(calls.length,1);
  assert.deepEqual(JSON.parse(JSON.stringify(calls[0])),{command:'get_highlight_preview',args:{sceneId:'scene',backgroundId:'owned',target:{regionId:'region',sourceId:'owned',x:20,y:10,width:120,height:60,rasterDrawings:[redaction]}}});
  waits[0]();await first;assert.equal(api.getHighlightPreview('scene',background,region),first);
  badMapping=true;const clean={...region,drawings:[ink]};const bad=api.getHighlightPreview('scene',background,clean);await tick();waits[1]();await assert.rejects(bad,/预览已失效/);
  badMapping=false;const retry=api.getHighlightPreview('scene',background,clean);await tick();assert.equal(calls.length,3);waits[2]();assert.equal((await retry).x,20);
});
