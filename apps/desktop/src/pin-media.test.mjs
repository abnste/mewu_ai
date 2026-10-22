// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule, createContext, runInContext } from 'node:vm';
import { parse } from '@babel/parser';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import * as solid from 'solid-js';
import * as web from 'solid-js/web';
const raw = name => readFile(new URL(name, import.meta.url), 'utf8');
const ts = source => stripTypeScriptTypes(source, { mode: 'transform' });
const modules = new Map();
async function load(name) {
  if (modules.has(name)) return modules.get(name);
  const module = new SourceTextModule(ts(await raw(name))); modules.set(name, module);
  await module.link(dependency => load(`${dependency}.ts`)); await module.evaluate(); return module;
}
const { validPinMedia } = (await load('pin-media.ts')).namespace;
const source = 'http://127.0.0.1:49213/asset', media = { kind: 'video', sourceId: '12345678-1234-4234-9234-123456789abc', sourceDurationTicks: 100_000_000, range: { startTicks: 30_000_000, endTicks: 80_000_000 }, annotationRevision: 2, annotations: [{ id: 'annotation-1', interval: { startTicks: 40_000_000, endTicks: 50_000_000 }, bounds: { x: 10, y: 20, width: 50, height: 30 }, imageUrl: 'http://127.0.0.1:49213/overlay' }] };
test('old image pins remain compatible; timed media requires bounded local immutable descriptors', () => {
  assert.equal(validPinMedia(undefined, source), true); assert.equal(validPinMedia(media, source), true);
  assert.equal(validPinMedia({ ...media, canvasPresentation: true }, source), true);
  assert.equal(validPinMedia({ ...media, canvasPresentation: false }, source), true);
  assert.equal(validPinMedia({ ...media, canvasPresentation: 'macos' }, source), false);
  for (const change of [value => value.kind = 'html', value => value.range.endTicks = 200_000_000, value => value.range.startTicks = -1, value => value.sourceDurationTicks = NaN, value => value.annotations[0].imageUrl = 'https://remote/image', value => value.annotations[0].imageUrl += '?private=1', value => value.annotations[0].interval.endTicks = 200_000_000, value => value.annotations[0].bounds.width = 0, value => value.annotations.push({ ...value.annotations[0] }), value => value.annotations = Array.from({ length: 49 }, (_, index) => ({ ...value.annotations[0], id: String(index) }))]) {
    const invalid = structuredClone(media); change(invalid); assert.equal(validPinMedia(invalid, source), false);
  }
});
test('actual pinned-video component renders real MP4 and bounded timed image layers without browser controls', async () => {
  const sourceText = await raw('components/PinVideo.tsx');
  const compiled = transformSync(sourceText, { filename: 'PinVideo.tsx', parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'ssr', hydratable: false }]], configFile: false, babelrc: false }).code;
  const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
  const module = new SourceTextModule(ts(compiled));
  await module.link(async name => name === 'solid-js' ? synthetic(solid) : name === 'solid-js/web' ? synthetic(web) : name === 'lucide-solid' ? synthetic({ Play: () => '', Pause: () => '' }) : name.endsWith('/i18n') ? synthetic({ t: value => value }) : name.endsWith('.css') ? synthetic({}) : name.endsWith('ObjectMoveGrip') ? synthetic({ default: () => '' }) : load(`${name.replace('../', '')}.ts`));
  await module.evaluate();
  const html = web.renderToString(() => module.namespace.default({ source, media, width: 160, height: 90, onError: () => {}, onDrag: () => {} }));
  assert.ok(html.includes('<video')); assert.ok(html.includes(`src="${source}"`)); assert.ok(html.includes('playsinline')); assert.ok(html.includes('preload="auto"')); assert.equal(/<video[^>]*\bcontrols(?:=|\s|>)/.test(html), false);
  assert.ok(html.includes(`src="${media.annotations[0].imageUrl}"`)); assert.ok(html.includes('播放')); assert.equal(html.includes('下载'), false); assert.equal(html.includes('引用'), false);
});

