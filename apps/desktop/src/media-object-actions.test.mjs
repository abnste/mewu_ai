// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import vm from 'node:vm';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';

const load = async name => import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(await readFile(new URL(name, import.meta.url), 'utf8'), { mode: 'transform' })).toString('base64')}`);
const { zoomImageObject } = await load('./image-object-geometry.ts');
async function sourceFunctions(file, names) {
  const source = await readFile(new URL(file, import.meta.url), 'utf8'), nodes = [];
  const walk = node => { if (!node || typeof node !== 'object') return; if (node.type) nodes.push(node); for (const [key, value] of Object.entries(node)) { if (key === 'loc') continue; if (Array.isArray(value)) value.forEach(walk); else if (value && typeof value === 'object') walk(value); } };
  walk(parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] }));
  return stripTypeScriptTypes(names.map(name => { const node = nodes.find(n => n.type === 'FunctionDeclaration' && n.id.name === name); assert.ok(node, name); return source.slice(node.start, node.end); }).join('\n') + `\n({${names.join(',')}})`, { mode: 'transform' });
}
const objectCode = await sourceFunctions('./components/ArtifactCard.tsx', ['wheel', 'flushZoom', 'pinObject']);
const appCode = await sourceFunctions('./App.tsx', ['pinMediaItem']);
const { projectBackgroundItem, persistBackgroundItem } = await load('./media-item-geometry.ts');
function objectFixture(kind = 'video') {
  let position = { x: .2, y: .2, width: .4, height: .25 }, moving = false, pinning = false, timer = 0;
  const timers = new Map(), commits = [], errors = [], pins = [];
  const props = { busy: false, item: { ...position, id: 'item', asset: { id: 'asset', kind, width: 400, height: 200 }, videoEdit: { revision: 7, range: { startTicks: 1, endTicks: 99 } }, videoAnnotations: { revision: 5 } }, onUpdate: async item => { commits.push(item); Object.assign(props.item, item); }, onPin: async () => { pins.push(structuredClone(props.item)); return true; }, onError: message => errors.push(message) };
  const context = vm.createContext({ props, disposed: false, zoomAsset: undefined, zoomTimer: undefined, gestureFlight: undefined, disposeGesture: undefined, zoomImageObject,
    position: () => position, setPosition: value => { position = value; }, imageBox: () => ({ width: position.width * 1000, height: position.height * 800 }),
    savedPosition: () => ({ x: props.item.x, y: props.item.y, width: props.item.width, height: props.item.height }),
    isRasterMedia: () => kind === 'image' || kind === 'video', pinning: () => pinning, setPinning: value => { pinning = value; }, setMoving: value => { moving = value; },
    innerWidth: 1000, innerHeight: 800, clearTimeout: id => timers.delete(id), setTimeout: callback => { timers.set(++timer, callback); return timer; },
  });
  const api = vm.runInContext(objectCode, context);
  const event = (deltaY = -30, deltaMode = 0) => ({ deltaY, deltaMode, clientX: 300, clientY: 240, defaultPrevented: false, preventDefault() { this.defaultPrevented = true; }, stopPropagation() {} });
  return { props, context, api, event, position: () => position, moving: () => moving, commits, errors, pins, timers };
}
test('video and image wheel gestures anchor the cursor, retain media edits and coalesce to one saved geometry', async () => {
  for (const kind of ['video', 'image']) {
    const f = objectFixture(kind), original = structuredClone(f.props.item), before = f.position();
    for (const [delta, mode] of [[-30, 0], [-2, 1], [-.03, 2]]) { const event = f.event(delta, mode); f.api.wheel(event); assert.equal(event.defaultPrevented, true); }
    assert.equal(f.commits.length, 0); assert.equal(f.timers.size, 1); assert.equal(f.moving(), true);
    assert.ok(f.position().width > before.width); assert.ok(Math.abs((.3-f.position().x)/f.position().width-(.3-before.x)/before.width)<1e-12);
    await f.api.flushZoom(); assert.equal(f.commits.length, 1); assert.equal(f.moving(), false);
    assert.deepEqual(f.props.item.asset, original.asset); assert.deepEqual(f.props.item.videoEdit, original.videoEdit); assert.deepEqual(f.props.item.videoAnnotations, original.videoAnnotations);
  }
});
test('pin waits for pending wheel persistence; a failed save does not issue a native pin', async () => {
  const f = objectFixture(); f.api.wheel(f.event()); await f.api.pinObject();
  assert.equal(f.commits.length, 1); assert.equal(f.pins.length, 1); assert.equal(f.pins[0].width, f.commits[0].width);
  const rejected = objectFixture(); rejected.props.onUpdate = async () => { throw Error('synthetic geometry conflict'); };
  rejected.api.wheel(rejected.event()); await rejected.api.pinObject();
  assert.equal(rejected.pins.length, 0); assert.ok(rejected.errors.some(message => message.includes('synthetic geometry conflict')));
  assert.equal(rejected.position().width, .4);
});
test('nonmedia/busy/disposed gestures cannot consume wheel or save geometry', async () => {
  for (const change of [f => f.props.busy = true, f => f.context.disposed = true]) { const f=objectFixture(); change(f); const event=f.event(); f.api.wheel(event); assert.equal(event.defaultPrevented,false); await f.api.flushZoom(); assert.equal(f.commits.length,0); }
  const f=objectFixture('text'), event=f.event(); f.api.wheel(event); assert.equal(event.defaultPrevented,false);
  const button=objectFixture(), control=button.event(); control.target={closest:()=>({})}; button.api.wheel(control); assert.equal(control.defaultPrevented,false);
});
test('scene-item pin uses only flushed same-scene asset ids, refreshes live objects and drains its operation', async () => {
  for (const kind of ['image','video']) {
    let busy=false,finished=0,refreshed=0;const calls=[],scene={id:'scene',items:[{id:'item',asset:{id:'asset',kind}}]};
    const context=vm.createContext({scene:()=>scene,busy:()=>busy,sending:()=>false,recording:()=>false,scroll:()=>false,exitPreparing:()=>false,disposed:false,spaceOperations:{begin:()=>()=>finished++},setBusy:value=>busy=value,flush:async()=>{},pinSceneItem:async target=>calls.push(structuredClone(target)),pinObjectsSubscription:{refresh:async()=>refreshed++},showError:()=>assert.fail('unexpected pin error')});
    const {pinMediaItem}=vm.runInContext(appCode,context);assert.equal(await pinMediaItem('item'),true);
    assert.deepEqual(calls,[{sceneId:'scene',itemId:'item',expectedAssetId:'asset'}]);assert.equal(refreshed,1);assert.equal(finished,1);assert.equal(busy,false);
  }
});
test('a scene/source change during pin flush cannot dispatch the previous object', async () => {
  for(const changed of ['scene','asset','source']){let busy=false,finished=0,calls=0,current={id:'scene',items:[{id:'item',asset:{id:'asset',kind:'video'}}]};
    const context=vm.createContext({scene:()=>current,busy:()=>busy,sending:()=>false,recording:()=>false,scroll:()=>false,exitPreparing:()=>false,disposed:false,spaceOperations:{begin:()=>()=>finished++},setBusy:value=>busy=value,flush:async()=>{if(changed==='scene')current={...current,id:'other'};else if(changed==='asset')current.items[0].asset.id='other';else current.items[0].asset.path='changed source';},pinSceneItem:async()=>calls++,pinObjectsSubscription:{refresh:async()=>assert.fail('unexpected refresh')},showError:()=>assert.fail('unexpected error')});
    assert.equal(await vm.runInContext(appCode,context).pinMediaItem('item'),false);assert.equal(calls,0);assert.equal(finished,1);assert.equal(busy,false);
  }
});
test('recordings keep off-frame positions and large uniform zoom after background-unit persistence', () => {
  const background={x:100,y:50,width:800,height:400},viewport={width:1200,height:800};
  for(const geometry of [{x:-.12,y:-.15,width:1.2,height:.9},{x:.8,y:.85,width:.3,height:.2}]){
    const item={...geometry,id:'recording',asset:{id:'source'},videoEdit:{revision:3,range:{startTicks:10,endTicks:90}}};
    const saved=persistBackgroundItem(item,background,viewport),shown=projectBackgroundItem(saved,background,viewport);
    for(const key of ['x','y','width','height'])assert.ok(Math.abs(shown[key]-item[key])<1e-12,`${key} must not snap back`);
    assert.equal(saved.asset,item.asset);assert.equal(saved.videoEdit,item.videoEdit);
  }
  const huge={x:0,y:0,width:20,height:4};const bounded=persistBackgroundItem(huge,background,viewport);
  assert.ok(bounded.width<=8&&bounded.height<=8);assert.ok(Math.abs((bounded.width/bounded.height)-(huge.width*viewport.width/background.width)/(huge.height*viewport.height/background.height))<1e-12,'size bound scales both axes together');
});
