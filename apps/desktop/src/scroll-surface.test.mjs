// SPDX-License-Identifier: MPL-2.0
// Actual production ScrollSurface/ScrollBackdrop with Solid's reactive runtime.
// Host events and DOM are synthetic; no screen, global shortcut or user data.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule, createContext } from 'node:vm';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import postcss from 'postcss';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';

const raw = path => readFile(new URL(path, import.meta.url), 'utf8');
const tick = () => new Promise(resolve => setImmediate(resolve));
const plain = value => JSON.parse(JSON.stringify(value));
const source = await raw('./ScrollSurface.tsx');
const compiled = stripTypeScriptTypes(transformSync(source, { filename: 'ScrollSurface.tsx', configFile: false, babelrc: false,
  parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'universal', moduleName: 'test-renderer' }]] }).code, { mode: 'transform' });
async function fixture({ hidden = false, ready = () => Promise.resolve() } = {}) {
  const keys = new Map(), calls = [], readyCalls = [], frames = new Map(); let listener, stopped = 0, disposed, frameId = 0;
  let bounds = { width: 1200, height: 800 };
  const document = { hidden, documentElement: { dataset: {} }, addEventListener: (name, fn) => keys.set(name, fn), removeEventListener: (name, fn) => { if (keys.get(name) === fn) keys.delete(name); } };
  const context = createContext({ document, console, Promise, Intl, requestAnimationFrame: fn => { frames.set(++frameId, fn); return frameId; }, cancelAnimationFrame: id => frames.delete(id),
    localStorage: { getItem: () => JSON.stringify({ maskOpacity: .66 }) } });
  const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }, { context });
  const sources = new Map();
  async function loadSource(url) {
    if (sources.has(url.href)) return sources.get(url.href);
    const value = new SourceTextModule(stripTypeScriptTypes(await readFile(url, 'utf8'), { mode: 'transform' }), { context, identifier: url.href });
    sources.set(url.href, value);
    await value.link(name => loadSource(new URL(`${name}.ts`, url)));
    return value;
  }
  const preferences = await loadSource(new URL('./interface-preferences.ts', import.meta.url)); await preferences.evaluate();
  const core = synthetic(solid), universal = new SourceTextModule(await raw('../../../node_modules/solid-js/universal/dist/universal.js'), { context });
  await universal.link(() => core); await universal.evaluate();
  const renderer = universal.namespace.createRenderer({
    createElement: type => ({ type, children: [], props: {}, parent: null, get isConnected() { return Boolean(this.parent?.isConnected); }, getBoundingClientRect: () => bounds }), createTextNode: text => ({ text, children: [], parent: null }),
    replaceText: (node, text) => { node.text = text; }, setProperty: (node, key, value) => { node.props[key] = value; },
    insertNode: (parent, node, anchor) => { if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1); const index = anchor ? parent.children.indexOf(anchor) : -1; parent.children.splice(index < 0 ? parent.children.length : index, 0, node); node.parent = parent; },
    removeNode: (parent, node) => { parent.children.splice(parent.children.indexOf(node), 1); node.parent = null; },
    isTextNode: node => 'text' in node, getParentNode: node => node.parent, getFirstChild: node => node.children[0], getNextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1],
  });
  const bridge = new SourceTextModule(stripTypeScriptTypes(await raw('./scroll-bridge.ts'), { mode: 'transform' }), { context });
  await bridge.link(name => name.endsWith('/core') ? synthetic({ isTauri: () => false, invoke: () => { throw Error('No fixture IPC'); } }) : synthetic({ listen: () => { throw Error('No fixture native events'); } })); await bridge.evaluate();
  const module = new SourceTextModule(compiled, { context });
  await module.link(name => {
    if (name === 'solid-js') return core;
    if (name === 'test-renderer') return synthetic(renderer);
    if (name === './i18n') return synthetic({ t: value => value });
    if (name === './interface-preferences') return preferences;
    if (name === './native-select-escape') return synthetic({ nativeSelectOwnsEscape: () => false });
    if (name === './scroll-bridge') return synthetic({ scrollNeedsKeep: bridge.namespace.scrollNeedsKeep, scrollSurfaceReady: id => { readyCalls.push(id); return ready(id); }, subscribeScroll: async (next, surface) => { assert.equal(surface, 'scroll-controls'); listener = next; return () => stopped++; }, controlScrollCapture: async (id, action) => { calls.push({ id, action }); } });
    if (name === 'lucide-solid') return synthetic({ Check: () => null, CircleAlert: () => null, LoaderCircle: () => null, X: () => null });
    if (name.endsWith('.css')) return synthetic({});
    throw Error(`Unexpected import ${name}`);
  }); await module.evaluate();
  const root = { children: [], isConnected: true };
  solid.createRoot(dispose => { disposed = dispose; renderer.insert(root, () => module.namespace.default()); });
  await tick();
  const nodes = () => { const result = []; const walk = node => { result.push(node); node.children?.forEach(walk); }; walk(root); return result; };
  const one = key => nodes().find(node => node.props?.class === key);
  return { next: value => listener(value), nodes, one, calls, readyCalls, keys, frames, root, module, renderer, document,
    setBounds: value => { bounds = value; }, flushFrames: () => { const batch = [...frames.values()]; frames.clear(); for (const fn of batch) fn(); },
    visibility: hidden => { document.hidden = hidden; keys.get('visibilitychange')?.(); }, dispose: () => disposed(), stopped: () => stopped };
}
const initial = { id: 'capture-a', sceneId: 'synthetic', phase: 'capturing', rect: { x: .3, y: .25, width: .4, height: .5 }, width: 600, height: 400, status: 'initial', stopHotkey: 'F8 / Esc', preview: 'data:image/png;base64,first',
  hud: { outline: { x: .299, y: .248, width: .402, height: .504 }, preview: { x: .15, y: .45, width: .12, height: .3 }, controls: { x: .35, y: .1, width: .2, height: .08 }, live: true, retainMain: false } };

