// SPDX-License-Identifier: MPL-2.0
// Exercise App's real capture wrapper, scene owner, lock props and backdrops with
// Solid. The SpaceCanvas leaf only observes ownership; no native screen or IPC.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';
import { parse } from '@babel/parser';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import postcss from 'postcss';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';

const raw = path => readFile(new URL(path, import.meta.url), 'utf8');
const nodesOf = source => {
  const nodes = [];
  const visit = node => {
    if (!node || typeof node !== 'object') return;
    if (node.type) nodes.push(node);
    for (const [key, value] of Object.entries(node)) {
      if (key === 'loc') continue;
      if (Array.isArray(value)) value.forEach(visit); else if (value && typeof value === 'object') visit(value);
    }
  };
  visit(parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] }));
  return nodes;
};
const app = await raw('./App.tsx'), nodes = nodesOf(app), slice = node => app.slice(node.start, node.end);
const definition = name => {
  const node = nodes.find(node => node.type === 'VariableDeclarator' && node.id.name === name);
  assert.ok(node, `production App binding ${name}`); return `const ${slice(node)};`;
};
const hasClass = (node, name) => node.type === 'JSXElement' && node.openingElement.attributes.some(value => value.name?.name === 'class' && value.value?.value === name);
const wrapper = nodes.find(node => hasClass(node, 'capture-scene-surface'));
assert.ok(wrapper, 'the production stable capture wrapper must exist');
const sceneShow = wrapper.children.find(node => node.type === 'JSXElement' && node.openingElement.name.name === 'Show' && node.openingElement.attributes.some(value => value.name?.name === 'keyed'));
assert.ok(sceneShow, 'keep the real keyed scene lifetime inside the stable wrapper');
const sceneCallback = sceneShow.children.find(node => node.type === 'JSXExpressionContainer' && node.expression.type === 'ArrowFunctionExpression').expression;
assert.equal(sceneCallback.params[0].name, 'currentId');
const ownedDeclarations = sceneCallback.body.body.filter(node => node.type === 'VariableDeclaration').map(slice).join('\n');
const canvas = nodes.find(node => node.type === 'JSXOpeningElement' && node.name.name === 'SpaceCanvas');
const canvasAttribute = name => {
  const attribute = canvas.attributes.find(value => value.name?.name === name);
  assert.ok(attribute, `production SpaceCanvas ${name}`); return slice(attribute);
};
const root = nodes.find(node => hasClass(node, 'app'));
assert.ok(root.children.includes(wrapper));
const backdrop = name => {
  const node = root.children.find(node => node.type === 'JSXElement' && node.openingElement.name.name === 'Show' && slice(node).includes(`<${name} `));
  assert.ok(node, `production ${name} is a sibling, not a replacement for the scene`); return slice(node);
};
const functionFrom = async (path, name) => {
  const source = await raw(path), node = nodesOf(source).find(node => node.type === 'FunctionDeclaration' && node.id.name === name);
  assert.ok(node, `production ${name}`); return source.slice(node.start, node.end);
};
const surface = await raw('./components/SpaceCanvas.tsx'), surfaceNodes = nodesOf(surface);
const blocked = surfaceNodes.find(node => node.type === 'VariableDeclarator' && node.id.name === 'blocked');
const begin = surfaceNodes.find(node => node.type === 'FunctionDeclaration' && node.id.name === 'begin');
assert.ok(blocked && begin);
const harness = `import {createMemo,untrack,onCleanup,Show} from 'solid-js';
const t=value=>value;
${await functionFrom('./RecordingSurface.tsx', 'RecordingBackdrop')}
${await functionFrom('./ScrollSurface.tsx', 'ScrollBackdrop')}
export function Presentation(props){
  const SpaceCanvas=props.Canvas;
  const snapshot=()=>props.snapshot,recording=()=>props.recording,scroll=()=>props.scroll;
  const busy=()=>props.busy,sending=()=>props.sending,exitPreparing=()=>props.exitPreparing;
  const preferences=()=>({maskOpacity:.47}),pinObjects=()=>[],isBlackboard=()=>false;
  const referenceDrafts=()=>({}),drafts=()=>({}),error=()=>'',LoaderCircle=()=>null,CircleAlert=()=>null;
  ${definition('scene')}
  ${definition('canvasScene')}
  ${definition('capturePresentationLive')}
  ${definition('captureSceneBackgroundOnly')}
  ${definition('captureSceneHidden')}
  return <div>${slice(wrapper.openingElement)}${slice(sceneShow.openingElement)}{currentId=>{
    ${ownedDeclarations}
    return <SpaceCanvas ${['scene','inputLocked','busy'].map(canvasAttribute).join(' ')}/>;
  }}</Show></div>${backdrop('RecordingBackdrop')}${backdrop('ScrollBackdrop')}</div>;
}
// Run the actual drawing-surface pointer admission with the actual App props.
// A hit-test spy terminates the unlocked path before starting a real gesture.
export function probeBegin(props,hit){
  const activity=()=>false,backgroundError=()=>false,geometry=()=>({width:1200});
  const exporting=()=>false,settling=()=>false,extensionRunning=()=>false;
  const drawingDiagnosticsEnabled=()=>false;
  const ${surface.slice(blocked.start,blocked.end)};
  ${surface.slice(begin.start,begin.end)}
  begin({button:0,target:{closest(){hit();return true;}}});
}`;
const compiled = stripTypeScriptTypes(transformSync(harness, { filename: 'capture-scene-lifetime.tsx', babelrc: false, configFile: false,
  parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'universal', moduleName: 'test-renderer' }]] }).code, { mode: 'transform' });