const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const tick = () => new Promise(resolve => setImmediate(resolve));
async function pinObjectFixture() {
  const sourceText=await raw('components/PinObjectLayer.tsx'),nodes=[];
  const walk=node=>{if(!node||typeof node!=='object')return;if(node.type)nodes.push(node);for(const [key,value] of Object.entries(node)){if(key==='loc')continue;if(Array.isArray(value))value.forEach(walk);else if(value&&typeof value==='object')walk(value);}};
  walk(parse(sourceText,{sourceType:'module',plugins:['typescript','jsx']}));
  const names=['cancelPendingIntent','drainZoom','wheel','closeObject','down'];
  const code=ts(names.map(name=>{const node=nodes.find(node=>node.type==='FunctionDeclaration'&&node.id.name===name);assert.ok(node,name);return sourceText.slice(node.start,node.end);}).join('\n')+`\n({${names.join(',')}})`);
  let position={x:.1,y:.2};const calls=[],errors=[],flights=[];
  const props={busy:false,exiting:false,object:{id:'pin',x:.1,y:.2,displayWidth:.3,displayHeight:.2},onError:cause=>errors.push(cause)};
  const {moveImageObject}=(await load('image-object-geometry.ts')).namespace;
  const {beginObjectDrag}=(await load('extension-object-drag.ts')).namespace;
  const context=createContext({props,moving:false,disposed:false,zooming:false,zoomSteps:0,intentGeneration:0,cleanup:undefined,
    position:()=>position,setPosition:value=>{position=value;},innerWidth:1000,innerHeight:800,moveImageObject,beginObjectDrag,
    controlPinObject:(id,action)=>{const flight=deferred();calls.push({id,action:JSON.parse(JSON.stringify(action))});flights.push(flight);return flight.promise;}});
  const api=runInContext(code,context);
  const wheel=()=>({deltaY:-1,preventDefault(){},stopPropagation(){}});
  const listeners=new Map();let captured=false;
  const target={addEventListener:(name,callback)=>listeners.set(name,callback),removeEventListener:(name,callback)=>{if(listeners.get(name)===callback)listeners.delete(name);},setPointerCapture:()=>{captured=true;},hasPointerCapture:()=>captured,releasePointerCapture:()=>{captured=false;}};
  const down=()=>({button:0,target:{closest:()=>null},currentTarget:target,clientX:10,clientY:20,pointerId:1,preventDefault(){},stopPropagation(){}});
  return {api,props,calls,errors,flights,listeners,wheel,down,position:()=>position};
}
test('pin wheel cancels its own pending queue on exit and never sends follow-up IPC', async () => {
  const f=await pinObjectFixture();f.api.wheel(f.wheel());f.api.wheel(f.wheel());f.api.wheel(f.wheel());
  assert.equal(f.calls.length,1);f.props.exiting=true;f.api.cancelPendingIntent();
  f.flights[0].reject(Error('应用正在退出'));await tick();assert.equal(f.calls.length,1);assert.deepEqual(f.errors,[]);
  f.api.wheel(f.wheel());assert.equal(f.calls.length,1);
});
test('pin wheel preserves genuine failures and only ignores a specifically revoked late intent', async () => {
  const ordinary=await pinObjectFixture();ordinary.api.wheel(ordinary.wheel());const failure=Error('native failure');ordinary.flights[0].reject(failure);await tick();assert.deepEqual(ordinary.errors,[failure]);
  const notRevoked=await pinObjectFixture();notRevoked.api.wheel(notRevoked.wheel());notRevoked.props.exiting=true;notRevoked.flights[0].reject(failure);await tick();assert.deepEqual(notRevoked.errors,[failure],'an exiting flag alone must not swallow an unknown error');
  const resumed=await pinObjectFixture();resumed.api.wheel(resumed.wheel());resumed.props.exiting=true;resumed.api.cancelPendingIntent();resumed.props.exiting=false;resumed.api.wheel(resumed.wheel());
  resumed.flights[0].reject(failure);await tick();assert.equal(resumed.calls.length,2,'fresh explicit wheel after aborted exit is not stranded by the older flight');resumed.flights[1].resolve();await tick();assert.deepEqual(resumed.errors,[]);
});
test('pin drag revocation releases capture and cannot submit or publish its old pointer intent', async () => {
  const canceled=await pinObjectFixture();canceled.api.down(canceled.down());canceled.listeners.get('pointermove')({clientX:100,clientY:120});const oldEnd=canceled.listeners.get('pointerup');canceled.props.exiting=true;canceled.api.cancelPendingIntent();
  assert.equal(canceled.listeners.size,0);await oldEnd({type:'pointerup'});assert.equal(canceled.calls.length,0);assert.deepEqual(JSON.parse(JSON.stringify(canceled.position())),{x:.1,y:.2});
  for(const revoke of [false,true]){
    const f=await pinObjectFixture();f.api.down(f.down());f.listeners.get('pointermove')({clientX:100,clientY:120});const end=f.listeners.get('pointerup')({type:'pointerup'});assert.equal(f.calls.length,1);
    if(revoke){f.props.exiting=true;f.api.cancelPendingIntent();}const failure=Error('move failed');f.flights[0].reject(failure);await end;
    assert.deepEqual(f.errors,revoke?[]:[failure]);
  }
});
test('pin close keeps real failures visible while its own revoked late intent cannot abort exit', async () => {
  for(const revoke of [false,true]){
    const f=await pinObjectFixture(),pending=f.api.closeObject();assert.equal(f.calls.length,1);assert.equal(f.calls[0].action.type,'close');
    if(revoke){f.props.exiting=true;f.api.cancelPendingIntent();}const failure=Error('close failed');f.flights[0].reject(failure);await pending;assert.deepEqual(f.errors,revoke?[]:[failure]);
  }
  const exiting=await pinObjectFixture();exiting.props.exiting=true;await exiting.api.closeObject();assert.equal(exiting.calls.length,0);
});
test('scene-item pin bridge preserves exact nullable cancellation and genuine IPC errors with only three IDs', async () => {
  const calls=[];let response=null,failure;
  const core=new SyntheticModule(['invoke','isTauri'],function(){this.setExport('isTauri',()=>true);this.setExport('invoke',async(command,args)=>{calls.push({command,args});if(failure)throw failure;return response;});});
  const module=new SourceTextModule(ts(await raw('media-pin-bridge.ts')));await module.link(name=>{assert.equal(name,'@tauri-apps/api/core');return core;});await module.evaluate();
  const target={sceneId:'scene',itemId:'item',expectedAssetId:'asset'};
  assert.equal(await module.namespace.pinSceneItem(target),null);response={id:'pin'};assert.equal((await module.namespace.pinSceneItem(target)).id,'pin');failure=Error('copy failed');await assert.rejects(module.namespace.pinSceneItem(target),/copy failed/);
  assert.ok(calls.every(call=>call.command==='pin_scene_item'&&JSON.stringify(call.args)===JSON.stringify(target)));
});