test('source-monitor outline and growing accepted preview stay mounted while image height and image pixels update', async () => {
  const f = await fixture(); f.next(initial); await tick();
  const outline = f.one('scroll-live-outline'), preview = f.one('scroll-preview'), controls = f.one('scroll-controls-slot');
  assert.ok(outline && preview && controls);
  assert.deepEqual(plain(outline.props.style), { left: '29.9%', top: '24.8%', width: '40.2%', height: '50.4%' });
  assert.deepEqual(plain(preview.props.style), { left: '15%', top: '45%', width: '12%', height: '30%' });
  assert.deepEqual(plain(controls.props.style), { left: '35%', top: '10%', width: '20%', height: '8%' });
  const image = f.nodes().find(node => node.type === 'img');
  f.next({ ...initial, height: 800, status: 'extended', preview: 'data:image/png;base64,second', hud: { ...initial.hud, preview: { ...initial.hud.preview, y: .15, height: .6 } } }); await tick();
  assert.equal(f.one('scroll-preview'), preview); assert.equal(f.one('scroll-controls-slot'), controls); assert.equal(f.one('scroll-live-outline'), outline);
  assert.equal(f.nodes().find(node => node.type === 'img'), image);
  assert.equal(image.props.src, 'data:image/png;base64,second'); assert.equal(preview.props.style.height, '60%'); assert.equal(preview.props.style.top, '15%');
  assert.ok(f.nodes().some(node => node.text?.includes('800')));
  f.dispose(); assert.equal(f.stopped(), 1); assert.equal(f.keys.has('keydown'), false);
});

test('no room outside a full-monitor source never invents an overlapping preview or controls; original keyboard finish still works', async () => {
  const f = await fixture(); f.next({ ...initial, hud: { ...initial.hud, preview: null, controls: null } }); await tick();
  assert.ok(f.one('scroll-live-outline')); assert.equal(f.one('scroll-preview'), undefined); assert.equal(f.one('scroll-controls-slot'), undefined);
  f.keys.get('keydown')({ key: 'F8', repeat: false, preventDefault() {}, stopPropagation() {} }); await tick();
  assert.deepEqual(f.calls, [{ id: 'capture-a', action: 'finish' }]); f.dispose();
});

test('fallback platforms keep the original preview/controls, keep is explicit, ending clears all live pixels', async () => {
  const f = await fixture(); f.next({ ...initial, hud: undefined, status: 'lost_overlap' }); await tick();
  assert.equal(f.one('scroll-live-outline'), undefined); assert.ok(f.one('scroll-preview')); assert.ok(f.one('scroll-controls-slot'));
  const keep = f.nodes().find(node => node.props?.['aria-label'] === '保存已拼部分'); assert.ok(keep); keep.props.onClick(); await tick();
  assert.deepEqual(f.calls, [{ id: 'capture-a', action: 'keep' }]);
  f.next(null); await tick(); assert.equal(f.one('scroll-preview'), undefined); assert.equal(f.one('scroll-controls-slot'), undefined); f.dispose();
});