async function fixture() {
  const document = { documentElement: { dataset: { surface: 'space' } } }, context = createContext({ document, console });
  const synthetic = value => new SyntheticModule(Object.keys(value), function () { for (const [key, item] of Object.entries(value)) this.setExport(key, item); }, { context });
  const core = synthetic(solid), universal = new SourceTextModule(await raw('../../../node_modules/solid-js/universal/dist/universal.js'), { context });
  await universal.link(() => core); await universal.evaluate();
  const renderer = universal.namespace.createRenderer({
    createElement: type => ({ type, children: [], props: {}, parent: null }), createTextNode: text => ({ text, children: [], parent: null }),
    replaceText: (node, text) => { node.text = text; }, setProperty: (node, name, value) => { node.props[name] = value; },
    insertNode: (parent, node, anchor) => { if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1); const index = anchor ? parent.children.indexOf(anchor) : -1; parent.children.splice(index < 0 ? parent.children.length : index, 0, node); node.parent = parent; },
    removeNode: (parent, node) => { parent.children.splice(parent.children.indexOf(node), 1); node.parent = null; },
    isTextNode: node => 'text' in node, getParentNode: node => node.parent, getFirstChild: node => node.children[0], getNextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1],
  });
  const module = new SourceTextModule(compiled, { context });
  await module.link(name => name === 'solid-js' ? core : name === 'test-renderer' ? synthetic(renderer) : Promise.reject(Error(name))); await module.evaluate();
  const initial = { id: 'qa-scene', background: { id: 'qa-background' }, regions: [{ id: 'saved-region' }], items: [] };
  const [snapshot, setSnapshot] = solid.createSignal({ activeSceneId: initial.id, scenes: [initial] });
  const [recording, setRecording] = solid.createSignal(null), [scroll, setScroll] = solid.createSignal(null);
  const [busy, setBusy] = solid.createSignal(false), [sending, setSending] = solid.createSignal(false), [exiting, setExiting] = solid.createSignal(false);
  const mounted = [], cleaned = []; let observedProps;
  function Canvas(props) {
    observedProps = props;
    const owner = renderer.createElement('qa-canvas-owner'), image = renderer.createElement('qa-background'), selection = renderer.createElement('qa-selection');
    renderer.insertNode(owner, image); renderer.insertNode(owner, selection); mounted.push(owner);
    solid.createEffect(() => {
      owner.props.inputLocked = props.inputLocked; owner.props.busy = props.busy; owner.props.sceneId = props.scene.id;
      image.props.id = props.scene.background?.id; selection.props.regions = props.scene.regions.map(value => value.id);
    });
    solid.onCleanup(() => cleaned.push(owner));
    return owner;
  }
  const props = { Canvas, get snapshot() { return snapshot(); }, get recording() { return recording(); }, get scroll() { return scroll(); },
    get busy() { return busy(); }, get sending() { return sending(); }, get exitPreparing() { return exiting(); } };
  const container = renderer.createElement('root'), dispose = renderer.render(() => module.namespace.Presentation(props), container);
  const all = () => { const found = []; const visit = node => { found.push(node); node.children.forEach(visit); }; visit(container); return found; };
  const one = value => all().find(node => node.props?.class === value || node.type === value);
  return { mounted, cleaned, one, all, document, dispose, setRecording, setScroll, setSnapshot, setBusy, setSending, setExiting,
    probeBegin: () => { let hits = 0; module.namespace.probeBegin(observedProps, () => hits++); return hits; } };
}
const recordingStatus = (phase, countdown = 0) => ({ id: 'qa-recording', sceneId: 'qa-scene', phase, countdown, rect: { x: .2, y: .2, width: .5, height: .4 } });
const scrollStatus = phase => ({ id: 'qa-scroll', sceneId: 'qa-scene', phase, rect: { x: .2, y: .2, width: .5, height: .4 } });
const plain = value => JSON.parse(JSON.stringify(value));