test('compound HUD is transparent/passive outside its controls and preview fits the entire growing image', async () => {
  const rules = new Map(); postcss.parse(await raw('./scroll.css')).walkRules(rule => { const values = rules.get(rule.selector) ?? {}; rule.walkDecls(decl => { values[decl.prop] = decl.value; }); rules.set(rule.selector, values); });
  assert.equal(rules.get('.scroll-compound-window')['pointer-events'], 'none');
  assert.equal(rules.get('.scroll-compound-window .scroll-controls-slot')['pointer-events'], 'auto');
  assert.equal(rules.get('.scroll-live-outline')['pointer-events'], 'none');
  assert.equal(rules.get('.scroll-compound-window .scroll-preview img')['object-fit'], 'contain');
  assert.equal(rules.get('.scroll-compound-window .scroll-preview img')['object-position'], 'center bottom');
  assert.equal(rules.get('.scroll-compound-window .scroll-preview img').height, '100%');
  assert.equal(rules.get('.scroll-compound-window .scroll-preview').overflow, 'hidden');
});

test('live native HUD keeps the configured outside dimming and an exact clear source hole through preview updates', async () => {
  const f = await fixture(); f.next(initial); await tick();
  const mask = f.one('scroll-live-mask'); assert.ok(mask);
  const cutout = f.nodes().find(node => node.type === 'mask');
  assert.equal(cutout.props.maskUnits, 'userSpaceOnUse');
  const hole = cutout.children.find(node => node.props?.fill === 'black');
  assert.deepEqual(plain({ x: hole.props.x, y: hole.props.y, width: hole.props.width, height: hole.props.height }), initial.rect);
  const shade = f.nodes().find(node => node.props?.mask === 'url(#scroll-cutout)');
  assert.equal(shade.props['fill-opacity'], .66);
  f.next({ ...initial, height: 1200, status: 'extended', preview: 'data:image/png;base64,tall' }); await tick();
  assert.equal(f.one('scroll-live-mask'), mask); assert.equal(shade.props['fill-opacity'], .66);
  f.next(null); await tick(); assert.equal(f.one('scroll-live-mask'), undefined);
  assert.equal(f.document.documentElement.dataset.surface, 'scroll'); f.dispose();
});

test('staged scroll HUD never shades the still-visible screenshot and verified retained main keeps its original outside presentation', async () => {
  const f = await fixture(); try {
    f.next({ ...initial, phase: 'starting', hud: { ...initial.hud, live: false, retainMain: false } }); await tick();
    const outline = f.one('scroll-live-outline'), controls = f.one('scroll-controls-slot'), preview = f.one('scroll-preview');
    assert.ok(outline && controls && preview);
    assert.equal(f.one('scroll-live-mask'), undefined, 'staging cannot double the main screenshot dimming');
    assert.equal(f.document.documentElement.dataset.surface, 'scroll');
    f.next({ ...initial, phase: 'starting', hud: { ...initial.hud, retainMain: true } }); await tick();
    assert.equal(f.one('scroll-live-mask'), undefined, 'a verified native main aperture keeps the frozen background outside the ROI');
    f.next({ ...initial, height: 1600, status: 'extended', preview: 'data:image/png;base64,retained', hud: { ...initial.hud, retainMain: true } }); await tick();
    assert.equal(f.one('scroll-live-mask'), undefined);
    assert.equal(f.one('scroll-live-outline'), outline); assert.equal(f.one('scroll-controls-slot'), controls); assert.equal(f.one('scroll-preview'), preview);
    assert.equal(f.nodes().find(node => node.type === 'img').props.src, 'data:image/png;base64,retained');
    f.next({ ...initial, phase: 'finishing', hud: { ...initial.hud, retainMain: true } }); await tick();
    assert.equal(f.one('scroll-live-mask'), undefined, 'finishing cannot add another dark layer while the original window is being restored');
  } finally { f.dispose(); }
});

test('cloaked scroll fallback adds exactly one configured ROI mask only after native preparation without remounting existing controls', async () => {
  const f = await fixture(); try {
    f.next({ ...initial, phase: 'starting', hud: { ...initial.hud, live: false, retainMain: false } }); await tick();
    const controls = f.one('scroll-controls-slot'), outline = f.one('scroll-live-outline');
    assert.equal(f.one('scroll-live-mask'), undefined);
    f.next({ ...initial, phase: 'starting' }); await tick();
    const mask = f.one('scroll-live-mask'); assert.ok(mask);
    assert.equal(f.nodes().filter(node => node.props?.class === 'scroll-live-mask').length, 1);
    assert.equal(f.nodes().find(node => node.props?.mask === 'url(#scroll-cutout)').props['fill-opacity'], .66);
    const hole = f.nodes().find(node => node.type === 'mask').children.find(node => node.props?.fill === 'black');
    assert.deepEqual(plain({ x: hole.props.x, y: hole.props.y, width: hole.props.width, height: hole.props.height }), initial.rect);
    f.next({ ...initial, phase: 'finishing' }); await tick();
    assert.equal(f.one('scroll-live-mask'), mask); assert.equal(f.one('scroll-controls-slot'), controls); assert.equal(f.one('scroll-live-outline'), outline);
    f.next(null); await tick(); assert.equal(f.one('scroll-live-mask'), undefined); assert.equal(f.document.documentElement.dataset.surface, 'scroll');
  } finally { f.dispose(); }
});