test('recording preparation countdown live capture and stop retain the same production scene owner and original selection', async () => {
  const f=await fixture();try{
    const wrapper=f.one('capture-scene-surface'),owner=f.one('qa-canvas-owner'),image=f.one('qa-background'),selection=f.one('qa-selection');
    assert.equal(f.mounted.length,1);assert.equal(wrapper.props.style.display,'contents');
    f.setRecording(recordingStatus('starting'));
    assert.equal(f.one('capture-scene-surface'),wrapper);assert.equal(wrapper.props.style.display,'contents');assert.equal(wrapper.props.inert,true);
    assert.equal(f.one('qa-background'),image);assert.equal(f.one('qa-selection'),selection);assert.equal(f.one('recording-live-mask'),undefined);
    f.setRecording({...recordingStatus('starting'),hud:{rect:null,stage:'layout'}});
    assert.equal(wrapper.props.style.display,'contents');assert.equal(f.one('recording-live-mask'),undefined);
    f.setRecording({...recordingStatus('countdown',3),hud:{rect:null,stage:'layout'}});
    assert.equal(wrapper.props.style.display,'contents');assert.equal(wrapper.props.classList['capture-background-only'],true);assert.ok(f.one('recording-live-mask'));assert.ok(f.one('recording-countdown'));
    assert.ok(f.one('recording-countdown').children.some(node=>node.text==='3'));
    const mask=f.one('recording-live-mask');
    f.setRecording({...recordingStatus('starting'),hud:{rect:null,stage:'mask'}});
    assert.equal(wrapper.props.style.display,'none');assert.equal(f.one('recording-live-mask'),mask,'the post-countdown starting stage must retain its main mask');
    assert.equal(f.one('recording-countdown'),undefined);assert.equal(f.one('qa-canvas-owner'),owner);
    f.setRecording({...recordingStatus('starting'),hud:{rect:null,stage:'live'}});
    assert.equal(wrapper.props.style.display,'none');assert.equal(f.one('recording-live-mask'),undefined,'the live native HUD owns the sole mask');
    for(const phase of ['recording','paused','stopping']){f.setRecording({...recordingStatus(phase),hud:{rect:null,stage:'live'}});assert.equal(f.one('qa-canvas-owner'),owner);assert.equal(f.one('recording-live-mask'),undefined);assert.equal(f.one('recording-countdown'),undefined);}
    f.setRecording(null);assert.equal(wrapper.props.style.display,'contents');assert.equal(wrapper.props.inert,false);
    assert.equal(f.one('qa-background'),image);assert.equal(f.one('qa-selection'),selection);assert.equal(f.one('recording-live-mask'),undefined);
    assert.equal(f.document.documentElement.dataset.surface,'space');assert.equal(f.mounted.length,1);assert.equal(f.cleaned.length,0);
  }finally{f.dispose();assert.equal(f.cleaned.length,1);}
});