test('scroll layout readiness waits for a visible connected nonzero production surface and is canceled when capture becomes live', async () => {
  const f=await fixture({hidden:true});try{
    f.next({...initial,phase:'starting',hud:undefined});await tick();assert.equal(f.frames.size,0);assert.deepEqual(f.readyCalls,[]);
    f.next({...initial,phase:'starting',hud:{...initial.hud,live:false}});await tick();
    assert.equal(f.frames.size,0);assert.ok(f.keys.has('visibilitychange'));
    f.visibility(false);assert.equal(f.frames.size,1);assert.deepEqual(f.readyCalls,[]);
    f.flushFrames();await tick();assert.deepEqual(f.readyCalls,['capture-a']);
    f.next({...initial,phase:'starting',height:700,hud:{...initial.hud,live:false}});await tick();
    assert.equal(f.frames.size,0,'a same-owner status refresh cannot restart readiness');
    f.next({...initial,phase:'starting',hud:{...initial.hud,live:true}});await tick();
    assert.equal(f.keys.has('visibilitychange'),false);assert.equal(f.frames.size,0);
  }finally{f.dispose();}
});

test('scroll readiness rejects hidden detached and zero-size layouts before acknowledging the exact current owner', async () => {
  const f=await fixture();try{
    f.next({...initial,phase:'starting',hud:{...initial.hud,live:false}});await tick();
    f.visibility(true);f.flushFrames();assert.deepEqual(f.readyCalls,[]);
    f.root.isConnected=false;f.visibility(false);f.flushFrames();assert.deepEqual(f.readyCalls,[]);
    f.root.isConnected=true;f.setBounds({width:0,height:800});f.visibility(false);f.flushFrames();assert.deepEqual(f.readyCalls,[]);
    f.setBounds({width:1200,height:800});f.visibility(false);f.flushFrames();await tick();assert.deepEqual(f.readyCalls,['capture-a']);
  }finally{f.dispose();}
});

test('canceling ending or disposing a pending scroll layout removes its frame and listener without a late acknowledgement', async () => {
  for(const action of ['finishing','null','dispose']){
    const f=await fixture();let disposed=false;try{
      f.next({...initial,phase:'starting',hud:{...initial.hud,live:false}});await tick();assert.equal(f.frames.size,1,action);
      if(action==='dispose'){f.dispose();disposed=true;}else f.next(action==='null'?null:{...initial,phase:'finishing'});
      await tick();assert.equal(f.frames.size,0,action);assert.equal(f.keys.has('visibilitychange'),false,action);
      f.flushFrames();assert.deepEqual(f.readyCalls,[],action);
    }finally{if(!disposed)f.dispose();}
  }
});

test('a late scroll layout acknowledgement failure cannot reach a successor or disposed surface while current failures remain visible', async () => {
  let rejectOld;
  const old=new Promise((resolve,reject)=>{rejectOld=reject;});
  const f=await fixture({ready:id=>id==='capture-a'?old:Promise.reject(Error('current readiness failed'))});try{
    f.next({...initial,phase:'starting',hud:{...initial.hud,live:false}});await tick();f.flushFrames();await tick();
    f.next({...initial,id:'capture-b',phase:'starting',hud:{...initial.hud,live:false}});await tick();
    rejectOld(Error('obsolete readiness failed'));await tick();assert.equal(f.one('scroll-notice'),undefined);
    f.flushFrames();await tick();assert.deepEqual(f.readyCalls,['capture-a','capture-b']);
    assert.ok(f.one('scroll-notice').children.some(node=>node.text?.includes('current readiness failed')));
  }finally{f.dispose();}
  let rejectDisposed;
  const ended=await fixture({ready:()=>new Promise((resolve,reject)=>{rejectDisposed=reject;})});
  ended.next({...initial,phase:'starting',hud:{...initial.hud,live:false}});await tick();ended.flushFrames();await tick();
  ended.dispose();rejectDisposed(Error('disposed readiness failed'));await tick();assert.equal(ended.frames.size,0);assert.equal(ended.keys.has('visibilitychange'),false);
});