test('scroll preparation capture completion and null keep one scene mounted while hidden authoritative updates remain visible on return', async () => {
  const f=await fixture();try{
    const wrapper=f.one('capture-scene-surface'),owner=f.one('qa-canvas-owner'),selection=f.one('qa-selection');
    f.setScroll(scrollStatus('starting'));assert.equal(wrapper.props.style.display,'contents');assert.equal(wrapper.props.inert,true);assert.equal(f.one('scroll-live-mask'),undefined);
    f.setScroll({...scrollStatus('starting'),hud:{live:false}});assert.equal(wrapper.props.style.display,'contents');assert.equal(f.one('scroll-live-mask'),undefined);
    f.setScroll({...scrollStatus('starting'),hud:{live:true}});assert.equal(wrapper.props.style.display,'none');assert.equal(f.one('scroll-live-mask'),undefined,'the native HUD owns the live mask, including its starting handoff');
    f.setScroll({...scrollStatus('capturing'),hud:{live:true}});assert.equal(wrapper.props.style.display,'none');assert.equal(f.one('scroll-live-mask'),undefined);
    f.setSnapshot({activeSceneId:'qa-scene',scenes:[{id:'qa-scene',background:{id:'qa-background'},regions:[{id:'saved-region'},{id:'accepted-during-capture'}],items:[]}]});
    assert.equal(f.one('qa-selection'),selection);assert.deepEqual(plain(selection.props.regions),['saved-region','accepted-during-capture']);
    f.setScroll({...scrollStatus('finishing'),hud:{live:true}});assert.equal(f.one('qa-canvas-owner'),owner);assert.equal(f.cleaned.length,0);
    f.setScroll(null);assert.equal(wrapper.props.style.display,'contents');assert.equal(wrapper.props.inert,false);assert.equal(f.one('qa-selection'),selection);
    assert.equal(f.one('scroll-live-mask'),undefined);assert.equal(f.document.documentElement.dataset.surface,'space');assert.equal(f.mounted.length,1);
  }finally{f.dispose();assert.equal(f.cleaned.length,1);}
});

test('native-aperture long capture keeps the same decoded background with one main mask and no editable overlays', async () => {
  const f=await fixture();try{
    const wrapper=f.one('capture-scene-surface'),owner=f.one('qa-canvas-owner'),image=f.one('qa-background');
    f.setScroll({...scrollStatus('starting'),hud:{live:false,retainMain:false}});
    assert.equal(wrapper.props.style.display,'contents');assert.equal(wrapper.props.classList['capture-background-only'],false);
    assert.equal(f.one('scroll-live-mask'),undefined);
    for(const phase of ['starting','capturing','finishing']){
      f.setScroll({...scrollStatus(phase),hud:{live:true,retainMain:true}});
      assert.equal(wrapper.props.style.display,'contents');assert.equal(wrapper.props.classList['capture-background-only'],true);
      assert.equal(wrapper.props.inert,true);assert.ok(f.one('scroll-live-mask'));
      assert.equal(f.one('qa-canvas-owner'),owner);assert.equal(f.one('qa-background'),image);assert.equal(f.probeBegin(),0);
    }
    f.setScroll(null);assert.equal(wrapper.props.classList['capture-background-only'],false);
    assert.equal(f.one('qa-background'),image);assert.equal(f.one('scroll-live-mask'),undefined);
    assert.equal(f.mounted.length,1);assert.equal(f.cleaned.length,0);
    // These are the actual CSS branches used above: no original mask, region,
    // drawing, object or composer is left over the single live capture mask.
    const css=await raw('./components/capture.css');
    let hide;
    postcss.parse(css).walkRules(rule=>{if(rule.selector.includes('.capture-background-only>'))hide=rule;});
    assert.ok(hide);
    assert.deepEqual(hide.selectors,[
      '.capture-background-only>.space-canvas>:not(.capture-background)',
      '.capture-background-only>.space-canvas>:not(.capture-background) *',
      '.capture-background-only>:not(.space-canvas)',
      '.capture-background-only>:not(.space-canvas) *',
    ]);
    assert.ok(hide.nodes.some(decl=>decl.prop==='visibility'&&decl.value==='hidden'&&decl.important),
      'focused/hovered object tools explicitly set visible, so the mode must override descendants too');
  }finally{f.dispose();}
});

test('prepared compact recording keeps the original countdown image then one main live mask without a compound overlay', async () => {
  const f=await fixture();try{
    const wrapper=f.one('capture-scene-surface'),image=f.one('qa-background');
    f.setRecording(recordingStatus('starting'));assert.equal(wrapper.props.style.display,'contents');
    f.setRecording(recordingStatus('countdown',3));
    assert.equal(wrapper.props.style.display,'contents');assert.equal(wrapper.props.classList['capture-background-only'],true);
    assert.ok(f.one('recording-countdown'));assert.equal(f.one('qa-background'),image);
    const mask=f.one('recording-live-mask');
    for(const phase of ['starting','recording','paused','stopping']){
      f.setRecording({...recordingStatus(phase),retainMain:true});
      assert.equal(wrapper.props.style.display,'none');assert.equal(wrapper.props.inert,true);
      assert.equal(f.one('recording-live-mask'),mask);assert.equal(f.one('recording-countdown'),undefined);
      assert.equal(f.one('qa-background'),image);assert.equal(f.probeBegin(),0);
    }
    f.setRecording(null);assert.equal(wrapper.props.style.display,'contents');assert.equal(f.one('qa-background'),image);
    assert.equal(f.mounted.length,1);assert.equal(f.cleaned.length,0);
  }finally{f.dispose();}
});

test('all recording and scroll phases lock the actual SpaceCanvas props and its production pointer admission despite remaining mounted', async () => {
  const f=await fixture();try{
    assert.equal(f.probeBegin(),1,'the unlocked production handler reaches DOM hit testing');
    for(const phase of ['starting','countdown','recording','paused','stopping']){
      f.setRecording(recordingStatus(phase));assert.equal(f.one('qa-canvas-owner').props.inputLocked,true,phase);assert.equal(f.one('qa-canvas-owner').props.busy,true,phase);assert.equal(f.probeBegin(),0,phase);
    }
    f.setRecording(null);
    for(const phase of ['starting','capturing','finishing']){
      f.setScroll(scrollStatus(phase));assert.equal(f.one('qa-canvas-owner').props.inputLocked,true,phase);assert.equal(f.one('qa-canvas-owner').props.busy,true,phase);assert.equal(f.probeBegin(),0,phase);
    }
    f.setScroll(null);assert.equal(f.one('qa-canvas-owner').props.inputLocked,false);assert.equal(f.probeBegin(),1);
    f.setBusy(true);assert.equal(f.probeBegin(),0);f.setBusy(false);f.setSending(true);assert.equal(f.probeBegin(),0);f.setSending(false);
    f.setExiting(true);assert.equal(f.one('qa-canvas-owner').props.inputLocked,true);assert.equal(f.probeBegin(),0);
    assert.equal(f.mounted.length,1);assert.equal(f.cleaned.length,0);
  }finally{f.dispose();}
});

test('the stable capture wrapper still disposes the old owner on a real scene identity change and once on final unmount', async () => {
  const f=await fixture();try{
    const wrapper=f.one('capture-scene-surface'),first=f.one('qa-canvas-owner');
    f.setRecording(recordingStatus('recording'));
    f.setSnapshot({activeSceneId:'successor',scenes:[{id:'successor',background:{id:'successor-background'},regions:[],items:[]}]});
    assert.equal(f.one('capture-scene-surface'),wrapper);assert.notEqual(f.one('qa-canvas-owner'),first);assert.equal(f.mounted.length,2);assert.equal(f.cleaned.length,1);
    f.setRecording(null);assert.equal(f.mounted.length,2);assert.equal(f.cleaned.length,1);
  }finally{f.dispose();assert.equal(f.cleaned.length,2);}
});
